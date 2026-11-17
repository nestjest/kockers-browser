use std::{ptr, sync::OnceLock};
use windows_sys::Win32::{
    Foundation::{COLORREF, RECT},
    Graphics::{Gdi::HDC, GdiPlus::*},
};

#[derive(Clone, Copy)]
pub enum Icon {
    Back,
    Forward,
    Reload,
    Home,
    NewTab,
    Download,
    Menu,
    Minimize,
    Maximize,
    Restore,
    Pause,
    Play,
    Close,
    Check,
    Bookmark,
    BookmarkFilled,
    Profile,
    Search,
    Fullscreen,
    Minus,
}

impl Icon {
    fn path(self) -> &'static str {
        match self {
            Self::Back => "M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z",
            Self::Forward => "M12 4l-1.41 1.41L16.17 11H4v2h12.17l-5.58 5.59L12 20l8-8z",
            Self::Reload => {
                "M17.65 6.35C16.2 4.9 14.21 4 12 4c-4.42 0-7.99 3.58-7.99 8s3.57 8 7.99 8c3.73 0 6.84-2.55 7.73-6h-2.08c-.82 2.33-3.04 4-5.65 4-3.31 0-6-2.69-6-6s2.69-6 6-6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z"
            }
            Self::Home => "M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z",
            Self::NewTab => "M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z",
            Self::Download => "M5 20h14v-2H5v2zM19 9h-4V3H9v6H5l7 7 7-7z",
            Self::Menu => {
                "M12 8c1.1 0 2-.9 2-2s-.9-2-2-2-2 .9-2 2 .9 2 2 2zm0 2c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm0 6c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2z"
            }
            Self::Minimize | Self::Minus => "M19 13H5v-2h14v2z",
            Self::Maximize => {
                "M18 4H6c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zm0 14H6V6h12v12z"
            }
            Self::Restore => {
                "M3 5H1v16c0 1.1.9 2 2 2h16v-2H3V5zm18-4H7c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V3c0-1.1-.9-2-2-2zm0 16H7V3h14v14z"
            }
            Self::Pause => "M6 19h4V5H6v14zm8-14v14h4V5h-4z",
            Self::Play => "M8 5v14l11-7z",
            Self::Close => {
                "M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"
            }
            Self::Check => "M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z",
            Self::Bookmark => {
                "M17 3H7c-1.1 0-2 .9-2 2v16l7-3 7 3V5c0-1.1-.9-2-2-2zm0 15-5-2.18L7 18V5h10v13z"
            }
            Self::BookmarkFilled => "M17 3H7c-1.1 0-2 .9-2 2v16l7-3 7 3V5c0-1.1-.9-2-2-2z",
            Self::Profile => {
                "M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z"
            }
            Self::Search => {
                "M15.5 14h-.79l-.28-.27C15.41 12.59 16 11.11 16 9.5 16 5.91 13.09 3 9.5 3S3 5.91 3 9.5 5.91 16 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z"
            }
            Self::Fullscreen => {
                "M7 14H5v5h5v-2H7v-3zm-2-4h2V7h3V5H5v5zm12 7h-3v2h5v-5h-2v3zM14 5v2h3v3h2V5h-5z"
            }
        }
    }
}

static GDI_PLUS: OnceLock<usize> = OnceLock::new();

pub unsafe fn draw_icon(dc: HDC, target: &RECT, icon: Icon, color: COLORREF) {
    const SIZE: f32 = 18.0;
    let scale = SIZE / 24.0;
    let left = (target.left + target.right) as f32 / 2.0 - SIZE / 2.0;
    let top = (target.top + target.bottom) as f32 / 2.0 - SIZE / 2.0;
    let token = *GDI_PLUS.get_or_init(|| {
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let mut token = 0;
        unsafe { GdiplusStartup(&mut token, &input, ptr::null_mut()) };
        token
    });
    if token == 0 {
        return;
    }
    unsafe {
        let Some(path) =
            SvgPath::parse(icon.path()).and_then(|mut parser| parser.build(left, top, scale))
        else {
            return;
        };
        let mut graphics = ptr::null_mut();
        if GdipCreateFromHDC(dc, &mut graphics) == 0 {
            GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias8x8);
            let argb = 0xFF00_0000
                | (color & 0xFF)
                | ((color >> 8) & 0xFF) << 8
                | ((color >> 16) & 0xFF) << 16;
            let mut fill = ptr::null_mut();
            if GdipCreateSolidFill(argb, &mut fill) == 0 {
                GdipFillPath(graphics, fill.cast(), path);
                GdipDeleteBrush(fill.cast());
            }
            GdipDeleteGraphics(graphics);
        }
        GdipDeletePath(path);
    }
}

struct SvgPath<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> SvgPath<'a> {
    fn parse(data: &'a str) -> Option<Self> {
        (!data.is_empty()).then_some(Self {
            bytes: data.as_bytes(),
            position: 0,
        })
    }

    fn build(&mut self, left: f32, top: f32, scale: f32) -> Option<*mut GpPath> {
        let mut path = ptr::null_mut();
        if unsafe { GdipCreatePath(FillModeAlternate, &mut path) } != 0 {
            return None;
        }
        let mut command = 0u8;
        let mut current = (0.0, 0.0);
        let mut start = current;
        let mut previous_control = current;
        while self.position < self.bytes.len() {
            self.skip_separators();
            if self.position >= self.bytes.len() {
                break;
            }
            if self.bytes[self.position].is_ascii_alphabetic() {
                command = self.bytes[self.position];
                self.position += 1;
            }
            let relative = command.is_ascii_lowercase();
            let upper = command.to_ascii_uppercase();
            if upper == b'Z' {
                unsafe { GdipClosePathFigure(path) };
                current = start;
                continue;
            }
            let point = |x: f32, y: f32| (left + x * scale, top + y * scale);
            match upper {
                b'M' | b'L' => {
                    let next = absolute((self.number()?, self.number()?), current, relative);
                    if upper == b'M' {
                        unsafe { GdipStartPathFigure(path) };
                        start = next;
                        command = if relative { b'l' } else { b'L' };
                    } else {
                        add_line(path, point, current, next);
                    }
                    current = next;
                }
                b'H' => {
                    let x = self.number()?;
                    let next = (if relative { current.0 + x } else { x }, current.1);
                    add_line(path, point, current, next);
                    current = next;
                }
                b'V' => {
                    let y = self.number()?;
                    let next = (current.0, if relative { current.1 + y } else { y });
                    add_line(path, point, current, next);
                    current = next;
                }
                b'C' | b'S' => {
                    let first = if upper == b'C' {
                        absolute((self.number()?, self.number()?), current, relative)
                    } else {
                        (
                            2.0 * current.0 - previous_control.0,
                            2.0 * current.1 - previous_control.1,
                        )
                    };
                    let second = absolute((self.number()?, self.number()?), current, relative);
                    let end = absolute((self.number()?, self.number()?), current, relative);
                    add_bezier(path, point, current, first, second, end);
                    previous_control = second;
                    current = end;
                }
                _ => {
                    unsafe { GdipDeletePath(path) };
                    return None;
                }
            }
        }
        Some(path)
    }

    fn skip_separators(&mut self) {
        while self.position < self.bytes.len()
            && (self.bytes[self.position].is_ascii_whitespace()
                || self.bytes[self.position] == b',')
        {
            self.position += 1;
        }
    }

    fn number(&mut self) -> Option<f32> {
        self.skip_separators();
        let start = self.position;
        if self
            .bytes
            .get(self.position)
            .is_some_and(|value| matches!(value, b'+' | b'-'))
        {
            self.position += 1;
        }
        let mut exponent = false;
        let mut decimal_point = false;
        while self.position < self.bytes.len() {
            let value = self.bytes[self.position];
            if value.is_ascii_digit() {
                self.position += 1;
            } else if value == b'.' && !decimal_point {
                decimal_point = true;
                self.position += 1;
            } else if matches!(value, b'e' | b'E') && !exponent {
                exponent = true;
                self.position += 1;
                if self
                    .bytes
                    .get(self.position)
                    .is_some_and(|value| matches!(value, b'+' | b'-'))
                {
                    self.position += 1;
                }
            } else {
                break;
            }
        }
        std::str::from_utf8(self.bytes.get(start..self.position)?)
            .ok()?
            .parse()
            .ok()
    }
}

fn absolute(point: (f32, f32), current: (f32, f32), relative: bool) -> (f32, f32) {
    if relative {
        (current.0 + point.0, current.1 + point.1)
    } else {
        point
    }
}

fn add_line(
    path: *mut GpPath,
    map: impl Fn(f32, f32) -> (f32, f32),
    start: (f32, f32),
    end: (f32, f32),
) {
    let start = map(start.0, start.1);
    let end = map(end.0, end.1);
    unsafe { GdipAddPathLine(path, start.0, start.1, end.0, end.1) };
}

fn add_bezier(
    path: *mut GpPath,
    map: impl Fn(f32, f32) -> (f32, f32),
    start: (f32, f32),
    first: (f32, f32),
    second: (f32, f32),
    end: (f32, f32),
) {
    let points = [start, first, second, end].map(|(x, y)| map(x, y));
    unsafe {
        GdipAddPathBezier(
            path,
            points[0].0,
            points[0].1,
            points[1].0,
            points[1].1,
            points[2].0,
            points[2].1,
            points[3].0,
            points[3].1,
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compact_material_numbers() {
        let mut path = SvgPath::parse("M1.5-2.5h4.25v3e0").unwrap();
        assert_eq!(path.number(), None);
        let mut number = SvgPath::parse(" -2.5e1").unwrap();
        assert_eq!(number.number(), Some(-25.0));
        let mut adjacent = SvgPath::parse("3.15.69").unwrap();
        assert_eq!(adjacent.number(), Some(3.15));
        assert_eq!(adjacent.number(), Some(0.69));
    }

    #[test]
    fn every_toolbar_icon_has_a_valid_material_path() {
        let token = *GDI_PLUS.get_or_init(|| {
            let input = GdiplusStartupInput {
                GdiplusVersion: 1,
                ..Default::default()
            };
            let mut token = 0;
            unsafe { GdiplusStartup(&mut token, &input, ptr::null_mut()) };
            token
        });
        assert_ne!(token, 0);
        let icons = [
            Icon::Back,
            Icon::Forward,
            Icon::Reload,
            Icon::Home,
            Icon::NewTab,
            Icon::Download,
            Icon::Menu,
            Icon::Minimize,
            Icon::Maximize,
            Icon::Restore,
            Icon::Pause,
            Icon::Play,
            Icon::Close,
            Icon::Check,
            Icon::Bookmark,
            Icon::BookmarkFilled,
            Icon::Profile,
            Icon::Search,
            Icon::Fullscreen,
            Icon::Minus,
        ];
        for (index, icon) in icons.into_iter().enumerate() {
            let mut parser = SvgPath::parse(icon.path()).unwrap();
            let path = parser.build(0.0, 0.0, 1.0);
            assert!(
                path.is_some(),
                "material icon #{index} failed at byte {}",
                parser.position
            );
            unsafe { GdipDeletePath(path.unwrap()) };
        }
    }
}
