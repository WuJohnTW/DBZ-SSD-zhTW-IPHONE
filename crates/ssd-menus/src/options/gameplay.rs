//! The Gameplay page (saved with the game): the text speed, the message auto-run, the battle animation speed
//! and the random-encounter rate.

use super::{Kind, Opens, RowDef, Source};

/// The five speeds of the original's Text screen, slowest first. The game's byte (`$01D0`) counts the other way:
/// 0 prints the most characters a frame and 4 the fewest, so the row shows `4 - byte` (see `options::TEXT_SPEED_LAST`).
pub const TEXT_SPEEDS: &[&str] = &["Slowest", "Slow", "Normal", "Fast", "Fastest"];

/// The steps of the encounter rate: Off (random battles off), then the Cheats screen's x1/4 to x4.
pub const ENCOUNTERS: &[&str] = &["Off", "x1/4", "x1/2", "x1", "x2", "x4"];

/// The key of the message auto-run switch (Off, On) and of its wait (Short, Normal, Long), kept in the slot file.
pub const AUTO_RUN: &str = "gameplay.auto_run";
pub const AUTO_WAIT: &str = "gameplay.auto_wait";
/// The key of the battle animation speed, kept in the slot file.
pub const ANIM_SPEED: &str = "gameplay.anim_speed";
/// The key of the battle speed (Off, 1.5x, 2x), kept in the slot file: the stored number is the choice's place, so a
/// slot of a build with the switch (Off 0, On 1) reads as Off and 1.5x.
pub const BATTLE_SPEED: &str = "gameplay.battle_speed";

/// The choices of the auto-run switch; the default is Off, the original's behaviour.
pub const AUTO_RUN_CHOICES: &[&str] = &["Off", "On"];

/// The choices of the auto-run wait.
pub const AUTO_WAITS: &[&str] = &["Short", "Normal", "Long"];
/// The frames each wait lets a finished message stay up before the host presses A (the console runs 60 frames a second).
pub const AUTO_WAIT_FRAMES: [u32; 3] = [45, 90, 180];
/// The default wait: Normal.
pub const AUTO_WAIT_DEFAULT: u8 = 1;

/// The choices of the battle animation speed: Normal is the original; the others run the attack animations that many
/// times as fast (the machine runs that many frames to one on the screen, see `ssd_game::gameplay`).
pub const ANIM_SPEEDS: &[&str] = &["Normal", "x2", "x3", "x4"];
/// The frames of the machine to one frame of the screen at each animation speed.
pub const ANIM_FACTORS: [u32; 4] = [1, 2, 3, 4];

/// The choices of the battle speed; the default is Off, the original's speed. 1.5x and 2x run the whole of a battle
/// at that many times its speed with the music and the sound effects at theirs (see `ssd_game::gameplay::battle_tempo`).
/// (2x is written 2.0x: the menus' table is keyed by the English, and the Video page's scale has a 2x of its own.)
pub const BATTLE_SPEED_CHOICES: &[&str] = &["Off", "1.5x", "2.0x"];

/// The rows of the page.
pub const ROWS: &[RowDef] = &[
    RowDef {
        key: "gameplay.text_speed",
        label: "Text speed",
        group: "Messages",
        help: "How fast a message is written, from Slowest to Fastest. The same setting as the Text entry of the Menu: change it in either place.",
        kind: Kind::Choice(TEXT_SPEEDS),
        source: Source::TextSpeed,
    },
    RowDef {
        key: AUTO_RUN,
        label: "Auto-run messages",
        group: "Messages",
        help: "When a message has been written and waits for A, the game presses A for you after the wait below.",
        kind: Kind::Choice(AUTO_RUN_CHOICES),
        source: Source::Stored { default: 0 },
    },
    RowDef {
        key: AUTO_WAIT,
        label: "Auto-run wait",
        group: "Messages",
        help: "How long a finished message stays up before A is pressed: a short, a normal or a long time.",
        kind: Kind::Choice(AUTO_WAITS),
        source: Source::Stored { default: AUTO_WAIT_DEFAULT },
    },
    RowDef {
        key: ANIM_SPEED,
        label: "Battle animation speed",
        group: "Battle",
        help: "Plays the battle animations faster. The turn order and the numbers do not change.",
        kind: Kind::Cycle(ANIM_SPEEDS),
        source: Source::Stored { default: 0 },
    },
    RowDef {
        key: BATTLE_SPEED,
        label: "Battle speed (music as is)",
        group: "Battle",
        help: "Runs the whole battle 1.5 or 2 times as fast. The music and the sound effects keep their speed and pitch. With it on, a battle's random results may differ from the normal speed's.",
        kind: Kind::Choice(BATTLE_SPEED_CHOICES),
        source: Source::Stored { default: 0 },
    },
    RowDef {
        key: "gameplay.encounters",
        label: "Random encounters",
        group: "Exploring",
        help: "How often a random battle comes. The same value as the Cheats screen rows Random battles and How often.",
        kind: Kind::Cycle(ENCOUNTERS),
        source: Source::Encounters,
    },
    RowDef {
        key: "gameplay.data_packs",
        label: "Data packs",
        group: "Packs",
        help: "The data packs: which are on, their order, and the chapters they start.",
        kind: Kind::Button("Open"),
        source: Source::Opens(Opens::Packs),
    },
];

/// A line under the rows, if the page has one.
pub const NOTE: Option<&str> = None;

/// Whether the message auto-run is on in the slot's values.
#[must_use]
pub fn auto_run_on(values: &super::Values) -> bool {
    values.get(AUTO_RUN).is_some_and(|value| value > 0)
}

/// The frames a finished message waits before the host presses A, in the slot's values.
#[must_use]
pub fn auto_wait_frames(values: &super::Values) -> u32 {
    let index = values
        .get(AUTO_WAIT)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(usize::from(AUTO_WAIT_DEFAULT))
        .min(AUTO_WAIT_FRAMES.len() - 1);
    AUTO_WAIT_FRAMES[index]
}

/// The frames of the machine to one frame of the screen the battle animations run at, in the slot's values (1 is Normal).
#[must_use]
pub fn anim_factor(values: &super::Values) -> u32 {
    let index = values
        .get(ANIM_SPEED)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
        .min(ANIM_FACTORS.len() - 1);
    ANIM_FACTORS[index]
}

/// The battle speed's choice in the slot's values: its place in [`BATTLE_SPEED_CHOICES`] (0 Off, 1 1.5x, 2 2x), Off
/// when the value is missing or not a choice's.
#[must_use]
pub fn battle_speed(values: &super::Values) -> usize {
    values
        .get(BATTLE_SPEED)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|&choice| choice < BATTLE_SPEED_CHOICES.len())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::Values;

    #[test]
    fn the_defaults_are_the_original_game() {
        let values = Values::new();
        assert!(!auto_run_on(&values));
        assert_eq!(auto_wait_frames(&values), 90);
        assert_eq!(anim_factor(&values), 1);
        assert_eq!(battle_speed(&values), 0);
    }

    #[test]
    fn the_battle_speed_row_is_a_stored_choice_off_by_default_in_the_battle_group() {
        let row = ROWS.iter().find(|row| row.key == BATTLE_SPEED).expect("the battle speed row");
        assert_eq!(row.group, "Battle");
        assert_eq!(row.source, Source::Stored { default: 0 });
        assert_eq!(row.kind, Kind::Choice(&["Off", "1.5x", "2.0x"]));
        assert!(!super::super::video::SCALES.contains(&"2.0x"), "not a word of the Video page");
        let anim = ROWS.iter().position(|row| row.key == ANIM_SPEED).unwrap();
        assert_eq!(ROWS[anim + 1].key, BATTLE_SPEED, "under the battle animation speed");
        let mut values = super::super::Values::new();
        // The switch's On (1) of a slot saved before 2x is 1.5x, its Off (0) Off
        for (stored, choice) in [(0, 0), (1, 1), (2, 2)] {
            values.set(BATTLE_SPEED, stored);
            assert_eq!(battle_speed(&values), choice, "stored {stored}");
        }
        for wild in [3, 7, -1] {
            values.set(BATTLE_SPEED, wild);
            assert_eq!(battle_speed(&values), 0, "a wild value ({wild}) is Off");
        }
    }

    #[test]
    fn the_battle_speed_choices_round_trip_through_the_row() {
        let row = ROWS.iter().find(|row| row.key == BATTLE_SPEED).unwrap();
        let mut cheats = crate::cheats::Cheats::default();
        for (choice, &text) in BATTLE_SPEED_CHOICES.iter().enumerate() {
            let mut values = Values::new();
            let _ = row.set(&mut values, &mut cheats, choice);
            assert_eq!(values.get(BATTLE_SPEED), Some(choice as i64));
            assert_eq!(row.index(&values, &cheats, 0), choice);
            assert_eq!(battle_speed(&values), choice);
            assert_eq!(row.text(choice), text);
        }
        assert_eq!(row.default_index(), 0, "Off by default");
    }

    #[test]
    fn the_stored_choices_map_to_their_frames_and_clamp() {
        let mut values = Values::new();
        values.set(AUTO_RUN, 1);
        values.set(AUTO_WAIT, 0);
        values.set(ANIM_SPEED, 3);
        assert!(auto_run_on(&values));
        assert_eq!(auto_wait_frames(&values), 45);
        assert_eq!(anim_factor(&values), 4);
        values.set(AUTO_WAIT, 9);
        values.set(ANIM_SPEED, -2);
        assert_eq!(auto_wait_frames(&values), 180, "a wild value is the last choice");
        assert_eq!(anim_factor(&values), 1, "a negative value is Normal");
    }

    #[test]
    fn each_label_list_matches_its_values() {
        assert_eq!(AUTO_WAITS.len(), AUTO_WAIT_FRAMES.len());
        assert_eq!(ANIM_SPEEDS.len(), ANIM_FACTORS.len());
        assert_eq!(TEXT_SPEEDS.len(), 5, "five steps, as the original");
    }

    #[test]
    fn the_data_packs_row_is_a_button_that_opens_the_packs_and_stores_nothing() {
        let row = ROWS.last().expect("the Data packs row");
        assert_eq!(row.key, "gameplay.data_packs");
        assert_eq!(row.source, Source::Opens(Opens::Packs));
        assert_eq!(row.text(0), "Open");
        assert_eq!(row.count(), 1, "a button has one value, which is its text");
        assert!(!row.is_info(), "a button has a cursor");
    }
}
