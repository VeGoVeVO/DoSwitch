//! The palette, written 0xRRGGBB, taken from the DoIntOFF accounts panel
//! (overlay/renderer/accounts.html) so the two apps are one design.
//!
//! 0xRRGGBB is already the layout a 32bpp DIB pixel wants. GDI's COLORREF
//! is the other way round, 0x00BBGGRR, so anything handed to a GDI call
//! goes through `colorref` first - a mistake there does not fail, it just
//! draws in the wrong colour, which is why the conversion has a name.
//!
//! The web panel leans on translucent borders (leaf at 16 percent, cream
//! at 8 percent). There is no cheap alpha for a stroke here, so those are
//! precomputed with `mix` against the surface they sit on, which is what
//! the eye was going to see anyway.

pub const INK: u32 = 0x121811; // --panel, the card
pub const SURFACE: u32 = 0x161d14; // --row
pub const RAISED: u32 = 0x192117; // --raised, the header top and hovers
pub const PILL: u32 = 0x101609; // the key and order fields
pub const LEAF: u32 = 0xa8d800;
pub const LEAF_DIM: u32 = 0x78a010; // column headings
pub const MOSS: u32 = 0x407010;
pub const CREAM: u32 = 0xf8f0c8;
pub const GOLD: u32 = 0xf8b000;
pub const MUTED: u32 = 0x98a48c; // --text-muted
pub const DIM: u32 = 0x5d6b52;

pub const fn colorref(rgb: u32) -> u32 {
    ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)
}

/// `a` faded towards `b`, per channel. Used for the hover states, for the
/// translucent borders the web panel draws, and for every antialiased
/// edge.
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let channel = |shift: u32| {
        let x = ((a >> shift) & 0xFF) as f32;
        let y = ((b >> shift) & 0xFF) as f32;
        (((x + (y - x) * t).round() as u32).min(255)) << shift
    };
    channel(16) | channel(8) | channel(0)
}

// The borders, precomputed. LINE is leaf at 16 percent over the card;
// LINE_SOFT is cream at 8 percent over a row or field.
pub fn line() -> u32 {
    mix(INK, LEAF, 0.16)
}
pub fn line_soft() -> u32 {
    mix(SURFACE, CREAM, 0.08)
}
/// A row belonging to an open client: leaf at 22 percent over the row.
pub fn line_live() -> u32 {
    mix(SURFACE, LEAF, 0.22)
}
/// The Next account row: gold at 22 percent.
pub fn line_gold() -> u32 {
    mix(SURFACE, GOLD, 0.22)
}
