//! The Gameplay page's battle speed against the real game: with the sound chip at 2/3 of its clock in battle at 1.5x
//! (three frames of the game to the sound chip's two, `ssd_game::gameplay::battle_tempo`) and at 1/2 at 2x, the
//! battles of recorded sessions play to their end, every sound effect the game asks for reaches the sound chip, the
//! music commands are the same, and the sound comes out at 2/3 (1/2) of the samples per frame of the game (so at 1.5
//! (2) frames of the game to one of the screen, the samples of one: the same tempo and pitch).
//!
//! Needs the game's ROM and the replays (the reference ones of `.work/replays/` or the secret battles of the
//! regression inputs; a replay of another core is read as recorded with this one, the sessions being the same):
//! `SSD_ROM_PATH=... SSD_REPLAY_DIR=... cargo test -p ssd-game --release --test battle_tempo_real_rom -- --ignored
//! --nocapture` (`SSD_REPLAY_NAMES=a,b` picks replays of the directory, all of them by default).

use ssd_game::gameplay::{BATTLE_TEMPO, BATTLE_TEMPO_2X, in_battle, tempo_apu};
use ssd_oracle::{
    AudioSink, CORE_VERSION, Joypad, Machine, MemorySaveStore, Replay, Stopped, Stops, apply_events,
};
use std::path::{Path, PathBuf};

/// The sound driver's routines in the ROM (bank 4): the effect queue's entry (A: the effect, `$80` empties the
/// queue), its exit when the 8 places are taken (the effect is dropped), the frame's send of the head of the queue to
/// `$2141` (A: the effect), and the check of the frames an effect waits for its echo (A: the frames; 3 gives up).
const QUEUE_EFFECT: u32 = 0x04_85B6;
const QUEUE_FULL: u32 = 0x04_85CB;
const SEND_EFFECT: u32 = 0x04_856D;
const ECHO_WAIT: u32 = 0x04_859E;
/// The routine that turns Goku into a Super Saiyan (`docs/re/data/kaioken-bp.md`, 5).
const SUPER_SAIYAN: u32 = 0x03_85F0;
/// The two RNG seeds (`rng_seed_a`, `rng_seed_b`, `re/symbols.toml`).
const RNG_SEEDS: std::ops::Range<usize> = 0x002F..0x0033;
/// `active_side` bit 3: a special attack (`docs/re/data/battle.md`); the Kaioken flag, `$0C36` bit 7.
const ACTIVE_SIDE: usize = 0x14C2;
const KAIOKEN: usize = 0x0C36;

/// The samples output in a frame, and while asked for, the first [`PCM_LEN`] of a battle's (left and right mixed).
#[derive(Default)]
struct Samples {
    count: u64,
    pcm: Option<Vec<f32>>,
}

/// 10 s of the sound at the default 48 kHz.
const PCM_LEN: usize = 480_000;

impl AudioSink for Samples {
    fn push(&mut self, left: f32, right: f32) {
        self.count += 1;
        if let Some(pcm) = self.pcm.as_mut()
            && pcm.len() < PCM_LEN
        {
            pcm.push(f32::midpoint(left, right));
        }
    }
}

/// What a run saw.
#[derive(Default, Debug)]
struct Seen {
    /// The frames in battle, and the battles (a frame in battle after one that is not): their first frame and their
    /// length (to the end of the run for one still on).
    battle_frames: u64,
    battles: u64,
    battle_spans: Vec<(u64, u64)>,
    /// The effects asked for (queued) and sent to the sound chip, with their frames; the ones dropped with a full
    /// queue, and the ones given up on.
    asked: Vec<u8>,
    sent: Vec<u8>,
    asked_at: Vec<u64>,
    dropped: u64,
    gave_up: u64,
    /// The music commands: the CPU's writes to `$2142` and `$2143`.
    music: Vec<(u32, u8)>,
    /// Frames with a special attack, with the Kaioken on, and the Super Saiyan transformations.
    special_frames: u64,
    kaioken_frames: u64,
    super_saiyan: u64,
    /// Samples out in battle and outside it, and the frames outside it.
    battle_samples: u64,
    field_samples: u64,
    field_frames: u64,
    /// The RNG seeds after each frame.
    rng: Vec<[u8; 4]>,
    /// The first [`PCM_LEN`] samples of the first battle.
    pcm: Vec<f32>,
}

/// The page's battle speeds: their name and tempo (frames of the game, frames of the screen).
const SPEEDS: [(&str, (u32, u32)); 2] = [("1.5x", BATTLE_TEMPO), ("2x", BATTLE_TEMPO_2X)];

/// The samples a frame of the game in battle at `tempo`, of those at 1x: the sound chip's clock (2/3, 1/2).
fn sample_ratio(tempo: (u32, u32)) -> f64 {
    let (num, den) = tempo_apu(tempo);
    f64::from(num) / f64::from(den)
}

/// A machine run frame by frame, with the sound chip at [`tempo_apu`] of `tempo` in battle when there is one, and what
/// it saw.
struct Run {
    machine: Machine,
    tempo: Option<(u32, u32)>,
    seen: Seen,
    was_in_battle: bool,
}

impl Run {
    fn new(mut machine: Machine, tempo: Option<(u32, u32)>) -> Self {
        for address in [QUEUE_EFFECT, QUEUE_FULL, SEND_EFFECT, ECHO_WAIT, SUPER_SAIYAN] {
            machine.set_breakpoint(address);
        }
        machine.set_cpu_write_log(true);
        Self { machine, tempo, seen: Seen::default(), was_in_battle: false }
    }

    /// Runs a frame with the joypad `pad`.
    fn step(&mut self, pad: Joypad) {
        let frame = self.machine.frame_count();
        let battle = in_battle(self.machine.wram());
        if let Some(tempo) = self.tempo {
            let (num, den) = if battle { tempo_apu(tempo) } else { (1, 1) };
            self.machine.set_apu_clock_ratio(num, den);
        }
        let seen = &mut self.seen;
        let first_battle = seen.battles == 0 || (seen.battles == 1 && self.was_in_battle);
        let capture = battle && first_battle && seen.pcm.len() < PCM_LEN;
        let mut samples = Samples { count: 0, pcm: capture.then(|| std::mem::take(&mut seen.pcm)) };
        let stops = Stops { frame_end: true, breakpoint: true, ..Stops::default() };
        loop {
            let stopped = self.machine.run(&stops, &mut |_| pad, &mut samples, 1).unwrap();
            let Stopped::Breakpoint { key } = stopped else { break };
            let a = self.machine.cpu_registers().a.to_le_bytes()[0];
            match key {
                QUEUE_EFFECT => {
                    seen.asked.push(a);
                    seen.asked_at.push(frame);
                }
                QUEUE_FULL => seen.dropped += 1,
                SEND_EFFECT => seen.sent.push(a),
                ECHO_WAIT => seen.gave_up += u64::from(a >= 3),
                SUPER_SAIYAN => seen.super_saiyan += 1,
                _ => unreachable!("breakpoint {key:06X}"),
            }
        }
        if let Some(pcm) = samples.pcm.take() {
            seen.pcm = pcm;
        }
        for (address, value) in self.machine.take_cpu_writes() {
            if address >> 16 & 0x40 == 0 && matches!(address & 0xFFFF, 0x2142 | 0x2143) {
                seen.music.push((address & 0xFFFF, value));
            }
        }
        let wram = self.machine.wram();
        seen.rng.push(wram[RNG_SEEDS].try_into().unwrap());
        if battle {
            seen.battle_frames += 1;
            if self.was_in_battle {
                seen.battle_spans.last_mut().unwrap().1 += 1;
            } else {
                seen.battles += 1;
                seen.battle_spans.push((frame, 1));
            }
            seen.battle_samples += samples.count;
            seen.special_frames += u64::from(wram[ACTIVE_SIDE] & 0x08 != 0);
            seen.kaioken_frames += u64::from(wram[KAIOKEN] & 0x80 != 0);
        } else {
            seen.field_samples += samples.count;
            seen.field_frames += 1;
        }
        self.was_in_battle = battle;
    }
}

fn rom() -> Vec<u8> {
    let path = std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH");
    let bytes = std::fs::read(path).unwrap();
    let rom = ssd_rom::normalize(&bytes, None).unwrap();
    assert_eq!(ssd_rom::crc32(&rom.bytes), ssd_rom::CANONICAL_CRC32, "not the canonical ROM");
    rom.bytes
}

fn replays() -> Vec<PathBuf> {
    let dir = PathBuf::from(std::env::var("SSD_REPLAY_DIR").expect("SSD_REPLAY_DIR"));
    let mut paths: Vec<PathBuf> = match std::env::var("SSD_REPLAY_NAMES") {
        Ok(names) => names.split(',').map(|name| dir.join(format!("{name}.ssdreplay"))).collect(),
        Err(_) => std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "ssdreplay"))
            .collect(),
    };
    paths.sort();
    paths
}

fn read_replay(path: &Path, rom: &[u8]) -> Replay {
    let bytes = std::fs::read(path).unwrap();
    let (_, bytes) = Replay::rebase(&bytes, CORE_VERSION).unwrap();
    Replay::from_bytes(&bytes, ssd_rom::crc32(rom), CORE_VERSION).unwrap()
}

/// Plays `replay`, with the sound chip at [`tempo_apu`] of `tempo` in battle when there is one.
fn play(replay: &Replay, rom: &[u8], tempo: Option<(u32, u32)>) -> Seen {
    let mut run = Run::new(replay.power_on(rom.to_vec()).unwrap(), tempo);
    while run.machine.frame_count() < replay.frames() {
        let frame = run.machine.frame_count();
        apply_events(&mut run.machine, replay.events_at(frame)).unwrap();
        run.step(replay.input(frame));
    }
    run.seen
}

/// The effects asked for that the queue keeps, or sent from it: all but `$80` (which empties the queue when asked
/// for, and is sent by the driver's other users of the head of the queue) and 0 (no effect; the send of a 0 between
/// two equal effects, or after the last one, is the driver's).
fn effects(values: &[u8]) -> Vec<u8> {
    values.iter().copied().filter(|&effect| effect != 0 && effect != 0x80).collect()
}

/// The `$80`s sent.
fn stops_sent(sent: &[u8]) -> usize {
    sent.iter().fold(0, |count, &value| count + usize::from(value == 0x80))
}

/// The longest battle of a session, in frames.
fn longest(seen: &Seen) -> u64 {
    seen.battle_spans.iter().map(|span| span.1).max().unwrap_or(0)
}

#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH) and replays (SSD_REPLAY_DIR)"]
fn every_effect_reaches_the_sound_chip_and_the_battles_end_at_two_thirds_and_at_half() {
    let rom = rom();
    let mut failures = Vec::new();
    let mut check = |ok: bool, what: String| {
        if !ok {
            failures.push(what);
        }
    };
    let (mut battles, mut special, mut kaioken, mut super_saiyan) =
        ([0; 2], [0; 2], [0; 2], [0; 2]);
    for path in replays() {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let replay = read_replay(&path, &rom);
        let off = play(&replay, &rom, None);
        for (speed_index, (label, tempo)) in SPEEDS.into_iter().enumerate() {
            let on = play(&replay, &rom, Some(tempo));
            // The first frame after which the RNG seeds differ: until then the game decided the same at both speeds.
            let same_until = off
                .rng
                .iter()
                .zip(&on.rng)
                .position(|(a, b)| a != b)
                .map_or(replay.frames(), |f| f as u64);
            let asked_before = |seen: &Seen| {
                effects(
                    &seen
                        .asked
                        .iter()
                        .zip(&seen.asked_at)
                        .filter(|(_, at)| **at < same_until)
                        .map(|(e, _)| *e)
                        .collect::<Vec<_>>(),
                )
            };
            let per_frame = |samples: u64, frames: u64| samples as f64 / frames.max(1) as f64;
            let battle_ratio = per_frame(on.battle_samples, on.battle_frames)
                / per_frame(off.battle_samples, off.battle_frames);
            let field_ratio = per_frame(on.field_samples, on.field_frames)
                / per_frame(off.field_samples, off.field_frames);
            println!(
                "{name} {label}: {} frames, the same RNG until {same_until}; battles off {} on {} (frames {} / {}, longest {} / {}); \
                 effects queued off {} on {}, sent off {} on {}, dropped {} / {}, given up {} / {}; $80s sent {} / {}; \
                 music commands {} / {} (same {}); special-attack frames {}, Kaioken frames {}, Super Saiyan {}; \
                 samples a frame in battle {:.2} / {:.2} ({battle_ratio:.4}), outside {:.2} / {:.2} ({field_ratio:.4})",
                replay.frames(),
                off.battles,
                on.battles,
                off.battle_frames,
                on.battle_frames,
                longest(&off),
                longest(&on),
                effects(&off.asked).len(),
                effects(&on.asked).len(),
                effects(&off.sent).len(),
                effects(&on.sent).len(),
                off.dropped,
                on.dropped,
                off.gave_up,
                on.gave_up,
                stops_sent(&off.sent),
                stops_sent(&on.sent),
                off.music.len(),
                on.music.len(),
                off.music == on.music,
                on.special_frames,
                on.kaioken_frames,
                on.super_saiyan,
                per_frame(off.battle_samples, off.battle_frames),
                per_frame(on.battle_samples, on.battle_frames),
                per_frame(off.field_samples, off.field_frames),
                per_frame(on.field_samples, on.field_frames),
            );
            // Every effect queued is sent, in order, at either speed: nothing dropped, nothing given up on.
            for (speed, seen) in [("1x", &off), (label, &on)] {
                check(
                    effects(&seen.sent) == effects(&seen.asked),
                    format!("{name} {speed}: the effects sent are not the ones queued"),
                );
                check(
                    (seen.dropped, seen.gave_up) == (0, 0),
                    format!("{name} {speed}: effects dropped or given up on"),
                );
            }
            // As long as the game decided the same, it asked for the same effects (the sound chip's later answers only
            // move a send, and the idle RNG with it, by a frame).
            check(
                asked_before(&on) == asked_before(&off),
                format!("{name} {label}: other effects asked for before frame {same_until}"),
            );
            // The music is the session's: the same commands in the same order. It holds across the RNG's differences
            // because these scripts keep to one route of the story; a new replay that turns it red: check first that both
            // runs went the same way.
            check(on.music == off.music, format!("{name} {label}: the music commands differ"));
            // A session that decided the same throughout has the same battles; none is stuck (a battle that never ends
            // would hold the rest of the session).
            if same_until == replay.frames() {
                check(on.battles == off.battles, format!("{name} {label}: other battles"));
                check(
                    stops_sent(&on.sent) == stops_sent(&off.sent),
                    format!("{name} {label}: other $80s sent"),
                );
            }
            check(
                longest(&on) <= 2 * longest(&off).max(1),
                format!(
                    "{name} {label}: a battle of {} frames, at 1x {}",
                    longest(&on),
                    longest(&off)
                ),
            );
            // The sound: 2/3 (1/2) of the samples a frame of the game in battle (so 1.5 (2) frames to one of the screen
            // give the samples of one), all of them outside.
            if off.battle_frames > 0 {
                check(
                    (battle_ratio - sample_ratio(tempo)).abs() < 0.002,
                    format!("{name} {label}: {battle_ratio} of the samples a battle frame"),
                );
            }
            check(
                (field_ratio - 1.0).abs() < 0.002,
                format!("{name} {label}: {field_ratio} of the samples a frame outside battles"),
            );
            battles[speed_index] += on.battles;
            special[speed_index] += u64::from(on.special_frames > 0);
            kaioken[speed_index] += u64::from(on.kaioken_frames > 0);
            super_saiyan[speed_index] += on.super_saiyan;
        }
    }
    for (i, (label, _)) in SPEEDS.into_iter().enumerate() {
        println!(
            "battles at {label} {}; sessions with special attacks {}, with the Kaioken {}; Super Saiyan \
             transformations {}",
            battles[i], special[i], kaioken[i], super_saiyan[i]
        );
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The power-on to the map of slot 1 of a save (`xtask replay-from-save`'s Continue).
const CONTINUE: &str =
    "1700 -\n4 start\n60 -\n4 right\n20 -\n4 start\n100 -\n4 a\n200 -\n8 a\n200 -\n";
/// Goku's party record (the first): its unit, level and KI (`docs/re/data/kaioken-bp.md`).
const GOKU_UNIT: u32 = 0x7E_0B01;
/// The game mode, the random battle's countdown and its switch (as the regression harness's `battle` starts one).
const GAME_MODE: usize = 0x0024;
const ENCOUNTER_STEPS: u32 = 0x7E_0D8A;
const ENCOUNTER_ON: u32 = 0x7E_0D8B;
/// The battle's step that waits for the player's card (`$0D5F` = `$11`).
const CARD_STEP: usize = 0x0D5F;

fn no_keys() -> Joypad {
    Joypad::default()
}

fn a_key() -> Joypad {
    Joypad { a: true, ..Joypad::default() }
}

impl Run {
    fn idle(&mut self, frames: u32) {
        for _ in 0..frames {
            self.step(no_keys());
        }
    }

    /// A press as the regression harness makes one: 4 frames held, 16 let go.
    fn press(&mut self, pad: Joypad) {
        for _ in 0..4 {
            self.step(pad);
        }
        self.idle(16);
    }
}

/// The regression harness's Kaioken battle (`kaio.sh`): Goku of the save (made unit `$39`, "knows the Kaioken", level
/// 30, KI 80) meets a random battle, picks his 必 card and the fourth move of its list, the Kaioken; then A every 30
/// frames until the battle is over (at most `budget` frames).
fn kaioken_battle(rom: &[u8], save: &[u8], tempo: Option<(u32, u32)>, budget: u64) -> Seen {
    let mut saves = MemorySaveStore::default();
    saves.0.insert("sav".to_owned(), save.to_vec());
    let mut run = Run::new(Machine::new(rom.to_vec(), Box::new(saves)).unwrap(), tempo);
    for pad in ssd_game::scripts::parse_timeline(CONTINUE).unwrap() {
        run.step(pad);
    }
    // Unit $39 at level 30 (`$0B01`, `$0B02`) with 80 KI (`$0B05`).
    for (offset, value) in [(0, 0x39), (1, 0x1E), (4, 0x50)] {
        run.machine.write_wram(GOKU_UNIT + offset, &[value]).unwrap();
    }
    let wram = |run: &Run, address: usize| run.machine.wram()[address];
    // On the map (game mode 0), then a random battle.
    let mut frame = 0_u32;
    while wram(&run, GAME_MODE) & 0x7F != 0 && frame < 2000 {
        run.step(no_keys());
        frame += 1;
    }
    while !in_battle(run.machine.wram()) && frame < 6000 {
        if wram(&run, GAME_MODE) & 0x7F == 0 {
            run.machine.write_wram(ENCOUNTER_ON, &[1]).unwrap();
            run.machine.write_wram(ENCOUNTER_STEPS, &[16]).unwrap();
        }
        let right = (frame / 24).is_multiple_of(2);
        run.step(Joypad { right, left: !right, ..Joypad::default() });
        frame += 1;
    }
    assert!(
        in_battle(run.machine.wram()),
        "a battle comes: after {frame} frames, mode {:#04x}, in_battle {:#04x}",
        wram(&run, GAME_MODE),
        wram(&run, 0x0364)
    );
    run.idle(240);
    for _ in 0..40 {
        if wram(&run, CARD_STEP) == 0x11 {
            break;
        }
        for i in 0..44 {
            run.step(if i < 4 { a_key() } else { no_keys() });
            if i >= 4 && wram(&run, CARD_STEP) == 0x11 {
                break;
            }
        }
    }
    run.idle(45);
    let (a, right, down) = (
        a_key(),
        Joypad { right: true, ..Joypad::default() },
        Joypad { down: true, ..Joypad::default() },
    );
    let picks = [
        (a, 40),
        (a, 40),
        (a, 60),
        (right, 16),
        (right, 16),
        (a, 40),
        (down, 20),
        (down, 20),
        (down, 20),
        (down, 20),
    ];
    for (pad, wait) in picks {
        run.press(pad);
        run.idle(wait);
    }
    let end = run.machine.frame_count() + budget;
    while in_battle(run.machine.wram()) && run.machine.frame_count() < end {
        run.press(a_key());
        run.idle(30);
    }
    run.seen
}

/// The Kaioken battle at 1x, 1.5x and 2x: it ends, the Kaioken goes on, and every effect asked for reaches the sound chip.
///
/// Needs the save the regression harness starts its battles from too:
/// `SSD_ROM_PATH=... SSD_SAVE_PATH=path/to/your.srm cargo test -p ssd-game --release
/// --test battle_tempo_real_rom kaioken -- --ignored --nocapture`.
#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH) and a save (SSD_SAVE_PATH)"]
fn the_kaioken_battle_ends_with_every_effect_at_one_and_a_half_and_at_two() {
    let rom = rom();
    let save = std::fs::read(std::env::var("SSD_SAVE_PATH").expect("SSD_SAVE_PATH")).unwrap();
    let off = kaioken_battle(&rom, &save, None, 20_000);
    let runs: Vec<_> = SPEEDS
        .into_iter()
        .map(|(label, tempo)| (label, tempo, kaioken_battle(&rom, &save, Some(tempo), 20_000)))
        .collect();
    for (speed, seen) in [("1x", &off)].into_iter().chain(runs.iter().map(|(l, _, s)| (*l, s))) {
        let per_frame = seen.battle_samples as f64 / seen.battle_frames as f64;
        println!(
            "Kaioken battle {speed}: battle frames {}, special-attack frames {}, Kaioken frames {}; effects queued {}, \
             sent {}, dropped {}, given up {}; music commands {}; samples a battle frame {per_frame:.2}",
            seen.battle_frames,
            seen.special_frames,
            seen.kaioken_frames,
            effects(&seen.asked).len(),
            effects(&seen.sent).len(),
            seen.dropped,
            seen.gave_up,
            seen.music.len(),
        );
        assert!(seen.kaioken_frames > 0, "{speed}: the Kaioken went on");
        assert_eq!(seen.battles, 1, "{speed}: one battle");
        assert!(
            seen.battle_spans[0].1 < seen.battle_frames + 1 && seen.field_frames > 0,
            "{speed}: it ended"
        );
        assert_eq!(
            effects(&seen.sent),
            effects(&seen.asked),
            "{speed}: the effects sent are the ones queued"
        );
        assert_eq!((seen.dropped, seen.gave_up), (0, 0), "{speed}: effects dropped or given up on");
    }
    for (label, tempo, on) in &runs {
        assert_eq!(on.music, off.music, "{label}: the music commands");
        let ratio = (on.battle_samples as f64 / on.battle_frames as f64)
            / (off.battle_samples as f64 / off.battle_frames as f64);
        assert!(
            (ratio - sample_ratio(*tempo)).abs() < 0.002,
            "{label}: {} of the samples a battle frame: {ratio}",
            sample_ratio(*tempo)
        );
    }
}

/// Pearson's correlation of the first `len` samples of `a` and `b`.
fn correlation(a: &[f32], b: &[f32], len: usize) -> f64 {
    let (a, b) = (&a[..len], &b[..len]);
    let mean = |x: &[f32]| x.iter().map(|&v| f64::from(v)).sum::<f64>() / len as f64;
    let (ma, mb) = (mean(a), mean(b));
    let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (&x, &y) in a.iter().zip(b) {
        let (x, y) = (f64::from(x) - ma, f64::from(y) - mb);
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    ab / (aa * bb).sqrt()
}

/// The sign changes of `pcm` a second at 48 kHz (a measure of its pitch).
fn crossings_a_second(pcm: &[f32]) -> f64 {
    let changes = pcm.windows(2).filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0)).count();
    changes as f64 / (pcm.len() as f64 / 48_000.0)
}

/// The music of a battle at 1.5x and at 2x is the music at 1x: the first 10 s of sound of the battle (480 000 samples
/// at 48 kHz, 601 frames of the game at 1x, 901 at 1.5x and 1202 at 2x, all 10 s on the screen) are the same sound,
/// sample for sample but for the effects' moments, and not the 1x sound sped up 1.5 (2) times.
///
/// `SSD_ROM_PATH=... SSD_REPLAY_DIR=... SSD_REPLAY_NAMES=secret-ssj-frieza cargo test -p ssd-game --release --test
/// battle_tempo_real_rom pitch -- --ignored --nocapture` (`SSD_PCM_DIR` also writes both as raw 32-bit floats).
#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH) and replays (SSD_REPLAY_DIR)"]
fn the_battle_music_keeps_its_pitch_and_tempo() {
    let rom = rom();
    for path in replays() {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let replay = read_replay(&path, &rom);
        let off = play(&replay, &rom, None);
        for (label, tempo) in SPEEDS {
            let on = play(&replay, &rom, Some(tempo));
            if off.pcm.len() < PCM_LEN || on.pcm.len() < PCM_LEN {
                println!("{name} {label}: no battle of 10 s");
                continue;
            }
            if let Ok(dir) = std::env::var("SSD_PCM_DIR") {
                for (speed, seen) in [("off", &off), (label, &on)] {
                    let bytes: Vec<u8> = seen.pcm.iter().flat_map(|v| v.to_le_bytes()).collect();
                    std::fs::write(Path::new(&dir).join(format!("{name}-{speed}.f32")), bytes)
                        .unwrap();
                }
            }
            // The 1x sound at f/r times its speed: what the sound would be if the music ran with the frames.
            let (f, r) = (tempo.0 as usize, tempo.1 as usize);
            let sped_up: Vec<f32> =
                (0..PCM_LEN).map(|i| off.pcm[(i * f / r).min(PCM_LEN - 1)]).collect();
            let same = correlation(&on.pcm, &off.pcm, PCM_LEN);
            let faster = correlation(&on.pcm, &sped_up, PCM_LEN * r / f);
            let (crossings_on, crossings_off) =
                (crossings_a_second(&on.pcm), crossings_a_second(&off.pcm));
            println!(
                "{name}: the first 10 s of the battle's sound, {label} against 1x: correlation {same:.4}, against 1x \
                 sped up {label} {faster:.4}; sign changes a second {crossings_on:.0} / {crossings_off:.0}"
            );
            // The music lines up at both speeds (the effects come at other moments: the battle goes faster), a music
            // running with the frames would not; and the pitch is the same.
            assert!(
                same > 0.5 && faster < 0.1,
                "{name} {label}: the same music: {same}, sped up {faster}"
            );
            assert!(
                (crossings_on / crossings_off - 1.0).abs() < 0.1,
                "{name} {label}: the same pitch (sped up would be {label})"
            );
        }
    }
}
