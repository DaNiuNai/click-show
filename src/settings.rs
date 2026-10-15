use std::{
    ffi::OsString,
    fs,
    io::{ErrorKind, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub a: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub fn parse(value: &str) -> Result<Self, String> {
        let value = value.trim();
        let hex = if let Some(hex) = value.strip_prefix('#') {
            hex.to_owned()
        } else {
            let hex = match value.to_ascii_lowercase().as_str() {
                "transparent" => "00FFFFFF",
                "black" => "FF000000",
                "white" => "FFFFFFFF",
                "red" => "FFFF0000",
                "green" => "FF008000",
                "blue" => "FF0000FF",
                "yellow" => "FFFFFF00",
                "cyan" | "aqua" => "FF00FFFF",
                "magenta" | "fuchsia" => "FFFF00FF",
                "gray" | "grey" => "FF808080",
                "silver" => "FFC0C0C0",
                "orange" => "FFFFA500",
                "orangered" => "FFFF4500",
                "dodgerblue" => "FF1E90FF",
                "blueviolet" => "FF8A2BE2",
                "purple" => "FF800080",
                "pink" => "FFFFC0CB",
                "brown" => "FFA52A2A",
                "lime" => "FF00FF00",
                "navy" => "FF000080",
                "teal" => "FF008080",
                "olive" => "FF808000",
                "maroon" => "FF800000",
                "gold" => "FFFFD700",
                "lightgray" | "lightgrey" => "FFD3D3D3",
                "darkgray" | "darkgrey" => "FFA9A9A9",
                _ => return Err(format!("无法识别颜色：{value}")),
            };
            hex.to_owned()
        };
        if !hex.is_ascii() || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("颜色必须使用十六进制格式：{value}"));
        }
        let hex = match hex.len() {
            3 => format!("FF{}", hex.chars().flat_map(|c| [c, c]).collect::<String>()),
            4 => hex.chars().flat_map(|c| [c, c]).collect(),
            6 => format!("FF{hex}"),
            8 => hex,
            _ => {
                return Err(format!(
                    "颜色必须为 #RGB、#ARGB、#RRGGBB 或 #AARRGGBB：{value}"
                ));
            }
        };
        let argb = u32::from_str_radix(&hex, 16).map_err(|_| format!("无效颜色：{value}"))?;
        Ok(Self {
            a: (argb >> 24) as u8,
            r: (argb >> 16) as u8,
            g: (argb >> 8) as u8,
            b: argb as u8,
        })
    }

    pub fn to_argb(self) -> String {
        format!("#{:02X}{:02X}{:02X}{:02X}", self.a, self.r, self.g, self.b)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    XButton1,
    XButton2,
}

impl MouseButton {
    pub const ALL: [Self; 5] = [
        Self::Left,
        Self::Middle,
        Self::Right,
        Self::XButton1,
        Self::XButton2,
    ];

    pub const fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Middle => 1,
            Self::Right => 2,
            Self::XButton1 => 3,
            Self::XButton2 => 4,
        }
    }

    const fn legacy_key(self) -> u32 {
        match self {
            Self::Left => 1_048_576,
            Self::Middle => 4_194_304,
            Self::Right => 2_097_152,
            Self::XButton1 => 8_388_608,
            Self::XButton2 => 16_777_216,
        }
    }

    fn from_legacy_key(key: u32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|button| button.legacy_key() == key)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ButtonSetting {
    pub is_enabled: bool,
    pub color: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppSettings {
    pub enable_click_circle: bool,
    pub enable_hover_dot: bool,
    pub indicator_size: f64,
    pub hover_dot_size: f64,
    pub hover_dot_fill: String,
    pub mouse_button_settings: [ButtonSetting; 5],
    pub last_notified_version: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            enable_click_circle: true,
            enable_hover_dot: false,
            indicator_size: 150.0,
            hover_dot_size: 60.0,
            hover_dot_fill: "#60FFFF5A".into(),
            mouse_button_settings: [
                "#FF1E90FF",
                "#FF008000",
                "#FFFF4500",
                "#FF808080",
                "#FF8A2BE2",
            ]
            .map(|color| ButtonSetting {
                is_enabled: true,
                color: color.into(),
            }),
            last_notified_version: None,
        }
    }
}

impl AppSettings {
    pub fn restore_appearance_defaults(&mut self) {
        let defaults = Self::default();
        self.indicator_size = defaults.indicator_size;
        self.hover_dot_size = defaults.hover_dot_size;
        self.hover_dot_fill = defaults.hover_dot_fill;
        self.mouse_button_settings = defaults.mouse_button_settings;
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.indicator_size.is_finite() || !(50.0..=400.0).contains(&self.indicator_size) {
            return Err("波纹大小必须在 50 到 400 之间".into());
        }
        if !self.hover_dot_size.is_finite() || !(10.0..=150.0).contains(&self.hover_dot_size) {
            return Err("浮标大小必须在 10 到 150 之间".into());
        }
        Color::parse(&self.hover_dot_fill).map_err(|error| format!("浮标颜色错误：{error}"))?;
        for button in MouseButton::ALL {
            Color::parse(&self.mouse_button_settings[button.index()].color)
                .map_err(|error| format!("{button:?} 按键颜色错误：{error}"))?;
        }
        Ok(())
    }
}

/// DataContractJsonSerializer stores MouseButtons as their .NET numeric values.
#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
struct SettingsDocument {
    #[serde(default)]
    enable_click_circle: Option<bool>,
    #[serde(default)]
    enable_hover_dot: Option<bool>,
    #[serde(default)]
    indicator_size: Option<f64>,
    #[serde(default)]
    hover_dot_size: Option<f64>,
    #[serde(default)]
    hover_dot_fill: Option<String>,
    #[serde(default)]
    mouse_button_settings: Option<Vec<ButtonEntry>>,
    #[serde(default)]
    last_notified_version: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
struct ButtonEntry {
    key: u32,
    value: ButtonDocument,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
struct ButtonDocument {
    #[serde(default)]
    is_enabled: Option<bool>,
    #[serde(default)]
    color: Option<String>,
}

impl SettingsDocument {
    fn into_settings(self) -> Result<AppSettings, String> {
        let mut settings = AppSettings::default();
        if let Some(value) = self.enable_click_circle {
            settings.enable_click_circle = value;
        }
        if let Some(value) = self.enable_hover_dot {
            settings.enable_hover_dot = value;
        }
        if let Some(value) = self.indicator_size {
            settings.indicator_size = value;
        }
        if let Some(value) = self.hover_dot_size {
            settings.hover_dot_size = value;
        }
        if let Some(value) = self.hover_dot_fill {
            settings.hover_dot_fill = value;
        }
        settings.last_notified_version = self.last_notified_version;
        for entry in self.mouse_button_settings.into_iter().flatten() {
            if let Some(button) = MouseButton::from_legacy_key(entry.key) {
                let target = &mut settings.mouse_button_settings[button.index()];
                if let Some(value) = entry.value.is_enabled {
                    target.is_enabled = value;
                }
                if let Some(value) = entry.value.color {
                    target.color = value;
                }
            }
        }
        settings.validate()?;
        Ok(settings)
    }

    fn from_settings(settings: &AppSettings) -> Self {
        Self {
            enable_click_circle: Some(settings.enable_click_circle),
            enable_hover_dot: Some(settings.enable_hover_dot),
            indicator_size: Some(settings.indicator_size),
            hover_dot_size: Some(settings.hover_dot_size),
            hover_dot_fill: Some(settings.hover_dot_fill.clone()),
            mouse_button_settings: Some(
                MouseButton::ALL
                    .into_iter()
                    .map(|button| {
                        let setting = &settings.mouse_button_settings[button.index()];
                        ButtonEntry {
                            key: button.legacy_key(),
                            value: ButtonDocument {
                                is_enabled: Some(setting.is_enabled),
                                color: Some(setting.color.clone()),
                            },
                        }
                    })
                    .collect(),
            ),
            last_notified_version: settings.last_notified_version.clone(),
        }
    }
}

pub fn load(path: impl AsRef<Path>) -> Result<AppSettings, String> {
    let text = match fs::read_to_string(path.as_ref()) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(AppSettings::default()),
        Err(error) => return Err(format!("读取设置失败：{error}")),
    };
    let document: SettingsDocument = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("设置 JSON 格式错误：{error}"))?;
    document.into_settings()
}

pub fn save(path: impl AsRef<Path>, settings: &AppSettings) -> Result<(), String> {
    settings.validate()?;
    let text = serde_json::to_string_pretty(&SettingsDocument::from_settings(settings))
        .map_err(|error| format!("序列化设置失败：{error}"))?;
    let path = path.as_ref();
    let filename = path
        .file_name()
        .ok_or("保存设置失败：目标路径必须包含文件名")?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("保存设置失败：{error}"))?
        .as_nanos();
    static SAVE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let sequence = SAVE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut temporary_name = OsString::from(".");
    temporary_name.push(filename);
    temporary_name.push(format!(
        ".{}.{timestamp}.{sequence}.tmp",
        std::process::id()
    ));
    let temporary_path = path.with_file_name(temporary_name);
    // Only remove temporary files created by this call; create_new never clobbers an existing file.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .map_err(|error| format!("保存设置失败：{error}"))?;
    let written = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all());
    // Close the handle before the Windows rename operation replaces the previous configuration.
    drop(file);
    if let Err(error) = written.and_then(|()| fs::rename(&temporary_path, path)) {
        return match fs::remove_file(&temporary_path) {
            Ok(()) => Err(format!("保存设置失败：{error}")),
            Err(cleanup_error) => Err(format!(
                "保存设置失败：{error}；清理临时文件 {} 失败：{cleanup_error}",
                temporary_path.display()
            )),
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_formats_preserve_alpha_and_expand_short_forms() {
        for (input, expected) in [
            ("#60FFFF5A", "#60FFFF5A"),
            ("#1e90ff", "#FF1E90FF"),
            ("#abc", "#FFAABBCC"),
            ("#6f05", "#66FF0055"),
            ("DodgerBlue", "#FF1E90FF"),
            ("Transparent", "#00FFFFFF"),
        ] {
            assert_eq!(Color::parse(input).unwrap().to_argb(), expected);
        }
        for invalid in ["", "#", "#12345", "#GG0000", "#１２３", "unknown"] {
            assert!(Color::parse(invalid).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn legacy_dictionary_numeric_keys_are_not_positional() {
        let json = r##"{
            "EnableClickCircle":false,"EnableHoverDot":true,
            "IndicatorSize":205,"HoverDotSize":75,"HoverDotFill":"#408899AA",
            "LastNotifiedVersion":"1.5.0","FutureSetting":true,
            "MouseButtonSettings":[
                {"Key":16777216,"Value":{"Color":"#FFAABBCC","IsEnabled":false}},
                {"Key":2097152,"Value":{"Color":"#FF010203"}},
                {"Key":1048576,"Value":{"IsEnabled":false}}
            ]
        }"##;
        let settings = serde_json::from_str::<SettingsDocument>(json)
            .unwrap()
            .into_settings()
            .unwrap();
        assert!(!settings.enable_click_circle);
        assert!(settings.enable_hover_dot);
        assert_eq!(settings.indicator_size, 205.0);
        assert_eq!(settings.hover_dot_size, 75.0);
        assert_eq!(settings.hover_dot_fill, "#408899AA");
        assert_eq!(settings.last_notified_version.as_deref(), Some("1.5.0"));
        assert_eq!(
            settings.mouse_button_settings[MouseButton::Right.index()].color,
            "#FF010203"
        );
        assert_eq!(
            settings.mouse_button_settings[MouseButton::Left.index()].color,
            "#FF1E90FF"
        );
        assert!(!settings.mouse_button_settings[MouseButton::Left.index()].is_enabled);
        assert!(!settings.mouse_button_settings[MouseButton::XButton2.index()].is_enabled);
        assert_eq!(
            settings.mouse_button_settings[MouseButton::Middle.index()],
            AppSettings::default().mouse_button_settings[1]
        );
    }

    #[test]
    fn serialization_retains_original_datacontract_shape() {
        let settings = AppSettings::default();
        let json = serde_json::to_value(SettingsDocument::from_settings(&settings)).unwrap();
        assert_eq!(json["MouseButtonSettings"][0]["Key"], 1048576);
        assert_eq!(json["MouseButtonSettings"][1]["Key"], 4194304);
        assert_eq!(json["MouseButtonSettings"][2]["Key"], 2097152);
        assert_eq!(
            json["MouseButtonSettings"][0]["Value"]["Color"],
            "#FF1E90FF"
        );
        assert_eq!(json["MouseButtonSettings"][0]["Value"]["IsEnabled"], true);
        let restored = serde_json::from_value::<SettingsDocument>(json)
            .unwrap()
            .into_settings()
            .unwrap();
        assert_eq!(restored, settings);
        assert_eq!(
            serde_json::from_str::<SettingsDocument>("{}")
                .unwrap()
                .into_settings()
                .unwrap(),
            settings
        );
    }

    #[test]
    fn restore_appearance_preserves_global_switches_and_version() {
        let mut settings = AppSettings {
            enable_click_circle: false,
            enable_hover_dot: true,
            indicator_size: 250.0,
            hover_dot_size: 100.0,
            hover_dot_fill: "#FF000000".into(),
            last_notified_version: Some("2.0".into()),
            ..AppSettings::default()
        };
        settings.mouse_button_settings[0].is_enabled = false;
        settings.restore_appearance_defaults();
        assert!(!settings.enable_click_circle);
        assert!(settings.enable_hover_dot);
        assert_eq!(settings.last_notified_version.as_deref(), Some("2.0"));
        let defaults = AppSettings::default();
        assert_eq!(settings.indicator_size, defaults.indicator_size);
        assert_eq!(settings.hover_dot_size, defaults.hover_dot_size);
        assert_eq!(settings.hover_dot_fill, defaults.hover_dot_fill);
        assert_eq!(
            settings.mouse_button_settings,
            defaults.mouse_button_settings
        );
    }

    #[test]
    fn validation_rejects_unrenderable_settings() {
        for size in [f64::NAN, f64::INFINITY, -1.0, 49.0, 401.0] {
            assert!(
                AppSettings {
                    indicator_size: size,
                    ..AppSettings::default()
                }
                .validate()
                .is_err()
            );
        }
        for size in [f64::NEG_INFINITY, 0.0, 9.0, 151.0] {
            assert!(
                AppSettings {
                    hover_dot_size: size,
                    ..AppSettings::default()
                }
                .validate()
                .is_err()
            );
        }
        let mut settings = AppSettings::default();
        settings.mouse_button_settings[4].color = "invalid".into();
        assert!(settings.validate().is_err());
    }

    #[test]
    fn configuration_file_roundtrip_bom_and_error_handling() {
        // Keep the fixture inside the build directory instead of consulting environment variables.
        let directory = Path::new("target").join(format!("settings-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.json");
        let _ = fs::remove_file(&path);
        assert_eq!(load(&path).unwrap(), AppSettings::default());
        let settings = AppSettings {
            enable_hover_dot: true,
            last_notified_version: Some("1.5.1".into()),
            ..AppSettings::default()
        };
        save(&path, &settings).unwrap();
        assert_eq!(load(&path).unwrap(), settings);
        save(&path, &AppSettings::default()).unwrap();
        assert_eq!(load(&path).unwrap(), AppSettings::default());
        save(&path, &settings).unwrap();
        assert_eq!(load(&path).unwrap(), settings);
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, format!("\u{feff}{text}")).unwrap();
        assert_eq!(load(&path).unwrap(), settings);
        fs::write(&path, "{invalid json}").unwrap();
        assert!(load(&path).is_err());
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn failed_replacement_preserves_previous_configuration_and_removes_temporary_file() {
        use std::os::windows::fs::OpenOptionsExt;

        let directory =
            Path::new("target").join(format!("settings-atomic-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.json");
        let previous = AppSettings::default();
        save(&path, &previous).unwrap();
        let original_bytes = fs::read(&path).unwrap();
        // An existing reader without delete sharing prevents Windows from replacing the file.
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let updated = AppSettings {
            enable_hover_dot: true,
            ..previous.clone()
        };
        assert!(save(&path, &updated).is_err());
        drop(lock);
        assert_eq!(fs::read(&path).unwrap(), original_bytes);
        assert_eq!(load(&path).unwrap(), previous);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        save(&path, &updated).unwrap();
        assert_eq!(load(&path).unwrap(), updated);
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&directory).unwrap();
    }
}
