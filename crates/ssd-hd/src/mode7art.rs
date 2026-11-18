//! HD Mode 7: the rotation and scaling plane (the world map the flights fly over) drawn again from
//! the approved art.
//!
//! The plane is 128 x 128 tiles, 1024 x 1024 game pixels, made of the tile map (the low bytes of
//! the first 16384 words of VRAM) and the 8-bit characters (their high bytes). The art is one
//! picture of the whole plane at [`SCALE`] art pixels to a game pixel, found by the content of that
//! VRAM ([`plane_key`]) and drawn through the same matrix the game gives each line (HDMA changes it
//! from line to line: the perspective of a flight), sampled bilinearly between the art's pixels. The
//! art is made in the colours of one palette (`palette`, 256 colours): a game colour that is not
//! the one the art was made for (a fade, a flash) changes the art as it changed the game's pixel
//! ([`crate::bgart::adapt`]).
//!
//! A plane that is far away on a line (the horizon of a flight) is sampled from a smaller copy of
//! the art, as a texture is: the copies are halved with a box filter and the two nearest are
//! blended, so the horizon does not shimmer. Everything is integer arithmetic: the picture does
//! not depend on the machine.

use std::path::Path;

/// The art's index in the pack: `<pack>/art/mode7/mode7.toml`.
pub const INDEX: &str = "art/mode7/mode7.toml";

/// Art pixels along a game pixel's edge in the art files.
pub const SCALE: usize = 4;

/// Game pixels along a tile of the art's edge.
pub const TILE: usize = 256;

/// The plane's size in game pixels.
pub const PLANE: usize = 1024;

/// The words of VRAM that make the plane: the tile map and the characters.
const PLANE_WORDS: usize = 0x4000;

/// Why the plane's art cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum Mode7Error {
    #[error("mode 7: {0}")]
    Read(String),
}

/// `mode7.toml`: the planes with art.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Index {
    #[serde(default)]
    pub plane: Vec<Entry>,
}

/// The art of one plane.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    /// The name of the art (its folder), for reports.
    pub id: String,
    /// [`plane_key`] of the plane's VRAM, 16 hexadecimal digits; or [`CONTENT_PREFIX`] and the 16 hexadecimal digits of
    /// its [`content_key`], for art found by what the plane shows whatever else VRAM holds (see [`Index::find_plane`]).
    pub key: String,
    /// The art tiles (RGBA, `TILE * SCALE` pixels square: a pixel with alpha 0 has no art), relative to the index's folder, with
    /// `{x}` and `{y}` the tile's column and row.
    pub tiles: String,
    /// The 256 colours of the palette the art was made in, `RRGGBB` each (index 0 included).
    pub palette: Vec<String>,
    /// Where the art is transparent and the plane is not, what is under the plane shows rather than the game's pixel of
    /// the plane: a picture of the art override with a file `replace` (see `Pack::with_art_override`). Never read from
    /// `mode7.toml`.
    #[serde(skip)]
    pub replace: bool,
}

impl Entry {
    /// The key as a number.
    #[must_use]
    pub fn key_value(&self) -> u64 {
        u64::from_str_radix(&self.key, 16).unwrap_or(0)
    }

    /// The content key as a number, when the entry is found by its content ([`CONTENT_PREFIX`]).
    #[must_use]
    pub fn content_value(&self) -> Option<u64> {
        let digits = self.key.strip_prefix(CONTENT_PREFIX)?;
        (digits.len() == 16).then(|| u64::from_str_radix(digits, 16).ok()).flatten()
    }

    /// The file of the tile in column `x` and row `y`, relative to the index's folder.
    #[must_use]
    pub fn tile_file(&self, x: usize, y: usize) -> String {
        self.tiles.replace("{x}", &x.to_string()).replace("{y}", &y.to_string())
    }

    /// The tiles along the plane's edge.
    #[must_use]
    pub const fn tiles_across() -> usize {
        PLANE / TILE
    }

    /// The palette as colours, `None` if it is not 256 entries of six hexadecimal digits.
    #[must_use]
    pub fn colours(&self) -> Option<[[u8; 3]; 256]> {
        if self.palette.len() != 256 {
            return None;
        }
        let mut out = [[0_u8; 3]; 256];
        for (colour, text) in out.iter_mut().zip(&self.palette) {
            let value = u32::from_str_radix(text, 16).ok().filter(|_| text.len() == 6)?;
            *colour = [(value >> 16) as u8, (value >> 8) as u8, value as u8];
        }
        Some(out)
    }
}

impl Index {
    /// The index of `<pack>/art/mode7/mode7.toml`; empty when the pack has none.
    ///
    /// # Errors
    ///
    /// The file is not a valid index.
    pub fn open(pack: &Path) -> Result<Self, Mode7Error> {
        let Ok(text) = std::fs::read_to_string(pack.join(INDEX)) else {
            return Ok(Self::default());
        };
        toml::from_str(&text).map_err(|e| Mode7Error::Read(e.to_string()))
    }

    /// The plane whose content is `key`, if the index has art for it.
    #[must_use]
    pub fn find(&self, key: u64) -> Option<usize> {
        self.plane
            .iter()
            .position(|entry| entry.content_value().is_none() && entry.key_value() == key)
    }

    /// The plane to draw for the VRAM `vram` and the colours `cgram`: the art found by its [`content_key`] (the last entry
    /// of it: a picture of the art override comes after the pack's own), else the pack's art for its [`plane_key`] (what
    /// was found before), else the pack's art of the plane key [`CONTENT_ALIASES`] gives its content. Art found by the
    /// content is drawn only in the colours it was made in ([`colours_agree`] at least [`CONTENT_COLOURS_AGREE`]): the
    /// game shows one content in other colours too (Super Saiyan Goku is the normal Goku's plane with yellow hair), and
    /// the art of the one is not the picture of the other; those keep the game's pixels.
    #[must_use]
    pub fn find_plane(&self, vram: &[u16], cgram: &[u16]) -> Option<usize> {
        self.find_plane_with(vram, cgram, CONTENT_ALIASES)
    }

    /// [`Self::find_plane`] with the aliases `aliases` (`(content, plane key, whose)`).
    #[must_use]
    pub fn find_plane_with(
        &self,
        vram: &[u16],
        cgram: &[u16],
        aliases: &[(u64, u64, &str)],
    ) -> Option<usize> {
        let content = content_key(vram);
        let agrees = |at: usize| {
            self.plane[at].colours().is_some_and(|palette| {
                colours_agree(vram, cgram, &palette) >= CONTENT_COLOURS_AGREE
            })
        };
        (0..self.plane.len())
            .rev()
            .find(|&at| self.plane[at].content_value() == Some(content) && agrees(at))
            .or_else(|| self.find(plane_key(vram)))
            .or_else(|| {
                let &(_, key, _) = aliases.iter().find(|&&(alias, _, _)| alias == content)?;
                self.find(key).filter(|&at| agrees(at))
            })
    }
}

/// The least [`colours_agree`] (in thousandths) with which art found by a plane's content is drawn: all its colours, so
/// that such art is never drawn recoloured (`bgart::adapt`). In 29 replays the planes found by their content agreed
/// 1000 in every frame, but Super Saiyan Goku's (732: the normal Goku's art, his hair yellow in the game).
pub const CONTENT_COLOURS_AGREE: u32 = 1000;

/// Of the plane's pixels of a colour other than 0 (the 64 of each character the map uses, once for each map entry that
/// uses it), the share in thousandths whose colour in `cgram` is the art's colour in `palette` within 3 on each channel
/// (as `bgart::adapt` draws the art unchanged; the 15-bit colour made 8-bit as the art's palettes are); 1000 when the
/// plane has no such pixel.
#[must_use]
pub fn colours_agree(vram: &[u16], cgram: &[u16], palette: &[[u8; 3]; 256]) -> u32 {
    let words = &vram[..PLANE_WORDS.min(vram.len())];
    let mut uses = [0_u64; 256];
    for &word in words {
        uses[usize::from(word as u8)] += 1;
    }
    let expand = |value: u16| {
        let channel = |shift: u16| ((u32::from(value >> shift) & 31) * 255 + 15) / 31;
        [channel(0) as u8, channel(5) as u8, channel(10) as u8]
    };
    let (mut all, mut same) = (0_u64, 0_u64);
    for (character, &count) in uses.iter().enumerate().filter(|&(_, &count)| count > 0) {
        for at in 64 * character..64 * character + 64 {
            let index = usize::from(words.get(at).map_or(0, |&word| (word >> 8) as u8));
            if index == 0 {
                continue;
            }
            all += count;
            let current = cgram.get(index).map_or([0; 3], |&word| expand(word));
            if (0..3).all(|c| current[c].abs_diff(palette[index][c]) <= 3) {
                same += count;
            }
        }
    }
    (same * 1000).checked_div(all).map_or(1000, |share| share as u32)
}

/// The start of the key of a plane found by its content ([`Entry::content_value`]).
pub const CONTENT_PREFIX: &str = "content:";

/// The planes of the pack's art (`mode7.toml`, by their [`plane_key`]) that the game shows with other characters left in
/// VRAM, by what they show ([`content_key`]): `(content, plane key, whose)`. The flights at the start of a battle (game
/// mode `$0C`) load the flier's plane over the characters the battle before left, so the pack's art of a flier was found
/// in few of the battles (8 of the 90 plane keys of the battles' flights in 29 replays). Each content was seen in the
/// replays; the plane key is the pack's art of that flier, one the replays showed with that content (Goku, Frieza,
/// Piccolo) or, for the fliers the replays never showed with a key of the pack, the
/// pack's art of the same flier, compared picture by picture (Super Vegeta, Gohan, Burter). A flier the pack has no art
/// for (Krillin, the green fighter of the false Nail's battle) is not here, and keeps the game's pixels.
pub const CONTENT_ALIASES: &[(u64, u64, &str)] = &[
    (0xcd83_cbee_861e_14ad, 0xb96e_cb15_72d8_8908, "Goku"),
    (0x8542_d962_a902_04f5, 0x1f84_2a37_f703_d3fe, "Frieza"),
    (0xeaab_c317_4d0f_006d, 0xddf5_23c3_0fae_b0ea, "Piccolo"),
    (0x1239_827f_e73e_508f, 0x08f4_3fc7_851f_0e36, "Super Vegeta"),
    (0xa60b_9c3e_c679_e0b6, 0xcbf4_5e18_cae6_93c4, "Gohan"),
    (0xc549_2591_4f96_5e7a, 0xb188_4f01_fa04_5ab2, "Burter"),
];

/// The key of a plane: a hash (FNV-1a, 64 bits) of the words of VRAM the plane is made of.
#[must_use]
pub fn plane_key(vram: &[u16]) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325_u64;
    for &word in &vram[..PLANE_WORDS.min(vram.len())] {
        for byte in word.to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    hash
}

/// The key of what a plane shows (FNV-1a, 64 bits): its tile map (the low bytes of the plane's words) and the eight-bit
/// characters the map uses (the high bytes of each used character's 64 words), each once, in the order of their numbers.
/// The characters no map entry uses are left out: the game loads the plane of a battle's opening flight (game mode
/// `$0C`) over what the battle before left in the other characters, so one plane had as many [`plane_key`]s as the
/// battles it was seen in, and the pack's art was found in a few of them only.
#[must_use]
pub fn content_key(vram: &[u16]) -> u64 {
    let words = &vram[..PLANE_WORDS.min(vram.len())];
    let mut hash = 0xCBF2_9CE4_8422_2325_u64;
    let mut eat = |byte: u8| hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3);
    let mut used = [false; 256];
    for &word in words {
        let character = word as u8;
        used[usize::from(character)] = true;
        eat(character);
    }
    for (character, _) in used.iter().enumerate().filter(|&(_, &used)| used) {
        for at in 64 * character..64 * character + 64 {
            eat(words.get(at).map_or(0, |&word| (word >> 8) as u8));
        }
    }
    hash
}

/// The art of a plane as a texture: RGBA (premultiplied inside), the art's size and each copy half
/// the one before.
#[derive(Debug)]
pub struct PlaneArt {
    levels: Vec<Level>,
    /// The colours the art was made in.
    pub palette: [[u8; 3]; 256],
}

/// A level of the texture: colours multiplied by their alpha (0 to 255 each), then the alpha, four
/// bytes a pixel.
#[derive(Debug)]
struct Level {
    side: usize,
    premultiplied: Vec<u8>,
}

impl PlaneArt {
    /// A texture of the `side` x `side` RGBA pixels `rgba` (a power of two), with its smaller copies.
    /// A pixel with alpha 0 is where the plane has no art (the game's own pixel shows).
    ///
    /// # Panics
    ///
    /// `side` is not a power of two, or `rgba` is not `side * side * 4` bytes.
    #[must_use]
    pub fn new(side: usize, rgba: &[u8], palette: [[u8; 3]; 256]) -> Self {
        assert!(side.is_power_of_two() && rgba.len() == side * side * 4);
        let mut base = vec![0_u8; rgba.len()];
        for (to, from) in base.as_chunks_mut::<4>().0.iter_mut().zip(rgba.as_chunks::<4>().0) {
            let alpha = u32::from(from[3]);
            for c in 0..3 {
                to[c] = ((u32::from(from[c]) * alpha + 127) / 255) as u8;
            }
            to[3] = from[3];
        }
        let mut levels = vec![Level { side, premultiplied: base }];
        while levels.last().is_some_and(|last| last.side > 1) && levels.len() < 6 {
            let last = levels.last().expect("a level");
            let half = last.side / 2;
            let mut next = vec![0_u8; half * half * 4];
            for y in 0..half {
                for x in 0..half {
                    for c in 0..4 {
                        let at = |dx: usize, dy: usize| {
                            u32::from(
                                last.premultiplied[((2 * y + dy) * last.side + 2 * x + dx) * 4 + c],
                            )
                        };
                        next[(y * half + x) * 4 + c] =
                            ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
                    }
                }
            }
            levels.push(Level { side: half, premultiplied: next });
        }
        Self { levels, palette }
    }

    /// The number of levels (1 is the art alone).
    #[must_use]
    pub fn levels(&self) -> usize {
        self.levels.len()
    }

    /// The colour and alpha at the plane position `(x, y)` in 1/256 of a game pixel, wrapped around
    /// the plane, for a footprint of `step` art pixels per picture pixel in 1/256 (256 is one: the
    /// art's own size; 512 two, and so on). Bilinear in the level the footprint says, blended with
    /// the next smaller one; the colour is not mixed with the colour of a transparent texel.
    #[must_use]
    pub fn sample(&self, x: i64, y: i64, step: i64) -> [u8; 4] {
        // The level (in 1/256) the footprint is at: log2 of the step, in steps of 1/256
        let step = step.max(256);
        let whole = (63 - i64::from((step as u64).leading_zeros())) - 8;
        let level = (whole.max(0) as usize).min(self.levels.len() - 1);
        let base = 256_i64 << level.min(40);
        // The fraction between this level and the next, by the step's place between the two powers
        let fraction = if level + 1 >= self.levels.len() {
            0
        } else {
            (((step - base) * 256) / base).clamp(0, 255)
        };
        let mut sum = self.bilinear(level, x, y);
        if fraction != 0 {
            let far = self.bilinear(level + 1, x, y);
            for c in 0..4 {
                sum[c] = (sum[c] * (256 - fraction) + far[c] * fraction + 128) >> 8;
            }
        }
        let alpha = sum[3];
        if alpha == 0 {
            return [0; 4];
        }
        let un = |v: i64| ((v * 255 + alpha / 2) / alpha).clamp(0, 255) as u8;
        [un(sum[0]), un(sum[1]), un(sum[2]), alpha.clamp(0, 255) as u8]
    }

    /// Bilinear sample of the premultiplied level `level` at the plane position `(x, y)` (1/256
    /// game pixel): the four channels, 0 to 255 each.
    fn bilinear(&self, level: usize, px: i64, py: i64) -> [i64; 4] {
        let lv = &self.levels[level];
        let side = lv.side as i64;
        // Texel centres lie half a texel in; the level has `side` texels across the plane, so a
        // position in 1/256 game pixel is `px * side / PLANE` in 1/256 texel
        let tu = (px * side).div_euclid(PLANE as i64) - 128;
        let tv = (py * side).div_euclid(PLANE as i64) - 128;
        let (u0, v0) = (tu.div_euclid(256), tv.div_euclid(256));
        let (fu, fv) = (tu.rem_euclid(256), tv.rem_euclid(256));
        let at = |tx: i64, ty: i64| {
            let (tx, ty) = (tx.rem_euclid(side) as usize, ty.rem_euclid(side) as usize);
            let o = (ty * lv.side + tx) * 4;
            [
                i64::from(lv.premultiplied[o]),
                i64::from(lv.premultiplied[o + 1]),
                i64::from(lv.premultiplied[o + 2]),
                i64::from(lv.premultiplied[o + 3]),
            ]
        };
        let (p00, p10, p01, p11) = (at(u0, v0), at(u0 + 1, v0), at(u0, v0 + 1), at(u0 + 1, v0 + 1));
        let mut out = [0_i64; 4];
        for ch in 0..4 {
            let top = p00[ch] * (256 - fu) + p10[ch] * fu;
            let bottom = p01[ch] * (256 - fu) + p11[ch] * fu;
            out[ch] = (top * (256 - fv) + bottom * fv + 32768) >> 16;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(side: usize, colour: [u8; 3]) -> PlaneArt {
        let rgba: Vec<u8> = [colour[0], colour[1], colour[2], 255]
            .iter()
            .copied()
            .cycle()
            .take(side * side * 4)
            .collect();
        PlaneArt::new(side, &rgba, [[0; 3]; 256])
    }

    #[test]
    fn a_flat_texture_samples_flat_at_any_place_and_footprint() {
        let art = flat(64, [10, 200, 30]);
        for (x, y, step) in [(0, 0, 256), (12345, 999, 256), (-5000, 70000, 700), (3, 3, 5000)] {
            assert_eq!(art.sample(x, y, step), [10, 200, 30, 255]);
        }
    }

    #[test]
    fn the_texture_wraps_around_the_plane() {
        let side = 8;
        let mut rgba = vec![255_u8; side * side * 4];
        for (n, v) in rgba.iter_mut().enumerate().filter(|(n, _)| n % 4 != 3) {
            *v = (n % 251) as u8;
        }
        let art = PlaneArt::new(side, &rgba, [[0; 3]; 256]);
        let at = |x: i64, y: i64| art.sample(x, y, 256);
        let plane = (PLANE as i64) * 256;
        assert_eq!(at(1000, 2000), at(1000 + plane, 2000 - plane));
    }

    #[test]
    fn a_texel_centre_gives_the_texel_and_halfway_the_mean() {
        // 2 x 2 texels over the plane: centres at a quarter and three quarters
        let rgba = vec![0, 0, 0, 255, 200, 0, 0, 255, 0, 100, 0, 255, 0, 0, 40, 255];
        let art = PlaneArt::new(2, &rgba, [[0; 3]; 256]);
        let quarter = (PLANE as i64) * 256 / 4;
        assert_eq!(art.sample(quarter, quarter, 256), [0, 0, 0, 255]);
        assert_eq!(art.sample(3 * quarter, quarter, 256), [200, 0, 0, 255]);
        assert_eq!(art.sample(2 * quarter, quarter, 256), [100, 0, 0, 255]);
        assert_eq!(art.levels(), 2);
    }

    #[test]
    fn a_far_plane_is_read_from_a_smaller_copy() {
        // a one-pixel checkerboard: at a footprint of two texels or more it is the mean grey
        let side = 16;
        let mut rgba = vec![255_u8; side * side * 4];
        for y in 0..side {
            for x in 0..side {
                let v = if (x + y) % 2 == 0 { 255 } else { 0 };
                rgba[(y * side + x) * 4..][..3].fill(v);
            }
        }
        let art = PlaneArt::new(side, &rgba, [[0; 3]; 256]);
        let c = art.sample(40_000, 52_000, 512);
        assert!((120..=136).contains(&c[0]), "{c:?}");
    }

    #[test]
    fn a_transparent_texel_neither_tints_nor_hides_the_art_beside_it() {
        // 2 x 2 texels: the left column has no art (its colour is junk), the right is solid
        let rgba = vec![250, 0, 250, 0, 0, 90, 0, 255, 250, 0, 250, 0, 0, 90, 0, 255];
        let art = PlaneArt::new(2, &rgba, [[0; 3]; 256]);
        let quarter = (PLANE as i64) * 256 / 4;
        assert_eq!(art.sample(quarter, quarter, 256), [0, 0, 0, 0]);
        assert_eq!(art.sample(3 * quarter, quarter, 256), [0, 90, 0, 255]);
        // halfway between them: half the alpha, the colour of the art alone
        assert_eq!(art.sample(2 * quarter, quarter, 256), [0, 90, 0, 128]);
    }

    #[test]
    fn the_key_follows_the_plane_words_only() {
        let mut vram = vec![0_u16; 0x8000];
        let key = plane_key(&vram);
        vram[0x4000] = 7;
        assert_eq!(plane_key(&vram), key);
        vram[0x3FFF] = 1;
        assert_ne!(plane_key(&vram), key);
    }

    #[test]
    fn the_content_key_follows_the_map_and_the_characters_it_uses_only() {
        // The map uses the characters 0 and 1 (the high bytes of words 0 to 127): the character 2 is not used
        let mut vram = vec![0_u16; 0x8000];
        vram[5] = 1;
        let (content, key) = (content_key(&vram), plane_key(&vram));
        vram[128] = 7 << 8;
        assert_ne!(plane_key(&vram), key, "the plane key follows every word");
        assert_eq!(content_key(&vram), content, "a character no map entry uses is left out");
        vram[0x4000] = 3;
        assert_eq!(content_key(&vram), content, "nor a word past the plane");
        vram[64 + 9] = 4 << 8;
        assert_ne!(content_key(&vram), content, "a used character's pixel changes it");
        vram[64 + 9] = 0;
        vram[6] = 1;
        assert_ne!(content_key(&vram), content, "and the map");
    }

    #[test]
    fn a_plane_is_found_by_its_content_key_then_its_plane_key_then_an_alias() {
        let entry = |id: &str, key: String| Entry {
            id: id.to_owned(),
            key,
            tiles: format!("{id}/t{{x}}_{{y}}.png"),
            palette: vec!["000000".to_owned(); 256],
            replace: false,
        };
        let mut vram = vec![0_u16; 0x4000];
        let (key, content) = (plane_key(&vram), content_key(&vram));
        vram[128] = 1 << 8;
        let other = vram.clone();
        vram[128] = 0;
        let mut index = Index {
            plane: vec![
                entry("own", format!("{key:016x}")),
                entry("seen", format!("{:016x}", 0x1234)),
            ],
        };
        // By the plane key only, as before
        assert_eq!(index.find_plane_with(&vram, &[], &[]), Some(0));
        assert_eq!(index.find_plane_with(&other, &[], &[]), None);
        // An alias names the art of `seen` for the content: found for the VRAM of another plane key, after the plane key
        let aliases = [(content, 0x1234, "test")];
        assert_eq!(index.find_plane_with(&other, &[], &aliases), Some(1));
        assert_eq!(index.find_plane_with(&vram, &[], &aliases), Some(0));
        assert_eq!(index.find_plane_with(&other, &[], &[(content, 0x9999, "absent")]), None);
        // Art keyed by the content (a picture of the art override, the last of them) comes first, for both
        index.plane.push(entry("by content", format!("{CONTENT_PREFIX}{content:016x}")));
        index.plane.push(entry("override", format!("{CONTENT_PREFIX}{content:016x}")));
        assert_eq!(index.plane[3].content_value(), Some(content));
        assert_eq!(index.find_plane_with(&vram, &[], &aliases), Some(3));
        assert_eq!(index.find_plane_with(&other, &[], &aliases), Some(3));
        // Its key is no plane key, and a key of other digits is no content key
        assert_eq!(index.find(0), None);
        assert_eq!(entry("x", format!("{CONTENT_PREFIX}12")).content_value(), None);
        assert_eq!(index.plane[0].content_value(), None);
    }

    #[test]
    fn art_found_by_its_content_is_drawn_in_the_colours_it_was_made_in_only() {
        // Every map entry the character 1, whose pixels are the colour 5 (left half) and 6 (right half)
        let mut vram = vec![1_u16; 0x4000];
        for pixel in 0..64 {
            vram[64 + pixel] |= if pixel % 8 < 4 { 5 << 8 } else { 6 << 8 };
        }
        let content = content_key(&vram);
        // The art was made in red (5) and blue (6)
        let mut palette = vec!["000000".to_owned(); 256];
        palette[5] = "FF0000".to_owned();
        palette[6] = "0000FF".to_owned();
        let entry = |id: &str, key: String| Entry {
            id: id.to_owned(),
            key,
            tiles: format!("{id}/t{{x}}_{{y}}.png"),
            palette: palette.clone(),
            replace: false,
        };
        let colours = entry("x", String::new()).colours().unwrap();
        let (mut cgram, aliases) = (vec![0_u16; 256], [(content, 0x1234, "test")]);
        cgram[5] = 0x001F;
        cgram[6] = 0x7C00;
        assert_eq!(colours_agree(&vram, &cgram, &colours), 1000);
        // The game shows the colour 6 in yellow: half the pixels are not the art's
        let mut yellow = cgram.clone();
        yellow[6] = 0x03FF;
        assert_eq!(colours_agree(&vram, &yellow, &colours), 500);
        // By an alias, and by the content: drawn in the art's colours only
        let mut index = Index { plane: vec![entry("seen", format!("{:016x}", 0x1234))] };
        assert_eq!(index.find_plane_with(&vram, &cgram, &aliases), Some(0));
        assert_eq!(index.find_plane_with(&vram, &yellow, &aliases), None);
        index.plane.push(entry("by content", format!("{CONTENT_PREFIX}{content:016x}")));
        assert_eq!(index.find_plane_with(&vram, &cgram, &aliases), Some(1));
        assert_eq!(index.find_plane_with(&vram, &yellow, &aliases), None);
        // The art of the plane key is found in any colours, as before
        index.plane.push(entry("own", format!("{:016x}", plane_key(&vram))));
        assert_eq!(index.find_plane_with(&vram, &yellow, &aliases), Some(2));
    }

    #[test]
    fn a_palette_is_256_six_digit_colours() {
        let mut entry = Entry {
            id: "p".to_owned(),
            key: "00000000000000ff".to_owned(),
            tiles: "p/t{x}_{y}.png".to_owned(),
            palette: vec!["102030".to_owned(); 256],
            replace: false,
        };
        assert_eq!(entry.key_value(), 255);
        assert_eq!(entry.colours().unwrap()[5], [0x10, 0x20, 0x30]);
        assert_eq!(entry.tile_file(3, 2), "p/t3_2.png");
        entry.palette[9] = "12".to_owned();
        assert!(entry.colours().is_none());
        entry.palette.pop();
        assert!(entry.colours().is_none());
    }

    #[test]
    fn the_index_round_trips_and_a_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Index::open(dir.path()).unwrap(), Index::default());
        let index = Index {
            plane: vec![Entry {
                id: "plane".to_owned(),
                key: "00000000000000ab".to_owned(),
                tiles: "plane/t{x}_{y}.png".to_owned(),
                palette: vec!["000000".to_owned(); 256],
                replace: false,
            }],
        };
        std::fs::create_dir_all(dir.path().join("art/mode7")).unwrap();
        std::fs::write(dir.path().join(INDEX), toml::to_string(&index).unwrap()).unwrap();
        assert_eq!(Index::open(dir.path()).unwrap(), index);
        assert_eq!(index.find(0xAB), Some(0));
        assert_eq!(index.find(1), None);
    }
}
