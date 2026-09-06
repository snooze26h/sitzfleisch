# Sitzfleisch · 坐功

给作息不规律的人用的学习计时器。不看几点起床，也不在午夜清零——**一天从你坐下的那一刻算起**。

Rust + TypeScript（Tauri 2），macOS + Windows 双端。

## 装

去 [Releases](https://github.com/snooze26h/sitzfleisch/releases) 下载。

**macOS**：下载 `.dmg` 拖进「应用程序」。通用二进制，Intel 和 Apple 芯片都能跑。
这个 app 没有 Apple 开发者签名和公证，首次打开系统会拦，说「已损坏」或「无法验证开发者」——
不是真的损坏，**右键点图标 → 打开 → 再点一次「打开」**即可，或者终端里跑
`xattr -dr com.apple.quarantine /Applications/Sitzfleisch.app`。

**Windows**：下载 `-setup.exe` 运行。SmartScreen 可能拦一下，点「更多信息 → 仍要运行」。
Windows 端从未在真机上验收过，遇到问题请开 issue。

数据只存在你自己电脑上，不联网、不上传。

## 四条规矩

- **一天从「开始今天」那一刻算起**，不跨午夜重置。
- **一天的每一秒不是在某个格里，就是在暂停里。** 心跳间隔超过 120 秒（合盖、休眠、重启）
  会被识别成空档，空档一秒都不补给任何格，但它会被一段自动暂停盖住——所以「已学 + 已暂停」
  永远等于「从坐下到现在」。
- **一格走完或手动结束就直接计入进度**，不需要验收。主动**放弃**的格时间只留在台账上，不计入。
- **状态文件损坏、来自更高版本、或读不出来时一律进保护模式**，绝不覆盖原文件。

## 能做什么

**开始一天**。三个档位（轻量 4h / 标准 8h30 / 冲刺 14h30），每档是各项目目标分钟数的合计，
在设置里按「项目 × 档位」矩阵改。进行中也可以切档。

**开一格**。选项目、选时长、选之后休息多久，开始。可以 +10 分钟延长，可以随时暂停 / 继续，
可以提前结束（计入）或放弃（不计入）。这一格要做的事可以列成清单，勾掉。
没有格在走的时候，界面和菜单栏报的是**下一格该做什么**——调度器给建议并附一句理由。

**看得见时间去哪了**。运行图（Bildfahrplan）把一天画成横轴时间、纵轴项目的走向图，
计入 / 未计入 / 暂停 / 休息四种笔画分得清；配额轨按项目显示进度；顶栏是「已学 / 已暂停」两个大读数。

**菜单栏常驻**。窗口关掉也不退出。菜单栏标题跟着状态走：跑的时候是倒计时，休息是休息倒计时，
停了是「哪一格 + 停了多久」，闲着是「下一格 X」。菜单里能开始一天、开格、记水、退出。

**网站屏蔽**。在设置里列域名，学习日开始时写进系统 hosts（裸域与 www、IPv4 与 IPv6 各一条），
收工或放弃时清掉。写入和解除各要一次管理员授权。解除会顺手重启 `mDNSResponder`，
让被解除的域名**立刻**恢复解析，不用等缓存过期。手动解除要走两步确认单（还得把域名原样打一遍）。

**身体**。喝水计数（可撤销）、久坐提醒、起身护眼提醒、闲置多久开始催，都可以关。
提醒有提示音和系统通知两条路。

**收工与历史**。收工归档进历史，也可以整天放弃（不留归档）。历史页有最近 14 天柱图、
可展开的归档行（配额、台账、复制成 Markdown、永久删除）。

**短名**。侧栏、运行图行标、菜单栏标题上用的名字。设置里填了就立刻用；留空自动回退——
放得下就用全名（4 个汉字 / 8 个拉丁字符），再长取空格前的首个词，还放不下取**前**两个字
（取前两个字不是后两个：「深度工作」截成「工作」会丢掉是哪一种）。改项目名不会清掉你填过的短名。

## 存不下来会说出来

磁盘写入失败时，今天 / 历史 / 设置三页顶部都会亮一条横条，写明系统原因、说明改动还在内存里，
右侧「立即重试」当场再存一次；之后任何一次保存成功就自动清空。

退出前那次保存写不进去时**不退出**，先问「重试并退出 / 不保存退出 / 留在这里」。

这条拦截只覆盖**应用菜单「退出 坐功」（含 ⌘Q）与菜单栏图标的「退出」**。从 **Dock 图标右键
「退出」**、AppleScript `quit` 或注销走，macOS 根本不给应用「要不要退」的问句
（tao 没注册 `applicationShouldTerminate:`），拦不住——那几条路会在进程走掉前补存一次，
磁盘好使时不丢，**磁盘写不进去时静默丢失、没有提示**。

## 已知边界

**原生验收**：16 个场景在隔离目录（`SITZFLEISCH_DATA_DIR`）上手动跑过，16 通过、0 失败。
覆盖开始学习日、开格 / 暂停 / 继续 / 结束、计时中断自动暂停、记水、关窗与 Dock 唤回、⌘Q 与重启、
设置控件、系统通知、网站屏蔽写入与解除、菜单栏菜单、收工归档、重启后历史与 Markdown 复制、
放弃测试日。真实存档与 `/etc/hosts` 验收前后逐字节一致。

没验到的，如实记着：

- **真合盖没做**。「计时中断」那条是用冻结进程等价复现的；WKWebView 睡醒重绘、
  菜单栏图标挺不挺得过真实睡眠，仍然空白。
- **锁屏不自动暂停**。单独锁屏但心跳继续时不会触发暂停，没接系统锁屏观察器。
- **屏蔽解除只测到系统解析这一层**。实测解除后 0.16 秒 `example.com` 就回到真实 IP、
  `curl` 拿到 200；但浏览器各自的 socket 池和内部 DNS 缓存什么时候放掉旧连接，不归 App 管。
- **设置页的下拉选择器没能逐个自动点**：WKWebView 不把原生 `<select>` 的选项暴露进辅助功能树。
- **Windows 从未在真机上验收过**，`#[cfg(windows)]` 的那几个函数也没有自动覆盖（CI 只有 macOS）。

## 结构

```
core/        纯 Rust 规则引擎（无平台依赖，cargo test 覆盖核心规则）
src-tauri/   Tauri 壳：命令、托盘、心跳线程、原子落盘（也有自己的 cargo test）
src/         TypeScript 界面（vanilla，无框架，用 morphdom 打补丁）
scripts/     守门脚本：死导出扫描、快照顺序、短名规则（CI 里跑）
```

`core/` 与 `src-tauri/` 是编译进产物的两个 crate，`src/` 打包成静态资源塞进同一个二进制。
`scripts/` 只在 CI 里跑，**一个字节都不进产物**——装出来的 app 就是一个 Mach-O 可执行文件加
图标，界面由系统自带的 WebView 渲染，不带 Node、不带 Python、不带 Chromium。

## 数据在哪

macOS `~/Library/Application Support/com.snooze26h.sitzfleisch.x/state.json`，
Windows `%APPDATA%\com.snooze26h.sitzfleisch.x\state.json`。只存在本机。

`SITZFLEISCH_DATA_DIR` 可以把状态文件、hosts 暂存文件与 QA 报告整体挪到别处，用来做不碰真实
存档的故障注入测试。语义是严格的：**没设置**就用上面的默认目录；**设置了**就必须是绝对路径且能
创建，空、只有空白、相对路径、非 UTF-8 一律报错停止（一行 stderr + 非零退出，不建窗口），
**绝不退回真实目录**。隔离目录首启拿到的是内置默认计划，所以那里跑出来的是合成数据。
窗口位置记忆与开机自启不受它影响。

## 开发与构建

```bash
export PATH="$HOME/.cargo/bin:$PATH"
npm install

# 提交前的最小集（CI 里跑的就是这几条）
npx tsc --noEmit
node scripts/check-dead-exports.mjs
node scripts/check-snapshot-order.mjs
node scripts/check-short-name.mjs
(cd core && cargo test && cargo clippy --all-targets -- -D warnings)
(cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings)

npm run dev               # 浏览器里跑（走 src/dev/mock.ts），见下面的 QA 场景
npm run tauri dev         # 开发模式
npm run tauri build       # 本机产物（mac: .app + .dmg / win: NSIS 安装包）
```

界面布局靠 mock 场景人工验收。`npm run dev` 之后按 hash 切场景：

```
#qa=start | fresh | chooser | running | paused | suspended | resting | done | protected | savefail | nohistory
```

`savefail` 是「存盘写不进去」那一版：三页顶部都有横条。浏览器里没有退出这回事，
要看退出前保存失败的对话框，在开发者工具里执行 `qaQuitBlocked()`。

Windows 安装包由 GitHub Actions 在 windows-latest 上构建（本机没有 Windows）：
Actions 页面手动触发 `build` workflow，或推一个 `v*` 标签，然后在 run 的 Artifacts 里下载。

## 还没做的

1. 正式签名与公证（需 Apple Developer ID 帐号）
2. Windows 端全部功能的真机验收（托盘没有标题位，倒计时只在悬停提示里）
3. 网站屏蔽的提权体验：现在每次写 hosts 都弹一次系统授权；SwitchHosts 的做法
   （`AuthorizationExecuteWithPrivileges` 缓存授权 / `SMAppService` 守护）可作后续改进

## 许可证

[MIT](LICENSE)
