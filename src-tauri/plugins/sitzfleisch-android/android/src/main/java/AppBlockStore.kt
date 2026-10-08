package com.snooze26h.sitzfleisch.android

import android.accessibilityservice.AccessibilityServiceInfo
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ResolveInfo
import android.app.Activity
import android.os.Build
import android.os.PowerManager
import android.os.Process
import android.os.SystemClock
import android.provider.Settings
import android.system.ErrnoException
import android.system.Os
import android.telecom.TelecomManager
import android.view.accessibility.AccessibilityManager
import java.io.File
import java.io.FileOutputStream
import java.io.IOException

/**
 * 应用屏蔽在原生侧的状态。
 *
 * 屏蔽服务跑在自己的进程（`:blocker`）里：划掉坐功时界面进程会退出，服务进程不受影响，
 * 屏蔽不留空档。所以两边只能通过文件和启动参数交换信息——
 * - 规则：界面进程把 Rust 推来的开关和包名整份写进文件（先写临时文件再改名），服务进程按文件
 *   修改时间与大小判断要不要重读。系统只为服务拉起进程、Rust 还没运行时也照样读得到。
 * - 「刚把谁送了回来」：服务在打开坐功的启动参数里带上，界面进程收到后记在内存里。
 */
object AppBlockStore {
  private const val RULES_FILE = "app_blocking_rules"
  /** 0.14.0 把规则存在这里；新版第一次读规则时迁移过来。 */
  private const val LEGACY_PREFS = "sitzfleisch_app_blocking"
  const val MAX_PACKAGES = 200
  const val EXTRA_BLOCKED_PACKAGE = "com.snooze26h.sitzfleisch.extra.BLOCKED_PACKAGE"
  const val EXTRA_SENT_BACK_AT = "com.snooze26h.sitzfleisch.extra.SENT_BACK_AT"
  /** 被送回坐功之后，界面在这段时间内回到前台才说明原因；再晚就是旧事了。 */
  private const val NOTICE_TTL_MS = 60_000L
  /** 送回坐功后这段时间内收到的「离开坐功」，是上一次离开迟到的回调，不算数。 */
  private const val LEAVE_GRACE_MS = 2_000L
  private val PACKAGE_NAME = Regex("[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z][A-Za-z0-9_]*)+")

  class Rules(val enabled: Boolean, val packages: Set<String>)

  /** 改名替换会换一个新的 inode：同一毫秒里写了两次、长度又相同，也认得出是新文件。 */
  private data class Stamp(val inode: Long, val modified: Long, val length: Long)

  private class Notice(val packageName: String, val at: Long)

  private val lock = Any()
  private var cached: Rules? = null
  private var cachedStamp: Stamp? = null
  @Volatile private var notice: Notice? = null
  /**
   * 屏蔽服务把坐功叫到前台的时刻（0 表示没有）。这时坐功下面可能还压着刚被拦下的应用：
   * 荣耀上实测，送回后在坐功里按返回只退到后台，坐功会又被送回前台。
   */
  @Volatile private var coveringSince = 0L

  fun validPackageName(name: String): Boolean = name.length <= 255 && PACKAGE_NAME.matches(name)

  private fun rulesFile(context: Context) = File(context.applicationContext.filesDir, RULES_FILE)

  private fun stampOf(file: File): Stamp {
    val inode = try { Os.stat(file.path).st_ino } catch (_: ErrnoException) { -1L }
    return Stamp(inode, file.lastModified(), file.length())
  }

  fun rules(context: Context): Rules {
    synchronized(lock) {
      val file = rulesFile(context)
      if (!file.exists()) return migrateLegacy(context)
      val stamp = stampOf(file)
      cached?.let { if (cachedStamp == stamp) return it }
      val lines = try { file.readLines() } catch (_: IOException) { return cached ?: Rules(false, emptySet()) }
      val rules = Rules(
        lines.firstOrNull() == "enabled",
        lines.drop(1).filter(::validPackageName).take(MAX_PACKAGES).toSet(),
      )
      cached = rules
      cachedStamp = stamp
      return rules
    }
  }

  /** 旧版的规则只在新文件还不存在时读一次，写成新文件后旧的就不再使用。 */
  private fun migrateLegacy(context: Context): Rules {
    val prefs = context.applicationContext.getSharedPreferences(LEGACY_PREFS, Context.MODE_PRIVATE)
    val rules = Rules(
      prefs.getBoolean("enabled", false),
      prefs.getStringSet("packages", null).orEmpty().filter(::validPackageName).take(MAX_PACKAGES).toSet(),
    )
    if (rules.enabled || rules.packages.isNotEmpty()) writeRules(context, rules)
    return rules
  }

  /** 整份写入：先写临时文件并落盘，再改名替换，另一个进程永远读不到写了一半的规则。 */
  private fun writeRules(context: Context, rules: Rules): Boolean {
    val file = rulesFile(context)
    // 两个进程都可能写（界面进程推规则、服务进程迁移旧规则）：临时文件按进程分开，互不改名对方写了一半的文件。
    val temp = File(file.parentFile, "$RULES_FILE.${Process.myPid()}.tmp")
    val text = buildString {
      append(if (rules.enabled) "enabled" else "disabled").append('\n')
      rules.packages.sorted().forEach { append(it).append('\n') }
    }
    return try {
      FileOutputStream(temp).use { output ->
        output.write(text.toByteArray(Charsets.UTF_8))
        output.fd.sync()
      }
      if (!temp.renameTo(file)) return false
      cached = rules
      cachedStamp = stampOf(file)
      true
    } catch (_: IOException) {
      temp.delete()
      false
    }
  }

  /** 写成功才算推送成功，Rust 那边据此决定要不要重试。 */
  fun save(context: Context, enabled: Boolean, packages: Set<String>): Boolean = synchronized(lock) {
    writeRules(context, Rules(enabled, packages.toSet()))
  }

  /**
   * 界面进程收到打开坐功的启动参数时调用：是屏蔽服务送回来的，就记下拦的是谁、坐功正压在它上面。
   * 主界面是导出的，别的应用也能带着这两个参数打开它，所以只认名单里的应用、只认一分钟内的。
   */
  fun acceptSentBack(context: Context, intent: Intent?) {
    if (intent == null || !intent.hasExtra(EXTRA_BLOCKED_PACKAGE)) return
    val packageName = intent.getStringExtra(EXTRA_BLOCKED_PACKAGE)
    val at = intent.getLongExtra(EXTRA_SENT_BACK_AT, -1L)
    // 读一次就去掉：界面重建时会拿到同一份启动参数，不能再提示一遍。
    intent.removeExtra(EXTRA_BLOCKED_PACKAGE)
    intent.removeExtra(EXTRA_SENT_BACK_AT)
    val age = SystemClock.elapsedRealtime() - at
    if (packageName == null || !validPackageName(packageName) || at <= 0L || age < 0L || age > NOTICE_TTL_MS) return
    if (packageName !in rules(context).packages) return
    notice = Notice(packageName, at)
    coveringSince = at
  }

  fun takeNotice(): String? {
    val current = notice ?: return null
    notice = null
    return current.packageName.takeIf { SystemClock.elapsedRealtime() - current.at <= NOTICE_TTL_MS }
  }

  /** 返回键用：坐功是不是正压在刚被拦下的应用上面。读一次就清掉。 */
  fun takeCoveringBlockedApp(): Boolean {
    val covering = coveringSince != 0L
    coveringSince = 0L
    return covering
  }

  /**
   * 坐功的界面不在前台了。只在亮屏时算「用户自己离开」（回桌面、切到别的应用），之后坐功下面是什么
   * 就不再确定；锁屏再解锁，坐功下面压着的还是刚被拦下的应用。
   */
  fun leftForeground(context: Context) {
    val power = context.getSystemService(Context.POWER_SERVICE) as? PowerManager
    if (power?.isInteractive != true) return
    val since = coveringSince
    if (since != 0L && SystemClock.elapsedRealtime() - since > LEAVE_GRACE_MS) coveringSince = 0L
  }

  /**
   * 根页面按返回时调用：刚被屏蔽服务送回来的话直接回桌面并返回 true——只退到后台，坐功会又被送回前台
   * （荣耀实测）。否则返回 false，由调用方照常退到后台。
   */
  fun goHomeIfCovering(activity: Activity): Boolean {
    if (!takeCoveringBlockedApp()) return false
    return try {
      activity.startActivity(Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
      true
    } catch (_: RuntimeException) {
      false
    }
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
      ?: return false
    return enabled.split(':').any { ComponentName.unflattenFromString(it) == own }
  }

  /**
   * 系统此刻真的连着坐功的屏蔽服务。开关开着、服务却断了（比如升级时进程被换掉，系统把它记成出错）
   * 时，设置里仍显示开启，只有这里看得出来。服务在另一个进程，只能问系统。
   */
  fun serviceRunning(context: Context): Boolean {
    val manager = context.getSystemService(Context.ACCESSIBILITY_SERVICE) as? AccessibilityManager ?: return false
    val own = ComponentName(context, AppBlockService::class.java)
    return manager.getEnabledAccessibilityServiceList(AccessibilityServiceInfo.FEEDBACK_ALL_MASK).any {
      val info = it.resolveInfo?.serviceInfo
      info != null && ComponentName(info.packageName, info.name) == own
    }
  }
}
