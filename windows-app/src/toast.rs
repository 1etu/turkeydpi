use std::cell::RefCell;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows_sys::Win32::Graphics::Gdi::{BeginPaint, DeleteObject, EndPaint, HFONT, PAINTSTRUCT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, KillTimer, RegisterClassW,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, SystemParametersInfoW,
    CS_DROPSHADOW, HWND_TOPMOST, LWA_ALPHA, SPI_GETWORKAREA, SWP_NOACTIVATE, SW_SHOWNOACTIVATE,
    WM_DESTROY, WM_LBUTTONUP, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::paint::{make_font, wide, Canvas, Rect, TextAlign, Weight};
use crate::theme::{metrics, Theme};

const TIMER_FADE_IN: usize = 1;
const TIMER_HOLD: usize = 2;
const TIMER_FADE_OUT: usize = 3;

const HOLD_MS: u32 = 5200;
const FADE_STEP: u32 = 16;

struct ToastState {
    hwnd: HWND,
    theme: Theme,
    title: String,
    body: String,
    alpha: i32,
    font_title: HFONT,
    font_body: HFONT,
}

thread_local! {
    static TOAST: RefCell<Option<ToastState>> = const { RefCell::new(None) };
}

pub fn show(title: &str, body: &str) {
    dismiss();

    unsafe {
        let instance = GetModuleHandleW(null());
        let class_name = wide("TurkeyDPIToast");

        let mut class: WNDCLASSW = std::mem::zeroed();
        class.style = CS_DROPSHADOW;
        class.lpfnWndProc = Some(toast_proc);
        class.hInstance = instance;
        class.lpszClassName = class_name.as_ptr();
        RegisterClassW(&class);

        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            wide("TurkeyDPI").as_ptr(),
            WS_POPUP,
            0,
            0,
            metrics::TOAST_WIDTH,
            metrics::TOAST_HEIGHT,
            null_mut(),
            null_mut(),
            instance,
            null_mut(),
        );

        if hwnd.is_null() {
            return;
        }

        let theme = Theme::current();

        let corner = DWMWCP_ROUND;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &corner as *const _ as *const _,
            std::mem::size_of::<i32>() as u32,
        );

        let border = theme.border.colorref();
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            &border as *const _ as *const _,
            std::mem::size_of::<u32>() as u32,
        );

        SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA);

        let mut work: RECT = std::mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut _ as *mut _, 0);

        let x = work.right - metrics::TOAST_WIDTH - metrics::TOAST_MARGIN;
        let y = work.bottom - metrics::TOAST_HEIGHT - metrics::TOAST_MARGIN;

        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            metrics::TOAST_WIDTH,
            metrics::TOAST_HEIGHT,
            SWP_NOACTIVATE,
        );

        TOAST.with(|toast| {
            *toast.borrow_mut() = Some(ToastState {
                hwnd,
                theme,
                title: title.to_string(),
                body: body.to_string(),
                alpha: 0,
                font_title: make_font(15, Weight::Semibold),
                font_body: make_font(13, Weight::Regular),
            })
        });

        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(hwnd, TIMER_FADE_IN, FADE_STEP, None);
    }
}

pub fn dismiss() {
    let hwnd = TOAST.with(|toast| toast.borrow().as_ref().map(|state| state.hwnd));

    if let Some(hwnd) = hwnd {
        unsafe { DestroyWindow(hwnd) };
    }
}

unsafe fn paint(state: &ToastState, hdc: windows_sys::Win32::Graphics::Gdi::HDC) {
    let canvas = match Canvas::new(metrics::TOAST_WIDTH, metrics::TOAST_HEIGHT) {
        Some(canvas) => canvas,
        None => return,
    };

    let theme = state.theme;
    canvas.clear(theme.surface_raised);

    let badge = 40;
    let badge_x = 18;
    let badge_y = (metrics::TOAST_HEIGHT - badge) / 2;

    let tint = if theme.dark { 0.72 } else { 0.86 };

    canvas.fill_rounded(
        Rect::new(badge_x, badge_y, badge, badge),
        11.0,
        theme.accent.mix(theme.surface_raised, tint),
    );
    canvas.circle(
        (badge_x + badge / 2) as f32,
        (badge_y + badge / 2) as f32,
        7.0,
        theme.success,
    );

    let text_x = badge_x + badge + 14;
    let text_width = metrics::TOAST_WIDTH - text_x - 18;

    canvas.text(
        state.font_title,
        &state.title,
        Rect::new(text_x, 20, text_width, 20),
        theme.text,
        TextAlign::Left,
    );

    canvas.text(
        state.font_body,
        &state.body,
        Rect::new(text_x, 41, text_width, 38),
        theme.text_dim,
        TextAlign::Wrap,
    );

    canvas.blit(hdc);
}

unsafe extern "system" fn toast_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            TOAST.with(|toast| {
                if let Some(state) = toast.borrow().as_ref() {
                    paint(state, hdc);
                }
            });
            EndPaint(hwnd, &ps);
            0
        }

        WM_TIMER => {
            match wparam {
                TIMER_FADE_IN => {
                    let done = TOAST.with(|toast| {
                        let mut borrow = toast.borrow_mut();
                        match borrow.as_mut() {
                            Some(state) => {
                                state.alpha = (state.alpha + 28).min(255);
                                SetLayeredWindowAttributes(hwnd, 0, state.alpha as u8, LWA_ALPHA);
                                state.alpha >= 255
                            }
                            None => true,
                        }
                    });

                    if done {
                        KillTimer(hwnd, TIMER_FADE_IN);
                        SetTimer(hwnd, TIMER_HOLD, HOLD_MS, None);
                    }
                }

                TIMER_HOLD => {
                    KillTimer(hwnd, TIMER_HOLD);
                    SetTimer(hwnd, TIMER_FADE_OUT, FADE_STEP, None);
                }

                TIMER_FADE_OUT => {
                    let done = TOAST.with(|toast| {
                        let mut borrow = toast.borrow_mut();
                        match borrow.as_mut() {
                            Some(state) => {
                                state.alpha = (state.alpha - 22).max(0);
                                SetLayeredWindowAttributes(hwnd, 0, state.alpha as u8, LWA_ALPHA);
                                state.alpha <= 0
                            }
                            None => true,
                        }
                    });

                    if done {
                        KillTimer(hwnd, TIMER_FADE_OUT);
                        DestroyWindow(hwnd);
                    }
                }

                _ => {}
            }
            0
        }

        WM_LBUTTONUP => {
            DestroyWindow(hwnd);
            0
        }

        WM_DESTROY => {
            TOAST.with(|toast| {
                if let Some(state) = toast.borrow_mut().take() {
                    DeleteObject(state.font_title as _);
                    DeleteObject(state.font_body as _);
                }
            });
            0
        }

        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
