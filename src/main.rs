#![windows_subsystem = "windows"]

mod input;
mod overlay;
mod platform;
mod settings;
mod ui;

fn main() {
    if let Err(error) = ui::run() {
        ui::show_error(&error);
    }
}
