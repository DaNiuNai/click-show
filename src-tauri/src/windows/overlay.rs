//! Click-through, non-activating mouse indicators rendered into layered windows.

use std::mem::size_of;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HDC,
    HGDIOBJ, SelectObject,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HTTRANSPARENT, HWND_TOPMOST, MA_NOACTIVATE,
    RegisterClassW, SW_HIDE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
    SetWindowPos, ShowWindow, ULW_ALPHA, UnregisterClassW, UpdateLayeredWindow, WM_ERASEBKGND,
    WM_MOUSEACTIVATE, WM_NCHITTEST, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use super::frame_clock::display_interval;
use crate::settings::{AppSettings, Color};

const WINDOW_CLASS: *const u16 = windows_sys::w!("ClickShow.Rust.Overlay");
const IDLE_TIMEOUT: Duration = Duration::from_secs(5);
const DOWN_DURATION: Duration = Duration::from_millis(400);
const UP_DURATION: Duration = Duration::from_millis(300);

pub struct Overlays {
    instance: HINSTANCE,
    clicks: Vec<ClickOverlay>,
    hover: Option<HoverOverlay>,
    dpi_probe: Option<LayerWindow>,
    dpi_probe_position: Option<POINT>,
}

impl Overlays {
    pub fn new(instance: HINSTANCE) -> Result<Self, String> {
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: WINDOW_CLASS,
            ..Default::default()
        };
        // The class has no background brush: all drawing uses per-pixel alpha.
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err(win32_error("注册覆盖窗口失败"));
        }
        let mut overlays = Self {
            instance,
            clicks: Vec::new(),
            hover: None,
            dpi_probe: None,
            dpi_probe_position: None,
        };
        // This 1×1 window is never shown. GetDpiForWindow is the DPI-aware API;
        // a window positioned at the pointer selects its monitor unambiguously.
        overlays.dpi_probe = Some(LayerWindow::new(instance)?);
        Ok(overlays)
    }

    pub fn click(
        &mut self,
        point: POINT,
        color: Color,
        size_dip: f64,
        is_down: bool,
    ) -> Result<(), String> {
        let now = Instant::now();
        self.discard_idle(now);
        let scale = self.dpi_scale(point)?;
        let size_px = physical_size(size_dip, scale)?;
        let index = match self
            .clicks
            .iter()
            .position(|click| click.animation.is_none())
        {
            Some(index) => index,
            None => {
                self.clicks.push(ClickOverlay {
                    window: LayerWindow::new(self.instance)?,
                    animation: None,
                    last_used: Instant::now(),
                });
                self.clicks.len() - 1
            }
        };
        let click = &mut self.clicks[index];
        let animation = Animation {
            point,
            color,
            size_px,
            stroke_px: 3.0 * scale,
            frame_interval: display_interval(point),
            is_down,
            started: now,
        };
        click.window.ensure_surface(size_px.ceil() as i32)?;
        animation.draw(&mut click.window, now)?;
        click.animation = Some(animation);
        click.last_used = now;
        Ok(())
    }

    pub fn update_hover(&mut self, point: POINT, settings: &AppSettings) -> Result<(), String> {
        if !settings.enable_hover_dot {
            if let Some(hover) = self.hover.as_mut() {
                hover.window.hide();
                hover.appearance = None;
            }
            return Ok(());
        }

        let size_px = physical_size(settings.hover_dot_size, self.dpi_scale(point)?)?;
        let color = Color::parse(&settings.hover_dot_fill).unwrap_or(Color {
            a: 0x60,
            r: 0xff,
            g: 0xff,
            b: 0x5a,
        });
        if self.hover.is_none() {
            self.hover = Some(HoverOverlay {
                window: LayerWindow::new(self.instance)?,
                appearance: None,
            });
        }
        let hover = self.hover.as_mut().expect("hover was initialized");
        let appearance_changed = hover
            .appearance
            .as_ref()
            .is_none_or(|previous| previous.size_px != size_px || previous.color != color);
        let position_changed = hover
            .appearance
            .as_ref()
            .is_none_or(|previous| previous.point.x != point.x || previous.point.y != point.y);
        if appearance_changed {
            hover.window.ensure_surface(size_px.ceil() as i32)?;
            hover.window.draw_circle(size_px, None, color, 1.0);
            hover.window.present(point)?;
        } else if position_changed {
            // Moving a layered window preserves its bitmap; no pixel repaint is needed.
            hover.window.move_to(point)?;
        }
        hover.appearance = Some(HoverAppearance {
            point,
            size_px,
            color,
        });
        Ok(())
    }

    pub fn tick(&mut self) -> Result<(), String> {
        let now = Instant::now();
        for click in &mut self.clicks {
            if let Some(animation) = &click.animation {
                if now.duration_since(animation.started) >= animation.duration() {
                    click.window.hide();
                    click.animation = None;
                    click.last_used = now;
                } else {
                    animation.draw(&mut click.window, now)?;
                }
            }
        }
        self.discard_idle(now);
        Ok(())
    }

    pub fn frame_interval(&self) -> Option<Duration> {
        self.clicks
            .iter()
            .filter_map(|click| {
                click
                    .animation
                    .as_ref()
                    .map(|animation| animation.frame_interval)
            })
            .min()
    }

    fn discard_idle(&mut self, now: Instant) {
        self.clicks.retain(|click| {
            click.animation.is_some() || now.duration_since(click.last_used) < IDLE_TIMEOUT
        });
    }

    fn dpi_scale(&mut self, point: POINT) -> Result<f64, String> {
        let probe = self.dpi_probe.as_ref().expect("DPI probe was initialized");
        if self
            .dpi_probe_position
            .is_none_or(|previous| previous.x != point.x || previous.y != point.y)
        {
            if unsafe {
                SetWindowPos(
                    probe.hwnd,
                    null_mut(),
                    point.x,
                    point.y,
                    1,
                    1,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                )
            } == 0
            {
                return Err(win32_error("定位 DPI 查询窗口失败"));
            }
            self.dpi_probe_position = Some(point);
        }
        let dpi = unsafe { GetDpiForWindow(probe.hwnd) };
        if dpi == 0 {
            return Err(win32_error("查询显示器 DPI 失败"));
        }
        Ok(f64::from(dpi) / 96.0)
    }
}

impl Drop for Overlays {
    fn drop(&mut self) {
        // Windows must be gone before their shared class is unregistered.
        self.clicks.clear();
        self.hover = None;
        self.dpi_probe = None;
        unsafe {
            UnregisterClassW(WINDOW_CLASS, self.instance);
        }
    }
}

struct ClickOverlay {
    window: LayerWindow,
    animation: Option<Animation>,
    last_used: Instant,
}

struct Animation {
    point: POINT,
    color: Color,
    size_px: f64,
    stroke_px: f64,
    frame_interval: Duration,
    is_down: bool,
    started: Instant,
}

impl Animation {
    fn duration(&self) -> Duration {
        if self.is_down {
            DOWN_DURATION
        } else {
            UP_DURATION
        }
    }

    fn draw(&self, window: &mut LayerWindow, now: Instant) -> Result<(), String> {
        self.render(
            window.surface.as_mut().expect("surface was initialized"),
            now,
        );
        window.present(self.point)
    }

    fn render(&self, surface: &mut Surface, now: Instant) {
        let progress = (now.duration_since(self.started).as_secs_f64()
            / self.duration().as_secs_f64())
        .clamp(0.0, 1.0);
        let (diameter, opacity) = if self.is_down {
            (
                self.size_px * (0.2 + 0.8 * progress),
                0.95 * (1.0 - progress),
            )
        } else {
            (self.size_px * 0.5 * progress, 0.7 * (1.0 - progress))
        };
        surface.draw_circle(diameter, Some(self.stroke_px), self.color, opacity);
    }
}

struct HoverOverlay {
    window: LayerWindow,
    appearance: Option<HoverAppearance>,
}

struct HoverAppearance {
    point: POINT,
    size_px: f64,
    color: Color,
}

struct LayerWindow {
    hwnd: HWND,
    surface: Option<Surface>,
    visible: bool,
}

impl LayerWindow {
    fn new(instance: HINSTANCE) -> Result<Self, String> {
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
                WINDOW_CLASS,
                windows_sys::w!("ClickShow"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                instance,
                null(),
            )
        };
        if hwnd.is_null() {
            return Err(win32_error("创建覆盖窗口失败"));
        }
        Ok(Self {
            hwnd,
            surface: None,
            visible: false,
        })
    }

    fn ensure_surface(&mut self, side: i32) -> Result<(), String> {
        if self
            .surface
            .as_ref()
            .is_none_or(|surface| surface.side != side)
        {
            self.surface = Some(Surface::new(side)?);
        }
        Ok(())
    }

    fn draw_circle(&mut self, diameter: f64, stroke: Option<f64>, color: Color, opacity: f64) {
        self.surface
            .as_mut()
            .expect("surface was initialized")
            .draw_circle(diameter, stroke, color, opacity);
    }

    fn present(&mut self, center: POINT) -> Result<(), String> {
        let surface = self.surface.as_ref().expect("surface was initialized");
        let destination = top_left(center, surface.side);
        let size = SIZE {
            cx: surface.side,
            cy: surface.side,
        };
        let source = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        if unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                null_mut(),
                &destination,
                &size,
                surface.dc,
                &source,
                0,
                &blend,
                ULW_ALPHA,
            )
        } == 0
        {
            return Err(win32_error("更新覆盖窗口失败"));
        }
        if !self.visible {
            if unsafe {
                SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
                )
            } == 0
            {
                return Err(win32_error("显示覆盖窗口失败"));
            }
            self.visible = true;
        }
        Ok(())
    }

    fn move_to(&mut self, center: POINT) -> Result<(), String> {
        let surface = self.surface.as_ref().expect("surface was initialized");
        let position = top_left(center, surface.side);
        if unsafe {
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                position.x,
                position.y,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOSIZE | SWP_SHOWWINDOW,
            )
        } == 0
        {
            return Err(win32_error("移动浮标窗口失败"));
        }
        self.visible = true;
        Ok(())
    }

    fn hide(&mut self) {
        if self.visible {
            unsafe {
                ShowWindow(self.hwnd, SW_HIDE);
            }
            self.visible = false;
        }
    }
}

impl Drop for LayerWindow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous_bitmap: HGDIOBJ,
    pixels: *mut u8,
    side: i32,
}

impl Surface {
    fn draw_circle(&mut self, diameter: f64, stroke: Option<f64>, color: Color, opacity: f64) {
        // A top-down, 32-bit DIB stores BGRA, with premultiplied color channels.
        let pixels = unsafe {
            std::slice::from_raw_parts_mut(self.pixels, self.side as usize * self.side as usize * 4)
        };
        pixels.fill(0);
        if diameter <= 0.0 || opacity <= 0.0 || color.a == 0 {
            return;
        }
        let radius = diameter / 2.0;
        let inner_radius = stroke.map_or(0.0, |width| (radius - width).max(0.0));
        let center = f64::from(self.side) / 2.0;
        let first = (center - radius - 1.0).floor().max(0.0) as usize;
        let last = ((center + radius + 1.0).ceil() as usize).min(self.side as usize);
        let outer_bound_sq = (radius + 0.5).powi(2);
        let inner_bound_sq = (inner_radius - 0.5).max(0.0).powi(2);
        for y in first..last {
            let dy = y as f64 + 0.5 - center;
            for x in first..last {
                let dx = x as f64 + 0.5 - center;
                let distance_sq = dx * dx + dy * dy;
                if distance_sq >= outer_bound_sq
                    || (inner_radius > 0.5 && distance_sq <= inner_bound_sq)
                {
                    continue;
                }
                let distance = distance_sq.sqrt();
                let mut coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
                if inner_radius > 0.0 {
                    coverage *= (distance - inner_radius + 0.5).clamp(0.0, 1.0);
                }
                let alpha = (f64::from(color.a) * opacity * coverage).round() as u8;
                let offset = (y * self.side as usize + x) * 4;
                let premultiply =
                    |channel: u8| ((u16::from(channel) * u16::from(alpha) + 127) / 255) as u8;
                pixels[offset..offset + 4].copy_from_slice(&[
                    premultiply(color.b),
                    premultiply(color.g),
                    premultiply(color.r),
                    alpha,
                ]);
            }
        }
    }

    fn new(side: i32) -> Result<Self, String> {
        let dc = unsafe { CreateCompatibleDC(null_mut()) };
        if dc.is_null() {
            return Err(win32_error("创建覆盖绘图上下文失败"));
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: side,
                biHeight: -side,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = null_mut();
        let bitmap =
            unsafe { CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0) };
        if bitmap.is_null() || pixels.is_null() {
            let error = win32_error("创建覆盖位图失败");
            unsafe {
                if !bitmap.is_null() {
                    DeleteObject(bitmap);
                }
                DeleteDC(dc);
            }
            return Err(error);
        }
        let previous_bitmap = unsafe { SelectObject(dc, bitmap) };
        if previous_bitmap.is_null() || previous_bitmap as isize == -1 {
            let error = win32_error("选择覆盖位图失败");
            unsafe {
                DeleteObject(bitmap);
                DeleteDC(dc);
            }
            return Err(error);
        }
        Ok(Self {
            dc,
            bitmap,
            previous_bitmap,
            pixels: pixels.cast(),
            side,
        })
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            // Restore the stock bitmap before deleting the selected DIB.
            SelectObject(self.dc, self.previous_bitmap);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_ERASEBKGND => 1,
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn physical_size(size_dip: f64, scale: f64) -> Result<f64, String> {
    let size = size_dip * scale;
    if !size.is_finite() || size <= 0.0 || size > 8192.0 {
        return Err("提示器尺寸必须为有效正数，且缩放后不能超过 8192 像素".into());
    }
    Ok(size.max(1.0))
}

fn top_left(center: POINT, side: i32) -> POINT {
    POINT {
        x: center.x.saturating_sub(side / 2),
        y: center.y.saturating_sub(side / 2),
    }
}

fn win32_error(context: &str) -> String {
    format!("{context}：{}", std::io::Error::last_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, HMONITOR, MONITOR_DEFAULTTONEAREST, MonitorFromWindow,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        SetThreadDpiAwarenessContext,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, IsWindow, IsWindowVisible,
    };
    use windows_sys::core::BOOL;

    const WHITE: Color = Color {
        a: 255,
        r: 255,
        g: 255,
        b: 255,
    };

    fn pixels(surface: &Surface) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                surface.pixels,
                surface.side as usize * surface.side as usize * 4,
            )
        }
    }

    fn pixel(surface: &Surface, x: usize, y: usize) -> [u8; 4] {
        let offset = (y * surface.side as usize + x) * 4;
        pixels(surface)[offset..offset + 4].try_into().unwrap()
    }

    fn nonzero_width(surface: &Surface) -> usize {
        let mut left = usize::MAX;
        let mut right = 0;
        for (index, bgra) in pixels(surface).as_chunks::<4>().0.iter().enumerate() {
            if bgra[3] != 0 {
                let x = index % surface.side as usize;
                left = left.min(x);
                right = right.max(x);
            }
        }
        if left == usize::MAX {
            0
        } else {
            right - left + 1
        }
    }

    fn maximum_alpha(surface: &Surface) -> u8 {
        pixels(surface)
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bgra| bgra[3])
            .max()
            .unwrap()
    }

    #[test]
    fn native_dib_uses_bgra_premultiplied_alpha_and_smooth_edges() {
        let mut surface = Surface::new(24).unwrap();
        surface.draw_circle(
            20.0,
            None,
            Color {
                a: 128,
                r: 200,
                g: 100,
                b: 50,
            },
            0.5,
        );
        assert_eq!(pixel(&surface, 12, 12), [13, 25, 50, 64]);
        assert_eq!(pixel(&surface, 0, 0), [0; 4]);
        assert!(
            pixels(&surface)
                .as_chunks::<4>()
                .0
                .iter()
                .any(|bgra| (1..64).contains(&bgra[3]))
        );
        for bgra in pixels(&surface).as_chunks::<4>().0 {
            assert!(bgra[..3].iter().all(|channel| *channel <= bgra[3]));
        }
        surface.draw_circle(0.0, None, WHITE, 1.0);
        assert!(pixels(&surface).iter().all(|byte| *byte == 0));
    }

    #[test]
    fn native_ring_has_transparent_center_and_three_dip_stroke() {
        for (side, diameter, stroke, expected_width) in [(100, 80.0, 3.0, 3), (200, 160.0, 6.0, 6)]
        {
            let mut surface = Surface::new(side).unwrap();
            surface.draw_circle(diameter, Some(stroke), WHITE, 1.0);
            let center = side as usize / 2;
            assert_eq!(pixel(&surface, center, center), [0; 4]);
            assert_eq!(pixel(&surface, 0, center), [0; 4]);
            let solid_stroke_pixels = (center..side as usize)
                .filter(|x| pixel(&surface, *x, center)[3] >= 128)
                .count();
            assert_eq!(solid_stroke_pixels, expected_width);
            assert_eq!(nonzero_width(&surface), diameter as usize);
        }
    }

    #[test]
    fn native_hover_dot_preserves_default_alpha_and_solid_center() {
        let mut surface = Surface::new(60).unwrap();
        let color = Color::parse("#60FFFF5A").unwrap();
        surface.draw_circle(60.0, None, color, 1.0);
        assert_eq!(pixel(&surface, 30, 30), [34, 96, 96, 96]);
        assert_eq!(pixel(&surface, 0, 0), [0; 4]);
        assert_eq!(nonzero_width(&surface), 60);
        assert_eq!(maximum_alpha(&surface), 96);
        surface.draw_circle(60.0, None, color, 0.0);
        assert!(pixels(&surface).iter().all(|byte| *byte == 0));
    }

    #[test]
    fn native_down_animation_expands_and_fades_over_400_milliseconds() {
        let started = Instant::now();
        let animation = Animation {
            point: POINT { x: 0, y: 0 },
            color: WHITE,
            size_px: 100.0,
            stroke_px: 3.0,
            frame_interval: Duration::from_millis(5),
            is_down: true,
            started,
        };
        let mut surface = Surface::new(100).unwrap();
        for (milliseconds, expected_width, expected_alpha) in [
            (0, 20, 242),
            (100, 40, 182),
            (200, 60, 121),
            (300, 80, 61),
            (400, 0, 0),
        ] {
            animation.render(&mut surface, started + Duration::from_millis(milliseconds));
            assert_eq!(
                nonzero_width(&surface),
                expected_width,
                "t={milliseconds} ms"
            );
            assert_eq!(
                maximum_alpha(&surface),
                expected_alpha,
                "t={milliseconds} ms"
            );
            assert_eq!(pixel(&surface, 50, 50), [0; 4]);
        }
    }

    #[test]
    fn native_up_animation_expands_to_half_size_and_fades_over_300_milliseconds() {
        let started = Instant::now();
        let animation = Animation {
            point: POINT { x: 0, y: 0 },
            color: WHITE,
            size_px: 120.0,
            stroke_px: 3.0,
            frame_interval: Duration::from_millis(5),
            is_down: false,
            started,
        };
        let mut surface = Surface::new(120).unwrap();
        for (milliseconds, expected_width, expected_alpha) in [
            (0, 0, 0),
            (60, 12, 143),
            (120, 24, 107),
            (180, 36, 71),
            (240, 48, 36),
            (300, 0, 0),
        ] {
            animation.render(&mut surface, started + Duration::from_millis(milliseconds));
            assert_eq!(
                nonzero_width(&surface),
                expected_width,
                "t={milliseconds} ms"
            );
            assert_eq!(
                maximum_alpha(&surface),
                expected_alpha,
                "t={milliseconds} ms"
            );
            assert_eq!(pixel(&surface, 60, 60), [0; 4]);
        }
    }

    #[test]
    fn native_windows_recycle_idle_clicks_and_follow_each_monitor_dpi_without_activation() {
        struct RestoreThreadDpi(DPI_AWARENESS_CONTEXT);
        impl Drop for RestoreThreadDpi {
            fn drop(&mut self) {
                unsafe {
                    SetThreadDpiAwarenessContext(self.0);
                }
            }
        }
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        assert!(!previous.is_null());
        let _restore = RestoreThreadDpi(previous);
        let instance = unsafe { GetModuleHandleW(null()) };
        let mut overlays = Overlays::new(instance).unwrap();
        let mut monitors: Vec<(HMONITOR, RECT)> = Vec::new();
        unsafe extern "system" fn collect_monitor(
            monitor: HMONITOR,
            _: HDC,
            rectangle: *mut RECT,
            data: LPARAM,
        ) -> BOOL {
            unsafe { &mut *(data as *mut Vec<(HMONITOR, RECT)>) }
                .push((monitor, unsafe { *rectangle }));
            1
        }
        assert_ne!(
            unsafe {
                EnumDisplayMonitors(
                    null_mut(),
                    null(),
                    Some(collect_monitor),
                    &mut monitors as *mut _ as LPARAM,
                )
            },
            0
        );
        assert!(!monitors.is_empty());
        for (monitor, rectangle) in &monitors {
            let center = POINT {
                x: rectangle.left + (rectangle.right - rectangle.left) / 2,
                y: rectangle.top + (rectangle.bottom - rectangle.top) / 2,
            };
            let scale = overlays.dpi_scale(center).unwrap();
            let probe = overlays.dpi_probe.as_ref().unwrap();
            assert!((1.0..=8.0).contains(&scale));
            assert_eq!(
                unsafe { MonitorFromWindow(probe.hwnd, MONITOR_DEFAULTTONEAREST) },
                *monitor
            );
            assert_eq!(unsafe { IsWindowVisible(probe.hwnd) }, 0);
            eprintln!(
                "monitor ({}, {})..({}, {}): {} DPI",
                rectangle.left,
                rectangle.top,
                rectangle.right,
                rectangle.bottom,
                scale * 96.0
            );
        }
        let now = Instant::now();
        let expired = LayerWindow::new(instance).unwrap();
        let expired_hwnd = expired.hwnd;
        let reusable = LayerWindow::new(instance).unwrap();
        let reusable_hwnd = reusable.hwnd;
        overlays.clicks.push(ClickOverlay {
            window: expired,
            animation: None,
            last_used: now - IDLE_TIMEOUT - Duration::from_millis(1),
        });
        overlays.clicks.push(ClickOverlay {
            window: reusable,
            animation: None,
            last_used: now,
        });
        // Fully transparent colors exercise real UpdateLayeredWindow without visible UI.
        let transparent = Color { a: 0, ..WHITE };
        let center = POINT { x: 0, y: 0 };
        overlays.click(center, transparent, 100.0, true).unwrap();
        assert_eq!(overlays.clicks.len(), 1);
        assert_eq!(overlays.clicks[0].window.hwnd, reusable_hwnd);
        assert_eq!(unsafe { IsWindow(expired_hwnd) }, 0);
        overlays.click(center, transparent, 100.0, false).unwrap();
        assert_eq!(overlays.clicks.len(), 2);
        assert_ne!(
            overlays.clicks[0].window.hwnd,
            overlays.clicks[1].window.hwnd
        );
        let fast = Duration::from_secs_f64(1.0 / 240.0);
        let slow = Duration::from_secs_f64(1.0 / 60.0);
        overlays.clicks[0]
            .animation
            .as_mut()
            .unwrap()
            .frame_interval = fast;
        overlays.clicks[1]
            .animation
            .as_mut()
            .unwrap()
            .frame_interval = slow;
        assert_eq!(overlays.frame_interval(), Some(fast));
        overlays.clicks[0].animation.as_mut().unwrap().started = Instant::now() - DOWN_DURATION;
        overlays.tick().unwrap();
        assert_eq!(overlays.frame_interval(), Some(slow));
        let mut handles = vec![overlays.dpi_probe.as_ref().unwrap().hwnd];
        for click in &mut overlays.clicks {
            let style = unsafe { GetWindowLongPtrW(click.window.hwnd, GWL_EXSTYLE) } as u32;
            let required = WS_EX_LAYERED
                | WS_EX_TRANSPARENT
                | WS_EX_TOPMOST
                | WS_EX_TOOLWINDOW
                | WS_EX_NOACTIVATE;
            assert_eq!(style & required, required);
            handles.push(click.window.hwnd);
            if let Some(animation) = &mut click.animation {
                animation.started = Instant::now() - Duration::from_secs(1);
            }
        }
        overlays.tick().unwrap();
        assert!(overlays.frame_interval().is_none());
        assert!(overlays.clicks.iter().all(|click| !click.window.visible));
        assert_eq!(
            unsafe { window_proc(reusable_hwnd, WM_NCHITTEST, 0, 0) },
            HTTRANSPARENT as LRESULT
        );
        assert_eq!(
            unsafe { window_proc(reusable_hwnd, WM_MOUSEACTIVATE, 0, 0) },
            MA_NOACTIVATE as LRESULT
        );
        drop(overlays);
        assert!(handles.iter().all(|hwnd| unsafe { IsWindow(*hwnd) } == 0));
    }
}
