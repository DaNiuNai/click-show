# ClickShow

ClickShow 在鼠标按下时显示彩色波纹，也可以用半透明浮标标出指针的位置。Windows 和 macOS 共用 React + TypeScript 设置界面。Windows 使用 Win32 分层窗口，macOS 使用 AppKit 面板。两端的提示都支持鼠标穿透和多显示器。

## 开发

工程使用官方 `create-tauri-app` 的 `react-ts` 模板创建。前端在 `src/`，Rust 后端、平台代码和打包配置在 `src-tauri/`。[Tauri 建项文档](https://v2.tauri.app/start/create-project/)

安装 Node.js 24、pnpm 和 Rust stable。Windows 还需安装 Visual Studio C++ 构建工具、Windows SDK 和 WebView2 Runtime。macOS 需安装 Xcode Command Line Tools，可运行 `xcode-select --install`。[Tauri 开发环境要求](https://v2.tauri.app/start/prerequisites/)

在项目根目录运行以下命令。

```sh
pnpm install --frozen-lockfile
pnpm tauri dev
```

应用默认进入 Windows 托盘或 macOS 菜单栏。单击 ClickShow 图标打开设置。关闭设置窗口后提示效果会继续运行，通过窗口或图标菜单中的“退出 ClickShow”结束程序。

开发时可以直接打开设置并关闭本次上游版本检查。使用绝对路径的 `--config` 可隔离配置文件。

```sh
pnpm exec tauri dev -- -- --show --no-update
```

## 打包

在目标操作系统上执行 `pnpm tauri build`。Windows 的 MSI 和 NSIS 安装包位于 `src-tauri/target/release/bundle/msi/` 和 `nsis/` 子目录。macOS 的 `.app` 和 `.dmg` 位于对应的 `bundle/macos/` 和 `bundle/dmg/` 子目录。[Windows 安装包](https://v2.tauri.app/distribute/windows-installer/) · [macOS 应用包](https://v2.tauri.app/distribute/macos-application-bundle/)

在 Mac 上执行以下命令，可以生成同时支持 Apple Silicon 和 Intel 的通用安装包。

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
pnpm tauri build --target universal-apple-darwin
```

通用包位于 `src-tauri/target/universal-apple-darwin/release/bundle/`。仓库的 GitHub Actions 工作流在 Windows 和 macOS 运行测试并构建安装包，产物保存在该次运行的 Artifacts 中。[双平台构建工作流](.github/workflows/desktop.yml)

Windows 普通构建使用 `asInvoker`。保留了可选的 `ui-access` 特性，需要时可运行 `pnpm tauri build --features ui-access --no-bundle`。部署 UIAccess 版本需要可信的 Authenticode 签名和受保护的安装目录。[Microsoft UIAccess 部署要求](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-securityoverview#uiaccess-requirements-for-assistive-technology-applications)

## 鼠标提示与设置

| 功能 | 默认值与行为 |
| --- | --- |
| 点击波纹 | 默认开启。按下时扩散，长按超过 500 ms 或拖动超过 200 个物理像素后，抬起时再显示小波纹。macOS 以按下时显示器的缩放比例换算拖动距离。 |
| 鼠标按键 | 左、中、右、后退、前进五键可以独立启用和配色。 |
| 波纹大小 | 默认 150 DIP，可调 50–400 DIP。 |
| 位置浮标 | 默认关闭，开启后跟随指针。默认 60 DIP，可调 10–150 DIP，默认颜色为 `#60FFFF5A`。 |
| 颜色 | 点击色块选择 RGB，在旁边的 `#AARRGGBB` 输入框可修改透明度。 |
| 恢复默认外观 | 重置大小、颜色和逐键设置，保留全局提示开关与登录启动选项。 |
| 版本提醒 | 启动约 10 秒后检查原 ClickShow 项目的版本服务。有新版本时可打开原项目的发布页。 |

设置修改后自动保存。Windows 沿用“文档”已知文件夹中的 `ClickShow.setting`，兼容旧版 JSON 字段和按键键值。macOS 使用 `~/Library/Application Support/io.github.daniunai.clickshow/ClickShow.setting`。两端都支持 `--config <文件路径>`、`--show` 和 `--no-update`。

Windows 登录启动继续使用当前用户注册表项 `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run` 中名为 `ClickShow` 的值。macOS 使用 Tauri autostart 插件注册当前用户的 Launch Agent。macOS 使用 AppKit 同时观察应用内和其他应用的鼠标事件。如果系统提示需要输入监控权限，可在“系统设置 → 隐私与安全性 → 输入监控”中授权并重启应用。[Apple 事件监听说明](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/EventOverview/MonitoringEvents/MonitoringEvents.html)

## 检查

```sh
pnpm build
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
```

桌面验收可使用隔离设置文件。核对五键点击、长按和拖动后的抬起波纹，开启浮标后跨不同缩放的显示器移动，检查提示中心、鼠标穿透和前台窗口焦点。最后检查设置保存、恢复默认、收起窗口、从托盘或菜单栏重新打开，以及退出后所有提示窗口消失。

本项目沿用 [cuiliang/ClickShow](https://github.com/cuiliang/ClickShow) 的功能和设置格式。许可证为 [MIT](LICENSE)。
