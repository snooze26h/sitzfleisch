#!/usr/bin/env python3
"""生成 Android 启动器图标（自适应图标的前景 / 单色层，以及旧版方形 / 圆形图标）。

为什么不用 `tauri icon` 的产物：它把整张桌面图标（深色圆角底板 + 立体 S）原样塞进
108dp 的前景层，而安卓桌面只显示中间 72dp、并且只保证 66dp 圆内不被裁切。结果 S 高
约 75dp，上下被裁、显得撑满。这里改用抠好的 S（sidebar-mark/sitzfleisch-cutout.png）
做前景，按安全区缩放；底板颜色单独做背景层，和桌面图标的观感一致。

用法（仓库根目录）：
    python3 design/android/make_launcher_icon.py            # 写入 gen/android 与 icons/android
    python3 design/android/make_launcher_icon.py --preview DIR  # 只输出预览，不改仓库
依赖 Pillow。
"""
import argparse
import math
import statistics
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "design/app-icon/time-fold/source.png"
CUTOUT = ROOT / "design/visual-refresh-20260909/sidebar-mark/sitzfleisch-cutout.png"
RES = ROOT / "src-tauri/gen/android/app/src/main/res"
ICONS = ROOT / "src-tauri/icons/android"

# S 的高度占 108dp 画布的比例。0.49 时最远的不透明像素离中心约 32.5dp，
# 落在 66dp 安全圆（半径 33dp）以内；与桌面图标里 S 占底板约 78% 的比例接近。
S_HEIGHT_RATIO = 0.49
DENSITIES = {"mdpi": 1.0, "hdpi": 1.5, "xhdpi": 2.0, "xxhdpi": 3.0, "xxxhdpi": 4.0}


def tile_color(src: Image.Image, cut: Image.Image) -> tuple:
    """取桌面图标底板的颜色：底板不透明、S 透明处像素的中位数。"""
    sa, ca = src.getchannel("A"), cut.getchannel("A")
    pixels = [
        src.getpixel((x, y))[:3]
        for y in range(0, src.size[1], 6)
        for x in range(0, src.size[0], 6)
        if sa.getpixel((x, y)) > 250 and ca.getpixel((x, y)) < 5
    ]
    return tuple(int(statistics.median(p[i] for p in pixels)) for i in range(3))


def s_mark(cut: Image.Image) -> Image.Image:
    return cut.crop(cut.getchannel("A").point(lambda a: 255 if a > 4 else 0).getbbox())


def foreground(mark: Image.Image, size: int) -> Image.Image:
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    h = round(size * S_HEIGHT_RATIO)
    w = round(mark.size[0] * h / mark.size[1])
    scaled = mark.resize((w, h), Image.LANCZOS)
    canvas.alpha_composite(scaled, ((size - w) // 2, (size - h) // 2))
    return canvas


def monochrome(fg: Image.Image) -> Image.Image:
    """主题图标（Android 13+）用的单色层：保留 S 的轮廓，颜色交给系统着色。"""
    white = Image.new("RGBA", fg.size, (255, 255, 255, 0))
    white.putalpha(fg.getchannel("A"))
    return white


def composite_108(bg: tuple, fg: Image.Image) -> Image.Image:
    full = Image.new("RGBA", fg.size, bg + (255,))
    full.alpha_composite(fg)
    return full


def viewport(full: Image.Image) -> Image.Image:
    """裁出桌面实际显示的中间 72dp（108dp 的 2/3）。"""
    n = full.size[0]
    m = round(n / 6)
    return full.crop((m, m, n - m, n - m))


def mask(size: int, shape: str) -> Image.Image:
    scale = 4
    big = size * scale
    m = Image.new("L", (big, big), 0)
    d = ImageDraw.Draw(m)
    if shape == "circle":
        d.ellipse((0, 0, big - 1, big - 1), fill=255)
    elif shape == "rounded":
        d.rounded_rectangle((0, 0, big - 1, big - 1), radius=round(big * 0.22), fill=255)
    else:  # squircle：超椭圆，接近多数国产桌面的图标形状
        n = 5.0
        pts = []
        for i in range(720):
            t = 2 * math.pi * i / 720
            c, s = math.cos(t), math.sin(t)
            x = math.copysign(abs(c) ** (2 / n), c)
            y = math.copysign(abs(s) ** (2 / n), s)
            pts.append(((x + 1) / 2 * (big - 1), (y + 1) / 2 * (big - 1)))
        d.polygon(pts, fill=255)
    return m.resize((size, size), Image.LANCZOS)


def masked(img: Image.Image, shape: str) -> Image.Image:
    out = img.copy()
    out.putalpha(mask(img.size[0], shape))
    return out


def legacy(full_108: Image.Image, px: int, shape: str) -> Image.Image:
    return masked(viewport(full_108).resize((px, px), Image.LANCZOS), shape)


def write_resources(bg: tuple, mark: Image.Image) -> None:
    for name, k in DENSITIES.items():
        fg = foreground(mark, round(108 * k))
        full = composite_108(bg, fg)
        for base in (RES / f"mipmap-{name}", ICONS / f"mipmap-{name}"):
            base.mkdir(parents=True, exist_ok=True)
            fg.save(base / "ic_launcher_foreground.png", optimize=True)
            monochrome(fg).save(base / "ic_launcher_monochrome.png", optimize=True)
            legacy(full, round(48 * k), "rounded").save(base / "ic_launcher.png", optimize=True)
            legacy(full, round(48 * k), "circle").save(base / "ic_launcher_round.png", optimize=True)
    color = "#%02x%02x%02x" % bg
    xml_color = (
        '<?xml version="1.0" encoding="utf-8"?>\n<resources>\n'
        f'  <color name="ic_launcher_background">{color}</color>\n</resources>\n'
    )
    xml_icon = (
        '<?xml version="1.0" encoding="utf-8"?>\n'
        '<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n'
        '  <background android:drawable="@color/ic_launcher_background"/>\n'
        '  <foreground android:drawable="@mipmap/ic_launcher_foreground"/>\n'
        '  <monochrome android:drawable="@mipmap/ic_launcher_monochrome"/>\n'
        "</adaptive-icon>\n"
    )
    for base in (RES, ICONS):
        (base / "values").mkdir(parents=True, exist_ok=True)
        (base / "values/ic_launcher_background.xml").write_text(xml_color, encoding="utf-8")
        (base / "mipmap-anydpi-v26").mkdir(parents=True, exist_ok=True)
        (base / "mipmap-anydpi-v26/ic_launcher.xml").write_text(xml_icon, encoding="utf-8")


def preview(bg: tuple, mark: Image.Image, out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    cell = 216
    shapes = ("rounded", "squircle", "circle")
    old_fg = Image.open(RES / "mipmap-xxxhdpi/ic_launcher_foreground.png").convert("RGBA")
    old_full = composite_108((255, 255, 255), old_fg.resize((432, 432)))
    new_full = composite_108(bg, foreground(mark, 432))
    sheet = Image.new("RGBA", (cell * 3 + 80, cell * 2 + 60), (232, 228, 220, 255))
    d = ImageDraw.Draw(sheet)
    for row, (label, full) in enumerate((("before", old_full), ("after", new_full))):
        d.text((8, 20 + row * (cell + 30)), label, fill=(30, 30, 30, 255))
        for col, shape in enumerate(shapes):
            icon = masked(viewport(full).resize((cell - 24, cell - 24), Image.LANCZOS), shape)
            sheet.alpha_composite(icon, (60 + col * cell, 20 + row * (cell + 30)))
    sheet.save(out_dir / "launcher-icon-compare.png")
    small = Image.new("RGBA", (6 * 70 + 20, 90), (20, 20, 17, 255))
    for i, shape in enumerate(shapes * 2):
        size = 48 if i < 3 else 64
        icon = masked(viewport(new_full).resize((size, size), Image.LANCZOS), shape)
        small.alpha_composite(icon, (10 + i * 70, (90 - size) // 2))
    small.save(out_dir / "launcher-icon-small.png")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--preview", type=Path, help="只输出预览图到该目录，不写入仓库")
    args = parser.parse_args()
    src = Image.open(SOURCE).convert("RGBA")
    cut = Image.open(CUTOUT).convert("RGBA")
    bg = tile_color(src, cut)
    mark = s_mark(cut)
    if args.preview:
        preview(bg, mark, args.preview)
        print("preview written to", args.preview, "background", "#%02x%02x%02x" % bg)
        return
    write_resources(bg, mark)
    print("launcher icons written; background", "#%02x%02x%02x" % bg)


if __name__ == "__main__":
    main()
