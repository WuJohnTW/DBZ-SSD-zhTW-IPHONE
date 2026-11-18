//! The art index of an HD pack: which approved pictures the renderer draws, and what
//! identifies each one on screen (`art.bin`, written by `xtask hd-import`).
//!
//! Two kinds of entry:
//!
//! * a **picture** is the art of a 32 x 32 portrait or face (16 characters of an asset of the
//!   pack, in the order of the picture's 4 x 4 cells). The renderer finds it through the pack's
//!   tile binding: a character of the asset that is on screen draws its cell of the picture. Several
//!   pictures can cover the same characters (the same drawing in different palettes), and the
//!   palette of the pixel chooses between them;
//! * an **object** is the art of one assembled sprite pose, found by the key of its pixels (the
//!   palette-index array of the pose, or for an effect its colours): the sprites of a frame are
//!   grouped as the capture of the art was, and a group whose key is in the index draws the art.
//!
//! A pose also holds the silhouette of the pose it was made from (a bit for each opaque pixel), so
//! a pose of a frame that is only part of it (a fade-in, a sprite cut by the screen's edge) can be
//! found by the silhouette that holds it.
//!
//! The index holds names, keys, palettes and silhouettes; the art is in the PNG files it names,
//! relative to the pack directory.

use crate::pack::PackError;
use rustc_hash::FxHashMap;
use std::path::Path;

const MAGIC: &[u8; 8] = b"SSDHDART";
/// The version of the file.
pub const ART_FORMAT: u32 = 1;

/// The file name of the art index in a pack directory.
pub const FILE: &str = "art.bin";
/// The end of the id of a pose added by the data folder's `art-override\super-vegeta\` (patch 0049, see
/// `Pack::with_art_override`): a picture of Super Vegeta for the pixels of a pose only other art exists for (Vegeta's),
/// drawn only in Super Vegeta's colours.
pub const SUPER_VEGETA_ONLY: &str = "@super-vegeta";
/// The end of the id of a pose added by the data folder's `art-override\goku\` (patch 0051, see
/// `Pack::with_art_override`): a picture of the normal Goku (black hair) for the pixels of a pose only Super Saiyan art
/// exists for, drawn only in colours whose hair is not the Super Saiyan's.
pub const NORMAL_GOKU_ONLY: &str = "@goku";
/// The end of the id of a pose added by the data folder's `art-override\ending-goku\` (see `Pack::with_art_override`): a
/// picture of Goku in the ending's flight with Gohan (his torn clothes) for the pixels of a pose of his art, drawn in place
/// of that art only while the flight is on screen (`Conditions::of_work_ram`), and found by nothing else.
pub const ENDING_GOKU_ONLY: &str = "@ending-goku";

/// Up to 16 colours of a palette, and which of them the art's source uses (bit `i` for index `i`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Palette {
    pub mask: u16,
    pub colors: [[u8; 3]; 16],
}

impl Palette {
    /// How far the colours the source used are from the colours in `current` (a 16-colour row as
    /// RGB): the summed channel differences. 0 when they are the same, and 0 for a palette of no
    /// colour.
    #[must_use]
    pub fn distance(&self, current: &[[u8; 3]; 16]) -> u32 {
        let mut total = 0;
        for (index, (mine, theirs)) in self.colors.iter().zip(current).enumerate().skip(1) {
            if self.mask & (1 << index) != 0 {
                total +=
                    mine.iter().zip(theirs).map(|(a, b)| u32::from(a.abs_diff(*b))).sum::<u32>();
            }
        }
        total
    }
}

/// Objects of the shared pack (`hd-pack.zip`, revision 1) whose art is of another character than the pixels they
/// are found by, as `(id, key)`: left out when the index is opened. `battle-zarbon-801` has the key of the top 32 x 16
/// of Vegeta's face (the second picture of the stream `16:8A9B`, portraits `0F` and `50`) and Zarbon's face for art:
/// where that half of the face is drawn as sprites of a fighter's palette, a scene drew Zarbon's face over Vegeta's
/// (his retreat at the end of the Saiyan chapter). `fgfx-695459fedb` has the key of the whole of Captain Ginyu's face
/// (the second picture of the stream `16:B350`, portrait `31`, purple with a green scouter) and `battle-goku-802` the
/// key of its top 32 x 16, both with a blond face with a blue scouter for art: the Ginyu squad's scene where Goku
/// lands on Namek drew it beside Ginyu's lines (`str2:0x0C5` page 2, `str2:0x0C8`), the whole face, or its top half
/// over Ginyu's chin once the whole one is gone. Without them, those sprites show the picture of the face. The two
/// 15 x 16 poses of Frieza's first form walking on the field (white and purple, his horns: in the scene where he
/// comes for the wish of Porunga, and the cave where he is fought) have Captain Ginyu for art as
/// `field-frieza-force-004` and `-005`, and Nail's head as `battle-nail-803` and `-804`, the same keys: the field drew
/// Ginyu standing there and Nail once the party touched him. Without them, the field shows the game's own sprite of
/// Frieza (the pack has no art of him that size).
const WRONG_OBJECTS: &[(&str, u64)] = &[
    ("battle-zarbon-801", 0xB7D6_B360_FC00_302F),
    ("fgfx-695459fedb", 0x6954_59FE_DB6C_ECFD),
    ("battle-goku-802", 0xE688_DCEC_697E_F299),
    ("field-frieza-force-004", 0x3BB2_D6BF_3C99_F517),
    ("field-frieza-force-005", 0xAC77_2095_857A_6244),
    ("battle-nail-803", 0x3BB2_D6BF_3C99_F517),
    ("battle-nail-804", 0xAC77_2095_857A_6244),
];

/// Pictures of the shared pack whose art is not the picture the game draws, as `(picture, object, key, bytes)`: the
/// picture is drawn from the art of the object of the same 32 x 32 pixels (`key`, the key of the whole picture), while
/// its own art file is the one of `bytes` (the pack's file the fix was made for: a redrawn picture keeps its art). The
/// art of `face-8A9B-1` in revision 1 of the pack is a close-up of Vegeta's eyes, where the game draws his whole face;
/// `field-fg-5a85cef13b` is the art made for the same pixels as sprites of the field, in the same colours. Revision 2
/// (0.3.9) draws his whole face there.
const PICTURE_ART_FROM: &[(&str, &str, u64, u64)] =
    &[("face-8A9B-1", "field-fg-5a85cef13b", 0x5A85_CEF1_3BD2_0F5A, 15_532)];

/// Entries of the shared pack whose art is another character than the one the game draws there, drawn from the art
/// of the pack's picture of that character, as `(entry, key, picture)`: an object by its `id` and `key` (`Some`), or a
/// picture by its name (`None`: a picture has no key), and the name of the picture whose art it takes. An object
/// takes it only at 32 x 32, the size of a picture. Nail's face (portrait `22`, the first picture of the stream
/// `16:8E09`: bald, the antennae) has Piccolo's face with his turban for art, both as the object of its pixels
/// (`field-fg-fb9d731520`, his lines at the Guru's house and his portrait in the battle against Frieza) and as the
/// picture (`face-8E09-0`, drawn once the object is gone); `face-8E09-1`, the second picture of the stream, is Nail.
const ART_FROM_PICTURE: &[(&str, Option<u64>, &str)] = &[
    ("field-fg-fb9d731520", Some(0xFB9D_7315_2067_CC38), "face-8E09-1"),
    ("face-8E09-0", None, "face-8E09-1"),
];

/// The art of a 32 x 32 picture of an asset of the pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// The name of the art (its file's stem), for reports. It names the art's source, not the
    /// object that shows it: several objects share a set, and a name says nothing about which
    /// half or palette an object uses (the HD menus find the art by the picture's characters,
    /// `Pack::menu_picture`).
    pub name: String,
    /// The logical id of the asset whose characters the picture covers.
    pub asset: String,
    /// The character of the asset in each cell of the 4 x 4 picture, row by row.
    pub tiles: [u16; 16],
    pub palette: Palette,
    /// The art file, relative to the pack directory: RGBA, 32 art pixels to a character.
    pub art: String,
}

/// The art of one sprite pose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    /// The key of the pose's pixels (see `poses`).
    pub key: u64,
    /// The name of the art (its file's stem), for reports.
    pub id: String,
    /// The art is a greyscale shape that takes its colours from the original pixels.
    pub luma: bool,
    pub palette: Palette,
    /// The pose in game pixels.
    pub width: u16,
    pub height: u16,
    /// The silhouette of the pose: a bit for each pixel of its `width` x `height` box, row by row,
    /// the first pixel in the low bit of the first byte (set where the pose is opaque). Empty when
    /// the source did not say.
    pub mask: Vec<u8>,
    /// The art file, relative to the pack directory: RGBA, four art pixels to a game pixel.
    pub art: String,
}

impl Object {
    /// Whether the art is a sprite group of the field (class 3, whose ids start with `field-`):
    /// found among the field's groups, never among a fighter's or an effect's.
    #[must_use]
    pub fn field(&self) -> bool {
        self.id.starts_with("field-")
    }

    /// Whether the pose was added by `art-override\super-vegeta\` ([`SUPER_VEGETA_ONLY`]): drawn only in Super
    /// Vegeta's colours, never found by its shape or as a piece of another pose.
    #[must_use]
    pub fn super_vegeta_only(&self) -> bool {
        self.id.ends_with(SUPER_VEGETA_ONLY)
    }

    /// Whether the pose was added by `art-override\goku\` ([`NORMAL_GOKU_ONLY`]): drawn only in colours whose hair is not
    /// Super Saiyan Goku's, never found by its shape or as a piece of another pose.
    #[must_use]
    pub fn normal_goku_only(&self) -> bool {
        self.id.ends_with(NORMAL_GOKU_ONLY)
    }

    /// Whether the pose was added by `art-override\ending-goku\` ([`ENDING_GOKU_ONLY`]): drawn only in place of the art of
    /// its name while the ending's flight is on screen, never found by its key, its shape or as a piece of another pose.
    #[must_use]
    pub fn ending_goku_only(&self) -> bool {
        self.id.ends_with(ENDING_GOKU_ONLY)
    }

    /// Whether the pose was added by a folder of `art-override\` for one set of colours only ([`Self::super_vegeta_only`],
    /// [`Self::normal_goku_only`]) or for the ending's flight only ([`Self::ending_goku_only`]): never found by its shape,
    /// its palette or as a piece of another pose.
    #[must_use]
    pub fn override_only(&self) -> bool {
        self.super_vegeta_only() || self.normal_goku_only() || self.ending_goku_only()
    }

    /// Whether pixel `(x, y)` of the pose's box is opaque in its silhouette.
    #[must_use]
    pub fn opaque_at(&self, x: usize, y: usize) -> bool {
        let at = y * usize::from(self.width) + x;
        self.mask.get(at / 8).is_some_and(|byte| byte >> (at % 8) & 1 != 0)
    }

    /// The number of opaque pixels of the silhouette.
    #[must_use]
    pub fn opaque(&self) -> u32 {
        self.mask.iter().map(|byte| byte.count_ones()).sum()
    }
}

/// The poses of one fighter palette (the colours its art was made for), the smallest first: what
/// a partial pose is looked for in.
#[derive(Debug, Clone)]
pub struct PaletteGroup {
    pub palette: Palette,
    /// Indexes of [`ArtIndex::objects`] with a silhouette, by the number of their opaque pixels.
    pub objects: Vec<u32>,
}

/// The art index of a pack.
#[derive(Debug, Clone, Default)]
pub struct ArtIndex {
    pictures: Vec<Picture>,
    objects: Vec<Object>,
    by_asset: FxHashMap<String, Vec<u32>>,
    by_key: FxHashMap<u64, Vec<u32>>,
    /// By the size and the number of opaque pixels of the silhouette, for the poses a palette
    /// changes the key of.
    by_shape: FxHashMap<(u16, u16, u32), Vec<u32>>,
    groups: Vec<PaletteGroup>,
}

impl ArtIndex {
    #[must_use]
    pub fn new(pictures: Vec<Picture>, objects: Vec<Object>) -> Self {
        let mut by_asset: FxHashMap<String, Vec<u32>> = FxHashMap::default();
        for (at, picture) in pictures.iter().enumerate() {
            by_asset.entry(picture.asset.clone()).or_default().push(at as u32);
        }
        let mut by_key: FxHashMap<u64, Vec<u32>> = FxHashMap::default();
        for (at, object) in objects.iter().enumerate() {
            by_key.entry(object.key).or_default().push(at as u32);
        }
        let mut by_shape: FxHashMap<(u16, u16, u32), Vec<u32>> = FxHashMap::default();
        // (Super Vegeta's own pictures of other art's poses, and the normal Goku's of Super Saiyan art's, are found by their
        // key only: a shape or a piece of another pose would draw them in colours that are not theirs)
        for (at, object) in objects.iter().enumerate() {
            if !object.mask.is_empty() && !object.override_only() {
                by_shape
                    .entry((object.width, object.height, object.opaque()))
                    .or_default()
                    .push(at as u32);
            }
        }
        // The poses with a silhouette and a palette, by palette: the colours of the used indexes
        let mut groups: Vec<PaletteGroup> = Vec::new();
        let mut order: Vec<(u32, u32)> = objects
            .iter()
            .enumerate()
            .filter(|(_, object)| {
                !object.mask.is_empty() && object.palette.mask != 0 && !object.override_only()
            })
            .map(|(at, object)| (object.opaque(), at as u32))
            .collect();
        order.sort_unstable();
        for (_, at) in order {
            let palette = objects[at as usize].palette;
            let same = |group: &PaletteGroup| {
                group.palette.mask == palette.mask
                    && (0..16).all(|i| {
                        palette.mask & (1 << i) == 0 || group.palette.colors[i] == palette.colors[i]
                    })
            };
            match groups.iter_mut().find(|group| same(group)) {
                Some(group) => group.objects.push(at),
                None => groups.push(PaletteGroup { palette, objects: vec![at] }),
            }
        }
        Self { pictures, objects, by_asset, by_key, by_shape, groups }
    }

    /// The index with the entries of the shared pack drawn as another character fixed ([`WRONG_OBJECTS`],
    /// [`PICTURE_ART_FROM`], [`ART_FROM_PICTURE`]); [`ArtIndex::open`] gives it. An entry is fixed only where its id
    /// and key are the ones listed, so a pack whose art was redrawn under another key is drawn as it is.
    #[must_use]
    pub fn with_pack_fixes(self) -> Self {
        self.with_pack_fixes_where(|_, _| true)
    }

    /// [`ArtIndex::with_pack_fixes`], a picture of [`PICTURE_ART_FROM`] fixed only where `wrong_art` says its art (the
    /// path in the pack, and the bytes listed) is the file the fix was made for.
    #[must_use]
    pub fn with_pack_fixes_where(self, wrong_art: impl Fn(&str, u64) -> bool) -> Self {
        let Self { mut pictures, mut objects, .. } = self;
        objects.retain(|object| {
            !WRONG_OBJECTS.iter().any(|&(id, key)| object.id == id && object.key == key)
        });
        for &(name, id, key, bytes) in PICTURE_ART_FROM {
            let Some(art) = objects
                .iter()
                .find(|object| {
                    object.id == id
                        && object.key == key
                        && (object.width, object.height) == (32, 32)
                })
                .map(|object| object.art.clone())
            else {
                continue;
            };
            for picture in pictures
                .iter_mut()
                .filter(|picture| picture.name == name && wrong_art(&picture.art, bytes))
            {
                picture.art.clone_from(&art);
            }
        }
        // (the art of the pack's own pictures, so a listed picture is never taken from one fixed here)
        let picture_art = |name: &str, pictures: &[Picture]| {
            pictures.iter().find(|picture| picture.name == name).map(|picture| picture.art.clone())
        };
        let own: Vec<Option<String>> =
            ART_FROM_PICTURE.iter().map(|&(_, _, from)| picture_art(from, &pictures)).collect();
        for (&(entry, key, _), art) in ART_FROM_PICTURE.iter().zip(own) {
            let Some(art) = art else { continue };
            match key {
                Some(key) => {
                    for object in objects.iter_mut().filter(|object| {
                        object.id == entry
                            && object.key == key
                            && (object.width, object.height) == (32, 32)
                    }) {
                        object.art.clone_from(&art);
                    }
                }
                None => {
                    for picture in pictures.iter_mut().filter(|picture| picture.name == entry) {
                        picture.art.clone_from(&art);
                    }
                }
            }
        }
        Self::new(pictures, objects)
    }

    #[must_use]
    pub fn pictures(&self) -> &[Picture] {
        &self.pictures
    }

    #[must_use]
    pub fn objects(&self) -> &[Object] {
        &self.objects
    }

    /// The pictures that cover characters of the asset `id`, as indexes of [`ArtIndex::pictures`].
    #[must_use]
    pub fn pictures_of(&self, id: &str) -> &[u32] {
        self.by_asset.get(id).map_or(&[], Vec::as_slice)
    }

    /// The fighters' poses with a silhouette, by palette.
    #[must_use]
    pub fn palette_groups(&self) -> &[PaletteGroup] {
        &self.groups
    }

    /// The objects whose silhouette is `width` x `height` with `opaque` opaque pixels (several can
    /// have another shape of that size and count: the caller compares the silhouettes).
    #[must_use]
    pub fn objects_shaped(&self, width: u16, height: u16, opaque: u32) -> &[u32] {
        self.by_shape.get(&(width, height, opaque)).map_or(&[], Vec::as_slice)
    }

    /// The objects with key `key`, as indexes of [`ArtIndex::objects`].
    #[must_use]
    pub fn objects_with(&self, key: u64) -> &[u32] {
        self.by_key.get(&key).map_or(&[], Vec::as_slice)
    }

    /// The art index read from `dir`; empty when the pack has none.
    ///
    /// # Errors
    ///
    /// The file exists and is damaged.
    pub fn open(dir: &Path) -> Result<Self, PackError> {
        match std::fs::read(dir.join(FILE)) {
            Ok(bytes) => Self::decode(&bytes).map(|index| {
                index.with_pack_fixes_where(|art, bytes| {
                    std::fs::metadata(dir.join(art)).is_ok_and(|file| file.len() == bytes)
                })
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => {
                Err(PackError::Read { path: dir.to_path_buf(), message: format!("{FILE}: {e}") })
            }
        }
    }

    /// The index as its file: magic, version, the pictures, the objects, and the CRC-32 of
    /// everything before it (little-endian).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&ART_FORMAT.to_le_bytes());
        out.extend_from_slice(&(self.pictures.len() as u32).to_le_bytes());
        for picture in &self.pictures {
            put_str(&mut out, &picture.name);
            put_str(&mut out, &picture.asset);
            for tile in picture.tiles {
                out.extend_from_slice(&tile.to_le_bytes());
            }
            put_palette(&mut out, &picture.palette);
            put_str(&mut out, &picture.art);
        }
        out.extend_from_slice(&(self.objects.len() as u32).to_le_bytes());
        for object in &self.objects {
            out.extend_from_slice(&object.key.to_le_bytes());
            put_str(&mut out, &object.id);
            out.push(u8::from(object.luma));
            put_palette(&mut out, &object.palette);
            out.extend_from_slice(&object.width.to_le_bytes());
            out.extend_from_slice(&object.height.to_le_bytes());
            out.extend_from_slice(&(object.mask.len() as u32).to_le_bytes());
            out.extend_from_slice(&object.mask);
            put_str(&mut out, &object.art);
        }
        let crc = ssd_rom::crc32(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        out
    }

    /// Reads an index written by [`ArtIndex::encode`].
    ///
    /// # Errors
    ///
    /// The bytes are not an art index of this version.
    pub fn decode(bytes: &[u8]) -> Result<Self, PackError> {
        let body_len = bytes.len().checked_sub(4).ok_or(PackError::Index("art: truncated"))?;
        let (body, crc) = bytes.split_at(body_len);
        if ssd_rom::crc32(body).to_le_bytes() != crc {
            return Err(PackError::Index("art: checksum"));
        }
        let mut reader = Reader { bytes: body, at: 0 };
        if reader.take(8)? != MAGIC {
            return Err(PackError::Index("not an art index"));
        }
        if reader.u32()? != ART_FORMAT {
            return Err(PackError::Index("art: another version"));
        }
        let mut pictures = Vec::new();
        for _ in 0..reader.u32()? {
            let name = reader.string()?;
            let asset = reader.string()?;
            let mut tiles = [0_u16; 16];
            for tile in &mut tiles {
                *tile = reader.u16()?;
            }
            let palette = reader.palette()?;
            let art = reader.string()?;
            pictures.push(Picture { name, asset, tiles, palette, art });
        }
        let mut objects = Vec::new();
        for _ in 0..reader.u32()? {
            let key = reader.u64()?;
            let id = reader.string()?;
            let luma = reader.u8()? != 0;
            let palette = reader.palette()?;
            let width = reader.u16()?;
            let height = reader.u16()?;
            let length = reader.u32()? as usize;
            let mask = reader.take(length)?.to_vec();
            let art = reader.string()?;
            objects.push(Object { key, id, luma, palette, width, height, mask, art });
        }
        if reader.at != body.len() {
            return Err(PackError::Index("art: trailing bytes"));
        }
        Ok(Self::new(pictures, objects))
    }
}

fn put_str(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u16).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn put_palette(out: &mut Vec<u8>, palette: &Palette) {
    out.extend_from_slice(&palette.mask.to_le_bytes());
    for color in palette.colors {
        out.extend_from_slice(&color);
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, len: usize) -> Result<&[u8], PackError> {
        let end = self.at.checked_add(len).ok_or(PackError::Index("art: truncated"))?;
        let bytes = self.bytes.get(self.at..end).ok_or(PackError::Index("art: truncated"))?;
        self.at = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, PackError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, PackError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("two bytes")))
    }

    fn u32(&mut self) -> Result<u32, PackError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four bytes")))
    }

    fn u64(&mut self) -> Result<u64, PackError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("eight bytes")))
    }

    fn string(&mut self) -> Result<String, PackError> {
        let len = usize::from(self.u16()?);
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| PackError::Index("art: a name"))
    }

    fn palette(&mut self) -> Result<Palette, PackError> {
        let mask = self.u16()?;
        let mut colors = [[0_u8; 3]; 16];
        for color in &mut colors {
            color.copy_from_slice(self.take(3)?);
        }
        Ok(Palette { mask, colors })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette(seed: u8) -> Palette {
        let mut colors = [[0_u8; 3]; 16];
        for (index, color) in colors.iter_mut().enumerate() {
            *color = [seed, index as u8, 3];
        }
        Palette { mask: 0b1010, colors }
    }

    fn index() -> ArtIndex {
        ArtIndex::new(
            vec![Picture {
                name: "obj-00".to_owned(),
                asset: "portrait/16:8000/4bpp".to_owned(),
                tiles: std::array::from_fn(|n| n as u16),
                palette: palette(1),
                art: "art/picture/a.png".to_owned(),
            }],
            vec![
                Object {
                    key: 0xAB,
                    id: "battle-x-000".to_owned(),
                    luma: false,
                    palette: palette(2),
                    width: 20,
                    height: 30,
                    mask: vec![0b0000_0101, 0xFF],
                    art: "art/object/a.png".to_owned(),
                },
                Object {
                    key: 0xAB,
                    id: "battle-y-000".to_owned(),
                    luma: true,
                    palette: Palette::default(),
                    width: 4,
                    height: 5,
                    mask: Vec::new(),
                    art: "art/object/b.png".to_owned(),
                },
            ],
        )
    }

    #[test]
    fn an_art_index_reads_back_as_written_and_a_damaged_one_is_refused() {
        let index = index();
        let bytes = index.encode();
        let back = ArtIndex::decode(&bytes).unwrap();
        assert_eq!(back.pictures(), index.pictures());
        assert_eq!(back.objects(), index.objects());
        assert_eq!(back.encode(), bytes);
        let mut damaged = bytes.clone();
        damaged[12] ^= 1;
        assert!(ArtIndex::decode(&damaged).is_err());
        assert!(ArtIndex::decode(&bytes[..9]).is_err());
    }

    #[test]
    fn a_silhouette_says_where_a_pose_is_opaque_and_the_poses_are_grouped_by_palette() {
        let index = index();
        let object = &index.objects()[0];
        assert!(object.opaque_at(0, 0) && !object.opaque_at(1, 0) && object.opaque_at(2, 0));
        assert!(object.opaque_at(8, 0), "the second byte is the next eight pixels");
        assert_eq!(object.opaque(), 2 + 8);
        assert!(!index.objects()[1].opaque_at(0, 0), "no silhouette: nothing is opaque");
        // Only the pose with a silhouette and a palette is in a group
        let groups = index.palette_groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].objects, [0]);
    }

    #[test]
    fn the_lookups_name_every_entry_of_an_asset_or_a_key() {
        let index = index();
        assert_eq!(index.pictures_of("portrait/16:8000/4bpp"), [0]);
        assert_eq!(index.pictures_of("portrait/16:9999/4bpp"), &[] as &[u32]);
        assert_eq!(index.objects_with(0xAB), [0, 1]);
        assert_eq!(index.objects_with(1), &[] as &[u32]);
    }

    /// The entries of the shared pack that draw another character, with an entry of the same id and another key and
    /// a picture of another name beside them.
    fn shared_pack() -> ArtIndex {
        let picture = |name: &str| Picture {
            name: name.to_owned(),
            asset: "portrait/16:8A9B/4bpp".to_owned(),
            tiles: std::array::from_fn(|n| n as u16),
            palette: palette(1),
            art: format!("art/picture/{name}.webp"),
        };
        let object = |id: &str, key: u64, size: u16| Object {
            key,
            id: id.to_owned(),
            luma: false,
            palette: palette(2),
            width: 32,
            height: size,
            mask: Vec::new(),
            art: format!("art/object/{id}.webp"),
        };
        ArtIndex::new(
            vec![picture("face-8A9B-1"), picture("obj-19")],
            vec![
                object("battle-zarbon-801", 0xB7D6_B360_FC00_302F, 16),
                object("battle-zarbon-801", 0x1234, 16),
                object("field-fg-5a85cef13b", 0x5A85_CEF1_3BD2_0F5A, 32),
            ],
        )
    }

    #[test]
    fn the_shared_packs_art_of_another_character_is_left_out_or_taken_from_the_same_pixels() {
        let fixed = shared_pack().with_pack_fixes();
        // Zarbon's eyes are no longer found by the pixels of the top of Vegeta's face; an entry of that id with
        // another key stays
        assert_eq!(fixed.objects_with(0xB7D6_B360_FC00_302F), &[] as &[u32]);
        assert_eq!(fixed.objects_with(0x1234).len(), 1);
        assert_eq!(fixed.objects().len(), 2);
        // Vegeta's face is drawn from the art of the field's sprites of the same pixels, not the close-up
        assert_eq!(fixed.pictures()[0].art, "art/object/field-fg-5a85cef13b.webp");
        assert_eq!(fixed.pictures()[1].art, "art/picture/obj-19.webp", "another picture as it was");
        // The lookups are made again over what is left
        let at = fixed.objects_with(0x5A85_CEF1_3BD2_0F5A)[0];
        assert_eq!(fixed.objects()[at as usize].id, "field-fg-5a85cef13b");
    }

    #[test]
    fn ginyus_face_is_not_found_as_the_blond_face_of_the_shared_pack() {
        // The object of the pixels of Ginyu's face (`16:B350`, the second picture) whose art is another face is left
        // out; an object of that id with another key, and another object of the face's size, stay
        let object = |id: &str, key: u64| Object {
            key,
            id: id.to_owned(),
            luma: false,
            palette: palette(2),
            width: 32,
            height: 32,
            mask: Vec::new(),
            art: format!("art/object/{id}.webp"),
        };
        let index = ArtIndex::new(
            Vec::new(),
            vec![
                object("fgfx-695459fedb", 0x6954_59FE_DB6C_ECFD),
                object("fgfx-695459fedb", 0x6954),
                object("battle-goku-802", 0xE688_DCEC_697E_F299),
                object("battle-goku-801", 0xE688),
                object("field-fg-499a647e4b", 0x499A_647E_4B80_548D),
            ],
        );
        assert_eq!(index.objects_with(0x6954_59FE_DB6C_ECFD).len(), 1);
        let fixed = index.with_pack_fixes();
        assert_eq!(fixed.objects_with(0x6954_59FE_DB6C_ECFD), &[] as &[u32], "the whole face");
        assert_eq!(fixed.objects_with(0xE688_DCEC_697E_F299), &[] as &[u32], "its top half");
        assert_eq!(fixed.objects_with(0x6954).len(), 1);
        assert_eq!(fixed.objects_with(0xE688).len(), 1);
        assert_eq!(fixed.objects_with(0x499A_647E_4B80_548D).len(), 1, "Goku's face as it was");
        assert_eq!(fixed.objects().len(), 3);
    }

    #[test]
    fn friezas_walk_on_the_field_is_found_as_neither_ginyu_nor_nail() {
        // The two keys of Frieza's first form walking, each with a field object of Ginyu and a battle object of Nail:
        // all four left out; Ginyu's field objects of other keys, and those ids with other keys, stay
        let object = |id: &str, key: u64| Object {
            key,
            id: id.to_owned(),
            luma: false,
            palette: palette(2),
            width: 15,
            height: 16,
            mask: Vec::new(),
            art: format!("art/object/{id}.webp"),
        };
        let index = ArtIndex::new(
            Vec::new(),
            vec![
                object("field-frieza-force-004", 0x3BB2_D6BF_3C99_F517),
                object("field-frieza-force-005", 0xAC77_2095_857A_6244),
                object("battle-nail-803", 0x3BB2_D6BF_3C99_F517),
                object("battle-nail-804", 0xAC77_2095_857A_6244),
                object("field-frieza-force-004", 0x3BB2),
                object("field-frieza-force-020", 0x1234),
            ],
        );
        assert_eq!(index.objects_with(0x3BB2_D6BF_3C99_F517).len(), 2);
        let fixed = index.with_pack_fixes();
        assert_eq!(fixed.objects_with(0x3BB2_D6BF_3C99_F517), &[] as &[u32]);
        assert_eq!(fixed.objects_with(0xAC77_2095_857A_6244), &[] as &[u32]);
        assert_eq!(fixed.objects_with(0x3BB2).len(), 1);
        assert_eq!(fixed.objects_with(0x1234).len(), 1);
        assert_eq!(fixed.objects().len(), 2);
    }

    #[test]
    fn nails_face_takes_the_art_of_the_packs_picture_of_nail() {
        // The object of the pixels of Nail's face and the picture of it are drawn from the art of `face-8E09-1`; an
        // object of that id with another key or another size, and another picture, keep theirs
        let picture = |name: &str| Picture {
            name: name.to_owned(),
            asset: "portrait/16:8E09/4bpp".to_owned(),
            tiles: std::array::from_fn(|n| n as u16),
            palette: palette(1),
            art: format!("art/picture/{name}.webp"),
        };
        let object = |id: &str, key: u64, height: u16| Object {
            key,
            id: id.to_owned(),
            luma: false,
            palette: palette(2),
            width: 32,
            height,
            mask: Vec::new(),
            art: format!("art/object/{id}.webp"),
        };
        let index = ArtIndex::new(
            vec![picture("face-8E09-0"), picture("face-8E09-1"), picture("face-8367-1")],
            vec![
                object("field-fg-fb9d731520", 0xFB9D_7315_2067_CC38, 32),
                object("field-fg-fb9d731520", 0xFB9D, 32),
                object("field-fg-fb9d731520", 0xFB9D_7315_2067_CC38, 16),
            ],
        );
        let fixed = index.with_pack_fixes();
        let nail = "art/picture/face-8E09-1.webp";
        assert_eq!(fixed.objects()[0].art, nail, "the object of Nail's face");
        assert_eq!(fixed.pictures()[0].art, nail, "the picture of Nail's face");
        assert_eq!(fixed.pictures()[1].art, nail, "Nail's own picture as it was");
        assert_eq!(fixed.objects()[1].art, "art/object/field-fg-fb9d731520.webp", "another key");
        assert_eq!(fixed.objects()[2].art, "art/object/field-fg-fb9d731520.webp", "another size");
        assert_eq!(fixed.pictures()[2].art, "art/picture/face-8367-1.webp", "another picture");
        // The lookups still find the object by the face's pixels
        assert_eq!(fixed.objects_with(0xFB9D_7315_2067_CC38).len(), 2);
    }

    #[test]
    fn an_entry_keeps_its_art_when_the_pack_has_no_picture_to_take_it_from() {
        let object = Object {
            key: 0xFB9D_7315_2067_CC38,
            id: "field-fg-fb9d731520".to_owned(),
            luma: false,
            palette: palette(2),
            width: 32,
            height: 32,
            mask: Vec::new(),
            art: "art/object/field-fg-fb9d731520.webp".to_owned(),
        };
        let fixed = ArtIndex::new(Vec::new(), vec![object]).with_pack_fixes();
        assert_eq!(fixed.objects()[0].art, "art/object/field-fg-fb9d731520.webp");
    }

    #[test]
    fn a_picture_keeps_its_art_when_the_pack_has_no_object_of_its_pixels() {
        let index = shared_pack();
        let (pictures, mut objects) = (index.pictures().to_vec(), index.objects().to_vec());
        // (the object of the face's pixels with another key: a pack redrawn under other keys)
        objects[2].key = 0x5A85;
        let fixed = ArtIndex::new(pictures, objects).with_pack_fixes();
        assert_eq!(fixed.pictures()[0].art, "art/picture/face-8A9B-1.webp");
    }

    #[test]
    fn the_index_a_pack_opens_with_is_fixed() {
        // Revision 1's face-8A9B-1 (15532 bytes, Vegeta's eyes) is drawn from the field's art; revision 2's (his whole
        // face, 15874 bytes) keeps its own; the wrong objects go from both
        for (bytes, art) in [
            (15_532, "art/object/field-fg-5a85cef13b.webp"),
            (15_874, "art/picture/face-8A9B-1.webp"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join(FILE), shared_pack().encode()).unwrap();
            std::fs::create_dir_all(dir.path().join("art/picture")).unwrap();
            std::fs::write(dir.path().join("art/picture/face-8A9B-1.webp"), vec![0_u8; bytes])
                .unwrap();
            let opened = ArtIndex::open(dir.path()).unwrap();
            assert_eq!(opened.objects_with(0xB7D6_B360_FC00_302F), &[] as &[u32]);
            assert_eq!(opened.pictures()[0].art, art, "the art file of {bytes} bytes");
        }
    }

    #[test]
    fn a_palette_is_compared_on_the_colours_its_source_used() {
        let used = palette(1);
        assert_eq!(used.distance(&used.colors), 0);
        let mut other = used.colors;
        other[1][0] += 5;
        other[2][2] += 7; // index 2 is not in the mask (bits 1 and 3)
        other[3][1] += 2;
        assert_eq!(used.distance(&other), 5 + 2);
    }

    #[test]
    fn a_pose_of_super_vegeta_s_folder_is_found_by_its_key_only() {
        // Found by its shape or among a palette's poses (a piece, a colour variant), Super Vegeta's own picture of
        // Vegeta's pose would be drawn in colours that are not his (patch 0049): only its key finds it
        let object = |id: &str| {
            let mut palette = Palette { mask: 0b110, ..Palette::default() };
            palette.colors[1] = [10, 20, 30];
            Object {
                key: 7,
                id: id.to_owned(),
                luma: false,
                palette,
                width: 2,
                height: 1,
                mask: vec![0b11],
                art: "art/object/v.webp".to_owned(),
            }
        };
        let index = ArtIndex::new(
            Vec::new(),
            vec![
                object("battle-vegeta-803"),
                object(&format!("battle-vegeta-803{SUPER_VEGETA_ONLY}")),
            ],
        );
        assert_eq!(index.objects_with(7), [0, 1]);
        assert_eq!(index.objects_shaped(2, 1, 2), [0]);
        assert_eq!(index.palette_groups().len(), 1);
        assert_eq!(index.palette_groups()[0].objects, [0]);
        assert!(index.objects()[1].super_vegeta_only() && !index.objects()[0].super_vegeta_only());
    }

    #[test]
    fn a_normal_goku_override_pose_is_found_by_its_key_only() {
        // `art-override\goku\battle-goku-ssj-020.png` (patch 0051): the pose added beside the Super Saiyan art is found by
        // the key, never by its shape nor in a palette group, and it is no Super Vegeta pose
        let object = |id: &str| {
            let mut palette = Palette { mask: 0b10, ..Palette::default() };
            palette.colors[1] = [10, 20, 30];
            Object {
                key: 9,
                id: id.to_owned(),
                luma: false,
                palette,
                width: 2,
                height: 1,
                mask: vec![0b11],
                art: "art/object/g.webp".to_owned(),
            }
        };
        let index = ArtIndex::new(
            Vec::new(),
            vec![
                object("battle-goku-ssj-020"),
                object(&format!("battle-goku-ssj-020{NORMAL_GOKU_ONLY}")),
            ],
        );
        assert_eq!(index.objects_with(9), [0, 1]);
        assert_eq!(index.objects_shaped(2, 1, 2), [0]);
        assert_eq!(index.palette_groups().len(), 1);
        assert_eq!(index.palette_groups()[0].objects, [0]);
        let added = &index.objects()[1];
        assert!(added.normal_goku_only() && added.override_only() && !added.super_vegeta_only());
        assert!(!index.objects()[0].override_only());
    }
}
