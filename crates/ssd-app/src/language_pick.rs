//! The first launch's choice of the language (Enhanced mode, Android too): before the game's first frame, while
//! the PC file holds no language and `--lang` names none, the game's window shows the setup page of `ssd_menus::setup`
//! ([`Page::Language`]) in the HD menus' style, full screen. The arrows, the d-pad, Enter, Space, X and the pad's
//! South and East buttons choose with the keys; a click (desktop) or a tap (the touch screen's fingers, which the game
//! takes from SDL itself) chooses a button. The choice is written to the PC file, so it is asked once. A run that
//! does not persist (a headless run, a snapshot, a replay) never asks.

use crate::egui_input::InputAdapter;
use crate::hd::{Art, Fit};
use crate::video::Video;
use crate::{paths, setup_ui};
use anyhow::Context;
use sdl3::event::{Event, WindowEvent};
use sdl3::gamepad::{Button, Gamepad};
use sdl3::keyboard::Scancode;
#[cfg(test)]
use ssd_menus::language::Locale;
use ssd_menus::options::Values;
use ssd_menus::setup::{self, Command, Page, Setup};
use ssd_menus::{Key, Language};
use std::path::Path;
use std::time::Duration;

/// Whether the run asks for the language: an Enhanced run (`enhanced`) that writes the player's files, where the
/// player has a translation, whose PC file (`settings`) holds no language, and whose command line gave none. Android
/// asks as the desktop does; a player of an earlier Android version is not asked, the language being settled before
/// ([`settle_saved_language`]).
#[must_use]
pub fn needed(enhanced: bool, settings: &Values) -> bool {
    needed_on(enhanced, settings, cfg!(target_os = "android"))
}

/// [`needed`] on Android (`_android`) or not: the same on both.
fn needed_on(enhanced: bool, settings: &Values, _android: bool) -> bool {
    enhanced
        && paths::persists()
        && ssd_menus::options::language_row_shown()
        && crate::lang::overridden().is_none()
        && Language::from_values(settings).is_none()
}

/// The language an earlier version's player keeps: the earlier versions brought Traditional Chinese (Taiwan) alone.
const EARLIER_LANGUAGE: &str = "zh-tw";

/// The PC file in the saves folder (`crate::options`, `SETTINGS_FILE`).
const SETTINGS_FILE: &str = "ssd.settings";

/// The mark of the data folder (beside the saves folder, not in it) that [`settle_saved_language`] judged the folder
/// once: an earlier version's player is told from a new one at the first launch of this version only. Later, a saves
/// folder that has files is no sign of an earlier version: a run left before its first frame (the Language page
/// closed, Classic left at once) flushes the machine's blank battery save there.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const SETTLED_MARK: &str = "language-settled";

/// Stores Traditional Chinese (Taiwan) in the PC file's `values` of the data folder `data` when they hold no language
/// and the saves folder there holds a file already: the player of an earlier version, which never asked, keeps the
/// language it played in, and the first launch's page asks a new player only. `true` when `values` changed.
/// A language the PC file holds already is never changed: the Android app never passes `--lang`, so the stored value
/// is the language the player actually saw. (Windows differs: the earlier launchers passed `--lang zh-tw` over the
/// stored value; see the launcher's `settle_language`.)
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn settle_language_for_existing(values: &mut Values, data: &Path) -> bool {
    let played =
        std::fs::read_dir(data.join("saves")).is_ok_and(|mut files| files.next().is_some());
    if Language::from_values(values).is_some() || !played {
        return false;
    }
    let Some(language) = Language::from_code(EARLIER_LANGUAGE) else { return false };
    language.store(values);
    true
}

/// [`settle_language_for_existing`] on the PC file of the data folder `data`, before the game reads it (Android, at
/// the start of the process: the Change language of the touch MENU, which forgets the language and starts the game
/// again, still asks), once in the folder's life: nothing when its [`SETTLED_MARK`] is there, else the mark is written
/// after the judgement, whether the language was written or not (a mark that cannot be written is said in the log,
/// and the folder is judged again at the next launch). The file is replaced through a `.tmp` file and a rename.
/// `true` when it was written.
///
/// # Errors
///
/// The file cannot be read, is not a PC file (left as it is: the game keeps it aside), or cannot be written; the
/// folder is not marked then.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn settle_saved_language(data: &Path) -> anyhow::Result<bool> {
    let mark = data.join(SETTLED_MARK);
    if mark.exists() {
        return Ok(false);
    }
    let written = settle_settings_file(data)?;
    if let Err(error) = std::fs::write(&mark, "") {
        log::warn!("{}: {error}; the folder is judged again at the next launch", mark.display());
    }
    Ok(written)
}

/// [`settle_language_for_existing`] on the PC file of the data folder `data`; `true` when it was written.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn settle_settings_file(data: &Path) -> anyhow::Result<bool> {
    let file = data.join("saves").join(SETTINGS_FILE);
    let mut values = match std::fs::read_to_string(&file) {
        Ok(text) => Values::from_text(&text)
            .map_err(|error| anyhow::anyhow!("{}: {error}", file.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Values::new(),
        Err(error) => return Err(error).with_context(|| format!("reading {}", file.display())),
    };
    if !settle_language_for_existing(&mut values, data) {
        return Ok(false);
    }
    let partial = data.join("saves").join(format!("{SETTINGS_FILE}.tmp"));
    std::fs::write(&partial, values.to_text())
        .with_context(|| format!("writing {}", partial.display()))?;
    std::fs::rename(&partial, &file).with_context(|| format!("replacing {}", file.display()))?;
    Ok(true)
}

/// The key of the page a keyboard key is, if any.
fn key_of(code: Scancode) -> Option<Key> {
    match code {
        Scancode::Up => Some(Key::Up),
        Scancode::Down => Some(Key::Down),
        Scancode::Left => Some(Key::Left),
        Scancode::Right => Some(Key::Right),
        Scancode::Tab => Some(Key::Tab),
        Scancode::Return | Scancode::KpEnter | Scancode::Space | Scancode::X => Some(Key::Confirm),
        _ => None,
    }
}

/// The key of the page a gamepad button is, if any: both South and East confirm (the game's A is East, the system's
/// usual confirm South).
fn button_of(button: Button) -> Option<Key> {
    match button {
        Button::DPadUp => Some(Key::Up),
        Button::DPadDown => Some(Key::Down),
        Button::DPadLeft => Some(Key::Left),
        Button::DPadRight => Some(Key::Right),
        Button::South | Button::East | Button::Start => Some(Key::Confirm),
        _ => None,
    }
}

/// The language whose button is under the point `at` (egui points) of a page drawn over `screen`.
fn tapped(page: &Setup, screen: egui::Rect, at: egui::Pos2) -> Option<Command> {
    let fit = Fit::sized(screen, setup::WIDTH, setup::HEIGHT);
    page.buttons()
        .into_iter()
        .find(|button| button.enabled && fit.rect(button.rect).contains(at))
        .map(|button| button.command)
}

/// Shows the page in the game's window until a language is chosen; `None` when the player closes the window.
///
/// # Errors
///
/// The window cannot be drawn.
pub fn run(
    video: &mut Video,
    events: &mut sdl3::EventPump,
    input: &mut InputAdapter,
    gamepads: (Option<&sdl3::GamepadSubsystem>, &mut Vec<Gamepad>),
) -> anyhow::Result<Option<Language>> {
    let (subsystem, pads) = gamepads;
    let mut page = Setup::new(Page::Language);
    // No language is chosen yet: the page is in the app's own language, the first translation of the page
    // (`crate::lang::interface`; English without one).
    let words = crate::lang::interface_text(&crate::options::read_settings().unwrap_or_default());
    let mut art = Art::empty();
    #[cfg_attr(not(target_os = "android"), allow(unused_mut))]
    let mut background = false;
    loop {
        let mut commands = Vec::new();
        for event in events.poll_iter() {
            video.handle_event(input, &event);
            match event {
                Event::Quit { .. } => return Ok(None),
                Event::KeyDown { scancode: Some(code), repeat: false, .. } => {
                    commands.extend(key_of(code).and_then(|key| page.key(key)));
                }
                Event::GamepadButtonDown { button, .. } => {
                    commands.extend(button_of(button).and_then(|key| page.key(key)));
                }
                Event::GamepadAdded { which, .. } => {
                    if let Some(pad) = subsystem.and_then(|subsystem| subsystem.open(which).ok()) {
                        pads.push(pad);
                    }
                }
                Event::GamepadRemoved { which, .. } => {
                    pads.retain(|pad| pad.id().ok() != Some(which));
                }
                // A finger (the game turns SDL's touch-to-mouse events off where it has touch controls).
                Event::FingerUp { x, y, .. } => {
                    let screen = video.screen_points();
                    let at = egui::pos2(
                        screen.min.x + x * screen.width(),
                        screen.min.y + y * screen.height(),
                    );
                    commands.extend(tapped(&page, screen, at));
                }
                Event::Window { win_event, .. } => match win_event {
                    WindowEvent::Minimized => video.set_minimized(true),
                    WindowEvent::Restored | WindowEvent::Maximized => {
                        video.set_minimized(false);
                        video.handle_resize();
                    }
                    WindowEvent::Resized(..) | WindowEvent::PixelSizeChanged(..) => {
                        video.handle_resize();
                    }
                    #[cfg(target_os = "android")]
                    WindowEvent::FocusLost if !background => {
                        background = true;
                        video.suspend();
                    }
                    #[cfg(target_os = "android")]
                    WindowEvent::FocusGained if background => {
                        background = false;
                        video.resume();
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        if background {
            std::thread::sleep(Duration::from_millis(30));
            continue;
        }
        let shown = page.clone();
        video.present(&[], input, |ui, _| {
            commands.extend(setup_ui::show(ui, &shown, words.as_deref(), &mut art));
        })?;
        if let Some(Command::Language(language)) =
            commands.into_iter().find(|command| matches!(command, Command::Language(_)))
        {
            return Ok(Some(language));
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tap_on_a_button_chooses_its_language_wherever_the_page_is_fitted() {
        let made_up = Locale::unnamed(Language::from_code("xx").unwrap());
        let languages = vec![Locale::english(), made_up, Locale::unnamed(Language::ZH_HK)];
        let page = Setup::with_languages(Page::Language, languages.clone());
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1864.0, 860.0));
        let fit = Fit::sized(screen, setup::WIDTH, setup::HEIGHT);
        for (button, locale) in page.buttons().iter().zip(&languages) {
            let centre = fit.rect(button.rect).center();
            assert_eq!(tapped(&page, screen, centre), Some(Command::Language(locale.language)));
        }
        assert_eq!(tapped(&page, screen, egui::pos2(2.0, 2.0)), None, "the ground chooses nothing");
    }

    #[test]
    fn the_keys_and_buttons_confirm_and_move() {
        assert_eq!(key_of(Scancode::Return), Some(Key::Confirm));
        assert_eq!(key_of(Scancode::Down), Some(Key::Down));
        assert_eq!(key_of(Scancode::Escape), None, "there is no going back");
        assert_eq!(button_of(Button::East), Some(Key::Confirm));
        assert_eq!(button_of(Button::DPadUp), Some(Key::Up));
    }

    #[test]
    fn only_an_enhanced_run_that_persists_with_no_language_asks() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _lang = crate::lang::LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::lang::configure(None);
        paths::set_persistent(true);
        let mut values = Values::new();
        ssd_menus::options::offer_languages(Vec::new());
        assert!(!needed(true, &values), "no translation: never asked");
        ssd_menus::options::offer_languages(vec![Locale::unnamed(Language::ZH_HK)]);
        assert!(needed(true, &values));
        assert!(needed_on(true, &values, false), "the desktop asks");
        assert!(needed_on(true, &values, true), "Android asks as well");
        assert!(!needed(false, &values), "Classic never asks");
        Language::ENGLISH.store(&mut values);
        assert!(!needed(true, &values), "asked once");
        paths::set_persistent(false);
        assert!(!needed(true, &Values::new()), "a run that does not persist never asks");
        ssd_menus::options::offer_languages(Vec::new());
    }

    /// Runs `check` with a translation offered, a run that persists and no `--lang`, as a player's launch is.
    fn with_a_translation(check: impl FnOnce()) {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _lang = crate::lang::LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::lang::configure(None);
        paths::set_persistent(true);
        let taiwan = Language::from_code("zh-tw").unwrap();
        ssd_menus::options::offer_languages(vec![
            Locale::unnamed(Language::ZH_HK),
            Locale::unnamed(taiwan),
        ]);
        check();
        paths::set_persistent(false);
        ssd_menus::options::offer_languages(Vec::new());
    }

    #[test]
    fn android_asks_too_when_no_language_is_stored() {
        with_a_translation(|| {
            assert!(needed_on(true, &Values::new(), true), "Android");
            assert!(needed_on(true, &Values::new(), false), "the desktop");
            assert!(!needed_on(false, &Values::new(), true), "Classic never asks");
        });
    }

    #[test]
    fn a_reset_language_row_is_asked_again_at_the_next_start() {
        with_a_translation(|| {
            let row = ssd_menus::options::Category::Accessibility
                .rows()
                .iter()
                .find(|row| row.key == Language::KEY)
                .expect("the Language row");
            let mut values = Values::new();
            Language::ZH_HK.store(&mut values);
            row.reset(&mut values, &mut ssd_menus::cheats::Cheats::neutral());
            // What the run follows at once is not written: the file still holds no language.
            let _ = crate::lang::followed(&values);
            assert_eq!(values.word(Language::KEY), None);
            assert_eq!(values.get(Language::KEY), None);
            assert!(needed(true, &values), "the next start asks");
        });
    }

    #[test]
    fn a_stored_language_is_never_asked_again() {
        with_a_translation(|| {
            for code in ["zh-tw", "zh-hk"] {
                let mut values = Values::new();
                Language::from_code(code).unwrap().store(&mut values);
                assert!(!needed_on(true, &values, true), "{code} on Android");
                assert!(!needed_on(true, &values, false), "{code} on the desktop");
            }
        });
    }

    /// A data folder whose `saves` folder holds `files`.
    fn data_with_saves(files: &[&str]) -> tempfile::TempDir {
        let data = tempfile::tempdir().unwrap();
        if !files.is_empty() {
            std::fs::create_dir(data.path().join("saves")).unwrap();
        }
        for file in files {
            std::fs::write(data.path().join("saves").join(file), b"x").unwrap();
        }
        data
    }

    #[test]
    fn an_existing_save_settles_taiwan_once() {
        let data = data_with_saves(&["ssd.sav"]);
        let mut values = Values::new();
        assert!(settle_language_for_existing(&mut values, data.path()), "written");
        assert_eq!(values.word(Language::KEY), Some("zh-tw"));
        assert!(!settle_language_for_existing(&mut values, data.path()), "once");
        assert_eq!(values.word(Language::KEY), Some("zh-tw"));
    }

    #[test]
    fn a_new_install_is_left_to_the_language_page() {
        let data = data_with_saves(&[]);
        let mut values = Values::new();
        assert!(!settle_language_for_existing(&mut values, data.path()), "no saves folder");
        std::fs::create_dir(data.path().join("saves")).unwrap();
        assert!(!settle_language_for_existing(&mut values, data.path()), "an empty saves folder");
        assert_eq!(Language::from_values(&values), None, "the page asks");
    }

    #[test]
    fn a_chosen_hong_kong_is_kept() {
        let data = data_with_saves(&["ssd.sav", "ssd.settings"]);
        for language in [Language::ZH_HK, Language::ENGLISH] {
            let mut values = Values::new();
            language.store(&mut values);
            assert!(!settle_language_for_existing(&mut values, data.path()), "{}", language.code());
            assert_eq!(Language::from_values(&values), Some(language));
        }
    }

    /// The PC file of the data folder `data` (none when it does not exist).
    fn settings_of(data: &Path) -> Option<Values> {
        let text = std::fs::read_to_string(data.join("saves").join(SETTINGS_FILE)).ok()?;
        Some(Values::from_text(&text).unwrap())
    }

    #[test]
    fn the_settings_file_of_an_existing_player_gets_taiwan_once_and_a_damaged_one_is_left() {
        let data = data_with_saves(&["ssd.sav"]);
        let file = data.path().join("saves").join("ssd.settings");
        assert!(settle_saved_language(data.path()).unwrap(), "no settings file yet: written");
        let written = std::fs::read_to_string(&file).unwrap();
        assert_eq!(settings_of(data.path()).unwrap().word(Language::KEY), Some("zh-tw"));
        assert!(!settle_saved_language(data.path()).unwrap(), "a second launch");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), written, "unchanged");
        let data = data_with_saves(&["ssd.sav"]);
        let file = data.path().join("saves").join("ssd.settings");
        let mut values = Values::new();
        values.set("video.scale", 3);
        std::fs::write(&file, values.to_text()).unwrap();
        assert!(settle_saved_language(data.path()).unwrap());
        let values = settings_of(data.path()).unwrap();
        assert_eq!(values.word(Language::KEY), Some("zh-tw"));
        assert_eq!(values.get("video.scale"), Some(3), "the other keys kept");
        let data = data_with_saves(&["ssd.sav"]);
        let file = data.path().join("saves").join("ssd.settings");
        std::fs::write(&file, "not a settings file").unwrap();
        assert!(settle_saved_language(data.path()).is_err(), "damaged: left to the game");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "not a settings file");
        assert!(!data.path().join("saves").join("ssd.settings.tmp").exists(), "no partial file");
        assert!(!data.path().join(SETTLED_MARK).exists(), "not settled: judged again next time");
    }

    #[test]
    fn a_new_install_is_settled_once_and_a_blank_save_left_on_the_page_does_not_make_it_a_player() {
        let data = tempfile::tempdir().unwrap();
        assert!(!settle_saved_language(data.path()).unwrap(), "the first launch: nothing written");
        assert!(settings_of(data.path()).is_none(), "no language: the page asks");
        assert!(data.path().join(SETTLED_MARK).is_file(), "the mark");
        // The player leaves the Language page: the run flushes a blank battery save.
        leave_before_the_first_frame(&data.path().join("saves"));
        assert!(!settle_saved_language(data.path()).unwrap(), "the next launch");
        assert!(settings_of(data.path()).is_none(), "still no language: the page asks again");
    }

    #[test]
    fn an_upgrading_player_is_settled_to_taiwan_once() {
        let data = data_with_saves(&["ssd.sav"]);
        assert!(settle_saved_language(data.path()).unwrap());
        assert_eq!(settings_of(data.path()).unwrap().word(Language::KEY), Some("zh-tw"));
        assert!(data.path().join(SETTLED_MARK).is_file(), "the mark");
        let written = std::fs::read(data.path().join("saves").join(SETTINGS_FILE)).unwrap();
        assert!(!settle_saved_language(data.path()).unwrap(), "again: nothing");
        assert_eq!(std::fs::read(data.path().join("saves").join(SETTINGS_FILE)).unwrap(), written);
    }

    #[test]
    fn an_upgrading_players_chosen_language_is_kept_and_marked() {
        for language in [Language::ZH_HK, Language::ENGLISH] {
            let data = data_with_saves(&["ssd.sav"]);
            let mut values = Values::new();
            language.store(&mut values);
            std::fs::write(data.path().join("saves").join(SETTINGS_FILE), values.to_text())
                .unwrap();
            assert!(!settle_saved_language(data.path()).unwrap(), "{}", language.code());
            let kept = settings_of(data.path()).unwrap();
            assert_eq!(Language::from_values(&kept), Some(language));
            assert!(data.path().join(SETTLED_MARK).is_file(), "{}: the mark", language.code());
        }
    }

    #[test]
    fn once_marked_the_saves_are_never_judged_again() {
        let data = data_with_saves(&["ssd.sav", "ssd.options"]);
        std::fs::write(data.path().join(SETTLED_MARK), "").unwrap();
        assert!(!settle_saved_language(data.path()).unwrap());
        assert!(settings_of(data.path()).is_none(), "no language written");
    }

    /// What a run that ends before its first frame (the Language page closed, Classic left at once) leaves in the
    /// saves folder `saves` of a new install: the machine's battery save flushed as `run_windowed` flushes it.
    fn leave_before_the_first_frame(saves: &Path) {
        let store = ssd_oracle::FsSaveStore::new(saves.to_path_buf(), "ssd").unwrap();
        let mut machine =
            ssd_oracle::Machine::new(crate::test_support::sram_counter_rom(), Box::new(store))
                .unwrap();
        machine.flush_saves().unwrap();
    }

    #[test]
    fn a_new_install_left_before_the_first_frame_has_a_blank_battery_save() {
        let data = tempfile::tempdir().unwrap();
        let saves = data.path().join("saves");
        leave_before_the_first_frame(&saves);
        let sav = std::fs::read(saves.join("ssd.sav")).expect("a battery save is written");
        assert!(!sav.is_empty() && sav.iter().all(|&b| b == 0xFF), "blank: {} bytes", sav.len());
    }
}
