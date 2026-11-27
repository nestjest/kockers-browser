use std::{
    cell::RefCell,
    collections::HashMap,
    iter,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
use windows_sys::Win32::{
    Foundation::{COLORREF, RECT},
    Graphics::{Gdi::HDC, GdiPlus::*},
};

const MAX_CACHED_IMAGES: usize = 128;
const MAX_CACHED_IMAGE_DIM: u32 = 64;
const PIXEL_FORMAT_32BPP_PARGB: i32 = 0x000E_200B;
const ICON_COUNT: usize = 21;

static IMAGE_CACHE_GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn invalidate_image_cache() {
    IMAGE_CACHE_GENERATION.fetch_add(1, Ordering::Release);
}

#[derive(Clone, Copy)]
pub enum Icon {
    Back,
    Forward,
    Reload,
    Home,
    Kockers,
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
    Extensions,
    Search,
    Fullscreen,
    Minus,
}

impl Icon {
    fn index(self) -> usize {
        match self {
            Self::Back => 0,
            Self::Forward => 1,
            Self::Reload => 2,
            Self::Home => 3,
            Self::Kockers => 4,
            Self::NewTab => 5,
            Self::Download => 6,
            Self::Menu => 7,
            Self::Minimize => 8,
            Self::Maximize => 9,
            Self::Restore => 10,
            Self::Pause => 11,
            Self::Play => 12,
            Self::Close => 13,
            Self::Check => 14,
            Self::Bookmark => 15,
            Self::BookmarkFilled => 16,
            Self::Extensions => 17,
            Self::Search => 18,
            Self::Fullscreen => 19,
            Self::Minus => 20,
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::Back => "M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z",
            Self::Forward => "M12 4l-1.41 1.41L16.17 11H4v2h12.17l-5.58 5.59L12 20l8-8z",
            Self::Reload => {
                "M17.65 6.35C16.2 4.9 14.21 4 12 4c-4.42 0-7.99 3.58-7.99 8s3.57 8 7.99 8c3.73 0 6.84-2.55 7.73-6h-2.08c-.82 2.33-3.04 4-5.65 4-3.31 0-6-2.69-6-6s2.69-6 6-6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z"
            }
            Self::Home => "M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z",
            Self::Kockers => {
                "M248,31L239,37L225,51L215,68L208,91L206,117L210,151L234,263L254,368L271,479L256,487L180,517L133,540L88,570L59,598L45,617L34,637L28,653L23,680L23,700L26,720L32,738L40,754L55,773L75,789L97,800L126,807L159,807L190,801L224,787L253,768L286,734L303,707L318,674L333,616L339,571L342,508L345,503L367,519L387,543L407,577L439,642L464,685L499,734L532,769L564,794L596,809L614,813L635,813L657,807L669,800L681,789L691,772L695,759L696,742L693,727L687,712L679,700L664,686L651,678L631,670L599,663L577,656L559,648L532,631L506,609L494,596L445,528L422,502L395,482L357,467L433,412L504,370L658,287L702,258L733,234L764,204L772,194L784,175L791,159L795,141L795,120L792,106L786,94L772,78L763,72L747,65L729,62L711,63L689,69L672,77L651,91L635,104L599,140L573,170L478,295L432,350L382,401L344,433L342,423L339,352L339,268L343,212L354,130L354,112L351,90L346,72L338,56L321,37L312,31L296,25L284,23L263,25ZM273,524L273,572L266,622L255,657L239,686L211,715L182,729L154,731L142,728L129,721L115,706L110,696L107,684L107,671L110,657L124,630L149,604L187,575L231,547Z"
            }
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
            Self::Extensions => {
                "M20.5 11H19V7c0-1.1-.9-2-2-2h-4V3.5C13 2.12 11.88 1 10.5 1S8 2.12 8 3.5V5H4c-1.1 0-2 .9-2 2v3.8h1.5c1.49 0 2.7 1.21 2.7 2.7s-1.21 2.7-2.7 2.7H2V20c0 1.1.9 2 2 2h3.8v-1.5c0-1.49 1.21-2.7 2.7-2.7s2.7 1.21 2.7 2.7V22H17c1.1 0 2-.9 2-2v-4h1.5c1.38 0 2.5-1.12 2.5-2.5S21.88 11 20.5 11z"
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
static ICON_PATHS: [OnceLock<Option<Vec<PathOp>>>; ICON_COUNT] =
    [const { OnceLock::new() }; ICON_COUNT];

#[derive(Clone, Copy)]
enum PathOp {
    StartFigure,
    CloseFigure,
    Line((f32, f32), (f32, f32)),
    Bezier((f32, f32), (f32, f32), (f32, f32), (f32, f32)),
}

#[derive(Default)]
struct ImageCache {
    generation: u64,
    entries: HashMap<PathBuf, Option<*mut GpImage>>,
}

impl ImageCache {
    fn clear(&mut self) {
        for (_, image) in self.entries.drain() {
            if let Some(image) = image {
                unsafe { GdipDisposeImage(image) };
            }
        }
    }
}

thread_local! {
    static IMAGE_CACHE: RefCell<ImageCache> = RefCell::new(ImageCache::default());
}

pub fn clear_image_cache() {
    IMAGE_CACHE.with(|cell| {
        cell.borrow_mut().clear();
    });
}

fn ensure_gdi_plus() -> usize {
    *GDI_PLUS.get_or_init(|| {
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let mut token = 0;
        unsafe { GdiplusStartup(&mut token, &input, ptr::null_mut()) };
        token
    })
}

fn cached_icon_ops(icon: Icon) -> Option<&'static [PathOp]> {
    ICON_PATHS[icon.index()]
        .get_or_init(|| SvgPath::parse(icon.path()).and_then(|mut parser| parser.parse_ops()))
        .as_deref()
}

pub unsafe fn draw_icon(dc: HDC, target: &RECT, icon: Icon, color: COLORREF) {
    let size = if matches!(icon, Icon::Extensions) {
        16.0
    } else {
        18.0
    };
    let (view_width, view_height): (f32, f32) = if matches!(icon, Icon::Kockers) {
        (820.0, 838.0)
    } else {
        (24.0, 24.0)
    };
    let scale = size / view_width.max(view_height);
    let left = (target.left + target.right) as f32 / 2.0 - view_width * scale / 2.0;
    let top = (target.top + target.bottom) as f32 / 2.0 - view_height * scale / 2.0;
    if ensure_gdi_plus() == 0 {
        return;
    }
    unsafe {
        let Some(ops) = cached_icon_ops(icon) else {
            return;
        };
        let Some(path) = build_gp_path(ops, left, top, scale) else {
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

unsafe fn load_memory_image(path: &Path) -> Option<*mut GpImage> {
    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    unsafe {
        let mut file_bitmap = ptr::null_mut();
        if GdipCreateBitmapFromFile(wide_path.as_ptr(), &mut file_bitmap) != 0
            || file_bitmap.is_null()
        {
            return None;
        }
        let mut width = 0u32;
        let mut height = 0u32;
        let got_size = GdipGetImageWidth(file_bitmap.cast(), &mut width) == 0
            && GdipGetImageHeight(file_bitmap.cast(), &mut height) == 0
            && width > 0
            && height > 0;
        if !got_size {
            GdipDisposeImage(file_bitmap.cast());
            return None;
        }
        let max_dim = width.max(height);
        let (target_w, target_h) = if max_dim > MAX_CACHED_IMAGE_DIM {
            let scale = MAX_CACHED_IMAGE_DIM as f32 / max_dim as f32;
            (
                (width as f32 * scale).round().max(1.0) as i32,
                (height as f32 * scale).round().max(1.0) as i32,
            )
        } else {
            (width as i32, height as i32)
        };
        let mut memory_bitmap = ptr::null_mut();
        let created = GdipCreateBitmapFromScan0(
            target_w,
            target_h,
            0,
            PIXEL_FORMAT_32BPP_PARGB,
            ptr::null_mut(),
            &mut memory_bitmap,
        ) == 0
            && !memory_bitmap.is_null();
        if !created {
            GdipDisposeImage(file_bitmap.cast());
            return None;
        }
        let mut graphics = ptr::null_mut();
        let copied = if GdipGetImageGraphicsContext(memory_bitmap.cast(), &mut graphics) == 0 {
            GdipSetInterpolationMode(graphics, InterpolationModeHighQualityBicubic);
            let ok = GdipDrawImageRect(
                graphics,
                file_bitmap.cast(),
                0.0,
                0.0,
                target_w as f32,
                target_h as f32,
            ) == 0;
            GdipDeleteGraphics(graphics);
            ok
        } else {
            false
        };
        GdipDisposeImage(file_bitmap.cast());
        if copied {
            Some(memory_bitmap.cast())
        } else {
            GdipDisposeImage(memory_bitmap.cast());
            None
        }
    }
}

pub unsafe fn draw_image(dc: HDC, target: &RECT, path: &Path, size: i32) -> bool {
    if ensure_gdi_plus() == 0 {
        return false;
    }
    let current_gen = IMAGE_CACHE_GENERATION.load(Ordering::Acquire);
    let image = IMAGE_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if cache.generation != current_gen {
            cache.clear();
            cache.generation = current_gen;
        }
        if let Some(cached) = cache.entries.get(path) {
            return *cached;
        }
        if cache.entries.len() >= MAX_CACHED_IMAGES {
            cache.clear();
        }
        let loaded = unsafe { load_memory_image(path) };
        cache.entries.insert(path.to_path_buf(), loaded);
        loaded
    });
    let Some(image) = image else {
        return false;
    };
    unsafe {
        let mut graphics = ptr::null_mut();
        let mut drawn = false;
        if GdipCreateFromHDC(dc, &mut graphics) == 0 {
            GdipSetInterpolationMode(graphics, InterpolationModeHighQualityBicubic);
            let size = size as f32;
            drawn = GdipDrawImageRect(
                graphics,
                image,
                (target.left + target.right) as f32 / 2.0 - size / 2.0,
                (target.top + target.bottom) as f32 / 2.0 - size / 2.0,
                size,
                size,
            ) == 0;
            GdipDeleteGraphics(graphics);
        }
        drawn
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

    fn parse_ops(&mut self) -> Option<Vec<PathOp>> {
        let mut ops = Vec::new();
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
                ops.push(PathOp::CloseFigure);
                current = start;
                continue;
            }
            match upper {
                b'M' | b'L' => {
                    let next = absolute((self.number()?, self.number()?), current, relative);
                    if upper == b'M' {
                        ops.push(PathOp::StartFigure);
                        start = next;
                        command = if relative { b'l' } else { b'L' };
                    } else {
                        ops.push(PathOp::Line(current, next));
                    }
                    current = next;
                }
                b'H' => {
                    let x = self.number()?;
                    let next = (if relative { current.0 + x } else { x }, current.1);
                    ops.push(PathOp::Line(current, next));
                    current = next;
                }
                b'V' => {
                    let y = self.number()?;
                    let next = (current.0, if relative { current.1 + y } else { y });
                    ops.push(PathOp::Line(current, next));
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
                    ops.push(PathOp::Bezier(current, first, second, end));
                    previous_control = second;
                    current = end;
                }
                _ => {
                    return None;
                }
            }
        }
        Some(ops)
    }

    #[cfg(test)]
    fn build(&mut self, left: f32, top: f32, scale: f32) -> Option<*mut GpPath> {
        let ops = self.parse_ops()?;
        build_gp_path(&ops, left, top, scale)
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

fn build_gp_path(ops: &[PathOp], left: f32, top: f32, scale: f32) -> Option<*mut GpPath> {
    let mut path = ptr::null_mut();
    if unsafe { GdipCreatePath(FillModeAlternate, &mut path) } != 0 {
        return None;
    }
    let point = |x: f32, y: f32| (left + x * scale, top + y * scale);
    for op in ops {
        match *op {
            PathOp::StartFigure => unsafe {
                GdipStartPathFigure(path);
            },
            PathOp::CloseFigure => unsafe {
                GdipClosePathFigure(path);
            },
            PathOp::Line(start, end) => add_line(path, point, start, end),
            PathOp::Bezier(start, first, second, end) => {
                add_bezier(path, point, start, first, second, end);
            }
        }
    }
    Some(path)
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
            Icon::Kockers,
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
            Icon::Extensions,
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
