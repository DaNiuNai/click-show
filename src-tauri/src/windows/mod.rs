#![allow(unsafe_op_in_unsafe_fn)]
pub mod autostart;
mod frame_clock;
mod overlay;

use crate::{
    input::ButtonTracker,
    settings::{AppSettings, Color, MouseButton},
};
use frame_clock::{FrameTimer, next_deadline};
use overlay::Overlays;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    ptr::{null, null_mut},
    sync::OnceLock,
    time::Instant,
};
use windows_sys::{
    Win32::{Foundation::*, System::LibraryLoader::GetModuleHandleW, UI::WindowsAndMessaging::*},
    w,
};

const CLASS: *const u16 = w!("ClickShow.MouseEvents");
const INPUT_MESSAGE: u32 = WM_APP + 1;
const FRAME_MESSAGE: u32 = WM_APP + 2;
const ANIMATION_TIMER: usize = 1;
static START: OnceLock<Instant> = OnceLock::new();

thread_local! {
    static RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
    static INPUT: RefCell<VecDeque<MouseInput>> = const { RefCell::new(VecDeque::new()) };
    static RECEIVER: Cell<HWND> = const { Cell::new(null_mut()) };
}

struct Runtime {
    hwnd: HWND,
    hook: HHOOK,
    frame_timer: Option<FrameTimer>,
    frame_deadline: Option<Instant>,
    overlays: Overlays,
    tracker: ButtonTracker,
    settings: AppSettings,
}

#[derive(Clone, Copy)]
struct MouseInput {
    message: u32,
    point: POINT,
    data: u32,
    time: u64,
}
unsafe extern "system" fn mouse_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 {
        let raw = &*(lp as *const MSLLHOOKSTRUCT);
        let message = wp as u32;
        if matches!(
            message,
            WM_MOUSEMOVE
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_RBUTTONDOWN
                | WM_RBUTTONUP
                | WM_MBUTTONDOWN
                | WM_MBUTTONUP
                | WM_XBUTTONDOWN
                | WM_XBUTTONUP
        ) {
            let event = MouseInput {
                message,
                point: raw.pt,
                data: raw.mouseData,
                time: START.get_or_init(Instant::now).elapsed().as_millis() as u64,
            };
            INPUT.with(|queue| {
                let mut queue = queue.borrow_mut();
                let wake = queue.is_empty();
                if message == WM_MOUSEMOVE
                    && queue.back().is_some_and(|e| e.message == WM_MOUSEMOVE)
                {
                    queue.pop_back();
                }
                queue.push_back(event);
                if wake {
                    PostMessageW(RECEIVER.get(), INPUT_MESSAGE, 0, 0);
                }
            });
        }
    }
    CallNextHookEx(null_mut(), code, wp, lp)
}

impl Runtime {
    fn input(&mut self, event: MouseInput) -> Result<(), String> {
        if event.message == WM_MOUSEMOVE {
            if self.settings.enable_hover_dot {
                self.overlays.update_hover(event.point, &self.settings)?;
            }
            return Ok(());
        }
        let button = match event.message {
            WM_LBUTTONDOWN | WM_LBUTTONUP => MouseButton::Left,
            WM_MBUTTONDOWN | WM_MBUTTONUP => MouseButton::Middle,
            WM_RBUTTONDOWN | WM_RBUTTONUP => MouseButton::Right,
            _ if event.data >> 16 == 2 => MouseButton::XButton2,
            _ => MouseButton::XButton1,
        };
        let down = matches!(
            event.message,
            WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONDOWN | WM_XBUTTONDOWN
        );
        let config = &self.settings.mouse_button_settings[button.index()];
        if self.tracker.should_display(
            button,
            down,
            (event.point.x.into(), event.point.y.into()),
            1.0,
            event.time,
            self.settings.enable_click_circle && config.is_enabled,
        ) {
            self.overlays.click(
                event.point,
                Color::parse(&config.color)?,
                self.settings.indicator_size,
                down,
            )?;
            if self.frame_deadline.is_none() {
                self.schedule_frame()?;
            }
        }
        Ok(())
    }

    fn schedule_frame(&mut self) -> Result<(), String> {
        let previous = self.frame_deadline.take();
        let Some(interval) = self.overlays.frame_interval() else {
            return Ok(());
        };
        let now = Instant::now();
        let deadline = next_deadline(previous.unwrap_or(now), now, interval);
        let delay = deadline - now;
        if !self
            .frame_timer
            .as_ref()
            .is_some_and(|timer| timer.arm(delay))
        {
            // Older Windows versions do not support high-resolution waitable timers.
            let millis = delay.as_nanos().div_ceil(1_000_000) as u32;
            if unsafe { SetTimer(self.hwnd, ANIMATION_TIMER, millis, None) } == 0 {
                return Err(format!(
                    "无法安排波纹刷新：{}",
                    std::io::Error::last_os_error()
                ));
            }
        }
        self.frame_deadline = Some(deadline);
        Ok(())
    }

    fn refresh_hover(&mut self) -> Result<(), String> {
        let mut point = POINT::default();
        unsafe {
            GetCursorPos(&mut point);
        }
        self.overlays.update_hover(point, &self.settings)
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match message {
        INPUT_MESSAGE => {
            let events = INPUT.with_borrow_mut(std::mem::take);
            RUNTIME.with_borrow_mut(|runtime| {
                if let Some(runtime) = runtime {
                    for event in events {
                        if let Err(error) = runtime.input(event) {
                            eprintln!("{error}");
                        }
                    }
                }
            });
        }
        FRAME_MESSAGE | WM_TIMER if message == FRAME_MESSAGE || wp == ANIMATION_TIMER => {
            if message == WM_TIMER {
                KillTimer(hwnd, ANIMATION_TIMER);
            }
            RUNTIME.with_borrow_mut(|runtime| {
                if let Some(runtime) = runtime {
                    if let Err(error) = runtime.overlays.tick() {
                        eprintln!("{error}");
                    }
                    if let Err(error) = runtime.schedule_frame() {
                        eprintln!("{error}");
                    }
                }
            });
        }
        _ => return DefWindowProcW(hwnd, message, wp, lp),
    }
    0
}

pub fn start(settings: &AppSettings) -> Result<(), String> {
    unsafe {
        let instance = GetModuleHandleW(null());
        let overlays = Overlays::new(instance)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: CLASS,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let hwnd = CreateWindowExW(
            0,
            CLASS,
            null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            let error = std::io::Error::last_os_error();
            UnregisterClassW(CLASS, instance);
            return Err(error.to_string());
        }
        RECEIVER.set(hwnd);
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), instance, 0);
        let mut runtime = Runtime {
            hwnd,
            hook,
            frame_timer: FrameTimer::new(hwnd),
            frame_deadline: None,
            overlays,
            tracker: ButtonTracker::default(),
            settings: settings.clone(),
        };
        if hook.is_null() {
            return Err(format!("无法监听鼠标：{}", std::io::Error::last_os_error()));
        }
        runtime.refresh_hover()?;
        RUNTIME.with_borrow_mut(|slot| *slot = Some(runtime));
    }
    Ok(())
}

pub fn apply(settings: &AppSettings) -> Result<(), String> {
    RUNTIME.with_borrow_mut(|slot| {
        let runtime = slot.as_mut().ok_or("鼠标监听未启动")?;
        runtime.settings = settings.clone();
        runtime.refresh_hover()
    })
}

impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            drop(self.frame_timer.take());
            if !self.hook.is_null() {
                UnhookWindowsHookEx(self.hook);
            }
            RECEIVER.set(null_mut());
            DestroyWindow(self.hwnd);
            UnregisterClassW(CLASS, GetModuleHandleW(null()));
        }
    }
}

pub fn stop() {
    drop(RUNTIME.with_borrow_mut(Option::take));
}
