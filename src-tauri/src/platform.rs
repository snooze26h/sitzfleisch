use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct PlatformInfo {
    pub os: &'static str,
    pub mobile: bool,
    pub features: PlatformFeatures,
}

#[derive(Debug, Serialize)]
pub struct PlatformFeatures {
    pub tray: bool,
    pub website_blocking: bool,
    pub browser_extension: bool,
    pub autostart: bool,
    pub reveal_state_file: bool,
    pub window_title: bool,
    pub quit_flow: bool,
    pub in_app_sound_toggle: bool,
    pub system_settings: bool,
    pub exact_alarm_status: bool,
}

pub fn info() -> PlatformInfo {
    info_for(std::env::consts::OS)
}

fn info_for(os: &'static str) -> PlatformInfo {
    let mobile = matches!(os, "android" | "ios");
    PlatformInfo {
        os,
        mobile,
        features: PlatformFeatures {
            tray: !mobile,
            website_blocking: !mobile,
            browser_extension: !mobile,
            autostart: !mobile,
            reveal_state_file: !mobile,
            window_title: !mobile,
            quit_flow: !mobile,
            in_app_sound_toggle: !mobile,
            system_settings: os == "android",
            exact_alarm_status: os == "android",
        },
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SettingsTarget {
    AppNotifications,
    Channel,
    ExactAlarm,
    Battery,
    AppDetails,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsRequest {
    pub target: SettingsTarget,
    pub channel_id: Option<String>,
}

impl SettingsRequest {
    pub fn new(target: SettingsTarget, channel_id: Option<String>) -> Result<Self, String> {
        match (target, channel_id.as_deref()) {
            (SettingsTarget::Channel, Some("timer" | "body" | "water" | "status")) => (),
            (SettingsTarget::Channel, _) => return Err("请选择坐功的有效通知渠道。".into()),
            (_, Some(_)) => return Err("只有通知渠道设置可以指定渠道。".into()),
            (_, None) => (),
        }
        Ok(Self { target, channel_id })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatus {
    pub sdk_int: u32,
    pub manufacturer: String,
    pub notifications_enabled: bool,
    pub channels: Vec<ChannelStatus>,
    pub can_schedule_exact_alarms: bool,
    pub ignoring_battery_optimizations: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ChannelStatus {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub importance: i32,
    pub vibration: bool,
    pub sound: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_contract_exposes_all_ten_features_without_snapshot_data() {
        for os in ["macos", "windows", "linux", "android", "ios"] {
            let json = serde_json::to_value(info_for(os)).unwrap();
            assert_eq!(json.as_object().unwrap().len(), 3);
            assert_eq!(json["features"].as_object().unwrap().len(), 10);
            let mobile = matches!(os, "android" | "ios");
            assert_eq!(json["os"], os);
            assert_eq!(json["mobile"], mobile);
            for name in ["tray", "website_blocking", "browser_extension", "autostart",
                "reveal_state_file", "window_title", "quit_flow", "in_app_sound_toggle"] {
                assert_eq!(json["features"][name], !mobile, "{os}: {name}");
            }
            assert_eq!(json["features"]["system_settings"], os == "android");
            assert_eq!(json["features"]["exact_alarm_status"], os == "android");
        }
    }

    #[test]
    fn settings_accept_only_known_targets_and_own_channels() {
        for raw in ["app_notifications", "channel", "exact_alarm", "battery", "app_details"] {
            assert!(serde_json::from_value::<SettingsTarget>(serde_json::json!(raw)).is_ok());
        }
        for raw in ["", "other", "android.intent.action.VIEW", &"x".repeat(4096)] {
            assert!(serde_json::from_value::<SettingsTarget>(serde_json::json!(raw)).is_err());
        }
        for id in ["timer", "body", "water", "status"] {
            assert!(SettingsRequest::new(SettingsTarget::Channel, Some(id.into())).is_ok());
        }
        for id in [None, Some(""), Some("default"), Some("../water"), Some("water\n")] {
            assert!(SettingsRequest::new(SettingsTarget::Channel, id.map(str::to_owned)).is_err());
        }
        assert!(SettingsRequest::new(SettingsTarget::Battery, Some("water".into())).is_err());
        assert_eq!(serde_json::to_value(SettingsRequest::new(
            SettingsTarget::Channel, Some("water".into())
        ).unwrap()).unwrap(), serde_json::json!({"target": "channel", "channelId": "water"}));
    }
}
