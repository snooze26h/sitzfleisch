#[cfg(target_os = "android")]
use tauri::{plugin::{Builder, TauriPlugin}, Manager, Runtime};

#[cfg(target_os = "android")]
mod mobile;
#[cfg(target_os = "android")]
pub use mobile::SitzfleischAndroid;

#[cfg(target_os = "android")]
pub trait SitzfleischAndroidExt<R: Runtime> {
    fn sitzfleisch_android(&self) -> &SitzfleischAndroid<R>;
}

#[cfg(target_os = "android")]
impl<R: Runtime, T: Manager<R>> SitzfleischAndroidExt<R> for T {
    fn sitzfleisch_android(&self) -> &SitzfleischAndroid<R> {
        self.state::<SitzfleischAndroid<R>>().inner()
    }
}

#[cfg(target_os = "android")]
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("sitzfleisch-android")
        .setup(|app, api| {
            let handle = api.register_android_plugin(
                "com.snooze26h.sitzfleisch.android", "SitzfleischPlugin"
            )?;
            app.manage(SitzfleischAndroid::new(handle));
            Ok(())
        })
        .build()
}
