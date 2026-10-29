fn main() {
    println!("cargo:rerun-if-env-changed=SITZFLEISCH_SUPABASE_URL");
    println!("cargo:rerun-if-env-changed=SITZFLEISCH_SUPABASE_PUBLISHABLE_KEY");
    tauri_build::build()
}
