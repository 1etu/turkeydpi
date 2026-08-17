use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn colorref(self) -> u32 {
        (self.0 as u32) | ((self.1 as u32) << 8) | ((self.2 as u32) << 16)
    }

    pub fn packed(self) -> u32 {
        ((self.0 as u32) << 16) | ((self.1 as u32) << 8) | (self.2 as u32)
    }

    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb(
            lerp(self.0, other.0),
            lerp(self.1, other.1),
            lerp(self.2, other.2),
        )
    }
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub surface: Rgb,
    pub surface_raised: Rgb,
    pub border: Rgb,
    pub text: Rgb,
    pub text_dim: Rgb,
    pub text_on_accent: Rgb,
    pub accent: Rgb,
    pub separator: Rgb,
    pub success: Rgb,
    pub idle: Rgb,
}

impl Theme {
    pub fn current() -> Self {
        if system_prefers_dark() {
            Self::dark()
        } else {
            Self::light()
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            surface: Rgb(0xF6, 0xF6, 0xF7),
            surface_raised: Rgb(0xFF, 0xFF, 0xFF),
            border: Rgb(0xD8, 0xD8, 0xDC),
            text: Rgb(0x1D, 0x1D, 0x1F),
            text_dim: Rgb(0x86, 0x86, 0x8B),
            text_on_accent: Rgb(0xFF, 0xFF, 0xFF),
            accent: Rgb(0x0A, 0x84, 0xFF),
            separator: Rgb(0xE3, 0xE3, 0xE6),
            success: Rgb(0x30, 0xB5, 0x7E),
            idle: Rgb(0x98, 0x98, 0x9E),
        }
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            surface: Rgb(0x2A, 0x2A, 0x2C),
            surface_raised: Rgb(0x32, 0x32, 0x35),
            border: Rgb(0x48, 0x48, 0x4C),
            text: Rgb(0xF2, 0xF2, 0xF4),
            text_dim: Rgb(0x9A, 0x9A, 0xA0),
            text_on_accent: Rgb(0xFF, 0xFF, 0xFF),
            accent: Rgb(0x0A, 0x84, 0xFF),
            separator: Rgb(0x3D, 0x3D, 0x41),
            success: Rgb(0x32, 0xD7, 0x4B),
            idle: Rgb(0x7C, 0x7C, 0x82),
        }
    }
}

pub fn system_prefers_dark() -> bool {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");

    match key {
        Ok(key) => key
            .get_value::<u32, _>("AppsUseLightTheme")
            .map(|v| v == 0)
            .unwrap_or(false),
        Err(_) => false,
    }
}

pub mod metrics {
    pub const MENU_WIDTH: i32 = 248;

    pub const MENU_PAD_V: i32 = 6;
    pub const MENU_PAD_H: i32 = 6;
    pub const ROW_HEIGHT: i32 = 28;
    pub const ROW_RADIUS: f32 = 6.0;
    pub const ROW_TEXT_INSET: i32 = 30;
    pub const SECTION_HEIGHT: i32 = 24;
    pub const SEPARATOR_HEIGHT: i32 = 9;

    pub const TOAST_WIDTH: i32 = 348;
    pub const TOAST_HEIGHT: i32 = 92;

    pub const TOAST_MARGIN: i32 = 16;

    pub const WIZARD_WIDTH: i32 = 440;
    pub const WIZARD_HEIGHT: i32 = 404;
}
