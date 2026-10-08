// 坐功的界面需要 Chromium 99 及以上的系统 WebView（用到 structuredClone 与 canvas 的 roundRect）。
// 手机上 WebView 太旧时，主程序要么解析失败直接白屏，要么用到一半出错；这里用最老的写法先查一遍，
// 不合格就把原因和办法写在页面上，并标记给主程序不要启动。只在 Android 上检查，桌面保持原样。
(function () {
  if (!/Android/i.test(navigator.userAgent)) return;
  var canvas = window.CanvasRenderingContext2D;
  var supported = typeof window.structuredClone === "function"
    && !!canvas && typeof canvas.prototype.roundRect === "function";
  if (supported) return;
  var app = document.getElementById("app");
  if (!app) return;
  var match = /Chrome\/(\d+)/.exec(navigator.userAgent);
  app.setAttribute("data-unsupported", "");
  app.innerHTML = '<main class="unsupported"><h1>需要更新系统 WebView</h1>'
    + '<p>坐功的界面由系统 WebView 显示，需要 Chromium 99 或更新的版本'
    + (match ? '，这台手机上是 ' + match[1] : '') + '。</p>'
    + '<p>请在应用商店或系统更新里更新「Android System WebView」，然后重新打开坐功。已有的记录不受影响。</p></main>';
})();
