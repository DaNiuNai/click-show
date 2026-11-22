fn main() {
    let manifest = if cfg!(feature = "ui-access") {
        include_str!("assets/app-ui-access.manifest")
    } else {
        include_str!("assets/app.manifest")
    };
    let windows = tauri_build::WindowsAttributes::new().app_manifest(manifest);
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("无法构建 Tauri 应用资源");
}
