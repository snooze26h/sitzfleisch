fn main() {
    // 没有供 WebView 调用的插件命令，原生桥只接受 Rust 的 run_mobile_plugin。
    tauri_plugin::Builder::new(&[]).android_path("android").build();
}
