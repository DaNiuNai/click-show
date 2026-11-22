use std::{
    os::windows::ffi::OsStrExt,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::*,
};
const RUN_KEY: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "ClickShow";
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn registry_error(action: &str, code: u32) -> String {
    format!(
        "{action}：{}",
        std::io::Error::from_raw_os_error(code as i32)
    )
}

struct RegistryKey(HKEY);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

fn open_run_key(access: REG_SAM_FLAGS, create: bool) -> Result<Option<RegistryKey>, String> {
    let name = wide(RUN_KEY);
    let mut key = null_mut();
    let status = unsafe {
        if create {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                name.as_ptr(),
                0,
                null(),
                REG_OPTION_NON_VOLATILE,
                access,
                null(),
                &mut key,
                null_mut(),
            )
        } else {
            RegOpenKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, access, &mut key)
        }
    };
    match status {
        ERROR_SUCCESS => Ok(Some(RegistryKey(key))),
        ERROR_FILE_NOT_FOUND if !create => Ok(None),
        code => Err(registry_error("无法打开开机启动配置", code)),
    }
}

pub fn autostart_enabled() -> Result<bool, String> {
    let Some(key) = open_run_key(KEY_QUERY_VALUE, false)? else {
        return Ok(false);
    };
    let name = wide(RUN_VALUE);
    let mut size = 0;
    // Existence is the original application's criterion, regardless of value
    // type or whether another copy wrote the executable path.
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            null(),
            null_mut(),
            null_mut(),
            &mut size,
        )
    };
    match status {
        ERROR_SUCCESS => Ok(true),
        ERROR_FILE_NOT_FOUND => Ok(false),
        code => Err(registry_error("无法加载开机自动启动状态", code)),
    }
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let Some(key) = open_run_key(KEY_SET_VALUE, enabled)? else {
        return Ok(());
    };
    let name = wide(RUN_VALUE);
    let status = if enabled {
        let executable = std::env::current_exe().map_err(|e| format!("无法获取程序路径：{e}"))?;
        let command: Vec<u16> = std::iter::once(u16::from(b'"'))
            .chain(executable.as_os_str().encode_wide())
            .chain([u16::from(b'"'), 0])
            .collect();
        let size = u32::try_from(std::mem::size_of_val(command.as_slice()))
            .map_err(|_| "程序路径过长".to_owned())?;
        unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr().cast(),
                size,
            )
        }
    } else {
        unsafe { RegDeleteValueW(key.0, name.as_ptr()) }
    };
    if status == ERROR_SUCCESS || (!enabled && status == ERROR_FILE_NOT_FOUND) {
        Ok(())
    } else {
        Err(registry_error("无法保存开机自动启动状态", status))
    }
}
