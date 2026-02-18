#[cfg(target_os = "macos")]
use std::sync::OnceLock;

/// Returns the current monotonic time in nanoseconds.
#[inline(always)]
#[cfg(target_os = "macos")]
#[allow(deprecated)]
pub fn now_ns() -> u64 {
    static TIMEBASE: OnceLock<(u64, u64)> = OnceLock::new();
    let (numer, denom) = *TIMEBASE.get_or_init(|| {
        let mut info = libc::mach_timebase_info_data_t { numer: 0, denom: 0 };
        let rc = unsafe { libc::mach_timebase_info(&mut info) };
        if rc != 0 || info.denom == 0 {
            (1, 1)
        } else {
            (info.numer as u64, info.denom as u64)
        }
    });
    let t = unsafe { libc::mach_absolute_time() } as u128;
    ((t * numer as u128) / denom as u128) as u64
}

/// Returns the current monotonic time in nanoseconds.
#[inline(always)]
#[cfg(not(target_os = "macos"))]
pub fn now_ns() -> u64 {
    let mut ts: libc::timespec = unsafe { core::mem::zeroed() };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}
