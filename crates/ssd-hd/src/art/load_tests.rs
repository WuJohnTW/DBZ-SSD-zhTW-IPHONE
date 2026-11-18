//! The loading of the pack's art: the background layers kept within their budget, and the art read
//! on its own thread while the frames draw what they have.

use super::tests::{
    background_frame, background_pack, fighter_log, object, options, pack_of, plane_frame,
    plane_pack, portrait_log, red_picture_pack, variant_pack, wram_with_kaioken, write_png,
};
use super::{ArtState, Conditions};
use crate::Renderer;
use crate::art_index::ArtIndex;
use crate::pack::Pack;
use ssd_oracle::FrameLog;
use std::sync::Arc;

/// A pack with `count` background layers (BG4 of mode 0, tile rows 0 to 4, one screen), each with
/// its own art file.
fn layers_pack(dir: &std::path::Path, count: usize) -> Arc<Pack> {
    let mut pack = Arc::try_unwrap(pack_of(dir, Vec::new(), ArtIndex::default())).unwrap();
    let mut layer = Vec::new();
    for n in 0..count {
        let art = format!("art/bg/layer{n}.png");
        write_png(&dir.join(&art), 1024, 128, |x, y| [n as u8, (x / 4) as u8, (y / 4) as u8, 255]);
        layer.push(crate::bgart::Entry {
            id: format!("layer{n}"),
            key: format!("{n:016x}"),
            bg: 3,
            mode: 0,
            map_base: 0x0400,
            char_base: 0x1000,
            size: 0,
            rows: [0, 4],
            fill: true,
            art,
            source: String::new(),
            palette_key: String::new(),
            palette: String::new(),
        });
    }
    pack.bg = crate::bgart::Index { layer };
    Arc::new(pack)
}

#[test]
fn five_layers_shown_two_at_a_time_in_turn_are_each_read_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut art = ArtState::new(layers_pack(dir.path(), 5));
    // Frame f binds layers 2f and 2f + 1 (mod 5): layer 0 is in frames 0 and 2, layer 1 in 0 and 3…
    for frame in 0..10_u32 {
        art.begin(256, 224, 4);
        for at in [frame * 2 % 5, (frame * 2 + 1) % 5] {
            assert!(art.bg_image(at).is_some(), "layer {at} of frame {frame}");
        }
    }
    assert_eq!(art.bg_images.reads, 5, "each layer is read once");
}

/// Waits until the reading thread of `renderer` has read the art asked for.
fn wait_read(renderer: &Renderer) {
    assert!(renderer.art.as_ref().unwrap().wait_read(), "the art is read within 30 seconds");
}

/// Draws `log` with the art of `pack` read on its own thread: the first picture is the one drawn
/// with no art (the original's pixels), the reading thread reads the art, and the picture after it
/// is the one drawn with the art read in the frame (the synchronous default) byte for byte.
/// The frame is drawn under `conditions`.
fn the_art_read_on_its_own_thread_is_drawn_from_the_next_picture(
    pack: &Arc<Pack>,
    log: &FrameLog,
    conditions: Conditions,
) {
    let mut plain = Renderer::new();
    let original = plain.render_hd(log, options(4)).unwrap();
    let mut synchronous = Renderer::new();
    synchronous.set_art(Some(Arc::clone(pack)));
    synchronous.set_conditions(conditions);
    let with_art = synchronous.render_hd(log, options(4)).unwrap();
    // The frame reads its art itself and draws it at once
    assert!(with_art.rgba != original.rgba, "the art changes the picture");
    assert!(synchronous.art_stats().loads_on_draw > 0);
    let mut asynchronous = Renderer::new();
    asynchronous.set_art(Some(Arc::clone(pack)));
    asynchronous.set_art_async(true);
    asynchronous.set_conditions(conditions);
    let first = asynchronous.render_hd(log, options(4)).unwrap();
    assert!(first.rgba == original.rgba, "the first picture is the original's");
    assert_eq!(asynchronous.art_stats().loads_on_draw, 0, "nothing is read by the frame");
    assert!(asynchronous.art.as_ref().unwrap().art_pending());
    wait_read(&asynchronous);
    let next = asynchronous.render_hd(log, options(4)).unwrap();
    assert!(next.rgba == with_art.rgba, "the picture after the art is read is the one with it");
    assert_eq!(asynchronous.art_stats().loads_on_draw, 0);
    assert!(!asynchronous.art.as_ref().unwrap().art_pending());
}

#[test]
fn a_background_layer_read_on_its_own_thread_is_drawn_from_the_picture_after_it_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let log = background_frame(10);
    let pack = background_pack(dir.path(), &log.vram, true);
    the_art_read_on_its_own_thread_is_drawn_from_the_next_picture(
        &pack,
        &log,
        Conditions::default(),
    );
}

#[test]
fn a_pose_read_on_its_own_thread_keeps_the_games_sprites_until_its_art_is_read() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/o.png"), 64, 64, |x, y| [x as u8 * 4, y as u8 * 4, 255, 255]);
    let (log, pose) = fighter_log(40, 50);
    let pack = pack_of(
        dir.path(),
        Vec::new(),
        ArtIndex::new(Vec::new(), vec![object(&pose, "art/o.png", false)]),
    );
    the_art_read_on_its_own_thread_is_drawn_from_the_next_picture(
        &pack,
        &log,
        Conditions::default(),
    );
}

#[test]
fn a_picture_read_on_its_own_thread_is_drawn_from_the_picture_after_it_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let pack = red_picture_pack(dir.path(), [200, 20, 20, 255]);
    let log = portrait_log(0x001F);
    the_art_read_on_its_own_thread_is_drawn_from_the_next_picture(
        &pack,
        &log,
        Conditions::default(),
    );
}

#[test]
fn a_mode_7_plane_read_on_its_own_thread_is_drawn_from_the_picture_after_it_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let log = plane_frame();
    let pack = plane_pack(dir.path(), &log.vram);
    the_art_read_on_its_own_thread_is_drawn_from_the_next_picture(
        &pack,
        &log,
        Conditions::default(),
    );
}

#[test]
fn a_pose_variant_read_on_its_own_thread_keeps_the_games_sprites_until_it_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let (log, pose) = fighter_log(40, 50);
    let pack = variant_pack(&dir, &pose, Some((80, 80)));
    let on = Conditions::of_work_ram(&wram_with_kaioken(true, 0));
    the_art_read_on_its_own_thread_is_drawn_from_the_next_picture(&pack, &log, on);
    // The object's own picture read, the variant not yet: the pose still keeps the game's sprites
    let mut renderer = Renderer::new();
    renderer.set_art(Some(Arc::clone(&pack)));
    renderer.set_art_async(true);
    renderer.render_hd(&log, options(4)).unwrap();
    wait_read(&renderer);
    let object = renderer.render_hd(&log, options(4)).unwrap();
    renderer.set_conditions(on);
    let asked = renderer.render_hd(&log, options(4)).unwrap();
    assert!(asked.rgba != object.rgba, "not the object's picture");
    let mut plain = Renderer::new();
    assert!(asked.rgba == plain.render_hd(&log, options(4)).unwrap().rgba, "the original's");
}

#[test]
fn a_layer_asked_twice_by_a_frame_is_read_once_and_art_read_at_another_scale_is_not_kept() {
    let dir = tempfile::tempdir().unwrap();
    let mut art = ArtState::new(layers_pack(dir.path(), 5));
    art.set_async(true);
    art.begin(256, 224, 4);
    assert!(art.bg_image(0).is_none() && art.bg_image(0).is_none());
    assert!(art.wait_read());
    // Asked once, read once by the thread, kept by the next frame
    art.begin(256, 224, 4);
    assert!(art.bg_image(0).is_some());
    assert_eq!((art.bg_images.reads, art.stats.loads_on_draw), (0, 0));
    // Read at 4x, the art is not kept for a frame at 2x
    assert!(art.bg_image(1).is_none());
    assert!(art.wait_read());
    art.begin(256, 224, 2);
    assert!(!art.bg_images.contains(1));
}
