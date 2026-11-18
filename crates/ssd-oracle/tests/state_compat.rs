//! Files written by an earlier release (core `+73548AE1`) load in this build and the game goes on exactly as it
//! would have there. The battle speed's patch of the core (`ApuClockScale`) changed the core's sources, so the
//! replays' `CORE_VERSION` changed, but not what a save state holds: the state is keyed on
//! `SnesEmulator::save_state_version()` and its payload has no field of the new setting. These tests hold that.
//!
//! The fixtures in `tests/fixtures/v34/` were made by an earlier release's `ssd-oracle` on [`common::sram_counter_rom`], running
//! [`varied_input`] from power-on (`MemorySaveStore`, the default WRAM seed):
//! - `sram_counter_f60.ssdstate`: `save_state()` after frames 0..60;
//! - `sram_counter_f60.trace`: one line per frame 60..180 after that, `crc32(save_state())` and the CRC32 of the
//!   frame's pixels (r, g, b of each), in hex;
//! - `sram_counter_f180.sav`: the SRAM after frame 179 (the battery save).
//!
//! They change only if `sram_counter_rom` does (then the state names another ROM and the tests say so).

mod common;

use ssd_oracle::{Joypad, Machine, MemorySaveStore, NullAudio, SaveStore, joypad_from_bits};

const V34_STATE: &[u8] = include_bytes!("fixtures/v34/sram_counter_f60.ssdstate");
const V34_TRACE: &str = include_str!("fixtures/v34/sram_counter_f60.trace");
const V34_SAV: &[u8] = include_bytes!("fixtures/v34/sram_counter_f180.sav");
/// The frame the earlier release's state was saved at, and the frames its trace covers after it.
const STATE_FRAME: u64 = 60;
const TRACE_FRAMES: u64 = 120;

/// The generator's joypad: a different one on every frame, every button used.
fn varied_input(frame: u64) -> Joypad {
    joypad_from_bits((frame as u16).wrapping_mul(0x9E37) & 0x0FFF)
}

fn pixels_crc(m: &Machine) -> u32 {
    let bytes: Vec<u8> = m.frame().pixels.iter().flat_map(|c| [c.r, c.g, c.b]).collect();
    ssd_rom::crc32(&bytes)
}

/// The machine of this build on the fixtures' ROM, the battle speed off (the sound chip at the hardware's 1/1).
fn machine(saves: Box<dyn SaveStore>) -> Machine {
    let mut m = Machine::new(common::sram_counter_rom(), saves).unwrap();
    m.set_apu_clock_ratio(1, 1);
    m
}

#[test]
fn a_v34_save_state_loads_and_plays_on_frame_for_frame_as_in_v34() {
    let mut m = machine(Box::new(MemorySaveStore::default()));
    m.load_state(V34_STATE).expect("a v34 save state loads in this build");
    assert_eq!(m.frame_count(), STATE_FRAME);
    assert!(
        m.save_state() == V34_STATE,
        "the loaded state is saved again byte for byte as v34 wrote it"
    );

    let expected: Vec<&str> = V34_TRACE.lines().collect();
    assert_eq!(expected.len() as u64, TRACE_FRAMES, "the v34 trace");
    for (i, want) in expected.iter().enumerate() {
        let frame = STATE_FRAME + i as u64;
        m.run_frame(varied_input(frame), &mut NullAudio).unwrap();
        let got = format!("{:08x} {:08x}", ssd_rom::crc32(&m.save_state()), pixels_crc(&m));
        assert_eq!(
            &got, want,
            "frame {frame} after the v34 state differs from v34's own run (state crc, pixels crc)"
        );
    }
}

#[test]
fn a_v34_battery_save_loads_and_the_game_goes_on_from_it() {
    let mut store = MemorySaveStore::default();
    store.persist("sav", V34_SAV).unwrap();
    let mut m = machine(Box::new(store));
    assert_eq!(m.sram(), Some(V34_SAV), "the v34 battery save is the SRAM at power-on");
    let counter = V34_SAV[0];
    // The ROM counts the vblanks in SRAM byte 0: the count goes on from the earlier release's.
    for f in 0..5 {
        m.run_frame(varied_input(f), &mut NullAudio).unwrap();
        let now = m.sram().unwrap()[0];
        if now != counter {
            assert_eq!(now, counter.wrapping_add(1), "the first count after v34's");
            return;
        }
    }
    panic!("the SRAM counter did not go on from v34's {counter}");
}
