package com.snooze26h.sitzfleisch.android

import android.app.Activity
import android.app.AlarmManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.media.AudioAttributes
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import android.webkit.WebView
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONObject

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

@TauriPlugin
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
    invoke.resolve(result)
  }

  private fun settingsArgs(invoke: Invoke): SettingsArgs {
    require(invoke.getRawArgs().length <= 256)
    val raw = invoke.getArgs()
    require(raw.opt("target") is String)
    val channelId = raw.opt("channelId")
    require(channelId == null || channelId == JSONObject.NULL || channelId is String)
    val args = invoke.parseArgs(SettingsArgs::class.java)
    require(args.target in setOf("app_notifications", "channel", "exact_alarm", "battery", "app_details"))
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

  @Command
  fun moveTaskToBack(invoke: Invoke) {
    if (activity.moveTaskToBack(true)) invoke.resolve()
    else invoke.reject("无法把当前任务移到后台。")
  }
}
