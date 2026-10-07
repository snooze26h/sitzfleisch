package com.snooze26h.sitzfleisch.android

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ResolveInfo
import android.os.Build
import android.os.SystemClock
import android.provider.Settings
import android.telecom.TelecomManager

/**
 * 应用屏蔽在原生侧的状态。规则由 Rust 推过来、落进 SharedPreferences：
 * 系统可能只为无障碍服务拉起进程，那时 Rust 还没运行，服务只能读这里。
 * 服务和界面跑在同一个进程里，所以内存里的缓存和「刚屏蔽了谁」两边都看得到。
 */
object AppBlockStore {
  private const val PREFS = "sitzfleisch_app_blocking"
  private const val KEY_ENABLED = "enabled"
  private const val KEY_PACKAGES = "packages"
  const val MAX_PACKAGES = 200
  /** 被送回坐功之后，界面在这段时间内回到前台才说明原因；再晚就是旧事了。 */
  private const val NOTICE_TTL_MS = 60_000L
  private val PACKAGE_NAME = Regex("[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z][A-Za-z0-9_]*)+")

  class Rules(val enabled: Boolean, val packages: Set<String>)

  private class Notice(val packageName: String, val at: Long)

  @Volatile private var cached: Rules? = null
  @Volatile private var notice: Notice? = null

  fun validPackageName(name: String): Boolean = name.length <= 255 && PACKAGE_NAME.matches(name)

  fun rules(context: Context): Rules {
    cached?.let { return it }
    val prefs = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    val packages = prefs.getStringSet(KEY_PACKAGES, null).orEmpty().filter(::validPackageName).toSet()
    return Rules(prefs.getBoolean(KEY_ENABLED, false), packages).also { cached = it }
  }

  /** 同步写盘：写成功才算推送成功，Rust 那边据此决定要不要重试。 */
  fun save(context: Context, enabled: Boolean, packages: Set<String>): Boolean {
    val saved = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
      .putBoolean(KEY_ENABLED, enabled)
      .putStringSet(KEY_PACKAGES, HashSet(packages))
      .commit()
    if (saved) cached = Rules(enabled, packages.toSet())
    return saved
  }

  fun recordNotice(packageName: String) {
    notice = Notice(packageName, SystemClock.elapsedRealtime())
  }

  fun takeNotice(): String? {
    val current = notice ?: return null
    notice = null
    return current.packageName.takeIf { SystemClock.elapsedRealtime() - current.at <= NOTICE_TTL_MS }
  }

  /** 坐功自己的包（正式版和测试版）都不进名单，也永远不拦。 */
  fun isOwnPackage(packageName: String): Boolean =
    packageName == "com.snooze26h.sitzfleisch.x" || packageName.startsWith("com.snooze26h.sitzfleisch.")

  /**
   * 永远不拦的应用：桌面、系统界面、设置和拨号。屏蔽它们会让人关不掉屏蔽、卸载不了，
   * 或者打不了紧急电话。选择器里也不列出它们。
   */
  fun protectedPackages(context: Context): Set<String> {
    val pm = context.packageManager
    val result = mutableSetOf("android", "com.android.systemui", context.packageName)
    for (intent in listOf(
      Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME),
      Intent(Settings.ACTION_SETTINGS),
      Intent(Intent.ACTION_DIAL),
    )) {
      queryActivities(pm, intent).mapNotNullTo(result) { it.activityInfo?.packageName }
    }
    try {
      (context.getSystemService(Context.TELECOM_SERVICE) as? TelecomManager)?.defaultDialerPackage?.let(result::add)
    } catch (_: SecurityException) {
      // 个别系统要求额外权限才能读默认拨号应用；上面按拨号意图解析到的已经算进去了。
    }
    return result
  }

  fun isProtected(context: Context, packageName: String): Boolean =
    isOwnPackage(packageName) || packageName in protectedPackages(context)

  fun queryActivities(pm: PackageManager, intent: Intent): List<ResolveInfo> =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      pm.queryIntentActivities(intent, PackageManager.ResolveInfoFlags.of(0))
    } else {
      @Suppress("DEPRECATION")
      pm.queryIntentActivities(intent, 0)
    }

  /** 系统设置里打开的无障碍服务列表有没有坐功这一项（以用户的开关为准）。 */
  fun serviceEnabled(context: Context): Boolean {
    val own = ComponentName(context, AppBlockService::class.java)
    val enabled = Settings.Secure.getString(context.contentResolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES)
      ?: return AppBlockService.connected
    return enabled.split(':').any { ComponentName.unflattenFromString(it) == own } || AppBlockService.connected
  }
}
