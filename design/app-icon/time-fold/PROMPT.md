# 时间折页：生成记录

- 方式：内置 ImageGen，单次生成。
- 原始输出：1254 × 1254，RGBA PNG；外侧背景透明。
- `source.png` 是原始文件的逐字节副本。其余桌面格式使用项目当前安装的 Tauri CLI 从该文件导出，无内容重绘。
- 设计范围：应用图标。用户确认直接替换后，已导出至 `src-tauri/icons/` 并接入原有打包引用。

## 完整提示词

```text
Use case: logo-brand.
Asset type: finished desktop application icon master for Sitzfleisch / 坐功, a personal study timer where a day begins when you sit down and focus time accumulates in blocks.
Primary request: Design one exceptionally beautiful and original, professionally art-directed macOS app icon. Concept: "Time Fold". A bold custom S-shaped sculptural folded band also subtly suggests a seat in its negative space. It must feel like a distinctive app identity, not a stock alphabet glyph. The previous icon was an overly literal chair, ruler and flag; replace that composition entirely.
Subject and form: ONE large, confident sculptural S mark made from a broad warm ivory band, bent twice with refined rounded outer bends and precise inner folds. A single small vermilion end face at the upper-right terminal, integrated into the band rather than floating, signifies the current time block. Strong silhouette, generous open negative space through the bends. The folds create the identity; no separate chair illustration, no additional symbols. Very restrained shallow ceramic/enamel relief, mostly face-on with subtle material thickness, optical centering and beautifully balanced proportions.
Scene/backdrop: a deep warm graphite macOS rounded-square tile with a very subtle charcoal material finish, no grain noise. Front-facing orthographic view, no perspective of the tile. The tile occupies approximately 82% of the square canvas, centered, with truly transparent alpha outside its rounded-square silhouette. Keep generous icon-grid margins and avoid any square background outside the tile.
Color palette: graphite #1B1B1E tile, bone-white #EDEAE3 main mark, restrained vermilion #C7452F terminal accent only. Black-and-ivory contrast should read instantly at small Dock sizes.
Lighting/mood: calm, intelligent, tactile, meticulously crafted; gentle upper-left studio illumination and tight natural contact shading only. No glow, neon, chrome or glossy plastic. The main mark fills about 60% of the tile and remains clear at 32px.
Composition: exactly one centered icon, square 1024x1024 image. No presentation board, no extra small icons, no captions.
Text: none. The S is a bespoke symbol, not typeset text.
Avoid: clock faces, hourglasses, progress rings, checkmarks, rockets, flames, flags, rulers, decorative ticks, literal furniture, extra colored objects, blue or purple gradients, glassmorphism, dramatic shadows, watermark.
```
