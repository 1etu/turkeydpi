use std::net::{SocketAddr, TcpStream};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;

use engine::BypassConfig;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyIcon, DispatchMessageW, GetMessageW, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, TranslateMessage, HICON, MSG, WM_APP,
    WM_CLOSE, WM_DESTROY, WM_ENDSESSION, WM_LBUTTONUP, WM_QUERYENDSESSION, WM_RBUTTONUP, WNDCLASSW,
};

use crate::guard;
use crate::icon::make_icon;
use crate::menu::{
    self, MenuModel, ACTION_PRESET_BASE, ACTION_QUIT, ACTION_SETUP, ACTION_TOGGLE, MENU_COMMAND,
};
use crate::proxy_thread::ProxyThread;
use crate::settings::Settings;
use crate::toast;
use crate::wizard::{self, CHOICES, WIZARD_DONE};

const TRAY_MESSAGE: u32 = WM_APP + 1;
pub const PROXY_FAILED: u32 = WM_APP + 3;

const PROBE_TIMEOUT: Duration = Duration::from_millis(300);

static ENGAGED: AtomicBool = AtomicBool::new(false);

struct State {
    hwnd: HWND,
    icon: HICON,
    enabled: bool,
    preset: usize,
    listen: SocketAddr,
    provider: Option<String>,
    proxy: ProxyThread,
    taskbar_created: u32,
    settings: Settings,
}

unsafe impl Send for State {}

#[derive(Clone, Copy)]
struct Snapshot {
    hwnd: HWND,
    icon: HICON,
    enabled: bool,
    preset: usize,
}

unsafe impl Send for Snapshot {}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn snapshot() -> Option<Snapshot> {
    let state = STATE.get()?.lock().ok()?;
    Some(Snapshot {
        hwnd: state.hwnd,
        icon: state.icon,
        enabled: state.enabled,
        preset: state.preset,
    })
}

fn engage(listen: SocketAddr) {
    guard::arm();

    match sysproxy::enable(&listen.ip().to_string(), listen.port()) {
        Ok(()) => ENGAGED.store(true, Ordering::SeqCst),
        Err(_) => guard::disarm(),
    }
}

fn release() {
    let _ = sysproxy::disable();
    guard::disarm();
    ENGAGED.store(false, Ordering::SeqCst);
}

fn is_listening(addr: SocketAddr) -> bool {
    TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).is_ok()
}

fn recover_stale_proxy(listen: SocketAddr) -> bool {
    let armed = guard::is_armed();

    let server = match sysproxy::active_server() {
        Ok(Some(server)) => server,
        _ => {
            if armed {
                guard::disarm();
            }
            return false;
        }
    };

    if !sysproxy::points_at_loopback(&server) {
        if armed {
            guard::disarm();
        }
        return false;
    }

    if !armed && !server.contains(&listen.to_string()) {
        return false;
    }

    let addr = server
        .rsplit('=')
        .next()
        .unwrap_or(&server)
        .parse::<SocketAddr>()
        .unwrap_or(listen);

    if is_listening(addr) {
        return false;
    }

    release();
    true
}

pub fn run() {
    unsafe {
        let instance = GetModuleHandleW(null());
        let class_name = wide("TurkeyDPITray");

        let mut class: WNDCLASSW = std::mem::zeroed();
        class.lpfnWndProc = Some(window_proc);
        class.hInstance = instance;
        class.lpszClassName = class_name.as_ptr();
        RegisterClassW(&class);

        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("TurkeyDPI").as_ptr(),
            0,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null_mut(),
        );

        if hwnd.is_null() {
            return;
        }

        let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        let icon = make_icon(false);

        let settings = Settings::load();
        let first_run = !settings.setup_done;

        let preset = CHOICES
            .iter()
            .position(|choice| choice.key == settings.preset)
            .unwrap_or(3);

        let listen: SocketAddr = format!("127.0.0.1:{}", settings.listen_port)
            .parse()
            .unwrap_or_else(|_| "127.0.0.1:8844".parse().unwrap());

        let recovered = recover_stale_proxy(listen);

        let state = State {
            hwnd,
            icon,
            enabled: false,
            preset,
            listen,
            provider: settings.detected_provider.clone(),
            proxy: ProxyThread::spawn(),
            taskbar_created,
            settings,
        };

        let snap = Snapshot {
            hwnd,
            icon,
            enabled: false,
            preset,
        };

        let _ = STATE.set(Mutex::new(state));
        add_tray_icon(&snap);

        if recovered {
            toast::show(
                "Your connection is back",
                "TurkeyDPI closed last time without handing the connection back to Windows. That is fixed now, click the icon to turn protection on again.",
            );
        } else if first_run {
            toast::show(
                "TurkeyDPI is running",
                "It lives in the tray, next to the clock. Click the icon to turn it on or change your provider.",
            );
        }

        if first_run {
            wizard::show(hwnd, CHOICES[preset].key);
        }

        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        teardown();
        shutdown();
    }
}

unsafe fn teardown() {
    if ENGAGED.load(Ordering::SeqCst) {
        release();
    } else {
        guard::disarm();
    }

    if let Some(snap) = snapshot() {
        remove_tray_icon(&snap);
    }

    toast::dismiss();
}

unsafe fn restore_after_cancelled_shutdown() {
    let (snap, listen, enabled) = match STATE.get().and_then(|mutex| mutex.lock().ok()) {
        Some(state) => (
            Snapshot {
                hwnd: state.hwnd,
                icon: state.icon,
                enabled: state.enabled,
                preset: state.preset,
            },
            state.listen,
            state.enabled,
        ),
        None => return,
    };

    add_tray_icon(&snap);

    if enabled {
        engage(listen);
    }
}

fn shutdown() {
    if let Some(mutex) = STATE.get() {
        if let Ok(mut state) = mutex.lock() {
            state.proxy.shutdown();
        }
    }
}

unsafe fn notify_data(snap: &Snapshot) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = std::mem::zeroed();
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = snap.hwnd;
    data.uID = 1;
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = TRAY_MESSAGE;
    data.hIcon = snap.icon;

    let tip = if snap.enabled {
        format!("TurkeyDPI on ({})", CHOICES[snap.preset].name)
    } else {
        "TurkeyDPI off".to_string()
    };

    let encoded = wide(&tip);
    let limit = encoded.len().min(data.szTip.len());
    data.szTip[..limit].copy_from_slice(&encoded[..limit]);

    data
}

unsafe fn add_tray_icon(snap: &Snapshot) {
    let data = notify_data(snap);
    Shell_NotifyIconW(NIM_ADD, &data);
}

unsafe fn update_tray_icon(snap: &Snapshot) {
    let data = notify_data(snap);
    Shell_NotifyIconW(NIM_MODIFY, &data);
}

unsafe fn remove_tray_icon(snap: &Snapshot) {
    let mut data: NOTIFYICONDATAW = std::mem::zeroed();
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = snap.hwnd;
    data.uID = 1;
    Shell_NotifyIconW(NIM_DELETE, &data);
}

fn open_menu(hwnd: HWND) {
    let model = {
        let state = match STATE.get().and_then(|mutex| mutex.lock().ok()) {
            Some(state) => state,
            None => return,
        };

        MenuModel {
            enabled: state.enabled,
            preset: state.preset,
            provider: state.provider.clone(),
            presets: CHOICES
                .iter()
                .map(|choice| (choice.name.to_string(), choice.detail.to_string()))
                .collect(),
        }
    };

    menu::show(hwnd, model);
}

fn apply_toggle(enabled: bool) -> Option<(Snapshot, HICON, SocketAddr)> {
    let mutex = STATE.get()?;
    let mut state = mutex.lock().ok()?;

    state.enabled = enabled;

    if enabled {
        let key = CHOICES[state.preset].key;
        let bypass = BypassConfig::preset(key).unwrap_or_default();
        state.proxy.start(bypass, state.listen, state.hwnd as isize);
    } else {
        state.proxy.stop();
    }

    let old_icon = state.icon;
    state.icon = make_icon(enabled);

    Some((
        Snapshot {
            hwnd: state.hwnd,
            icon: state.icon,
            enabled: state.enabled,
            preset: state.preset,
        },
        old_icon,
        state.listen,
    ))
}

fn apply_preset(index: usize) -> Option<(Snapshot, HICON)> {
    let mutex = STATE.get()?;
    let mut state = mutex.lock().ok()?;

    state.preset = index;
    state.settings.preset = CHOICES[index].key.to_string();
    state.settings.save();

    if state.enabled {
        let bypass = BypassConfig::preset(CHOICES[index].key).unwrap_or_default();
        state.proxy.start(bypass, state.listen, state.hwnd as isize);
    }

    let old_icon = state.icon;
    state.icon = make_icon(state.enabled);

    Some((
        Snapshot {
            hwnd: state.hwnd,
            icon: state.icon,
            enabled: state.enabled,
            preset: state.preset,
        },
        old_icon,
    ))
}

fn finish_setup(index: usize) -> Option<(Snapshot, HICON, SocketAddr, bool)> {
    let mutex = STATE.get()?;
    let mut state = mutex.lock().ok()?;

    let detected = Settings::load().detected_provider;

    state.preset = index;
    state.provider = detected.clone();
    state.settings.detected_provider = detected;
    state.settings.preset = CHOICES[index].key.to_string();
    state.settings.setup_done = true;
    state.settings.save();

    let was_enabled = state.enabled;
    state.enabled = true;

    let bypass = BypassConfig::preset(CHOICES[index].key).unwrap_or_default();
    state.proxy.start(bypass, state.listen, state.hwnd as isize);

    let old_icon = state.icon;
    state.icon = make_icon(true);

    Some((
        Snapshot {
            hwnd: state.hwnd,
            icon: state.icon,
            enabled: true,
            preset: index,
        },
        old_icon,
        state.listen,
        was_enabled,
    ))
}

unsafe fn refresh(snap: &Snapshot, old_icon: HICON) {
    update_tray_icon(snap);
    if !old_icon.is_null() && old_icon != snap.icon {
        DestroyIcon(old_icon);
    }
}

unsafe fn handle_action(action: usize) {
    match action {
        ACTION_TOGGLE => {
            let enabled = match snapshot() {
                Some(snap) => !snap.enabled,
                None => return,
            };

            let (snap, old_icon, listen) = match apply_toggle(enabled) {
                Some(result) => result,
                None => return,
            };

            if enabled {
                engage(listen);
            } else {
                release();
            }

            refresh(&snap, old_icon);
        }

        ACTION_SETUP => {
            let (hwnd, preset) = match snapshot() {
                Some(snap) => (snap.hwnd, snap.preset),
                None => return,
            };
            wizard::show(hwnd, CHOICES[preset].key);
        }

        ACTION_QUIT => {
            if let Some(snap) = snapshot() {
                if snap.enabled {
                    release();
                    if let Some((_, old_icon, _)) = apply_toggle(false) {
                        if !old_icon.is_null() {
                            DestroyIcon(old_icon);
                        }
                    }
                }
            }

            teardown();
            PostQuitMessage(0);
        }

        _ if action >= ACTION_PRESET_BASE && action < ACTION_PRESET_BASE + CHOICES.len() => {
            if let Some((snap, old_icon)) = apply_preset(action - ACTION_PRESET_BASE) {
                refresh(&snap, old_icon);
            }
        }

        _ => {}
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let taskbar_created = STATE
        .get()
        .and_then(|mutex| mutex.lock().ok().map(|state| state.taskbar_created))
        .unwrap_or(0);

    if taskbar_created != 0 && message == taskbar_created {
        if let Some(snap) = snapshot() {
            add_tray_icon(&snap);
        }
        return 0;
    }

    match message {
        TRAY_MESSAGE => {
            let event = lparam as u32;
            if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                open_menu(hwnd);
            }
            0
        }

        MENU_COMMAND => {
            PostMessageW(hwnd, WM_APP + 2, wparam, 0);
            0
        }

        WIZARD_DONE => {
            let index = wparam.min(CHOICES.len() - 1);

            if let Some((snap, old_icon, listen, was_enabled)) = finish_setup(index) {
                engage(listen);
                refresh(&snap, old_icon);

                if !was_enabled {
                    toast::show(
                        "Protection is on",
                        &format!(
                            "Using the {} preset. Your browser traffic now goes through TurkeyDPI.",
                            CHOICES[index].name
                        ),
                    );
                }
            }
            0
        }

        _ if message == WM_APP + 2 => {
            handle_action(wparam);
            0
        }

        PROXY_FAILED => {
            if let Some((snap, old_icon, listen)) = apply_toggle(false) {
                release();
                refresh(&snap, old_icon);
                toast::show(
                    "TurkeyDPI turned itself off",
                    &format!(
                        "It could not listen on port {}, so your connection went back to Windows untouched. Another program may already be using that port.",
                        listen.port()
                    ),
                );
            }
            0
        }

        WM_QUERYENDSESSION => {
            teardown();
            1
        }

        WM_ENDSESSION => {
            if wparam == 0 {
                restore_after_cancelled_shutdown();
            } else {
                teardown();
            }
            0
        }

        WM_CLOSE => {
            teardown();
            PostQuitMessage(0);
            0
        }

        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }

        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
