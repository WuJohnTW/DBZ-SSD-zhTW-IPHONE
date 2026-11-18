//! The Japanese voice of Goku's first Super Saiyan transformation: `<data dir>\voice\ssj-transform.wav`, played once, the
//! frame the transformation's flashing starts (he shouts as he transforms). Without the file nothing is played.
//!
//! Only the standard library is used here (the watch, the mixing, the file's place), so this file's tests run on any machine
//! without SDL (`rustc --edition 2024 --test voice.rs`). Reading the WAV is SDL's ([`crate::audio`]).
//!
//! Goku's record: the party's nine records of 32 bytes at `$7E:0B00`, the one in the party (flags `$C0` or more) with
//! character id 0 at `+$1C` (the search of `03:F021`); his unit id is at `+$01` (`docs/re/data/characters.md`). His unit ids
//! (`docs/re/data/kaioken-bp.md`, 1.4): `$00` plain, `$39` knows the Kaioken, `$3A` Kaioken on, `$3B` Super Saiyan. The
//! builder `03:85F0` makes him `$3B` once in a game.
//!
//! The transformation (`docs/re/data/kaioken-bp.md`, 6): the battle's trigger `05:8772..87B5` (group `$56` or `$6E`, the
//! acting record Goku's, level 47 or more, a friend fallen) starts the figure script `$68` on side A (`05:879D`) and sets
//! `battle_phase` (`$AB`) to `$1A`, its only writer (`05:87B1`). Phase `$1A` (`05:92C9`) runs the figures until both are
//! done (`05:DFE3` returns to the main loop while `$B8 & $C0` is not `$C0`), then `05:C45F` calls `03:8683`, which builds
//! the Super Saiyan (`$3B`) only when Goku is in the party at level 47 or more, the group is `$56`, and `$01BB` is below
//! `$0D9A` or fewer records of the party than `$0D9A` remain. The script first plays Goku's words, then its command
//! `05:D943` (`LDA #$5F`, `STA $0026,Y`, the game's only `$5F` stored there) sets side A's `+$26` to `$5F`: the next frame
//! the flashing backdrop is drawn, and the shout starts. His unit id becomes `$3B` at the end, about 5.7 s later
//! (342 frames in the reference replay). The voice is started at that command when `03:8683` will build him.
//!
//! The same watch times the lightning the Enhanced picture draws over the whole transformation (the pictures of
//! `<data dir>\art-override\ssj-lightning\`, `ssd_hd::lightning`): [`SsjWatch::lightning_at`] counts its frames from the
//! frame the voice starts at to the one Goku is built a Super Saiyan, by [`Goku::in_transformation`], which the voice's
//! start reads too.

use std::path::{Path, PathBuf};

/// The folder of the data folder the voices are in.
pub const FOLDER: &str = "voice";
/// The voice of the transformation.
pub const SSJ_CLIP: &str = "ssj-transform.wav";

/// The party records (`$7E:0B00`, index `$0B00` of work RAM): nine of 32 bytes.
const PARTY_RECORDS: usize = 0x0B00;
const RECORD_LENGTH: usize = 0x20;
const RECORDS: usize = 9;
const RECORD_FLAGS: usize = 0x00;
const RECORD_UNIT: usize = 0x01;
const RECORD_CHARACTER: usize = 0x1C;
/// A record of the party has its flags at `$C0` or more (`03:F021`).
const IN_PARTY: u8 = 0xC0;
/// Goku's character id.
const GOKU: u8 = 0;
/// Goku's unit ids before the Super Saiyan's (`$3B`): plain, knows the Kaioken, Kaioken on.
const BEFORE: [u8; 3] = [0x00, 0x39, 0x3A];
/// A record's level (`+$02`), and the level `03:8683` asks of Goku (47).
const RECORD_LEVEL: usize = 0x02;
const SSJ_LEVEL: u8 = 0x2F;
/// `battle_phase` (`$AB`) and its transformation phase `$1A`.
const BATTLE_PHASE: usize = 0x00AB;
const TRANSFORMATION_PHASE: u8 = 0x1A;
/// Side A's record (`$1400`) `+$26`, and the `$5F` the transformation's script command `05:D943` writes to it.
const SIDE_A_26: usize = 0x1426;
const FLASHING: u8 = 0x5F;
/// `encounter_group_index` (`$0EA3`), and the group `03:8683` builds the Super Saiyan in (`05:8772` also lets `$6E` in).
const ENCOUNTER_GROUP: usize = 0x0EA3;
const SSJ_GROUP: u8 = 0x56;
/// `$01BB`, and `$0D9A`: the party size counted by the last fight's set-up (`03:C820..C848`).
const MAP_PARAM: usize = 0x01BB;
const PARTY_SIZE: usize = 0x0D9A;

/// Where the voice of the transformation is: `<data dir>\voice\ssj-transform.wav`.
#[must_use]
pub fn ssj_clip_path(data_dir: &Path) -> PathBuf {
    data_dir.join(FOLDER).join(SSJ_CLIP)
}

/// The voice of the transformation, if the data folder has the file.
#[must_use]
pub fn find_ssj_clip(data_dir: &Path) -> Option<PathBuf> {
    Some(ssj_clip_path(data_dir)).filter(|path| path.is_file())
}

/// The records of the party in `wram` (`$7E:0B00`), nine of 32 bytes.
fn party_records() -> impl Iterator<Item = usize> {
    (0..RECORDS).map(|slot| PARTY_RECORDS + slot * RECORD_LENGTH)
}

/// Where Goku's record is in `wram` (the search of `03:F021`), or `None` when he is not in the party.
fn goku_record(wram: &[u8]) -> Option<usize> {
    party_records().find(|&record| {
        let flags = wram.get(record + RECORD_FLAGS);
        let character = wram.get(record + RECORD_CHARACTER);
        flags.is_some_and(|&f| f >= IN_PARTY) && character == Some(&GOKU)
    })
}

/// Goku's unit id in `wram` (128 KiB from `$7E:0000`), or `None` when he is not in the party.
#[must_use]
pub fn goku_unit(wram: &[u8]) -> Option<u8> {
    wram.get(goku_record(wram)? + RECORD_UNIT).copied()
}

/// Whether `03:8683` will build the Super Saiyan from `wram` (`03:8683..86C1`): Goku in the party at level 47 or more,
/// the group `$56`, and `$01BB` below `$0D9A` or fewer records of the party (flags `$C0` or more) than `$0D9A`.
fn super_saiyan_built(wram: &[u8]) -> bool {
    let byte = |address: usize| wram.get(address).copied();
    let Some(level) = goku_record(wram).and_then(|record| byte(record + RECORD_LEVEL)) else {
        return false;
    };
    let (Some(group), Some(param), Some(size)) =
        (byte(ENCOUNTER_GROUP), byte(MAP_PARAM), byte(PARTY_SIZE))
    else {
        return false;
    };
    let in_party =
        party_records().filter(|&r| byte(r + RECORD_FLAGS).is_some_and(|f| f >= IN_PARTY)).count();
    level >= SSJ_LEVEL && group == SSJ_GROUP && (param < size || in_party < usize::from(size))
}

/// What the watch reads of a frame's work RAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Goku {
    /// Goku's unit id, `None` when he is not in the party.
    pub unit: Option<u8>,
    /// The transformation's flashing is on (phase `$1A`, side A's `+$26` at `$5F`) and `03:8683` will build him.
    pub transforming: bool,
}

impl Goku {
    /// Whether Goku is in his transformation into a Super Saiyan: its flashing is on and he is not built one yet (his unit
    /// id one of [`BEFORE`]). The watch's start of the voice and the lightning ([`SsjWatch::lightning_at`]) both read
    /// this, so the two never disagree on the frames of the transformation.
    #[must_use]
    pub fn in_transformation(self) -> bool {
        self.transforming && self.unit.is_some_and(|u| BEFORE.contains(&u))
    }
}

/// Goku as the watch reads him in `wram` (128 KiB from `$7E:0000`).
#[must_use]
pub fn goku(wram: &[u8]) -> Goku {
    let flashing = wram.get(BATTLE_PHASE) == Some(&TRANSFORMATION_PHASE)
        && wram.get(SIDE_A_26) == Some(&FLASHING);
    Goku { unit: goku_unit(wram), transforming: flashing && super_saiyan_built(wram) }
}

/// Follows Goku frame by frame and says when his first Super Saiyan transformation starts.
#[derive(Debug, Default)]
pub struct SsjWatch {
    /// The frame count and Goku as the last frame ended with him; `None` before the first.
    seen: Option<(u64, Goku)>,
    /// The voice was played: it is played once.
    played: bool,
    /// The frames since the transformation started, while it goes on (the lightning of `art-override\ssj-lightning\`):
    /// `None` outside it, and in a transformation the watch did not see start.
    lightning: Option<u32>,
}

impl SsjWatch {
    /// Notes Goku `now` at the end of frame `frame`, and says whether this is the frame his transformation into a
    /// Super Saiyan starts: it is not on at the frame before and is on now, his unit id one of [`BEFORE`] at both (not
    /// `$3B` yet), for the first time. A frame that does not follow the last one (a state loaded, a reset)
    /// only takes Goku as he is: a game loaded in the middle of the transformation, or with Goku a Super Saiyan
    /// already, plays nothing. The same frame again (a breakpoint stopped it) follows.
    pub fn observe(&mut self, frame: u64, now: Goku) -> bool {
        let last = self.seen.replace((frame, now));
        let Some((last_frame, last)) = last else {
            self.lightning = None;
            return false;
        };
        let follows = frame == last_frame || frame == last_frame.wrapping_add(1);
        let before = |goku: Goku| goku.unit.is_some_and(|u| BEFORE.contains(&u));
        let starts = follows
            && !self.played
            && now.in_transformation()
            && !last.transforming
            && before(last);
        self.played |= starts;
        // The lightning: from the frame the transformation starts (as the voice's, but every time, not once in a run) to
        // its end; a frame that does not follow the last (a state loaded, a reset) stops it, and a transformation it did
        // not see start (a state loaded in the middle of it) has none
        self.lightning = match self.lightning {
            _ if !follows || !now.in_transformation() => None,
            Some(frames) if frame == last_frame => Some(frames),
            Some(frames) => Some(frames.saturating_add(1)),
            None if !last.in_transformation() => Some(0),
            None => None,
        };
        starts
    }

    /// The frames since the transformation the watch follows started, when frame `frame` is the last one it noted and
    /// Goku is in his transformation there (see [`Goku::in_transformation`]); `None` otherwise, and for a frame the watch
    /// has not noted (a state loaded or a reset since: the lightning stops at once).
    #[must_use]
    pub fn lightning_at(&self, frame: u64) -> Option<u32> {
        self.seen.filter(|&(seen, _)| seen == frame).and(self.lightning)
    }
}

/// A voice clip and where it is at: stereo f32 samples at the sound's rate (48 kHz), mixed into the game's sound.
#[derive(Debug, Default)]
pub struct Voice {
    /// The clip, interleaved left and right; empty without one.
    clip: Vec<f32>,
    /// The next sample of `clip` to mix, while it plays.
    at: Option<usize>,
}

impl Voice {
    /// A voice of `clip` (stereo, interleaved; a last odd sample is dropped), not playing.
    #[must_use]
    pub fn new(mut clip: Vec<f32>) -> Self {
        clip.truncate(clip.len() & !1);
        Self { clip, at: None }
    }

    /// Plays the clip from its start (nothing without a clip).
    pub fn start(&mut self) {
        self.at = (!self.clip.is_empty()).then_some(0);
    }

    /// Whether the clip is playing.
    #[must_use]
    pub fn playing(&self) -> bool {
        self.at.is_some()
    }

    /// Adds the next samples of the clip to `pending` (the stereo samples about to be sent, already shaped), each frame
    /// shaped by `shape` (the volume and output of the Audio page) as the game's are. The clip goes on by the samples
    /// mixed: the samples not sent (none pending, a fast-forward's frames) do not move it.
    pub fn mix(&mut self, pending: &mut [f32], shape: impl Fn(f32, f32) -> (f32, f32)) {
        let Some(at) = self.at else { return };
        let rest = &self.clip[at..];
        let n = rest.len().min(pending.len()) & !1;
        for (out, &[left, right]) in
            pending[..n].as_chunks_mut::<2>().0.iter_mut().zip(rest[..n].as_chunks::<2>().0)
        {
            let (left, right) = shape(left, right);
            out[0] += left;
            out[1] += right;
        }
        let at = at + n;
        self.at = (at < self.clip.len()).then_some(at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Work RAM with Goku in record `slot` (in the party) with unit id `unit`, and Piccolo in the first record.
    fn wram_with_goku(slot: usize, unit: u8) -> Vec<u8> {
        let mut wram = vec![0_u8; 0x2_0000];
        let piccolo = PARTY_RECORDS;
        wram[piccolo + RECORD_FLAGS] = 0xC0;
        wram[piccolo + RECORD_UNIT] = 0x02;
        wram[piccolo + RECORD_CHARACTER] = 2;
        let goku = PARTY_RECORDS + slot * RECORD_LENGTH;
        wram[goku + RECORD_FLAGS] = 0xC0;
        wram[goku + RECORD_UNIT] = unit;
        wram[goku + RECORD_CHARACTER] = GOKU;
        wram
    }

    #[test]
    fn goku_is_found_by_his_character_id_in_any_record_of_the_party() {
        assert_eq!(goku_unit(&wram_with_goku(0, 0x39)), Some(0x39));
        assert_eq!(goku_unit(&wram_with_goku(3, 0x3B)), Some(0x3B));
        let mut out = wram_with_goku(3, 0x3B);
        out[PARTY_RECORDS + 3 * RECORD_LENGTH] = 0x00;
        assert_eq!(goku_unit(&out), None, "a record not in the party is not Goku");
        // The empty records of zeros have character id 0 too, but are not in the party
        assert_eq!(goku_unit(&vec![0_u8; 0x2_0000]), None);
        assert_eq!(goku_unit(&[0_u8; 16]), None, "a short slice is not read past its end");
    }

    /// Goku of unit id `unit`, the transformation's flashing off.
    fn plain(unit: impl Into<Option<u8>>) -> Goku {
        Goku { unit: unit.into(), transforming: false }
    }

    /// Goku of unit id `unit`, the transformation's flashing on.
    fn flashing(unit: impl Into<Option<u8>>) -> Goku {
        Goku { unit: unit.into(), transforming: true }
    }

    /// The frames the watch fires at, for Goku at consecutive frames from frame 100.
    fn fired(frames: &[Goku]) -> Vec<usize> {
        let mut watch = SsjWatch::default();
        (0..frames.len()).filter(|&n| watch.observe(100 + n as u64, frames[n])).collect()
    }

    #[test]
    fn the_voice_fires_once_the_frame_the_transformation_starts() {
        for before in BEFORE {
            assert_eq!(
                fired(&[
                    plain(before),
                    plain(before),
                    flashing(before),
                    flashing(before),
                    plain(0x3B)
                ]),
                [2],
                "from {before:#04x}"
            );
        }
        assert_eq!(
            fired(&[plain(0x3A), plain(0x3A), plain(0x3B), plain(0x3B)]),
            Vec::<usize>::new(),
            "not when his unit id becomes the Super Saiyan's, at the end of the transformation"
        );
        assert_eq!(
            fired(&[plain(0x00), flashing(0x00), plain(0x3B), plain(0x00), flashing(0x00)]),
            [1],
            "the second time is not played"
        );
        assert_eq!(
            fired(&[flashing(0x00), flashing(0x00), plain(0x3B)]),
            Vec::<usize>::new(),
            "a game loaded in the middle of the transformation plays nothing"
        );
        assert_eq!(
            fired(&[plain(0x3B), flashing(0x3B)]),
            Vec::<usize>::new(),
            "a Super Saiyan already plays nothing"
        );
        assert_eq!(
            fired(&[plain(None), flashing(None)]),
            Vec::<usize>::new(),
            "Goku not in the party"
        );
        assert_eq!(
            fired(&[plain(0x3C), flashing(0x3C)]),
            Vec::<usize>::new(),
            "only from his forms before it"
        );
        assert_eq!(
            fired(&[plain(None), flashing(0x00)]),
            Vec::<usize>::new(),
            "Goku joining in the middle of it"
        );
    }

    /// The lightning's count at each of `frames` (Goku at consecutive frames from frame 100).
    fn lit(frames: &[Goku]) -> Vec<Option<u32>> {
        let mut watch = SsjWatch::default();
        (0..frames.len())
            .map(|n| {
                watch.observe(100 + n as u64, frames[n]);
                watch.lightning_at(100 + n as u64)
            })
            .collect()
    }

    #[test]
    fn the_lightning_counts_the_frames_of_the_transformation_from_its_start_to_its_end() {
        assert_eq!(
            lit(&[
                plain(0x3A),
                flashing(0x3A),
                flashing(0x3A),
                flashing(0x3A),
                flashing(0x3B),
                plain(0x3B)
            ]),
            [None, Some(0), Some(1), Some(2), None, None],
            "from the frame the voice starts at to the one he is built a Super Saiyan"
        );
        assert_eq!(
            lit(&[plain(0x00), flashing(0x00), plain(0x00), flashing(0x00), flashing(0x00)]),
            [None, Some(0), None, Some(0), Some(1)],
            "every transformation, not once in a run as the voice"
        );
        assert_eq!(
            lit(&[flashing(0x00), flashing(0x00), flashing(0x00)]),
            [None, None, None],
            "a game loaded in the middle of the transformation has none"
        );
        assert_eq!(lit(&[plain(0x3B), flashing(0x3B)]), [None, None], "a Super Saiyan already");
        assert_eq!(lit(&[plain(None), flashing(None)]), [None, None], "Goku not in the party");
    }

    #[test]
    fn a_state_loaded_or_a_reset_stops_the_lightning_at_once() {
        let mut watch = SsjWatch::default();
        watch.observe(500, plain(0x3A));
        watch.observe(501, flashing(0x3A));
        watch.observe(502, flashing(0x3A));
        assert_eq!(watch.lightning_at(502), Some(1));
        assert_eq!(
            watch.lightning_at(40),
            None,
            "a state loaded: the machine is at another frame, none before it runs"
        );
        watch.observe(40, flashing(0x3A));
        assert_eq!(watch.lightning_at(40), None, "a state of the transformation loaded over it");
        watch.observe(41, flashing(0x3A));
        assert_eq!(watch.lightning_at(41), None, "and none after it");
        let mut watch = SsjWatch::default();
        watch.observe(500, plain(0x3A));
        watch.observe(501, flashing(0x3A));
        watch.observe(0, plain(0x3A));
        assert_eq!(watch.lightning_at(0), None, "a reset");
        let mut watch = SsjWatch::default();
        watch.observe(500, plain(0x3A));
        watch.observe(501, flashing(0x3A));
        watch.observe(501, flashing(0x3A));
        assert_eq!(watch.lightning_at(501), Some(0), "the frame a breakpoint stopped, run on");
        watch.observe(502, flashing(0x3A));
        assert_eq!(watch.lightning_at(502), Some(1));
    }

    #[test]
    fn a_frame_that_does_not_follow_the_last_only_takes_goku_as_he_is() {
        let mut watch = SsjWatch::default();
        assert!(!watch.observe(500, plain(0x00)));
        assert!(
            !watch.observe(9_000, flashing(0x00)),
            "a state of the transformation loaded over a plain Goku"
        );
        let mut watch = SsjWatch::default();
        assert!(!watch.observe(500, plain(0x00)));
        assert!(!watch.observe(20, plain(0x00)), "a state of the past loaded");
        assert!(watch.observe(21, flashing(0x00)), "the transformation after it plays");
        let mut watch = SsjWatch::default();
        assert!(!watch.observe(500, plain(0x3A)));
        assert!(watch.observe(500, flashing(0x3A)), "the frame a breakpoint stopped, run on");
    }

    /// Work RAM of the Frieza fight with Goku (record 1, unit id `unit`, level 47) the transformation can build: group
    /// `$56`, `$01BB` 0 below the party size 3 of `$0D9A`, in phase `$1A`, with side A's `+$26` at `side_a_26`.
    fn wram_of_the_transformation(unit: u8, side_a_26: u8) -> Vec<u8> {
        let mut wram = wram_with_goku(1, unit);
        wram[PARTY_RECORDS + RECORD_LENGTH + RECORD_LEVEL] = 47;
        wram[ENCOUNTER_GROUP] = 0x56;
        wram[MAP_PARAM] = 0;
        wram[PARTY_SIZE] = 3;
        wram[BATTLE_PHASE] = 0x1A;
        wram[SIDE_A_26] = side_a_26;
        wram
    }

    #[test]
    fn the_transformation_is_on_with_the_flashing_of_phase_1a_when_03_8683_will_build_him() {
        let on = wram_of_the_transformation(0x00, 0x5F);
        assert_eq!(goku(&on), flashing(0x00));
        assert_eq!(
            goku(&wram_of_the_transformation(0x00, 0x70)),
            plain(0x00),
            "Goku's words, before the flashing"
        );
        let mut other_phase = on.clone();
        other_phase[BATTLE_PHASE] = 0x04;
        assert_eq!(goku(&other_phase), plain(0x00), "a `$5F` out of phase `$1A`");
        let mut group_6e = on.clone();
        group_6e[ENCOUNTER_GROUP] = 0x6E;
        assert_eq!(
            goku(&group_6e),
            plain(0x00),
            "group `$6E`: the script plays, 03:8683 does not build him"
        );
        let mut low = on.clone();
        low[PARTY_RECORDS + RECORD_LENGTH + RECORD_LEVEL] = 46;
        assert_eq!(goku(&low), plain(0x00), "below level 47");
        let mut counted = on.clone();
        counted[MAP_PARAM] = 3;
        assert_eq!(
            goku(&counted),
            flashing(0x00),
            "$01BB not below, but 2 records of the party below 3"
        );
        counted[PARTY_SIZE] = 2;
        counted[MAP_PARAM] = 2;
        assert_eq!(
            goku(&counted),
            plain(0x00),
            "$01BB not below and 2 records of the party, not below 2"
        );
        let mut away = on.clone();
        away[PARTY_RECORDS + RECORD_LENGTH + RECORD_FLAGS] = 0x00;
        assert_eq!(goku(&away), Goku::default(), "Goku not in the party");
        assert_eq!(goku(&[0_u8; 16]), Goku::default(), "a short slice is not read past its end");
    }

    #[test]
    fn the_watch_reads_goku_from_work_ram_frame_by_frame() {
        let mut watch = SsjWatch::default();
        let mut built = wram_of_the_transformation(0x3B, 0x5F);
        built[BATTLE_PHASE] = 0x01;
        let frames = [
            wram_of_the_transformation(0x3A, 0x70),
            wram_of_the_transformation(0x3A, 0x5F),
            wram_of_the_transformation(0x3A, 0x5F),
            built,
        ];
        let fired: Vec<bool> = frames
            .iter()
            .zip(1_u64..)
            .map(|(wram, frame)| watch.observe(frame, goku(wram)))
            .collect();
        assert_eq!(fired, [false, true, false, false]);
    }

    /// The Audio page's shaping at volume `gain`, stereo.
    fn at_volume(gain: f32) -> impl Fn(f32, f32) -> (f32, f32) {
        move |left, right| (left * gain, right * gain)
    }

    #[test]
    fn the_clip_is_mixed_at_the_volume_until_it_ends() {
        // 3 stereo frames of the clip
        let mut voice = Voice::new(vec![0.5, -0.5, 0.25, -0.25, 1.0, -1.0]);
        let mut pending = vec![0.125_f32; 4];
        voice.mix(&mut pending, at_volume(1.0));
        assert_eq!(pending, [0.125; 4], "not playing: nothing mixed");
        voice.start();
        assert!(voice.playing());
        voice.mix(&mut pending, at_volume(0.5));
        assert_eq!(pending, [0.375, -0.125, 0.25, 0.0], "added to the game's sound at the volume");
        let mut next = vec![0.0_f32; 4];
        voice.mix(&mut next, at_volume(1.0));
        assert_eq!(next, [1.0, -1.0, 0.0, 0.0], "the rest of the clip, then nothing");
        assert!(!voice.playing(), "the clip ended");
        let mut after = vec![0.0_f32; 4];
        voice.mix(&mut after, at_volume(1.0));
        assert_eq!(after, [0.0; 4], "an ended clip mixes nothing");
    }

    #[test]
    fn the_clip_goes_on_by_the_samples_sent_only() {
        let clip: Vec<f32> = (1..=8).map(|n| n as f32).collect();
        let mut voice = Voice::new(clip.clone());
        voice.start();
        let mut heard = Vec::new();
        // A frame trimmed for the latency before the clip is mixed (its oldest samples drained), a fast-forward's frame
        // that sends nothing, an odd pending: every sample of the clip is heard once, in order
        for frame in [vec![0.0; 6], vec![], vec![0.0; 2], vec![0.0; 3], vec![0.0; 10]] {
            let mut pending: Vec<f32> = frame;
            let drop = pending.len().min(2) & !1;
            pending.drain(..drop);
            voice.mix(&mut pending, at_volume(1.0));
            heard.extend(pending.iter().copied().filter(|&s| s != 0.0));
        }
        assert_eq!(heard, clip);
        assert!(!voice.playing());
    }

    #[test]
    fn a_clip_of_an_odd_length_drops_its_last_sample_and_an_empty_one_never_plays() {
        let mut voice = Voice::new(vec![0.5, 0.5, 0.5]);
        voice.start();
        let mut pending = vec![0.0_f32; 4];
        voice.mix(&mut pending, at_volume(1.0));
        assert_eq!(pending, [0.5, 0.5, 0.0, 0.0]);
        let mut none = Voice::default();
        none.start();
        assert!(!none.playing());
    }

    #[test]
    fn a_missing_clip_is_none_and_the_file_is_in_the_voice_folder() {
        let dir = std::env::temp_dir().join(format!("ssd-voice-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(find_ssj_clip(&dir), None, "no data folder");
        std::fs::create_dir_all(dir.join("voice").join(SSJ_CLIP)).unwrap();
        assert_eq!(find_ssj_clip(&dir), None, "a folder of the clip's name is not the clip");
        std::fs::remove_dir(dir.join("voice").join(SSJ_CLIP)).unwrap();
        std::fs::write(dir.join("voice").join(SSJ_CLIP), b"RIFF").unwrap();
        assert_eq!(find_ssj_clip(&dir), Some(dir.join("voice").join("ssj-transform.wav")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
