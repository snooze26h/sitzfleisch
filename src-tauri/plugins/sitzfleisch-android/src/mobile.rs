use serde::{de::DeserializeOwned, Serialize};
use tauri::{plugin::{mobile::PluginInvokeError, PluginHandle}, Runtime};

/// 参数和响应由外壳的共同契约定型，避免为了桌面检查引入 Android 目标依赖。
pub struct SitzfleischAndroid<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> SitzfleischAndroid<R> {
    pub(crate) fn new(handle: PluginHandle<R>) -> Self { Self(handle) }

    pub fn update_status(&self, payload: impl Serialize) -> Result<(), PluginInvokeError> {
        self.0.run_mobile_plugin("updateStatus", payload)
    }

    pub fn system_status<T: DeserializeOwned>(&self) -> Result<T, PluginInvokeError> {
        self.0.run_mobile_plugin("systemStatus", ())
    }

    pub fn open_settings(&self, payload: impl Serialize) -> Result<(), PluginInvokeError> {
        self.0.run_mobile_plugin("openSettings", payload)
    }

    pub fn move_task_to_back(&self) -> Result<(), PluginInvokeError> {
        self.0.run_mobile_plugin("moveTaskToBack", ())
    }

    pub fn set_block_rules(&self, payload: impl Serialize) -> Result<(), PluginInvokeError> {
        self.0.run_mobile_plugin("setBlockRules", payload)
    }

    pub fn installed_apps<T: DeserializeOwned>(&self) -> Result<T, PluginInvokeError> {
        self.0.run_mobile_plugin("installedApps", ())
    }

    pub fn request_app_list_permission<T: DeserializeOwned>(&self) -> Result<T, PluginInvokeError> {
        self.0.run_mobile_plugin("requestAppListPermission", ())
    }

    pub fn take_block_notice<T: DeserializeOwned>(&self) -> Result<T, PluginInvokeError> {
        self.0.run_mobile_plugin("takeBlockNotice", ())
    }
}
