# 坐功 Android 本地插件

本插件只由 Rust 调用；没有 WebView 插件命令，不需要改 capabilities。

## 外壳接口

- `platform_info`：返回 `{ os, mobile, features }`，独立于每秒快照。features 为十个布尔值：`tray`、`website_blocking`、`browser_extension`、`autostart`、`reveal_state_file`、`window_title`、`quit_flow`、`in_app_sound_toggle`、`system_settings`、`exact_alarm_status`。Android 只开启后两项；桌面开启前八项。
- `system_status`：返回 `{ sdkInt, manufacturer, notificationsEnabled, channels, canScheduleExactAlarms, ignoringBatteryOptimizations }`。channels 是数组，每项为 `{ id, name, enabled, importance, vibration, sound }`；sound 可以为 null。
- `open_system_settings`：参数 `{ target, channelId? }`。target 仅接受 `app_notifications`、`channel`、`exact_alarm`、`battery`、`app_details`；只有 channel 可携带 channelId，且必须为 timer / body / water / status 之一。
- `move_task_to_back`：保留学习日，将任务移到后台。
- `notification_status`、`request_notification_permission`：保留 granted / denied / unknown 返回值；请求前先查询，已授权时直接返回。
- `test_notification` / `test_water_sound`：分别发 ID 9100 的 timer 通知、ID 9101 的 water 通知。

除不调用原生插件的 platform_info 外，以上 Tauri 命令均为 async。新系统接口在桌面明确返回不支持；原有桌面通知接口保持原先行为。

## 原生通知

启动时创建 timer / body / water / status 四个渠道。前三个 High 且允许振动；timer / body 用系统声音，water 使用命名资源 URI `android.resource://<应用包名>/raw/water`。status 为 Low、无声、无振动。已有渠道的用户设置由系统保留。

`SitzfleischPlugin.updateStatus` 参数为 `{ visible, title, text, chronometerBaseMs?, countDown, timeoutAfterMs? }`。通知 ID 固定 9000，单色半月小图标，点按打开应用，不带按钮，不设置强调色。PendingIntent 使用 FLAG_IMMUTABLE。chronometerBaseMs 是墙钟毫秒；timeoutAfterMs 是从投递起算的毫秒数。

Rust 的 StatusModel 保存固定计时终点或暂停起点，以整体相等去重；超时也保存绝对终点，投递前才换成 duration，避免每秒重发。仅心跳线程调用 updateStatus，调用前释放 Shared 锁；Activity 不可用时捕获 panic，不写入已应用缓存，下一轮重试。MobileShared 独立保存缓存与心跳唤醒句柄。

## 资源

半月小图标位于 `src-tauri/gen/android/app/src/main/res/drawable/ic_stat_zuogong.xml`，需按计划由用户目视确认。

喝水音由标准库脚本生成：

```bash
python3 src-tauri/plugins/sitzfleisch-android/scripts/generate_water.py
```

官方通知插件 2.4.0 的 Rust 初始化只接受空配置；不要向 `plugins.notification` 写入图标配置对象。测试通知以及后续 L1-3 的每条预排通知都应显式调用 `.icon("ic_stat_zuogong")`。

输出为 `src-tauri/gen/android/app/src/main/res/raw/water.wav`：单声道、16 位 PCM、44100 Hz、0.2 秒。脚本放在本插件内，以遵守并行模式线 ① 的目录边界；用户可替换 WAV。

## 当前范围

L1-1 提供平台与原生底座。计时仍沿用 A1 的策略，墙钟路径由 L1-2 接入；闹钟预排、差分和 alarms.json 由 L1-3 接入。通知权限说明和设置界面由线 ③ 完成。当前阶段不能作为锁屏提醒已经通过的证据。

依据：[Android 通知构建 API](https://developer.android.com/reference/androidx/core/app/NotificationCompat.Builder)、[系统设置 Intent](https://developer.android.com/reference/android/provider/Settings)、[通知渠道](https://developer.android.com/reference/android/app/NotificationChannel)。
