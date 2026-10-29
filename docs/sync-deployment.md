# 共享账号与计时同步的部署

这份说明把 Mac 和 Android 接到同一个 Supabase 项目。仓库包含数据库迁移和本地测试；按下面的顺序完成项目、邮件与客户端配置后，再进行真实账号和双设备验收。未配置云端地址与公开密钥的构建继续使用单机模式。

协议和离线冲突规则见 [同步实现约定](sync-design.md)。首版通过有界轮询、操作后唤醒和恢复前台补拉同步，不依赖 Supabase Realtime，不需要配置 publication 或开放私有表。

## 1. 创建项目并保存公开配置

在自己的 Supabase 账号中创建项目，选择 Mac 与手机都能稳定连接的地区。项目准备好后，在 Dashboard 的连接/API 配置中取得：

- Project URL，例如 `https://YOUR_PROJECT_REF.supabase.co`。
- **Publishable key**，格式为 `sb_publishable_...`。旧项目的 `anon` 公钥仅用于兼容；新配置优先使用 publishable key。

客户端中的公开密钥用于识别项目，用户登录后的访问令牌才代表账号。不要把 `sb_secret_...`、`service_role`、数据库口令、SMTP 口令或管理员令牌放进客户端、仓库或构建日志。这些高权限密钥能绕过数据隔离。[Supabase API keys](https://supabase.com/docs/guides/getting-started/api-keys)

## 2. 安装数据库迁移

在目标项目的 SQL Editor 中运行完整的 [202610270001_shared_sync.sql](../supabase/migrations/202610270001_shared_sync.sql)。迁移自带事务，失败时不会留下半套权限；再次运行会保留已有文档和操作记录。

如果已用 PostgreSQL 的 service 配置安全保存了目标数据库连接，也可以执行：

```bash
psql 'service=sitzfleisch-sync' -v ON_ERROR_STOP=1 \
  -f supabase/migrations/202610270001_shared_sync.sql
```

迁移会创建：

| 对象 | 用途 |
| --- | --- |
| `sitz_sync_private.documents` | 每个账号一份共享文档与递增版本 |
| `sitz_sync_private.operations` | 操作摘要与原提交版本，供重试去重 |
| `public.sitz_sync_read` | 读取当前账号的文档；已知版本不重复传正文 |
| `public.sitz_sync_write` | 在账号行锁内比较版本、核对操作身份并写入 |

保持 `public` 在 Data API 的 exposed schemas 中；**不要把 `sitz_sync_private` 加进去**。迁移已启用并强制 RLS，撤销客户端对两张表的权限，仅向 `authenticated` 授予两个 RPC 的执行权限。匿名角色不能调用 RPC，函数也再次检查 `auth.uid()`。[数据库函数权限说明](https://supabase.com/docs/guides/database/functions)

操作摘要不会自动过期。删除旧摘要可能让旧设备的重试失去去重依据，不应把它们当普通日志定期清空。删除 Supabase Auth 用户会级联删除该账号的共享文档和操作记录。

## 3. 配置邮箱验证码

先配置发信服务，再修改模板。如果控制台显示 **Set up custom SMTP to edit templates**，表示默认发信服务仅使用固定模板，当前不能编辑正文；不能直接跳到验证码模板步骤。

在 Authentication → Emails 点击 **Set up SMTP**，配置邮件服务商提供的 SMTP 主机、端口、发件邮箱、用户名和专用凭据。SMTP 密钥只填在 Supabase 控制台，不写进坐功客户端或仓库。[自定义 SMTP 说明](https://supabase.com/docs/guides/auth/auth-smtp)

个人 Gmail 用于少量测试时，需要先在 Google 账号启用两步验证，再生成命名为“坐功 Supabase”的应用专用密码；不能填 Gmail 的普通登录密码。凭据由账号本人创建和填写，不发到聊天或截图中。某些工作/学校账号、高级保护账号不提供此选项，应遵循账号限制选择其他发信方式。[Google 应用专用密码说明](https://support.google.com/accounts/answer/185833?hl=zh-Hans)

SMTP 配置保存后，在 Authentication 中保持 Email 登录启用。在 Emails / Email Templates 中，将 **Magic Link（Magic link or OTP）** 和 **Confirm sign up** 两个模板都改为包含验证码变量 `{{ .Token }}`，覆盖后续登录和首次注册。例如：

```html
<h2>登录坐功</h2>
<p>你的验证码：<strong>{{ .Token }}</strong></p>
<p>回到坐功输入验证码即可登录。若不是你发起的请求，请忽略这封邮件。</p>
```

验证码登录与 Magic Link 共用发送入口；模板只放链接时，用户收不到应用所需的数字验证码。两个模板分别保存，首次注册模板也需核对。有效期、验证码长度与发送频率以项目配置为准。[邮箱验证码说明](https://supabase.com/docs/guides/auth/auth-email-passwordless)、[邮件模板说明](https://supabase.com/docs/guides/auth/auth-email-templates)

Supabase 的默认 SMTP 只适合项目成员邮箱的有限测试；在模板被锁定的项目中，它不能满足当前客户端所需的数字验证码流程。使用自定义 SMTP 后，发送额度和发件身份要求以所选服务为准；使用自有域名的服务还需按服务商要求验证域名。[默认服务限制](https://supabase.com/docs/guides/auth/auth-smtp)

先用测试邮箱验证发送和验码，再连接个人学习记录。不要通过关闭账号验证或开放表权限来绕过邮件配置问题。

## 4. 构建两端应用

两端必须使用**同一个 Project URL 和 publishable key**。在仓库根目录的构建终端中设置；以下值都是待替换的占位符：

```bash
export SITZFLEISCH_SUPABASE_URL='https://YOUR_PROJECT_REF.supabase.co'
export SITZFLEISCH_SUPABASE_PUBLISHABLE_KEY='sb_publishable_REPLACE_WITH_PROJECT_PUBLIC_KEY'

npm ci
node scripts/check-sync-db.mjs
```

配置由 Rust 构建时读取，改变它们后需重新构建。只有网页预览或在应用启动后修改终端环境变量，不会改变已经打包的客户端。

Mac：

```bash
npm run tauri build
```

Android 环境、签名与升级约定沿用 [Android 开发说明](development.md#android-开发)。先生成与正式应用隔离的调试 APK：

```bash
npm run tauri android build -- --debug --apk --target aarch64
```

通过真机测试后，在同一套公开配置下构建发布 APK：

```bash
npm run tauri android build -- --apk --target aarch64 --ci
```

发布包需要既有签名配置；不要为升级已有应用临时换签名。覆盖安装保留数据，卸载会删除手机的本机存档。

### 用既有 GitHub Actions 签名 Android

本机没有发布签名口令、但仓库原有 `ANDROID_KEY_BASE64`、`ANDROID_KEY_ALIAS`、`ANDROID_KEY_PASSWORD` 三项 Actions Secrets 仍齐全时，可以让已有 `build.yml` 工作流使用它们签名。无需取回、查看或打印 Secret 值；签名使用原有证书，才能覆盖安装手机上的正式版本。

手动入口提供三个参数：

| 参数 | 默认值 | 用途 |
| --- | --- | --- |
| `android_only` | `false` | `true` 时跳过 Mac/Windows，只构建并签名 Android；缺签名配置直接失败，不回退为 debug |
| `supabase_url` | 空 | 编译进客户端的 HTTPS 项目地址 |
| `supabase_publishable_key` | 空 | 对应项目的 publishable/anon 公开密钥；禁止 secret/service_role |

地址与公开密钥必须同时填写或同时留空，先校验格式和长度，再启动构建。输入经环境变量传入，不拼接到 shell 命令。两项留空继续构建单机版本；Android 专用构建也可以留空，但仍要求原有签名配置完整。

需要先把待构建代码和工作流变更提交到目标分支并推送。工作流本身必须已存在于默认分支，随后可通过 `--ref` 指定其他分支运行；`ref` 与 `inputs` 也可以作为 REST 请求参数传入。[GitHub 手动运行说明](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/manually-run-a-workflow)、[workflow dispatch API](https://docs.github.com/en/rest/actions/workflows#create-a-workflow-dispatch-event)

以下仅为参数示例，替换分支名和公开配置后才执行：

```bash
gh workflow run build.yml --repo snooze26h/sitzfleisch --ref YOUR_REVIEWED_BRANCH \
  -f android_only=true \
  -f supabase_url='https://YOUR_PROJECT_REF.supabase.co' \
  -f supabase_publishable_key='sb_publishable_REPLACE_WITH_PROJECT_PUBLIC_KEY'
```

运行成功后，从该次运行的 **Artifacts → sitzfleisch-Android** 下载 APK 与 `.sha256` 文件，核对包签名和版本后再覆盖安装。手动运行（包括分支 dispatch）只生成 artifact，不发布 GitHub Release；原有推送 `v*` 标签的 Mac/Windows/Android 构建和发布路径保持。公开配置会进入应用构建产物，不能用手动输入传递任何私密凭据。

## 5. 首次连接与旧数据迁移

在两端设置页使用同一邮箱登录。首次接入账号时，先核对本机与云端的项目、历史和未结束学习日，再选择迁移方式；不要直接复制或用同步盘覆盖 `state.json`。

项目、计划、当前学习日、专注记录和完成内容属于共享数据。通知权限、提醒设置、提示音、网站/应用屏蔽配置属于设备数据，不上传云端。共享模式按墙钟继续计时，Mac 合盖或安卓锁屏不会自动暂停。

账号存档与原单机存档分开保存。退出账号后回到原单机数据，不会把前一个账号的数据自动放到下一个账号里。发生冲突时，先保留恢复副本，再选择冲突内容；不同历史记录继续合并。

云端文档上限为 2 MiB（以数据库的 JSONB 文本序列化大小计算），共享历史最多 1000 个学习日、删除标记最多 10000 个。超过限制会拒绝该次写入并保留已有数据，不会自动删历史或放弃删除标记。遇到容量错误时先导出/备份，再安排协议升级或归档方案。

## 6. 验证与上线边界

仓库内的数据库测试无需云端账号：

```bash
node scripts/check-sync-db.mjs
```

它在 [PGlite](https://pglite.dev/docs/about) 的内存 PostgreSQL 中执行**同一份迁移 SQL**，只模拟 Supabase 的 `auth.users` 和 `auth.uid()` 身份上下文。测试包括：匿名拒绝、两账号隔离、直接读写拒绝、RLS 后备防护、版本冲突、请求幂等、输入边界、迁移重跑和账号删除级联清理。PGlite 是单连接引擎；同基线双请求检查证明 CAS 结果，不替代生产多连接锁竞争测试。

部署后，用合成记录进行双设备验收：

1. Mac 开始一格，手机读取同一格；在手机暂停、继续、延长和结束，核对 Mac 的计时、当天进度和历史。
2. 两端同时结束同一格，确认只结算一次；一端断网期间操作，恢复后检查待同步和冲突提示。
3. 两端都编辑同一份计划，确认不会静默覆盖；分别新增不同历史记录，确认合并保留。
4. 在一端删除历史，另一端重连后不能把已删除记录重新上传回来。
5. 重启应用后恢复未确认操作；退出并登录另一个测试账号，确认不会串数据。
6. Mac 合盖、手机锁屏以及手机回到前台后，检查计时、系统通知和闹钟是否更新；后台受系统省电限制时，恢复前台必须补拉。

本地测试不覆盖真实 Supabase Auth、SMTP 投递、API 网关和两端网络；这些需要项目配置完成后的实际验收。测试期间不读取、导入或上传个人正式记录。

## 故障定位

| 表现 | 核对位置 |
| --- | --- |
| 同步入口显示未配置 | 两个构建环境变量是否同时存在，是否重新构建 |
| 只有项目成员邮箱能收到验证码 | 自定义 SMTP 和发件域名验证 |
| 邮件模板是只读的，提示 Set up custom SMTP | 先完成 SMTP 发信服务配置，再编辑模板 |
| 邮件只有链接，没有数字验证码 | Magic Link 与 Confirm sign up 模板是否均包含 `{{ .Token }}` |
| RPC 不存在 | 迁移是否运行在客户端配置对应的项目，`public` 是否暴露给 Data API |
| 登录后提示权限不足 | RPC 的 authenticated 执行权限与用户会话；不要给私有表补写权限 |
| 同步冲突 | 两端是否离线编辑同一学习日/记录；先核对差异再选择 |
| 手机锁屏时没有立即更新 | 系统后台网络限制；回到前台观察补同步和通知修正 |

排查时只记录状态码、错误类别和操作编号；不要复制令牌、验证码、SMTP 密钥或完整学习记录到公开日志。
