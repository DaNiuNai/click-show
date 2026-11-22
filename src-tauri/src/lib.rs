mod input;
#[cfg(target_os = "macos")]
#[path = "macos/mod.rs"]
mod native;
#[cfg(windows)]
#[path = "windows/mod.rs"]
mod native;
mod settings;
mod update;

use serde::Serialize;
use settings::{AppSettings, Color};
use std::{path::PathBuf, sync::Mutex, time::Duration};
use tauri::{
    AppHandle, Emitter, Manager, State, WindowEvent,
    menu::MenuBuilder,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
#[cfg(target_os = "macos")]
use tauri_plugin_autostart::ManagerExt;

#[derive(Default)]
struct Args {
    show: bool,
    no_update: bool,
    config: Option<PathBuf>,
}

impl Args {
    fn read() -> Result<Self, String> {
        let mut result = Self::default();
        let mut args = std::env::args_os().skip(1);
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--show") => result.show = true,
                Some("--no-update") => result.no_update = true,
                Some("--config") => {
                    result.config = Some(args.next().ok_or("--config 需要配置文件路径")?.into())
                }
                #[cfg(target_os = "macos")]
                Some(arg) if arg.starts_with("-psn_") => (),
                _ => return Err(format!("未知参数：{}", arg.to_string_lossy())),
            }
        }
        Ok(result)
    }
}

struct Model {
    path: PathBuf,
    settings: AppSettings,
    update: Option<update::UpdateInfo>,
    warning: Option<String>,
}

#[derive(Serialize)]
struct View {
    settings: AppSettings,
    update: Option<update::UpdateInfo>,
    warning: Option<String>,
}

#[tauri::command]
fn preferences(model: State<'_, Mutex<Model>>) -> Result<View, String> {
    let model = model.lock().map_err(|e| e.to_string())?;
    let mut settings = model.settings.clone();
    // Legacy files also accept named and shorthand colors. The color picker needs ARGB.
    settings.hover_dot_fill = Color::parse(&settings.hover_dot_fill)?.to_argb();
    for button in &mut settings.mouse_button_settings {
        button.color = Color::parse(&button.color)?.to_argb();
    }
    Ok(View {
        settings,
        update: model.update.clone(),
        warning: model.warning.clone(),
    })
}

fn change_preferences(
    app: &AppHandle,
    model: &Mutex<Model>,
    change: impl FnOnce(&mut AppSettings),
) -> Result<AppSettings, String> {
    let mut model = model.lock().map_err(|e| e.to_string())?;
    let mut next = model.settings.clone();
    change(&mut next);
    settings::save(&model.path, &next)?;
    model.settings = next.clone();
    drop(model);
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let model = handle.state::<Mutex<Model>>();
        let mut model = model.lock().unwrap();
        if let Err(error) = native::apply(&model.settings) {
            model.warning = Some(error.clone());
            let _ = handle.emit("settings-warning", error);
        }
    })
    .map_err(|e| e.to_string())?;
    Ok(next)
}

#[tauri::command]
fn save_preferences(
    app: AppHandle,
    model: State<'_, Mutex<Model>>,
    mut settings: AppSettings,
) -> Result<(), String> {
    change_preferences(&app, &model, |current| {
        settings.last_notified_version = current.last_notified_version.take();
        *current = settings;
    })
    .map(|_| ())
}

#[tauri::command]
fn restore_defaults(app: AppHandle, model: State<'_, Mutex<Model>>) -> Result<AppSettings, String> {
    change_preferences(&app, &model, AppSettings::restore_appearance_defaults)
}

#[tauri::command]
fn autostart_enabled(_app: AppHandle) -> Result<bool, String> {
    #[cfg(windows)]
    return native::autostart::autostart_enabled();
    #[cfg(target_os = "macos")]
    return _app.autolaunch().is_enabled().map_err(|e| e.to_string());
}

#[tauri::command]
fn set_autostart(_app: AppHandle, enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    return native::autostart::set_autostart(enabled);
    #[cfg(target_os = "macos")]
    {
        let launch = _app.autolaunch();
        if enabled {
            launch.enable()
        } else {
            launch.disable()
        }
        .map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

fn reveal(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn schedule_update(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(10));
        let Some(update) = update::check_update() else {
            return;
        };
        let state = app.state::<Mutex<Model>>();
        let mut model = state.lock().unwrap();
        let first_notice = model.settings.last_notified_version.as_ref() != Some(&update.raw);
        if first_notice {
            let mut next = model.settings.clone();
            next.last_notified_version = Some(update.raw.clone());
            match settings::save(&model.path, &next) {
                Ok(()) => model.settings = next,
                Err(error) => {
                    model.warning = Some(error.clone());
                    let _ = app.emit("settings-warning", error);
                }
            }
        }
        model.update = Some(update.clone());
        drop(model);
        let _ = app.emit("update-available", update);
        if first_notice {
            reveal(&app);
        }
    });
}

fn config_path(app: &AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
    #[cfg(windows)]
    let directory = app.path().document_dir()?;
    #[cfg(target_os = "macos")]
    let directory = app.path().app_config_dir()?;
    std::fs::create_dir_all(&directory)?;
    Ok(directory.join("ClickShow.setting"))
}

pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(
        tauri_plugin_autostart::Builder::new()
            .app_name("ClickShow")
            .build(),
    );
    builder
        .invoke_handler(tauri::generate_handler![
            preferences,
            save_preferences,
            restore_defaults,
            autostart_enabled,
            set_autostart,
            quit
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let args = Args::read()?;
            let path = match args.config {
                Some(path) => path,
                None => config_path(app.handle())?,
            };
            let (settings, mut warning) = match settings::load(&path) {
                Ok(settings) => (settings, None),
                Err(error) => (
                    AppSettings::default(),
                    Some(format!("已加载默认设置。{error}")),
                ),
            };
            if let Err(error) = native::start(&settings) {
                warning = Some(
                    [warning, Some(error)]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
            let show = args.show || warning.is_some();
            app.manage(Mutex::new(Model {
                path,
                settings,
                update: None,
                warning,
            }));
            let menu = MenuBuilder::new(app)
                .text("settings", "打开设置")
                .separator()
                .text("quit", "退出 ClickShow")
                .build()?;
            TrayIconBuilder::with_id("clickshow")
                .icon(app.default_window_icon().ok_or("缺少应用图标")?.clone())
                .tooltip("ClickShow")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "settings" => reveal(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        reveal(tray.app_handle());
                    }
                })
                .build(app)?;
            if show {
                reveal(app.handle());
            }
            if !args.no_update {
                schedule_update(app.handle().clone());
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::Resized(_) if window.is_minimized().unwrap_or(false) => {
                let _ = window.hide();
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("ClickShow 初始化失败")
        .run(|_, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                native::stop();
            }
        });
}
