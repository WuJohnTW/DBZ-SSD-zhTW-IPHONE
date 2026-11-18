use crate::options::audio::AudioSettings;
use crate::voice::{self, Voice};
use ssd_oracle::AudioSink;
use std::path::Path;

const FREQUENCY: u64 = 48_000;
/// The queue the rate control keeps: room for a stall of the window (a picture drawn late, the system busy) of up to
/// about 100 ms before the device runs dry. The sound is that much behind the picture.
const TARGET_MS: f32 = 120.0;
/// The most queued plus pending sound: the target and 90 ms over it, for the window frames that come close together
/// (vsync's jitter, two frames of the game in one at the battle speed) without cutting samples. A window frame brings
/// at most the 4 frames of the game of the pacer's catch-up (`app.rs` `CATCH_UP`), each `48000 / 60.0988 x r / f`
/// samples at a battle tempo of f frames to r refreshes: 4 x 798.7 (66.6 ms) at 1x, 4 x 532.5 (44.4 ms) at 1.5x,
/// 4 x 399.4 (33.3 ms) at 2x, all under the 90 ms; a refresh brings 798.7 samples at every speed.
const MAX_LATENCY_MS: usize = TARGET_MS as usize + 90;
/// The most the playback speed is bent to get back to [`TARGET_MS`]: half a percent, which no ear hears.
const MAX_BEND: f32 = 0.005;
/// A queue a frame finds this low before its samples go in has (nearly) run dry: the frame is counted.
const DRY_MS: f32 = 2.0;
/// How often the sound's health goes to the log.
const REPORT_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// What the sound did since the last report: for the log, to tell the device, the game and the queue apart.
#[derive(Debug)]
struct Health {
    since: std::time::Instant,
    /// Stereo floats the game made.
    floats: usize,
    /// Frames flushed, and those that found the queue dry.
    frames: u32,
    dry: u32,
    /// The least and the most queue a frame found, in milliseconds.
    least: f32,
    most: f32,
}

impl Health {
    fn new() -> Self {
        Self {
            since: std::time::Instant::now(),
            floats: 0,
            frames: 0,
            dry: 0,
            least: f32::MAX,
            most: 0.0,
        }
    }
}

/// Milliseconds of `bytes` of stereo f32 samples.
fn ms_of(bytes: usize) -> f32 {
    bytes as f32 / (FREQUENCY as f32 * 2.0 * 4.0) * 1000.0
}

/// The speed SDL plays the queue at for a queue of `queued_ms`: faster above the target, slower below, by up to
/// [`MAX_BEND`]. The game makes its samples on the clock of its frames, the device takes them on its own (and a
/// phone's frames are not a desktop's), so without the bend the queue drifts until it runs dry (gaps: crackle) or
/// overflows (cut samples: clicks).
pub(crate) fn rate_ratio(queued_ms: f32) -> f32 {
    bend_toward(TARGET_MS, queued_ms)
}

/// The playback speed for a queue of `queued_ms` kept at `target_ms` (see [`rate_ratio`]).
fn bend_toward(target_ms: f32, queued_ms: f32) -> f32 {
    1.0 + MAX_BEND * ((queued_ms - target_ms) / target_ms).clamp(-1.0, 1.0)
}

/// Keeps queued plus pending audio under `MAX_LATENCY_MS` by dropping the oldest pending samples.
///
/// Frame pacing is driven by the clock, so the SDL queue can drift away from the emulator's
/// sample production; this is the drift rule. Whole stereo frames are dropped, never a single
/// channel.
pub(crate) fn trim_for_latency(queued_bytes: usize, frequency: u64, pending: &mut Vec<f32>) {
    let max_bytes = (frequency as usize * MAX_LATENCY_MS / 1000) * 2 * 4;
    let total = queued_bytes + pending.len() * 4;
    if total > max_bytes {
        let excess_floats = ((total - max_bytes) / 4).min(pending.len());
        let excess_floats = excess_floats - excess_floats % 2;
        pending.drain(..excess_floats);
    }
}

/// Keep the bound stream and its logical playback device alive for the same run.
struct Playback {
    stream: sdl3::audio::AudioStreamOwner,
    #[cfg(target_os = "android")]
    _device: sdl3::audio::AudioDevice,
}

#[cfg(target_os = "android")]
fn open_android(sdl: &sdl3::Sdl, source: &sdl3::audio::AudioSpec) -> Result<Playback, String> {
    // The default Android backend can initialise even when opening the playback device fails.
    // Try each backend with its own subsystem lifetime so that a failed one is fully closed.
    const DRIVER: &str = "SDL_AUDIO_DRIVER";
    let configured = sdl3::hint::get(DRIVER);
    let requested = configured.as_deref().map(|driver| [driver]);
    let backends = requested.as_ref().map_or(&["openslES", "AAudio"][..], |one| &one[..]);
    let request = sdl3::audio::AudioSpec {
        freq: source.freq,
        channels: source.channels,
        format: Some(sdl3::audio::AudioFormat::s16_sys()),
    };
    let mut last_error = String::new();
    for backend in backends {
        if configured.is_none() {
            sdl3::hint::set(DRIVER, backend);
        }
        let result = (|| {
            let audio = sdl.audio().map_err(|error| error.to_string())?;
            let device = audio.open_playback_device(&request).map_err(|error| error.to_string())?;
            let stream = audio
                .new_playback_stream(source, Some(&request))
                .map_err(|error| error.to_string())?;
            device.bind_stream(&stream).map_err(|error| error.to_string())?;
            start_stream(&stream)?;
            log::info!("Android audio backend: {}", audio.current_audio_driver());
            Ok(Playback { stream, _device: device })
        })();
        if configured.is_none() {
            // SDL has already selected (or rejected) this backend. Limit the hint to this attempt.
            unsafe { sdl3::sys::hints::SDL_ResetHint(c"SDL_AUDIO_DRIVER".as_ptr()) };
        }
        match result {
            Ok(playback) => return Ok(playback),
            Err(error) => {
                log::warn!("Android audio backend {backend} failed: {error}");
                last_error = error;
            }
        }
    }
    Err(last_error)
}

/// Audio output: collects the samples of a frame and queues them to an SDL3 playback stream.
pub struct Audio {
    playback: Option<Playback>,
    pending: Vec<f32>,
    settings: AudioSettings,
    /// The queue at the last frame's end, in milliseconds.
    queued_ms: f32,
    /// The frames that found the queue dry (under [`DRY_MS`]): each may be a gap the ear hears.
    underruns: u32,
    health: Health,
    /// The voice of Goku's first Super Saiyan transformation ([`voice`]), mixed into the game's sound while it plays.
    voice: Voice,
}

fn open_with<F>(open: F) -> Audio
where
    F: FnOnce() -> Result<Playback, String>,
{
    let playback = open().inspect_err(|e| log::warn!("audio unavailable, running muted: {e}")).ok();
    Audio {
        pending: if playback.is_some() { Vec::with_capacity(4096) } else { Vec::new() },
        playback,
        settings: AudioSettings::default(),
        queued_ms: 0.0,
        underruns: 0,
        health: Health::new(),
        voice: Voice::default(),
    }
}

/// Of the stereo f32 samples about to be sent, the ones over the latency cap dropped ([`trim_for_latency`]), then the voice
/// that plays mixed into the rest, shaped by `settings` as the game's samples are: the clip goes on by the samples sent, so
/// none of it is trimmed away.
fn prepare(queued: usize, pending: &mut Vec<f32>, voice: &mut Voice, settings: AudioSettings) {
    trim_for_latency(queued, FREQUENCY, pending);
    voice.mix(pending, |left, right| settings.shape(left, right));
}

/// The WAV at `path` as stereo f32 samples at [`FREQUENCY`]: SDL reads it and converts its format, channels and rate.
fn read_wav(path: &Path) -> Result<Vec<f32>, String> {
    use sdl3::sys::audio::{SDL_AudioSpec, SDL_ConvertAudioSamples, SDL_LoadWAV};
    use sdl3::sys::stdinc::SDL_free;
    let name = path
        .to_str()
        .and_then(|p| std::ffi::CString::new(p).ok())
        .ok_or_else(|| "the path is not UTF-8".to_owned())?;
    let mut spec = SDL_AudioSpec::default();
    let mut wav: *mut u8 = std::ptr::null_mut();
    let mut wav_len: u32 = 0;
    // SAFETY: `name` is a NUL-terminated path; SDL fills the three out values, the buffer is freed below.
    if !unsafe { SDL_LoadWAV(name.as_ptr(), &raw mut spec, &raw mut wav, &raw mut wav_len) } {
        return Err(sdl3::get_error().to_string());
    }
    let to = SDL_AudioSpec {
        format: sdl3::audio::AudioFormat::f32_sys().into(),
        channels: 2,
        freq: FREQUENCY as i32,
    };
    let mut out: *mut u8 = std::ptr::null_mut();
    let mut out_len = 0;
    let converted = i32::try_from(wav_len).is_ok_and(|len| {
        // SAFETY: `wav` holds `len` bytes of `spec`'s samples; SDL allocates `out`, freed below.
        unsafe {
            SDL_ConvertAudioSamples(
                &raw const spec,
                wav,
                len,
                &raw const to,
                &raw mut out,
                &raw mut out_len,
            )
        }
    });
    // SAFETY: SDL allocated `wav`, which is not used after this.
    unsafe { SDL_free(wav.cast()) };
    if !converted {
        return Err(sdl3::get_error().to_string());
    }
    let samples = {
        // SAFETY: SDL converted `out_len` bytes into `out`, read here once before it is freed.
        let bytes =
            unsafe { std::slice::from_raw_parts(out, usize::try_from(out_len).unwrap_or(0)) };
        bytes.as_chunks::<4>().0.iter().map(|b| f32::from_ne_bytes(*b)).collect()
    };
    // SAFETY: SDL allocated `out`, which is not used after this.
    unsafe { SDL_free(out.cast()) };
    Ok(samples)
}

/// The voice of the transformation in the data folder `data_dir` ([`voice::ssj_clip_path`]) as samples to mix, or `None`
/// (said in the log) when the folder lacks it or SDL cannot read it.
fn ssj_clip(data_dir: &Path) -> Option<Vec<f32>> {
    let Some(path) = voice::find_ssj_clip(data_dir) else {
        log::info!("voice: {} not found", voice::SSJ_CLIP);
        return None;
    };
    match read_wav(&path) {
        Ok(samples) => {
            log::info!(
                "voice: {} read, {:.2} s",
                path.display(),
                samples.len() as f32 / 2.0 / FREQUENCY as f32
            );
            Some(samples)
        }
        Err(e) => {
            log::warn!("voice: {} not played: {e}", path.display());
            None
        }
    }
}

fn start_stream(stream: &sdl3::audio::AudioStream) -> Result<(), String> {
    let mut real = sdl3::sys::audio::SDL_AudioSpec::default();
    let mut frames = 0;
    // SAFETY: the stream is alive; SDL fills the two out values.
    let known = unsafe {
        let id = sdl3::sys::audio::SDL_GetAudioStreamDevice(stream.raw());
        sdl3::sys::audio::SDL_GetAudioDeviceFormat(id, &raw mut real, &raw mut frames)
    };
    if known {
        log::info!(
            "sound device: {} Hz, {} channels, {frames} sample frames at a time ({:.1} ms)",
            real.freq,
            real.channels,
            frames as f32 * 1000.0 / real.freq.max(1) as f32
        );
    }
    // Prime the target's worth of silence so the first frames do not find the device dry.
    let silence = vec![0.0_f32; (FREQUENCY as f32 * TARGET_MS / 1000.0) as usize * 2];
    stream.put_data_f32(&silence).map_err(|e| e.to_string())?;
    stream.resume().map_err(|e| e.to_string())
}

impl Audio {
    /// Opens the default playback device; never fails, falling back to muted output.
    pub fn open(sdl: &sdl3::Sdl) -> Audio {
        let spec = sdl3::audio::AudioSpec {
            freq: Some(FREQUENCY as i32),
            channels: Some(2),
            format: Some(sdl3::audio::AudioFormat::f32_sys()),
        };
        #[cfg(target_os = "android")]
        let playback = open_with(|| open_android(sdl, &spec));
        #[cfg(not(target_os = "android"))]
        let playback = open_with(|| {
            let audio = sdl.audio().map_err(|e| e.to_string())?;
            let device = audio.open_playback_device(&spec).map_err(|e| e.to_string())?;
            let stream = device.open_device_stream(Some(&spec)).map_err(|e| e.to_string())?;
            start_stream(&stream)?;
            Ok(Playback { stream })
        });
        playback
    }

    /// A muted output, with no device: what a run without sound has (the bench).
    pub fn muted() -> Audio {
        open_with(|| Err("muted".to_owned()))
    }

    /// The queue at the last frame's end in milliseconds, and the frames so far that found it empty.
    pub fn health(&self) -> (f32, u32) {
        (self.queued_ms, self.underruns)
    }

    /// Sets the volume and the output the samples are shaped by from now on.
    pub fn set_settings(&mut self, settings: AudioSettings) {
        self.settings = settings;
    }

    pub fn frequency() -> u64 {
        FREQUENCY
    }

    pub fn is_muted(&self) -> bool {
        self.playback.is_none()
    }

    /// Reads the voice of Goku's first Super Saiyan transformation from the data folder `data_dir`, if it has one (a muted
    /// output reads none).
    pub fn load_voice(&mut self, data_dir: &Path) {
        if !self.is_muted() {
            self.voice = Voice::new(ssj_clip(data_dir).unwrap_or_default());
        }
    }

    /// Plays the voice of the transformation from its start, mixed into the sound from this window frame's on.
    pub fn play_voice(&mut self) {
        self.voice.start();
        if self.voice.playing() {
            log::info!("voice: Goku becomes a Super Saiyan, {} plays", voice::SSJ_CLIP);
        }
    }

    /// Flushes the samples pushed during the frame to SDL, trimming them to the latency cap.
    pub fn end_frame(&mut self) {
        let Some(stream) = self.playback.as_ref().map(|playback| &playback.stream) else {
            self.pending.clear();
            return;
        };
        let queued = stream.queued_bytes().map_or(0, |q| q.max(0) as usize);
        let found = ms_of(queued);
        let health = &mut self.health;
        health.floats += self.pending.len();
        health.frames += 1;
        health.least = health.least.min(found);
        health.most = health.most.max(found);
        if found < DRY_MS && !self.pending.is_empty() {
            self.underruns += 1;
            health.dry += 1;
        }
        let elapsed = health.since.elapsed();
        if elapsed >= REPORT_EVERY {
            log::info!(
                "sound: the game made {:.0} samples/s over {} flushes, {} dry; the queue a flush found {:.1} to {:.1} ms",
                health.floats as f64 / 2.0 / elapsed.as_secs_f64(),
                health.frames,
                health.dry,
                health.least,
                health.most
            );
            *health = Health::new();
        }
        prepare(queued, &mut self.pending, &mut self.voice, self.settings);
        if let Err(e) = stream.put_data_f32(&self.pending) {
            log::warn!("dropping audio after SDL error: {e}");
        }
        self.queued_ms = ms_of(queued + self.pending.len() * 4);
        // SAFETY: the stream is alive (owned by `self`); the call only sets the stream's playback ratio.
        unsafe {
            sdl3::sys::audio::SDL_SetAudioStreamFrequencyRatio(
                stream.raw(),
                rate_ratio(self.queued_ms),
            );
        }
        self.pending.clear();
    }
}

impl AudioSink for Audio {
    fn push(&mut self, left: f32, right: f32) {
        if !self.is_muted() {
            let (left, right) = self.settings.shape(left, right);
            self.pending.push(left);
            self.pending.push(right);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_failure_falls_back_to_muted() {
        let mut audio = open_with(|| Err("no device".to_owned()));
        assert!(audio.is_muted());
        assert_eq!(Audio::frequency(), 48_000);
        audio.push(0.5, -0.5);
        assert!(audio.pending.is_empty(), "a muted Audio must not accumulate samples");
        audio.end_frame(); // must not panic
    }

    #[test]
    fn the_rate_bends_toward_the_target_queue_by_half_a_percent_at_most() {
        assert!((rate_ratio(TARGET_MS) - 1.0).abs() < 1e-6);
        assert!(rate_ratio(TARGET_MS * 1.5) > 1.0, "a full queue plays a little faster");
        assert!(rate_ratio(TARGET_MS * 0.5) < 1.0, "a low one a little slower");
        assert!((rate_ratio(10_000.0) - 1.005).abs() < 1e-6);
        assert!((rate_ratio(0.0) - 0.995).abs() < 1e-6);
    }

    /// The device's queue over window frames, as [`Audio::end_frame`] keeps it: each window frame comes `elapsed` ms
    /// after the last, while the device played the queue at the speed the last one set, and brings `samples` stereo
    /// samples. The queue starts primed with `target_ms` of silence, bent toward `target_ms`, capped at
    /// [`MAX_LATENCY_MS`]. The frames that found it dry, and the queue after each flush, in ms, are returned.
    fn simulate(
        target_ms: f32,
        window_frames: impl IntoIterator<Item = (f32, usize)>,
    ) -> (u32, Vec<f32>) {
        let bytes_per_ms = FREQUENCY as f32 * 2.0 * 4.0 / 1000.0;
        let (mut queued, mut ratio, mut dry, mut after) = (target_ms, 1.0, 0, Vec::new());
        for (elapsed, samples) in window_frames {
            let found = (queued - elapsed * ratio).max(0.0);
            let mut pending = vec![0.0_f32; samples * 2];
            if found < DRY_MS && !pending.is_empty() {
                dry += 1;
            }
            trim_for_latency((found * bytes_per_ms) as usize, FREQUENCY, &mut pending);
            queued = found + ms_of(pending.len() * 4);
            ratio = bend_toward(target_ms, queued);
            after.push(queued);
        }
        (dry, after)
    }

    const FRAME_MS: f32 = 1000.0 / 60.0;

    /// `seconds` of window frames at speed 1 (a frame of the game, 800 samples, a refresh), then one that comes
    /// `stall` ms late and brings the frames of the game the stall held back, then a second at speed 1 again.
    fn stalled(seconds: usize, stall: f32) -> Vec<(f32, usize)> {
        let steady = std::iter::repeat_n((FRAME_MS, 800), seconds * 60);
        let held = (stall / FRAME_MS).round() as usize * 800;
        steady
            .clone()
            .chain([(stall, held)])
            .chain(std::iter::repeat_n((FRAME_MS, 800), 60))
            .collect()
    }

    #[test]
    fn a_stall_of_100_ms_without_a_flush_leaves_the_queue_not_dry() {
        let frames = stalled(30, 100.0);
        let (dry, after) = simulate(TARGET_MS, frames.clone());
        assert_eq!(dry, 0, "the target's queue outlasts the stall: {:?}", &after[1795..1805]);
        let (dry, _) = simulate(60.0, frames);
        assert!(dry > 0, "the old 60 ms queue ran dry in it");
    }

    /// The samples of the sound chip a frame of the game makes at a battle `tempo` of f frames of the game to r
    /// refreshes (the sound chip at r/f of its clock; (1, 1) is speed 1), on the model's 60 Hz (the game's 60.0988
    /// makes 798.7 where this makes 800): 800 x r / f.
    fn samples_a_frame((frames, refreshes): (u32, u32)) -> f64 {
        FREQUENCY as f64 * f64::from(FRAME_MS) / 1000.0 * f64::from(refreshes) / f64::from(frames)
    }

    /// The window frames of `seconds` at a battle `tempo` on a 60 Hz screen as the pacer gives them (`app.rs`
    /// `Pacer::frames`, `Pacer::wait`): a window frame runs 1 frame of the game and those it owes, at most
    /// `CATCH_UP` (4), and a clock more than 5 refreshes behind starts over. Window frame `stall_at` comes `stall` ms
    /// late. Each is (ms since the last, samples), the samples counted whole frames of [`samples_a_frame`] with the
    /// fraction carried, as the sound chip carries it.
    fn paced(tempo: (u32, u32), seconds: usize, stall_at: usize, stall: f32) -> Vec<(f32, usize)> {
        const CATCH_UP: u32 = 4;
        let game_frame = f64::from(FRAME_MS) * f64::from(tempo.1) / f64::from(tempo.0);
        let (mut now, mut next, mut made, mut sent) = (0.0_f64, 0.0_f64, 0.0_f64, 0_usize);
        let mut out = Vec::new();
        for i in 0..seconds * 60 {
            let elapsed = if i == stall_at { FRAME_MS + stall } else { FRAME_MS };
            now += f64::from(elapsed);
            let owed = ((now - next).max(0.0) / game_frame) as u32;
            let requested = (1 + owed).min(CATCH_UP);
            next += game_frame * f64::from(requested);
            if now - next > 5.0 * f64::from(FRAME_MS) {
                next = now;
            }
            made += f64::from(requested) * samples_a_frame(tempo);
            out.push((elapsed, made as usize - sent));
            sent = made as usize;
        }
        out
    }

    const TEMPOS: [(&str, (u32, u32)); 3] = [("1x", (1, 1)), ("1.5x", (3, 2)), ("2x", (2, 1))];

    #[test]
    fn a_refresh_brings_the_samples_of_speed_1_at_every_battle_speed() {
        for (name, tempo) in TEMPOS {
            let a_refresh = samples_a_frame(tempo) * f64::from(tempo.0) / f64::from(tempo.1);
            assert!((a_refresh - 800.0).abs() < 0.01, "{name}: {a_refresh}");
        }
        assert!(
            (samples_a_frame((2, 1)) - 400.0).abs() < 0.01,
            "2x: half of 1x a frame of the game"
        );
    }

    #[test]
    fn a_stall_of_100_ms_as_the_pacer_catches_up_leaves_the_queue_not_dry_at_every_battle_speed() {
        for (name, tempo) in TEMPOS {
            let frames = paced(tempo, 31, 30 * 60, 100.0);
            let (dry, after) = simulate(TARGET_MS, frames);
            let least = after[30 * 60..].iter().copied().fold(f32::MAX, f32::min);
            assert_eq!(dry, 0, "{name}: the queue after the stall went down to {least:.1} ms");
            let longest = (100..400)
                .take_while(|&ms| simulate(TARGET_MS, paced(tempo, 31, 30 * 60, ms as f32)).0 == 0)
                .last()
                .unwrap_or(0);
            println!(
                "{name}: a stall of 100 ms leaves {least:.1} ms of the queue; the longest stall without a dry frame \
                 {longest} ms"
            );
        }
    }

    #[test]
    fn the_queue_settles_within_a_tenth_of_the_target_at_speed_1_and_at_the_battle_speed() {
        let speed_1 = std::iter::repeat_n((FRAME_MS, 800), 60 * 60);
        // The battle speed: 3 frames of the game (533 samples each, the sound chip at 2/3) to 2 refreshes.
        let battle = std::iter::repeat_n([(FRAME_MS, 533), (FRAME_MS, 1067)], 30 * 60).flatten();
        for (name, after) in [
            ("speed 1", simulate(TARGET_MS, speed_1).1),
            ("battle speed", simulate(TARGET_MS, battle).1),
            ("battle speed 2x", simulate(TARGET_MS, paced((2, 1), 60, usize::MAX, 0.0)).1),
        ] {
            let tail = &after[after.len() - 600..];
            let (least, most) =
                tail.iter().fold((f32::MAX, 0.0_f32), |(l, m), &q| (l.min(q), m.max(q)));
            assert!(
                least >= TARGET_MS * 0.9 && most <= TARGET_MS * 1.1,
                "{name}: the last 10 s kept {least:.1} to {most:.1} ms"
            );
        }
    }

    #[test]
    fn the_cap_leaves_room_over_the_target_for_a_window_frame_of_the_catch_up_at_every_battle_speed()
     {
        for (name, tempo) in TEMPOS {
            let most =
                paced(tempo, 2, 60, 1000.0).iter().map(|&(_, samples)| samples).max().unwrap();
            assert!(
                MAX_LATENCY_MS as f32 >= TARGET_MS + ms_of(most * 2 * 4),
                "{name}: a window frame of {most} samples"
            );
        }
    }

    #[test]
    fn the_cap_leaves_room_over_the_target_for_two_window_frames_of_the_battle_speed() {
        // A window frame of the battle speed brings two frames of the game (2 x 533 samples, 22 ms): room for two
        // such over the target
        assert!(MAX_LATENCY_MS as f32 >= TARGET_MS + 2.0 * ms_of(1067 * 2 * 4));
        let primed = (FREQUENCY as f32 * TARGET_MS / 1000.0) as usize * 2;
        assert!((ms_of(primed * 4) - TARGET_MS).abs() < 0.1, "the priming is the target");
    }

    #[test]
    fn bytes_of_stereo_floats_are_counted_in_milliseconds() {
        assert!((ms_of(48_000 * 2 * 4) - 1000.0).abs() < 1e-3);
        assert!((ms_of(800 * 2 * 4) - 16.667).abs() < 1e-2, "a game frame's 800 samples");
    }

    #[test]
    fn the_voice_is_mixed_after_the_trim_at_the_volume() {
        let max_bytes = (FREQUENCY as usize * MAX_LATENCY_MS / 1000) * 2 * 4;
        let mut voice = Voice::new(vec![0.5, -0.5, 0.25, -0.25]);
        voice.start();
        // 8 floats pending, 4 of them over the cap: trimmed first, then the clip goes into the 4 kept, all of it
        let mut pending = vec![0.0_f32; 8];
        let half = AudioSettings { volume: 5, mono: false };
        prepare(max_bytes - 16, &mut pending, &mut voice, half);
        assert_eq!(pending, [0.25, -0.25, 0.125, -0.125], "the whole clip, at half volume");
        assert!(!voice.playing(), "and it ended: none of it was trimmed away");
        let mut silent = vec![0.0_f32; 4];
        let mut voice = Voice::new(vec![0.5, -0.5]);
        voice.start();
        prepare(0, &mut silent, &mut voice, AudioSettings { volume: 0, mono: false });
        assert_eq!(silent, [0.0; 4], "volume 0 silences the voice too");
    }

    /// A PCM 16-bit WAV of `channels` channels at `rate` Hz holding `samples` (interleaved).
    fn pcm16_wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        wav.extend_from_slice(&(channels * 2).to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        wav
    }

    #[test]
    fn a_missing_or_unreadable_voice_is_none_and_a_wav_is_read_at_48_khz_stereo() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(ssj_clip(dir.path()), None, "no voice folder");
        let folder = dir.path().join(voice::FOLDER);
        std::fs::create_dir_all(&folder).unwrap();
        assert_eq!(ssj_clip(dir.path()), None, "an empty voice folder");
        std::fs::write(folder.join(voice::SSJ_CLIP), b"not a wav at all").unwrap();
        assert_eq!(ssj_clip(dir.path()), None, "a file SDL cannot read is not played");
        // A second of a mono 24 kHz sine of 440 Hz at half scale
        let tone: Vec<i16> = (0..24_000)
            .map(|n| {
                (16_384.0 * (std::f32::consts::TAU * 440.0 * n as f32 / 24_000.0).sin()) as i16
            })
            .collect();
        std::fs::write(folder.join(voice::SSJ_CLIP), pcm16_wav(24_000, 1, &tone)).unwrap();
        let clip = ssj_clip(dir.path()).expect("the WAV is read");
        let frames = clip.len() / 2;
        assert!((47_000..=49_000).contains(&frames), "a second at 48 kHz, {frames} frames");
        // the same sample on both sides, bit for bit
        assert!(
            clip.as_chunks::<2>().0.iter().all(|[l, r]| l.to_bits() == r.to_bits()),
            "mono to both sides"
        );
        let peak = clip.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!((0.45..=0.6).contains(&peak), "half scale, {peak}");
    }

    #[test]
    fn a_muted_output_reads_no_voice_and_plays_none() {
        let dir = tempfile::tempdir().unwrap();
        let mut audio = Audio::muted();
        audio.load_voice(dir.path());
        audio.play_voice();
        assert!(!audio.voice.playing());
        audio.end_frame(); // must not panic
    }

    #[test]
    fn latency_trim_drops_oldest_samples_above_the_cap() {
        let freq = 48_000;
        let max_bytes = (freq as usize * MAX_LATENCY_MS / 1000) * 2 * 4;
        // pending = 0.0, 1.0, ... 1599.0 (6400 bytes)
        let fresh = || (0..1600).map(|i| i as f32).collect::<Vec<f32>>();

        // total = queued + 6400 = max + 12 bytes -> 3 excess floats, aligned down to 2 (one stereo frame)
        let mut pending = fresh();
        trim_for_latency(max_bytes - 6400 + 12, freq, &mut pending);
        assert_eq!(pending.len(), 1598);
        assert_eq!(pending[0], 2.0, "oldest samples are dropped first");
        assert_eq!(*pending.last().unwrap(), 1599.0, "newest samples are kept");

        // exactly at the cap: untouched
        let mut at_cap = fresh();
        trim_for_latency(max_bytes - 6400, freq, &mut at_cap);
        assert_eq!(at_cap.len(), 1600);

        // far above the cap: everything pending may go, but never more than pending
        let mut all = fresh();
        trim_for_latency(max_bytes + 100_000, freq, &mut all);
        assert_eq!(all, Vec::<f32>::new());
    }
}
