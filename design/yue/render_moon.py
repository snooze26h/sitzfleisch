#!/usr/bin/env python3
# 今天这一轮月：月面资料图 + 一张预先照好的半月。
#
# 界面不再裁一张平光的满月，而是按进度现算光照：太阳从右边斜照过来，越靠近明暗界线光越斜、越暗，
# 界线附近的环形山自己显出起伏——这才是真月亮的样子。这里只准备算光要用的资料：
#   src/assets/yue/moon-maps.webp（无损）——正面月盘 1024×1024：
#       R = 月面反照率；G、B = 环形山起伏造成的法线偏移（屏幕坐标 x、y，128 为零，满量程 ±RELIEF_RANGE）
#   src/assets/yue/moon-shading.json —— 光照参数，界面（src/moon.ts）与这里共用
#   browser-extension/assets/moon-half.webp —— 拦截页那一轮：用同一套公式预先照好的上弦月
#
# 数据：NASA Scientific Visualization Studio「CGI Moon Kit」（https://svs.gsfc.nasa.gov/4720）
#   lroc_color_poles_4k.tif（LRO 广角相机彩色拼图，4096×2048）
#   ldem_16_uint.tif（LOLA 高程，每度 16 像素；无符号 16 位，单位半米，整体加了 20000 的偏移）
# 使用要求：注明 NASA's Scientific Visualization Studio。原始数据不进仓库，运行前下载到 design/yue/source/。
#
# 在仓库根目录运行：python3 design/yue/render_moon.py
import json
import math
import os

import numpy as np
from PIL import Image

Image.MAX_IMAGE_PIXELS = None  # 高程图 5760×2880，超过 PIL 默认的「防炸弹」阈值

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
SRC = os.path.join(HERE, "source")
OUT = os.path.join(ROOT, "src", "assets", "yue")
EXT = os.path.join(ROOT, "browser-extension", "assets")
CFG = json.load(open(os.path.join(HERE, "moon.json"), encoding="utf-8"))
MOON_KM = 1737.4


def bilinear(img, u, v):
    """在等距圆柱图上双线性取样；u 横向（经度）环绕，v 纵向截断。"""
    h, w = img.shape[:2]
    x = u * w - 0.5
    y = np.clip(v * h - 0.5, 0, h - 1.001)
    x0 = np.floor(x).astype(np.int64)
    y0 = np.floor(y).astype(np.int64)
    fx = x - x0
    fy = y - y0
    x0 %= w
    x1 = (x0 + 1) % w
    y1 = np.minimum(y0 + 1, h - 1)
    a, b, c, d = img[y0, x0], img[y0, x1], img[y1, x0], img[y1, x1]
    return (a * (1 - fx) + b * fx) * (1 - fy) + (c * (1 - fx) + d * fx) * fy


def disc(size, radius, ss):
    """正面月盘上每个像素的球面坐标与法线。ss 倍超采样。"""
    S = size * ss
    r = radius * ss
    c = S / 2
    yy, xx = np.mgrid[0:S, 0:S].astype(np.float64)
    nx = (xx + 0.5 - c) / r
    ny = -(yy + 0.5 - c) / r
    rr = nx * nx + ny * ny
    nz = np.sqrt(np.clip(1 - rr, 0, 1))
    lat = np.arcsin(np.clip(ny, -1, 1))
    lon = np.arctan2(nx, np.maximum(nz, 1e-9))
    cover = np.clip(0.5 - (np.sqrt(rr) - 1.0) * r, 0, 1)
    return nx, ny, nz, lat, lon, cover


def gblur(a, sigma):
    """FFT 高斯模糊（经向环绕，纬向两端也按环绕处理，极区用不到）。sigma 以像素计。"""
    ky = np.fft.fftfreq(a.shape[0])
    kx = np.fft.rfftfreq(a.shape[1])
    g = np.exp(-2.0 * (np.pi ** 2) * (sigma ** 2) * (ky[:, None] ** 2 + kx[None, :] ** 2))
    return np.fft.irfft2(np.fft.rfft2(a) * g, s=a.shape)


def maps():
    color = np.asarray(Image.open(os.path.join(SRC, CFG["color_map"])).convert("RGB"), np.float32) / 255.0
    raw = np.asarray(Image.open(os.path.join(SRC, CFG["elev_map"])), np.float64)
    elev = (raw - CFG["elev_offset"]) * CFG["elev_km_per_unit"]  # 千米
    lum = (color @ np.array([0.2126, 0.7152, 0.0722], np.float32)).astype(np.float64)
    lo, hi = np.percentile(lum, [CFG["black_pct"], CFG["white_pct"]])
    albedo = np.clip((lum - lo) / (hi - lo), 0, 1)

    # 高程的坡度（无量纲）：经向每像素的地面距离随纬度变短
    H, W = elev.shape
    dlat = math.pi / H
    dlon = 2 * math.pi / W
    lat_rows = (math.pi / 2 - (np.arange(H) + 0.5) * dlat)[:, None]
    # 先轻轻去掉比月盘像素还细的起伏，免得取样时混叠成噪点
    elev = gblur(elev, CFG["elev_presmooth"])
    gy, gx = np.gradient(elev)
    slope_e = gx / (MOON_KM * dlon * np.maximum(np.cos(lat_rows), 0.05))
    slope_n = -gy / (MOON_KM * dlat)
    return albedo, slope_e, slope_n


def main():
    size, radius, ss = CFG["size"], CFG["radius"], CFG["supersample"]
    albedo, slope_e, slope_n = maps()
    nx, ny, nz, lat, lon, cover = disc(size, radius, ss)
    u = (lon + math.pi) / (2 * math.pi)
    v = (math.pi / 2 - lat) / math.pi
    A = bilinear(albedo, u, v)
    se = bilinear(slope_e, u, v) * CFG["relief"]
    sn = bilinear(slope_n, u, v) * CFG["relief"]
    # 起伏把法线往下坡方向推：切向东 Te=(cos lon, 0, −sin lon)，切向北 Tn=(−sin lat·sin lon, cos lat, −sin lat·cos lon)
    te = np.stack([np.cos(lon), np.zeros_like(lon), -np.sin(lon)], -1)
    tn = np.stack([-np.sin(lat) * np.sin(lon), np.cos(lat), -np.sin(lat) * np.cos(lon)], -1)
    n0 = np.stack([nx, ny, nz], -1)
    n1 = n0 - se[..., None] * te - sn[..., None] * tn
    n1 /= np.linalg.norm(n1, axis=-1, keepdims=True)
    # 只存偏移量（屏幕坐标），球面本身在界面里按解析式算——整张球的法线若也压成 8 位，界线附近会起台阶
    dx = np.clip((n1[..., 0] - nx), -CFG["relief_range"], CFG["relief_range"])
    dy = np.clip((n1[..., 1] - ny), -CFG["relief_range"], CFG["relief_range"])

    def down(a):
        s = a.shape[0] // ss
        return a.reshape(s, ss, s, ss).mean(axis=(1, 3))

    A_d, dx_d, dy_d, cov_d = down(A * cover), down(dx * cover), down(dy * cover), down(cover)
    safe = np.maximum(cov_d, 1e-6)
    A_d, dx_d, dy_d = A_d / safe, dx_d / safe, dy_d / safe
    # 反照率轻度锐化（天文摄影处理月面的常规一步）：只在月盘里做，月缘外不外溢
    inside = cov_d > 0.999
    blur = gblur(np.where(inside, A_d, 0.0), CFG["sharpen_sigma"])
    wts = gblur(inside.astype(np.float64), CFG["sharpen_sigma"])
    local = blur / np.maximum(wts, 1e-6)
    A_d = np.where(inside, A_d + CFG["sharpen_amount"] * (A_d - local), A_d)
    rng = CFG["relief_range"]
    rgb = np.dstack([
        np.clip(A_d, 0, 1) * 255,
        128 + np.clip(dx_d / rng, -1, 1) * 127,
        128 + np.clip(dy_d / rng, -1, 1) * 127,
    ])
    rgb = np.where(cov_d[..., None] > 0, rgb, 0)
    os.makedirs(OUT, exist_ok=True)
    maps_path = os.path.join(OUT, "moon-maps.webp")
    Image.fromarray((rgb + 0.5).astype(np.uint8), "RGB").save(maps_path, "WEBP", lossless=True, method=6)

    shading = {k: CFG[k] for k in ("size", "radius", "relief_range", "lommel", "gamma", "albedo_floor", "earthshine", "light", "exposure")}
    json.dump(shading, open(os.path.join(OUT, "moon-shading.json"), "w", encoding="utf-8"), indent=1)
    credit = {
        "source": f"NASA Scientific Visualization Studio, CGI Moon Kit (https://svs.gsfc.nasa.gov/4720): {CFG['color_map']}, {CFG['elev_map']}",
        "credit": "NASA's Scientific Visualization Studio",
        "tool": "design/yue/render_moon.py（正射投影 + 高程坡度 → 法线偏移 + 反照率轻度锐化，程序处理，无生成模型）",
        "command": "python3 design/yue/render_moon.py",
        "config": "design/yue/moon.json",
    }
    json.dump(dict(credit, file="src/assets/yue/moon-maps.webp", use="月面资料：R 反照率，G/B 起伏法线偏移；界面按进度现算光照（src/moon.ts）"),
              open(maps_path + ".json", "w", encoding="utf-8"), ensure_ascii=False, indent=2)

    # 拦截页那一轮：同一套公式，预先照好一张上弦月（亮一半）
    half = shade(A, n1, nx, nz, cover, 0.5)
    im = Image.fromarray((half * 255 + 0.5).astype(np.uint8), "RGBA").resize((CFG["ext_size"], CFG["ext_size"]), Image.LANCZOS)
    ext_path = os.path.join(EXT, "moon-half.webp")
    im.save(ext_path, "WEBP", quality=92, alpha_quality=100, method=6)
    json.dump(dict(credit, file="browser-extension/assets/moon-half.webp", use="浏览器拦截页：学习日还没结束，月亮亮了一半"),
              open(ext_path + ".json", "w", encoding="utf-8"), ensure_ascii=False, indent=2)
    print(maps_path, os.path.getsize(maps_path), ext_path, os.path.getsize(ext_path))


def shade(A, n1, nx, nz, cover, lit):
    """与 src/moon.ts 的 shade 同一套：月面光度按 Lommel–Seeliger 与 Lambert 混合，暗面只留一点地照。"""
    g = math.acos(2 * lit - 1)  # 相位角：满月 0，新月 π
    L = np.array([math.sin(g), 0.0, math.cos(g)])
    mu0 = n1 @ L
    mu = np.maximum(n1[..., 2], 1e-4)
    w = CFG["lommel"]
    ph = np.where(mu0 > 0, (1 - w) * mu0 + w * 2 * mu0 / (mu0 + mu), 0.0)
    alb = CFG["albedo_floor"] + (1 - CFG["albedo_floor"]) * A
    val = np.clip(ph * alb * CFG["exposure"], 0, 1) ** CFG["gamma"]
    val = np.maximum(val, CFG["earthshine"] * alb * (0.55 + 0.45 * nz))
    col = np.array(CFG["light"], np.float64) / 255.0
    return np.concatenate([np.broadcast_to(col, val.shape + (3,)), (val * cover)[..., None]], -1)


if __name__ == "__main__":
    main()
