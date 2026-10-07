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
 */
class AppBlockService : AccessibilityService() {
  companion object {
    /** 系统确实绑定着这个服务。界面据此和系统开关一起判断屏蔽能不能生效。 */
    @Volatile var connected = false
      private set

    /** 同一个应用连着发好几条窗口事件时，只送回一次。 */
    private const val REPEAT_WINDOW_MS = 1_500L
    private const val CACHE_LIMIT = 256
  }

  private var lastPackage: String? = null
  private var lastBlockedAt = 0L
  private val windowKinds = HashMap<String, Boolean>()

  override fun onServiceConnected() {
    super.onServiceConnected()
    connected = true
  }

  override fun onUnbind(intent: Intent?): Boolean {
    connected = false
    return super.onUnbind(intent)
  }

  override fun onDestroy() {
    connected = false
    super.onDestroy()
  }

  override fun onInterrupt() {}

  override fun onAccessibilityEvent(event: AccessibilityEvent?) {
    if (event?.eventType != AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED) return
    val packageName = event.packageName?.toString() ?: return
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
    AppBlockStore.recordSentBack(packageName)
    val home = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME)
      .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    val zuogong = packageManager.getLaunchIntentForPackage(this.packageName)
      ?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    try {
      if (zuogong != null) startActivities(arrayOf(home, zuogong)) else startActivity(home)
    } catch (_: RuntimeException) {
      // 系统不许从这里打开界面时，至少回到桌面。
      performGlobalAction(GLOBAL_ACTION_HOME)
    }
  }
}
