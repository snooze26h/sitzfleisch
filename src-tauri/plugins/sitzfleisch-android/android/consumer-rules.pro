# Rust 按类名注册插件，Invoke 参数由 Jackson 反射读取；R8 必须保留两处入口。
-keep class com.snooze26h.sitzfleisch.android.SitzfleischPlugin { *; }
-keep @app.tauri.annotation.InvokeArg class com.snooze26h.sitzfleisch.android.** { *; }
