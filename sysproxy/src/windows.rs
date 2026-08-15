use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

use crate::{Result, SysProxyError};

const SETTINGS_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
const BYPASS_LIST: &str = "localhost;127.*;10.*;172.16.*;192.168.*;<local>";

fn open_settings(access: u32) -> Result<RegKey> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(SETTINGS_PATH, access)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::PermissionDenied => SysProxyError::PermissionDenied,
            _ => SysProxyError::Failed(e.to_string()),
        })
}

fn notify_wininet() {
    use windows_sys::Win32::Networking::WinInet::{
        InternetSetOptionW, INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED,
    };

    unsafe {
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_SETTINGS_CHANGED,
            std::ptr::null_mut(),
            0,
        );
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_REFRESH,
            std::ptr::null_mut(),
            0,
        );
    }
}

pub fn enable(host: &str, port: u16) -> Result<()> {
    let settings = open_settings(KEY_READ | KEY_WRITE)?;
    let server = format!("{}:{}", host, port);

    settings
        .set_value("ProxyServer", &server)
        .map_err(|e| SysProxyError::Failed(e.to_string()))?;
    settings
        .set_value("ProxyOverride", &BYPASS_LIST.to_string())
        .map_err(|e| SysProxyError::Failed(e.to_string()))?;
    settings
        .set_value("ProxyEnable", &1u32)
        .map_err(|e| SysProxyError::Failed(e.to_string()))?;

    notify_wininet();
    Ok(())
}

pub fn disable() -> Result<()> {
    let settings = open_settings(KEY_READ | KEY_WRITE)?;

    settings
        .set_value("ProxyEnable", &0u32)
        .map_err(|e| SysProxyError::Failed(e.to_string()))?;

    notify_wininet();
    Ok(())
}

pub fn is_enabled() -> Result<bool> {
    let settings = open_settings(KEY_READ)?;

    let enabled: u32 = settings.get_value("ProxyEnable").unwrap_or(0);
    Ok(enabled == 1)
}

pub fn active_server() -> Result<Option<String>> {
    let settings = open_settings(KEY_READ)?;

    let enabled: u32 = settings.get_value("ProxyEnable").unwrap_or(0);
    if enabled != 1 {
        return Ok(None);
    }

    Ok(settings.get_value::<String, _>("ProxyServer").ok())
}
