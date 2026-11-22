mod overlay;

use crate::{
    input::ButtonTracker,
    settings::{AppSettings, Color, MouseButton},
};
use block2::RcBlock;
use objc2::{MainThreadMarker, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSEvent, NSEventMask as Mask, NSEventType as Kind, NSScreen};
use objc2_core_graphics::CGEvent;
use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes, NSTimer};
use overlay::Indicator;
use std::{cell::RefCell, ptr::NonNull, time::Instant};

thread_local! { static RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) }; }

struct Monitor(Retained<AnyObject>);
impl Drop for Monitor {
    fn drop(&mut self) {
        unsafe {
            NSEvent::removeMonitor(&self.0);
        }
    }
}

struct Ripple {
    indicator: Indicator,
    began: Instant,
    down: bool,
}
impl Ripple {
    fn frame(&self) -> bool {
        let duration = if self.down { 0.4 } else { 0.3 };
        let t = self.began.elapsed().as_secs_f64() / duration;
        if t >= 1. {
            return false;
        }
        let (diameter, alpha) = if self.down {
            (0.2 + 0.8 * t, 0.95)
        } else {
            (0.5 * t, 0.7)
        };
        self.indicator.draw(diameter, (alpha * (1. - t)) as f32);
        true
    }
}

struct Runtime {
    _monitors: [Monitor; 2],
    settings: AppSettings,
    tracker: ButtonTracker,
    hover: Option<Indicator>,
    ripples: Vec<Ripple>,
    timer: Option<Retained<NSTimer>>,
}

impl Runtime {
    fn refresh_hover(&mut self, mtm: MainThreadMarker) -> Result<(), String> {
        self.hover = None;
        if self.settings.enable_hover_dot {
            let dot = Indicator::new(
                mtm,
                NSEvent::mouseLocation(),
                self.settings.hover_dot_size,
                Color::parse(&self.settings.hover_dot_fill)?,
                true,
            );
            dot.draw(1., 1.);
            dot.show();
            self.hover = Some(dot);
        }
        Ok(())
    }

    fn handle(&mut self, event: &NSEvent, mtm: MainThreadMarker) {
        let down = match event.r#type() {
            Kind::LeftMouseDown | Kind::RightMouseDown | Kind::OtherMouseDown => Some(true),
            Kind::LeftMouseUp | Kind::RightMouseUp | Kind::OtherMouseUp => Some(false),
            _ if self.hover.is_some() => None,
            _ => return,
        };
        let point = event
            .CGEvent()
            .map(|raw| CGEvent::unflipped_location(Some(&raw)))
            .unwrap_or_else(NSEvent::mouseLocation);
        if let Some(dot) = &self.hover {
            dot.place(point);
        }
        let Some(down) = down else { return };
        let button = match event.buttonNumber() {
            0 => MouseButton::Left,
            1 => MouseButton::Right,
            2 => MouseButton::Middle,
            3 => MouseButton::XButton1,
            4 => MouseButton::XButton2,
            _ => return,
        };
        let config = &self.settings.mouse_button_settings[button.index()];
        let scale = NSScreen::screens(mtm)
            .iter()
            .find(|screen| {
                let rect = screen.frame();
                point.x >= rect.origin.x
                    && point.x < rect.origin.x + rect.size.width
                    && point.y >= rect.origin.y
                    && point.y < rect.origin.y + rect.size.height
            })
            .map_or(1., |screen| screen.backingScaleFactor());
        if !self.tracker.should_display(
            button,
            down,
            (point.x, point.y),
            scale,
            (event.timestamp() * 1000.) as u64,
            self.settings.enable_click_circle && config.is_enabled,
        ) {
            return;
        }
        let Ok(color) = Color::parse(&config.color) else {
            return;
        };
        let indicator = Indicator::new(mtm, point, self.settings.indicator_size, color, false);
        let ripple = Ripple {
            indicator,
            down,
            began: Instant::now(),
        };
        ripple.frame();
        ripple.indicator.show();
        self.ripples.push(ripple);
        if self.timer.is_none() {
            let block = RcBlock::new(|_: NonNull<NSTimer>| {
                RUNTIME.with_borrow_mut(|slot| {
                    if let Some(runtime) = slot {
                        runtime.ripples.retain(Ripple::frame);
                        if runtime.ripples.is_empty()
                            && let Some(timer) = runtime.timer.take()
                        {
                            timer.invalidate();
                        }
                    }
                })
            });
            // Common modes keep the animation running during menus and drags.
            unsafe {
                let timer = NSTimer::timerWithTimeInterval_repeats_block(1. / 60., true, &block);
                NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
                self.timer = Some(timer);
            }
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.invalidate();
        }
    }
}

fn handle_event(event: NonNull<NSEvent>) {
    // AppKit delivers both monitor callbacks on its main thread.
    let mtm = MainThreadMarker::new().expect("AppKit main thread");
    RUNTIME.with_borrow_mut(|slot| {
        if let Some(runtime) = slot {
            runtime.handle(unsafe { event.as_ref() }, mtm);
        }
    });
}

pub fn start(settings: &AppSettings) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("需要在 AppKit 主线程启动鼠标监听")?;
    let mask = Mask::LeftMouseDown
        | Mask::LeftMouseUp
        | Mask::RightMouseDown
        | Mask::RightMouseUp
        | Mask::OtherMouseDown
        | Mask::OtherMouseUp
        | Mask::MouseMoved
        | Mask::LeftMouseDragged
        | Mask::RightMouseDragged
        | Mask::OtherMouseDragged;
    let global = Monitor(
        NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &RcBlock::new(handle_event))
            .ok_or("无法监听全局鼠标事件，请检查系统设置中的输入监控权限并重新启动 ClickShow。")?,
    );
    let local_block = RcBlock::new(|event: NonNull<NSEvent>| {
        handle_event(event);
        event.as_ptr()
    });
    // Return the original event so the settings window and status item receive clicks.
    let local = Monitor(
        unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &local_block) }
            .ok_or("无法监听应用内鼠标事件")?,
    );
    let mut runtime = Runtime {
        _monitors: [global, local],
        settings: settings.clone(),
        tracker: ButtonTracker::default(),
        hover: None,
        ripples: vec![],
        timer: None,
    };
    runtime.refresh_hover(mtm)?;
    RUNTIME.with_borrow_mut(|slot| *slot = Some(runtime));
    Ok(())
}

pub fn apply(settings: &AppSettings) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("需要在 AppKit 主线程更新设置")?;
    RUNTIME.with_borrow_mut(|slot| {
        let runtime = slot.as_mut().ok_or("鼠标监听未启动")?;
        runtime.settings = settings.clone();
        runtime.refresh_hover(mtm)
    })
}

pub fn stop() {
    drop(RUNTIME.with_borrow_mut(Option::take));
}
