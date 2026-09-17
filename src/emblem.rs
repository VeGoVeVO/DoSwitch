//! The class emblem that sits at the front of an account's row.
//!
//! The artwork is decoded at build time into one buffer of equal tiles
//! (see build.rs), so the executable carries pixels and the panel carries
//! no image decoder. A tile is found by its class id, and the id comes
//! from the breed the window title gave us - see `breeds::id_for`, which
//! is what knows that a Spanish client writes "Yopuka" for an Iop.
//!
//! `draw` REPORTS whether it drew. A class the table does not know - a
//! client language nobody has added yet, a class newer than the last time
//! the artwork was regenerated - gets no emblem and no stand-in, and the
//! panel falls back to the plain dot it has always drawn. A wrong emblem
//! beside a name is worse than no emblem, because nothing about it looks
//! wrong.

use crate::draw::{blit_lit, Canvas, R};

/// The halo behind an emblem: white, and soft enough that it reads as the
/// mark being lit rather than as an outline drawn round it. The panel is
/// nearly black and the artwork is saturated, so a white fringe is what
/// separates the two - a leaf-coloured one disappeared into the row's own
/// border and a black one just muddied the edge.
const HALO: u32 = 0x00FF_FFFF;
const HALO_STRENGTH: f32 = 0.55;

include!(concat!(env!("OUT_DIR"), "/breeds_atlas.rs"));
const EMBLEM_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/breeds.bgra"));

/// The bytes of one class's tile, or None when the atlas has no such id.
fn tile(id: u32) -> Option<&'static [u8]> {
    let slot = EMBLEM_IDS.iter().position(|&known| known == id)?;
    let size = (EMBLEM_TILE * EMBLEM_TILE * 4) as usize;
    EMBLEM_BYTES.get(slot * size..(slot + 1) * size)
}

/// Draw the emblem for this breed name into `area`. True when there was
/// one; false leaves the area untouched for the caller to fall back on.
pub fn draw(canvas: &Canvas, area: R, breed: &str) -> bool {
    let Some(bytes) = crate::breeds::id_for(breed).and_then(tile) else {
        return false;
    };
    blit_lit(canvas, area, bytes, EMBLEM_TILE, EMBLEM_TILE, HALO, HALO_STRENGTH);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every class the table names must have artwork, and every piece of
    /// artwork must belong to a class. The two are generated together by
    /// tools/fetch_breeds.py; this is what catches one of them being
    /// regenerated without the other, which would otherwise show up as a
    /// single class quietly losing its emblem.
    #[test]
    fn the_table_and_the_artwork_agree() {
        for (id, names) in crate::breeds::BREEDS {
            assert!(tile(id).is_some(), "class {id} ({}) has no emblem", names[0]);
        }
        for id in EMBLEM_IDS {
            assert!(
                crate::breeds::BREEDS.iter().any(|(known, _)| *known == id),
                "assets/breeds/{id}.png belongs to no class in the table",
            );
        }
    }

    /// A tile is a whole tile: a short atlas would otherwise be read as a
    /// smaller emblem and drawn stretched rather than refused.
    #[test]
    fn every_tile_is_whole() {
        let size = (EMBLEM_TILE * EMBLEM_TILE * 4) as usize;
        assert_eq!(EMBLEM_BYTES.len(), size * EMBLEM_IDS.len());
    }
}
