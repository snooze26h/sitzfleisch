# 开发与验证

[返回项目首页](../README.md) · [使用说明](usage.md)

## 开发环境

CI 使用 Node.js 22、Rust stable。原生开发还需安装对应平台的构建工具，见 [Tauri 2 环境准备](https://v2.tauri.app/start/prerequisites/)。

在仓库根目录运行：

```bash
npm ci
npm run dev          # 浏览器预览，使用内存中的模拟数据
npm run tauri dev    # 原生开发模式
npm run tauri build  # 构建当前系统的安装包
```

上面的启动命令按需选择。macOS 构建产物为 `.app` 和 `.dmg`，Windows 为 NSIS 安装包。`cargo` 需在 PATH 中；使用 rustup 默认安装且终端未加载路径时，可执行：

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## 目录结构

```text
core/               纯 Rust 规则引擎
src-tauri/          Tauri 系统集成：命令、托盘、心跳、存储与网站屏蔽
src/                TypeScript 界面，使用 morphdom 更新页面
browser-extension/  Chrome / Edge 扩展、本地协议与回归测试
docs/               使用说明、开发说明与 README 截图
scripts/            静态检查与回归检查脚本
```

两个 Rust crate 编译进应用，前端打包为静态资源；界面由系统 WebView 渲染。应用不附带 Node.js、Python 或 Chromium。`scripts/` 中仅可选的 macOS hosts 助手及其安装脚本随应用打包，其余脚本用于开发检查。

## 自动检查

按变更范围运行受影响的检查。完整 CI 检查集见 [check.yml](../.github/workflows/check.yml)，在仓库根目录可执行：

```bash
npx tsc --noEmit
node scripts/check-dead-exports.mjs
node scripts/check-snapshot-order.mjs
node scripts/check-short-name.mjs
node scripts/check-scheduler.mjs
node scripts/check-timeline.mjs
node scripts/check-blocking-rules.mjs
node --test browser-extension/*.test.mjs
node scripts/check-security.mjs
cargo test --manifest-path core/Cargo.toml
cargo clippy --manifest-path core/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

## 界面预览

### 完整退出解除屏蔽（0.11.3 / 扩展 1.2.0）

- 正常 `ExitRequested` 先阻止退出，在后台串行清理系统 hosts；成功后再允许退出，取消授权或清理失败时保留应用供重试。Dock / 注销等只有 `Exit` 的路径做兜底清理，强制终止进程不作保证。
- 退出标记只在内存中，规则接口在退出过程中返回未启用；不结束或丢弃学习日、不删除用户屏蔽列表。排队的 hosts 启用任务看到退出标记后只能清理，避免退出末尾重新写回。再次启动按保存的状态恢复规则。
- 扩展在连接不可达或超时时解除执行缓存，恢复屏蔽页；新导航先同步，避免先被旧缓存拦一次。无效响应和执行失败仍保留校验与错误状态，不伪报同步成功。
- 隔离数据目录禁止执行系统 hosts 写入/清理，也不连接真实浏览器桥接端口。测试用临时 hosts 文件验证只清理托管段、标记不完整或嵌套时保留原文件、授权失败和回读失败；扩展测试覆盖旧缓存、断开、重连、导航竞争和解除失败重试。
- 本机已安装 0.11.3 与随包扩展 1.2.0；用户重新加载扩展到 1.2.0 后，确认原先被拦的页面可以访问。系统 hosts 清理验证使用临时文件，不能替代所有系统退出入口的真机验收。
- 退出保存失败转为系统屏蔽清理失败时，弹窗按新阶段替换按钮和回调；重试等待清理完成。浏览器预览可依次调用 `qaQuitBlocked()`、`qaQuitBlockingFailed()` 检查两个阶段。

### macOS 全屏桌面预览

0.11.1 在 [space_preview.rs](../src-tauri/src/space_preview.rs) 中增加全屏 Space 的预览兼容层，用户已确认重启后调度中心缩略图恢复。WKWebView 使用远程绘制图层，普通视图截图不能可靠取得内容；这里用 Apple 的公开 [`takeSnapshot`](https://developer.apple.com/documentation/webkit/wkwebview/takesnapshot(with:completionhandler:)) API 生成原生位图预览。

0.11.2 修正了预览层的双屏回归：旧判断把失去键盘焦点也当成显示快照的条件，焦点移到外接屏后，主屏仍可见的窗口会被低分辨率静态图覆盖，倒计时看似停止。现在仅依据 [`occlusionState`](https://developer.apple.com/documentation/appkit/nswindow/occlusionstate-swift.property) 判断是否完全被遮挡；失焦但仍可见的窗口继续显示实时 WebView。

- 仅前台全屏窗口每 5 秒更新一次，长边最多 480 点；后台不持续截图，不关闭 WebKit 的节电机制。
- 全屏窗口完全被遮挡时显示已有预览，重新可见时立即隐藏，即使焦点仍在另一块屏幕上；普通窗口、隐藏和最小化状态不显示预览层。
- 预览层不参与鼠标命中和辅助功能树；尺寸变化、退出全屏后拒绝旧快照，截图失败时保留上次有效内容。
- 位图只留在内存中，不写入存档、不上传，也不读取其他窗口。计时、提醒与存储逻辑未改动。

相关依据：[WebKit 的远程视图截图问题与快照 API](https://bugs.webkit.org/show_bug.cgi?id=161450)、[同类 Tauri 应用的原生位图预览方案](https://menketechnologies.github.io/MenkeTechnologiesMeta/Audio-Haxor/#ux--desktop-integration)。这些资料支持兼容方式，不能单凭它们认定本机这一例的精确系统根因。

回归检查包括另一块屏幕取得焦点、未重新聚焦但窗口恢复可见、普通/隐藏/最小化窗口、尺寸变化后的异步回调和失败重试。双屏新增用例在 0.11.1 上失败、修复后通过。此前的离屏原生探针还检查了预览视图的创建、鼠标穿透、辅助功能标记、图像设置与移除。

真机回归需覆盖：坐功全屏时操作另一块屏幕、来回切换焦点、调度中心预览、切回立即点击操作、退出全屏、最小化后恢复。双屏修复前已实机观察到画面停留在旧倒计时、辅助功能树中的实时读数仍持续更新；修复后的验证结果按实际执行记录。

### 浏览器模拟场景

运行 `npm run dev` 后，在浏览器打开 `http://localhost:1420/`，通过 URL 片段选择内置场景，例如 `http://localhost:1420/#qa=running`。切换片段后刷新页面以重新加载场景。

```text
#qa=start | fresh | chooser | running | paused | suspended | resting | completed | finishing | done | protected | savefail | nohistory
```

这些场景来自 [src/dev/mock.ts](../src/dev/mock.ts)，不会读取真实存档，也不执行系统网站屏蔽。`savefail` 模拟保存失败；浏览器没有应用退出流程，可在控制台执行 `qaQuitBlocked()` 查看退出前保存失败的对话框。

加 `&platform=android` 预览手机版。应用屏蔽另有几个片段：`&apps=off|none`（开关关着或从没设置过）、`&a11y=off`（无障碍服务没开）、`&applist=limited`（系统只交出部分应用）、`&notice=<包名>`（刚被送回坐功的提示）。

### README 截图

[images/](images/) 中的三张 WebP 来自当前前端的浏览器预览，使用内置演示数据；只用于展示界面，不作为原生功能验收证据。

| 文件 | 场景 | 视口（CSS 像素） |
| --- | --- | --- |
| `plan.webp` | `#qa=nohistory`，开始页 | 1240 × 820 |
| `focus.webp` | `#qa=running` | 1240 × 920 |
| `history.webp` | `#qa=start`，点击「历史」 | 1240 × 840 |

截图于 2026-09-30 随「一轮月」界面更新（今天页的月相与历史页月历），设备像素比为 2，WebP 质量为 88。为固定演示日期与倒计时，页面加载前将 `Date.now()` 固定为 `2026-09-22T14:20:00-07:00`。截图使用内置模拟数据，没有读取个人记录。

### 月面资料

今天页、历史月历和侧栏里的月亮都是运行时按进度现算光照（[moon.ts](../src/moon.ts)），用的是 [moon-maps.webp](../src/assets/yue/moon-maps.webp)（R 为月面反照率，G/B 为环形山起伏的法线偏移）和 [moon-shading.json](../src/assets/yue/moon-shading.json) 里的光照参数；浏览器扩展拦截页的半月是同一套公式预先渲染的。

月面数据来自 NASA Scientific Visualization Studio 的 [CGI Moon Kit](https://svs.gsfc.nasa.gov/4720)（`lroc_color_poles_4k.tif` 与 `ldem_16_uint.tif`），署名：NASA's Scientific Visualization Studio。原始数据不进仓库；需要重新生成时，把这两个文件下载到 `design/yue/source/`，再在仓库根目录运行 `python3 design/yue/render_moon.py`。光照参数改在 [moon.json](../design/yue/moon.json)。

## 隔离原生测试数据

`SITZFLEISCH_DATA_DIR` 可将状态文件、hosts 暂存文件和 QA 报告放入独立目录。

- 未设置时，使用[默认数据目录](usage.md#历史与数据)。
- 设置后，必须是可创建的绝对路径。空值、纯空白、相对路径或非 UTF-8 值都会报错停止，不创建窗口，也不回退到真实目录。
- 隔离目录首次启动使用内置默认计划，生成的是测试数据。
- 窗口位置记忆与开机自启不受此变量影响；系统 hosts 也没有被虚拟化，测试整站屏蔽仍会影响系统。

## 构建与发布

[build.yml](../.github/workflows/build.yml) 在 macOS 和 Windows runner 上构建安装包。可在 Actions 中手动触发并下载 Artifacts；推送 `v*` 标签会构建并发布 GitHub Release。

所有 Action 使用完整提交 SHA；默认 `contents: read`，checkout 不保留凭据，仅发布任务有 `contents: write`。两个平台的构建均运行外壳测试；Windows 的提权脚本测试只写临时文件，不调用 UAC 或刷新系统 DNS。安全扫描与本地修复记录见 [2026-10-02 安全修复](security-20261002.md)。

macOS 发布包是 Apple 芯片与 Intel 通用二进制。当前仅做 ad-hoc 签名，未做 Developer ID 签名与公证；Windows 构建成功不等同于真机验收通过。

## Android 开发

使用 Java 17、Android API 36、Build Tools 36.0.0、NDK 30.0.16248370，以及 Rust 的 `aarch64-linux-android` target。最低 API 为 26，只构建 ARM64；前端仍由系统 WebView 渲染。环境准备参见 [Tauri](https://v2.tauri.app/start/prerequisites/#android)；项目使用的组件版本以 Gradle 配置和 CI 为准。

设置 `JAVA_HOME`、`ANDROID_HOME` 和 `NDK_HOME`，其中 `NDK_HOME` 指向 `$ANDROID_HOME/ndk/30.0.16248370`。在仓库根目录执行：

```bash
npm run tauri android build -- --debug --apk --target aarch64
npm run tauri android build -- --apk --target aarch64 --ci
python3 -m unittest discover -s src-tauri/gen/android/tests -v
```

第二条是 release 构建：启用 R8，只有配置了本地密钥时才签名；无配置时仍可构建，未签名产物不能用于正式安装或发布。调试包 ID 为 `com.snooze26h.sitzfleisch.x.debug`，正式包 ID 为 `com.snooze26h.sitzfleisch.x`，数据独立。

### 真机调试

手机允许 USB 调试后，先用 `adb devices` 确认连接。单台真机可以使用以下命令；多设备时改用 `adb -s <设备标识>`，不把标识或私人日志提交进仓库。

```bash
adb -d install -r path/to/sitzfleisch-debug.apk
adb -d shell am start -n com.snooze26h.sitzfleisch.x.debug/com.snooze26h.sitzfleisch.x.MainActivity
adb -d shell pidof com.snooze26h.sitzfleisch.x.debug
adb -d shell logcat --pid <上一步返回的PID>
mkdir -p .android-work/shots
adb -d exec-out screencap -p > .android-work/shots/device.png
```

安装需按手机系统提示确认；不卸载旧应用、不清数据。Chrome 中打开 `chrome://inspect/#devices`，可调试已启动的 debug WebView。生产包不打开 WebView 调试。截图、日志、APK 和本机配置放在忽略的 `.android-work/` 中。

应用屏蔽依赖无障碍服务。只在模拟器上用命令打开它，真机由使用者在系统设置里自己打开：

```bash
adb -e shell settings put secure enabled_accessibility_services com.snooze26h.sitzfleisch.x.debug/com.snooze26h.sitzfleisch.android.AppBlockService
adb -e shell settings put secure accessibility_enabled 1
adb -e shell dumpsys accessibility | grep -i sitzfleisch
```

这条命令会覆盖模拟器上已打开的其他无障碍服务。

### 发布签名

发布者在自己的终端生成并备份密钥，密码不写入聊天、源码、命令参数或日志：

```bash
mkdir -p ~/.android-keys
keytool -genkeypair -v -keystore ~/.android-keys/sitzfleisch-release.jks -keyalg RSA -keysize 4096 -validity 10000 -alias sitzfleisch
```

使用同一个密码作为 keystore 和该 key 的密码。由发布者在 `src-tauri/gen/android/keystore.properties` 中填写 `storeFile`、`keyAlias`、`password`；文件已忽略，使用绝对的密钥文件路径即可。配置文件不存在时不启用 release 签名；存在但字段无效或 keystore 缺失时明确失败。遵循 [Tauri 的 Android 签名格式](https://v2.tauri.app/distribute/sign/android/)，其中反斜线、换行、前导空格和 Unicode 密码按 Java Properties 规则转义。

发布者自行设置仓库 Secrets，`gh` 未写 `--body` 时会交互询问，不把密码放进 shell 参数：

```bash
base64 -i ~/.android-keys/sitzfleisch-release.jks | gh secret set ANDROID_KEY_BASE64 --repo snooze26h/sitzfleisch
gh secret set ANDROID_KEY_ALIAS --repo snooze26h/sitzfleisch
gh secret set ANDROID_KEY_PASSWORD --repo snooze26h/sitzfleisch
```

alias 填 `sitzfleisch`，password 填生成密钥时的密码。这三项的管理方式见 [GitHub Secrets 文档](https://docs.github.com/en/actions/how-tos/write-workflows/choose-what-workflows-do/use-secrets)。密钥同时备份到密码管理器和离线介质；丢失后无法用新签名直接覆盖旧应用，卸载再安装会删除记录。

### CI 产物与验收

`build.yml` 的 Android job 沿用同一工具链。三个 Secrets 齐全时构建正式包，否则手动触发时产出 debug APK（文件名带 `-debug`），发布标签缺任何一项都会失败。构建期间磁盘上没有密钥：先产出未签名的 release APK，再由单独一步把 keystore 解码到临时目录、用 `apksigner` 签名（密码经环境变量传入），签完立即删除；不生成 `keystore.properties`，也不上传 keystore。

产物命名为 `Sitzfleisch_<版本>_android-arm64.apk` 和同名 `.sha256`，artifact 为 `sitzfleisch-Android`。CI 用 `zipalign -c -P 16` 检查 16 KB 页对齐，用 [apksigner](https://developer.android.com/tools/apksigner) 校验签名，再计算校验和；Release 等待桌面和 Android 两个构建，并且只接受 release 模式的 Android 产物。

发版前用签名包在真机上复测：首次安装与通知授权、锁屏到点提醒、从最近任务划掉后的提醒、延长或暂停后旧提醒被取消、常驻通知、返回键、复制 Markdown，以及用新版覆盖安装后记录保留。

## 已有验证记录

以下承接整理前 README 的已有记录，并非本次文档更新重新执行的原生验收：

- 在隔离数据目录手动验证过 16 个原生场景，记录为 16 通过、0 失败。覆盖学习日与计时操作、计时中断自动暂停、记水、关窗与 Dock 唤回、⌘Q 与重启、设置控件、通知、hosts 写入与解除、菜单栏操作、收工归档、历史与 Markdown 复制，以及放弃测试日。记录中真实存档与 `/etc/hosts` 在验收前后逐字节一致。
- 计时中断使用冻结进程复现，未做真实合盖验收。WKWebView 原生 `<select>` 选项未能逐个自动点击；Windows 特有系统集成路径没有自动覆盖，完整检查工作流使用 macOS runner。
- 原有屏蔽解除验收覆盖系统解析层，不保证浏览器旧连接和内部 DNS 缓存同步恢复；用户真实 Chrome / Edge 登录会话尚未完成全面验收。

专项记录：

- [0.11.0 项目审查与修复](audit-20260922.md)

- [视觉更新说明](../design/visual-refresh-20260909/README.md)与[验证范围](../design/visual-refresh-20260909/VALIDATION.md)
- [菜单栏稳定性记录](../design/menu-bar-stability-20260914.md)
- [浏览器扩展协议与回归测试](../browser-extension/README.md)
