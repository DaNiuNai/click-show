fn main() {
    println!("cargo:rerun-if-changed=assets/clickshow.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    println!("cargo:rerun-if-changed=assets/app-ui-access.manifest");
    let manifest = if cfg!(feature = "ui-access") {
        "assets/app-ui-access.manifest"
    } else {
        "assets/app.manifest"
    };
    winresource::WindowsResource::new()
        .set_icon("assets/clickshow.ico")
        .set_manifest_file(manifest)
        .set("ProductName", "ClickShow")
        .set("FileDescription", "ClickShow 鼠标点击提示器")
        .compile()
        .expect("无法编译 Windows 图标与应用清单");
}
