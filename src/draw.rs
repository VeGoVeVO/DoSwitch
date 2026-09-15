//! Drawing, into one bitmap, by hand where it matters.
//!
//! The panel is painted into a 32bpp DIB section and pushed to the screen
//! in a single blit, so nothing ever flickers and there is no partially
//! drawn frame to see.
//!
//! The rounded shapes are filled pixel by pixel rather than with GDI's
//! RoundRect, because GDI does not antialias: its corners are a visible
//! staircase at every scale. The coverage maths here is the ordinary
//! distance-to-a-rounded-rectangle, and it costs a few microseconds for a
//! window this size, on a repaint that only happens when something moved.
//!
//! Text is left to GDI, which is good at it. The order matters: every
//! direct pixel write happens BEFORE any GDI call touches the same
//! bitmap, because the two see each other only when told to.

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::*;

use crate::theme::{colorref, mix};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct R {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

impl R {
    pub fn new(l: i32, t: i32, w: i32, h: i32) -> R {
        R { l, t, r: l + w, b: t + h }
    }

    pub fn width(&self) -> i32 {
        self.r - self.l
    }

    pub fn height(&self) -> i32 {
        self.b - self.t
    }

    pub fn inset(&self, by: i32) -> R {
        R { l: self.l + by, t: self.t + by, r: self.r - by, b: self.b - by }
    }

    pub fn holds(&self, x: i32, y: i32) -> bool {
        x >= self.l && x < self.r && y >= self.t && y < self.b
    }

    pub fn rect(&self) -> RECT {
        RECT { left: self.l, top: self.t, right: self.r, bottom: self.b }
    }
}

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct Canvas {
    pub dc: HDC,
    pub width: i32,
    pub height: i32,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    pixels: *mut u32,
}

impl Canvas {
    pub fn new(reference: HDC, width: i32, height: i32) -> Option<Canvas> {
        unsafe {
            let dc = CreateCompatibleDC(reference);
            if dc.is_null() {
                return None;
            }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            info.bmiHeader.biHeight = -height; // top down, so row 0 is the top
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                dc,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            if bitmap.is_null() || bits.is_null() {
                DeleteDC(dc);
                return None;
            }
            let old_bitmap = SelectObject(dc, bitmap as HGDIOBJ);
            SetBkMode(dc, TRANSPARENT as i32);
            Some(Canvas {
                dc,
                width,
                height,
                bitmap,
                old_bitmap,
                pixels: bits as *mut u32,
            })
        }
    }

    fn slice(&self) -> &mut [u32] {
        unsafe {
            std::slice::from_raw_parts_mut(
                self.pixels,
                (self.width * self.height) as usize,
            )
        }
    }

    pub fn clear(&self, color: u32) {
        for pixel in self.slice() {
            *pixel = color;
        }
    }

    /// A rounded rectangle, antialiased, blended over what is there.
    pub fn round(&self, area: R, radius: i32, color: u32) {
        let pixels = self.slice();
        let radius = radius.min(area.width() / 2).min(area.height() / 2).max(0) as f32;
        let (left, top) = (area.l as f32, area.t as f32);
        let (right, bottom) = (area.r as f32, area.b as f32);
        let y0 = area.t.max(0);
        let y1 = area.b.min(self.height);
        let x0 = area.l.max(0);
        let x1 = area.r.min(self.width);
        for y in y0..y1 {
            let py = y as f32 + 0.5;
            let cy = py.clamp(top + radius, bottom - radius);
            for x in x0..x1 {
                let px = x as f32 + 0.5;
                let cx = px.clamp(left + radius, right - radius);
                let distance = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
                let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
                if coverage <= 0.0 {
                    continue;
                }
                let index = (y * self.width + x) as usize;
                pixels[index] = mix(pixels[index], color, coverage);
            }
        }
    }

    /// A rounded outline: the shape in the border colour, with the inside
    /// painted back over it. Both edges end up antialiased, which is what
    /// a stroke drawn as a stroke does not give.
    pub fn outline(&self, area: R, radius: i32, thickness: i32, edge: u32, inside: u32) {
        self.round(area, radius, edge);
        self.round(area.inset(thickness), (radius - thickness).max(0), inside);
    }

    pub fn dot(&self, cx: i32, cy: i32, radius: i32, color: u32) {
        self.round(
            R::new(cx - radius, cy - radius, radius * 2, radius * 2),
            radius,
            color,
        );
    }

    /// A status dot with the soft halo the web panel gives it: the glow is
    /// laid down first, wide and faint, then the solid dot on top. It is
    /// the one detail that makes an idle green light look lit rather than
    /// painted.
    pub fn lit_dot(&self, cx: i32, cy: i32, radius: i32, color: u32) {
        self.glow(cx, cy, radius * 3, color, 0.55);
        self.dot(cx, cy, radius, color);
    }

    /// A soft circular glow, brightest at the centre and fading to nothing
    /// at the edge, blended over what is there. Used for the header's leaf
    /// light and for the lit status dots.
    pub fn glow(&self, cx: i32, cy: i32, radius: i32, color: u32, strength: f32) {
        if radius <= 0 {
            return;
        }
        let pixels = self.slice();
        let r = radius as f32;
        let y0 = (cy - radius).max(0);
        let y1 = (cy + radius).min(self.height);
        let x0 = (cx - radius).max(0);
        let x1 = (cx + radius).min(self.width);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = (((x - cx).pow(2) + (y - cy).pow(2)) as f32).sqrt();
                if d >= r {
                    continue;
                }
                // Squared falloff, so the centre reads as a light and the
                // edge disappears into the surface rather than ringing it.
                let t = (1.0 - d / r).powi(2) * strength;
                let index = (y * self.width + x) as usize;
                pixels[index] = mix(pixels[index], color, t);
            }
        }
    }

    /// A vertical gradient filling an area, top colour to bottom colour.
    /// The header sits on one of these, which is most of why it reads as a
    /// header rather than a band of flat colour.
    pub fn vgradient(&self, area: R, top: u32, bottom: u32) {
        let pixels = self.slice();
        let y0 = area.t.max(0);
        let y1 = area.b.min(self.height);
        let x0 = area.l.max(0);
        let x1 = area.r.min(self.width);
        let span = (area.b - area.t).max(1) as f32;
        for y in y0..y1 {
            let t = (y - area.t) as f32 / span;
            let colour = mix(top, bottom, t);
            for x in x0..x1 {
                pixels[(y * self.width + x) as usize] = colour;
            }
        }
    }

    pub fn text(&self, text: &str, area: R, font: HFONT, color: u32, flags: u32) {
        unsafe {
            let old = SelectObject(self.dc, font as HGDIOBJ);
            SetTextColor(self.dc, colorref(color));
            let mut rect = area.rect();
            let wide_text = wide(text);
            DrawTextW(self.dc, wide_text.as_ptr(), -1, &mut rect, flags);
            SelectObject(self.dc, old);
        }
    }

    /// How tall this text is once wrapped to a width, so the panel can be
    /// made the size its own contents need in both languages.
    pub fn wrapped_height(&self, text: &str, width: i32, font: HFONT) -> i32 {
        unsafe {
            let old = SelectObject(self.dc, font as HGDIOBJ);
            let mut rect = RECT { left: 0, top: 0, right: width, bottom: 0 };
            let wide_text = wide(text);
            DrawTextW(
                self.dc,
                wide_text.as_ptr(),
                -1,
                &mut rect,
                DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
            );
            SelectObject(self.dc, old);
            rect.bottom - rect.top
        }
    }

    pub fn blit_to(&self, target: HDC, x: i32, y: i32) {
        unsafe {
            BitBlt(target, x, y, self.width, self.height, self.dc, 0, 0, SRCCOPY);
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old_bitmap);
            DeleteObject(self.bitmap as HGDIOBJ);
            DeleteDC(self.dc);
        }
    }
}

/// The logo, decoded at build time, drawn with its own alpha.
pub struct Logo {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    pub width: i32,
    pub height: i32,
}

include!(concat!(env!("OUT_DIR"), "/logo.rs"));
const LOGO_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/logo.bgra"));

impl Logo {
    pub fn new(reference: HDC) -> Option<Logo> {
        unsafe {
            let dc = CreateCompatibleDC(reference);
            if dc.is_null() {
                return None;
            }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = LOGO_WIDTH;
            info.bmiHeader.biHeight = -LOGO_HEIGHT;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                dc,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            if bitmap.is_null() || bits.is_null() {
                DeleteDC(dc);
                return None;
            }
            std::ptr::copy_nonoverlapping(
                LOGO_BYTES.as_ptr(),
                bits as *mut u8,
                LOGO_BYTES.len(),
            );
            let old = SelectObject(dc, bitmap as HGDIOBJ);
            Some(Logo { dc, bitmap, old, width: LOGO_WIDTH, height: LOGO_HEIGHT })
        }
    }

    /// Drawn into a box, scaled to fit, centred, never cropped.
    ///
    /// The scaling is done here, by averaging the source pixels that fall
    /// under each destination pixel, rather than by handing the job to
    /// AlphaBlend. AlphaBlend ignores SetStretchBltMode - HALFTONE was
    /// being set above this call and doing nothing - and point-samples
    /// instead, so shrinking a 151x140 mark into a thirty pixel box threw
    /// away nineteen pixels in twenty and kept whichever one it landed on.
    /// That is what made the logo look chewed at every size in the app.
    ///
    /// The source is premultiplied BGRA (build.rs premultiplies it for
    /// exactly this kind of blending), so a box average of it is valid
    /// without un-premultiplying first, and compositing is the plain
    /// `source + destination * (1 - alpha)`.
    pub fn draw(&self, canvas: &Canvas, area: R) {
        let fit = (area.width() as f32 / self.width as f32)
            .min(area.height() as f32 / self.height as f32);
        let width = (self.width as f32 * fit).round() as i32;
        let height = (self.height as f32 * fit).round() as i32;
        if width <= 0 || height <= 0 {
            return;
        }
        let x0 = area.l + (area.width() - width) / 2;
        let y0 = area.t + (area.height() - height) / 2;
        let pixels = canvas.slice();
        let source = |sx: i32, sy: i32| -> (u32, u32, u32, u32) {
            let index = ((sy * self.width + sx) * 4) as usize;
            (
                LOGO_BYTES[index] as u32,
                LOGO_BYTES[index + 1] as u32,
                LOGO_BYTES[index + 2] as u32,
                LOGO_BYTES[index + 3] as u32,
            )
        };
        for dy in 0..height {
            let ty = y0 + dy;
            if ty < 0 || ty >= canvas.height {
                continue;
            }
            // The half-open band of source rows this destination row covers.
            let sy0 = (dy * self.height / height).clamp(0, self.height - 1);
            let sy1 = (((dy + 1) * self.height + height - 1) / height).clamp(sy0 + 1, self.height);
            for dx in 0..width {
                let tx = x0 + dx;
                if tx < 0 || tx >= canvas.width {
                    continue;
                }
                let sx0 = (dx * self.width / width).clamp(0, self.width - 1);
                let sx1 = (((dx + 1) * self.width + width - 1) / width).clamp(sx0 + 1, self.width);
                let (mut b, mut g, mut r, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
                for sy in sy0..sy1 {
                    for sx in sx0..sx1 {
                        let (sb, sg, sr, sa) = source(sx, sy);
                        b += sb;
                        g += sg;
                        r += sr;
                        a += sa;
                        n += 1;
                    }
                }
                if n == 0 {
                    continue;
                }
                let (b, g, r, a) = (b / n, g / n, r / n, a / n);
                if a == 0 {
                    continue;
                }
                let index = (ty * canvas.width + tx) as usize;
                let under = pixels[index];
                let keep = 255 - a;
                let out_r = r + (((under >> 16) & 0xFF) * keep) / 255;
                let out_g = g + (((under >> 8) & 0xFF) * keep) / 255;
                let out_b = b + ((under & 0xFF) * keep) / 255;
                pixels[index] = (out_r.min(255) << 16) | (out_g.min(255) << 8) | out_b.min(255);
            }
        }
    }
}

impl Drop for Logo {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap as HGDIOBJ);
            DeleteDC(self.dc);
        }
    }
}

pub fn font(height: i32, weight: i32) -> HFONT {
    unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET.into(),
            OUT_DEFAULT_PRECIS.into(),
            CLIP_DEFAULT_PRECIS.into(),
            CLEARTYPE_QUALITY.into(),
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            wide("Segoe UI").as_ptr(),
        )
    }
}
