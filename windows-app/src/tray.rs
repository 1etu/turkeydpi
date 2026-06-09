use std::net::SocketAddr;
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::sync::OnceLock;

use engine::BypassConfig;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateMenu, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, TrackPopupMenu, TranslateMessage,
    HICON, MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, TPM_BOTTOMALIGN,
    TPM_RIGHTALIGN, WM_APP, WM_COMMAND, WM_DESTROY, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
};

use crate::icon::make_icon;
use crate::proxy_thread::ProxyThread;

const TRAY_MESSAGE: u32 = WM_APP + 1;
const ID_TOGGLE: usize = 1;
const ID_QUIT: usize = 2;
const ID_PRESET_BASE: usize = 100;

const PRESETS: &[(&str, &str)] = &[
    ("turk-telekom", "Turk Telekom"),
    ("vodafone", "Vodafone TR"),
    ("superonline", "Superonline"),
    ("aggressive", "Aggressive"),
];

struct State {
    hwnd: HWND,
    icon: HICON,
    enabled: bool,
    preset: usize,
    listen: SocketAddr,
    proxy: ProxyThread,
    taskbar_created: u32,
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

pub fn run() {
    unsafe {
        let instance = GetModuleHandleW(null());
        let class_name = wide("TurkeyDPITray");

        let mut class: WNDCLASSW = std::mem::zeroed();
        class.lpfnWndProc = Some(window_proc);
        class.hInstance = instance;
        class.lpszClassName = class_name.as_ptr();
        RegisterClassW(&class);

        let window_name = wide("TurkeyDPI");
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            window_name.as_ptr(),
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

        let state = State {
            hwnd,
            icon,
            enabled: false,
            preset: 3,
            listen: "127.0.0.1:8844".parse().unwrap(),
            proxy: ProxyThread::spawn(),
            taskbar_created,
        };

        let snap = Snapshot {
            hwnd,
            icon,
            enabled: false,
            preset: 3,
        };

        let _ = STATE.set(Mutex::new(state));
        add_tray_icon(&snap);

        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        shutdown();
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
        format!("TurkeyDPI on ({})", PRESETS[snap.preset].1)
    } else {
        "TurkeyDPI off".to_string()
    };

    let encoded = wide(&tip);
    let limit = encoded.len().min(data.szTip.len());
    data.szTip[..limit].copy_from_slice(&encoded[..limit]);

    data
}

unsafe fn add_tray_icon(snap: &Snapshot) {
    let mut data = notify_data(snap);
    Shell_NotifyIconW(NIM_ADD, &mut data);
}

unsafe fn update_tray_icon(snap: &Snapshot) {
    let mut data = notify_data(snap);
    Shell_NotifyIconW(NIM_MODIFY, &mut data);
}

unsafe fn remove_tray_icon(snap: &Snapshot) {
    let mut data: NOTIFYICONDATAW = std::mem::zeroed();
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = snap.hwnd;
    data.uID = 1;
    Shell_NotifyIconW(NIM_DELETE, &mut data);
}

unsafe fn show_menu(snap: &Snapshot) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }

    let toggle_label = if snap.enabled {
        wide("Turn off")
    } else {
        wide("Turn on")
    };
    AppendMenuW(menu, MF_STRING, ID_TOGGLE, toggle_label.as_ptr());

    let presets = CreateMenu();
    for (index, (_, label)) in PRESETS.iter().enumerate() {
        let flags = if index == snap.preset {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING | MF_UNCHECKED
        };
        AppendMenuW(presets, flags, ID_PRESET_BASE + index, wide(label).as_ptr());
    }

    AppendMenuW(menu, MF_POPUP, presets as usize, wide("Preset").as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    AppendMenuW(menu, MF_STRING, ID_QUIT, wide("Quit").as_ptr());

    let mut point: POINT = std::mem::zeroed();
    GetCursorPos(&mut point);

    SetForegroundWindow(snap.hwnd);
    TrackPopupMenu(
        menu,
        TPM_RIGHTALIGN | TPM_BOTTOMALIGN,
        point.x,
        point.y,
        0,
        snap.hwnd,
        null(),
    );

    DestroyMenu(menu);
}

fn apply_toggle(enabled: bool) -> Option<(Snapshot, HICON, SocketAddr)> {
    let mutex = STATE.get()?;
    let mut state = mutex.lock().ok()?;

    state.enabled = enabled;

    if enabled {
        let bypass = BypassConfig::preset(PRESETS[state.preset].0).unwrap_or_default();
        state.proxy.start(bypass, state.listen);
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

    if state.enabled {
        let bypass = BypassConfig::preset(PRESETS[index].0).unwrap_or_default();
        state.proxy.start(bypass, state.listen);
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

unsafe fn handle_command(id: usize) {
    match id {
        ID_TOGGLE => {
            let enabled = match snapshot() {
                Some(snap) => !snap.enabled,
                None => return,
            };

            let (snap, old_icon, listen) = match apply_toggle(enabled) {
                Some(result) => result,
                None => return,
            };

            if enabled {
                let _ = sysproxy::enable(&listen.ip().to_string(), listen.port());
            } else {
                let _ = sysproxy::disable();
            }

            update_tray_icon(&snap);
            if !old_icon.is_null() {
                DestroyIcon(old_icon);
            }
        }

        ID_QUIT => {
            if let Some(snap) = snapshot() {
                if snap.enabled {
                    let _ = sysproxy::disable();
                    if let Some((snap, old_icon, _)) = apply_toggle(false) {
                        if !old_icon.is_null() {
                            DestroyIcon(old_icon);
                        }
                        remove_tray_icon(&snap);
                    }
                } else {
                    remove_tray_icon(&snap);
                }
            }

            PostQuitMessage(0);
        }

        _ if id >= ID_PRESET_BASE && id < ID_PRESET_BASE + PRESETS.len() => {
            if let Some((snap, old_icon)) = apply_preset(id - ID_PRESET_BASE) {
                update_tray_icon(&snap);
                if !old_icon.is_null() {
                    DestroyIcon(old_icon);
                }
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
                if let Some(snap) = snapshot() {
                    show_menu(&snap);
                }
            }
            0
        }

        WM_COMMAND => {
            PostMessageW(hwnd, WM_APP + 2, wparam & 0xFFFF, 0);
            0
        }

        _ if message == WM_APP + 2 => {
            handle_command(wparam);
            0
        }

        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }

        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
