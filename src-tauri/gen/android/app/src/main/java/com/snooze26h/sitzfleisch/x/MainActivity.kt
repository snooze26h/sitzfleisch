package com.snooze26h.sitzfleisch.x

import android.graphics.Color
import android.os.Bundle
import android.view.View
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  // 与界面的 --bed 一致。窗口、内容根视图和 WebView 都先刷成这一色，启动和切回前台不闪白。
  private val bed = Color.rgb(0x14, 0x14, 0x11)

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
        moveTaskToBack(true)
      }
    })
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

  override fun onWebViewCreate(webView: WebView) {
    // 网页第一帧画出来之前 WebView 默认是白底；先刷成界面的底色。
    webView.setBackgroundColor(bed)
  }
}
