//! The pack's art read from its files at the picture's scale, and the thread that reads it while the
//! frames draw.
//!
//! The art is read where the frame wants it (synchronously, the default: the tests, the tools and the
//! regression pictures rely on every frame drawing all its art), or, once [`ArtState::set_async`] is
//! on, by a thread of its own (a [`Reader`]): a frame asks for the art its caches lack and draws
//! without it, as it draws art that cannot be read (the original's pixels), and the next frame after
//! it is read draws it. The pixels of the art are the same either way.

use super::reader::Reader;
use super::{ArtState, Image, LoadKind, MIRROR, mirror, real, resample};
use crate::art_index::Object;
use crate::bgart::{self, BgArt, BgImage};
use crate::mode7art::PlaneArt;
use crate::pack::{Pack, PoseVariant};
use rayon::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The art read by the frames themselves (not by the reading thread), by any renderer of the process
/// (see [`art_loads_on_draw`]).
static LOADS_ON_DRAW: AtomicU64 = AtomicU64::new(0);

/// How many art files (a background layer, a picture, an object, a pose variant, a Mode 7 plane) the
/// renderers of the process have read on the thread that draws, since it started: none once the art
/// is read by its own thread ([`crate::Renderer::set_art_async`]). The field maps' tiles are counted
/// apart ([`crate::FieldMapArt::read`]).
#[must_use]
pub fn art_loads_on_draw() -> u64 {
    LOADS_ON_DRAW.load(Ordering::Relaxed)
}

/// Whether the environment asks for every frame drawn whole (`SSD_HD_SERIAL`, as the wide renderer
/// reads it): the art is then read by the frames, whatever [`ArtState::set_async`] says.
fn serial_env() -> bool {
    // (read once: the environment does not change while a run draws)
    static SERIAL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SERIAL.get_or_init(|| std::env::var_os("SSD_HD_SERIAL").is_some())
}

/// An art file to read: a background layer, a picture or an object (with [`MIRROR`]) by its index,
/// a pose variant by its number (the data packs' first, then the pack's own, see
/// [`ArtState::pose_variant`]) and whether it is mirrored, a Mode 7 plane by its index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ArtKey {
    Bg(u32),
    Picture(u32),
    Object(u32),
    Variant(usize, bool),
    Plane(u32),
}

/// What reading an [`ArtKey`] needs besides the pack: the picture's scale, whether a background
/// layer has large characters, and the object a pose variant replaces.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArtJob {
    pub(crate) scale: usize,
    pub(crate) large: bool,
    pub(crate) object: u32,
}

/// The art read for an [`ArtKey`] (`None` inside: a file that cannot be read or is not its size).
pub(crate) enum Loaded {
    Image(Option<Arc<Image>>),
    Bg(Option<Arc<BgImage>>),
    Plane(Option<Arc<PlaneArt>>),
}

/// Pose variant `n` of the pack: the data packs' first, then the pack's own.
fn variant_of(pack: &Pack, n: usize) -> Option<&PoseVariant> {
    pack.variants.get(n).or_else(|| pack.pose_art.get(n.checked_sub(pack.variants.len())?))
}

/// Reads `key` of `pack` for `job`, adding the bytes of the files read to `bytes`. `parallel` reads the tiles of a
/// Mode 7 plane on the thread pool.
pub(crate) fn read(
    pack: &Pack,
    key: ArtKey,
    job: &ArtJob,
    parallel: bool,
    bytes: &AtomicU64,
) -> Loaded {
    match key {
        ArtKey::Bg(at) => Loaded::Bg(bg(pack, at, job.large, job.scale, bytes)),
        ArtKey::Picture(at) => Loaded::Image(picture(pack, at, job.scale, bytes)),
        ArtKey::Object(at) => Loaded::Image(object(pack, at, job.scale, bytes)),
        ArtKey::Variant(n, mirrored) => Loaded::Image(variant_of(pack, n).and_then(|variant| {
            let object = &pack.art.objects()[real(job.object)];
            pose_variant(pack, variant, object, mirrored, job.scale, bytes)
        })),
        ArtKey::Plane(at) => Loaded::Plane(plane(pack, at, parallel, bytes)),
    }
}

/// The kind of art `key` is, as a picture's cost counts it (see [`crate::DrawCost`]).
fn kind_of(key: ArtKey) -> LoadKind {
    match key {
        ArtKey::Bg(_) => LoadKind::Bg,
        ArtKey::Picture(_) => LoadKind::Picture,
        ArtKey::Object(_) => LoadKind::Object,
        ArtKey::Variant(..) => LoadKind::Variant,
        ArtKey::Plane(_) => LoadKind::Plane,
    }
}

/// The file at `path`, its bytes added to `bytes`.
fn read_file(path: &std::path::Path, bytes: &AtomicU64) -> std::io::Result<Vec<u8>> {
    let read = std::fs::read(path)?;
    bytes.fetch_add(read.len() as u64, Ordering::Relaxed);
    Ok(read)
}

/// The art of the Mode 7 plane of entry `at`, read whole (the art's own size).
pub(crate) fn plane(
    pack: &Pack,
    at: u32,
    parallel: bool,
    bytes: &AtomicU64,
) -> Option<Arc<PlaneArt>> {
    use crate::mode7art::{Entry, PLANE, SCALE, TILE};
    let entry = &pack.mode7.plane[at as usize];
    let palette = entry.colours()?;
    let (side, across) = (PLANE * SCALE, Entry::tiles_across());
    let tile_side = TILE * SCALE;
    let read_tile = |n: usize| {
        let file = read_file(
            &pack.dir.join("art/mode7").join(entry.tile_file(n % across, n / across)),
            bytes,
        )
        .ok()?;
        let (w, h, rgba) = crate::png::read_rgba(&file).ok()?;
        (w as usize == tile_side && h as usize == tile_side).then_some(rgba)
    };
    let tiles: Vec<Option<Vec<u8>>> = if parallel {
        (0..across * across).into_par_iter().map(read_tile).collect()
    } else {
        (0..across * across).map(read_tile).collect()
    };
    let mut rgba = vec![0_u8; side * side * 4];
    // Art found by its content (a picture of the art override) may have files for some tiles only: the others have no
    // art. The pack's own planes need all their tiles
    let sparse = entry.content_value().is_some();
    if sparse && tiles.iter().all(Option::is_none) {
        return None;
    }
    for (n, tile) in tiles.into_iter().enumerate() {
        let tile = tile.or_else(|| sparse.then(Vec::new))?;
        if tile.is_empty() {
            continue;
        }
        let (ox, oy) = ((n % across) * tile_side, (n / across) * tile_side);
        for y in 0..tile_side {
            let from = y * tile_side * 4;
            let to = ((oy + y) * side + ox) * 4;
            rgba[to..to + tile_side * 4].copy_from_slice(&tile[from..from + tile_side * 4]);
        }
    }
    Some(Arc::new(PlaneArt::new(side, &rgba, palette)))
}

/// The art of background entry `at` (`large`: its layer has 16 x 16 characters) at `scale`, with
/// its source.
pub(crate) fn bg(
    pack: &Pack,
    at: u32,
    large: bool,
    scale: usize,
    bytes: &AtomicU64,
) -> Option<Arc<BgImage>> {
    let entry = &pack.bg.layer[at as usize];
    let (width, height) = entry.art_size(large);
    let read = |relative: &str| -> Option<(usize, usize, Vec<u8>)> {
        let file = read_file(&pack.dir.join(relative), bytes).ok()?;
        let (w, h, rgba) = crate::png::read_rgba(&file).ok()?;
        Some((w as usize, h as usize, rgba))
    };
    let (aw, ah, art) = read(&entry.art)?;
    // The files are what the index says: the layer's rows at the art's scale, and at 1x
    if (aw, ah) != (width * bgart::SCALE, height * bgart::SCALE) {
        return None;
    }
    let source = if entry.source.is_empty() {
        None
    } else {
        let (sw, sh, source) = read(&entry.source)?;
        if (sw, sh) != (width, height) {
            return None;
        }
        Some(Arc::new(Image::new(sw, sh, source)))
    };
    let art = resample::resize(&Image::new(aw, ah, art), width * scale, height * scale);
    let base = bgart::parse_palette(&entry.palette).map(Arc::new);
    Some(Arc::new(BgImage { art: BgArt::Whole(art), base, source }))
}

/// The art in file `relative` of the pack sized to `width` x `height`.
fn image(
    pack: &Pack,
    relative: &str,
    width: usize,
    height: usize,
    bytes: &AtomicU64,
) -> Option<Arc<Image>> {
    let file = read_file(&pack.dir.join(relative), bytes).ok()?;
    let (w, h, rgba) = crate::png::read_rgba(&file).ok()?;
    let image = Image::new(w as usize, h as usize, rgba);
    if image.width == 0 || image.height == 0 {
        return None;
    }
    Some(Arc::new(resample::resize(&image, width, height)))
}

/// The art of picture `at` of the index at `scale`: 32 art pixels to a character.
pub(crate) fn picture(pack: &Pack, at: u32, scale: usize, bytes: &AtomicU64) -> Option<Arc<Image>> {
    let side = 32 * scale;
    image(pack, &pack.art.pictures()[at as usize].art, side, side, bytes)
}

/// The art of object `at` of the index at `scale`, mirrored when `at` has [`MIRROR`].
pub(crate) fn object(pack: &Pack, at: u32, scale: usize, bytes: &AtomicU64) -> Option<Arc<Image>> {
    let object = &pack.art.objects()[real(at)];
    let (w, h) = (usize::from(object.width) * scale, usize::from(object.height) * scale);
    let image = image(pack, &object.art, w, h, bytes)?;
    if at & MIRROR == 0 {
        return Some(image);
    }
    Some(Arc::new(mirror(&image)))
}

/// The picture of a pose variant of `object` at `scale`, mirrored when the pose faces the other way (as the object's
/// own picture is). The file must be the object's picture grown by the margin, four art pixels to a game pixel: a file
/// that is missing or of another size is not drawn, and the log says why.
pub(crate) fn pose_variant(
    pack: &Pack,
    variant: &PoseVariant,
    object: &Object,
    mirrored: bool,
    scale: usize,
    bytes: &AtomicU64,
) -> Option<Arc<Image>> {
    let margin = variant.margin as usize;
    let (w, h) = (usize::from(object.width) + 2 * margin, usize::from(object.height) + 2 * margin);
    let refused = |reason: String| {
        log::warn!("the pose variant {} of {} is not drawn: {reason}", variant.file, object.id);
    };
    let file = match read_file(&pack.dir.join(&variant.file), bytes) {
        Ok(file) => file,
        Err(error) => {
            refused(error.to_string());
            return None;
        }
    };
    let (pw, ph, rgba) = match crate::png::read_rgba(&file) {
        Ok(picture) => picture,
        Err(error) => {
            refused(error);
            return None;
        }
    };
    if (pw as usize, ph as usize) != (w * 4, h * 4) {
        refused(format!("it is {pw} x {ph} pixels, not {} x {}", w * 4, h * 4));
        return None;
    }
    let picture = Image::new(pw as usize, ph as usize, rgba);
    let image = resample::resize(&picture, w * scale, h * scale);
    Some(Arc::new(if mirrored { mirror(&image) } else { image }))
}

/// The reading thread: each art read comes back with the bytes of its files.
type ArtReader = Reader<ArtKey, (ArtKey, ArtJob), (Loaded, u64)>;

/// What [`ArtReader`] read since it was last asked, as [`Reader::take`] gives it.
type ArtRead = Vec<(ArtKey, (ArtKey, ArtJob), (Loaded, u64))>;

/// How the art is read (see the module): by the frames, or by a thread of its own, and what the frame
/// being drawn asked of it.
#[derive(Default)]
pub(crate) struct Loading {
    /// The art is read by the thread ([`ArtState::set_async`]).
    on: bool,
    /// Started at the first art asked for; `None` when the thread cannot be started or is gone (the
    /// frames then read the art themselves).
    reader: Option<ArtReader>,
    /// Whether the thread was started once (it is not tried again).
    started: bool,
    /// The art the frame being drawn asked for, in order.
    asked: Vec<(ArtKey, ArtJob)>,
    /// The art the frame before asked for was not empty (the jobs it left are dropped).
    asked_before: bool,
    /// The frame being drawn lacks art it asked the thread for: its picture is not the one drawn
    /// once the art is read.
    pub(crate) pending: bool,
}

impl Loading {
    /// Whether the frames ask the thread for the art instead of reading it.
    pub(crate) fn asynchronous(&self) -> bool {
        self.on && (!self.started || self.reader.is_some())
    }

    /// Starts a frame: the art the thread read since the last frame, to keep; the art asked for by
    /// the frame before and not started is dropped (the frame that still wants it asks again).
    fn begin(&mut self) -> ArtRead {
        self.pending = false;
        if self.reader.as_ref().is_some_and(|reader| !reader.alive()) {
            self.reader = None;
        }
        let Some(reader) = self.reader.as_mut() else {
            self.asked.clear();
            return Vec::new();
        };
        let read = reader.take();
        if std::mem::take(&mut self.asked_before) {
            reader.ask(Vec::new());
        }
        self.asked.clear();
        read
    }

    /// Asks the thread for `key` (once a frame), for the frame being drawn, which goes without it;
    /// whether it was not asked for yet in the frame.
    fn want(&mut self, pack: &Arc<Pack>, key: ArtKey, job: ArtJob) -> bool {
        self.pending = true;
        if self.asked.iter().any(|(asked, _)| *asked == key) {
            return false;
        }
        self.asked.push((key, job));
        self.asked_before = true;
        if !self.started {
            self.started = true;
            let pack = Arc::clone(pack);
            self.reader = Reader::start("hd-art", move |&(key, job): &(ArtKey, ArtJob)| {
                let bytes = AtomicU64::new(0);
                let loaded = read(&pack, key, &job, false, &bytes);
                (loaded, bytes.into_inner())
            });
        }
        if let Some(reader) = self.reader.as_mut() {
            reader.ask(self.asked.iter().map(|&(key, job)| (key, (key, job))).collect());
        }
        true
    }
}

impl ArtState {
    /// Reads the art on a thread of its own (`true`) instead of in the frames that want it (`false`,
    /// the start): a frame draws the art it has and asks for the rest, which the frame after it is
    /// read draws (see the module). `SSD_HD_SERIAL` in the environment keeps the art read by the
    /// frames.
    pub(crate) fn set_async(&mut self, on: bool) {
        self.loading.on = on && !serial_env();
    }

    /// Whether the frame drawn last lacks art it asked for (see [`ArtState::set_async`]).
    pub(crate) fn art_pending(&self) -> bool {
        self.loading.pending
    }

    /// Keeps the art the reading thread read since the last frame, counted as received in the frame's
    /// cost; call it when a frame starts, once its scale is set and its cost started over.
    pub(crate) fn begin_loading(&mut self) {
        for (key, (_, job), (loaded, bytes)) in self.loading.begin() {
            if job.scale != self.scale {
                continue;
            }
            self.loads.receive(kind_of(key), bytes);
            match (key, loaded) {
                (ArtKey::Bg(at), Loaded::Bg(art)) => {
                    if !self.bg_images.contains(at) {
                        self.bg_images.insert(at, art, super::bg_cache::BG_BUDGET);
                    }
                }
                (ArtKey::Picture(at), Loaded::Image(art)) => {
                    self.pictures.entry(at).or_insert(art);
                }
                (ArtKey::Object(at), Loaded::Image(art)) => {
                    if !self.objects.contains_key(&at) {
                        self.remember_object(at, art);
                    }
                }
                (ArtKey::Variant(n, mirrored), Loaded::Image(art)) => {
                    self.pose_images.entry((n, mirrored)).or_insert(art);
                }
                (ArtKey::Plane(at), Loaded::Plane(art)) => {
                    self.plane_arts.entry(at).or_insert(art);
                }
                _ => {}
            }
        }
    }

    /// Whether the art is asked of the reading thread rather than read by the frame.
    pub(crate) fn loads_async(&self) -> bool {
        self.loading.asynchronous()
    }

    /// Asks the reading thread for `key` (the frame goes without it).
    pub(crate) fn want(&mut self, key: ArtKey, object: u32) {
        let large = match key {
            ArtKey::Bg(at) => {
                self.lookup.bg_slots.iter().any(|slot| slot.entry == at && slot.large)
            }
            _ => false,
        };
        let job = ArtJob { scale: self.scale, large, object };
        if self.loading.want(&self.pack, key, job) {
            self.loads.request();
        }
    }

    /// Waits until the reading thread has read every art asked for (the art of the frames after it is
    /// all read); whether it did within 30 seconds.
    #[cfg(test)]
    pub(crate) fn wait_read(&self) -> bool {
        self.loading
            .reader
            .as_ref()
            .is_none_or(|reader| reader.wait_read(std::time::Duration::from_secs(30)))
    }

    /// Counts `count` art files read by the frame itself.
    pub(crate) fn loaded_on_draw(&mut self, count: usize) {
        self.stats.loads_on_draw += count as u32;
        LOADS_ON_DRAW.fetch_add(count as u64, Ordering::Relaxed);
    }
}
