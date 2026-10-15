# ClickShow Rust

参考 `参考ClickShow/` 中的 ClickShow 1.4.1，以 Rust 和 Win32 API 重写的鼠标点击、位置提示器，面向 Windows 10 1703 及以上 / Windows 11。

以原项目功能一致为目标。主窗口和设置窗口使用 Win32 原生控件重新构建，与原 WPF 界面并非逐像素复制；字体、控件样式和颜色选择交互会采用原生实现。

## 功能与默认设置

- 左、中、右键及两个侧键分别显示彩色波纹，可单独启用和设置颜色。
- 按下显示扩散波纹；按住超过 500ms，或按下到抬起的位置距离超过 200 个物理像素时，抬起额外显示小波纹。两个阈值均为严格大于。
- 可开启跟随鼠标的位置浮标，单独设置大小和透明颜色。
- 波纹和浮标置顶显示、不抢焦点、鼠标事件穿透，并按显示器 DPI 调整大小。
- 提供托盘、当前用户开机启动、设置自动保存和启动后更新检查。

| 参数 | 默认值 | 可调范围 |
| --- | --- | --- |
| 显示点击特效 | 开启 | 开 / 关 |
| 显示位置提示浮标 | 关闭 | 开 / 关 |
| 五个鼠标按键 | 全部启用 | 各键独立开 / 关 |
| 波纹大小 | 150 DIP | 50～400，步长 5 |
| 浮标大小 | 60 DIP | 10～150，步长 5 |
| 浮标颜色 | `#60FFFF5A` | ARGB |

| 按键 | 默认波纹颜色 |
| --- | --- |
| 左键 | `#FF1E90FF`（DodgerBlue） |
| 中键 | `#FF008000`（Green） |
| 右键 | `#FFFF4500`（OrangeRed） |
| 后退侧键 XButton1 | `#FF808080`（Gray） |
| 前进侧键 XButton2 | `#FF8A2BE2`（BlueViolet） |

颜色采用 `#AARRGGBB`，`AA` 为透明度：`00` 完全透明，`FF` 完全不透明。DIP 为逻辑尺寸，例如 150 DIP 在 150% 缩放屏幕上对应 225 个物理像素。

## 构建与检查

在 Windows 上安装 Rust MSVC 工具链，以及 Visual Studio Build Tools 的 C++ 构建工具和 Windows SDK。在项目根目录使用 PowerShell 执行：

```powershell
cargo build --release
cargo test
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

可执行文件为 `target/release/click-show.exe`：

```powershell
.\target\release\click-show.exe
```

## 使用

默认启动后收起到系统托盘。点击托盘图标打开主窗口，点击主窗口的 X 或最小化按钮会隐藏窗口。通过主窗口“退出”按钮或托盘菜单“退出(Exit)”结束程序并关闭所有提示效果。

主窗口可切换点击特效、位置浮标和开机自动启动。“更多设置”可调整大小、逐键开关和颜色。修改自动应用并保存，不需要点击“确定”。“默认值”恢复外观参数及逐键设置，保留主窗口中的全局点击、浮标开关和开机启动状态。

开机启动使用当前用户的注册表 `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run`，值名为 `ClickShow`，记录带双引号的当前可执行文件路径。移动程序后需要重新勾选开机启动以更新路径。

默认设置文件位于 Windows“文档”已知文件夹下的 `ClickShow.setting`，即 `Documents/ClickShow.setting`；文档目录可能被重定向。保留原项目 JSON 字段与鼠标按键数值键的格式，以兼容原有设置。原版和重写版共用这一默认设置文件，建议先退出另一版本再运行。

启动约十秒后检查原项目的版本服务；发现新版本时提示，已提醒版本记录在设置中。网络或版本解析失败时忽略该次检查。

可用下列参数进行隔离验证：

| 参数 | 作用 |
| --- | --- |
| `--show` | 启动时直接显示主窗口 |
| `--no-update` | 禁用本次运行的更新检查 |
| `--config <path>` | 使用指定设置文件，避免覆盖默认文档目录中的设置 |

```powershell
.\target\release\click-show.exe --show --no-update --config .\target\manual-test.setting
```

`--config` 仅指定设置文件；开机启动选项仍操作当前用户的上述注册表项。隔离验证过程中，可保持开机启动选项原有状态。

## 权限与 UIAccess

普通构建使用 `asInvoker` 和 `uiAccess="false"`，继承启动进程的权限。如需在以管理员身份运行的窗口上显示效果，可退出程序后，右键可执行文件选择“以管理员身份运行”。`asInvoker` 的行为见 Microsoft 的[应用程序清单文档](https://learn.microsoft.com/en-us/windows/win32/sbscs/application-manifests#trustinfo)。

可构建带 `uiAccess="true"` 的版本：

```powershell
cargo build --release --features ui-access
```

此命令生成同名的 `target/release/click-show.exe`，会替换该目录中的普通构建产物。UIAccess 版本仍声明 `asInvoker`；启用该特性不等于自动获取管理员权限。正常部署需要系统信任的 Authenticode 数字签名，并安装到受保护的安全目录，例如 `C:\Program Files\ClickShow`。仅复制未签名文件到 Program Files 不满足 UIAccess 要求，具体还受系统安全策略约束。详见 Microsoft 的[辅助技术安全与 UIAccess 要求](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-securityoverview#uiaccess-requirements-for-assistive-technology-applications)。

普通版和 UIAccess 版均不承诺在 UAC 安全桌面、登录桌面或锁屏界面显示效果。

## 手工验收

当前实现已通过 19 项测试、格式检查和全特性 Clippy 严格检查。测试覆盖原版配置结构、保存失败后原配置完整性、五键状态、严格触发阈值、更新响应解析、真实 DIB 像素、透明度、动画、分层窗口及资源释放。本机两块显示器的 DPI 查询分别验证为 96 和 168。

已实际运行普通 Release 版，检查主窗口、浮标开关、设置和颜色对话框、Alpha 修改保存、重启读取及退出；另外使用 .NET 原版的 `DataContractJsonSerializer` 成功读取 Rust 写出的五键配置。普通版和 UIAccess 版均已构建。以下清单还包含不同权限、重新登录和签名部署等需要对应环境的验收场景。

在实际设备上逐项执行以下验收并记录结果；单元测试和编译检查不能代替图形界面、多显示器及权限场景的运行验证。

1. 使用 `--show --no-update --config .\target\manual-test.setting` 启动。核对默认值，改变设置后退出并重启，确认设置保留；核对“默认值”对外观和全局开关的影响。
2. 在普通应用中分别操作左、中、右、后退侧键、前进侧键，确认五种颜色；分别禁用各键并确认不再显示该键波纹。快速连击和交替按键，确认波纹能重叠显示，程序不阻塞鼠标输入。
3. 原地短按后抬起，确认只有按下波纹；原地长按超过 500ms 后抬起，确认额外小波纹。短时间内拖动超过 200 个物理像素再抬起，确认额外小波纹。阈值的精确等于情形通过输入逻辑单元测试检查。
4. 开启浮标，调整波纹与浮标的最小、最大尺寸和颜色 Alpha，检查完全透明、半透明及不透明效果；确认浮标跟随鼠标、各覆盖窗口不会拦截点击或抢走键盘焦点。
5. 使用两块不同缩放比例的显示器，例如 100% 与 150%，将副屏排列在主屏左侧或上方形成负坐标。跨屏移动、点击和拖动，核对浮标与波纹始终以鼠标位置为中心，尺寸随所在屏幕 DPI 调整。
6. 点击 X、最小化及托盘图标，核对隐藏和恢复行为；右键打开托盘菜单，选择“退出(Exit)”，确认进程、托盘图标和所有覆盖效果均退出。
7. 在允许变更开机启动状态的测试账户中勾选和取消“开机自动启动”，检查注册表路径与带空格的可执行文件路径，重新登录验证启动；最后恢复原状态。
8. 备份并复制原项目设置到隔离配置路径，确认尺寸、ARGB 颜色、五键设置和更新提醒记录可读取；用损坏副本检查恢复默认及错误提示。
9. 分别以普通权限、管理员权限运行普通版并测试管理员窗口。UIAccess 版在完成可信签名和安全目录部署后单独验收；记录系统策略对结果的影响。

## 原项目

原始源码保留在 `参考ClickShow/`，上游为 [cuiliang/ClickShow](https://github.com/cuiliang/ClickShow)。项目采用 MIT 许可证，见 [LICENSE](LICENSE)。
