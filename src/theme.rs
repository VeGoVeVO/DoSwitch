//! The palette, written 0xRRGGBB.
//!
//! That is already the layout a 32bpp DIB pixel wants. GDI's COLORREF is
//! the other way round, 0x00BBGGRR, so anything handed to a GDI call goes
//! through `colorref` first - a mistake there does not fail, it simply
//! draws in the wrong colour, which is why the conversion has a name.

pub const INK: u32 = 0x0b0f0c;
pub const SURFACE: u32 = 0x131c11;
pub const RAISED: u32 = 0x16200f;
pub const BORDER: u32 = 0x24331b;
pub const LEAF: u32 = 0xa8d800;
pub const MOSS: u32 = 0x407010;
pub const CREAM: u32 = 0xf8f0c8;
pub const GOLD: u32 = 0xf8b000;
pub const MUTED: u32 = 0x8a9a7a;
pub const DIM: u32 = 0x5d6b52;

pub const fn colorref(rgb: u32) -> u32 {
    ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)
}

/// `a` faded towards `b`, per channel. Used for the hover states and for
/// the edge pixels of everything with a rounded corner.
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let channel = |shift: u32| {
        let x = ((a >> shift) & 0xFF) as f32;
        let y = ((b >> shift) & 0xFF) as f32;
        (((x + (y - x) * t).round() as u32).min(255)) << shift
    };
    channel(16) | channel(8) | channel(0)
}
