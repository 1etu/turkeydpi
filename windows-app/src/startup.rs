use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

pub const AUTOSTART_FLAG: &str = "--autostart";

const RUN_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const ENTRY: &str = "TurkeyDPI";

fn run_key(access: u32) -> Option<RegKey> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_PATH, access)
        .ok()
}

fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" {}", exe.display(), AUTOSTART_FLAG))
}

fn entry() -> Option<String> {
    run_key(KEY_READ)?.get_value::<String, _>(ENTRY).ok()
}

pub fn autostarted() -> bool {
    std::env::args().any(|arg| arg == AUTOSTART_FLAG)
}

pub fn is_enabled() -> bool {
    entry().is_some()
}

fn enable() -> bool {
    match (command(), run_key(KEY_READ | KEY_WRITE)) {
        (Some(command), Some(key)) => key.set_value(ENTRY, &command).is_ok(),
        _ => false,
    }
}

fn disable() {
    if let Some(key) = run_key(KEY_READ | KEY_WRITE) {
        let _ = key.delete_value(ENTRY);
    }
}

pub fn set(wanted: bool) -> bool {
    if wanted {
        enable()
    } else {
        disable();
        false
    }
}

pub fn reconcile(wanted: bool) -> bool {
    match (wanted, entry()) {
        (true, Some(current)) if Some(&current) == command().as_ref() => true,
        (true, _) => enable(),
        (false, Some(_)) => {
            disable();
            false
        }
        (false, None) => false,
    }
}
