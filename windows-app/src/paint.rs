use std::ffi::c_void;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW,
    GetDC, ReleaseDC, SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    CLEARTYPE_QUALITY, DIB_RGB_COLORS, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT,
    DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, FF_DONTCARE, FW_NORMAL, FW_SEMIBOLD, HDC, HFONT,
    SRCCOPY, TRANSPARENT,
};

use crate::theme::Rgb;

pub struct Canvas {
    pub dc: HDC,
    bitmap: windows_sys::Win32::Graphics::Gdi::HBITMAP,
    old_bitmap: windows_sys::Win32::Graphics::Gdi::HGDIOBJ,
    pixels: *mut u32,
    pub width: i32,
    pub height: i32,
}

impl Canvas {
    pub fn new(width: i32, height: i32) -> Option<Self> {
        unsafe {
            let screen = GetDC(null_mut());
            let dc = CreateCompatibleDC(screen);
            ReleaseDC(null_mut(), screen);

            if dc.is_null() {
                return None;
            }

            let mut header: BITMAPINFOHEADER = std::mem::zeroed();
            header.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            header.biWidth = width;
            header.biHeight = -height;
            header.biPlanes = 1;
            header.biBitCount = 32;
            header.biCompression = BI_RGB;

            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = header;

            let mut bits: *mut c_void = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);

            if bitmap.is_null() || bits.is_null() {
                DeleteDC(dc);
                return None;
            }

            let old_bitmap = SelectObject(dc, bitmap as _);
            SetBkMode(dc, TRANSPARENT as i32);

            Some(Self {
                dc,
                bitmap,
                old_bitmap,
                pixels: bits as *mut u32,
                width,
                height,
            })
        }
    }

    pub fn clear(&self, color: Rgb) {
        let value = color.packed();
        for index in 0..(self.width * self.height) as usize {
            unsafe { *self.pixels.add(index) = value };
        }
    }

    fn blend(&self, x: i32, y: i32, color: Rgb, alpha: f32) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }

        let alpha = alpha.clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }

        let index = (y * self.width + x) as usize;
        let current = unsafe { *self.pixels.add(index) };

        let cr = ((current >> 16) & 0xFF) as f32;
        let cg = ((current >> 8) & 0xFF) as f32;
        let cb = (current & 0xFF) as f32;

        let r = (cr + (color.0 as f32 - cr) * alpha).round() as u32;
        let g = (cg + (color.1 as f32 - cg) * alpha).round() as u32;
        let b = (cb + (color.2 as f32 - cb) * alpha).round() as u32;

        unsafe { *self.pixels.add(index) = (r << 16) | (g << 8) | b };
    }

    pub fn fill_rect(&self, rect: Rect, color: Rgb) {
        for py in rect.y..(rect.y + rect.h) {
            for px in rect.x..(rect.x + rect.w) {
                self.blend(px, py, color, 1.0);
            }
        }
    }

    pub fn fill_rounded(&self, rect: Rect, radius: f32, color: Rgb) {
        self.fill_rounded_alpha(rect, radius, color, 1.0);
    }

    pub fn fill_rounded_alpha(&self, rect: Rect, radius: f32, color: Rgb, alpha: f32) {
        let (x, y, w, h) = rect.parts();
        if w <= 0 || h <= 0 {
            return;
        }

        let radius = radius.min(w as f32 / 2.0).min(h as f32 / 2.0);
        let left = x as f32;
        let top = y as f32;
        let right = (x + w) as f32;
        let bottom = (y + h) as f32;

        for py in y..(y + h) {
            for px in x..(x + w) {
                let cx = px as f32 + 0.5;
                let cy = py as f32 + 0.5;

                let dx = if cx < left + radius {
                    left + radius - cx
                } else if cx > right - radius {
                    cx - (right - radius)
                } else {
                    0.0
                };

                let dy = if cy < top + radius {
                    top + radius - cy
                } else if cy > bottom - radius {
                    cy - (bottom - radius)
                } else {
                    0.0
                };

                let coverage = if dx == 0.0 && dy == 0.0 {
                    1.0
                } else {
                    let distance = (dx * dx + dy * dy).sqrt();
                    (radius - distance + 0.5).clamp(0.0, 1.0)
                };

                self.blend(px, py, color, coverage * alpha);
            }
        }
    }

    pub fn stroke_rounded(&self, rect: Rect, radius: f32, color: Rgb) {
        let (x, y, w, h) = rect.parts();
        if w <= 2 || h <= 2 {
            return;
        }

        let radius = radius.min(w as f32 / 2.0).min(h as f32 / 2.0);
        let left = x as f32;
        let top = y as f32;
        let right = (x + w) as f32;
        let bottom = (y + h) as f32;

        for py in y..(y + h) {
            for px in x..(x + w) {
                let cx = px as f32 + 0.5;
                let cy = py as f32 + 0.5;

                let dx = if cx < left + radius {
                    left + radius - cx
                } else if cx > right - radius {
                    cx - (right - radius)
                } else {
                    0.0
                };

                let dy = if cy < top + radius {
                    top + radius - cy
                } else if cy > bottom - radius {
                    cy - (bottom - radius)
                } else {
                    0.0
                };

                let outer = if dx == 0.0 && dy == 0.0 {
                    1.0
                } else {
                    let distance = (dx * dx + dy * dy).sqrt();
                    (radius - distance + 0.5).clamp(0.0, 1.0)
                };

                let edge_x = (cx - left).min(right - cx);
                let edge_y = (cy - top).min(bottom - cy);
                let inner_edge = edge_x.min(edge_y);

                let inner = if dx == 0.0 && dy == 0.0 {
                    (inner_edge - 1.0).clamp(0.0, 1.0)
                } else {
                    let distance = (dx * dx + dy * dy).sqrt();
                    (radius - 1.0 - distance + 0.5).clamp(0.0, 1.0)
                };

                let coverage = (outer - inner).clamp(0.0, 1.0);
                self.blend(px, py, color, coverage);
            }
        }
    }

    pub fn circle(&self, cx: f32, cy: f32, radius: f32, color: Rgb) {
        let min_x = (cx - radius - 1.0).floor() as i32;
        let max_x = (cx + radius + 1.0).ceil() as i32;
        let min_y = (cy - radius - 1.0).floor() as i32;
        let max_y = (cy + radius + 1.0).ceil() as i32;

        for py in min_y..max_y {
            for px in min_x..max_x {
                let dx = px as f32 + 0.5 - cx;
                let dy = py as f32 + 0.5 - cy;
                let distance = (dx * dx + dy * dy).sqrt();
                let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
                self.blend(px, py, color, coverage);
            }
        }
    }

    pub fn ring(&self, cx: f32, cy: f32, radius: f32, thickness: f32, color: Rgb) {
        let min_x = (cx - radius - 1.0).floor() as i32;
        let max_x = (cx + radius + 1.0).ceil() as i32;
        let min_y = (cy - radius - 1.0).floor() as i32;
        let max_y = (cy + radius + 1.0).ceil() as i32;

        for py in min_y..max_y {
            for px in min_x..max_x {
                let dx = px as f32 + 0.5 - cx;
                let dy = py as f32 + 0.5 - cy;
                let distance = (dx * dx + dy * dy).sqrt();
                let outer = (radius - distance + 0.5).clamp(0.0, 1.0);
                let inner = (radius - thickness - distance + 0.5).clamp(0.0, 1.0);
                self.blend(px, py, color, (outer - inner).clamp(0.0, 1.0));
            }
        }
    }

    pub fn checkmark(&self, x: f32, y: f32, size: f32, color: Rgb) {
        let points = [
            (0.18 * size, 0.52 * size),
            (0.42 * size, 0.74 * size),
            (0.84 * size, 0.24 * size),
        ];

        self.line(
            x + points[0].0,
            y + points[0].1,
            x + points[1].0,
            y + points[1].1,
            1.6,
            color,
        );
        self.line(
            x + points[1].0,
            y + points[1].1,
            x + points[2].0,
            y + points[2].1,
            1.6,
            color,
        );
    }

    pub fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, thickness: f32, color: Rgb) {
        let steps = ((x1 - x0).abs().max((y1 - y0).abs()) * 3.0).ceil().max(1.0) as i32;

        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            let px = x0 + (x1 - x0) * t;
            let py = y0 + (y1 - y0) * t;
            self.circle(px, py, thickness / 2.0, color);
        }
    }

    pub fn text(&self, font: HFONT, content: &str, rect: Rect, color: Rgb, align: TextAlign) {
        let (x, y, w, h) = rect.parts();
        unsafe {
            let old = SelectObject(self.dc, font as _);
            SetTextColor(self.dc, color.colorref());

            let mut rect = RECT {
                left: x,
                top: y,
                right: x + w,
                bottom: y + h,
            };

            let mut wide: Vec<u16> = content.encode_utf16().collect();
            let flags = match align {
                TextAlign::Left => DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                TextAlign::Right => DT_RIGHT | DT_SINGLELINE | DT_VCENTER,
                TextAlign::Center => DT_CENTER | DT_SINGLELINE | DT_VCENTER,
                TextAlign::Wrap => DT_LEFT | DT_WORDBREAK,
            };

            DrawTextW(
                self.dc,
                wide.as_mut_ptr(),
                wide.len() as i32,
                &mut rect,
                flags | DT_NOPREFIX,
            );

            SelectObject(self.dc, old);
        }
    }

    pub fn blit(&self, target: HDC) {
        unsafe {
            BitBlt(
                target,
                0,
                0,
                self.width,
                self.height,
                self.dc,
                0,
                0,
                SRCCOPY,
            );
        }
    }

    pub fn blit_to_window(&self, hwnd: HWND) {
        unsafe {
            let dc = GetDC(hwnd);
            self.blit(dc);
            ReleaseDC(hwnd, dc);
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old_bitmap);
            DeleteObject(self.bitmap as _);
            DeleteDC(self.dc);
        }
    }
}

#[derive(Clone, Copy)]
pub enum TextAlign {
    Left,
    Right,
    Center,
    Wrap,
}

#[derive(Clone, Copy)]
pub enum Weight {
    Regular,
    Semibold,
}

pub fn make_font(size: i32, weight: Weight) -> HFONT {
    let face: Vec<u16> = "Segoe UI Variable Text\0".encode_utf16().collect();
    let fallback: Vec<u16> = "Segoe UI\0".encode_utf16().collect();

    let weight_value = match weight {
        Weight::Regular => FW_NORMAL,
        Weight::Semibold => FW_SEMIBOLD,
    };

    unsafe {
        let font = CreateFontW(
            -size,
            0,
            0,
            0,
            weight_value as i32,
            0,
            0,
            0,
            0,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            FF_DONTCARE as u32,
            face.as_ptr(),
        );

        if font.is_null() {
            CreateFontW(
                -size,
                0,
                0,
                0,
                weight_value as i32,
                0,
                0,
                0,
                0,
                0,
                0,
                CLEARTYPE_QUALITY as u32,
                FF_DONTCARE as u32,
                fallback.as_ptr(),
            )
        } else {
            font
        }
    }
}

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[derive(Clone, Copy)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn parts(&self) -> (i32, i32, i32, i32) {
        (self.x, self.y, self.w, self.h)
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}
