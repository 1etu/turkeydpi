#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod guard;
#[cfg(windows)]
mod icon;
#[cfg(windows)]
mod isp;
#[cfg(windows)]
mod menu;
#[cfg(windows)]
mod paint;
#[cfg(windows)]
mod proxy_thread;
#[cfg(windows)]
mod settings;
#[cfg(windows)]
mod startup;
#[cfg(windows)]
mod theme;
#[cfg(windows)]
mod toast;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod wizard;

#[cfg(windows)]
fn main() {
    if std::env::args().any(|arg| arg == guard::CLEAR_FLAG) {
        guard::clear();
        return;
    }

    tray::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("turkeydpi-tray only runs on Windows");
    std::process::exit(1);
}
