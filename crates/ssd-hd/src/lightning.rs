//! The lightning of Goku's first Super Saiyan transformation: the pictures of `<data dir>\art-override\ssj-lightning\`
//! (`frame-1.png` to `frame-4.png`, made by hand) drawn over the whole Enhanced picture while the transformation goes on.
//! The pictures are drawn as they are: nothing here draws, moves or bends the lightning; a flicker is the pictures shown
//! in turn, [`FRAMES_PER_PICTURE`] frames each. Without the folder, or with no picture in it, nothing is drawn.
//!
//! When the transformation goes on is the app's to say (the watch of its voice, `ssd-app`'s `voice.rs`): it hands the
//! frames since it started to the renderer with the frame's conditions ([`crate::Conditions::with_ssj_lightning`]).
//!
//! A picture is meant to be [`SIZE`] (the 256x224 frame at scale 4, the SNES's 8:7), RGB on black or RGBA. One of
//! another size is scaled, keeping its proportions, to the largest size that fits the frame, centred (a warning is
//! logged). It is drawn over the PPU's picture (the backgrounds, the sprites and the pack's art) and under the HD text:
//! the game's own windows are layers of that picture, so they are under it too.
//!
//! How a picture is laid over the frame ([`BLEND`]): a picture without alpha takes its brightness for its alpha
//! ([`Blend::Brightness`]: black is nothing, its colours stay theirs over the bright backdrop of the transformation);
//! a picture with alpha is laid over by its alpha. [`Blend::Additive`] adds the colours instead (weighted by the alpha).

use crate::HdFrame;
use rayon::prelude::*;
use std::path::Path;

/// The folder of the pictures in the data folder's `art-override\` ([`crate::pack::ART_OVERRIDE`]).
pub const DIR: &str = "ssj-lightning";
/// The most pictures read: `frame-1.png` to `frame-4.png`.
pub const MAX_PICTURES: usize = 4;
/// The frames each picture is shown for, in turn, while there are several: 4 frames at 60 a second is 15 pictures a
/// second, a flicker the eye sees as one (every picture is seen, not a blur of them), about 85 turns over the 342 frames
/// of the transformation.
pub const FRAMES_PER_PICTURE: u32 = 4;
/// The size a picture is made at: the frame at scale 4 (256x224, the SNES's 8:7).
pub const SIZE: (u32, u32) = (1024, 896);

/// How a picture is laid over the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    /// A picture without alpha takes its brightness for its alpha: `a = max(r, g, b)`, its colour `c = rgb / a`, and
    /// the frame becomes `frame * (1 - a) + c * a` (black is nothing). A picture with alpha is laid over by its alpha:
    /// `frame * (1 - alpha) + rgb * alpha`.
    Brightness,
    /// The picture's colours, weighted by its alpha, are added to the frame's (saturating): over a bright frame a
    /// light colour goes to white.
    Additive,
}

/// The blend the pictures are drawn with.
pub const BLEND: Blend = Blend::Brightness;

/// The pictures of the lightning, as read from their folder.
#[derive(Debug)]
pub struct Lightning {
    pictures: Vec<Picture>,
}

/// A picture as read: its size, and each pixel as what it adds (`rgb`, alpha-weighted) and the alpha it covers the
/// frame with, both 0 to 255.
#[derive(Debug)]
struct Picture {
    width: usize,
    height: usize,
    /// `[r, g, b, a]` a pixel, row by row: the frame becomes `frame * (255 - a) / 255 + rgb`.
    pixels: Vec<[u8; 4]>,
}

impl Lightning {
    /// The pictures of `dir` (the data folder's `art-override\ssj-lightning\`): `frame-1.png`, `frame-2.png`, ... up to
    /// [`MAX_PICTURES`], in order, up to the first missing. `None` without the folder, or with no `frame-1.png`. A
    /// picture that cannot be read ends the list there, and the log says why; one not of [`SIZE`] is scaled to the frame
    /// and the log warns of it.
    #[must_use]
    pub fn load(dir: &Path) -> Option<Self> {
        let mut pictures = Vec::new();
        for n in 1..=MAX_PICTURES {
            let path = dir.join(format!("frame-{n}.png"));
            let Ok(bytes) = std::fs::read(&path) else { break };
            match Picture::read(&bytes, BLEND) {
                Ok(picture) => {
                    if (picture.width, picture.height) != (SIZE.0 as usize, SIZE.1 as usize) {
                        log::warn!(
                            "{} is {}x{}, not {}x{}: it is scaled to the frame",
                            path.display(),
                            picture.width,
                            picture.height,
                            SIZE.0,
                            SIZE.1
                        );
                    }
                    pictures.push(picture);
                }
                Err(error) => {
                    log::warn!("{} is left out: {error}", path.display());
                    break;
                }
            }
        }
        if pictures.is_empty() {
            return None;
        }
        log::info!(
            "the transformation's lightning: {} pictures from {}",
            pictures.len(),
            dir.display()
        );
        Some(Self { pictures })
    }

    /// How many pictures there are (1 to [`MAX_PICTURES`]).
    #[must_use]
    pub fn len(&self) -> usize {
        self.pictures.len()
    }

    /// Never: a `Lightning` has a picture at least.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pictures.is_empty()
    }

    /// The picture shown `frames` frames after the transformation started: each in turn for [`FRAMES_PER_PICTURE`]
    /// frames, the only one all along.
    fn picture_at(&self, frames: u32) -> usize {
        (frames / FRAMES_PER_PICTURE) as usize % self.pictures.len()
    }
}

impl Picture {
    /// The picture of the PNG `bytes`, its pixels made ready for `blend`.
    fn read(bytes: &[u8], blend: Blend) -> Result<Self, String> {
        let alpha = declares_alpha(bytes);
        let (width, height, rgba) = crate::png::read_rgba(bytes)?;
        let pixels = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&[r, g, b, a]| {
                let weighted = |c: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
                match (blend, alpha) {
                    (Blend::Brightness, false) => [r, g, b, r.max(g).max(b)],
                    (Blend::Brightness, true) => [weighted(r), weighted(g), weighted(b), a],
                    (Blend::Additive, _) => [weighted(r), weighted(g), weighted(b), 0],
                }
            })
            .collect();
        Ok(Self { width: width as usize, height: height as usize, pixels })
    }

    /// The picture made `width` x `height`: each pixel the mean of the picture's pixels its area covers (the nearest one
    /// where it is larger than they are).
    fn scaled(&self, width: usize, height: usize) -> Vec<[u8; 4]> {
        if (width, height) == (self.width, self.height) {
            return self.pixels.clone();
        }
        let span = |at: usize, to: usize, from: usize| {
            let first = at * from / to;
            let past = ((at + 1) * from).div_ceil(to).max(first + 1).min(from);
            first..past
        };
        let mut out = vec![[0; 4]; width * height];
        out.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
            let rows = span(y, height, self.height);
            for (x, pixel) in row.iter_mut().enumerate() {
                let columns = span(x, width, self.width);
                let mut sum = [0_u32; 4];
                for source in rows.clone() {
                    for p in &self.pixels
                        [source * self.width + columns.start..source * self.width + columns.end]
                    {
                        for (s, &c) in sum.iter_mut().zip(p) {
                            *s += u32::from(c);
                        }
                    }
                }
                let count = (rows.len() * columns.len()) as u32;
                *pixel = sum.map(|s| ((s + count / 2) / count) as u8);
            }
        });
        out
    }
}

/// Whether the PNG `bytes` has alpha of its own: a colour type with alpha, or a `tRNS` chunk. Not a PNG: whether a pixel
/// is not opaque.
fn declares_alpha(bytes: &[u8]) -> bool {
    let decoder = ::png::Decoder::new(std::io::Cursor::new(bytes));
    match decoder.read_info() {
        Ok(reader) => {
            let info = reader.info();
            info.color_type.samples() == 2 || info.color_type.samples() == 4 || info.trns.is_some()
        }
        Err(_) => crate::png::read_rgba(bytes)
            .is_ok_and(|(_, _, rgba)| rgba.as_chunks::<4>().0.iter().any(|p| p[3] != 255)),
    }
}

/// A place in a frame: x, y, width, height.
type Place = (usize, usize, usize, usize);

/// The lightning a renderer draws with: the pictures, and the last picture made the size of a frame (pictures of
/// one size follow each other: it is made once).
#[derive(Debug)]
pub(crate) struct Overlay {
    lightning: std::sync::Arc<Lightning>,
    /// The picture, the place in the frame it was made for, and its pixels.
    scaled: Option<(usize, Place, Vec<[u8; 4]>)>,
}

impl Overlay {
    pub(crate) fn new(lightning: std::sync::Arc<Lightning>) -> Self {
        Self { lightning, scaled: None }
    }

    /// Draws the picture of the lightning `frames` frames after the transformation started over `hd`, in its PPU
    /// picture (`hd.logical`).
    pub(crate) fn draw(&mut self, hd: &mut HdFrame, frames: u32) {
        let index = self.lightning.picture_at(frames);
        let picture = &self.lightning.pictures[index];
        let area = (hd.logical.width as usize, hd.logical.height as usize);
        let place = fit(
            picture.width,
            picture.height,
            area,
            (hd.logical.x as usize, hd.logical.y as usize),
        );
        if !self.scaled.as_ref().is_some_and(|(at, made, _)| *at == index && *made == place) {
            self.scaled = Some((index, place, picture.scaled(place.2, place.3)));
        }
        let Some((_, (x, y, width, height), pixels)) = self.scaled.as_ref() else { return };
        let stride = hd.width as usize * 4;
        hd.rgba
            .par_chunks_mut(stride)
            .skip(*y)
            .take(*height)
            .zip(pixels.par_chunks(*width))
            .for_each(|(row, over)| {
                let row = &mut row[x * 4..(x + width) * 4];
                for (pixel, &[r, g, b, a]) in row.as_chunks_mut::<4>().0.iter_mut().zip(over) {
                    if r | g | b | a == 0 {
                        continue;
                    }
                    let keep = u16::from(255 - a);
                    for (channel, add) in pixel.iter_mut().zip([r, g, b]) {
                        let kept = (u16::from(*channel) * keep + 127) / 255;
                        *channel = (kept + u16::from(add)).min(255) as u8;
                    }
                }
            });
    }
}

/// Where a picture of `width` x `height` goes in an area of `area` at `origin`: the largest size of its proportions that
/// fits, centred; the whole area for a picture of the area's proportions.
fn fit(width: usize, height: usize, area: (usize, usize), origin: (usize, usize)) -> Place {
    let (area_width, area_height) = area;
    // (the width the area's height gives the picture, rounded: within a pixel of the area's is the area's)
    let wide = width * area_height;
    let (w, h) = if wide.abs_diff(area_width * height) <= height {
        (area_width, area_height)
    } else if wide > area_width * height {
        (area_width, (height * area_width + width / 2) / width)
    } else {
        ((wide + height / 2) / height, area_height)
    };
    let (w, h) = (w.clamp(1, area_width.max(1)), h.clamp(1, area_height.max(1)));
    (origin.0 + (area_width - w) / 2, origin.1 + (area_height - h) / 2, w, h)
}

#[cfg(test)]
mod tests;
