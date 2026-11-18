//! The lightning on synthetic frames: a backdrop of one colour, and pictures made in a temporary folder (a block of one
//! colour on black, nothing like the real lightning).

use super::{Blend, FRAMES_PER_PICTURE, Lightning, SIZE};
use crate::world::{HudPlacement, Rom, WideOptions, WideRenderer};
use crate::{Conditions, HdFrame, HdOptions, Look, Renderer};
use ssd_oracle::{FrameLog, LineEntry, LineRegs, SpriteLine};
use std::path::Path;
use std::sync::Arc;

/// The orange of the transformation's backdrop (BGR555 31, 16, 0): (255, 132, 0) in the picture.
const ORANGE: u16 = 16 << 5 | 31;
const ORANGE_RGB: [u8; 3] = [255, 132, 0];

/// A frame of 224 lines showing only the backdrop, in `colour`.
fn backdrop(colour: u16) -> FrameLog {
    let regs = LineRegs { brightness: 15, bg_mode: 1, ..LineRegs::default() };
    let entries = (1..=224_u16)
        .map(|scanline| LineEntry {
            scanline,
            from_pixel: 0,
            regs,
            sprites: SpriteLine { eval_line: scanline - 1, rebuilt: true, ..SpriteLine::default() },
        })
        .collect();
    let mut log = FrameLog {
        complete: true,
        height: 224,
        h_hi_res: false,
        v_hi_res: false,
        entries,
        cgram_start: vec![0; 256],
        cgram_writes: Vec::new(),
        vram: vec![0; 32768],
        vram_writes: Vec::new(),
        oam_low: vec![0; 512],
        oam_high: vec![0; 32],
        oam_writes: Vec::new(),
    };
    for n in 0..128 {
        log.oam_low[n * 4 + 1] = 0xF0;
    }
    log.cgram_start[0] = colour;
    log
}

fn options(scale: u32) -> HdOptions {
    HdOptions { scale, look: Look::Crisp }
}

fn pixel(frame: &HdFrame, x: usize, y: usize) -> [u8; 3] {
    let at = (y * frame.width as usize + x) * 4;
    [frame.rgba[at], frame.rgba[at + 1], frame.rgba[at + 2]]
}

/// Writes an 8-bit PNG of `width` x `height` to `path`: RGB (no alpha) or RGBA, the pixel at (x, y) `colour(x, y)`.
fn write_png(
    path: &Path,
    width: u32,
    height: u32,
    alpha: bool,
    colour: impl Fn(u32, u32) -> [u8; 4],
) {
    let mut data = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let [r, g, b, a] = colour(x, y);
            data.extend_from_slice(&[r, g, b]);
            if alpha {
                data.push(a);
            }
        }
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = ::png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(if alpha { ::png::ColorType::Rgba } else { ::png::ColorType::Rgb });
    encoder.set_depth(::png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&data).unwrap();
}

/// A picture of [`SIZE`] on black (RGB), with `colour` in the block of columns 0 to 511 and rows 0 to 447.
fn block_picture(path: &Path, colour: [u8; 3]) {
    let [r, g, b] = colour;
    write_png(path, SIZE.0, SIZE.1, false, |x, y| {
        if x < 512 && y < 448 { [r, g, b, 255] } else { [0, 0, 0, 255] }
    });
}

/// The Enhanced picture of `log` at scale 4 by a renderer with the lightning of `dir` (`None`: none set at all), the
/// transformation's frames `frames`.
fn drawn(log: &FrameLog, dir: Option<&Path>, frames: Option<u32>) -> HdFrame {
    let mut renderer = Renderer::new();
    if let Some(dir) = dir {
        renderer.set_lightning(Lightning::load(dir).map(Arc::new));
    }
    renderer.set_conditions(Conditions::default().with_ssj_lightning(frames));
    renderer.render_hd(log, options(4)).unwrap()
}

/// `frame * (255 - a) / 255 + add`, rounded, as a colour of the brightness blend should come out.
fn over(frame: [u8; 3], add: [u8; 3], a: u8) -> [u8; 3] {
    std::array::from_fn(|c| {
        let kept = (f64::from(frame[c]) * f64::from(255 - a) / 255.0).round();
        (kept + f64::from(add[c])).min(255.0) as u8
    })
}

fn close(got: [u8; 3], expected: [u8; 3]) -> bool {
    got.iter().zip(expected).all(|(&g, e)| g.abs_diff(e) <= 1)
}

#[test]
fn without_the_folder_or_a_first_picture_the_picture_is_the_one_drawn_before_to_the_pixel() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("ssj-lightning");
    assert!(Lightning::load(&folder).is_none(), "no folder");
    std::fs::create_dir_all(&folder).unwrap();
    assert!(Lightning::load(&folder).is_none(), "an empty folder");
    block_picture(&folder.join("frame-2.png"), [100, 150, 200]);
    assert!(Lightning::load(&folder).is_none(), "the pictures start at frame-1.png");
    std::fs::write(folder.join("frame-1.png"), b"not a picture").unwrap();
    assert!(Lightning::load(&folder).is_none(), "a frame-1.png that cannot be read");

    let log = backdrop(ORANGE);
    // The renderer as it was before the lightning: never told of any
    let before = drawn(&log, None, None);
    for frames in [None, Some(0), Some(7), Some(341)] {
        assert!(drawn(&log, Some(&folder), frames) == before, "no picture, at {frames:?}");
        assert!(
            drawn(&log, Some(&dir.path().join("none")), frames) == before,
            "no folder, at {frames:?}"
        );
    }
    // With a picture, the frames outside the transformation are those of before too
    block_picture(&folder.join("frame-1.png"), [100, 150, 200]);
    assert!(Lightning::load(&folder).is_some());
    assert!(drawn(&log, Some(&folder), None) == before, "not in the transformation");
}

#[test]
fn a_picture_without_alpha_takes_its_brightness_for_alpha_and_its_blue_stays_blue() {
    assert_eq!(super::BLEND, Blend::Brightness, "the blend the pictures are drawn with");
    let dir = tempfile::tempdir().unwrap();
    let blue = [100, 150, 200];
    block_picture(&dir.path().join("frame-1.png"), blue);
    let log = backdrop(ORANGE);
    let before = drawn(&log, None, None);
    assert_eq!(pixel(&before, 0, 0), ORANGE_RGB);
    let lit = drawn(&log, Some(dir.path()), Some(0));
    // In the block: a = max(100, 150, 200) = 200, the frame keeps 55/255 of itself and takes the picture's colour
    let expected = over(ORANGE_RGB, blue, 200);
    for (x, y) in [(0, 0), (511, 447), (200, 100)] {
        let got = pixel(&lit, x, y);
        assert!(close(got, expected), "({x}, {y}): {got:?}, not {expected:?}");
        assert!(got[2] > got[0], "the blue stays blue over the orange: {got:?}");
    }
    // Black is nothing: the rest is the frame as it was
    for (x, y) in [(512, 0), (0, 448), (1023, 895)] {
        assert_eq!(pixel(&lit, x, y), ORANGE_RGB, "({x}, {y})");
    }
    // Added instead, the light blue over the orange would go to white
    let added: [u8; 3] = std::array::from_fn(|c| ORANGE_RGB[c].saturating_add(blue[c]));
    assert!(added[0] > added[2], "additive: {added:?}, the orange wins");
}

#[test]
fn a_picture_with_alpha_is_laid_over_by_its_alpha() {
    let dir = tempfile::tempdir().unwrap();
    // Opaque black in the block's left half, half-transparent blue in its right half, transparent elsewhere
    write_png(&dir.path().join("frame-1.png"), SIZE.0, SIZE.1, true, |x, y| match (x, y) {
        (0..256, 0..448) => [0, 0, 0, 255],
        (256..512, 0..448) => [0, 0, 255, 128],
        _ => [255, 255, 255, 0],
    });
    let log = backdrop(ORANGE);
    let lit = drawn(&log, Some(dir.path()), Some(0));
    assert_eq!(pixel(&lit, 10, 10), [0, 0, 0], "opaque black covers the frame");
    let expected = over(ORANGE_RGB, [0, 0, 128], 128);
    assert!(close(pixel(&lit, 300, 10), expected), "{:?}, not {expected:?}", pixel(&lit, 300, 10));
    assert_eq!(pixel(&lit, 600, 10), ORANGE_RGB, "transparent white is nothing");
}

#[test]
fn the_pictures_take_turns_every_few_frames_and_one_picture_stays() {
    let dir = tempfile::tempdir().unwrap();
    let colours = [[200, 0, 0], [0, 200, 0], [0, 0, 200]];
    for (n, colour) in colours.iter().enumerate() {
        block_picture(&dir.path().join(format!("frame-{}.png", n + 1)), *colour);
    }
    let log = backdrop(0);
    let shown = |frames: u32| pixel(&drawn(&log, Some(dir.path()), Some(frames)), 0, 0);
    let step = FRAMES_PER_PICTURE;
    assert_eq!(step, 4);
    for (frames, picture) in [
        (0, 0),
        (step - 1, 0),
        (step, 1),
        (2 * step - 1, 1),
        (2 * step, 2),
        (3 * step, 0),
        (341, 1),
    ] {
        assert_eq!(shown(frames), colours[picture], "frame {frames} of the transformation");
    }
    // Only frame-1.png: the same picture all along
    let one = tempfile::tempdir().unwrap();
    block_picture(&one.path().join("frame-1.png"), colours[0]);
    for frames in [0, 1, step, 3 * step + 2, 341] {
        assert_eq!(pixel(&drawn(&log, Some(one.path()), Some(frames)), 0, 0), colours[0]);
    }
    // Up to four pictures are read
    let five = tempfile::tempdir().unwrap();
    for n in 1..=5 {
        block_picture(&five.path().join(format!("frame-{n}.png")), [n * 40, 0, 0]);
    }
    assert_eq!(Lightning::load(five.path()).unwrap().len(), 4);
}

#[test]
fn a_picture_of_another_size_is_scaled_to_the_frame_keeping_its_proportions() {
    let log = backdrop(0);
    // Half the size, the frame's proportions: the whole frame, each pixel two by two
    let half = tempfile::tempdir().unwrap();
    write_png(&half.path().join("frame-1.png"), 512, 448, false, |x, _| {
        if x < 256 { [0, 0, 200, 255] } else { [0, 0, 0, 255] }
    });
    let lit = drawn(&log, Some(half.path()), Some(0));
    assert_eq!(pixel(&lit, 0, 0), [0, 0, 200]);
    assert_eq!(pixel(&lit, 511, 895), [0, 0, 200], "the left half of the frame");
    assert_eq!(pixel(&lit, 512, 0), [0, 0, 0]);
    // Twice as wide as the frame's proportions: as wide as the frame, half its height, centred
    let wide = tempfile::tempdir().unwrap();
    write_png(&wide.path().join("frame-1.png"), 1024, 448, false, |_, _| [0, 0, 200, 255]);
    let lit = drawn(&log, Some(wide.path()), Some(0));
    assert_eq!(pixel(&lit, 0, 223), [0, 0, 0], "above it, nothing");
    assert_eq!(pixel(&lit, 0, 224), [0, 0, 200]);
    assert_eq!(pixel(&lit, 1023, 671), [0, 0, 200]);
    assert_eq!(pixel(&lit, 0, 672), [0, 0, 0], "below it, nothing");
    // At scale 2 the picture of the frame's size is made half as big
    let full = tempfile::tempdir().unwrap();
    block_picture(&full.path().join("frame-1.png"), [0, 0, 200]);
    let mut renderer = Renderer::new();
    renderer.set_lightning(Lightning::load(full.path()).map(Arc::new));
    renderer.set_conditions(Conditions::default().with_ssj_lightning(Some(0)));
    let small = renderer.render_hd(&log, options(2)).unwrap();
    assert_eq!((small.width, small.height), (512, 448));
    assert_eq!(pixel(&small, 255, 223), [0, 0, 200]);
    assert_eq!(pixel(&small, 256, 224), [0, 0, 0]);
}

#[test]
fn the_wide_view_draws_it_over_the_ppus_picture_only() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("frame-1.png"), SIZE.0, SIZE.1, false, |_, _| [0, 0, 200, 255]);
    let log = backdrop(ORANGE);
    let wide_options = WideOptions { width: 372, hud: HudPlacement::Window };
    let draw = |lightning: Option<&Path>, frames: Option<u32>| {
        let mut wide = WideRenderer::new();
        if let Some(dir) = lightning {
            wide.set_lightning(Lightning::load(dir).map(Arc::new));
        }
        wide.set_conditions(Conditions::default().with_ssj_lightning(frames));
        wide.render_hd(Rom(&[]), 0x02, None, &log, wide_options, options(4)).unwrap()
    };
    let before = draw(None, None);
    assert!(draw(Some(&dir.path().join("none")), Some(0)).picture == before.picture, "no folder");
    assert!(draw(Some(dir.path()), None).picture == before.picture, "not in the transformation");
    let lit = draw(Some(dir.path()), Some(0));
    let logical = lit.picture.logical;
    assert_eq!((logical.width, logical.height), SIZE, "the PPU's picture at scale 4");
    let inside = pixel(&lit.picture, logical.x as usize, logical.y as usize);
    let expected = over(ORANGE_RGB, [0, 0, 200], 200);
    assert!(close(inside, expected), "{inside:?}, not {expected:?}");
    let last = (logical.x + logical.width - 1) as usize;
    assert!(close(pixel(&lit.picture, last, 895), expected));
    // Beside the PPU's picture, the margins of the wide view are left as they were
    assert!(
        logical.x > 0 && logical.x + logical.width < lit.picture.width,
        "the view is wider than the PPU's"
    );
    let margin = (logical.x - 1) as usize;
    assert_eq!(pixel(&lit.picture, margin, 10), pixel(&before.picture, margin, 10));
    let right = (logical.x + logical.width) as usize;
    assert_eq!(pixel(&lit.picture, right, 10), pixel(&before.picture, right, 10));
}
