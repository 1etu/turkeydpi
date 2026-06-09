use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

const SIZE: i32 = 16;

pub fn make_icon(enabled: bool) -> HICON {
    unsafe { build(enabled) }
}

unsafe fn build(enabled: bool) -> HICON {
    let mut header: BITMAPINFOHEADER = std::mem::zeroed();
    header.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    header.biWidth = SIZE;
    header.biHeight = -SIZE;
    header.biPlanes = 1;
    header.biBitCount = 32;
    header.biCompression = BI_RGB;

    let mut info: BITMAPINFO = std::mem::zeroed();
    info.bmiHeader = header;

    let screen = GetDC(null_mut::<c_void>() as HWND);
    let mut bits: *mut c_void = null_mut();
    let color = CreateDIBSection(screen, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    ReleaseDC(null_mut::<c_void>() as HWND, screen);

    if color.is_null() || bits.is_null() {
        return null_mut();
    }

    let (r, g, b) = if enabled {
        (0x30u32, 0xB5u32, 0x7Eu32)
    } else {
        (0x88u32, 0x8Fu32, 0x94u32)
    };

    let pixels = bits as *mut u32;
    let center = (SIZE as f32 - 1.0) / 2.0;
    let radius = 6.6f32;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let distance = (dx * dx + dy * dy).sqrt();

            let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
            let alpha = (coverage * 255.0) as u32;

            let pr = r * alpha / 255;
            let pg = g * alpha / 255;
            let pb = b * alpha / 255;

            *pixels.add((y * SIZE + x) as usize) = (alpha << 24) | (pr << 16) | (pg << 8) | pb;
        }
    }

    let mask: HBITMAP = CreateBitmap(SIZE, SIZE, 1, 1, null());

    let mut icon_info: ICONINFO = std::mem::zeroed();
    icon_info.fIcon = 1;
    icon_info.hbmMask = mask;
    icon_info.hbmColor = color;

    let icon = CreateIconIndirect(&icon_info);

    DeleteObject(color as _);
    DeleteObject(mask as _);

    icon
}
