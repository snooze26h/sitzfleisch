# 时间折页

Sitzfleisch · 坐功的应用图标：骨白折带形成字母 S，朱红端面标记正在进行的时间，石墨底板延续应用现有配色。整体采用克制的立体材质，减少旧图标里椅子、旗子与刻度的并列元素。

## 文件

| 文件 | 用途 |
| --- | --- |
| `source.png` | 1254 × 1254 RGBA 原始母图，圆角底板外透明 |
| `icon.icns` | macOS 多尺寸图标 |
| `icon.ico` | Windows 多尺寸图标 |
| `32x32.png` | 32 × 32 PNG |
| `128x128.png` | 128 × 128 PNG |
| `128x128@2x.png` | 256 × 256 PNG |
| `sizes-preview.png` | 实际尺寸检查图；上排深底、下排浅底；从左到右 128、64、48、32、22、16px |
| `PROMPT.md` | 内置 ImageGen 的完整生成提示词与来源记录 |

PNG、ICNS、ICO 由项目当前安装的 Tauri CLI 从母图导出。检查图仅通过 ImageMagick 缩放、铺底与拼接制作，未重绘图标内容。

## 当前状态与验证

用户确认直接替换后，已将平台图标重新导出到 `src-tauri/icons/`，原有打包引用直接使用新图标。此目录保留母图与桌面导出副本。已检查透明背景、PNG 尺寸与 RGBA 通道；ICNS 可用系统 `iconutil` 解包；ICO 包含 16、24、32、48、64、256px。已人工检查深浅背景下的六档尺寸，并做独立视觉复核。

32px 仍能辨认 S 与折带；16–22px 的材质与朱红端面减弱。当前代码的托盘使用 `default_window_icon()`，随应用图标同步更新；尚未单独设计菜单栏单色稿。

已使用本机离线依赖重新构建 release `.app`，安装到 `/Applications/Sitzfleisch.app`，核验安装包签名与图标资源一致性，并通过应用自身菜单退出、重启后继续原来的计时。旧安装版保存在 `backup-apps/time-fold-installed-original-20260907-1810/Sitzfleisch.app`。

未单独完成 Dock、菜单栏的像素级视觉验收或 Windows 真机验收；没有为静态图标资产运行业务测试。

## 重新导出

在项目根目录执行，先输出到独立临时目录：

```bash
task_icon_export=$(mktemp -d /private/tmp/sitzfleisch-icon.XXXXXX)
npm run tauri -- icon design/app-icon/time-fold/source.png -o "$task_icon_export"
```

Tauri 会同时生成移动端文件；`src-tauri/icons/` 已同步更新完整派生集合，此目录只保留上表列出的桌面文件。
