// Win32 handles belong to the GUI thread. Nested message-loop effects must run
// after releasing App's RefCell borrow; the hook only queues input.
#![allow(unsafe_op_in_unsafe_fn)]

use crate::{
    input::ButtonTracker,
    overlay::Overlays,
    platform::{self, UpdateInfo},
    settings::{self, AppSettings, Color, MouseButton},
};
use std::{
    cell::RefCell,
    collections::VecDeque,
    mem::{size_of, zeroed},
    path::PathBuf,
    ptr::{null, null_mut},
    sync::{
        OnceLock,
        atomic::{AtomicPtr, Ordering},
        mpsc,
    },
    time::Instant,
};
use windows_sys::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::{Dialogs::*, *},
            HiDpi::*,
            Input::KeyboardAndMouse::*,
            Shell::*,
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

const INPUT_MESSAGE: u32 = WM_APP + 1;
const TRAY_MESSAGE: u32 = WM_APP + 2;
const UPDATE_MESSAGE: u32 = WM_APP + 3;
const ANIMATION_TIMER: usize = 1;
const SAVE_TIMER: usize = 2;
const UPDATE_TIMER: usize = 3;
const STARTUP: u16 = 100;
const CIRCLES: u16 = 101;
const HOVER: u16 = 102;
const HOMEPAGE: u16 = 103;
const MORE: u16 = 104;
const EXIT: u16 = 105;
const NEW_VERSION: u16 = 106;
const INDICATOR_SIZE: u16 = 200;
const HOVER_SIZE: u16 = 201;
const INDICATOR_LABEL: u16 = 202;
const HOVER_LABEL: u16 = 203;
const ENABLE_BASE: u16 = 210;
const COLOR_BASE: u16 = 220;
const HOVER_COLOR: u16 = 230;
const DEFAULTS: u16 = 240;
const SETTINGS_CLOSE: u16 = 241;
const COLOR_TEXT: u16 = 300;
const COLOR_PICK: u16 = 301;
const ALPHA_SLIDER: u16 = 302;
const ALPHA_LABEL: u16 = 303;
const COLOR_CLOSE: u16 = 304;
// The SDK defines TBM_GETPOS as WM_USER (not exported by windows-sys).
const TBM_GETPOS: u32 = WM_USER;

static MAIN: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(null_mut());
static START: OnceLock<Instant> = OnceLock::new();
thread_local! {
    static APP:RefCell<Option<App>>=const {RefCell::new(None)};
    static INPUT:RefCell<VecDeque<MouseInput>>=const {RefCell::new(VecDeque::new())};
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
// Win32 MAKEINTRESOURCEW encodes a resource ID in a pointer-shaped argument.
// This value is passed to the resource loader and must never be dereferenced.
const fn make_int_resource(id: u16) -> *const u16 {
    id as usize as *const u16
}
fn last_error(context: &str) -> String {
    format!("{context}：{}", std::io::Error::last_os_error())
}
fn fill_text<const N: usize>(target: &mut [u16; N], text: &str) {
    for (dest, ch) in target.iter_mut().take(N - 1).zip(text.encode_utf16()) {
        *dest = ch;
    }
}
pub fn show_error(error: &str) {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(error).as_ptr(),
            w!("ClickShow"),
            MB_OK | MB_ICONERROR,
        );
    }
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
                    PostMessageW(MAIN.load(Ordering::Relaxed), INPUT_MESSAGE, 0, 0);
                }
            });
        }
    }
    CallNextHookEx(null_mut(), code, wp, lp)
}

struct Control {
    hwnd: HWND,
    rect: [i32; 4],
}
struct WindowUi {
    hwnd: HWND,
    controls: Vec<Control>,
    font: HFONT,
}
impl WindowUi {
    unsafe fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            controls: Vec::new(),
            font: null_mut(),
        }
    }
    unsafe fn add(
        &mut self,
        class: *const u16,
        text: &str,
        id: u16,
        style: u32,
        rect: [i32; 4],
    ) -> Result<HWND, String> {
        let hwnd = CreateWindowExW(
            0,
            class,
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            0,
            0,
            self.hwnd,
            id as usize as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        if hwnd.is_null() {
            return Err(last_error("无法创建界面控件"));
        }
        self.controls.push(Control { hwnd, rect });
        Ok(hwnd)
    }
    unsafe fn layout(&mut self) {
        let dpi = GetDpiForWindow(self.hwnd).max(96) as i32;
        let font = CreateFontW(
            -14 * dpi / 96,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32,
            w!("Microsoft YaHei UI"),
        );
        for control in &self.controls {
            let [x, y, width, height] = control.rect.map(|v| v * dpi / 96);
            SetWindowPos(
                control.hwnd,
                null_mut(),
                x,
                y,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW,
            );
            // Queue paint after the enclosing App borrow has ended. Owner-drawn
            // buttons must not synchronously re-enter the state dispatcher here.
            SendMessageW(control.hwnd, WM_SETFONT, font as usize, 0);
            InvalidateRect(control.hwnd, null(), 1);
        }
        if !self.font.is_null() {
            DeleteObject(self.font);
        }
        self.font = font;
    }
}
impl Drop for WindowUi {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(self.hwnd) != 0 {
                DestroyWindow(self.hwnd);
            }
            if !self.font.is_null() {
                DeleteObject(self.font);
            }
        }
    }
}
struct ColorEditor {
    ui: WindowUi,
    target: usize,
}
enum Effect {
    None,
    Error(String),
    OpenUrl(HWND, String),
    PickColor(HWND, Color),
    Update(UpdateInfo),
    TrayMenu(POINT),
}
struct App {
    instance: HINSTANCE,
    main: WindowUi,
    settings_ui: Option<WindowUi>,
    color_ui: Option<ColorEditor>,
    settings: AppSettings,
    path: PathBuf,
    overlays: Overlays,
    tracker: ButtonTracker,
    hook: HHOOK,
    icon: HICON,
    taskbar_created: u32,
    updates: mpsc::Receiver<UpdateInfo>,
    update_sender: Option<mpsc::Sender<UpdateInfo>>,
    custom_colors: [u32; 16],
    animation_timer_active: bool,
}

unsafe fn register_class(instance: HINSTANCE) -> Result<(), String> {
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        hIcon: LoadIconW(instance, make_int_resource(1)),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: (COLOR_BTNFACE + 1) as HBRUSH,
        lpszClassName: w!("ClickShow.Rust.UI"),
        ..zeroed()
    };
    if RegisterClassExW(&class) == 0 {
        Err(last_error("无法注册窗口"))
    } else {
        Ok(())
    }
}
unsafe fn create_window(
    instance: HINSTANCE,
    owner: HWND,
    title: &str,
    width: i32,
    height: i32,
    minimize: bool,
) -> Result<WindowUi, String> {
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | if minimize { WS_MINIMIZEBOX } else { 0 };
    let dpi = if owner.is_null() {
        GetDpiForSystem()
    } else {
        GetDpiForWindow(owner)
    }
    .max(96);
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: width * dpi as i32 / 96,
        bottom: height * dpi as i32 / 96,
    };
    AdjustWindowRectExForDpi(&mut rect, style, 0, 0, dpi);
    let width_px = rect.right - rect.left;
    let height_px = rect.bottom - rect.top;
    let mut area = RECT::default();
    if owner.is_null() {
        SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut area as *mut RECT).cast(), 0);
    } else {
        GetWindowRect(owner, &mut area);
    }
    let hwnd = CreateWindowExW(
        WS_EX_CONTROLPARENT,
        w!("ClickShow.Rust.UI"),
        wide(title).as_ptr(),
        style,
        area.left + (area.right - area.left - width_px) / 2,
        area.top + (area.bottom - area.top - height_px) / 2,
        width_px,
        height_px,
        owner,
        null_mut(),
        instance,
        null(),
    );
    if hwnd.is_null() {
        Err(last_error("无法创建窗口"))
    } else {
        Ok(WindowUi::new(hwnd))
    }
}
unsafe fn set_checked(hwnd: HWND, id: u16, checked: bool) {
    SendDlgItemMessageW(hwnd, id as i32, BM_SETCHECK, usize::from(checked), 0);
}
unsafe fn checked(hwnd: HWND, id: u16) -> bool {
    SendDlgItemMessageW(hwnd, id as i32, BM_GETCHECK, 0, 0) == BST_CHECKED as isize
}
unsafe fn text(hwnd: HWND, id: u16, value: &str) {
    SetDlgItemTextW(hwnd, id as i32, wide(value).as_ptr());
}
unsafe fn slider(
    ui: &mut WindowUi,
    id: u16,
    min: i32,
    max: i32,
    step: i32,
    rect: [i32; 4],
) -> Result<(), String> {
    let hwnd = ui.add(
        w!("msctls_trackbar32"),
        "",
        id,
        WS_TABSTOP | TBS_NOTICKS | TBS_TOOLTIPS,
        rect,
    )?;
    SendMessageW(hwnd, TBM_SETRANGEMIN, 0, min as isize);
    SendMessageW(hwnd, TBM_SETRANGEMAX, 0, max as isize);
    SendMessageW(hwnd, TBM_SETLINESIZE, 0, step as isize);
    SendMessageW(hwnd, TBM_SETPAGESIZE, 0, step as isize);
    Ok(())
}

impl App {
    unsafe fn new(path: PathBuf, settings: AppSettings) -> Result<Self, String> {
        let instance = GetModuleHandleW(null());
        register_class(instance)?;
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES | ICC_LINK_CLASS | ICC_STANDARD_CLASSES,
        });
        let mut main = create_window(instance, null_mut(), "ClickShow 1.4.1.0", 450, 180, true)?;
        main.add(
            w!("BUTTON"),
            "开机自动启动(&S)",
            STARTUP,
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            [25, 35, 192, 24],
        )?;
        main.add(
            w!("BUTTON"),
            "显示点击特效(&C)",
            CIRCLES,
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            [25, 68, 192, 24],
        )?;
        main.add(
            w!("BUTTON"),
            "显示位置提示浮标(&F)",
            HOVER,
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            [25, 101, 195, 24],
        )?;
        main.add(
            WC_LINK,
            "<a>反馈与更新 / Homepage</a>",
            HOMEPAGE,
            WS_TABSTOP,
            [237, 40, 200, 25],
        )?;
        main.add(
            w!("STATIC"),
            "已有新版本!",
            NEW_VERSION,
            0,
            [240, 65, 175, 22],
        )?;
        ShowWindow(GetDlgItem(main.hwnd, NEW_VERSION as i32), SW_HIDE);
        main.add(
            w!("BUTTON"),
            "更多设置(&S)...",
            MORE,
            WS_TABSTOP,
            [237, 99, 117, 34],
        )?;
        main.add(
            w!("BUTTON"),
            "退出(&X)",
            EXIT,
            WS_TABSTOP,
            [364, 99, 65, 34],
        )?;
        main.layout();
        set_checked(main.hwnd, CIRCLES, settings.enable_click_circle);
        set_checked(main.hwnd, HOVER, settings.enable_hover_dot);
        let overlays = Overlays::new(instance)?;
        let (sender, updates) = mpsc::channel();
        Ok(Self {
            instance,
            main,
            settings_ui: None,
            color_ui: None,
            settings,
            path,
            overlays,
            tracker: ButtonTracker::default(),
            hook: null_mut(),
            icon: LoadIconW(instance, make_int_resource(1)),
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            updates,
            update_sender: Some(sender),
            custom_colors: [0xffffff; 16],
            animation_timer_active: false,
        })
    }
    unsafe fn tray_data(&self) -> NOTIFYICONDATAW {
        let mut data: NOTIFYICONDATAW = zeroed();
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.main.hwnd;
        data.uID = 1;
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = TRAY_MESSAGE;
        data.hIcon = self.icon;
        fill_text(&mut data.szTip, "ClickShow\n鼠标点击提示器\n点击打开");
        data
    }
    unsafe fn add_tray(&self) -> Result<(), String> {
        if Shell_NotifyIconW(NIM_ADD, &self.tray_data()) == 0 {
            Err(last_error("无法添加系统托盘图标"))
        } else {
            Ok(())
        }
    }
    unsafe fn warn(&self, message: &str) {
        let mut data = self.tray_data();
        data.uFlags = NIF_INFO;
        fill_text(&mut data.szInfoTitle, "ClickShow");
        fill_text(&mut data.szInfo, message);
        data.dwInfoFlags = NIIF_WARNING;
        Shell_NotifyIconW(NIM_MODIFY, &data);
    }
    unsafe fn reveal(&self) {
        ShowWindow(self.main.hwnd, SW_RESTORE);
        let target = self
            .color_ui
            .as_ref()
            .map(|c| c.ui.hwnd)
            .or_else(|| self.settings_ui.as_ref().map(|s| s.hwnd))
            .unwrap_or(self.main.hwnd);
        SetForegroundWindow(target);
    }
    unsafe fn schedule_save(&self) {
        SetTimer(self.main.hwnd, SAVE_TIMER, 100, None);
    }
    unsafe fn save(&self) {
        if let Err(error) = settings::save(&self.path, &self.settings) {
            self.warn(&format!("设置保存出错：{error}"));
        }
    }
    unsafe fn apply_hover(&mut self) {
        let mut point = POINT::default();
        GetCursorPos(&mut point);
        if let Err(error) = self.overlays.update_hover(point, &self.settings) {
            self.warn(&error);
        }
    }
    unsafe fn open_settings(&mut self) -> Result<(), String> {
        if let Some(ui) = &self.settings_ui {
            SetForegroundWindow(ui.hwnd);
            return Ok(());
        }
        let mut ui = create_window(self.instance, self.main.hwnd, "设置", 400, 490, false)?;
        ui.add(
            w!("BUTTON"),
            "点击波纹外观设置",
            0,
            BS_GROUPBOX as u32,
            [15, 10, 370, 285],
        )?;
        ui.add(w!("STATIC"), "", INDICATOR_LABEL, 0, [30, 36, 320, 22])?;
        slider(&mut ui, INDICATOR_SIZE, 50, 400, 5, [30, 62, 335, 30])?;
        ui.add(w!("STATIC"), "各按键波纹颜色：", 0, 0, [30, 103, 320, 22])?;
        for (index, label) in ["左：", "中：", "右：", "侧键(后退)：", "侧键(前进)："]
            .iter()
            .enumerate()
        {
            let y = 133 + index as i32 * 29;
            ui.add(w!("STATIC"), label, 0, 0, [30, y + 3, 105, 23])?;
            ui.add(
                w!("BUTTON"),
                "启用",
                ENABLE_BASE + index as u16,
                WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                [138, y, 67, 25],
            )?;
            ui.add(
                w!("BUTTON"),
                "",
                COLOR_BASE + index as u16,
                WS_TABSTOP | BS_OWNERDRAW as u32,
                [220, y, 140, 25],
            )?;
        }
        ui.add(
            w!("BUTTON"),
            "位置浮标外观",
            0,
            BS_GROUPBOX as u32,
            [15, 308, 370, 118],
        )?;
        ui.add(w!("STATIC"), "", HOVER_LABEL, 0, [30, 331, 320, 22])?;
        slider(&mut ui, HOVER_SIZE, 10, 150, 5, [30, 356, 335, 30])?;
        ui.add(w!("STATIC"), "悬浮标颜色：", 0, 0, [30, 393, 130, 24])?;
        ui.add(
            w!("BUTTON"),
            "",
            HOVER_COLOR,
            WS_TABSTOP | BS_OWNERDRAW as u32,
            [220, 390, 140, 25],
        )?;
        ui.add(
            w!("BUTTON"),
            "默认值",
            DEFAULTS,
            WS_TABSTOP,
            [15, 443, 175, 33],
        )?;
        ui.add(
            w!("BUTTON"),
            "关闭(&C)",
            SETTINGS_CLOSE,
            WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            [210, 443, 175, 33],
        )?;
        ui.layout();
        let hwnd = ui.hwnd;
        self.settings_ui = Some(ui);
        self.sync_settings();
        EnableWindow(self.main.hwnd, 0);
        ShowWindow(hwnd, SW_SHOW);
        SetFocus(GetDlgItem(hwnd, INDICATOR_SIZE as i32));
        Ok(())
    }
    unsafe fn sync_settings(&self) {
        let Some(ui) = &self.settings_ui else { return };
        SendDlgItemMessageW(
            ui.hwnd,
            INDICATOR_SIZE as i32,
            TBM_SETPOS,
            1,
            self.settings.indicator_size as isize,
        );
        SendDlgItemMessageW(
            ui.hwnd,
            HOVER_SIZE as i32,
            TBM_SETPOS,
            1,
            self.settings.hover_dot_size as isize,
        );
        text(
            ui.hwnd,
            INDICATOR_LABEL,
            &format!("波纹大小：{:.0}", self.settings.indicator_size),
        );
        text(
            ui.hwnd,
            HOVER_LABEL,
            &format!("悬浮标大小：{:.0}", self.settings.hover_dot_size),
        );
        for i in 0..5 {
            set_checked(
                ui.hwnd,
                ENABLE_BASE + i as u16,
                self.settings.mouse_button_settings[i].is_enabled,
            );
            text(
                ui.hwnd,
                COLOR_BASE + i as u16,
                &self.settings.mouse_button_settings[i].color,
            );
            InvalidateRect(
                GetDlgItem(ui.hwnd, (COLOR_BASE + i as u16) as i32),
                null(),
                1,
            );
        }
        text(ui.hwnd, HOVER_COLOR, &self.settings.hover_dot_fill);
        InvalidateRect(GetDlgItem(ui.hwnd, HOVER_COLOR as i32), null(), 1);
    }
    unsafe fn close_color(&mut self) {
        if let Some(editor) = self.color_ui.take() {
            let owner = GetWindow(editor.ui.hwnd, GW_OWNER);
            EnableWindow(owner, 1);
            drop(editor);
            SetForegroundWindow(owner);
        }
    }
    unsafe fn close_settings(&mut self) {
        self.close_color();
        if let Some(ui) = self.settings_ui.take() {
            EnableWindow(self.main.hwnd, 1);
            drop(ui);
            SetForegroundWindow(self.main.hwnd);
        }
    }
    fn current_color(&self, target: usize) -> Color {
        let value = if target < 5 {
            &self.settings.mouse_button_settings[target].color
        } else {
            &self.settings.hover_dot_fill
        };
        Color::parse(value).expect("validated settings")
    }
    unsafe fn open_color(&mut self, target: usize) -> Result<(), String> {
        let Some(parent) = &self.settings_ui else {
            return Ok(());
        };
        let owner = parent.hwnd;
        let mut ui = create_window(self.instance, owner, "颜色", 400, 218, false)?;
        ui.add(
            w!("STATIC"),
            "颜色值（#AARRGGBB）：",
            0,
            0,
            [20, 18, 260, 22],
        )?;
        let edit = ui.add(
            w!("EDIT"),
            "",
            COLOR_TEXT,
            WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
            [20, 46, 218, 28],
        )?;
        SendMessageW(edit, EM_SETLIMITTEXT, 32, 0);
        ui.add(
            w!("BUTTON"),
            "选择颜色...",
            COLOR_PICK,
            WS_TABSTOP,
            [250, 45, 130, 30],
        )?;
        ui.add(w!("STATIC"), "", ALPHA_LABEL, 0, [20, 92, 350, 23])?;
        slider(&mut ui, ALPHA_SLIDER, 0, 255, 1, [20, 120, 360, 30])?;
        ui.add(
            w!("BUTTON"),
            "关闭(&C)",
            COLOR_CLOSE,
            WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            [250, 171, 130, 32],
        )?;
        ui.layout();
        let hwnd = ui.hwnd;
        self.color_ui = Some(ColorEditor { ui, target });
        self.sync_color();
        EnableWindow(owner, 0);
        ShowWindow(hwnd, SW_SHOW);
        SetFocus(edit);
        SendMessageW(edit, EM_SETSEL, 0, -1);
        Ok(())
    }
    unsafe fn sync_color(&self) {
        let Some(editor) = &self.color_ui else { return };
        let color = self.current_color(editor.target);
        text(editor.ui.hwnd, COLOR_TEXT, &color.to_argb());
        text(
            editor.ui.hwnd,
            ALPHA_LABEL,
            &format!(
                "不透明度：{:.0}%（Alpha {}）",
                color.a as f64 / 255.0 * 100.0,
                color.a
            ),
        );
        SendDlgItemMessageW(
            editor.ui.hwnd,
            ALPHA_SLIDER as i32,
            TBM_SETPOS,
            1,
            color.a as isize,
        );
    }
    unsafe fn set_color(&mut self, color: Color, refresh_text: bool) {
        let Some(editor) = &self.color_ui else { return };
        if editor.target < 5 {
            self.settings.mouse_button_settings[editor.target].color = color.to_argb();
        } else {
            self.settings.hover_dot_fill = color.to_argb();
        }
        self.schedule_save();
        self.sync_settings();
        if refresh_text {
            self.sync_color();
        } else if let Some(editor) = &self.color_ui {
            text(
                editor.ui.hwnd,
                ALPHA_LABEL,
                &format!(
                    "不透明度：{:.0}%（Alpha {}）",
                    color.a as f64 / 255.0 * 100.0,
                    color.a
                ),
            );
            SendDlgItemMessageW(
                editor.ui.hwnd,
                ALPHA_SLIDER as i32,
                TBM_SETPOS,
                1,
                color.a as isize,
            );
        }
    }
    unsafe fn draw_color(&self, draw: &DRAWITEMSTRUCT) {
        let target = if draw.CtlID == HOVER_COLOR as u32 {
            5
        } else {
            (draw.CtlID - COLOR_BASE as u32) as usize
        };
        if target > 5 {
            return;
        }
        let color = self.current_color(target);
        let rect = draw.rcItem;
        let cell = (GetDpiForWindow(draw.hwndItem).max(96) / 12) as i32;
        let mut y = rect.top;
        while y < rect.bottom {
            let mut x = rect.left;
            while x < rect.right {
                let bg = if ((x - rect.left) / cell + (y - rect.top) / cell) % 2 == 0 {
                    245u32
                } else {
                    205u32
                };
                let blend = |v: u8| (v as u32 * color.a as u32 + bg * (255 - color.a as u32)) / 255;
                let brush = CreateSolidBrush(
                    blend(color.r) | (blend(color.g) << 8) | (blend(color.b) << 16),
                );
                let tile = RECT {
                    left: x,
                    top: y,
                    right: (x + cell).min(rect.right),
                    bottom: (y + cell).min(rect.bottom),
                };
                FillRect(draw.hDC, &tile, brush);
                DeleteObject(brush);
                x += cell;
            }
            y += cell;
        }
        let luminance = (color.r as u32 * 299 + color.g as u32 * 587 + color.b as u32 * 114) / 1000;
        let effective = (luminance * color.a as u32 + 225 * (255 - color.a as u32)) / 255;
        SetTextColor(draw.hDC, if effective > 135 { 0 } else { 0xffffff });
        SetBkMode(draw.hDC, TRANSPARENT as i32);
        let previous = SelectObject(
            draw.hDC,
            SendMessageW(draw.hwndItem, WM_GETFONT, 0, 0) as HGDIOBJ,
        );
        let mut label_rect = rect;
        DrawTextW(
            draw.hDC,
            wide(&color.to_argb()).as_ptr(),
            -1,
            &mut label_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(draw.hDC, previous);
        FrameRect(draw.hDC, &rect, GetSysColorBrush(COLOR_BTNSHADOW));
        if draw.itemState & ODS_FOCUS != 0 {
            let mut r = rect;
            InflateRect(&mut r, -3, -3);
            DrawFocusRect(draw.hDC, &r);
        }
    }
    unsafe fn input(&mut self) {
        let events = INPUT.with(|queue| std::mem::take(&mut *queue.borrow_mut()));
        for event in events {
            if event.message == WM_MOUSEMOVE {
                if self.settings.enable_hover_dot
                    && let Err(error) = self.overlays.update_hover(event.point, &self.settings)
                {
                    self.warn(&error);
                }
                continue;
            }
            let button = match event.message {
                WM_LBUTTONDOWN | WM_LBUTTONUP => MouseButton::Left,
                WM_MBUTTONDOWN | WM_MBUTTONUP => MouseButton::Middle,
                WM_RBUTTONDOWN | WM_RBUTTONUP => MouseButton::Right,
                _ => {
                    if (event.data >> 16) & 0xffff == 2 {
                        MouseButton::XButton2
                    } else {
                        MouseButton::XButton1
                    }
                }
            };
            let config = &self.settings.mouse_button_settings[button.index()];
            if !self.settings.enable_click_circle || !config.is_enabled {
                continue;
            }
            let down = matches!(
                event.message,
                WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONDOWN | WM_XBUTTONDOWN
            );
            let show = if down {
                self.tracker
                    .on_down(button, event.point.x, event.point.y, event.time);
                true
            } else {
                self.tracker
                    .on_up(button, event.point.x, event.point.y, event.time)
            };
            if show && let Ok(color) = Color::parse(&config.color) {
                if let Err(error) =
                    self.overlays
                        .click(event.point, color, self.settings.indicator_size, down)
                {
                    self.warn(&error);
                }
                if !self.animation_timer_active {
                    self.animation_timer_active =
                        SetTimer(self.main.hwnd, ANIMATION_TIMER, 16, None) != 0;
                }
            }
        }
    }
    unsafe fn dispatch(
        &mut self,
        hwnd: HWND,
        message: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> Option<Effect> {
        if message == self.taskbar_created && hwnd == self.main.hwnd {
            if let Err(error) = self.add_tray() {
                return Some(Effect::Error(error));
            }
            return Some(Effect::None);
        }
        match message {
            WM_CLOSE => {
                if self.color_ui.as_ref().is_some_and(|c| c.ui.hwnd == hwnd) {
                    self.close_color();
                } else if self.settings_ui.as_ref().is_some_and(|u| u.hwnd == hwnd) {
                    self.close_settings();
                } else {
                    ShowWindow(hwnd, SW_HIDE);
                }
            }
            WM_SIZE if hwnd == self.main.hwnd && wp == SIZE_MINIMIZED as usize => {
                ShowWindow(hwnd, SW_HIDE);
            }
            WM_DPICHANGED => {
                let rect = &*(lp as *const RECT);
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                if hwnd == self.main.hwnd {
                    self.main.layout();
                } else if let Some(ui) = &mut self.settings_ui
                    && ui.hwnd == hwnd
                {
                    ui.layout();
                }
                if let Some(editor) = &mut self.color_ui
                    && editor.ui.hwnd == hwnd
                {
                    editor.ui.layout();
                }
            }
            WM_COMMAND => {
                let id = (wp & 0xffff) as u16;
                let notification = (wp >> 16) as u16;
                if self.color_ui.as_ref().is_some_and(|c| c.ui.hwnd == hwnd) {
                    match id {
                        COLOR_CLOSE | 1 | 2 => self.close_color(),
                        COLOR_PICK => {
                            return Some(Effect::PickColor(
                                hwnd,
                                self.current_color(self.color_ui.as_ref().unwrap().target),
                            ));
                        }
                        COLOR_TEXT if notification == EN_CHANGE as u16 => {
                            let mut buffer = [0u16; 64];
                            let len = GetDlgItemTextW(
                                hwnd,
                                COLOR_TEXT as i32,
                                buffer.as_mut_ptr(),
                                buffer.len() as i32,
                            );
                            if let Ok(color) =
                                Color::parse(&String::from_utf16_lossy(&buffer[..len as usize]))
                            {
                                self.set_color(color, false);
                            }
                        }
                        _ => {}
                    }
                } else if self.settings_ui.as_ref().is_some_and(|u| u.hwnd == hwnd) {
                    match id {
                        SETTINGS_CLOSE | 1 | 2 => self.close_settings(),
                        DEFAULTS => {
                            self.settings.restore_appearance_defaults();
                            self.sync_settings();
                            self.schedule_save();
                        }
                        ENABLE_BASE..=214 => {
                            let i = (id - ENABLE_BASE) as usize;
                            self.settings.mouse_button_settings[i].is_enabled = checked(hwnd, id);
                            self.schedule_save();
                        }
                        COLOR_BASE..=224 => {
                            if let Err(error) = self.open_color((id - COLOR_BASE) as usize) {
                                return Some(Effect::Error(error));
                            }
                        }
                        HOVER_COLOR => {
                            if let Err(error) = self.open_color(5) {
                                return Some(Effect::Error(error));
                            }
                        }
                        _ => {}
                    }
                } else if hwnd == self.main.hwnd {
                    match id {
                        STARTUP => {
                            if let Err(error) = platform::set_autostart(checked(hwnd, STARTUP)) {
                                set_checked(
                                    hwnd,
                                    STARTUP,
                                    platform::autostart_enabled().unwrap_or(false),
                                );
                                return Some(Effect::Error(error));
                            }
                        }
                        CIRCLES => {
                            self.settings.enable_click_circle = checked(hwnd, CIRCLES);
                            self.schedule_save();
                        }
                        HOVER => {
                            self.settings.enable_hover_dot = checked(hwnd, HOVER);
                            self.schedule_save();
                        }
                        MORE => {
                            if let Err(error) = self.open_settings() {
                                return Some(Effect::Error(error));
                            }
                        }
                        EXIT => {
                            DestroyWindow(self.main.hwnd);
                        }
                        _ => {}
                    }
                }
            }
            WM_HSCROLL => {
                let id = GetDlgCtrlID(lp as HWND) as u16;
                let position = SendMessageW(lp as HWND, TBM_GETPOS, 0, 0) as i32;
                if id == ALPHA_SLIDER {
                    if let Some(editor) = &self.color_ui {
                        let mut color = self.current_color(editor.target);
                        color.a = position as u8;
                        self.set_color(color, true);
                    }
                } else {
                    let value = ((position + 2) / 5 * 5) as f64;
                    if id == INDICATOR_SIZE {
                        self.settings.indicator_size = value;
                    } else if id == HOVER_SIZE {
                        self.settings.hover_dot_size = value;
                    }
                    self.sync_settings();
                    self.schedule_save();
                }
            }
            WM_NOTIFY => {
                let header = &*(lp as *const NMHDR);
                if header.idFrom == HOMEPAGE as usize && matches!(header.code, NM_CLICK | NM_RETURN)
                {
                    return Some(Effect::OpenUrl(
                        hwnd,
                        "https://github.com/cuiliang/clickshow".into(),
                    ));
                }
            }
            WM_DRAWITEM => {
                let draw = &*(lp as *const DRAWITEMSTRUCT);
                if (COLOR_BASE as u32..=224).contains(&draw.CtlID)
                    || draw.CtlID == HOVER_COLOR as u32
                {
                    self.draw_color(draw);
                } else {
                    return None;
                }
            }
            INPUT_MESSAGE => self.input(),
            WM_TIMER if hwnd == self.main.hwnd => match wp {
                ANIMATION_TIMER => {
                    if let Err(error) = self.overlays.tick() {
                        self.warn(&error);
                    }
                    if !self.overlays.has_animation() {
                        KillTimer(hwnd, ANIMATION_TIMER);
                        self.animation_timer_active = false;
                    }
                }
                SAVE_TIMER => {
                    KillTimer(hwnd, SAVE_TIMER);
                    self.save();
                    self.apply_hover();
                }
                UPDATE_TIMER => {
                    KillTimer(hwnd, UPDATE_TIMER);
                    if let Some(sender) = self.update_sender.take() {
                        let target = hwnd as usize;
                        std::thread::spawn(move || {
                            if let Some(info) = platform::check_update()
                                && sender.send(info).is_ok()
                            {
                                PostMessageW(target as HWND, UPDATE_MESSAGE, 0, 0);
                            }
                        });
                    }
                }
                _ => {}
            },
            UPDATE_MESSAGE => {
                if let Ok(update) = self.updates.try_recv() {
                    if self.settings.last_notified_version.as_deref() == Some(&update.raw) {
                        ShowWindow(GetDlgItem(self.main.hwnd, NEW_VERSION as i32), SW_SHOW);
                    } else {
                        self.settings.last_notified_version = Some(update.raw.clone());
                        self.save();
                        return Some(Effect::Update(update));
                    }
                }
            }
            TRAY_MESSAGE => match lp as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => self.reveal(),
                WM_RBUTTONUP | WM_CONTEXTMENU => {
                    self.reveal();
                    let mut point = POINT::default();
                    GetCursorPos(&mut point);
                    return Some(Effect::TrayMenu(point));
                }
                _ => {}
            },
            _ => return None,
        }
        Some(Effect::None)
    }
}
impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            MAIN.store(null_mut(), Ordering::Relaxed);
            if !self.hook.is_null() {
                UnhookWindowsHookEx(self.hook);
            }
            for timer in [ANIMATION_TIMER, SAVE_TIMER, UPDATE_TIMER] {
                KillTimer(self.main.hwnd, timer);
            }
            self.save();
            Shell_NotifyIconW(NIM_DELETE, &self.tray_data());
            self.close_settings();
        }
    }
}

unsafe fn effect(action: Effect) {
    match action {
        Effect::None => {}
        Effect::Error(error) => show_error(&error),
        Effect::OpenUrl(owner, url) => {
            if let Err(error) = platform::open_url(owner, &url) {
                show_error(&error);
            }
        }
        Effect::PickColor(owner, color) => {
            let mut colors = APP.with(|app| {
                app.borrow()
                    .as_ref()
                    .map(|a| a.custom_colors)
                    .unwrap_or([0; 16])
            });
            let mut picker: CHOOSECOLORW = zeroed();
            picker.lStructSize = size_of::<CHOOSECOLORW>() as u32;
            picker.hwndOwner = owner;
            picker.rgbResult = color.r as u32 | ((color.g as u32) << 8) | ((color.b as u32) << 16);
            picker.lpCustColors = colors.as_mut_ptr();
            picker.Flags = CC_RGBINIT | CC_FULLOPEN;
            let accepted = ChooseColorW(&mut picker) != 0;
            APP.with(|app| {
                if let Some(app) = app.borrow_mut().as_mut() {
                    app.custom_colors = colors;
                    if accepted {
                        app.set_color(
                            Color {
                                a: color.a,
                                r: picker.rgbResult as u8,
                                g: (picker.rgbResult >> 8) as u8,
                                b: (picker.rgbResult >> 16) as u8,
                            },
                            true,
                        );
                    }
                }
            });
        }
        Effect::Update(update) => {
            let owner = MAIN.load(Ordering::Relaxed);
            if MessageBoxW(
                owner,
                wide(&format!(
                    "ClickShow有新版本（{}），是否立即打开网页？",
                    update.version
                ))
                .as_ptr(),
                w!("ClickShow"),
                MB_YESNO | MB_ICONINFORMATION,
            ) == IDYES
                && let Err(error) = platform::open_url(owner, &update.url)
            {
                show_error(&error);
            }
        }
        Effect::TrayMenu(point) => {
            let hwnd = MAIN.load(Ordering::Relaxed);
            let menu = CreatePopupMenu();
            if !menu.is_null() {
                AppendMenuW(menu, MF_STRING, EXIT as usize, w!("退出(Exit)"));
                SetForegroundWindow(hwnd);
                let selected = TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                    point.x,
                    point.y,
                    0,
                    hwnd,
                    null(),
                );
                DestroyMenu(menu);
                if selected == EXIT as i32 {
                    PostMessageW(hwnd, WM_COMMAND, EXIT as usize, 0);
                }
                PostMessageW(hwnd, WM_NULL, 0, 0);
            }
        }
    }
}
unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if message == WM_CTLCOLORSTATIC && GetDlgCtrlID(lp as HWND) == NEW_VERSION as i32 {
        SetTextColor(wp as HDC, 0x0045ff);
        SetBkMode(wp as HDC, TRANSPARENT as i32);
        return GetSysColorBrush(COLOR_BTNFACE) as LRESULT;
    }
    if message == WM_DESTROY && hwnd == MAIN.load(Ordering::Relaxed) {
        PostQuitMessage(0);
        return 0;
    }
    let result = APP.with(|cell| {
        let Ok(mut slot) = cell.try_borrow_mut() else {
            return None;
        };
        slot.as_mut()
            .and_then(|app| app.dispatch(hwnd, message, wp, lp))
    });
    if let Some(action) = result {
        effect(action);
        return if message == WM_DRAWITEM { 1 } else { 0 };
    }
    DefWindowProcW(hwnd, message, wp, lp)
}
pub fn run() -> Result<(), String> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        START.get_or_init(Instant::now);
        let (mut path, mut show, mut no_update) = (None, false, false);
        let mut args = std::env::args_os().skip(1);
        while let Some(argument) = args.next() {
            match argument.to_str() {
                Some("--show") => show = true,
                Some("--no-update") => no_update = true,
                Some("--config") => {
                    path = Some(PathBuf::from(
                        args.next().ok_or("--config 需要配置文件路径")?,
                    ))
                }
                _ => return Err(format!("未知参数：{}", argument.to_string_lossy())),
            }
        }
        let path = match path {
            Some(path) => path,
            None => platform::settings_path()?,
        };
        let (settings, load_error) = match settings::load(&path) {
            Ok(settings) => (settings, None),
            Err(error) => (AppSettings::default(), Some(error)),
        };
        let mut app = App::new(path, settings)?;
        MAIN.store(app.main.hwnd, Ordering::Relaxed);
        // Explorer broadcasts this registered message at a lower integrity level
        // when ClickShow is run as administrator. Allow only this tray-recovery message.
        ChangeWindowMessageFilterEx(app.main.hwnd, app.taskbar_created, MSGFLT_ALLOW, null_mut());
        app.add_tray()?;
        app.hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), app.instance, 0);
        if app.hook.is_null() {
            return Err(last_error("鼠标挂钩建立失败"));
        }
        let startup_error = match platform::autostart_enabled() {
            Ok(enabled) => {
                set_checked(app.main.hwnd, STARTUP, enabled);
                None
            }
            Err(error) => Some(error),
        };
        app.apply_hover();
        if !no_update {
            SetTimer(app.main.hwnd, UPDATE_TIMER, 10000, None);
        }
        if show {
            app.reveal();
        }
        APP.with(|slot| *slot.borrow_mut() = Some(app));
        if let Some(error) = load_error {
            show_error(&format!("配置文件损坏了，设置已恢复为默认值。{error}"));
            APP.with(|slot| {
                if let Some(app) = slot.borrow_mut().as_mut() {
                    app.save();
                }
            });
        }
        if let Some(error) = startup_error {
            show_error(&format!("无法加载开机自动启动状态。{error}"));
        }
        let mut message = MSG::default();
        let outcome = loop {
            let status = GetMessageW(&mut message, null_mut(), 0, 0);
            if status == 0 {
                break Ok(());
            }
            if status == -1 {
                break Err(last_error("窗口消息循环失败"));
            }
            let dialog = APP.with(|slot| {
                slot.borrow().as_ref().map(|app| {
                    app.color_ui
                        .as_ref()
                        .map(|c| c.ui.hwnd)
                        .or_else(|| app.settings_ui.as_ref().map(|s| s.hwnd))
                        .unwrap_or(app.main.hwnd)
                })
            });
            if let Some(hwnd) = dialog {
                if message.message == WM_KEYDOWN
                    && (message.wParam == VK_ESCAPE as usize
                        || message.wParam == VK_RETURN as usize)
                    && hwnd != MAIN.load(Ordering::Relaxed)
                {
                    SendMessageW(hwnd, WM_CLOSE, 0, 0);
                    continue;
                }
                if IsDialogMessageW(hwnd, &message) != 0 {
                    continue;
                }
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        };
        let app = APP.with(|slot| slot.borrow_mut().take());
        drop(app);
        outcome
    }
}
