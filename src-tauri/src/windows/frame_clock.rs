use std::{
    ffi::c_void,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HANDLE, HWND, INVALID_HANDLE_VALUE, POINT},
    Graphics::Gdi::{
        DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW, GetMonitorInfoW,
        MONITOR_DEFAULTTONEAREST, MONITORINFOEXW, MonitorFromPoint,
    },
    System::Threading::{
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, INFINITE,
        RegisterWaitForSingleObject, SYNCHRONIZATION_SYNCHRONIZE, SetWaitableTimer,
        TIMER_MODIFY_STATE, UnregisterWaitEx, WT_EXECUTEINWAITTHREAD,
    },
    UI::WindowsAndMessaging::PostMessageW,
};

use super::FRAME_MESSAGE;

pub(super) fn display_interval(point: POINT) -> Duration {
    let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let mut mode = DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    let hz = if unsafe { GetMonitorInfoW(monitor, (&raw mut info).cast()) } != 0
        && unsafe { EnumDisplaySettingsW(info.szDevice.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode) }
            != 0
        && mode.dmDisplayFrequency > 1
    {
        mode.dmDisplayFrequency
    } else {
        // Drivers can report 0 or 1 for an unspecified refresh rate.
        60
    };
    Duration::from_nanos(1_000_000_000u64.div_ceil(u64::from(hz)))
}

pub(super) fn next_deadline(previous: Instant, now: Instant, interval: Duration) -> Instant {
    // Keep the original cadence even when painting or message delivery misses a frame.
    let period = interval.as_nanos();
    let elapsed = now.saturating_duration_since(previous).as_nanos();
    now.max(previous) + Duration::from_nanos((period - elapsed % period) as u64)
}

pub(super) struct FrameTimer {
    timer: OwnedHandle,
    wait: HANDLE,
}

impl FrameTimer {
    pub(super) fn new(hwnd: HWND) -> Option<Self> {
        let handle = unsafe {
            CreateWaitableTimerExW(
                null(),
                null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE,
            )
        };
        if handle.is_null() {
            return None;
        }
        let timer = unsafe { OwnedHandle::from_raw_handle(handle) };
        let mut wait = null_mut();
        if unsafe {
            RegisterWaitForSingleObject(
                &mut wait,
                timer.as_raw_handle(),
                Some(post_frame),
                hwnd,
                INFINITE,
                WT_EXECUTEINWAITTHREAD,
            )
        } == 0
        {
            return None;
        }
        Some(Self { timer, wait })
    }

    pub(super) fn arm(&self, delay: Duration) -> bool {
        // Negative due times are relative, in units of 100 ns. A zero period is one-shot.
        let due = -(delay.as_nanos().div_ceil(100).max(1) as i64);
        unsafe { SetWaitableTimer(self.timer.as_raw_handle(), &due, 0, None, null(), 0) != 0 }
    }
}

unsafe extern "system" fn post_frame(hwnd: *mut c_void, _: bool) {
    // All window painting stays on the owning UI thread.
    unsafe { PostMessageW(hwnd, FRAME_MESSAGE, 0, 0) };
}

impl Drop for FrameTimer {
    fn drop(&mut self) {
        // Drain the callback before its HWND is destroyed and the timer handle closes.
        unsafe { UnregisterWaitEx(self.wait, INVALID_HANDLE_VALUE) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_refresh_deadlines_preserve_phase_and_skip_missed_frames() {
        let start = Instant::now();
        for hz in [60, 144, 165, 240] {
            let period = Duration::from_secs_f64(1.0 / f64::from(hz));
            let first = next_deadline(start, start, period);
            assert_eq!(first, start + period);
            assert_eq!(
                next_deadline(first, first + period / 3, period),
                first + period
            );
            assert_eq!(
                next_deadline(first, first + period * 3 + period / 2, period),
                first + period * 4
            );
        }
    }
}
