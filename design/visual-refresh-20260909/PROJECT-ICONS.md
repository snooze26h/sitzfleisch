# 项目图标设计记录

本轮将常用项目图标延伸为与折页导航一致的手写 SVG 系列，保留 Sitzfleisch 与 S 标识，不落地新的中文名称。应用规则、用户记录与已保存图标标识保持原有语义。

## 已实现的图形

来源为 [src/project-icons.ts](../../src/project-icons.ts)，不是图像模型生成素材。八枚图形共用 24 × 24 画布、1.6px 描边、圆头圆角、.13–.20 透明度的形体填充，以及 `--brand-accent: #cd6145` 的朱红切面。轮廓和填充继承 `currentColor`，随所在表面及选中态改变明暗。

| 标签 | 保存标识 |
| --- | --- |
| 科研 | `flask-conical` |
| 阅读 | `book-open` |
| 语言 | `languages` |
| 浏览 | `newspaper` |
| 交流 | `message-square` |
| 编程 | `binary` |
| 锻炼 | `dumbbell` |
| 思考 | `brain` |

统一渲染入口为 [src/icons.ts](../../src/icons.ts)。命中上述标识时使用专属形体，其余已知图标继续使用 Lucide，未知标识回退至 Lucide BookOpen。旧 `code` 标识只是 `binary` 的外观别名，不迁移或重写已有保存值。

## 选择器与兼容性

[设置编辑器](../../src/views/settings.ts) 默认展示八项，主区四列；[最终 CSS](../../src/visual.css) 定义按钮 64 × 65px、9px 圆角、9px 间距，≤ 700px 改为两列。按钮内图标 25px，标签 11px、行高 1.3，二者间距 6px。标签最大宽 56px，过长时省略。

普通按钮使用石墨 `plate` 背景与 `ink-muted` 文字；悬停切换至 `plate-lift` / `ink-high`；选中项采用骨白 `stroke` 背景和深色 `bed` 文字，朱红切面为 `#a4462d`。按钮保留可见焦点、短标签、`aria-label` 和 `aria-pressed`。

「更多图标」用原生 details 保留其余通用图标。若当前保存的图标不在八项中，会将它额外放在主区首项，保持可见且可继续选用，主区此时为九项。项目行内专属图标显示为 22px，计划行内为 20px。

## 验证记录与边界

依据本轮主流程与独立审查已报告的结果：

- TypeScript、Vite 与 Tauri release app 构建通过。
- 八枚图标、`code` 别名、通用标识与未知标识回退的 smoke 检查通过。
- 模拟浏览器中点击「科研」及旧「指南针」后，保存与 `aria-pressed` 回显通过；1020px 宽度未出现横向溢出。
- 独立审查 `project_icon_review` 检查了 [1240px 截图](../../.impeccable/review/project-icons-1240.png)、[1020px 截图](../../.impeccable/review/project-icons-1020.png) 和图标、保存源码，结论为 `disposition: ship`，无 material findings。
- [检测器记录](../../.impeccable/review/project-icon-detector.json) 共 19 项，均为 advisory；本轮新增的 9px 圆角和 11px 标签是本组件的有意设计，不据此调整其他组件。

截图与交互检查使用模拟环境，不据此宣称原生视觉验证通过；原生安装状态由主流程记录。
