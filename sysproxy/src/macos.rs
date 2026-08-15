use std::process::Command;

use crate::{Result, SysProxyError};

fn networksetup(args: &[&str]) -> Result<String> {
    let output = Command::new("/usr/sbin/networksetup")
        .args(args)
        .output()
        .map_err(|e| SysProxyError::Failed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if stderr.contains("not have permission") || stderr.contains("administrator") {
            return Err(SysProxyError::PermissionDenied);
        }
        return Err(SysProxyError::Failed(stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn active_service() -> Result<String> {
    let listing = networksetup(&["-listnetworkserviceorder"])?;

    for line in listing.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('(') {
            continue;
        }
        if !(trimmed.contains("Wi-Fi") || trimmed.contains("Ethernet")) {
            continue;
        }
        if let Some(pos) = trimmed.find(')') {
            let name = trimmed[pos + 1..].trim();
            if !name.is_empty() {
                return Ok(name.to_string());
            }
        }
    }

    Ok("Wi-Fi".to_string())
}

pub fn enable(host: &str, port: u16) -> Result<()> {
    let service = active_service()?;
    let port = port.to_string();

    networksetup(&["-setwebproxy", &service, host, &port])?;
    networksetup(&["-setwebproxystate", &service, "on"])?;
    networksetup(&["-setsecurewebproxy", &service, host, &port])?;
    networksetup(&["-setsecurewebproxystate", &service, "on"])?;

    Ok(())
}

pub fn disable() -> Result<()> {
    let service = active_service()?;

    networksetup(&["-setwebproxystate", &service, "off"])?;
    networksetup(&["-setsecurewebproxystate", &service, "off"])?;
    networksetup(&["-setsocksfirewallproxystate", &service, "off"])?;

    Ok(())
}

pub fn is_enabled() -> Result<bool> {
    let service = active_service()?;
    let info = networksetup(&["-getwebproxy", &service])?;

    Ok(info.lines().any(|line| line.trim() == "Enabled: Yes"))
}

pub fn active_server() -> Result<Option<String>> {
    let service = active_service()?;
    let info = networksetup(&["-getwebproxy", &service])?;

    let mut enabled = false;
    let mut host = String::new();
    let mut port = String::new();

    for line in info.lines() {
        let line = line.trim();

        if let Some(value) = line.strip_prefix("Enabled:") {
            enabled = value.trim() == "Yes";
        } else if let Some(value) = line.strip_prefix("Server:") {
            host = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("Port:") {
            port = value.trim().to_string();
        }
    }

    if !enabled || host.is_empty() {
        return Ok(None);
    }

    Ok(Some(format!("{}:{}", host, port)))
}
