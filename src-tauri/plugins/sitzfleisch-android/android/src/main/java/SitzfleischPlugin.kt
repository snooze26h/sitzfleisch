package com.snooze26h.sitzfleisch.android

import android.app.Activity
import android.app.ActivityManager
import android.app.AlarmManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ActivityNotFoundException
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import android.webkit.WebView
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONArray
import org.json.JSONObject

/** 荣耀、小米等系统自带的「获取应用列表」权限；没给时系统可能只交出一部分应用。 */
private const val APP_LIST_PERMISSION = "com.android.permission.GET_INSTALLED_APPS"
private const val MAX_APP_LABEL_CHARS = 80

@InvokeArg
class StatusArgs {
  var visible: Boolean = false
  var title: String = ""
  var text: String = ""
  var chronometerBaseMs: Long? = null
  var countDown: Boolean = false
  var timeoutAfterMs: Long? = null
}

@InvokeArg
class SettingsArgs {
  var target: String = ""
  var channelId: String? = null
}

@TauriPlugin(permissions = [Permission(strings = [APP_LIST_PERMISSION], alias = "appList")])
class SitzfleischPlugin(private val activity: Activity) : Plugin(activity) {
  private val manager = activity.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
  private val channelIds = listOf("timer", "body", "water", "status")

  init {
    // 插件一构造就建好渠道：心跳可能在页面加载完之前就要发常驻通知或预排提醒。
    ensureChannels()
  }

  override fun load(webView: WebView) {
    super.load(webView)
    ensureChannels()
    // 冷启动：坐功可能正是被屏蔽服务叫起来的。
    AppBlockStore.acceptSentBack(activity, activity.intent)
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    AppBlockStore.acceptSentBack(activity, intent)
  }

  /** 重建同 ID 的渠道会保留用户在系统里选的声音、开关和重要性，所以可以放心重复调用。 */
  private fun ensureChannels() {
    val attributes = AudioAttributes.Builder()
      .setUsage(AudioAttributes.USAGE_NOTIFICATION)
      .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
      .build()
    fun reminder(id: String, name: String) =
      NotificationChannel(id, name, NotificationManager.IMPORTANCE_HIGH).apply {
        enableVibration(true)
        setSound(Settings.System.DEFAULT_NOTIFICATION_URI, attributes)
      }
    val water = reminder("water", "喝水").apply {
      // 使用资源名，不把会在升级时变动的数字资源 ID 写进持久化渠道。
      setSound(Uri.parse("android.resource://${activity.packageName}/raw/water"), attributes)
    }
    val status = NotificationChannel("status", "进行中", NotificationManager.IMPORTANCE_LOW).apply {
      enableVibration(false)
      setSound(null, null)
    }
    manager.createNotificationChannels(listOf(
      reminder("timer", "计时"), reminder("body", "身体提醒"), water, status
    ))
  }

  private fun nullableInteger(args: JSObject, key: String, maximum: Long): Boolean {
    val value = args.opt(key)
    if (value == null || value == JSONObject.NULL) return true
    val number = when (value) {
      is Int -> value.toLong()
      is Long -> value
      else -> return false
    }
    return number in 0..maximum
  }

  private fun statusArgs(invoke: Invoke): StatusArgs {
    require(invoke.getRawArgs().length <= 4096)
    val raw = invoke.getArgs()
    require(raw.opt("visible") is Boolean && raw.opt("countDown") is Boolean)
    require(raw.opt("title") is String && raw.opt("text") is String)
    require(nullableInteger(raw, "chronometerBaseMs", 253402300799000L))
    require(nullableInteger(raw, "timeoutAfterMs", 86400000L))
    val args = invoke.parseArgs(StatusArgs::class.java)
    require(args.title.length <= 160 && args.text.length <= 320)
    require(!args.title.contains('\u0000') && !args.text.contains('\u0000'))
    require(!args.visible || args.title.isNotBlank())
    require(args.timeoutAfterMs == null || args.timeoutAfterMs!! > 0)
    require(!args.countDown || args.chronometerBaseMs != null)
    return args
  }

  @Command
  fun updateStatus(invoke: Invoke) {
    val args = try { statusArgs(invoke) } catch (_: Exception) {
      invoke.reject("常驻通知参数无效。")
      return
    }
    if (!args.visible) {
      manager.cancel(9000)
      invoke.resolve()
      return
    }
    if (!NotificationManagerCompat.from(activity).areNotificationsEnabled()) {
      invoke.reject("通知未开启，请先在系统设置中允许坐功通知。")
      return
    }
    ensureChannels()
    val icon = activity.resources.getIdentifier("ic_stat_zuogong", "drawable", activity.packageName)
    if (icon == 0) {
      invoke.reject("通知小图标缺失。")
      return
    }
    val launch = activity.packageManager.getLaunchIntentForPackage(activity.packageName)
    if (launch == null) {
      invoke.reject("无法打开坐功主界面。")
      return
    }
    launch.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
    val content = PendingIntent.getActivity(
      activity, 9000, launch, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
    )
    val builder = NotificationCompat.Builder(activity, "status")
      .setSmallIcon(icon)
      .setContentTitle(args.title)
      .setContentText(args.text)
      .setContentIntent(content)
      .setOngoing(true)
      .setOnlyAlertOnce(true)
      .setSilent(true)
      .setShowWhen(false)
    args.chronometerBaseMs?.let {
      // NotificationCompat 的 when 使用墙钟毫秒，系统负责换算并走秒。
      builder.setWhen(it).setUsesChronometer(true).setChronometerCountDown(args.countDown)
    }
    args.timeoutAfterMs?.let { builder.setTimeoutAfter(it) }
    try {
      manager.notify(9000, builder.build())
      invoke.resolve()
    } catch (_: SecurityException) {
      invoke.reject("系统未允许坐功发送通知。")
    }
  }

  @Command
  fun systemStatus(invoke: Invoke) {
    val channels = JSArray()
    for (id in channelIds) {
      val channel = manager.getNotificationChannel(id) ?: continue
      val item = JSObject()
      item.put("id", channel.id)
      item.put("name", channel.name.toString())
      item.put("enabled", channel.importance != NotificationManager.IMPORTANCE_NONE)
      item.put("importance", channel.importance)
      item.put("vibration", channel.shouldVibrate())
      item.put("sound", channel.sound?.toString() ?: JSONObject.NULL)
      channels.put(item)
    }
    val alarmManager = activity.getSystemService(Context.ALARM_SERVICE) as AlarmManager
    val powerManager = activity.getSystemService(Context.POWER_SERVICE) as PowerManager
    val result = JSObject()
    result.put("sdkInt", Build.VERSION.SDK_INT)
    result.put("manufacturer", Build.MANUFACTURER)
    result.put("notificationsEnabled", NotificationManagerCompat.from(activity).areNotificationsEnabled())
    result.put("channels", channels)
    result.put("canScheduleExactAlarms",
      Build.VERSION.SDK_INT < Build.VERSION_CODES.S || alarmManager.canScheduleExactAlarms())
    result.put("ignoringBatteryOptimizations", powerManager.isIgnoringBatteryOptimizations(activity.packageName))
    result.put("appBlockServiceEnabled", AppBlockStore.serviceEnabled(activity))
    result.put("appBlockServiceRunning", AppBlockStore.serviceRunning(activity))
    // 「后台活动」没被允许（荣耀「应用启动管理」里关着就是这样）：划掉坐功时系统会强行停止它，
    // 预排的提醒和无障碍服务都会被一起清掉。
    val activityManager = activity.getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager
    result.put("backgroundRestricted",
      Build.VERSION.SDK_INT >= Build.VERSION_CODES.P && activityManager.isBackgroundRestricted)
    invoke.resolve(result)
  }

  private fun settingsArgs(invoke: Invoke): SettingsArgs {
    require(invoke.getRawArgs().length <= 256)
    val raw = invoke.getArgs()
    require(raw.opt("target") is String)
    val channelId = raw.opt("channelId")
    require(channelId == null || channelId == JSONObject.NULL || channelId is String)
    val args = invoke.parseArgs(SettingsArgs::class.java)
    require(args.target in setOf("app_notifications", "channel", "exact_alarm", "battery", "app_details", "accessibility", "startup"))
    if (args.target == "channel") require(args.channelId in channelIds)
    else require(args.channelId == null)
    return args
  }

  @Command
  fun openSettings(invoke: Invoke) {
    val args = try { settingsArgs(invoke) } catch (_: Exception) {
      invoke.reject("系统设置目标或通知渠道无效。")
      return
    }
    val details = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS)
      .setData(Uri.parse("package:${activity.packageName}"))
    if (args.target == "startup") {
      openStartupManager(invoke, details)
      return
    }
    val intent = when (args.target) {
      "app_notifications" -> Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
        .putExtra(Settings.EXTRA_APP_PACKAGE, activity.packageName)
      "channel" -> Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS)
        .putExtra(Settings.EXTRA_APP_PACKAGE, activity.packageName)
        .putExtra(Settings.EXTRA_CHANNEL_ID, args.channelId)
      "exact_alarm" -> {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
          invoke.reject("当前系统版本无需单独设置精确闹钟权限。")
          return
        }
        Intent(Settings.ACTION_REQUEST_SCHEDULE_EXACT_ALARM)
          .setData(Uri.parse("package:${activity.packageName}"))
      }
      "battery" -> Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)
      // 系统不让普通应用直接跳到某个无障碍服务的详情页，只能打开无障碍设置首页。
      "accessibility" -> Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)
      else -> details
    }
    try {
      activity.startActivity(intent)
      invoke.resolve()
    } catch (_: ActivityNotFoundException) {
      // 厂商删掉标准设置入口时，仍提供可用的应用详情入口。
      try {
        activity.startActivity(details)
        invoke.resolve()
      } catch (_: Exception) {
        invoke.reject("此设备没有可用的应用设置入口。")
      }
    } catch (_: SecurityException) {
      invoke.reject("系统拒绝打开这个设置入口。")
    }
  }

  override fun onStop() {
    super.onStop()
    // 只在亮屏时算「离开」：锁屏再解锁，坐功下面压着的还是刚被拦下的应用。
    val power = activity.getSystemService(Context.POWER_SERVICE) as PowerManager
    if (power.isInteractive) AppBlockStore.leftForeground()
  }

  /**
   * 荣耀（以及华为）的「应用启动管理」。自动管理下，从最近任务里划掉坐功会被系统强行停止，
   * 无障碍服务和预排提醒都随之失效；改成手动管理才能保住。只打开不需要额外权限的那个公开页面，
   * 找不到就退到应用详情。
   */
  private fun openStartupManager(invoke: Invoke, fallback: Intent) {
    val candidates = listOf(
      ComponentName("com.hihonor.systemmanager", "com.hihonor.systemmanager.startupmgr.ui.StartupNormalAppListActivity"),
      ComponentName("com.huawei.systemmanager", "com.huawei.systemmanager.startupmgr.ui.StartupNormalAppListActivity"),
    )
    for (component in candidates) {
      try {
        activity.startActivity(Intent().setComponent(component))
        invoke.resolve()
        return
      } catch (_: ActivityNotFoundException) {
        // 这台手机没有这个入口，换下一个。
      } catch (_: SecurityException) {
        // 系统不许直接打开，换下一个。
      }
    }
    try {
      activity.startActivity(fallback)
      invoke.resolve()
    } catch (_: Exception) {
      invoke.reject("此设备没有可用的后台管理入口。")
    }
  }

  @Command
  fun moveTaskToBack(invoke: Invoke) {
    if (AppBlockStore.takeCoveringBlockedApp()) {
      // 屏蔽服务刚送回来的：只退到后台的话，坐功会又被送回前台（荣耀实测），所以直接回到桌面。
      val home = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME)
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
      try {
        activity.startActivity(home)
        invoke.resolve()
        return
      } catch (_: RuntimeException) {
        // 叫不出桌面时，退回到普通的退到后台。
      }
    }
    if (activity.moveTaskToBack(true)) invoke.resolve()
    else invoke.reject("无法把当前任务移到后台。")
  }

  // ---------- 应用屏蔽 ----------

  private fun blockRules(invoke: Invoke): Pair<Boolean, Set<String>> {
    require(invoke.getRawArgs().length <= 64 * 1024)
    val raw = invoke.getArgs()
    val enabled = raw.opt("enabled") as? Boolean ?: throw IllegalArgumentException("enabled")
    val list = raw.opt("packages") as? JSONArray ?: throw IllegalArgumentException("packages")
    require(list.length() <= AppBlockStore.MAX_PACKAGES)
    val packages = (0 until list.length()).map { list.opt(it) as? String ?: throw IllegalArgumentException("package") }.toSet()
    require(packages.all(AppBlockStore::validPackageName))
    return enabled to packages
  }

  /** Rust 每次在规则变化时推一次；写进本地存储，屏蔽服务从那里读。 */
  @Command
  fun setBlockRules(invoke: Invoke) {
    val (enabled, packages) = try { blockRules(invoke) } catch (_: Exception) {
      invoke.reject("应用屏蔽规则无效。")
      return
    }
    if (AppBlockStore.save(activity, enabled, packages)) invoke.resolve()
    else invoke.reject("应用屏蔽规则写入失败，将稍后重试。")
  }

  private fun appListPermissionDefined(): Boolean = try {
    activity.packageManager.getPermissionInfo(APP_LIST_PERMISSION, 0)
    true
  } catch (_: PackageManager.NameNotFoundException) {
    false
  }

  private fun appListGranted(): Boolean =
    ContextCompat.checkSelfPermission(activity, APP_LIST_PERMISSION) == PackageManager.PERMISSION_GRANTED

  /** 应用名来自各个应用自己，只留可显示的一行字；按字符截断，不把表情等拆成半个。 */
  private fun cleanLabel(label: CharSequence?, packageName: String): String {
    val text = label?.toString().orEmpty()
      .map { if (Character.isISOControl(it)) ' ' else it }.joinToString("")
      .replace(Regex("\\s+"), " ").trim()
    val end = text.offsetByCodePoints(0, minOf(MAX_APP_LABEL_CHARS, text.codePointCount(0, text.length)))
    return text.substring(0, end).trim().ifEmpty { packageName }
  }

  /** 选择器用：能从桌面打开的应用，去掉坐功自己和永远不拦的系统应用。 */
  @Command
  fun installedApps(invoke: Invoke) {
    val context = activity.applicationContext
    // 读一两百个应用名要逐个加载它们的资源，放到后台线程，不卡住界面。
    Thread {
      try {
        val pm = context.packageManager
        val protected = AppBlockStore.protectedPackages(context)
        val seen = HashSet<String>()
        val apps = JSArray()
        var userInstalled = 0
        val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
        for (info in AppBlockStore.queryActivities(pm, launcher)) {
          val app = info.activityInfo?.applicationInfo ?: continue
          val packageName = app.packageName
          if (!AppBlockStore.validPackageName(packageName) || AppBlockStore.isOwnPackage(packageName)
            || packageName in protected || !seen.add(packageName)) continue
          val item = JSObject()
          item.put("packageName", packageName)
          item.put("label", cleanLabel(app.loadLabel(pm), packageName))
          apps.put(item)
          if (app.flags and (ApplicationInfo.FLAG_SYSTEM or ApplicationInfo.FLAG_UPDATED_SYSTEM_APP) == 0) userInstalled++
          if (apps.length() >= 1000) break
        }
        val defined = appListPermissionDefined()
        val result = JSObject()
        result.put("apps", apps)
        // 有的系统没给这项权限也照样交出完整列表：一个自己装的应用都看不到，才算真被限制了。
        result.put("limited", defined && !appListGranted() && userInstalled == 0)
        result.put("canRequestFullList", defined)
        invoke.resolve(result)
      } catch (_: Exception) {
        invoke.reject("读不到手机上的应用列表。")
      }
    }.apply { name = "sitzfleisch-installed-apps" }.start()
  }

  /** 请求厂商的「获取应用列表」权限；没有这项权限的系统直接返回当前状态。 */
  @Command
  fun requestAppListPermission(invoke: Invoke) {
    if (!appListPermissionDefined() || appListGranted()) {
      resolveAppListPermission(invoke)
      return
    }
    requestPermissionForAlias("appList", invoke, "appListPermissionCallback")
  }

  @PermissionCallback
  private fun appListPermissionCallback(invoke: Invoke) {
    resolveAppListPermission(invoke)
  }

  private fun resolveAppListPermission(invoke: Invoke) {
    val result = JSObject()
    result.put("granted", !appListPermissionDefined() || appListGranted())
    invoke.resolve(result)
  }

  /** 屏蔽服务刚把人送回坐功时记下的那个应用；取一次就清掉。 */
  @Command
  fun takeBlockNotice(invoke: Invoke) {
    val result = JSObject()
    result.put("packageName", AppBlockStore.takeNotice() ?: JSONObject.NULL)
    invoke.resolve(result)
  }
}
