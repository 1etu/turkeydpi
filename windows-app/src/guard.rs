use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

pub const CLEAR_FLAG: &str = "--clear-system-proxy";

const RUN_ONCE_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
const ENTRY: &str = "TurkeyDPIClearProxy";
const PROBE_TIMEOUT: Duration = Duration::from_millis(300);

fn run_once(access: u32) -> Option<RegKey> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_ONCE_PATH, access)
        .ok()
}

fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" {}", exe.display(), CLEAR_FLAG))
}

pub fn arm() {
    let (command, key) = match (command(), run_once(KEY_READ | KEY_WRITE)) {
        (Some(command), Some(key)) => (command, key),
        _ => return,
    };

    let _ = key.set_value(ENTRY, &command);
}

pub fn disarm() {
    if let Some(key) = run_once(KEY_READ | KEY_WRITE) {
        let _ = key.delete_value(ENTRY);
    }
}

pub fn is_armed() -> bool {
    run_once(KEY_READ)
        .and_then(|key| key.get_value::<String, _>(ENTRY).ok())
        .is_some()
}

pub fn server_addr(server: &str) -> Option<SocketAddr> {
    server.rsplit('=').next()?.trim().parse().ok()
}

pub fn is_listening(addr: SocketAddr) -> bool {
    TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).is_ok()
}

pub fn clear() {
    disarm();

    if let Ok(Some(server)) = sysproxy::active_server() {
        if sysproxy::points_at_loopback(&server) {
            let _ = sysproxy::disable();
        }
    }
}
