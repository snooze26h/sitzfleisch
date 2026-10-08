package com.snooze26h.sitzfleisch.android

import android.accessibilityservice.AccessibilityService
import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.SystemClock
import android.view.accessibility.AccessibilityEvent

/**
 * 应用屏蔽：名单里的应用一到前台，就把人送回坐功。
 *
 * 只订阅「窗口切换」这一类事件，只读事件里的包名和界面类名；不读取屏幕内容
 * （配置里 canRetrieveWindowContent=false），不代替用户点按，也不联网。
 * 开关和名单由坐功界面决定，这里只照着 [AppBlockStore] 里的规则执行。
 *
 * 服务在独立进程（`:blocker`）里运行：划掉坐功时界面进程按设计退出，这个进程不跟着退，屏蔽不断档。
 */
class AppBlockService : AccessibilityService() {
  companion object {
    /** 同一个应用连着发好几条窗口事件时，只送回一次。 */
    private const val REPEAT_WINDOW_MS = 1_500L
    private const val CACHE_LIMIT = 256
  }

  private var lastPackage: String? = null
  private var lastBlockedAt = 0L
  private val windowKinds = HashMap<String, Boolean>()

  override fun onInterrupt() {}

  override fun onAccessibilityEvent(event: AccessibilityEvent?) {
    if (event?.eventType != AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED) return
    val packageName = event.packageName?.toString() ?: return
    // 别的应用（桌面、坐功自己……）的窗口一出现，上一次拦截就算结束：用户马上又打开同一个应用，也要再拦。
    // 去重只用来合并同一次打开连着发来的几条窗口事件。
    if (packageName != lastPackage) lastPackage = null
    val rules = AppBlockStore.rules(this)
    if (!rules.enabled || packageName !in rules.packages) return
    if (!isAppScreen(packageName, event.className?.toString())) return
    // 名单在入库前已经排除了这些应用；这里再挡一次，防止存档被改坏后把人锁在外面。
    if (AppBlockStore.isProtected(this, packageName)) return
    val now = SystemClock.elapsedRealtime()
    if (packageName == lastPackage && now - lastBlockedAt < REPEAT_WINDOW_MS) return
    lastPackage = packageName
    lastBlockedAt = now
    sendBack(packageName)
  }

  /**
   * 只认「这个应用的某个界面到了前台」。悬浮窗、输入法、浮层提示也会发窗口事件，
   * 不能因为它们把人从别的应用里拽走。应用对坐功不可见、无从核对时，按界面处理。
   */
  private fun isAppScreen(packageName: String, className: String?): Boolean {
    if (className.isNullOrEmpty()) return !packageVisible(packageName)
    val key = "$packageName/$className"
    windowKinds[key]?.let { return it }
    val screen = try {
      val component = ComponentName(packageName, className)
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        packageManager.getActivityInfo(component, PackageManager.ComponentInfoFlags.of(0))
      } else {
        @Suppress("DEPRECATION")
        packageManager.getActivityInfo(component, 0)
      }
      true
    } catch (_: PackageManager.NameNotFoundException) {
      !packageVisible(packageName)
    }
    if (windowKinds.size >= CACHE_LIMIT) windowKinds.clear()
    windowKinds[key] = screen
    return screen
  }

  private fun packageVisible(packageName: String): Boolean = try {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      packageManager.getApplicationInfo(packageName, PackageManager.ApplicationInfoFlags.of(0))
    } else {
      @Suppress("DEPRECATION")
      packageManager.getApplicationInfo(packageName, 0)
    }
    true
  } catch (_: PackageManager.NameNotFoundException) {
    false
  }

  /**
   * 先把桌面叫到前台、再把坐功放在它上面，免得在坐功里按返回又露出刚才那个应用。有的系统（荣耀）
   * 上光靠这一步不够，所以返回键另外照 [AppBlockStore.takeCoveringBlockedApp] 直接回桌面。
   * 无障碍服务由系统绑定，允许从后台打开界面。
   */
  private fun sendBack(packageName: String) {
    val home = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME)
      .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    // 拦的是谁、什么时候拦的，随启动参数交给界面进程：界面据此提示，并让返回键直接回桌面。
    val zuogong = packageManager.getLaunchIntentForPackage(this.packageName)
      ?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
      ?.putExtra(AppBlockStore.EXTRA_BLOCKED_PACKAGE, packageName)
      ?.putExtra(AppBlockStore.EXTRA_SENT_BACK_AT, SystemClock.elapsedRealtime())
    try {
      if (zuogong != null) startActivities(arrayOf(home, zuogong)) else startActivity(home)
    } catch (_: RuntimeException) {
      // 系统不许从这里打开界面时，至少回到桌面。
      performGlobalAction(GLOBAL_ACTION_HOME)
    }
  }
}
