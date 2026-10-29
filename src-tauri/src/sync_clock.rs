//! 校准后的计时必须包括合盖/锁屏。std::time::Instant 在这些平台可能排除睡眠时间，不能用于专注进度。

#[cfg(target_os = "macos")]
pub fn continuous_seconds() -> Option<u64> {
    use std::sync::OnceLock;
    // 对应 SDK mach/mach_time.h 的稳定 C ABI；libc 中的 Mach 类型已弃用。
    #[repr(C)]
    struct Timebase { numer: u32, denom: u32 }
    unsafe extern "C" {
        fn mach_continuous_time() -> u64;
        fn mach_timebase_info(info: *mut Timebase) -> std::ffi::c_int;
    }
    static TIMEBASE: OnceLock<Option<(u32, u32)>> = OnceLock::new();
    let (numer, denom) = (*TIMEBASE.get_or_init(|| {
        let mut info = Timebase { numer: 0, denom: 0 };
        if unsafe { mach_timebase_info(&mut info) } != 0 || info.denom == 0 { None }
        else { Some((info.numer, info.denom)) }
    }))?;
    // mach_continuous_time 与 mach_absolute_time 使用相同时基，但前者明确包括系统睡眠。
    let ticks = unsafe { mach_continuous_time() };
    let seconds = u128::from(ticks) * u128::from(numer) / (u128::from(denom) * 1_000_000_000);
    u64::try_from(seconds).ok()
}

#[cfg(any(target_os = "android", target_os = "linux"))]
pub fn continuous_seconds() -> Option<u64> {
    let mut value = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // CLOCK_BOOTTIME 包含 suspend；CLOCK_MONOTONIC 不包含。
    if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut value) } != 0 { return None; }
    u64::try_from(value.tv_sec).ok()
}

#[cfg(target_os = "windows")]
pub fn continuous_seconds() -> Option<u64> {
    Some(unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() } / 1000)
}

#[cfg(not(any(target_os = "macos", target_os = "android", target_os = "linux", target_os = "windows")))]
pub fn continuous_seconds() -> Option<u64> { None }

pub fn anchored_time(anchor: Option<(u64, i64)>, current: Option<u64>, wall_fallback: i64) -> i64 {
    match (anchor, current) {
        (Some((start, server_time)), Some(current)) if current >= start => {
            server_time.saturating_add(current.saturating_sub(start).min(i64::MAX as u64) as i64)
        }
        _ => wall_fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_clock_counts_suspended_time_and_ignores_wall_clock_changes() {
        let anchor = Some((500, 1000));
        assert_eq!(anchored_time(anchor, Some(560), 50_000), 1060);
        assert_eq!(anchored_time(anchor, Some(560), 10), 1060);
        assert_eq!(anchored_time(anchor, Some(2300), 50_000), 2800, "系统睡眠1800秒仍计入同一时间锚点");
        assert_eq!(anchored_time(anchor, Some(20), 2000), 2000, "重启或不可用时使用已保存的墙钟校准");
        assert!(continuous_seconds().is_some());
    }
}
