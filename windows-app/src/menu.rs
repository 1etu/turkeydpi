use std::cell::RefCell;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, DeleteObject, EndPaint, InvalidateRect, HFONT, PAINTSTRUCT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, PostMessageW, RegisterClassW,
    SetForegroundWindow, SetWindowPos, ShowWindow, SystemParametersInfoW, CS_DROPSHADOW,
    HWND_TOPMOST, SPI_GETWORKAREA, SWP_NOACTIVATE, SW_SHOWNOACTIVATE, WM_ACTIVATE, WM_DESTROY,
    WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};

use crate::paint::{make_font, wide, Canvas, Rect, TextAlign, Weight};
use crate::theme::{metrics, Theme};

pub const MENU_COMMAND: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 20;

pub const ACTION_TOGGLE: usize = 1;
pub const ACTION_SETUP: usize = 2;
pub const ACTION_QUIT: usize = 3;
pub const ACTION_PRESET_BASE: usize = 100;

pub struct MenuModel {
    pub enabled: bool,
    pub preset: usize,
    pub provider: Option<String>,
    pub presets: Vec<(String, String)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Header,
    Separator,
    Section,
    Toggle,
    Preset(usize),
    Setup,
    Quit,
}

impl Row {
    fn height(&self) -> i32 {
        match self {
            Row::Header => 46,
            Row::Separator => metrics::SEPARATOR_HEIGHT,
            Row::Section => metrics::SECTION_HEIGHT,
            _ => metrics::ROW_HEIGHT,
        }
    }

    fn selectable(&self) -> bool {
        matches!(self, Row::Toggle | Row::Preset(_) | Row::Setup | Row::Quit)
    }

    fn action(&self) -> Option<usize> {
        match self {
            Row::Toggle => Some(ACTION_TOGGLE),
            Row::Setup => Some(ACTION_SETUP),
            Row::Quit => Some(ACTION_QUIT),
            Row::Preset(index) => Some(ACTION_PRESET_BASE + index),
            _ => None,
        }
    }
}

struct MenuState {
    hwnd: HWND,
    owner: HWND,
    model: MenuModel,
    rows: Vec<Row>,
    hover: Option<usize>,
    theme: Theme,
    font_title: HFONT,
    font_row: HFONT,
    font_small: HFONT,
    font_section: HFONT,
}

impl MenuState {
    fn row_bounds(&self, index: usize) -> (i32, i32) {
        let mut y = metrics::MENU_PAD_V;
        for (i, row) in self.rows.iter().enumerate() {
            if i == index {
                return (y, row.height());
            }
            y += row.height();
        }
        (y, 0)
    }

    fn hit(&self, y: i32) -> Option<usize> {
        let mut top = metrics::MENU_PAD_V;
        for (index, row) in self.rows.iter().enumerate() {
            let height = row.height();
            if y >= top && y < top + height && row.selectable() {
                return Some(index);
            }
            top += height;
        }
        None
    }

    fn total_height(&self) -> i32 {
        metrics::MENU_PAD_V * 2 + self.rows.iter().map(|r| r.height()).sum::<i32>()
    }
}

thread_local! {
    static MENU: RefCell<Option<MenuState>> = const { RefCell::new(None) };
}

fn build_rows(model: &MenuModel) -> Vec<Row> {
    let mut rows = vec![
        Row::Header,
        Row::Separator,
        Row::Toggle,
        Row::Separator,
        Row::Section,
    ];

    for index in 0..model.presets.len() {
        rows.push(Row::Preset(index));
    }

    rows.push(Row::Separator);
    rows.push(Row::Setup);
    rows.push(Row::Quit);
    rows
}

pub fn is_open() -> bool {
    MENU.with(|menu| menu.borrow().is_some())
}

pub fn close() {
    let hwnd = MENU.with(|menu| menu.borrow().as_ref().map(|state| state.hwnd));

    if let Some(hwnd) = hwnd {
        unsafe { DestroyWindow(hwnd) };
    }
}

pub fn show(owner: HWND, model: MenuModel) {
    if is_open() {
        close();
    }

    unsafe {
        let instance = GetModuleHandleW(null());
        let class_name = wide("TurkeyDPIMenu");

        let mut class: WNDCLASSW = std::mem::zeroed();
        class.style = CS_DROPSHADOW;
        class.lpfnWndProc = Some(menu_proc);
        class.hInstance = instance;
        class.lpszClassName = class_name.as_ptr();
        RegisterClassW(&class);

        let theme = Theme::current();
        let rows = build_rows(&model);

        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class_name.as_ptr(),
            wide("TurkeyDPI").as_ptr(),
            WS_POPUP,
            0,
            0,
            metrics::MENU_WIDTH,
            100,
            null_mut(),
            null_mut(),
            instance,
            null_mut(),
        );

        if hwnd.is_null() {
            return;
        }

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

        let state = MenuState {
            hwnd,
            owner,
            model,
            rows,
            hover: None,
            theme,
            font_title: make_font(15, Weight::Semibold),
            font_row: make_font(14, Weight::Regular),
            font_small: make_font(12, Weight::Regular),
            font_section: make_font(11, Weight::Semibold),
        };

        let height = state.total_height();

        MENU.with(|menu| *menu.borrow_mut() = Some(state));

        let (x, y) = anchor_position(height);
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            metrics::MENU_WIDTH,
            height,
            SWP_NOACTIVATE,
        );

        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetForegroundWindow(hwnd);
        SetCapture(hwnd);
    }
}

fn anchor_position(height: i32) -> (i32, i32) {
    unsafe {
        let mut cursor: POINT = std::mem::zeroed();
        GetCursorPos(&mut cursor);

        let mut work: RECT = std::mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut _ as *mut _, 0);

        let margin = 8;
        let mut x = cursor.x - metrics::MENU_WIDTH / 2;
        let mut y = cursor.y - height - margin;

        if x + metrics::MENU_WIDTH > work.right - margin {
            x = work.right - metrics::MENU_WIDTH - margin;
        }
        if x < work.left + margin {
            x = work.left + margin;
        }
        if y < work.top + margin {
            y = cursor.y + margin;
        }

        (x, y)
    }
}

unsafe fn paint(state: &MenuState, hdc: windows_sys::Win32::Graphics::Gdi::HDC) {
    let width = metrics::MENU_WIDTH;
    let height = state.total_height();

    let canvas = match Canvas::new(width, height) {
        Some(canvas) => canvas,
        None => return,
    };

    let theme = state.theme;
    canvas.clear(theme.surface);

    let pad = metrics::MENU_PAD_H;
    let inner_width = width - pad * 2;

    for (index, row) in state.rows.iter().enumerate() {
        let (top, row_height) = state.row_bounds(index);
        let hovered = state.hover == Some(index);

        if hovered && row.selectable() {
            canvas.fill_rounded(
                Rect::new(pad, top, inner_width, row_height),
                metrics::ROW_RADIUS,
                theme.accent,
            );
        }

        let label_color = if hovered {
            theme.text_on_accent
        } else {
            theme.text
        };

        let label_rect = Rect::new(pad + 12, top, inner_width - 24, row_height);

        match row {
            Row::Header => {
                let dot_color = if state.model.enabled {
                    theme.success
                } else {
                    theme.idle
                };

                canvas.circle((pad + 12) as f32, (top + 17) as f32, 4.5, dot_color);

                canvas.text(
                    state.font_title,
                    "TurkeyDPI",
                    Rect::new(pad + 24, top + 5, inner_width - 70, 22),
                    theme.text,
                    TextAlign::Left,
                );

                let status = if state.model.enabled { "On" } else { "Off" };
                let status_color = if state.model.enabled {
                    theme.success
                } else {
                    theme.text_dim
                };

                canvas.text(
                    state.font_small,
                    status,
                    Rect::new(width - pad - 52, top + 5, 44, 22),
                    status_color,
                    TextAlign::Right,
                );

                let subtitle = state
                    .model
                    .provider
                    .clone()
                    .unwrap_or_else(|| "Provider not detected".to_string());

                canvas.text(
                    state.font_small,
                    &subtitle,
                    Rect::new(pad + 24, top + 24, inner_width - 30, 18),
                    theme.text_dim,
                    TextAlign::Left,
                );
            }

            Row::Separator => {
                canvas.fill_rect(
                    Rect::new(pad + 8, top + row_height / 2, inner_width - 16, 1),
                    theme.separator,
                );
            }

            Row::Section => {
                canvas.text(
                    state.font_section,
                    "PRESET",
                    label_rect,
                    theme.text_dim,
                    TextAlign::Left,
                );
            }

            Row::Toggle => {
                let label = if state.model.enabled {
                    "Turn off"
                } else {
                    "Turn on"
                };

                canvas.text(
                    state.font_row,
                    label,
                    label_rect,
                    label_color,
                    TextAlign::Left,
                );
            }

            Row::Preset(preset_index) => {
                let (name, _) = &state.model.presets[*preset_index];

                if *preset_index == state.model.preset {
                    let tick = if hovered {
                        theme.text_on_accent
                    } else {
                        theme.accent
                    };
                    canvas.checkmark(
                        (pad + 10) as f32,
                        (top + (row_height - 14) / 2) as f32,
                        14.0,
                        tick,
                    );
                }

                canvas.text(
                    state.font_row,
                    name,
                    Rect::new(
                        pad + metrics::ROW_TEXT_INSET,
                        top,
                        inner_width - metrics::ROW_TEXT_INSET - 12,
                        row_height,
                    ),
                    label_color,
                    TextAlign::Left,
                );
            }

            Row::Setup => {
                canvas.text(
                    state.font_row,
                    "Setup assistant",
                    label_rect,
                    label_color,
                    TextAlign::Left,
                );
            }

            Row::Quit => {
                canvas.text(
                    state.font_row,
                    "Quit TurkeyDPI",
                    label_rect,
                    label_color,
                    TextAlign::Left,
                );
            }
        }
    }

    canvas.blit(hdc);
}

unsafe extern "system" fn menu_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            MENU.with(|menu| {
                if let Some(state) = menu.borrow().as_ref() {
                    paint(state, hdc);
                }
            });
            EndPaint(hwnd, &ps);
            0
        }

        WM_MOUSEMOVE => {
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            let x = (lparam & 0xFFFF) as i16 as i32;

            let changed = MENU.with(|menu| {
                let mut borrow = menu.borrow_mut();
                let state = match borrow.as_mut() {
                    Some(state) => state,
                    None => return false,
                };

                let bounds = Rect::new(0, 0, metrics::MENU_WIDTH, state.total_height());
                let inside = bounds.contains(x, y);
                let hover = if inside { state.hit(y) } else { None };

                if hover != state.hover {
                    state.hover = hover;
                    true
                } else {
                    false
                }
            });

            if changed {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }

        WM_LBUTTONUP => {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;

            let result = MENU.with(|menu| {
                let borrow = menu.borrow();
                let state = borrow.as_ref()?;

                let bounds = Rect::new(0, 0, metrics::MENU_WIDTH, state.total_height());
                let inside = bounds.contains(x, y);
                if !inside {
                    return Some((state.owner, None));
                }

                let action = state.hit(y).and_then(|index| state.rows[index].action());
                Some((state.owner, action))
            });

            if let Some((owner, action)) = result {
                DestroyWindow(hwnd);
                if let Some(action) = action {
                    PostMessageW(owner, MENU_COMMAND, action, 0);
                }
            }
            0
        }

        WM_KEYDOWN => {
            if wparam as u16 == VK_ESCAPE {
                DestroyWindow(hwnd);
            }
            0
        }

        WM_ACTIVATE => {
            if (wparam & 0xFFFF) == 0 {
                DestroyWindow(hwnd);
            }
            0
        }

        WM_DESTROY => {
            ReleaseCapture();
            MENU.with(|menu| {
                if let Some(state) = menu.borrow_mut().take() {
                    DeleteObject(state.font_title as _);
                    DeleteObject(state.font_row as _);
                    DeleteObject(state.font_small as _);
                    DeleteObject(state.font_section as _);
                }
            });
            0
        }

        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
