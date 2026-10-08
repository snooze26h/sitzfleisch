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
    pub app_blocking: bool,
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
            app_blocking: os == "android",
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
    Accessibility,
    /// 荣耀 / 华为的「应用启动管理」；其他手机退到应用详情。
    Startup,
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
    /// 坐功的无障碍服务在系统里是否打开；应用屏蔽靠它才能生效。
    pub app_block_service_enabled: bool,
    /// 系统不许坐功在后台运行：划掉坐功会被强行停止，提醒和屏蔽服务一起失效。
    pub background_restricted: bool,
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

/// 交给原生屏蔽服务的规则：只有开关和包名。原生侧落进自己的存储，
/// 进程只为无障碍服务启动、Rust 还没跑起来时也照样生效。
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppBlockRules {
    pub enabled: bool,
    pub packages: Vec<String>,
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
impl AppBlockRules {
    pub fn from_preferences(blocking: &sitzfleisch_core::AppBlocking) -> Self {
        let mut packages: Vec<String> = blocking.apps.iter().map(|app| app.package_name.clone()).collect();
        packages.sort();
        packages.dedup();
        Self { enabled: blocking.enabled, packages }
    }
}

/// 应用选择器的一项：可从桌面启动的应用。
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledApp {
    pub package_name: String,
    pub label: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledApps {
    pub apps: Vec<InstalledApp>,
    /// 厂商的「获取应用列表」权限没给时，系统只交出一部分应用。
    pub limited: bool,
    /// 这台手机有没有那项厂商权限可申请。
    pub can_request_full_list: bool,
}

/// 选择器一次最多列这么多应用；手机上能从桌面打开的应用远少于此。
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const MAX_INSTALLED_APPS: usize = 1000;
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const MAX_APP_LABEL_CHARS: usize = 80;

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
impl InstalledApps {
    /// 应用名来自手机上的各个应用：入界面前只留合法包名、可显示且不过长的名字，按包名去重。
    pub fn sanitized(mut self) -> Self {
        let mut seen = std::collections::HashSet::new();
        self.apps.retain(|app| {
            let label = app.label.trim();
            sitzfleisch_core::valid_package_name(&app.package_name)
                && !label.is_empty()
                && label.chars().count() <= MAX_APP_LABEL_CHARS
                && !label.chars().any(char::is_control)
                && seen.insert(app.package_name.clone())
        });
        self.apps.truncate(MAX_INSTALLED_APPS);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_contract_exposes_all_eleven_features_without_snapshot_data() {
        for os in ["macos", "windows", "linux", "android", "ios"] {
            let json = serde_json::to_value(info_for(os)).unwrap();
            assert_eq!(json.as_object().unwrap().len(), 3);
            assert_eq!(json["features"].as_object().unwrap().len(), 11);
            let mobile = matches!(os, "android" | "ios");
            assert_eq!(json["os"], os);
            assert_eq!(json["mobile"], mobile);
            for name in ["tray", "website_blocking", "browser_extension", "autostart",
                "reveal_state_file", "window_title", "quit_flow", "in_app_sound_toggle"] {
                assert_eq!(json["features"][name], !mobile, "{os}: {name}");
            }
            assert_eq!(json["features"]["system_settings"], os == "android");
            assert_eq!(json["features"]["exact_alarm_status"], os == "android");
            assert_eq!(json["features"]["app_blocking"], os == "android");
        }
    }

    #[test]
    fn settings_accept_only_known_targets_and_own_channels() {
        for raw in ["app_notifications", "channel", "exact_alarm", "battery", "app_details", "accessibility", "startup"] {
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
        assert!(SettingsRequest::new(SettingsTarget::Accessibility, Some("water".into())).is_err());
        assert!(SettingsRequest::new(SettingsTarget::Startup, Some("water".into())).is_err());
        assert_eq!(serde_json::to_value(SettingsRequest::new(
            SettingsTarget::Channel, Some("water".into())
        ).unwrap()).unwrap(), serde_json::json!({"target": "channel", "channelId": "water"}));
    }

    #[test]
    fn installed_apps_keep_only_valid_packages_and_displayable_labels() {
        let app = |package_name: &str, label: &str| InstalledApp { package_name: package_name.into(), label: label.into() };
        let list = InstalledApps {
            apps: vec![
                app("com.ss.android.ugc.aweme", "抖音"), app("com.ss.android.ugc.aweme", "抖音（重复）"),
                app("bad package", "坏包名"), app("tv.danmaku.bili", ""), app("com.tencent.mm", "微\n信"),
                app("com.xingin.xhs", &"长".repeat(MAX_APP_LABEL_CHARS + 1)), app("com.tencent.mobileqq", "QQ"),
            ],
            limited: true,
            can_request_full_list: true,
        }.sanitized();
        let kept: Vec<_> = list.apps.iter().map(|a| (a.package_name.as_str(), a.label.as_str())).collect();
        assert_eq!(kept, [("com.ss.android.ugc.aweme", "抖音"), ("com.tencent.mobileqq", "QQ")]);
        assert_eq!(serde_json::to_value(&list).unwrap()["apps"][0], serde_json::json!({"packageName": "com.ss.android.ugc.aweme", "label": "抖音"}));
        let many = InstalledApps {
            apps: (0..MAX_INSTALLED_APPS + 5).map(|i| app(&format!("com.example.app{i}"), "应用")).collect(),
            limited: false, can_request_full_list: false,
        }.sanitized();
        assert_eq!(many.apps.len(), MAX_INSTALLED_APPS);
    }

    #[test]
    fn block_rules_carry_only_the_switch_and_sorted_unique_packages() {
        use sitzfleisch_core::{AppBlocking, BlockedApp};
        let app = |package_name: &str, label: &str| BlockedApp { package_name: package_name.into(), label: label.into() };
        let blocking = AppBlocking { enabled: true, apps: vec![app("tv.danmaku.bili", "哔哩哔哩"), app("com.ss.android.ugc.aweme", "抖音")] };
        let rules = AppBlockRules::from_preferences(&blocking);
        assert_eq!(rules.packages, ["com.ss.android.ugc.aweme", "tv.danmaku.bili"]);
        assert_eq!(serde_json::to_value(&rules).unwrap(), serde_json::json!({
            "enabled": true, "packages": ["com.ss.android.ugc.aweme", "tv.danmaku.bili"]
        }), "应用名不出外壳：原生侧只按包名判断");
        let mut reordered = blocking.clone();
        reordered.apps.reverse();
        assert_eq!(AppBlockRules::from_preferences(&reordered), rules, "顺序变化不触发重推");
        assert!(!AppBlockRules::from_preferences(&AppBlocking::default()).enabled);
    }
}
