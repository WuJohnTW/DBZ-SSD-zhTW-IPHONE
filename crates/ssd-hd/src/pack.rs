//! The HD pack's index and manifest: the assets the converter found in the ROM, each as the keys
//! (content hashes) of its tiles in order, and where its art is. The index holds no ROM bytes and
//! no pixel data; the art is in the pack's image files (PNG or WebP). Format 1.

use crate::art_index::{
    ArtIndex, ENDING_GOKU_ONLY, NORMAL_GOKU_ONLY, Palette, Picture, SUPER_VEGETA_ONLY,
};
use crate::mapart;
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

/// The version of the pack and of its index (`hdpack.toml`'s `format`).
pub const FORMAT: u32 = 1;

const MAGIC: &[u8; 8] = b"SSDHDIDX";

/// Why a pack cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    /// The manifest or the index cannot be read.
    #[error("reading the pack at {}: {message}", path.display())]
    Read { path: PathBuf, message: String },
    /// The index is not an index of this format (wrong magic, version, checksum or layout).
    #[error("index: {0}")]
    Index(&'static str),
    /// The manifest says another format or another number of assets than the index holds.
    #[error("manifest: {0}")]
    Manifest(String),
}

/// The content hash of a tile: FNV-1a over its bytes as they sit in VRAM.
#[must_use]
pub fn tile_key(bytes: &[u8]) -> u64 {
    fnv1a(bytes.iter().copied())
}

/// [`tile_key`] of the VRAM words `words` (each word's low byte first).
#[must_use]
pub fn word_key(words: &[u16]) -> u64 {
    fnv1a(words.iter().flat_map(|word| word.to_le_bytes()))
}

fn fnv1a(bytes: impl Iterator<Item = u8>) -> u64 {
    bytes.fold(0xCBF2_9CE4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3)
    })
}

/// How an asset's pixels are read: palette indexes resolved through the CGRAM row the game
/// uses (`Indexed`, so fades and palette cycles keep working), or colours as drawn (`Rgba`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Indexed,
    Rgba,
}

/// One asset of the pack: tiles of one bit depth, in the order of the stream they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// The logical id (`<class>/<source>/<n>`): a name, not data, the same in every ROM revision.
    pub id: String,
    /// Bits a pixel takes: 2 or 4.
    pub bpp: u8,
    pub mode: Mode,
    /// Tiles per row of the art image.
    pub columns: u16,
    /// The key of each tile, in order.
    pub keys: Vec<u64>,
    /// The art file, relative to the pack directory.
    pub art: String,
}

/// A tile of the index: the asset and the position of the tile in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileRef {
    pub asset: u32,
    pub tile: u32,
}

/// The index: the assets, and the tiles that each key names.
#[derive(Debug, Clone)]
pub struct Index {
    assets: Vec<Asset>,
    tiles: FxHashMap<(u8, u64), Vec<TileRef>>,
}

impl Index {
    /// The index of `assets`, in this order.
    #[must_use]
    pub fn new(assets: Vec<Asset>) -> Self {
        let mut tiles: FxHashMap<(u8, u64), Vec<TileRef>> = FxHashMap::default();
        for (asset, data) in assets.iter().enumerate() {
            for (tile, &key) in data.keys.iter().enumerate() {
                let reference = TileRef { asset: asset as u32, tile: tile as u32 };
                tiles.entry((data.bpp, key)).or_default().push(reference);
            }
        }
        Self { assets, tiles }
    }

    #[must_use]
    pub fn assets(&self) -> &[Asset] {
        &self.assets
    }

    /// The asset `index` names.
    ///
    /// # Panics
    ///
    /// `index` is not an asset of this index.
    #[must_use]
    pub fn asset(&self, index: u32) -> &Asset {
        &self.assets[index as usize]
    }

    /// Every tile of `bpp` bits whose key is `key`, in index order (none, one, or several when
    /// the same tile art is in more than one stream).
    #[must_use]
    pub fn candidates(&self, bpp: u8, key: u64) -> &[TileRef] {
        self.tiles.get(&(bpp, key)).map_or(&[], Vec::as_slice)
    }

    /// The index as its file: magic, format, the assets (id, bit depth, mode, columns, art
    /// path and keys, little-endian), then the CRC-32 of everything before it.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&FORMAT.to_le_bytes());
        out.extend_from_slice(&(self.assets.len() as u32).to_le_bytes());
        for asset in &self.assets {
            put_str(&mut out, &asset.id);
            out.push(asset.bpp);
            out.push(match asset.mode {
                Mode::Indexed => 0,
                Mode::Rgba => 1,
            });
            out.extend_from_slice(&asset.columns.to_le_bytes());
            put_str(&mut out, &asset.art);
            out.extend_from_slice(&(asset.keys.len() as u32).to_le_bytes());
            for key in &asset.keys {
                out.extend_from_slice(&key.to_le_bytes());
            }
        }
        let crc = ssd_rom::crc32(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        out
    }

    /// Reads an index written by [`Index::encode`].
    ///
    /// # Errors
    ///
    /// The bytes are not an index of [`FORMAT`]: the magic, the version, the checksum or a
    /// length is wrong.
    pub fn decode(bytes: &[u8]) -> Result<Self, PackError> {
        let body_len = bytes.len().checked_sub(4).ok_or(PackError::Index("truncated"))?;
        let (body, crc) = bytes.split_at(body_len);
        if ssd_rom::crc32(body).to_le_bytes() != crc {
            return Err(PackError::Index("checksum"));
        }
        let mut reader = Reader { bytes: body, at: 0 };
        if reader.take(8)? != MAGIC {
            return Err(PackError::Index("not an HD pack index"));
        }
        if reader.u32()? != FORMAT {
            return Err(PackError::Index("another format"));
        }
        let count = reader.u32()?;
        let mut assets = Vec::new();
        for _ in 0..count {
            let id = reader.string()?;
            let bpp = reader.u8()?;
            let mode = match reader.u8()? {
                0 => Mode::Indexed,
                1 => Mode::Rgba,
                _ => return Err(PackError::Index("a mode")),
            };
            let columns = reader.u16()?;
            let art = reader.string()?;
            let keys = (0..reader.u32()?).map(|_| reader.u64()).collect::<Result<Vec<_>, _>>()?;
            assets.push(Asset { id, bpp, mode, columns, keys, art });
        }
        if reader.at != body.len() {
            return Err(PackError::Index("trailing bytes"));
        }
        Ok(Self::new(assets))
    }
}

fn put_str(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u16).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

/// A cursor over the index bytes; every read is checked.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, len: usize) -> Result<&[u8], PackError> {
        let end = self.at.checked_add(len).ok_or(PackError::Index("truncated"))?;
        let bytes = self.bytes.get(self.at..end).ok_or(PackError::Index("truncated"))?;
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
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| PackError::Index("a name"))
    }
}

/// `hdpack.toml`: the pack's format, scale, converter version and the ROM it was made from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub scale: u32,
    pub converter: u32,
    /// The CRC-32 of the canonical ROM, as eight hex digits.
    pub rom_crc32: String,
    /// The assets of the index, checked against it on load.
    pub assets: usize,
}

/// A pack on disk: its manifest and its index (the art files are read by the renderer).
#[derive(Debug, Clone)]
pub struct Pack {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub index: Index,
    /// The approved art the pack draws (empty without `art.bin`).
    pub art: crate::art_index::ArtIndex,
    /// The art of the field maps (empty without `art/maps/maps.toml`).
    pub maps: crate::mapart::Index,
    /// The art of the battle backgrounds (empty without `art/bg/bg.toml`).
    pub bg: crate::bgart::Index,
    /// The art of the Mode 7 plane (empty without `art/mode7/mode7.toml`).
    pub mode7: crate::mode7art::Index,
    /// The pose variants of the data packs drawn over this pack (empty without them; see [`Pack::with_overlay`]).
    pub variants: Vec<PoseVariant>,
    /// The pack's own pose variants: art bigger than the box of the original sprite for the poses that need it, those
    /// `art/variants/variants.toml` lists and the pictures of an art override's [`VARIANTS_DIR`] (each in place of the
    /// listed `always` variant of its art, see [`Pack::with_art_override`]); empty without both. The renderer draws one
    /// only while no data pack's variant of the same art holds (`ArtState::pose_variant`).
    pub pose_art: Vec<PoseVariant>,
}

/// Where the pack lists its own pose variants.
pub const POSE_ART: &str = "art/variants/variants.toml";

/// `art/variants/variants.toml`: the pack's own pose variants, as the art import writes them and [`Pack::open`] reads them.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PoseArt {
    #[serde(default, rename = "variant")]
    pub variants: Vec<PoseArtEntry>,
}

/// One pose variant of the pack: the same fields as a data pack's `[[pose_variant]]` (`ssd_mod::PoseVariantEntry`),
/// with the picture's file relative to the pack's folder.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PoseArtEntry {
    /// The name of the object's art it replaces (its `id` in the art index).
    pub art: String,
    /// The condition under which it is drawn (`always` or `kaioken`).
    pub when: String,
    /// The margin on each side, in art pixels: a multiple of 4 up to 96 (`ssd_mod::pose::refusal`).
    pub margin: u32,
    /// The picture, relative to the pack's folder.
    pub file: String,
}

impl PoseArtEntry {
    /// The entry as the renderer's variant (the margin in game pixels), or why it is refused.
    ///
    /// # Errors
    ///
    /// The condition is not one the game has, or the margin is not a whole number of game pixels up to the limit.
    pub fn variant(&self) -> Result<PoseVariant, String> {
        let entry = ssd_mod::PoseVariantEntry {
            art: self.art.clone(),
            when: self.when.clone(),
            file: self.file.clone(),
            margin: self.margin,
        };
        if let Some(reason) = ssd_mod::pose::refusal(&entry) {
            return Err(reason);
        }
        let when = ssd_mod::pose::Condition::from_name(&self.when).ok_or("a condition")?;
        Ok(PoseVariant {
            art: self.art.clone(),
            when,
            margin: self.margin / 4,
            file: self.file.clone(),
        })
    }
}

/// The pose variants `art/variants/variants.toml` of the pack in `dir` lists: none when there is no such file. An entry that
/// is refused is left out, and the log says why.
fn read_pose_art(dir: &Path) -> Result<Vec<PoseVariant>, PackError> {
    let path = dir.join(POSE_ART);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(PackError::Read {
                path: dir.to_path_buf(),
                message: format!("{POSE_ART}: {e}"),
            });
        }
    };
    let listed: PoseArt = toml::from_str(&text).map_err(|e| PackError::Read {
        path: dir.to_path_buf(),
        message: format!("{POSE_ART}: {e}"),
    })?;
    Ok(listed
        .variants
        .iter()
        .filter_map(|entry| match entry.variant() {
            Ok(variant) => Some(variant),
            Err(reason) => {
                log::warn!(
                    "the pack's pose variant {} of {} is left out: {reason}",
                    entry.file,
                    entry.art
                );
                None
            }
        })
        .collect())
}

/// A pose variant of a data pack (its `[[pose_variant]]` entry, `ssd_mod::pose`): the picture drawn in place of the art of the
/// object `art` while `when` holds. The picture is the object's own, grown by `margin` game pixels on each side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoseVariant {
    /// The name of the object's art it replaces (the object's `id` in the art index).
    pub art: String,
    /// The condition under which it is drawn.
    pub when: ssd_mod::pose::Condition,
    /// The margin on each side, in game pixels (the picture's art pixels divided by four).
    pub margin: u32,
    /// The picture: relative to the pack's folder, absolute once it is in a [`Pack`].
    pub file: String,
}

impl Pack {
    /// The newest pack of the ROM `rom_crc32` that the converters left under `cache`
    /// (`<cache>/hd/<crc>/<converter>/`): the highest converter number whose pack opens. `None`
    /// when there is none.
    #[must_use]
    pub fn find(cache: &Path, rom_crc32: u32) -> Option<Self> {
        let folder = cache.join("hd").join(format!("{rom_crc32:08X}"));
        let mut versions: Vec<(u32, PathBuf)> = std::fs::read_dir(&folder)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| Some((entry.file_name().to_str()?.parse().ok()?, entry.path())))
            .collect();
        versions.sort_by_key(|&(version, _)| std::cmp::Reverse(version));
        versions.into_iter().find_map(|(_, dir)| Self::open(&dir).ok())
    }

    /// Opens the pack in `dir`.
    ///
    /// # Errors
    ///
    /// The manifest or the index cannot be read, or they do not agree.
    pub fn open(dir: &Path) -> Result<Self, PackError> {
        let read = |name: &str| {
            std::fs::read(dir.join(name)).map_err(|e| PackError::Read {
                path: dir.to_path_buf(),
                message: format!("{name}: {e}"),
            })
        };
        let text = String::from_utf8(read("hdpack.toml")?)
            .map_err(|_| PackError::Manifest("not UTF-8".to_owned()))?;
        let manifest: Manifest =
            toml::from_str(&text).map_err(|e| PackError::Manifest(e.to_string()))?;
        if manifest.format != FORMAT {
            return Err(PackError::Manifest(format!("format {} is not {FORMAT}", manifest.format)));
        }
        let index = Index::decode(&read("index.bin")?)?;
        if index.assets().len() != manifest.assets {
            return Err(PackError::Manifest(format!(
                "{} assets, the index has {}",
                manifest.assets,
                index.assets().len()
            )));
        }
        let art = crate::art_index::ArtIndex::open(dir)?;
        let maps = crate::mapart::Index::open(dir)
            .map_err(|e| PackError::Read { path: dir.to_path_buf(), message: e.to_string() })?;
        let bg = crate::bgart::Index::open(dir)
            .map_err(|e| PackError::Read { path: dir.to_path_buf(), message: e.to_string() })?;
        let mode7 = crate::mode7art::Index::open(dir)
            .map_err(|e| PackError::Read { path: dir.to_path_buf(), message: e.to_string() })?;
        Ok(Self {
            dir: dir.to_path_buf(),
            manifest,
            index,
            art,
            maps,
            bg,
            mode7,
            variants: Vec::new(),
            pose_art: read_pose_art(dir)?,
        })
    }
}

/// The art of one data pack, drawn over the installed pack's (see [`Pack::with_overlay`]): the layers of its pack maps, the
/// pictures of its faces and its pose variants. The files it names are relative to the folder `dir` of the pack.
#[derive(Debug, Clone, Default)]
pub struct Overlay {
    /// The folder of the pack: the overlay's files are read from it.
    pub dir: PathBuf,
    /// The layers of the pack's maps. They come before the installed layers, so they are the ones drawn.
    pub maps: Vec<mapart::Entry>,
    /// The assets of the pack's faces (one per face: the keys of the 32 tiles of its record), appended to the index.
    pub assets: Vec<Asset>,
    /// The pictures of the pack's faces, appended to the art index: the picture of each face's cells.
    pub pictures: Vec<Picture>,
    /// The pose variants of the pack (`[[pose_variant]]`), in the order of the pack.
    pub variants: Vec<PoseVariant>,
}

/// The field's metatile table (`09:EFB0`): the table a pack map's layout names.
const METATILES: &str = "09:EFB0";

impl Overlay {
    /// The overlay of the art of the data pack `pack` (its id) in `image`, whose folder is `dir`.
    #[must_use]
    pub fn of_pack(image: &ssd_mod::Image, pack: &str, dir: &Path) -> Self {
        let maps = image
            .map_art
            .iter()
            .filter(|art| art.pack == pack)
            .map(|art| mapart::Entry {
                id: format!("{pack} map {}", art.map),
                maps: vec![art.map],
                blocks: [art.blocks.0, art.blocks.1],
                grid: format!("{:02X}:{:04X}", art.grid >> 16, art.grid & 0xFFFF),
                metatiles: METATILES.to_owned(),
                source: String::new(),
                palette: String::new(),
                palette_variant: false,
                tiles: art.tiles.clone(),
            })
            .collect();
        let mut assets = Vec::new();
        let mut pictures = Vec::new();
        for face in image.faces.iter().filter(|face| face.pack == pack) {
            // The keys are the tile keys of the binder (`word_key`, the VRAM bytes of each tile), so the characters the
            // game uploads for the face bind to this asset.
            let id = format!("face/{pack}/{}", face.id);
            let keys = face.tiles.iter().map(|tile| word_key(&tile_words(tile))).collect();
            assets.push(Asset {
                id: id.clone(),
                bpp: 4,
                mode: Mode::Indexed,
                columns: 4,
                keys,
                art: face.picture.clone(),
            });
            pictures.push(Picture {
                name: Path::new(&face.picture)
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                asset: id,
                tiles: std::array::from_fn(|cell| {
                    u16::try_from(ssd_mod::derive::FIRST_PICTURE[cell]).unwrap_or(0)
                }),
                palette: picture_palette(&face.palette, &face.tiles),
                art: face.picture.clone(),
            });
        }
        let variants = image
            .pose_variants
            .iter()
            .filter(|variant| variant.pack == pack)
            .map(|variant| PoseVariant {
                art: variant.art.clone(),
                when: variant.when,
                // The margin is a whole number of game pixels (ssd-mod checks it): four art pixels to one
                margin: variant.margin / 4,
                file: variant.file.clone(),
            })
            .collect();
        Self { dir: dir.to_path_buf(), maps, assets, pictures, variants }
    }
}

/// The 16 words of a 4-bit tile in the SNES layout (the words the VRAM holds, low byte first).
fn tile_words(tile: &[u8; 32]) -> [u16; 16] {
    std::array::from_fn(|word| u16::from_le_bytes([tile[2 * word], tile[2 * word + 1]]))
}

/// The palette index of pixel `(x, y)` of a 4-bit tile in the SNES layout.
fn nibble(tile: &[u8; 32], x: usize, y: usize) -> u8 {
    (0..4).fold(0_u8, |value, plane| {
        let byte = tile[plane / 2 * 16 + 2 * y + plane % 2];
        value | ((byte >> (7 - x)) & 1) << plane
    })
}

/// The palette a face's art was made for: the colours of its row, and the indexes its tiles use (bit `i` for index `i`,
/// 0 excepted: it is transparent).
fn picture_palette(row: &[u16; 16], tiles: &[[u8; 32]]) -> Palette {
    let mut mask = 0_u16;
    for tile in tiles {
        for y in 0..8 {
            for x in 0..8 {
                mask |= 1 << nibble(tile, x, y);
            }
        }
    }
    Palette {
        mask: mask & !1,
        colors: std::array::from_fn(|at| crate::art::poses::rgb555(row[at])),
    }
}

impl Pack {
    /// This pack with `overlay` on top: the overlay's map layers come first (so they are the ones drawn), and its assets,
    /// pictures and pose variants come after the installed ones. Each file the overlay names is read from the overlay's folder.
    #[must_use]
    pub fn with_overlay(mut self, overlay: Overlay) -> Self {
        let Overlay { dir, maps, assets, pictures, variants } = overlay;
        // Absolute: the renderer joins the names below the installed pack's folder, which an absolute name replaces
        let dir = std::path::absolute(&dir).unwrap_or(dir);
        let file = |relative: &str| dir.join(relative).to_string_lossy().into_owned();
        let mut layer: Vec<mapart::Entry> = maps
            .into_iter()
            .map(|entry| mapart::Entry { tiles: file(&entry.tiles), ..entry })
            .collect();
        layer.append(&mut self.maps.layer);
        self.maps.layer = layer;
        let mut all = self.index.assets().to_vec();
        all.extend(assets.into_iter().map(|asset| Asset { art: file(&asset.art), ..asset }));
        self.index = Index::new(all);
        let mut shown = self.art.pictures().to_vec();
        shown.extend(
            pictures.into_iter().map(|picture| Picture { art: file(&picture.art), ..picture }),
        );
        self.art = ArtIndex::new(shown, self.art.objects().to_vec());
        self.variants.extend(
            variants
                .into_iter()
                .map(|variant| PoseVariant { file: file(&variant.file), ..variant }),
        );
        self
    }
}

/// The folder of the data folder whose pictures replace the installed pack's art of a pose (see [`Pack::with_art_override`]).
pub const ART_OVERRIDE: &str = "art-override";
/// The folder in [`ART_OVERRIDE`] of Super Vegeta's own pictures of poses only other art exists for (patch 0049, see
/// [`Pack::with_art_override`]).
pub const SUPER_VEGETA_DIR: &str = "super-vegeta";
/// The start of the name of a picture of [`SUPER_VEGETA_DIR`] for a pose the pack has no art for: `key-<key>.png`, `<key>`
/// the 16 hexadecimal digits of the pose's key (either case), beside its pixels `key-<key>.txt` or, without them, its
/// pose `key-<key>.pose` ([`KEYED_POSE_EXTENSION`]; see `Pack::add_keyed_poses`).
pub const KEYED_PICTURE: &str = "key-";
/// The extension of the file beside a picture of [`KEYED_PICTURE`] that says what its pose is without the game's pixels
/// ([`keyed_pose_text`]): a release ships it in place of the pixels `key-<key>.txt`, which are the original sprite's.
pub const KEYED_POSE_EXTENSION: &str = "pose";
/// The largest `key-<key>.pose` read: its one line is some 70 bytes.
const KEYED_POSE_MAX_BYTES: u64 = 256;
/// Art pixels to a game pixel in a picture of a pose that has no art ([`KEYED_PICTURE`]), as in every pose's art.
const KEYED_PICTURE_SCALE: u32 = 4;
/// The folder in [`ART_OVERRIDE`] of the normal Goku's own pictures (black hair) of poses only Super Saiyan art exists for
/// (patch 0051, see [`Pack::with_art_override`]).
pub const NORMAL_GOKU_DIR: &str = "goku";
/// The folder in [`ART_OVERRIDE`] of Goku's pictures for the ending's flight with Gohan only (his torn clothes): each is
/// drawn in place of the art of its name while the flight is on screen, and the art as before everywhere else (see
/// [`Pack::with_art_override`]).
pub const ENDING_GOKU_DIR: &str = "ending-goku";
/// The folder in [`ART_OVERRIDE`] of pose variants: `<name>.png` is drawn as an `always` variant of the art `name` with a
/// margin of [`OVERRIDE_VARIANT_MARGIN`] art pixels (see [`Pack::with_art_override`]).
pub const VARIANTS_DIR: &str = "variants";
/// The folder in [`ART_OVERRIDE`] of the art of Mode 7 planes, a folder for each plane by what it shows
/// (`mode7\<content key>\t<x>_<y>.png`, see `Pack::mode7_override`).
pub const MODE7_DIR: &str = "mode7";
/// The margin, in art pixels on each side, of a picture of [`VARIANTS_DIR`]: the margin of the pack's own variants.
pub const OVERRIDE_VARIANT_MARGIN: u32 = 76;

/// The PNG files of `dir` (its own, not those of a folder in it) by name, sorted; `None` when it cannot be read.
fn override_pictures(dir: &Path) -> Option<Vec<(String, PathBuf)>> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut files: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let png =
                path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("png"));
            let name = path.file_stem()?.to_str()?.to_owned();
            (png && path.is_file()).then_some((name, path))
        })
        .collect();
    files.sort();
    // Absolute: the renderer joins the art's name below the pack's folder, which an absolute name replaces
    Some(
        files
            .into_iter()
            .map(|(name, path)| (name, std::path::absolute(&path).unwrap_or(path)))
            .collect(),
    )
}

/// The size of the PNG at `path`.
fn picture_size(path: &Path) -> Result<(u32, u32), String> {
    std::fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| crate::png::read_rgba(&bytes))
        .map(|(width, height, _)| (width, height))
}

/// What a folder of override pictures does to the poses of a pack ([`Pack::art_override`]).
#[derive(Debug, Default)]
struct ArtOverride {
    /// The poses of the index, the art of each pose a picture replaces naming that picture.
    objects: Vec<crate::art_index::Object>,
    /// The names of the art replaced, in the order of the files' names.
    replaced: Vec<String>,
    /// The pictures (or the poses of a picture) left out, and why.
    refused: Vec<String>,
    /// The pose variants of the pictures of [`VARIANTS_DIR`], in the order of the files' names.
    variants: Vec<PoseVariant>,
}

/// A folder of [`ART_OVERRIDE`] whose pictures are poses added for one set of colours only (patches 0049, 0051).
struct OnlyFor {
    /// The folder's name (in the log and in [`ArtOverride::replaced`]).
    folder: &'static str,
    /// The end of the added poses' ids.
    suffix: &'static str,
    /// Whose colours, in the log.
    whose: &'static str,
    /// The palette indexes a palette must have besides the pose's own (the hair).
    hair: u16,
    /// The palette a pose whose art may be drawn in any colours (an effect's, palette mask 0) is added in; `None`: such a
    /// pose is left out.
    effect: Option<Palette>,
    /// The palettes the added poses take, the first with the colours wanted.
    palettes: Vec<Palette>,
    /// Whether its pictures named [`KEYED_PICTURE`] are poses the pack has no art for (`Pack::add_keyed_poses`), not
    /// pictures of art of that name.
    keyed: bool,
}

/// The key a picture's name `key-<key>` ([`KEYED_PICTURE`]) gives, `None` for any other name.
fn keyed_picture(name: &str) -> Option<u64> {
    let digits = name.strip_prefix(KEYED_PICTURE)?;
    (digits.len() == 16 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| u64::from_str_radix(digits, 16).ok())
        .flatten()
}

/// The pixels of a pose as `key-<key>.txt` gives them ([`KEYED_PICTURE`]): a line for each row of the pose's box, top to
/// bottom, a hexadecimal digit for each pixel (its palette index, `0` transparent), every line as long; blank lines and
/// spaces at a line's end are left out, and so is a UTF-8 byte order mark at the start (Notepad's). Returns the array row
/// by row, its width and its height. Pixels wider or taller than a pose's box can be ([`KEYED_PIXELS_MAX_SIDE`]) are
/// refused before the array is made.
fn keyed_pixels(text: &str) -> Result<(Vec<u8>, usize, usize), String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rows: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .take(KEYED_PIXELS_MAX_SIDE + 1)
        .collect();
    let width = rows.first().map_or(0, |row| row.len());
    if width == 0 {
        return Err("it has no pixel".to_owned());
    }
    if width > KEYED_PIXELS_MAX_SIDE {
        return Err(format!(
            "line 1 is {width} pixels, a pose is 1 to {KEYED_PIXELS_MAX_SIDE} each way"
        ));
    }
    if rows.len() > KEYED_PIXELS_MAX_SIDE {
        return Err(format!(
            "it has more than {KEYED_PIXELS_MAX_SIDE} lines of pixels, a pose is 1 to {KEYED_PIXELS_MAX_SIDE} each way"
        ));
    }
    let mut arr = Vec::with_capacity(width * rows.len());
    for (at, row) in rows.iter().enumerate() {
        if row.len() != width {
            return Err(format!("line {} is {} pixels, the first {width}", at + 1, row.len()));
        }
        for c in row.chars() {
            let index = c
                .to_digit(16)
                .ok_or_else(|| format!("line {} has {c:?}, not a hexadecimal digit", at + 1))?;
            arr.push(index as u8);
        }
    }
    Ok((arr, width, rows.len()))
}

/// The widest and the tallest a pose's box is, in game pixels ([`keyed_pixels`]).
const KEYED_PIXELS_MAX_SIDE: usize = 255;

/// The largest `key-<key>.txt` read ([`keyed_pixels_file`]): the pixels of a pose of 255 x 255 are some 64 KiB of text.
const KEYED_PIXELS_MAX_BYTES: u64 = 1 << 20;

/// [`keyed_pixels`] of the file `path`. A file larger than [`KEYED_PIXELS_MAX_BYTES`] is refused without reading it, and
/// one that is not a regular file (a FIFO, whose opening waits for a writer; a folder) without opening it.
fn keyed_pixels_file(path: &Path) -> Result<(Vec<u8>, usize, usize), String> {
    use std::io::Read;
    if !std::fs::metadata(path).map_err(|error| error.to_string())?.is_file() {
        return Err("it is not a regular file".to_owned());
    }
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let size = file.metadata().map_err(|error| error.to_string())?.len();
    if size > KEYED_PIXELS_MAX_BYTES {
        return Err(format!("it is {size} bytes, more than {KEYED_PIXELS_MAX_BYTES}"));
    }
    let mut text = String::new();
    file.take(KEYED_PIXELS_MAX_BYTES)
        .read_to_string(&mut text)
        .map_err(|error| error.to_string())?;
    keyed_pixels(&text)
}

/// What a keyed picture's pose is ([`KEYED_PICTURE`]), all the renderer needs of it: the box in game pixels, the palette
/// indexes the pose uses, the key of its mirror image (an index that has that pose already keeps its art), and its
/// silhouette (empty when only `key-<key>.pose` said what the pose is: a pose added for one set of colours only is found
/// by its key alone, never by its shape or as a piece of another pose, see `ArtIndex::new`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct KeyedShape {
    key: u64,
    width: usize,
    height: usize,
    used: u16,
    mirror: u64,
    silhouette: Vec<u8>,
}

/// The [`KeyedShape`] of a pose's pixels ([`keyed_pixels`]); `None` for a box out of a pose's size.
fn keyed_shape(arr: &[u8], width: usize, height: usize) -> Option<KeyedShape> {
    let key = crate::art::poses::fighter_key(arr, width, height)?;
    let mut flipped = Vec::with_capacity(arr.len());
    for row in arr.chunks(width) {
        flipped.extend(row.iter().rev());
    }
    let mirror = crate::art::poses::fighter_key(&flipped, width, height).unwrap_or(key);
    let (mut used, mut silhouette) = (0_u16, vec![0_u8; arr.len().div_ceil(8)]);
    for (at, &index) in arr.iter().enumerate() {
        if index != 0 {
            used |= 1 << index;
            silhouette[at / 8] |= 1 << (at % 8);
        }
    }
    Some(KeyedShape { key, width, height, used, mirror, silhouette })
}

/// The text of `key-<key>.pose` ([`KEYED_POSE_EXTENSION`]) for the pixels `pixels` of `key-<key>.txt`
/// ([`keyed_pixels`]): one line `key=<key> size=<width>x<height> colours=<used> mirror=<key of the mirror image>`, the
/// keys 16 hexadecimal digits, the palette indexes the pose uses a bit each in 4. Nothing of the pixels themselves (a
/// 64-bit key cannot be turned back into them), so a release ships it in place of the original sprite's pixels.
///
/// # Errors
///
/// Pixels [`keyed_pixels`] refuses, or a box out of a pose's size.
pub fn keyed_pose_text(pixels: &str) -> Result<String, String> {
    let (arr, width, height) = keyed_pixels(pixels)?;
    let shape = keyed_shape(&arr, width, height)
        .ok_or_else(|| format!("its pixels are {width} x {height}, a pose is 1 to 255 each way"))?;
    Ok(format!(
        "key={:016x} size={}x{} colours={:04x} mirror={:016x}\n",
        shape.key, shape.width, shape.height, shape.used, shape.mirror
    ))
}

/// The [`KeyedShape`] `key-<key>.pose` gives ([`keyed_pose_text`]), with no silhouette. A UTF-8 byte order mark and blank
/// space around the line are left out; anything else that is not the line is refused.
fn keyed_pose(text: &str) -> Result<KeyedShape, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text).trim();
    let fields: Vec<&str> = text.split_whitespace().collect();
    let [key, size, colours, mirror] = fields.as_slice() else {
        return Err(format!("it is not one line of key=, size=, colours= and mirror=: {text:?}"));
    };
    let hex = |field: &str, name: &str, digits: usize| -> Result<u64, String> {
        let value = field
            .strip_prefix(name)
            .and_then(|value| value.strip_prefix('='))
            .ok_or_else(|| format!("{field:?} is not {name}="))?;
        if value.len() != digits || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("{name}={value} is not {digits} hexadecimal digits"));
        }
        u64::from_str_radix(value, 16).map_err(|error| error.to_string())
    };
    let (key, mirror) = (hex(key, "key", 16)?, hex(mirror, "mirror", 16)?);
    let used = hex(colours, "colours", 4)? as u16;
    if used == 0 || used & 1 != 0 {
        return Err(format!("colours={used:04x}: a pose uses some of the indexes 1 to 15 only"));
    }
    let side = |value: &str| {
        value.parse::<usize>().ok().filter(|side| (1..=KEYED_PIXELS_MAX_SIDE).contains(side))
    };
    let (width, height) = size
        .strip_prefix("size=")
        .and_then(|size| size.split_once('x'))
        .and_then(|(w, h)| Some((side(w)?, side(h)?)))
        .ok_or_else(|| {
            format!("{size:?} is not size=<width>x<height>, 1 to {KEYED_PIXELS_MAX_SIDE} each")
        })?;
    Ok(KeyedShape { key, width, height, used, mirror, silhouette: Vec::new() })
}

/// [`keyed_pose`] of the file `path`, refused as [`keyed_pixels_file`] refuses (larger than [`KEYED_POSE_MAX_BYTES`],
/// not a regular file).
fn keyed_pose_file(path: &Path) -> Result<KeyedShape, String> {
    use std::io::Read;
    if !std::fs::metadata(path).map_err(|error| error.to_string())?.is_file() {
        return Err("it is not a regular file".to_owned());
    }
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let size = file.metadata().map_err(|error| error.to_string())?.len();
    if size > KEYED_POSE_MAX_BYTES {
        return Err(format!("it is {size} bytes, more than {KEYED_POSE_MAX_BYTES}"));
    }
    let mut text = String::new();
    file.take(KEYED_POSE_MAX_BYTES).read_to_string(&mut text).map_err(|error| error.to_string())?;
    keyed_pose(&text)
}

/// The [`KeyedShape`] of the keyed picture `path` ([`KEYED_PICTURE`]): from its pixels `key-<key>.txt` when that file is
/// there (the pose's key, size and silhouette computed from them), else from its pose `key-<key>.pose`. The error names
/// the file it comes from.
/// Returns it with the words for its file in the log (`pixels <path> have`, `pose <path> has`).
fn keyed_shape_file(path: &Path) -> Result<(KeyedShape, String), String> {
    let pixels_file = path.with_extension("txt");
    let pose_file = path.with_extension(KEYED_POSE_EXTENSION);
    if std::fs::symlink_metadata(&pixels_file).is_err()
        && std::fs::symlink_metadata(&pose_file).is_ok()
    {
        let shape = keyed_pose_file(&pose_file)
            .map_err(|error| format!("its pose {}: {error}", pose_file.display()))?;
        return Ok((shape, format!("pose {} has", pose_file.display())));
    }
    let (arr, width, height) = keyed_pixels_file(&pixels_file).map_err(|error| {
        format!(
            "its pixels {} (or its pose {}): {error}",
            pixels_file.display(),
            pose_file.display()
        )
    })?;
    let shape = keyed_shape(&arr, width, height)
        .ok_or_else(|| format!("its pixels are {width} x {height}, a pose is 1 to 255 each way"))?;
    Ok((shape, format!("pixels {} have", pixels_file.display())))
}

impl Pack {
    /// This pack with the pictures of `dir` in place of the art of its poses: `<dir>/<name>.png` replaces the art of every
    /// pose of the index whose name is `name` (its `id`, the stem of the art's file in the pack). Only the picture changes:
    /// the pose keeps its key, its palette and its silhouette, so it is found, recoloured, flashed and faded as the pack's
    /// own art is. A picture must be the size of the art it replaces: one of another size, one that cannot be read and one
    /// that names no pose of the pack are left out, and the log says why. Without the folder the pack is as it was. The
    /// pictures of its folder [`SUPER_VEGETA_DIR`] are poses added for Super Vegeta only (patch 0049, see
    /// `Pack::super_vegeta_override`; `key-<key>.png` with `key-<key>.txt`, [`KEYED_PICTURE`], a pose the pack has no art
    /// for), those of [`NORMAL_GOKU_DIR`] poses added for the normal Goku only (patch 0051, see
    /// `Pack::normal_goku_override`), those of [`ENDING_GOKU_DIR`] poses added for the ending's flight only (see
    /// `Pack::ending_goku_override`), and those of [`VARIANTS_DIR`] pose variants (see `Pack::variant_override`): each is
    /// one of the pack's own variants ([`Pack::pose_art`]) in place of the pack's `always` variant of the same art.
    #[must_use]
    pub fn with_art_override(mut self, dir: &Path) -> Self {
        let found = self.art_override(dir);
        for reason in &found.refused {
            log::warn!("the art override {reason}");
        }
        if !found.replaced.is_empty() {
            log::info!(
                "art override: {} pictures from {} ({})",
                found.replaced.len(),
                dir.display(),
                found.replaced.join(", ")
            );
            self.art = ArtIndex::new(self.art.pictures().to_vec(), found.objects);
        }
        if !found.variants.is_empty() {
            let names: Vec<&str> =
                found.variants.iter().map(|variant| variant.art.as_str()).collect();
            let replaced: Vec<&str> = names
                .iter()
                .copied()
                .filter(|&name| {
                    self.pose_art
                        .iter()
                        .any(|own| own.art == name && own.when == ssd_mod::pose::Condition::Always)
                })
                .collect();
            log::info!(
                "art override: {} pose variants from {} ({}){}",
                names.len(),
                dir.join(VARIANTS_DIR).display(),
                names.join(", "),
                if replaced.is_empty() {
                    String::new()
                } else {
                    format!(", in place of the pack's own of {}", replaced.join(", "))
                }
            );
            // The pack's `always` variant of an art a picture names gives way; one for a condition (the Kaioken) stays,
            // and is drawn while its condition holds, as it is over the pack's own `always` one
            self.pose_art.retain(|own| {
                own.when != ssd_mod::pose::Condition::Always || !names.contains(&own.art.as_str())
            });
            self.pose_art.extend(found.variants);
        }
        self.mode7_override(dir);
        self
    }

    /// The Mode 7 planes of `dir`'s folder [`MODE7_DIR`] (see [`Pack::with_art_override`]): each folder
    /// `<content key>\` (the 16 hexadecimal digits of `ssd_hd::mode7art::content_key`, what the plane shows) holds tiles
    /// `t<x>_<y>.png` (x and y 0 to 3, each 1024 x 1024: a quarter of the plane's 1024 game pixels at four art pixels to
    /// one, RGBA, alpha 0 where the plane has no art; a tile without a file has none) and may hold `palette.txt`, the 256
    /// colours the art was made in (`RRGGBB`, one a line, index 0 first; without it, those of the pack's art of the same
    /// content). The plane is drawn from it for that content before the pack's own art. Where the art is transparent the
    /// game's pixel of the plane shows; with an empty file `replace` in the folder (that name exactly) what is under the
    /// plane shows there instead, as where the plane is transparent (the frame is drawn again without BG1 for it: the
    /// sides of the plane the art leaves out, Super Vegeta's shoulder pads, are gone and the speed lines behind them
    /// show). A folder whose name, tiles or palette are not that is left out, and the log says why; a file of another
    /// name (`Replace`, `replace.txt`) is left out, and logged.
    fn mode7_override(&mut self, dir: &Path) {
        use crate::mode7art::{CONTENT_ALIASES, CONTENT_PREFIX, Entry, SCALE, TILE};
        let Ok(folders) = std::fs::read_dir(dir.join(MODE7_DIR)) else {
            return;
        };
        let mut folders: Vec<PathBuf> = folders
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        folders.sort();
        let side = (TILE * SCALE) as u32;
        let mut added = Vec::new();
        for folder in folders {
            let shown = folder.display().to_string();
            let name = folder
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let Some(content) =
                (name.len() == 16).then(|| u64::from_str_radix(&name, 16).ok()).flatten()
            else {
                log::warn!(
                    "the art override {shown} is left out: its name is not a content key (16 hexadecimal digits)"
                );
                continue;
            };
            // The files of the folder by their exact names (a file system may not tell `Replace` from `replace`)
            let names: Vec<String> = std::fs::read_dir(&folder)
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .filter_map(|entry| entry.file_name().into_string().ok())
                        .collect()
                })
                .unwrap_or_default();
            let known = |file: &str| {
                file == "replace"
                    || file == "palette.txt"
                    || (0..4).any(|x| (0..4).any(|y| file == format!("t{x}_{y}.png")))
            };
            for other in names.iter().filter(|file| !known(file)) {
                log::warn!(
                    "the art override {shown}: {other} is left out (not t<x>_<y>.png, palette.txt or replace)"
                );
            }
            let replace = names.iter().any(|file| file == "replace");
            let mut tiles = 0;
            let mut refused = None;
            for (x, y) in (0..4).flat_map(|x| (0..4).map(move |y| (x, y))) {
                let file = folder.join(format!("t{x}_{y}.png"));
                if !file.is_file() {
                    continue;
                }
                match picture_size(&file) {
                    Ok((w, h)) if (w, h) == (side, side) => tiles += 1,
                    Ok((w, h)) => {
                        refused = Some(format!("{} is {w}x{h}, not {side}x{side}", file.display()));
                    }
                    Err(error) => refused = Some(format!("{}: {error}", file.display())),
                }
            }
            if let Some(reason) = refused {
                log::warn!("the art override {shown} is left out: {reason}");
                continue;
            }
            if tiles == 0 {
                log::warn!("the art override {shown} is left out: it has no tile t<x>_<y>.png");
                continue;
            }
            // Without palette.txt, the colours of the pack's art of the same content (by its content, or the plane key
            // `CONTENT_ALIASES` gives it)
            let own = self
                .mode7
                .plane
                .iter()
                .position(|entry| entry.content_value() == Some(content))
                .or_else(|| {
                    let &(_, key, _) =
                        CONTENT_ALIASES.iter().find(|&&(alias, _, _)| alias == content)?;
                    self.mode7.find(key)
                });
            let palette = match (std::fs::read_to_string(folder.join("palette.txt")), own) {
                (Ok(text), _) => text.split_whitespace().map(str::to_ascii_uppercase).collect(),
                (Err(_), Some(at)) => self.mode7.plane[at].palette.clone(),
                (Err(_), None) => {
                    log::warn!(
                        "the art override {shown} is left out: it has no palette.txt and the pack has no art of its content"
                    );
                    continue;
                }
            };
            let folder = std::path::absolute(&folder).unwrap_or(folder);
            let entry = Entry {
                id: format!("{ART_OVERRIDE}/{MODE7_DIR}/{name}"),
                key: format!("{CONTENT_PREFIX}{name}"),
                // Absolute: the renderer joins the tiles' name below the pack's `art/mode7`, which an absolute name replaces
                tiles: folder.join("t{x}_{y}.png").to_string_lossy().into_owned(),
                palette,
                replace,
            };
            if entry.colours().is_none() {
                log::warn!(
                    "the art override {shown} is left out: palette.txt is not 256 colours RRGGBB"
                );
                continue;
            }
            self.mode7.plane.push(entry);
            added.push(format!("{name} ({tiles} tiles{})", if replace { ", replace" } else { "" }));
        }
        if !added.is_empty() {
            log::info!(
                "art override: {} Mode 7 planes from {} ({})",
                added.len(),
                dir.join(MODE7_DIR).display(),
                added.join(", ")
            );
        }
    }

    /// The poses of the index with the art of `dir`'s pictures (see [`Pack::with_art_override`]), and what was left out.
    fn art_override(&self, dir: &Path) -> ArtOverride {
        let Some(files) = override_pictures(dir) else {
            return ArtOverride::default();
        };
        let mut found =
            ArtOverride { objects: self.art.objects().to_vec(), ..ArtOverride::default() };
        for (name, path) in files {
            let file = path.display();
            let size = match picture_size(&path) {
                Ok(size) => size,
                Err(error) => {
                    found.refused.push(format!("{file} is left out: {error}"));
                    continue;
                }
            };
            let (mut named, mut used) = (false, false);
            for object in found.objects.iter_mut().filter(|object| object.id == name) {
                named = true;
                let own = match crate::png::file_dimensions(&self.dir.join(&object.art)) {
                    Ok(own) => own,
                    Err(error) => {
                        found.refused.push(format!(
                            "{file} is left out for the pose {:016X}: its art {}: {error}",
                            object.key, object.art
                        ));
                        continue;
                    }
                };
                if size != own {
                    found.refused.push(format!(
                        "{file} is left out for the pose {:016X}: it is {} x {} pixels, the art {} is {} x {}",
                        object.key, size.0, size.1, object.art, own.0, own.1
                    ));
                    continue;
                }
                object.art = path.to_string_lossy().into_owned();
                used = true;
            }
            if !named {
                found.refused.push(format!("{file} is left out: the pack has no art named {name}"));
            }
            if used {
                found.replaced.push(name);
            }
        }
        self.super_vegeta_override(&dir.join(SUPER_VEGETA_DIR), &mut found);
        self.normal_goku_override(&dir.join(NORMAL_GOKU_DIR), &mut found);
        self.ending_goku_override(&dir.join(ENDING_GOKU_DIR), &mut found);
        self.variant_override(&dir.join(VARIANTS_DIR), &mut found);
        found
    }

    /// The pictures of `dir` (the [`VARIANTS_DIR`] of [`ART_OVERRIDE`]) as pose variants in `found`: `<dir>/<name>.png` is
    /// drawn in place of the art `name` in every frame (`always`), grown by [`OVERRIDE_VARIANT_MARGIN`] art pixels on each
    /// side, as a variant of the pack's own (`art/variants/variants.toml`) with that margin is: by the same renderer, so the
    /// HD text leaves its margin where the fighter is in front of the text cells, and the screen's fade shades it. A picture
    /// must be the size of the art of its name with the margin on each side; one of another size, one that cannot be read
    /// and one that names no pose are left out, and the log says why.
    fn variant_override(&self, dir: &Path, found: &mut ArtOverride) {
        let Some(files) = override_pictures(dir) else {
            return;
        };
        let grown = 2 * OVERRIDE_VARIANT_MARGIN;
        for (name, path) in files {
            let file = path.display();
            let size = match picture_size(&path) {
                Ok(size) => size,
                Err(error) => {
                    found.refused.push(format!("{file} is left out: {error}"));
                    continue;
                }
            };
            let (mut named, mut fits) = (false, true);
            for object in found.objects.iter().filter(|object| object.id == name) {
                named = true;
                let own = match crate::png::file_dimensions(&self.dir.join(&object.art)) {
                    Ok(own) => own,
                    Err(error) => {
                        found.refused.push(format!(
                            "{file} is left out for the pose {:016X}: its art {}: {error}",
                            object.key, object.art
                        ));
                        fits = false;
                        break;
                    }
                };
                if size != (own.0 + grown, own.1 + grown) {
                    found.refused.push(format!(
                        "{file} is left out for the pose {:016X}: it is {} x {} pixels, the art {} is {} x {}, a \
                         variant of it {} x {}",
                        object.key,
                        size.0,
                        size.1,
                        object.art,
                        own.0,
                        own.1,
                        own.0 + grown,
                        own.1 + grown
                    ));
                    fits = false;
                    break;
                }
            }
            if !named {
                found.refused.push(format!("{file} is left out: the pack has no art named {name}"));
                continue;
            }
            if fits {
                found.variants.push(PoseVariant {
                    art: name,
                    when: ssd_mod::pose::Condition::Always,
                    // four art pixels to a game pixel, as the pack's own variants' margins (`PoseArtEntry::variant`)
                    margin: OVERRIDE_VARIANT_MARGIN / 4,
                    file: path.to_string_lossy().into_owned(),
                });
            }
        }
    }

    /// The pictures of `dir` (the [`SUPER_VEGETA_DIR`] of [`ART_OVERRIDE`], patch 0049) added to `found`: the game draws
    /// Super Vegeta in some poses (his energy wave, a kick that throws him) with Vegeta's tiles in his own palette, and
    /// the pack has only Vegeta's art for them, with the shoulder pads (and black hair, so the pose keeps the game's
    /// sprite, patch 0043). `<dir>/<name>.png` is a pose added beside each pose named `name`, with its key, silhouette
    /// and size, the picture for art and for palette the colours of Super Vegeta's art (an art of `battle-super-vegeta`
    /// with every colour the pose uses and his hair), its id `name` and [`SUPER_VEGETA_ONLY`]: the renderer draws it
    /// only in Super Vegeta's colours, and the pose of the name stays as it was for Vegeta. A picture must be the size of
    /// the art of its name; one of another size, one that cannot be read, one that names no pose and one for a pose no
    /// art of Super Vegeta has the colours of (or a greyscale shape) are left out, and the log says why. A pose of an
    /// effect's art (palette mask 0, any colours: Vegeta flying over GAME OVER, `gallery-fx-1118`) is added in his palette
    /// of GAME OVER ([`crate::art::game_over_super_vegeta`]), so its picture is drawn only where the sprite has it.
    /// `<dir>/key-<key>.png` ([`KEYED_PICTURE`]) is a pose the pack has no art for, with its pixels beside it in
    /// `key-<key>.txt`, added in his colours only (see `Self::add_keyed_poses`).
    fn super_vegeta_override(&self, dir: &Path, found: &mut ArtOverride) {
        let palettes: Vec<Palette> = found
            .objects
            .iter()
            .filter(|object| {
                object.id.contains(crate::art::SUPER_VEGETA_MARK)
                    && object.palette.mask != 0
                    && !object.override_only()
            })
            .map(|object| object.palette)
            .collect();
        let added = OnlyFor {
            folder: SUPER_VEGETA_DIR,
            suffix: SUPER_VEGETA_ONLY,
            whose: "Super Vegeta",
            hair: crate::art::SUPER_VEGETA_HAIR,
            effect: Some(crate::art::game_over_super_vegeta()),
            palettes,
            keyed: true,
        };
        self.add_override_poses(dir, &added, found);
        Self::add_keyed_poses(dir, &added, found);
    }

    /// The pictures of `dir` (the [`NORMAL_GOKU_DIR`] of [`ART_OVERRIDE`], patch 0051) added to `found`: the pack has only
    /// Super Saiyan art for some poses the game draws for the normal Goku too (the whole body powering up,
    /// `battle-goku-ssj-020`, the close-up of the Kaioken; the face close-up `battle-goku-ssj-019`), so they were drawn with
    /// golden hair, or keep the game's sprite (patch 0050). `<dir>/<name>.png` is a pose added beside each pose named
    /// `name`, as in [`Self::super_vegeta_override`], for palette the colours of the normal Goku's art (an art of Goku that is
    /// not the Super Saiyan's with every colour the pose uses and his hair, a battle pose first), its id `name` and
    /// [`NORMAL_GOKU_ONLY`]: the renderer draws it only where the hair is not the Super Saiyan's, and the pose of the name
    /// stays as it was for him.
    fn normal_goku_override(&self, dir: &Path, found: &mut ArtOverride) {
        let normal = |id: &str| {
            id.contains(crate::art::GOKU_MARK) && !id.contains(crate::art::SUPER_SAIYAN_GOKU_MARK)
        };
        let mut palettes: Vec<(bool, Palette)> = found
            .objects
            .iter()
            .filter(|object| {
                normal(&object.id) && object.palette.mask != 0 && !object.override_only()
            })
            .map(|object| (!object.id.starts_with("battle-"), object.palette))
            .collect();
        // the battle poses' colours first (a stable sort keeps the index's order among them)
        palettes.sort_by_key(|&(other, _)| other);
        let added = OnlyFor {
            folder: NORMAL_GOKU_DIR,
            suffix: NORMAL_GOKU_ONLY,
            whose: "the normal Goku",
            hair: crate::art::GOKU_HAIR,
            effect: None,
            palettes: palettes.into_iter().map(|(_, palette)| palette).collect(),
            keyed: false,
        };
        self.add_override_poses(dir, &added, found);
    }

    /// The pictures of `dir` (the [`ENDING_GOKU_DIR`] of [`ART_OVERRIDE`]) added to `found`: `<dir>/<name>.png` is a pose
    /// added beside each pose of Goku's art named `name` (an id with [`crate::art::GOKU_MARK`]), the same pose (key,
    /// silhouette, size, palette) with the picture for art and for id the name and [`ENDING_GOKU_ONLY`]. The renderer
    /// finds it by nothing: it draws it in place of the art of its name while the ending's flight with Gohan is on screen
    /// (`Conditions::of_work_ram`), so Goku can wear his torn clothes there and nowhere else (the same art draws him in
    /// battles before Namek). A picture must be the size of the art of its name; one of another size, one that cannot be
    /// read, one that names no pose and one that names art not of Goku are left out, and the log says why. Without the
    /// folder, or with it empty, nothing is added.
    fn ending_goku_override(&self, dir: &Path, found: &mut ArtOverride) {
        let Some(files) = override_pictures(dir) else {
            return;
        };
        let mut added = Vec::new();
        for (name, path) in files {
            let file = path.display();
            let size = match picture_size(&path) {
                Ok(size) => size,
                Err(error) => {
                    found.refused.push(format!("{file} is left out: {error}"));
                    continue;
                }
            };
            let (mut named, mut used) = (false, false);
            for object in found.objects.iter().filter(|object| object.id == name) {
                named = true;
                if !object.id.contains(crate::art::GOKU_MARK) {
                    found.refused.push(format!(
                        "{file} is left out for the pose {:016X}: {} is not Goku's art",
                        object.key, object.id
                    ));
                    continue;
                }
                let own = match crate::png::file_dimensions(&self.dir.join(&object.art)) {
                    Ok(own) => own,
                    Err(error) => {
                        found.refused.push(format!(
                            "{file} is left out for the pose {:016X}: its art {}: {error}",
                            object.key, object.art
                        ));
                        continue;
                    }
                };
                if size != own {
                    found.refused.push(format!(
                        "{file} is left out for the pose {:016X}: it is {} x {} pixels, the art {} is {} x {}",
                        object.key, size.0, size.1, object.art, own.0, own.1
                    ));
                    continue;
                }
                added.push(crate::art_index::Object {
                    id: format!("{name}{ENDING_GOKU_ONLY}"),
                    art: path.to_string_lossy().into_owned(),
                    ..object.clone()
                });
                used = true;
            }
            if !named {
                found.refused.push(format!("{file} is left out: the pack has no art named {name}"));
            }
            if used {
                found.replaced.push(format!("{ENDING_GOKU_DIR}/{name}"));
            }
        }
        found.objects.extend(added);
    }

    /// The pictures of `dir` added to `found` as poses for one set of colours only ([`OnlyFor`]): each beside every pose
    /// of its name, with its key, silhouette and size, the picture for art, for palette the first of `only.palettes` with
    /// every colour the pose uses and `only.hair`, and for id the name and `only.suffix`. A picture must be the size of the
    /// art of its name; one of another size, one that cannot be read, one that names no pose and one for a pose none of
    /// the palettes has the colours of (or a greyscale shape) are left out, and the log says why.
    fn add_override_poses(&self, dir: &Path, only: &OnlyFor, found: &mut ArtOverride) {
        let Some(files) = override_pictures(dir) else {
            return;
        };
        let palettes = &only.palettes;
        let mut added = Vec::new();
        for (name, path) in files {
            if only.keyed && keyed_picture(&name).is_some() {
                // (a pose of no art: `Self::add_keyed_poses`)
                continue;
            }
            let file = path.display();
            let size = match picture_size(&path) {
                Ok(size) => size,
                Err(error) => {
                    found.refused.push(format!("{file} is left out: {error}"));
                    continue;
                }
            };
            let (mut named, mut used) = (false, false);
            for object in found.objects.iter().filter(|object| object.id == name) {
                named = true;
                let own = match crate::png::file_dimensions(&self.dir.join(&object.art)) {
                    Ok(own) => own,
                    Err(error) => {
                        found.refused.push(format!(
                            "{file} is left out for the pose {:016X}: its art {}: {error}",
                            object.key, object.art
                        ));
                        continue;
                    }
                };
                if size != own {
                    found.refused.push(format!(
                        "{file} is left out for the pose {:016X}: it is {} x {} pixels, the art {} is {} x {}",
                        object.key, size.0, size.1, object.art, own.0, own.1
                    ));
                    continue;
                }
                let wanted = object.palette.mask | only.hair;
                let colours = palettes.iter().find(|palette| palette.mask & wanted == wanted);
                let palette = match (only.effect, colours) {
                    // (an effect's art, palette mask 0, may be drawn in any colours: `only.effect`'s)
                    (Some(effect), _) if !object.luma && object.palette.mask == 0 => effect,
                    (_, Some(colours)) if !object.luma && object.palette.mask != 0 => {
                        Palette { mask: object.palette.mask, colors: colours.colors }
                    }
                    _ => {
                        found.refused.push(format!(
                            "{file} is left out for the pose {:016X}: no art of {} has the colours of {}",
                            object.key, only.whose, object.id
                        ));
                        continue;
                    }
                };
                added.push(crate::art_index::Object {
                    id: format!("{name}{}", only.suffix),
                    palette,
                    art: path.to_string_lossy().into_owned(),
                    ..object.clone()
                });
                used = true;
            }
            if !named {
                found.refused.push(format!("{file} is left out: the pack has no art named {name}"));
            }
            if used {
                found.replaced.push(format!("{}/{name}", only.folder));
            }
        }
        found.objects.extend(added);
    }

    /// The pictures of `dir` named `key-<key>` ([`KEYED_PICTURE`]) added to `found` as poses the pack has no art for, for
    /// one set of colours only ([`OnlyFor`], Super Vegeta's): the game draws him in some poses (the back he turns to
    /// Goku's Kamehameha and the throw that follows it) with Vegeta's tiles the pack has no art for at all, so the
    /// game's sprite was drawn, with the pads. `key-<key>.txt` beside the picture gives the pose's pixels (see
    /// [`keyed_pixels`]) or, without that file, `key-<key>.pose` says what the pose is without them ([`keyed_pose_text`],
    /// what a release ships: the pixels are the original sprite's), whose key must be `<key>`: the added pose's key, size
    /// and silhouette are theirs (no silhouette from `.pose`, which the renderer never needs of such a pose), its palette
    /// the colours of the first of `only.palettes` with every colour the pixels use and `only.hair`, its art the picture
    /// and its id `key-<key>` (lower case) and `only.suffix`, so the renderer draws it only in Super Vegeta's colours and
    /// the pose keeps the game's sprite in any other (Vegeta's). The picture must be four art pixels to a game pixel of
    /// the box. A picture without its pixels, whose pixels are not the key's, of another size, that cannot be read, for a
    /// key (or the key of the mirror image) the index has a pose of already (the pack's own art is never replaced this
    /// way), or for colours none of the palettes has, is left out, and the log says why.
    fn add_keyed_poses(dir: &Path, only: &OnlyFor, found: &mut ArtOverride) {
        let Some(files) = override_pictures(dir) else {
            return;
        };
        for (name, path) in files {
            let Some(key) = keyed_picture(&name) else {
                continue;
            };
            let file = path.display();
            let (shape, its) = match keyed_shape_file(&path) {
                Ok(shape) => shape,
                Err(error) => {
                    found.refused.push(format!("{file} is left out: {error}"));
                    continue;
                }
            };
            let KeyedShape { key: own, width, height, used, mirror, silhouette } = shape;
            if own != key {
                found.refused.push(format!(
                    "{file} is left out: its {its} the key {own:016X}, not {key:016X}"
                ));
                continue;
            }
            if let Some(object) =
                found.objects.iter().find(|object| object.key == key || object.key == mirror)
            {
                found.refused.push(format!(
                    "{file} is left out: the index has the pose {:016X} already ({}){}",
                    object.key,
                    object.id,
                    if object.key == key { "" } else { ", the mirror image of its pixels" }
                ));
                continue;
            }
            let size = match picture_size(&path) {
                Ok(size) => size,
                Err(error) => {
                    found.refused.push(format!("{file} is left out: {error}"));
                    continue;
                }
            };
            let wanted_size =
                (width as u32 * KEYED_PICTURE_SCALE, height as u32 * KEYED_PICTURE_SCALE);
            if size != wanted_size {
                found.refused.push(format!(
                    "{file} is left out: it is {} x {} pixels, its pose is {width} x {height}, its art {} x {}",
                    size.0, size.1, wanted_size.0, wanted_size.1
                ));
                continue;
            }
            let wanted = used | only.hair;
            let Some(colours) =
                only.palettes.iter().find(|palette| palette.mask & wanted == wanted)
            else {
                found.refused.push(format!(
                    "{file} is left out: no art of {} has the colours of its pixels",
                    only.whose
                ));
                continue;
            };
            found.objects.push(crate::art_index::Object {
                key,
                id: format!("{KEYED_PICTURE}{key:016x}{}", only.suffix),
                luma: false,
                palette: Palette { mask: used, colors: colours.colors },
                width: width as u16,
                height: height as u16,
                mask: silhouette,
                art: path.to_string_lossy().into_owned(),
            });
            found.replaced.push(format!("{}/{name}", only.folder));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding::Binder;

    fn asset(id: &str, bpp: u8, keys: &[u64]) -> Asset {
        Asset {
            id: id.to_owned(),
            bpp,
            mode: Mode::Indexed,
            columns: 4,
            keys: keys.to_vec(),
            art: format!("base/{id}.png"),
        }
    }

    #[test]
    fn a_key_names_every_tile_of_its_bit_depth() {
        let index =
            Index::new(vec![asset("a", 4, &[1, 2]), asset("b", 4, &[2]), asset("c", 2, &[2])]);
        assert_eq!(
            index.candidates(4, 2),
            [TileRef { asset: 0, tile: 1 }, TileRef { asset: 1, tile: 0 }]
        );
        assert_eq!(index.candidates(2, 2), [TileRef { asset: 2, tile: 0 }]);
        assert_eq!(index.candidates(4, 9), &[] as &[TileRef]);
    }

    #[test]
    fn an_index_reads_back_as_it_was_written_and_a_damaged_one_is_refused() {
        let index =
            Index::new(vec![asset("ui/11:8036/0", 2, &[7, 8]), asset("gfx/16:8000/0", 4, &[])]);
        let bytes = index.encode();
        let back = Index::decode(&bytes).unwrap();
        assert_eq!(back.assets(), index.assets());
        assert_eq!(back.encode(), bytes, "the same index, the same bytes");

        let mut damaged = bytes.clone();
        damaged[20] ^= 1;
        assert!(matches!(Index::decode(&damaged), Err(PackError::Index("checksum"))));
        assert!(matches!(Index::decode(&bytes[..10]), Err(PackError::Index(_))));
    }

    fn write_pack(dir: &std::path::Path, converter: u32) {
        std::fs::create_dir_all(dir).unwrap();
        let manifest = Manifest {
            format: FORMAT,
            scale: 4,
            converter,
            rom_crc32: "A8CF4F2A".to_owned(),
            assets: 0,
        };
        std::fs::write(dir.join("hdpack.toml"), toml::to_string(&manifest).unwrap()).unwrap();
        std::fs::write(dir.join("index.bin"), Index::new(Vec::new()).encode()).unwrap();
    }

    #[test]
    fn the_newest_pack_of_a_rom_is_the_highest_converter_that_opens() {
        let cache = tempfile::tempdir().unwrap();
        let rom = cache.path().join("hd").join("A8CF4F2A");
        assert!(Pack::find(cache.path(), 0xA8CF_4F2A).is_none(), "no pack yet");
        write_pack(&rom.join("1"), 1);
        write_pack(&rom.join("2"), 2);
        assert_eq!(Pack::find(cache.path(), 0xA8CF_4F2A).unwrap().manifest.converter, 2);
        // A pack that does not open (damaged index) is passed over for the older one
        std::fs::write(rom.join("2").join("index.bin"), b"junk").unwrap();
        assert_eq!(Pack::find(cache.path(), 0xA8CF_4F2A).unwrap().manifest.converter, 1);
        assert!(Pack::find(cache.path(), 0x1234_5678).is_none(), "another ROM");
    }

    #[test]
    fn keys_are_fnv1a_of_the_vram_bytes_low_byte_first() {
        assert_eq!(word_key(&[0x3412]), tile_key(&[0x12, 0x34]));
        assert_eq!(tile_key(b""), 0xCBF2_9CE4_8422_2325);
    }

    /// An installed pack of one asset (`installed/0`) and one map layer, in the folder `installed`.
    fn installed_pack() -> Pack {
        Pack {
            dir: PathBuf::from("installed"),
            manifest: Manifest {
                format: FORMAT,
                scale: 4,
                converter: 1,
                rom_crc32: "A8CF4F2A".to_owned(),
                assets: 1,
            },
            index: Index::new(vec![asset("installed/0", 4, &[7])]),
            art: ArtIndex::default(),
            maps: mapart::Index {
                layer: vec![mapart::Entry {
                    id: "installed".to_owned(),
                    maps: vec![0],
                    blocks: [1, 1],
                    grid: "06:F360".to_owned(),
                    metatiles: METATILES.to_owned(),
                    source: String::new(),
                    palette: String::new(),
                    palette_variant: false,
                    tiles: "t{x}_{y}.png".to_owned(),
                }],
            },
            bg: crate::bgart::Index::default(),
            mode7: crate::mode7art::Index::default(),
            variants: Vec::new(),
            pose_art: Vec::new(),
        }
    }

    #[test]
    fn an_overlay_comes_before_the_installed_layers_and_its_files_are_read_from_its_folder() {
        let overlay = Overlay {
            dir: PathBuf::from("packs").join("room"),
            maps: vec![mapart::Entry {
                id: "room map 64".to_owned(),
                maps: vec![64],
                blocks: [8, 6],
                grid: "06:0000".to_owned(),
                metatiles: METATILES.to_owned(),
                source: String::new(),
                palette: String::new(),
                palette_variant: false,
                tiles: "art/hd/maps/64/t{x}_{y}.png".to_owned(),
            }],
            assets: vec![asset("face/room/0", 4, &[1, 2])],
            pictures: Vec::new(),
            variants: vec![PoseVariant {
                art: "battle-goku-ssj-000".to_owned(),
                when: ssd_mod::pose::Condition::Kaioken,
                margin: 6,
                file: "art/hd/variants/battle-goku-ssj-000.png".to_owned(),
            }],
        };
        let pack = installed_pack().with_overlay(overlay);
        let ids: Vec<&str> = pack.maps.layer.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(ids, ["room map 64", "installed"], "the overlay's layer is drawn first");
        let folder = std::path::absolute(PathBuf::from("packs").join("room")).unwrap();
        assert_eq!(pack.variants.len(), 1);
        assert_eq!(
            pack.variants[0].file,
            folder.join("art/hd/variants/battle-goku-ssj-000.png").to_string_lossy(),
            "a pose variant's file is absolute too"
        );
        let folder = std::path::absolute(PathBuf::from("packs").join("room")).unwrap();
        let tiles = folder.join("art/hd/maps/64/t{x}_{y}.png");
        assert_eq!(
            pack.maps.layer[0].tiles,
            tiles.to_string_lossy(),
            "absolute, the renderer joins below the pack"
        );
        assert_eq!(pack.index.assets().len(), 2, "the overlay's asset is appended");
        assert_eq!(
            pack.index.asset(0).id,
            "installed/0",
            "the installed assets keep their numbers"
        );
    }

    #[test]
    fn the_tiles_of_a_face_record_bind_to_the_face_picture_in_the_cells_they_fill() {
        // A ROM with the face palette row: object 28's pointer at 02:A38E names the row at 09:8000
        let mut rom = vec![0_u8; ssd_mod::ROM_LEN];
        let table = 0x02 * 0x8000 + (0xA38E - 0x8000);
        rom[table..table + 2].copy_from_slice(&0x8000_u16.to_le_bytes());
        let row = 0x09 * 0x8000;
        for (at, word) in [0, 0x7FFF, 0x001F, 0x03E0, 0x7C00, 0x4210].iter().enumerate() {
            rom[row + 2 * at..row + 2 * at + 2].copy_from_slice(&u16::to_le_bytes(*word));
        }
        // An HD picture of 128 x 128 in two colours, as a PNG
        let rgba: Vec<u8> = (0..128 * 128)
            .flat_map(|at| {
                let (x, y) = (at % 128, at / 128);
                if (x / 16 + y / 16) % 2 == 0 { [255, 0, 0, 255] } else { [0, 0, 255, 255] }
            })
            .collect();
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, 128, 128);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&rgba).unwrap();
        let info = |id: &str| ssd_mod::PackInfo {
            id: id.to_owned(),
            name: id.to_owned(),
            version: "1.0.0".to_owned(),
            game: format!("{:08X}", ssd_rom::crc32(&rom)),
            author: String::new(),
            description: String::new(),
        };
        let manifest = ssd_mod::Manifest {
            pack: info("face.pack"),
            depends: Vec::new(),
            data: Vec::new(),
            scene: Vec::new(),
            messages: Vec::new(),
            map: Vec::new(),
            triggers: Vec::new(),
            map_art: Vec::new(),
            face: vec![ssd_mod::FaceEntry { id: 0, hd: "art/hd/faces/a.png".to_owned() }],
            tables: Vec::new(),
            chapters: Vec::new(),
            feature: Vec::new(),
            pose_variant: Vec::new(),
        };
        let pack = ssd_mod::Pack {
            manifest,
            files: std::collections::BTreeMap::from([("art/hd/faces/a.png".to_owned(), png)]),
        };
        let image = ssd_mod::build(&rom, std::slice::from_ref(&pack)).unwrap();
        let overlay = Overlay::of_pack(&image, "face.pack", Path::new("packs/face"));
        let pack = installed_pack().with_overlay(overlay);
        // The record's 32 tiles in VRAM, one per character slot, as the game uploads them
        let mut vram = vec![0_u16; 32768];
        for (slot, tile) in image.faces[0].tiles.iter().enumerate() {
            vram[slot * 16..slot * 16 + 16].copy_from_slice(&tile_words(tile));
        }
        let mut binder = Binder::new(&pack.index);
        binder.update(&vram);
        let asset = pack.index.assets().iter().position(|a| a.id == "face/face.pack/0").unwrap();
        // Each tile of the first picture binds to the face's asset at its first place in the record (a tile that the record
        // holds twice binds to its first place, as the binder does for any repeated tile), and that place is a cell
        // of the picture
        let tiles = &image.faces[0].tiles;
        let picture = &pack.art.pictures()[0];
        assert_eq!(picture.asset, "face/face.pack/0");
        for &slot in &ssd_mod::derive::FIRST_PICTURE {
            let first = (0..tiles.len()).find(|&s| tiles[s] == tiles[slot]).unwrap();
            let found = binder.tile(4, u16::try_from(slot * 16).unwrap()).expect("the tile binds");
            assert_eq!(found.asset as usize, asset);
            assert_eq!(
                found.tile as usize, first,
                "the slot {slot} binds to its tile's first place"
            );
            let cell = picture.tiles.iter().position(|&t| u32::from(t) == found.tile).unwrap();
            assert_eq!(ssd_mod::derive::FIRST_PICTURE[cell], first);
        }
        assert_eq!(picture.name, "a");
    }

    #[test]
    fn a_pack_lists_its_own_pose_variants_and_one_that_is_refused_is_left_out() {
        let cache = tempfile::tempdir().unwrap();
        let dir = cache.path().join("pack");
        write_pack(&dir, 1);
        assert!(Pack::open(&dir).unwrap().pose_art.is_empty(), "an old pack lists none");
        std::fs::create_dir_all(dir.join("art/variants")).unwrap();
        let entry = |art: &str, when: &str, margin: u32| PoseArtEntry {
            art: art.to_owned(),
            when: when.to_owned(),
            margin,
            file: format!("art/variants/{art}.webp"),
        };
        let listed = PoseArt {
            variants: vec![
                entry("battle-goku-ssj-000", "always", 76),
                entry("battle-goku-ssj-001", "bloom", 76),
                entry("battle-goku-ssj-002", "always", 77),
                entry("battle-goku-ssj-003", "kaioken", 96),
            ],
        };
        std::fs::write(dir.join(POSE_ART), toml::to_string(&listed).unwrap()).unwrap();
        let pack = Pack::open(&dir).unwrap();
        let drawn: Vec<(&str, u32)> =
            pack.pose_art.iter().map(|variant| (variant.art.as_str(), variant.margin)).collect();
        assert_eq!(
            drawn,
            [("battle-goku-ssj-000", 19), ("battle-goku-ssj-003", 24)],
            "the margin in game pixels; an unknown condition and half a game pixel are left out"
        );
        assert_eq!(pack.pose_art[0].file, "art/variants/battle-goku-ssj-000.webp");
        assert!(pack.variants.is_empty(), "the data packs' variants are apart");
        std::fs::write(dir.join(POSE_ART), "variant = 3").unwrap();
        assert!(Pack::open(&dir).is_err(), "a list that is not a list is an error");
    }

    /// A pose named `id` of `width` x `height` game pixels whose art is `art`.
    fn pose(id: &str, key: u64, width: u16, height: u16, art: &str) -> crate::art_index::Object {
        crate::art_index::Object {
            key,
            id: id.to_owned(),
            luma: false,
            palette: Palette::default(),
            mask: Vec::new(),
            width,
            height,
            art: art.to_owned(),
        }
    }

    #[test]
    fn an_override_picture_of_the_arts_size_replaces_the_art_of_every_pose_of_its_name_and_any_other_is_left_out()
     {
        let pack_dir = tempfile::tempdir().unwrap();
        let overrides = tempfile::tempdir().unwrap();
        // The pack's art: a WebP of 8 x 8 for two poses of one name, a PNG of 8 x 12 for another
        let mut webp = Vec::new();
        crate::png::encode_webp_lossless(&mut webp, &[0; 8 * 8 * 4], 8, 8);
        std::fs::create_dir_all(pack_dir.path().join("art/object")).unwrap();
        std::fs::write(pack_dir.path().join("art/object/battle-a-000.webp"), webp).unwrap();
        crate::png::write_rgba(
            &pack_dir.path().join("art/object/battle-b-000.png"),
            8,
            12,
            &[0; 8 * 12 * 4],
        )
        .unwrap();
        let mut pack = installed_pack();
        pack.dir = pack_dir.path().to_path_buf();
        pack.art = ArtIndex::new(
            Vec::new(),
            vec![
                pose("battle-a-000", 1, 2, 2, "art/object/battle-a-000.webp"),
                pose("battle-b-000", 2, 2, 3, "art/object/battle-b-000.png"),
                pose("battle-a-000", 3, 2, 2, "art/object/battle-a-000.webp"),
            ],
        );
        let unchanged: Vec<String> =
            pack.art.objects().iter().map(|object| object.art.clone()).collect();

        // No folder: nothing is replaced, nothing is said
        let found = pack.art_override(&overrides.path().join("none"));
        assert!(found.replaced.is_empty() && found.refused.is_empty() && found.objects.is_empty());
        assert_eq!(
            pack.clone().with_art_override(&overrides.path().join("none")).art.objects(),
            pack.art.objects()
        );

        let picture = |name: &str, width: u32, height: u32| {
            crate::png::write_rgba(
                &overrides.path().join(name),
                width,
                height,
                &vec![255; (width * height * 4) as usize],
            )
            .unwrap();
        };
        picture("battle-a-000.png", 8, 8);
        picture("battle-b-000.png", 12, 8);
        picture("battle-c-000.png", 8, 8);
        std::fs::write(overrides.path().join("battle-d-000.png"), b"not a picture").unwrap();
        std::fs::write(overrides.path().join("notes.txt"), b"not a png").unwrap();
        let found = pack.art_override(overrides.path());
        let a = std::path::absolute(overrides.path().join("battle-a-000.png")).unwrap();
        let arts: Vec<&str> = found.objects.iter().map(|object| object.art.as_str()).collect();
        assert_eq!(
            arts,
            [a.to_str().unwrap(), unchanged[1].as_str(), a.to_str().unwrap()],
            "both poses of the name take the picture; the picture of another size is not drawn"
        );
        assert_eq!(found.replaced, ["battle-a-000"]);
        assert_eq!(found.refused.len(), 3, "{:?}", found.refused);
        assert!(
            found.refused[0]
                .contains("12 x 8 pixels, the art art/object/battle-b-000.png is 8 x 12")
        );
        assert!(found.refused[1].contains("no art named battle-c-000"));
        assert!(found.refused[2].contains("battle-d-000.png is left out"));

        // The pack the game draws: the same poses, the replaced ones naming the picture
        let drawn = pack.clone().with_art_override(overrides.path());
        assert_eq!(drawn.art.objects().len(), 3);
        assert_eq!(drawn.art.objects()[0].art, a.to_str().unwrap());
        assert_eq!(drawn.art.objects()[1].art, unchanged[1]);
        assert_eq!(drawn.art.objects_with(3), [2], "the poses are found by their keys as before");
    }

    #[test]
    fn piccolos_makankosappo_charge_is_drawn_from_the_override_picture_and_without_it_from_the_packs_art()
     {
        // The 50 x 43 pose of Piccolo charging the Makankosappo (`battle:22`, the `makankosappo` recipe of
        // `tools/saves/secrets.sh`) faces the player with his blue belt; the pack's art for its key is his back with a
        // red belt. The pack's fixes keep the pose, so that the launcher's `art-override\gallery-party-piccolo-u01-061.png`
        // (the size of the pack's art) draws it: with the picture the pose is drawn from it, found by its key as
        // before; without it, from the pack's art
        const KEY: u64 = 0x8369_34CF_FE0E_596F;
        const NAME: &str = "gallery-party-piccolo-u01-061";
        let art = format!("art/object/{NAME}.webp");
        let pack_dir = tempfile::tempdir().unwrap();
        let overrides = tempfile::tempdir().unwrap();
        let mut webp = Vec::new();
        crate::png::encode_webp_lossless(&mut webp, &vec![0; 200 * 172 * 4], 200, 172);
        std::fs::create_dir_all(pack_dir.path().join("art/object")).unwrap();
        std::fs::write(pack_dir.path().join(&art), webp).unwrap();
        let mut pack = installed_pack();
        pack.dir = pack_dir.path().to_path_buf();
        // The index as `Pack::open` gives it: with the pack's fixes
        pack.art = ArtIndex::new(
            Vec::new(),
            vec![
                pose(NAME, KEY, 50, 43, &art),
                pose(
                    "battle-piccolo-050",
                    0x8A84_6AA3_F449_B1BF,
                    50,
                    47,
                    "art/object/battle-piccolo-050.webp",
                ),
            ],
        )
        .with_pack_fixes();
        let drawn = |pack: &Pack| -> Vec<String> {
            pack.art
                .objects_with(KEY)
                .iter()
                .map(|&at| pack.art.objects()[at as usize].art.clone())
                .collect()
        };

        // Without the picture: the pack's art
        assert_eq!(
            drawn(&pack.clone().with_art_override(overrides.path())),
            [art.as_str()],
            "without the picture"
        );

        // With it: the picture
        let picture = overrides.path().join(format!("{NAME}.png"));
        crate::png::write_rgba(&picture, 200, 172, &vec![255; 200 * 172 * 4]).unwrap();
        let picture = std::path::absolute(&picture).unwrap();
        assert_eq!(
            drawn(&pack.clone().with_art_override(overrides.path())),
            [picture.to_str().unwrap()],
            "with the picture"
        );
        assert_eq!(
            drawn(&pack.with_art_override(&overrides.path().join("none"))),
            [art.as_str()],
            "without the folder"
        );
    }

    #[test]
    fn a_picture_of_the_variants_folder_is_an_always_variant_with_the_packs_margin_in_place_of_the_packs_own()
     {
        let pack_dir = tempfile::tempdir().unwrap();
        let overrides = tempfile::tempdir().unwrap();
        // The pack's art: a WebP of 8 x 8 for two poses of one name, a PNG of 8 x 12 for another
        let mut webp = Vec::new();
        crate::png::encode_webp_lossless(&mut webp, &[0; 8 * 8 * 4], 8, 8);
        std::fs::create_dir_all(pack_dir.path().join("art/object")).unwrap();
        std::fs::write(pack_dir.path().join("art/object/battle-a-000.webp"), webp).unwrap();
        crate::png::write_rgba(
            &pack_dir.path().join("art/object/battle-b-000.png"),
            8,
            12,
            &[0; 8 * 12 * 4],
        )
        .unwrap();
        let mut pack = installed_pack();
        pack.dir = pack_dir.path().to_path_buf();
        pack.art = ArtIndex::new(
            Vec::new(),
            vec![
                pose("battle-a-000", 1, 2, 2, "art/object/battle-a-000.webp"),
                pose("battle-b-000", 2, 2, 3, "art/object/battle-b-000.png"),
                pose("battle-a-000", 3, 2, 2, "art/object/battle-a-000.webp"),
            ],
        );
        let own = |art: &str, when: ssd_mod::pose::Condition| PoseVariant {
            art: art.to_owned(),
            when,
            margin: 19,
            file: format!("art/variants/{art}.webp"),
        };
        let (always, kaioken) =
            (ssd_mod::pose::Condition::Always, ssd_mod::pose::Condition::Kaioken);
        pack.pose_art = vec![
            own("battle-a-000", always),
            own("battle-a-000", kaioken),
            own("battle-b-000", always),
        ];
        let sub = overrides.path().join(VARIANTS_DIR);

        // No folder, or an empty one: no variant, the pack as it was
        let found = pack.art_override(&overrides.path().join("none"));
        assert!(found.variants.is_empty() && found.refused.is_empty());
        std::fs::create_dir_all(&sub).unwrap();
        for dir in [overrides.path().join("none"), overrides.path().to_path_buf()] {
            let drawn = pack.clone().with_art_override(&dir);
            assert_eq!(drawn.pose_art, pack.pose_art);
            assert_eq!(drawn.art.objects(), pack.art.objects());
        }

        let picture = |name: &str, width: u32, height: u32| {
            crate::png::write_rgba(
                &sub.join(name),
                width,
                height,
                &vec![255; (width * height * 4) as usize],
            )
            .unwrap();
        };
        // The art grown by 76 art pixels on each side: 8 + 152 = 160
        picture("battle-a-000.png", 160, 160);
        picture("battle-b-000.png", 8, 12);
        picture("battle-c-000.png", 160, 160);
        std::fs::write(sub.join("battle-d-000.png"), b"not a picture").unwrap();
        std::fs::write(sub.join("notes.txt"), b"not a png").unwrap();
        let found = pack.art_override(overrides.path());
        let a = std::path::absolute(sub.join("battle-a-000.png")).unwrap();
        assert_eq!(
            found.variants,
            [PoseVariant {
                art: "battle-a-000".to_owned(),
                when: always,
                margin: 19,
                file: a.to_string_lossy().into_owned(),
            }],
            "drawn always, the margin in game pixels (76 / 4)"
        );
        assert!(found.replaced.is_empty(), "the poses' own art is not replaced");
        assert_eq!(found.refused.len(), 3, "{:?}", found.refused);
        assert!(
            found.refused[0].contains(
                "battle-b-000.png is left out for the pose 0000000000000002: it is 8 x 12 pixels, the art \
                 art/object/battle-b-000.png is 8 x 12, a variant of it 160 x 164"
            ),
            "{:?}",
            found.refused
        );
        assert!(found.refused[1].contains("no art named battle-c-000"));
        assert!(found.refused[2].contains("battle-d-000.png is left out"));

        // The pack the game draws: the picture in place of the pack's `always` variant of its name; the Kaioken's and
        // the variant of the art whose picture was left out stay
        let drawn = pack.clone().with_art_override(overrides.path());
        let listed: Vec<(&str, ssd_mod::pose::Condition, &str)> = drawn
            .pose_art
            .iter()
            .map(|variant| (variant.art.as_str(), variant.when, variant.file.as_str()))
            .collect();
        assert_eq!(
            listed,
            [
                ("battle-a-000", kaioken, "art/variants/battle-a-000.webp"),
                ("battle-b-000", always, "art/variants/battle-b-000.webp"),
                ("battle-a-000", always, a.to_str().unwrap()),
            ]
        );
        assert_eq!(drawn.art.objects(), pack.art.objects(), "the poses are as they were");
        assert!(drawn.variants.is_empty(), "the data packs' variants are apart");
    }

    #[test]
    fn a_picture_of_super_vegeta_s_folder_adds_a_pose_in_his_colours_and_leaves_the_pose_of_its_name_as_it_was()
     {
        let pack_dir = tempfile::tempdir().unwrap();
        let overrides = tempfile::tempdir().unwrap();
        let mut webp = Vec::new();
        crate::png::encode_webp_lossless(&mut webp, &[0; 8 * 8 * 4], 8, 8);
        std::fs::create_dir_all(pack_dir.path().join("art/object")).unwrap();
        std::fs::write(pack_dir.path().join("art/object/v.webp"), webp).unwrap();
        // Vegeta's art of a pose (outline 1, colour 2, black hair 14 and 15), a greyscale shape of another, Super Vegeta's
        // art of a third (the same colours but his hair) and a piece of him with no hair
        let coloured = |id: &str, key: u64, mask: u16, hair: [u8; 3]| {
            let mut object = pose(id, key, 2, 2, "art/object/v.webp");
            object.palette.mask = mask;
            object.palette.colors[2] = [0, 255, 0];
            object.palette.colors[14] = hair;
            object.palette.colors[15] = hair;
            object
        };
        let all = 1 << 1 | 1 << 2 | 1 << 14 | 1 << 15;
        let mut shape = pose("gallery-enemy-vegeta-u15-061", 3, 2, 2, "art/object/v.webp");
        shape.luma = true;
        let mut pack = installed_pack();
        pack.dir = pack_dir.path().to_path_buf();
        pack.art = ArtIndex::new(
            Vec::new(),
            vec![
                coloured("battle-vegeta-803", 1, all, [0, 0, 0]),
                coloured("battle-super-vegeta-001", 2, 1 << 1 | 1 << 2, [0, 0, 0]),
                coloured("battle-super-vegeta-000", 4, all, [255, 255, 0]),
                shape,
            ],
        );
        let sub = overrides.path().join(SUPER_VEGETA_DIR);
        std::fs::create_dir_all(&sub).unwrap();
        let picture = |name: &str, width: u32, height: u32| {
            crate::png::write_rgba(
                &sub.join(name),
                width,
                height,
                &vec![255; (width * height * 4) as usize],
            )
            .unwrap();
        };
        // An empty folder: the pack is as it was
        assert_eq!(
            pack.clone().with_art_override(overrides.path()).art.objects(),
            pack.art.objects()
        );
        picture("battle-vegeta-803.png", 8, 8);
        picture("gallery-enemy-vegeta-u15-061.png", 8, 8);
        picture("battle-vegeta-999.png", 8, 8);
        picture("battle-super-vegeta-001.png", 12, 8);
        let found = pack.art_override(overrides.path());
        assert_eq!(
            found.objects[..4],
            pack.art.objects()[..],
            "the poses of the names are left as they were"
        );
        assert_eq!(found.objects.len(), 5, "one pose added: {:?}", found.refused);
        let added = &found.objects[4];
        let picture_path = std::path::absolute(sub.join("battle-vegeta-803.png")).unwrap();
        assert_eq!(added.id, "battle-vegeta-803@super-vegeta");
        assert!(added.super_vegeta_only() && !found.objects[0].super_vegeta_only());
        assert_eq!(
            (added.key, added.width, added.height),
            (1, 2, 2),
            "the key and size of the pose of its name"
        );
        assert_eq!(added.art, picture_path.to_str().unwrap());
        assert_eq!(added.palette.mask, all, "the colours the pose uses");
        assert_eq!(added.palette.colors[14], [255, 255, 0], "in Super Vegeta's colours: his hair");
        assert_eq!(added.palette.colors[2], [0, 255, 0]);
        assert_eq!(found.replaced, ["super-vegeta/battle-vegeta-803"]);
        assert_eq!(found.refused.len(), 3, "{:?}", found.refused);
        assert!(found.refused[0].contains("12 x 8 pixels"), "{:?}", found.refused);
        assert!(
            found.refused[2]
                .contains("no art of Super Vegeta has the colours of gallery-enemy-vegeta-u15-061")
        );
        assert!(found.refused[1].contains("no art named battle-vegeta-999"));
        // The root folder's pictures still replace the art of their name, and the folder of Super Vegeta's is no picture
        let overridden = pack.clone().with_art_override(overrides.path());
        assert_eq!(overridden.art.objects().len(), 5);
        assert_eq!(
            overridden.art.objects_with(1),
            [0, 4],
            "found by the key of the pose of its name"
        );
    }

    #[test]
    fn a_picture_of_super_vegeta_s_folder_for_an_effect_s_pose_takes_his_palette_of_game_over() {
        // Vegeta flying over GAME OVER is drawn from `gallery-fx-1118`, an effect's art (palette mask 0, recoloured from the
        // sprite's own colours) with the pads. `art-override\super-vegeta\gallery-fx-1118.png` adds a pose of it in his
        // palette of that scene (`crate::art::game_over_super_vegeta`), whatever colours his battle art has; a greyscale
        // shape, and an effect's pose in the normal Goku's folder, are left out as before
        let pack_dir = tempfile::tempdir().unwrap();
        let mut webp = Vec::new();
        crate::png::encode_webp_lossless(&mut webp, &[0; 8 * 8 * 4], 8, 8);
        std::fs::create_dir_all(pack_dir.path().join("art/object")).unwrap();
        std::fs::write(pack_dir.path().join("art/object/v.webp"), webp).unwrap();
        let coloured = |id: &str, key: u64| {
            let mut object = pose(id, key, 2, 2, "art/object/v.webp");
            object.palette.mask = 1 << 1 | 1 << 2 | 1 << 14 | 1 << 15;
            object.palette.colors[14] = [255, 255, 0];
            object.palette.colors[15] = [255, 115, 33];
            object
        };
        let mut shape = pose("gallery-fx-0002", 8, 2, 2, "art/object/v.webp");
        shape.luma = true;
        let mut pack = installed_pack();
        pack.dir = pack_dir.path().to_path_buf();
        pack.art = ArtIndex::new(
            Vec::new(),
            vec![
                coloured("battle-super-vegeta-000", 2),
                coloured("battle-goku-000", 9),
                pose("gallery-fx-1118", 7, 2, 2, "art/object/v.webp"),
                shape,
            ],
        );
        let overrides = tempfile::tempdir().unwrap();
        for (folder, name) in [
            (SUPER_VEGETA_DIR, "gallery-fx-1118.png"),
            (SUPER_VEGETA_DIR, "gallery-fx-0002.png"),
            (NORMAL_GOKU_DIR, "gallery-fx-1118.png"),
        ] {
            let sub = overrides.path().join(folder);
            std::fs::create_dir_all(&sub).unwrap();
            crate::png::write_rgba(&sub.join(name), 8, 8, &[255; 8 * 8 * 4]).unwrap();
        }
        let found = pack.art_override(overrides.path());
        assert_eq!(found.objects[..4], pack.art.objects()[..], "the effect's own pose as it was");
        assert_eq!(found.objects.len(), 5, "one pose added: {:?}", found.refused);
        let added = &found.objects[4];
        assert_eq!(added.id, "gallery-fx-1118@super-vegeta");
        assert!(added.super_vegeta_only());
        assert_eq!(added.key, 7, "the key of the effect's pose");
        assert_eq!(added.palette, crate::art::game_over_super_vegeta(), "his palette of GAME OVER");
        assert_eq!(added.palette.mask, 0xFFFE, "every colour");
        assert_eq!(added.palette.colors[4], [255, 239, 0], "his hair there: ffef00");
        assert_eq!(found.replaced, ["super-vegeta/gallery-fx-1118"]);
        assert_eq!(found.refused.len(), 2, "{:?}", found.refused);
        assert!(
            found.refused[0].contains("no art of Super Vegeta has the colours of gallery-fx-0002"),
            "a greyscale shape: {:?}",
            found.refused
        );
        assert!(
            found.refused[1]
                .contains("no art of the normal Goku has the colours of gallery-fx-1118"),
            "the normal Goku's folder takes no effect's pose: {:?}",
            found.refused
        );
    }

    #[test]
    fn a_keyed_picture_of_super_vegeta_s_folder_adds_a_pose_of_no_art_and_never_one_the_index_has()
    {
        // The game draws Super Vegeta's back to Goku's Kamehameha (and the throw after it) with Vegeta's tiles the pack
        // has no art for at all, so the game's sprite showed the pads. `key-<key>.png` with its pixels `key-<key>.txt`
        // adds a pose of that key in his colours; one for a key the index has already (or its mirror image's), one of
        // another size, one whose pixels are not of its key and one without pixels are left out, and so is any in the
        // normal Goku's folder
        let key_of = |text: &str| {
            let (arr, w, h) = keyed_pixels(text).unwrap();
            crate::art::poses::fighter_key(&arr, w, h).unwrap()
        };
        let mirror_of = |text: &str| {
            let flipped: String =
                text.lines().map(|line| line.chars().rev().collect::<String>() + "\n").collect();
            key_of(&flipped)
        };
        let (new, taken, mirrored, sized) = ("21\n0e\n", "12\n2f\n", "2e\n01\n", "1\n2\n");
        assert_ne!(key_of(mirrored), mirror_of(mirrored), "an asymmetric pose");
        let pack_dir = tempfile::tempdir().unwrap();
        let overrides = tempfile::tempdir().unwrap();
        let mut super_vegeta = pose("battle-super-vegeta-000", 4, 2, 2, "art/object/v.webp");
        super_vegeta.palette.mask = 1 << 1 | 1 << 2 | 1 << 14 | 1 << 15;
        super_vegeta.palette.colors[2] = [0, 255, 0];
        super_vegeta.palette.colors[14] = [255, 255, 0];
        super_vegeta.palette.colors[15] = [255, 115, 33];
        let mut pack = installed_pack();
        pack.dir = pack_dir.path().to_path_buf();
        pack.art = ArtIndex::new(
            Vec::new(),
            vec![
                super_vegeta,
                pose("battle-vegeta-034", key_of(taken), 2, 2, "art/object/v.webp"),
                pose("battle-vegeta-035", mirror_of(mirrored), 2, 2, "art/object/v.webp"),
            ],
        );
        let keyed = |folder: &str, key: u64, pixels: Option<&str>, size: (u32, u32)| {
            let sub = overrides.path().join(folder);
            std::fs::create_dir_all(&sub).unwrap();
            let name = format!("key-{key:016x}");
            crate::png::write_rgba(
                &sub.join(format!("{name}.png")),
                size.0,
                size.1,
                &vec![255; (size.0 * size.1 * 4) as usize],
            )
            .unwrap();
            if let Some(pixels) = pixels {
                std::fs::write(sub.join(format!("{name}.txt")), pixels).unwrap();
            }
        };
        keyed(SUPER_VEGETA_DIR, key_of(new), Some(new), (8, 8));
        keyed(SUPER_VEGETA_DIR, key_of(taken), Some(taken), (8, 8));
        keyed(SUPER_VEGETA_DIR, key_of(mirrored), Some(mirrored), (8, 8));
        keyed(SUPER_VEGETA_DIR, key_of(sized), Some(sized), (8, 8));
        keyed(SUPER_VEGETA_DIR, 0x0123_4567_89AB_CDEF, Some(new), (8, 8));
        keyed(SUPER_VEGETA_DIR, 0xFEDC_BA98_7654_3210, None, (8, 8));
        keyed(NORMAL_GOKU_DIR, key_of(new) ^ 1, Some(new), (8, 8));

        let found = pack.art_override(overrides.path());
        assert_eq!(found.objects[..3], pack.art.objects()[..], "the index's poses as they were");
        assert_eq!(found.objects.len(), 4, "one pose added: {:?}", found.refused);
        let added = &found.objects[3];
        let picture = std::path::absolute(
            overrides.path().join(format!("{SUPER_VEGETA_DIR}/key-{:016x}.png", key_of(new))),
        )
        .unwrap();
        assert_eq!(added.id, format!("key-{:016x}@super-vegeta", key_of(new)));
        assert!(added.super_vegeta_only(), "drawn in Super Vegeta's colours only");
        assert_eq!((added.key, added.width, added.height), (key_of(new), 2, 2));
        assert_eq!(added.mask, [0b1011], "opaque but the pixel (0, 1), bit 2");
        assert!(!added.luma);
        assert_eq!(added.palette.mask, 1 << 1 | 1 << 2 | 1 << 14, "the colours the pixels use");
        assert_eq!(added.palette.colors[14], [255, 255, 0], "in Super Vegeta's colours");
        assert_eq!(added.palette.colors[2], [0, 255, 0]);
        assert_eq!(added.art, picture.to_str().unwrap());
        assert_eq!(found.replaced, [format!("super-vegeta/key-{:016x}", key_of(new))]);
        let refused = |text: &str| found.refused.iter().any(|reason| reason.contains(text));
        assert_eq!(found.refused.len(), 6, "{:?}", found.refused);
        assert!(
            refused(&format!(
                "the index has the pose {:016X} already (battle-vegeta-034)",
                key_of(taken)
            )),
            "the pack's own art is never replaced: {:?}",
            found.refused
        );
        assert!(
            refused(&format!(
                "the index has the pose {:016X} already (battle-vegeta-035), the mirror image",
                mirror_of(mirrored)
            )),
            "{:?}",
            found.refused
        );
        assert!(
            refused("it is 8 x 8 pixels, its pose is 1 x 2, its art 4 x 8"),
            "{:?}",
            found.refused
        );
        assert!(
            refused(&format!("have the key {:016X}, not 0123456789ABCDEF", key_of(new))),
            "{:?}",
            found.refused
        );
        assert!(refused("key-fedcba9876543210.txt"), "no pixels: {:?}", found.refused);
        assert!(
            refused(&format!("the pack has no art named key-{:016x}", key_of(new) ^ 1)),
            "the normal Goku's folder takes no keyed picture: {:?}",
            found.refused
        );
        // The pack the game draws finds the pose by its key
        let overridden = pack.clone().with_art_override(overrides.path());
        assert_eq!(overridden.art.objects_with(key_of(new)), [3]);
        assert_eq!(overridden.art.objects_with(key_of(taken)), [1], "the pack's art alone");
    }

    #[test]
    fn a_keyed_picture_with_its_pose_and_no_pixels_is_the_pose_its_pixels_give_but_the_silhouette()
    {
        // A release ships `key-<key>.pose` (what the pose is, made from the pixels by `keyed_pose_text`) in place of
        // the original sprite's pixels `key-<key>.txt`. Without the pixels the same pose is added (key, size, palette,
        // art, id); only the silhouette is left empty, which the renderer never reads of a pose found by its key alone.
        // The pose's checks hold as with the pixels: another key, the index's pose or its mirror image, another size
        let key_of = |text: &str| {
            let (arr, w, h) = keyed_pixels(text).unwrap();
            crate::art::poses::fighter_key(&arr, w, h).unwrap()
        };
        let mirror_of = |text: &str| {
            let flipped: String =
                text.lines().map(|line| line.chars().rev().collect::<String>() + "\n").collect();
            key_of(&flipped)
        };
        let (new, taken, mirrored, sized) = ("21\n0e\n", "12\n2f\n", "2e\n01\n", "1\n2\n");
        let pack_dir = tempfile::tempdir().unwrap();
        let mut super_vegeta = pose("battle-super-vegeta-000", 4, 2, 2, "art/object/v.webp");
        super_vegeta.palette.mask = 1 << 1 | 1 << 2 | 1 << 14 | 1 << 15;
        super_vegeta.palette.colors[2] = [0, 255, 0];
        super_vegeta.palette.colors[14] = [255, 255, 0];
        super_vegeta.palette.colors[15] = [255, 115, 33];
        let mut pack = installed_pack();
        pack.dir = pack_dir.path().to_path_buf();
        pack.art = ArtIndex::new(
            Vec::new(),
            vec![
                super_vegeta,
                pose("battle-vegeta-034", key_of(taken), 2, 2, "art/object/v.webp"),
                pose("battle-vegeta-035", mirror_of(mirrored), 2, 2, "art/object/v.webp"),
            ],
        );
        // the same pictures twice: with their pixels, and with their poses alone
        let folders: Vec<tempfile::TempDir> =
            (0..2).map(|_| tempfile::tempdir().unwrap()).collect();
        let keyed = |name_key: u64, pixels: &str, size: (u32, u32)| {
            for (n, folder) in folders.iter().enumerate() {
                let sub = folder.path().join(SUPER_VEGETA_DIR);
                std::fs::create_dir_all(&sub).unwrap();
                let name = format!("key-{name_key:016x}");
                crate::png::write_rgba(
                    &sub.join(format!("{name}.png")),
                    size.0,
                    size.1,
                    &vec![255; (size.0 * size.1 * 4) as usize],
                )
                .unwrap();
                let (file, text) = if n == 0 {
                    (format!("{name}.txt"), pixels.to_owned())
                } else {
                    (format!("{name}.{KEYED_POSE_EXTENSION}"), keyed_pose_text(pixels).unwrap())
                };
                std::fs::write(sub.join(file), text).unwrap();
            }
        };
        keyed(key_of(new), new, (8, 8));
        keyed(key_of(taken), taken, (8, 8));
        keyed(key_of(mirrored), mirrored, (8, 8));
        keyed(key_of(sized), sized, (8, 8));
        keyed(0x0123_4567_89AB_CDEF, new, (8, 8));
        let with_pixels = pack.art_override(folders[0].path());
        let with_poses = pack.art_override(folders[1].path());
        assert_eq!(with_pixels.objects.len(), 4, "{:?}", with_pixels.refused);
        assert_eq!(with_poses.objects.len(), 4, "{:?}", with_poses.refused);
        let (a, b) = (&with_pixels.objects[3], &with_poses.objects[3]);
        assert_eq!(a.mask, [0b1011], "the pixels' silhouette");
        assert!(b.mask.is_empty(), "no silhouette without the pixels");
        let art = |object: &crate::art_index::Object, folder: &tempfile::TempDir| {
            object
                .art
                .strip_prefix(&*std::path::absolute(folder.path()).unwrap().to_string_lossy())
                .map(str::to_owned)
        };
        assert_eq!(art(a, &folders[0]), art(b, &folders[1]));
        assert!(art(b, &folders[1]).is_some(), "{}", b.art);
        assert_eq!(
            (a.key, &a.id, a.luma, a.palette, a.width, a.height),
            (b.key, &b.id, b.luma, b.palette, b.width, b.height),
            "the same pose but its silhouette"
        );
        assert_eq!(with_pixels.replaced, with_poses.replaced);
        let why = |found: &ArtOverride| -> Vec<String> {
            found
                .refused
                .iter()
                .map(|reason| reason.split_once(" is left out: ").unwrap().1.to_owned())
                .map(|reason| reason.split(" the key ").last().unwrap().to_owned())
                .collect()
        };
        assert_eq!(why(&with_pixels).len(), 4, "{:?}", with_pixels.refused);
        assert_eq!(
            why(&with_pixels),
            why(&with_poses),
            "refused alike: {:?} {:?}",
            with_pixels.refused,
            with_poses.refused
        );
        assert!(
            with_poses.refused.iter().any(|reason| reason.contains(&format!(
                ".pose has the key {:016X}, not 0123456789ABCDEF",
                key_of(new)
            ))),
            "{:?}",
            with_poses.refused
        );
        // The pack the game draws finds the pose by its key, with the poses alone too
        let overridden = pack.clone().with_art_override(folders[1].path());
        assert_eq!(overridden.art.objects_with(key_of(new)), [3]);
        // pixels win where both are there
        let sub = folders[1].path().join(SUPER_VEGETA_DIR);
        std::fs::write(sub.join(format!("key-{:016x}.txt", key_of(new))), new).unwrap();
        assert_eq!(pack.art_override(folders[1].path()).objects[3].mask, [0b1011]);
    }

    #[test]
    fn a_keyed_pose_is_one_line_of_key_size_colours_and_mirror() {
        let text = keyed_pose_text("21\n0e\n").unwrap();
        let shape = keyed_pose(&text).unwrap();
        let (arr, w, h) = keyed_pixels("21\n0e\n").unwrap();
        let of_pixels = keyed_shape(&arr, w, h).unwrap();
        assert_eq!(shape, KeyedShape { silhouette: Vec::new(), ..of_pixels.clone() });
        assert_eq!(
            text,
            format!(
                "key={:016x} size=2x2 colours=4006 mirror={:016x}\n",
                of_pixels.key, of_pixels.mirror
            )
        );
        assert_eq!(keyed_pose(&format!("\u{feff}  {}\r\n\n", text.trim())), Ok(shape.clone()));
        let refused = |text: &str, words: &str| {
            let error = keyed_pose(text).unwrap_err();
            assert!(error.contains(words), "{text:?}: {error}");
        };
        refused("", "not one line");
        refused(&format!("{} extra", text.trim()), "not one line");
        refused(&text.replace("key=", "kee="), "is not key=");
        refused(&text.replace("size=2x2", "size=2x0"), "is not size=");
        refused(&text.replace("size=2x2", "size=256x2"), "is not size=");
        refused(&text.replace("colours=4006", "colours=4007"), "indexes 1 to 15");
        refused(&text.replace("colours=4006", "colours=0000"), "indexes 1 to 15");
        refused(&text.replace("colours=4006", "colours=406"), "4 hexadecimal digits");
        refused(&format!("{}0", text.trim()), "16 hexadecimal digits");
        assert!(keyed_pose_text("0g\n").unwrap_err().contains("'g'"));
    }

    #[test]
    fn the_pixels_of_a_keyed_picture_are_a_hexadecimal_digit_for_each_pixel() {
        assert_eq!(keyed_pixels("0a1\r\nF20  \n\n"), Ok((vec![0, 10, 1, 15, 2, 0], 3, 2)));
        assert!(keyed_pixels("\n\n").unwrap_err().contains("no pixel"));
        assert!(keyed_pixels("012\n01\n").unwrap_err().contains("line 2 is 2 pixels"));
        assert!(keyed_pixels("012\n0g1\n").unwrap_err().contains("'g'"));
        assert_eq!(keyed_picture("key-9FB61F64D7D15BE9"), Some(0x9FB6_1F64_D7D1_5BE9));
        assert_eq!(keyed_picture("key-9fb61f64d7d15be9"), Some(0x9FB6_1F64_D7D1_5BE9));
        assert_eq!(keyed_picture("key-9fb61f64d7d15be"), None);
        assert_eq!(keyed_picture("battle-vegeta-001"), None);
    }

    #[test]
    fn pixels_of_a_keyed_picture_wider_than_a_pose_are_refused() {
        // A pose's box is at most 255 pixels wide: a line of 256 is refused before the array is made
        let refused = keyed_pixels(&("1".repeat(256) + "\n1\n")).unwrap_err();
        assert!(refused.contains("line 1 is 256 pixels"), "{refused}");
        assert_eq!(keyed_pixels(&"1".repeat(255)).map(|(_, w, h)| (w, h)), Ok((255, 1)));
    }

    #[test]
    fn pixels_of_a_keyed_picture_taller_than_a_pose_are_refused() {
        // A pose's box is at most 255 pixels tall: 256 lines of pixels are refused before the array is made (blank lines
        // are not pixels)
        let refused = keyed_pixels(&"1\n".repeat(256)).unwrap_err();
        assert!(refused.contains("more than 255 lines"), "{refused}");
        let tall = "1\n\n".repeat(255);
        assert_eq!(keyed_pixels(&tall).map(|(_, w, h)| (w, h)), Ok((1, 255)));
    }

    #[test]
    fn a_keyed_picture_s_pixels_file_larger_than_a_mebibyte_is_refused_unread() {
        // The pixels of a 16 x 16 pose, then blank lines (left out of the pixels) to one byte past 1 MiB: refused for its
        // size, though its pixels alone would be a pose; at 1 MiB they are read
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key-0000000000000000.txt");
        let pixels = ("2".repeat(16) + "\n").repeat(16);
        let padded = |bytes: u64| {
            pixels.clone() + &"\n".repeat(usize::try_from(bytes).unwrap() - pixels.len())
        };
        std::fs::write(&path, padded(KEYED_PIXELS_MAX_BYTES)).unwrap();
        assert_eq!(keyed_pixels_file(&path).map(|(_, w, h)| (w, h)), Ok((16, 16)));
        std::fs::write(&path, padded(KEYED_PIXELS_MAX_BYTES + 1)).unwrap();
        let refused = keyed_pixels_file(&path).unwrap_err();
        assert!(refused.contains("1048577 bytes, more than 1048576"), "{refused}");
    }

    #[test]
    fn a_keyed_picture_s_pixels_file_saved_with_a_byte_order_mark_is_read_as_without_it() {
        // Notepad saves UTF-8 with a byte order mark: the pixels and the key are those of the same file without it
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key-0000000000000000.txt");
        std::fs::write(&path, "\u{feff}21\r\n0e\r\n").unwrap();
        let (arr, w, h) = keyed_pixels_file(&path).unwrap();
        assert_eq!((arr.as_slice(), w, h), ([2, 1, 0, 14].as_slice(), 2, 2));
        let (plain, _, _) = keyed_pixels("21\n0e\n").unwrap();
        assert_eq!(
            crate::art::poses::fighter_key(&arr, w, h).unwrap(),
            crate::art::poses::fighter_key(&plain, 2, 2).unwrap(),
            "the key of the pixels without the mark"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_keyed_picture_s_pixels_file_that_is_not_a_regular_file_is_refused_unopened() {
        // A FIFO named `key-<key>.txt`: opening it would wait for a writer, and the pack would never load. It is refused
        // without opening it (were it opened, the thread would still be waiting after five seconds), and so is a folder
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key-0000000000000000.txt");
        let made = std::process::Command::new("mkfifo").arg(&path).status().unwrap();
        assert!(made.success(), "mkfifo");
        let (sent, got) = std::sync::mpsc::channel();
        let fifo = path.clone();
        std::thread::spawn(move || sent.send(keyed_pixels_file(&fifo)).unwrap());
        let refused = got
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the FIFO was opened: it waits for a writer")
            .unwrap_err();
        assert!(refused.contains("not a regular file"), "{refused}");
        let folder = dir.path().join("key-0000000000000001.txt");
        std::fs::create_dir(&folder).unwrap();
        let refused = keyed_pixels_file(&folder).unwrap_err();
        assert!(refused.contains("not a regular file"), "{refused}");
    }

    #[test]
    fn a_picture_of_goku_s_folder_adds_a_pose_in_the_normal_goku_s_colours_and_leaves_the_super_saiyan_s_as_it_was()
     {
        // Patch 0051: `art-override\goku\battle-goku-ssj-020.png` adds a pose beside the Super Saiyan art of its name, in
        // the colours of the normal Goku's art (a battle pose's before another's); one whose pose uses a colour no art of
        // the normal Goku has is left out
        let pack_dir = tempfile::tempdir().unwrap();
        let overrides = tempfile::tempdir().unwrap();
        let mut webp = Vec::new();
        crate::png::encode_webp_lossless(&mut webp, &[0; 8 * 8 * 4], 8, 8);
        std::fs::create_dir_all(pack_dir.path().join("art/object")).unwrap();
        std::fs::write(pack_dir.path().join("art/object/g.webp"), webp).unwrap();
        let coloured = |id: &str, key: u64, mask: u16, two: [u8; 3], hair: [u8; 3]| {
            let mut object = pose(id, key, 2, 2, "art/object/g.webp");
            object.palette.mask = mask;
            object.palette.colors[2] = two;
            object.palette.colors[14] = hair;
            object.palette.colors[15] = hair;
            object
        };
        let all = 1 << 1 | 1 << 2 | 1 << 14 | 1 << 15;
        let mut pack = installed_pack();
        pack.dir = pack_dir.path().to_path_buf();
        pack.art = ArtIndex::new(
            Vec::new(),
            vec![
                coloured("battle-goku-ssj-020", 1, all, [0, 255, 0], [255, 255, 0]),
                coloured("gallery-party-goku-u00-001", 5, all, [0, 0, 255], [0, 0, 0]),
                coloured("battle-goku-000", 2, all, [0, 255, 0], [0, 0, 0]),
                coloured("battle-goku-ssj-019", 3, all | 1 << 3, [0, 255, 0], [255, 255, 0]),
            ],
        );
        let sub = overrides.path().join(NORMAL_GOKU_DIR);
        std::fs::create_dir_all(&sub).unwrap();
        for name in ["battle-goku-ssj-020.png", "battle-goku-ssj-019.png"] {
            crate::png::write_rgba(&sub.join(name), 8, 8, &[255; 8 * 8 * 4]).unwrap();
        }
        let found = pack.art_override(overrides.path());
        assert_eq!(
            found.objects[..4],
            pack.art.objects()[..],
            "the poses of the names are left as they were"
        );
        assert_eq!(found.objects.len(), 5, "one pose added: {:?}", found.refused);
        let added = &found.objects[4];
        assert_eq!(added.id, "battle-goku-ssj-020@goku");
        assert!(
            added.normal_goku_only()
                && !added.super_vegeta_only()
                && !found.objects[0].override_only()
        );
        assert_eq!(
            (added.key, added.width, added.height),
            (1, 2, 2),
            "the key and size of the pose of its name"
        );
        let picture_path = std::path::absolute(sub.join("battle-goku-ssj-020.png")).unwrap();
        assert_eq!(added.art, picture_path.to_str().unwrap());
        assert_eq!(added.palette.mask, all, "the colours the pose uses");
        assert_eq!(added.palette.colors[14], [0, 0, 0], "in the normal Goku's colours: black hair");
        assert_eq!(
            added.palette.colors[2],
            [0, 255, 0],
            "a battle pose's colours before another's"
        );
        assert_eq!(found.replaced, ["goku/battle-goku-ssj-020"]);
        assert_eq!(found.refused.len(), 1, "{:?}", found.refused);
        assert!(
            found.refused[0]
                .contains("no art of the normal Goku has the colours of battle-goku-ssj-019"),
            "{:?}",
            found.refused
        );
        let overridden = pack.clone().with_art_override(overrides.path());
        assert_eq!(
            overridden.art.objects_with(1),
            [0, 4],
            "found by the key of the pose of its name"
        );
    }

    #[test]
    fn a_picture_of_the_ending_goku_folder_adds_the_same_pose_of_goku_s_art_for_the_ending_only() {
        // `art-override\ending-goku\battle-goku-025.png` adds a pose beside the art of its name with its key, size and
        // palette; a picture for art not of Goku and one of another size are left out
        let pack_dir = tempfile::tempdir().unwrap();
        let overrides = tempfile::tempdir().unwrap();
        let mut webp = Vec::new();
        crate::png::encode_webp_lossless(&mut webp, &[0; 8 * 8 * 4], 8, 8);
        std::fs::create_dir_all(pack_dir.path().join("art/object")).unwrap();
        std::fs::write(pack_dir.path().join("art/object/g.webp"), webp).unwrap();
        let mut goku = pose("battle-goku-025", 1, 2, 2, "art/object/g.webp");
        goku.palette.mask = 1 << 1 | 1 << 2;
        goku.palette.colors[2] = [0, 255, 0];
        let mut pack = installed_pack();
        pack.dir = pack_dir.path().to_path_buf();
        pack.art = ArtIndex::new(
            Vec::new(),
            vec![
                goku,
                pose("battle-gohan-012", 2, 2, 2, "art/object/g.webp"),
                pose("battle-goku-024", 3, 2, 2, "art/object/g.webp"),
            ],
        );
        let sub = overrides.path().join(ENDING_GOKU_DIR);
        std::fs::create_dir_all(&sub).unwrap();
        for name in ["battle-goku-025.png", "battle-gohan-012.png"] {
            crate::png::write_rgba(&sub.join(name), 8, 8, &[255; 8 * 8 * 4]).unwrap();
        }
        crate::png::write_rgba(&sub.join("battle-goku-024.png"), 4, 8, &[255; 4 * 8 * 4]).unwrap();
        let found = pack.art_override(overrides.path());
        assert_eq!(
            found.objects[..3],
            pack.art.objects()[..],
            "the poses of the names are left as they were"
        );
        assert_eq!(found.objects.len(), 4, "one pose added: {:?}", found.refused);
        let added = &found.objects[3];
        assert_eq!(added.id, "battle-goku-025@ending-goku");
        assert!(added.ending_goku_only() && added.override_only() && !added.normal_goku_only());
        assert!(!found.objects[0].override_only());
        let picture_path = std::path::absolute(sub.join("battle-goku-025.png")).unwrap();
        assert_eq!(
            *added,
            crate::art_index::Object {
                id: "battle-goku-025@ending-goku".to_owned(),
                art: picture_path.to_str().unwrap().to_owned(),
                ..pack.art.objects()[0].clone()
            },
            "the pose of its name, its palette too"
        );
        assert_eq!(found.replaced, ["ending-goku/battle-goku-025"]);
        assert_eq!(found.refused.len(), 2, "{:?}", found.refused);
        assert!(
            found
                .refused
                .iter()
                .any(|reason| reason.contains("battle-gohan-012 is not Goku's art"))
        );
        assert!(found.refused.iter().any(|reason| reason.contains("it is 4 x 8 pixels")));
    }
}
