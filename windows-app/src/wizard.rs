use std::cell::RefCell;
use std::ptr::{null, null_mut};
use std::sync::mpsc::{channel, Receiver};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, DeleteObject, EndPaint, InvalidateRect, HFONT, PAINTSTRUCT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, KillTimer, PostMessageW, RegisterClassW,
    SendMessageW, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, SystemParametersInfoW,
    CS_DROPSHADOW, HTCAPTION, HWND_TOPMOST, SPI_GETWORKAREA, SWP_SHOWWINDOW, SW_SHOW, WM_DESTROY,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCLBUTTONDOWN, WM_PAINT, WM_TIMER, WNDCLASSW,
    WS_EX_TOPMOST, WS_POPUP,
};

use crate::paint::{make_font, wide, Canvas, Rect, TextAlign, Weight};
use crate::theme::{metrics, Theme};

pub const WIZARD_DONE: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 30;

const TIMER_SPIN: usize = 1;
const TIMER_POLL: usize = 2;

pub struct PresetChoice {
    pub key: &'static str,
    pub name: &'static str,
    pub detail: &'static str,
}

pub const CHOICES: &[PresetChoice] = &[
    PresetChoice {
        key: "turk-telekom",
        name: "Türk Telekom",
        detail: "Also covers TTNet lines",
    },
    PresetChoice {
        key: "vodafone",
        name: "Vodafone",
        detail: "Slower, steadier segments",
    },
    PresetChoice {
        key: "superonline",
        name: "Superonline",
        detail: "Turkcell fibre and mobile",
    },
    PresetChoice {
        key: "aggressive",
        name: "Aggressive",
        detail: "Try this if the others fail",
    },
];

#[derive(PartialEq)]
enum Stage {
    Detecting,
    Detected,
    Manual,
}

struct WizardState {
    hwnd: HWND,
    owner: HWND,
    stage: Stage,
    spin: f32,
    provider: Option<String>,
    selected: usize,
    hover: Option<usize>,
    hover_primary: bool,
    hover_secondary: bool,
    theme: Theme,
    receiver: Option<Receiver<Option<crate::isp::Provider>>>,
    font_title: HFONT,
    font_body: HFONT,
    font_row: HFONT,
    font_small: HFONT,
    font_button: HFONT,
}

thread_local! {
    static WIZARD: RefCell<Option<WizardState>> = const { RefCell::new(None) };
}

fn list_top() -> i32 {
    118
}

fn row_height() -> i32 {
    46
}

fn primary_rect() -> Rect {
    let w = 116;
    let h = 32;
    Rect::new(
        metrics::WIZARD_WIDTH - 28 - w,
        metrics::WIZARD_HEIGHT - 28 - h,
        w,
        h,
    )
}

fn secondary_rect() -> Rect {
    let w = 104;
    let h = 32;
    Rect::new(28, metrics::WIZARD_HEIGHT - 28 - h, w, h)
}

pub fn is_open() -> bool {
    WIZARD.with(|wizard| wizard.borrow().is_some())
}

pub fn show(owner: HWND, current_preset: &str) {
    if is_open() {
        WIZARD.with(|wizard| {
            if let Some(state) = wizard.borrow().as_ref() {
                unsafe { SetForegroundWindow(state.hwnd) };
            }
        });
        return;
    }

    unsafe {
        let instance = GetModuleHandleW(null());
        let class_name = wide("TurkeyDPIWizard");

        let mut class: WNDCLASSW = std::mem::zeroed();
        class.style = CS_DROPSHADOW;
        class.lpfnWndProc = Some(wizard_proc);
        class.hInstance = instance;
        class.lpszClassName = class_name.as_ptr();
        RegisterClassW(&class);

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            class_name.as_ptr(),
            wide("TurkeyDPI Setup").as_ptr(),
            WS_POPUP,
            0,
            0,
            metrics::WIZARD_WIDTH,
            metrics::WIZARD_HEIGHT,
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

        let mut work: RECT = std::mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut _ as *mut _, 0);

        let x = work.left + (work.right - work.left - metrics::WIZARD_WIDTH) / 2;
        let y = work.top + (work.bottom - work.top - metrics::WIZARD_HEIGHT) / 2;

        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            metrics::WIZARD_WIDTH,
            metrics::WIZARD_HEIGHT,
            SWP_SHOWWINDOW,
        );

        let (sender, receiver) = channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();

            let result = match runtime {
                Ok(runtime) => runtime.block_on(crate::isp::detect()),
                Err(_) => None,
            };

            let _ = sender.send(result);
        });

        let selected = CHOICES
            .iter()
            .position(|choice| choice.key == current_preset)
            .unwrap_or(3);

        WIZARD.with(|wizard| {
            *wizard.borrow_mut() = Some(WizardState {
                hwnd,
                owner,
                stage: Stage::Detecting,
                spin: 0.0,
                provider: None,
                selected,
                hover: None,
                hover_primary: false,
                hover_secondary: false,
                theme,
                receiver: Some(receiver),
                font_title: make_font(20, Weight::Semibold),
                font_body: make_font(13, Weight::Regular),
                font_row: make_font(14, Weight::Semibold),
                font_small: make_font(12, Weight::Regular),
                font_button: make_font(13, Weight::Semibold),
            })
        });

        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetTimer(hwnd, TIMER_SPIN, 40, None);
        SetTimer(hwnd, TIMER_POLL, 120, None);
    }
}

unsafe fn paint(state: &WizardState, hdc: windows_sys::Win32::Graphics::Gdi::HDC) {
    let canvas = match Canvas::new(metrics::WIZARD_WIDTH, metrics::WIZARD_HEIGHT) {
        Some(canvas) => canvas,
        None => return,
    };

    let theme = state.theme;
    canvas.clear(theme.surface);

    let margin = 28;
    let content_width = metrics::WIZARD_WIDTH - margin * 2;

    match state.stage {
        Stage::Detecting => {
            canvas.text(
                state.font_title,
                "Setting up",
                Rect::new(margin, 40, content_width, 30),
                theme.text,
                TextAlign::Left,
            );

            canvas.text(
                state.font_body,
                "Looking up your internet provider so the right preset can be chosen for you.",
                Rect::new(margin, 74, content_width, 44),
                theme.text_dim,
                TextAlign::Wrap,
            );

            let cx = metrics::WIZARD_WIDTH as f32 / 2.0;
            let cy = 214.0;

            canvas.ring(cx, cy, 17.0, 2.5, theme.separator);

            for step in 0..8 {
                let angle = state.spin - step as f32 * 0.26;
                let fade = step as f32 / 8.0;
                let px = cx + angle.cos() * 17.0;
                let py = cy + angle.sin() * 17.0;
                canvas.circle(px, py, 2.5, theme.accent.mix(theme.surface, fade));
            }
        }

        Stage::Detected => {
            canvas.text(
                state.font_title,
                "You are all set",
                Rect::new(margin, 40, content_width, 30),
                theme.text,
                TextAlign::Left,
            );

            let provider = state
                .provider
                .clone()
                .unwrap_or_else(|| "your provider".to_string());

            canvas.text(
                state.font_body,
                &format!(
                    "We recognised {} and picked the matching preset. You can change it any time from the tray menu.",
                    provider
                ),
                Rect::new(margin, 74, content_width, 60),
                theme.text_dim,
                TextAlign::Wrap,
            );

            let choice = &CHOICES[state.selected];
            let card = Rect::new(margin, 162, content_width, 68);

            canvas.fill_rounded(card, 10.0, theme.surface_raised);
            canvas.stroke_rounded(card, 10.0, theme.accent);

            let center_y = (card.y + card.h / 2) as f32;
            canvas.circle((margin + 30) as f32, center_y, 10.0, theme.accent);
            canvas.checkmark(
                (margin + 23) as f32,
                center_y - 7.0,
                14.0,
                theme.text_on_accent,
            );

            canvas.text(
                state.font_row,
                choice.name,
                Rect::new(margin + 52, card.y + 14, content_width - 64, 20),
                theme.text,
                TextAlign::Left,
            );
            canvas.text(
                state.font_small,
                choice.detail,
                Rect::new(margin + 52, card.y + 35, content_width - 64, 20),
                theme.text_dim,
                TextAlign::Left,
            );
        }

        Stage::Manual => {
            canvas.text(
                state.font_title,
                "Pick your provider",
                Rect::new(margin, 32, content_width, 30),
                theme.text,
                TextAlign::Left,
            );

            canvas.text(
                state.font_body,
                "We could not detect it automatically. Choose the one you use.",
                Rect::new(margin, 64, content_width, 38),
                theme.text_dim,
                TextAlign::Wrap,
            );

            for (index, choice) in CHOICES.iter().enumerate() {
                let y = list_top() + index as i32 * row_height();
                let row = Rect::new(margin, y, content_width, row_height() - 6);
                let selected = index == state.selected;
                let hovered = state.hover == Some(index);

                if selected {
                    let tint = if theme.dark { 0.8 } else { 0.92 };
                    canvas.fill_rounded(row, 9.0, theme.accent.mix(theme.surface, tint));
                    canvas.stroke_rounded(row, 9.0, theme.accent);
                } else if hovered {
                    canvas.fill_rounded(row, 9.0, theme.surface_raised);
                }

                let center_y = (row.y + row.h / 2) as f32;

                if selected {
                    canvas.circle((margin + 22) as f32, center_y, 8.0, theme.accent);
                    canvas.circle((margin + 22) as f32, center_y, 3.0, theme.text_on_accent);
                } else {
                    canvas.ring((margin + 22) as f32, center_y, 8.0, 1.4, theme.border);
                }

                canvas.text(
                    state.font_row,
                    choice.name,
                    Rect::new(margin + 42, row.y + 5, content_width - 54, 18),
                    theme.text,
                    TextAlign::Left,
                );
                canvas.text(
                    state.font_small,
                    choice.detail,
                    Rect::new(margin + 42, row.y + 21, content_width - 54, 16),
                    theme.text_dim,
                    TextAlign::Left,
                );
            }
        }
    }

    if state.stage != Stage::Detecting {
        let primary = primary_rect();
        let fill = if state.hover_primary {
            theme.accent.mix(crate::theme::Rgb(0, 0, 0), 0.14)
        } else {
            theme.accent
        };

        canvas.fill_rounded(primary, 8.0, fill);
        canvas.text(
            state.font_button,
            "Continue",
            primary,
            theme.text_on_accent,
            TextAlign::Center,
        );

        let secondary = secondary_rect();
        let label = if state.stage == Stage::Detected {
            "Choose myself"
        } else {
            "Skip"
        };

        if state.hover_secondary {
            canvas.fill_rounded(secondary, 8.0, theme.surface_raised);
        }

        canvas.text(
            state.font_button,
            label,
            secondary,
            theme.text_dim,
            TextAlign::Center,
        );
    }

    canvas.blit(hdc);
}

unsafe extern "system" fn wizard_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            WIZARD.with(|wizard| {
                if let Some(state) = wizard.borrow().as_ref() {
                    paint(state, hdc);
                }
            });
            EndPaint(hwnd, &ps);
            0
        }

        WM_TIMER => {
            match wparam {
                TIMER_SPIN => {
                    let spinning = WIZARD.with(|wizard| {
                        let mut borrow = wizard.borrow_mut();
                        match borrow.as_mut() {
                            Some(state) => {
                                state.spin += 0.32;
                                state.stage == Stage::Detecting
                            }
                            None => false,
                        }
                    });

                    if spinning {
                        InvalidateRect(hwnd, null(), 0);
                    } else {
                        KillTimer(hwnd, TIMER_SPIN);
                    }
                }

                TIMER_POLL => {
                    let resolved = WIZARD.with(|wizard| {
                        let mut borrow = wizard.borrow_mut();
                        let state = match borrow.as_mut() {
                            Some(state) => state,
                            None => return false,
                        };

                        let received = match state.receiver.as_ref() {
                            Some(receiver) => receiver.try_recv().ok(),
                            None => None,
                        };

                        match received {
                            Some(result) => {
                                state.receiver = None;

                                match result.and_then(|provider| {
                                    provider.preset.map(|preset| (provider.name, preset))
                                }) {
                                    Some((name, preset)) => {
                                        state.provider = Some(name);
                                        state.selected = CHOICES
                                            .iter()
                                            .position(|c| c.key == preset)
                                            .unwrap_or(3);
                                        state.stage = Stage::Detected;
                                    }
                                    None => state.stage = Stage::Manual,
                                }
                                true
                            }
                            None => false,
                        }
                    });

                    if resolved {
                        KillTimer(hwnd, TIMER_POLL);
                        InvalidateRect(hwnd, null(), 0);
                    }
                }

                _ => {}
            }
            0
        }

        WM_MOUSEMOVE => {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;

            let changed = WIZARD.with(|wizard| {
                let mut borrow = wizard.borrow_mut();
                let state = match borrow.as_mut() {
                    Some(state) => state,
                    None => return false,
                };

                let hover = if state.stage == Stage::Manual {
                    let index = (y - list_top()) / row_height();
                    if y >= list_top() && index >= 0 && (index as usize) < CHOICES.len() {
                        Some(index as usize)
                    } else {
                        None
                    }
                } else {
                    None
                };

                let primary = state.stage != Stage::Detecting && primary_rect().contains(x, y);
                let secondary = state.stage != Stage::Detecting && secondary_rect().contains(x, y);

                let changed = hover != state.hover
                    || primary != state.hover_primary
                    || secondary != state.hover_secondary;

                state.hover = hover;
                state.hover_primary = primary;
                state.hover_secondary = secondary;
                changed
            });

            if changed {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }

        WM_LBUTTONDOWN => {
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            if y < 24 {
                ReleaseCapture();
                SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
            }
            0
        }

        WM_LBUTTONUP => {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;

            enum Outcome {
                None,
                Repaint,
                Commit(&'static str),
            }

            let outcome = WIZARD.with(|wizard| {
                let mut borrow = wizard.borrow_mut();
                let state = match borrow.as_mut() {
                    Some(state) => state,
                    None => return Outcome::None,
                };

                if state.stage == Stage::Detecting {
                    return Outcome::None;
                }

                if primary_rect().contains(x, y) {
                    return Outcome::Commit(CHOICES[state.selected].key);
                }

                if secondary_rect().contains(x, y) {
                    if state.stage == Stage::Detected {
                        state.stage = Stage::Manual;
                        return Outcome::Repaint;
                    }
                    return Outcome::Commit(CHOICES[state.selected].key);
                }

                if state.stage == Stage::Manual {
                    let index = (y - list_top()) / row_height();
                    if y >= list_top() && index >= 0 && (index as usize) < CHOICES.len() {
                        state.selected = index as usize;
                        return Outcome::Repaint;
                    }
                }

                Outcome::None
            });

            match outcome {
                Outcome::Repaint => {
                    InvalidateRect(hwnd, null(), 0);
                }
                Outcome::Commit(preset) => {
                    let owner = WIZARD.with(|wizard| wizard.borrow().as_ref().map(|s| s.owner));
                    let index = CHOICES.iter().position(|c| c.key == preset).unwrap_or(3);
                    DestroyWindow(hwnd);
                    if let Some(owner) = owner {
                        PostMessageW(owner, WIZARD_DONE, index, 0);
                    }
                }
                Outcome::None => {}
            }
            0
        }

        WM_DESTROY => {
            WIZARD.with(|wizard| {
                if let Some(state) = wizard.borrow_mut().take() {
                    DeleteObject(state.font_title as _);
                    DeleteObject(state.font_body as _);
                    DeleteObject(state.font_row as _);
                    DeleteObject(state.font_small as _);
                    DeleteObject(state.font_button as _);
                }
            });
            0
        }

        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
