package com.snooze26h.sitzfleisch.x

import android.content.Intent
import android.content.res.Configuration
import android.graphics.Color
import android.os.Bundle
import android.view.View
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import com.snooze26h.sitzfleisch.android.AppBlockStore
import kotlin.math.roundToInt

class MainActivity : TauriActivity() {
  // 与界面的 --bed 一致。窗口、内容根视图和 WebView 都先刷成这一色，启动和切回前台不闪白。
  private val bed = Color.rgb(0x14, 0x14, 0x11)
  private var webView: WebView? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    // 坐功只有深色界面：系统栏固定用浅色图标、透明底。不用随系统深浅切换的默认样式，
    // 否则 Android 8–9 的浅色系统会把导航栏涂白，和白色按键混在一起。
    enableEdgeToEdge(
      statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
      navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
    )
    // 返回键兜底：在 Tauri 的返回键回调之前注册，所以优先级最低。界面注册了返回监听时由界面
    // 按层级退回；页面还没加载好或界面出错时，把应用退到后台，而不是结束 Activity（那会让进程退出）。
    onBackPressedDispatcher.addCallback(object : OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        if (!AppBlockStore.goHomeIfCovering(this@MainActivity)) moveTaskToBack(true)
      }
    })
    // 应用屏蔽把人送回来时，启动参数里带着拦下的是谁。在这里收，不等插件注册：
    // 进程被回收后重建界面时，新的启动参数会先于插件注册送到。
    AppBlockStore.acceptSentBack(this, intent)
    super.onCreate(savedInstanceState)

    val content = findViewById<View>(android.R.id.content)
    content.setBackgroundColor(bed)
    // 原生层统一让出系统栏、挖孔与键盘空间，避免依赖各厂商 WebView 的 CSS env() 支持。
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
      )
      val keyboard = insets.getInsets(WindowInsetsCompat.Type.ime())
      view.setPadding(bars.left, bars.top, bars.right, maxOf(bars.bottom, keyboard.bottom))
      // 根视图已应用边距，不再让子视图重复添加同一份空间。
      WindowInsetsCompat.CONSUMED
    }
    ViewCompat.requestApplyInsets(content)
  }

  override fun onNewIntent(intent: Intent) {
    AppBlockStore.acceptSentBack(this, intent)
    super.onNewIntent(intent)
  }

  override fun onStop() {
    super.onStop()
    AppBlockStore.leftForeground(this)
  }

  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    // 网页第一帧画出来之前 WebView 默认是白底；先刷成界面的底色。
    webView.setBackgroundColor(bed)
  }

  /**
   * 字体大小、显示大小等改了不再重建界面（清单里 configChanges 都接了下来）：重建会让 Tauri 的插件
   * 和返回键回调还绑着已销毁的旧界面。WebView 只在创建时按系统字号定一次缩放，这里跟上新字号。
   */
  override fun onConfigurationChanged(newConfig: Configuration) {
    super.onConfigurationChanged(newConfig)
    webView?.settings?.textZoom = (newConfig.fontScale * 100).roundToInt()
  }
}
