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

    /// A rounded border drawn as a ring, leaving the inside untouched.
    ///
    /// `outline` cannot do this: it paints the shape and then paints the
    /// interior back over it, so it needs a single flat colour to paint
    /// back - which silently flattens whatever gradient or glow is already
    /// underneath. Where the card is lit, the border has to be a stroke.
    pub fn ring(&self, area: R, radius: i32, thickness: i32, color: u32) {
        self.ring_alpha(area, radius, thickness, color, 1.0);
    }

    /// A ring at a fraction of its strength, for one that is fading.
    pub fn ring_alpha(&self, area: R, radius: i32, thickness: i32, color: u32, alpha: f32) {
        let alpha = alpha.clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        let pixels = self.slice();
        let thickness = thickness.max(1) as f32;
        let radius = radius.min(area.width() / 2).min(area.height() / 2).max(0) as f32;
        let inner = area.inset(thickness as i32);
        let inner_radius = (radius - thickness).max(0.0);
        let coverage = |area: &R, radius: f32, px: f32, py: f32| {
            let cx = px.clamp(area.l as f32 + radius, area.r as f32 - radius);
            let cy = py.clamp(area.t as f32 + radius, area.b as f32 - radius);
            let distance = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
            (radius - distance + 0.5).clamp(0.0, 1.0)
        };
        for y in area.t.max(0)..area.b.min(self.height) {
            let py = y as f32 + 0.5;
            for x in area.l.max(0)..area.r.min(self.width) {
                let px = x as f32 + 0.5;
                let on = (coverage(&area, radius, px, py) - coverage(&inner, inner_radius, px, py)) * alpha;
                if on <= 0.0 {
                    continue;
                }
                let index = (y * self.width + x) as usize;
                pixels[index] = mix(pixels[index], color, on);
            }
        }
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
    pub fn draw(&self, canvas: &Canvas, area: R) {
        blit(canvas, area, LOGO_BYTES, self.width, self.height);
    }
}

/// Premultiplied BGRA drawn into a box, scaled to fit, centred, never
/// cropped. The logo goes through this, and so does every class emblem.
///
/// The scaling is done here, by averaging the source pixels that fall
/// under each destination pixel, rather than by handing the job to
/// AlphaBlend. AlphaBlend ignores SetStretchBltMode - HALFTONE was
/// being set above this call and doing nothing - and point-samples
/// instead, so shrinking a 151x140 mark into a thirty pixel box threw
/// away nineteen pixels in twenty and kept whichever one it landed on.
/// That is what made the logo look chewed at every size in the app, and
/// it is exactly what a 48px emblem in a 24px slot would do again.
///
/// The source is premultiplied BGRA (build.rs premultiplies it for
/// exactly this kind of blending), so a box average of it is valid
/// without un-premultiplying first, and compositing is the plain
/// `source + destination * (1 - alpha)`.
pub fn blit(canvas: &Canvas, area: R, bytes: &[u8], source_width: i32, source_height: i32) {
    blit_lit(canvas, area, bytes, source_width, source_height, 0, 0.0);
}

/// One source pixel of the scaled-down image: premultiplied B, G, R, A.
type Cell = [u8; 4];

/// The source averaged down to exactly `width` x `height`.
///
/// Sampling into a buffer first, rather than compositing straight onto the
/// canvas, is what lets the halo be drawn from the SAME pixels the mark is
/// drawn from: the glow traces the shape that actually lands on screen, not
/// the full-size one it was scaled from.
/// The overlap of destination cell `d` with each source pixel it touches:
/// the first source index, and the weight of each one from there on.
///
/// This is the whole difference between a clean shrink and a blocky one.
/// Taking whole source pixels - the band from `d * src / dst` to the next
/// one - gives each destination pixel either N or N+1 of them depending on
/// where the boundaries happen to land, so neighbouring pixels are averaged
/// from different numbers of samples and the result grains up. At 48 into
/// 33 that is one sample or two, a 2:1 swing, and it reads as exactly the
/// pixellation it is. Weighting the partial pixels at each end makes every
/// destination pixel cover the same source AREA, whatever the ratio.
fn overlap(d: i32, source: i32, dest: i32, weights: &mut Vec<f32>) -> i32 {
    let from = d as f32 * source as f32 / dest as f32;
    let to = (d + 1) as f32 * source as f32 / dest as f32;
    let first = from.floor() as i32;
    let last = ((to.ceil() as i32) - 1).max(first);
    weights.clear();
    for index in first..=last {
        let lo = (index as f32).max(from);
        let hi = ((index + 1) as f32).min(to);
        weights.push((hi - lo).max(0.0));
    }
    first
}

fn sample(bytes: &[u8], source_width: i32, source_height: i32, width: i32, height: i32) -> Vec<Cell> {
    let mut cells = vec![[0u8; 4]; (width * height) as usize];
    let (mut rows, mut columns) = (Vec::new(), Vec::new());
    for dy in 0..height {
        let first_row = overlap(dy, source_height, height, &mut rows);
        for dx in 0..width {
            let first_column = overlap(dx, source_width, width, &mut columns);
            let (mut b, mut g, mut r, mut a, mut total) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for (row, weight_y) in rows.iter().enumerate() {
                let sy = (first_row + row as i32).clamp(0, source_height - 1);
                for (column, weight_x) in columns.iter().enumerate() {
                    let sx = (first_column + column as i32).clamp(0, source_width - 1);
                    let weight = weight_y * weight_x;
                    if weight <= 0.0 {
                        continue;
                    }
                    let index = ((sy * source_width + sx) * 4) as usize;
                    b += bytes[index] as f32 * weight;
                    g += bytes[index + 1] as f32 * weight;
                    r += bytes[index + 2] as f32 * weight;
                    a += bytes[index + 3] as f32 * weight;
                    total += weight;
                }
            }
            if total <= 0.0 {
                continue;
            }
            let pick = |sum: f32| (sum / total).round().clamp(0.0, 255.0) as u8;
            cells[(dy * width + dx) as usize] = [pick(b), pick(g), pick(r), pick(a)];
        }
    }
    cells
}

/// Premultiplied BGRA drawn to FILL a rectangle, clipped to a rounded
/// corner radius - the way an avatar is drawn.
///
/// `blit_lit` centres a transparent MARK in its box and lights the edges
/// it finds; this fills the box with an OPAQUE picture instead, so there
/// are no edges to find and the shape has to come from the clip. The
/// coverage is the same distance-to-a-rounded-rectangle `round` uses, so
/// a portrait's corners are antialiased to exactly the curve the badge
/// under it is drawn with rather than to a staircase a pixel off it.
pub fn blit_rounded(
    canvas: &Canvas,
    area: R,
    bytes: &[u8],
    source_width: i32,
    source_height: i32,
    radius: i32,
) {
    let (width, height) = (area.width(), area.height());
    if source_width <= 0 || source_height <= 0 || width <= 0 || height <= 0 {
        return;
    }
    let cells = sample(bytes, source_width, source_height, width, height);
    let pixels = canvas.slice();
    let radius = radius.min(width / 2).min(height / 2).max(0) as f32;
    let (left, top) = (area.l as f32, area.t as f32);
    let (right, bottom) = (area.r as f32, area.b as f32);
    for dy in 0..height {
        let ty = area.t + dy;
        if ty < 0 || ty >= canvas.height {
            continue;
        }
        let py = ty as f32 + 0.5;
        let cy = py.clamp(top + radius, bottom - radius);
        for dx in 0..width {
            let tx = area.l + dx;
            if tx < 0 || tx >= canvas.width {
                continue;
            }
            let px = tx as f32 + 0.5;
            let cx = px.clamp(left + radius, right - radius);
            let distance = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
            let inside = (radius - distance + 0.5).clamp(0.0, 1.0);
            if inside <= 0.0 {
                continue;
            }
            let [b, g, r, a] = cells[(dy * width + dx) as usize];
            let alpha = (a as f32 * inside) as u32;
            if alpha == 0 {
                continue;
            }
            // The cell is premultiplied, so scaling it by the clip's
            // coverage keeps it premultiplied and the composite stays the
            // plain `source + destination * (1 - alpha)`.
            let scale = |value: u8| (value as f32 * inside) as u32;
            let index = (ty * canvas.width + tx) as usize;
            let under = pixels[index];
            let keep = 255 - alpha.min(255);
            let out_r = scale(r) + (((under >> 16) & 0xFF) * keep) / 255;
            let out_g = scale(g) + (((under >> 8) & 0xFF) * keep) / 255;
            let out_b = scale(b) + ((under & 0xFF) * keep) / 255;
            pixels[index] = (out_r.min(255) << 16) | (out_g.min(255) << 8) | out_b.min(255);
        }
    }
}
/// A separable moving-average pass over one channel. Run twice it is close
/// enough to a gaussian for a halo, and it costs a few thousand adds on a
/// field this small rather than a kernel multiply per pixel.
fn blur(field: &[u16], width: i32, height: i32, radius: i32) -> Vec<u16> {
    let span = (radius * 2 + 1) as u32;
    let mut across = vec![0u16; field.len()];
    for y in 0..height {
        for x in 0..width {
            let mut total = 0u32;
            for k in -radius..=radius {
                let sx = (x + k).clamp(0, width - 1);
                total += field[(y * width + sx) as usize] as u32;
            }
            across[(y * width + x) as usize] = (total / span) as u16;
        }
    }
    let mut down = vec![0u16; field.len()];
    for y in 0..height {
        for x in 0..width {
            let mut total = 0u32;
            for k in -radius..=radius {
                let sy = (y + k).clamp(0, height - 1);
                total += across[(sy * width + x) as usize] as u32;
            }
            down[(y * width + x) as usize] = (total / span) as u16;
        }
    }
    down
}

/// Premultiplied BGRA drawn into a box, scaled to fit, centred, never
/// cropped - with an optional soft halo traced round its edges first.
///
/// The scaling is done here, by averaging the source pixels that fall
/// under each destination pixel, rather than by handing the job to
/// AlphaBlend. AlphaBlend ignores SetStretchBltMode - HALFTONE was
/// being set above this call and doing nothing - and point-samples
/// instead, so shrinking a 151x140 mark into a thirty pixel box threw
/// away nineteen pixels in twenty and kept whichever one it landed on.
/// That is what made the logo look chewed at every size in the app, and
/// it is exactly what a 48px emblem in a 24px slot would do again.
///
/// The source is premultiplied BGRA (build.rs premultiplies it for
/// exactly this kind of blending), so a box average of it is valid
/// without un-premultiplying first, and compositing is the plain
/// `source + destination * (1 - alpha)`.
///
/// The halo is the mark's own alpha, spread and blurred, painted in
/// `glow` UNDER the mark - so only the fringe that escapes past the edges
/// is ever seen. Drawing it under rather than over is what keeps a white
/// glow from washing the artwork out into a pale blob.
pub fn blit_lit(
    canvas: &Canvas,
    area: R,
    bytes: &[u8],
    source_width: i32,
    source_height: i32,
    glow: u32,
    strength: f32,
) {
    if source_width <= 0 || source_height <= 0 {
        return;
    }
    let fit = (area.width() as f32 / source_width as f32)
        .min(area.height() as f32 / source_height as f32);
    let width = (source_width as f32 * fit).round() as i32;
    let height = (source_height as f32 * fit).round() as i32;
    if width <= 0 || height <= 0 {
        return;
    }
    let x0 = area.l + (area.width() - width) / 2;
    let y0 = area.t + (area.height() - height) / 2;
    let cells = sample(bytes, source_width, source_height, width, height);

    if strength > 0.0 {
        // Two layers: a tight rim that reads as the mark being lit, and a
        // wider, fainter bloom under it that stops the rim looking like a
        // sticker cut out and pasted on. Both radii are scaled off the mark
        // rather than fixed, so the halo is the same thickness relative to
        // the emblem on a 4K panel as on a 1080p one.
        let size = width.min(height) as f32;
        for (fraction, share) in [(0.09_f32, 1.0_f32), (0.26, 0.42)] {
            let radius = ((size * fraction).round() as i32).clamp(1, 7);
            let (pad_w, pad_h) = (width + radius * 2, height + radius * 2);
            let mut field = vec![0u16; (pad_w * pad_h) as usize];
            for y in 0..height {
                for x in 0..width {
                    field[((y + radius) * pad_w + x + radius) as usize] =
                        cells[(y * width + x) as usize][3] as u16;
                }
            }
            let spread = blur(&blur(&field, pad_w, pad_h, radius), pad_w, pad_h, radius);
            let pixels = canvas.slice();
            for y in 0..pad_h {
                let ty = y0 - radius + y;
                if ty < 0 || ty >= canvas.height {
                    continue;
                }
                for x in 0..pad_w {
                    let tx = x0 - radius + x;
                    if tx < 0 || tx >= canvas.width {
                        continue;
                    }
                    let at = (y * pad_w + x) as usize;
                    // The spread MINUS the silhouette it came from. Without
                    // this the halo spends most of its strength under the
                    // mark, where the mark covers it, and what escapes past
                    // the edge is too faint to read as light - it just greys
                    // the outline. Taking the difference puts every bit of
                    // it outside the shape.
                    let outside = spread[at].saturating_sub(field[at]);
                    if outside == 0 {
                        continue;
                    }
                    // The difference peaks well below full alpha, so it is
                    // lifted before it is used; without the gain a "glow"
                    // at any sane strength is a smudge.
                    let coverage = (outside as f32 / 255.0 * 2.6 * strength * share).clamp(0.0, 1.0);
                    let index = (ty * canvas.width + tx) as usize;
                    pixels[index] = mix(pixels[index], glow, coverage);
                }
            }
        }
    }

    let pixels = canvas.slice();
    for dy in 0..height {
        let ty = y0 + dy;
        if ty < 0 || ty >= canvas.height {
            continue;
        }
        for dx in 0..width {
            let tx = x0 + dx;
            if tx < 0 || tx >= canvas.width {
                continue;
            }
            let [b, g, r, a] = cells[(dy * width + dx) as usize];
            if a == 0 {
                continue;
            }
            let (b, g, r, a) = (b as u32, g as u32, r as u32, a as u32);
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
