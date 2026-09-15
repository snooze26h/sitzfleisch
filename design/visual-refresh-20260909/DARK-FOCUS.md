# 深色工作台设计记录

本轮按用户确认的方向，将当前格统一为深色工作台：拒绝大面积白色工作面和直接粘贴方形 S 图标，使用骨白细线、小块选中态、朱红切面与原创衍生折带。Sitzfleisch、S 字标和「坐功」名称保留。

## 成品与来源

[最终 CSS](../../src/visual.css) 中已移除 `.paper-surface`。计划与当前格背景均为 #171715，文字和细线为暖骨白；输入、下拉和选项保持深色。主操作为 #ae422a 朱红底配 #fff5e7 文字，选择项以小块骨白反白，保留清楚的边线、键盘焦点和标签。

[brand.ts](../../src/brand.ts) 的 `ribbonScene()` 使用三张 960 × 640 WebP，合计 37,892 字节：

| 资源 | 形体与状态 |
| --- | --- |
| `ribbon-ready.webp` | 横向展开，供开始页与准备下一格使用 |
| `ribbon-flow.webp` | 横向延伸并轻扭，供正在执行使用 |
| `ribbon-rest.webp` | 低位收拢，供暂停、休息和完成使用；完成态缩至 .88 |

三图由内置 `image_gen` 参考既有骨白边线、深色材质与朱红切面衍生生成，不采用 S 字母轮廓。它们是纯黑背景的不透明图片，不含透明 alpha；CSS 使用 `screen` 与渐隐 mask 消除图块边界。精确提示词、生成来源、原图和选用文件见 [dark-focus-assets.json](dark-focus-assets.json)。

[today.ts](../../src/views/today.ts) 将三个图层保持在同一场景内，以 `ready`、`flow`、`rest`、`done` 状态控制交叠；透明度过渡 .45s，位移与缩放过渡 .55s。每秒 tick 不重播过渡。开始页改用 ready 折带，历史仍使用 fold-archive。旧 fold-open、fold-rest 和纸纹不进入本轮生产包，其 WebP 已从 src/assets 移除，原图与提示词记录仍留在本地 originals/ 与 assets.json。

## 局部交互

- 建议区域只显示建议文字，移除与下方项目选择重复的「选科研」等快捷按钮；项目仍从选择栏或右侧配额选择。
- 当前格选择项最小高 46px、6px 圆角。悬停抬起 2px并显出 10px 朱红折角，按下回原位；选中为骨白小块、深字和完整折角。
- 任务完成保留勾选与划线，并显示 5px 朱红折角；.35s 的旋转、缩放反馈从 rotate(-12deg) scale(.85) 回到原位。
- [runStrip()](../../src/components.ts) 使用 23px 高的时间尺容器、3px 骨白进度细带和 6px 朱红游标。游标为 skewY(-30deg)，位置限制在尺的边界内；进度条保留数值与已专注时长语义。
- 右上角标固定为 30 × 30px，通过 scale(.633333) → scale(1) 表达非执行与执行状态，过渡只作用于 transform。
- 保留全局减少动态效果覆盖；图层、选择项和任务反馈均服从该设置。

## 验证记录与边界

本轮主流程已报告以下结果，文档同时核对了最终源码与九张成品截图：

- 模拟浏览器中选择 25 分钟、输入两项任务、开始、勾选、暂停、继续、延长 10 分钟至 35 分钟、结束后进入 5 分钟休息，交互通过。
- 1020px 预览没有横向溢出。减少动态效果的实际 emulation 中，ribbon frame 与选择项 transition 均为 0s，检查后已恢复；按钮键盘焦点为可见 2px 外框。
- 指定元素对比度参考：次要文字 7.29:1，主按钮 5.39:1，控件边线最差悬停态 3.23:1，下拉箭头 5.91:1。这些比值对应本轮检查元素，不代表完整页面认证。
- TypeScript、Vite、Tauri release app 最终构建与签名检查通过。主流程报告的最终二进制 SHA-256 为 `542bf35a1098d30ce09ba19819c122c8ab1292221fbc89dcd8d68a90d7a2f931`。
- 独立全新审查 `dark_focus_review` 返回 `disposition: ship`，无 material findings。
- 检测器单次输出 16 条：1 warning、15 advisory。width/height 动画 warning 对应的角标已改为固定 30px 尺寸加 scale；最终源码复核已解决。未将此结果记作重跑扫描或完整 HTML 扫描。

| 状态 | 已核对截图 |
| --- | --- |
| 开始 | [1240px](../../.impeccable/review/dark-start-1240.png) |
| 选择下一格 | [1240px](../../.impeccable/review/dark-chooser-1240.png)、[1020px](../../.impeccable/review/dark-chooser-1020.png) |
| 执行 | [1240px](../../.impeccable/review/dark-running-1240.png)、[1020px](../../.impeccable/review/dark-running-1020.png)、[1600px](../../.impeccable/review/dark-running-1600.png) |
| 暂停 | [1240px](../../.impeccable/review/dark-paused-1240.png) |
| 休息 | [1240px](../../.impeccable/review/dark-resting-1240.png) |
| 完成 | [1240px](../../.impeccable/review/dark-done-1240.png) |

以上截图和交互均来自 Chrome 模拟预览，不是原生窗口实拍。深色新版已安装到 `/Applications/Sitzfleisch.app`，签名、二进制及用户数据核对通过，安装记录见 [VALIDATION.md](VALIDATION.md)。自动重开因本机控制工具连接失败而未完成，原生视觉仍未实拍验证。
