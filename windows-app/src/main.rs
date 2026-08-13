#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod icon;
#[cfg(windows)]
mod proxy_thread;
#[cfg(windows)]
mod paint;
#[cfg(windows)]
mod theme;
#[cfg(windows)]
mod tray;

#[cfg(windows)]
fn main() {
    tray::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("turkeydpi-tray only runs on Windows");
    std::process::exit(1);
}
