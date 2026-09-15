# 侧栏立体 S 抠图

按用户要求，用原应用图标中的立体 S 替换左上角简化的手绘 SVG；文字名称和其他导航图标保留。

- 原图：`design/app-icon/time-fold/source.png`。
- 内置 image_gen 制作黑白轮廓蒙版，作为原图抠取的空间约束；ImageMagick 结合原图颜色分离背景，保留原像素。首轮返回的棋盘格 RGB 图片没有用于产品。
- 原尺寸抠图：`sitzfleisch-cutout.png`；缩放前所有可见像素 RGB 与原图一致，差异数为 0。
- 最终素材：[sitzfleisch-mark.webp](../../../src/assets/brand/sitzfleisch-mark.webp)，256 × 318，50,348 字节，无损 WebP，具有真实透明 alpha。外部背景与上下内部空隙抽样 alpha 均为 0。
- 精确提示词、来源和处理步骤：[素材记录](../../../src/assets/brand/sitzfleisch-mark.webp.json)。
- 侧栏按 38 × 48 CSS px 显示，使用普通透明图像，不设置底板、圆角图框或混合模式。

已检查 Chrome 本地模拟预览中 1240 与 1020 两种窗口宽度，素材加载正常、字标没有横向溢出，材质与镂空边缘可见。截图在 `.impeccable/review/sidebar-mark-1240.png`、`sidebar-mark-detail.png` 和 `sidebar-mark-1020-detail.png`。这些是浏览器预览，不等同于原生 WebView 实拍。
