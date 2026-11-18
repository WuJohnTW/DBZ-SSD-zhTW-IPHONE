//! The card tooltip (Enhanced mode only): while the pick cursor (a down arrow, one of the cards'
//! tiles) is over a card, a small window in the game's style says what the card's kanji means.
//!
//! The cursor is the arrow tile of the UI set drawn in the row above the hand; the card it is on is
//! the one whose kanji tile sits 24 pixels under it. The arrow blinks, so the tooltip is kept for a
//! few frames after the arrow was last seen, as long as the card's kanji is still on the screen.
//! The window has the colours of the game's own windows (cream, an orange band, a dark red line,
//! corner caps), the text is the HD font, and the kanji (or the emblem) is the art store's picture.
//! What each symbol of a card is called and means is in `docs/re/data/cards.md`.
//!
//! The tooltip is not drawn while a window of the game's own is open where it would be (the technique list or the
//! target window that opens once the card is picked: the card and its cursor may stay on the screen under it), nor
//! at all with a translation that turns it off (`translate::CARD_TOOLTIP_KEY`: the zh-tw pack, whose players read the
//! cards' kanji as they are).

use crate::cards::{CARD_BASE, CURSOR_CODE, CardArt};
use crate::font::{Font, MARGIN, TextCell};
use crate::translate::Overlay;

/// One symbol of the cards: the code of the tile at the top left of its kanji in the UI set, the
/// reading (the first line, after the picture) and a short meaning (the second line). Only the
/// letters the HD font has: a-z, A-Z, digits, `-`, `.`, `,`, `?`, `!`.
pub(crate) struct Tip {
    pub code: u16,
    pub reading: &'static str,
    pub meaning: &'static str,
}

/// The seven symbols a card can carry (symbol 0 to 6 of `docs/re/data/cards.md`).
pub(crate) const TIPS: [Tip; 7] = [
    Tip { code: 0xF4, reading: "Kai", meaning: "World" },
    Tip { code: 0xF8, reading: "Kami", meaning: "God" },
    Tip { code: 0xF2, reading: "Ma", meaning: "Demon" },
    Tip { code: 0xF6, reading: "Waku", meaning: "Doubt" },
    Tip { code: 0xFA, reading: "Ginyu", meaning: "Force" },
    Tip { code: 0xF0, reading: "Hitsu", meaning: "Sure" },
    Tip { code: 0xFC, reading: "Chou", meaning: "Super" },
];

/// The reading and the meaning of each of the seven symbols, symbol 0 to 6, as the tooltip says them: the HD Cards screen
/// of the menus says the same words.
#[must_use]
pub fn symbol_words() -> [(&'static str, &'static str); 7] {
    std::array::from_fn(|symbol| (TIPS[symbol].reading, TIPS[symbol].meaning))
}

/// Draws that follow the cursor for this many frames after it was last seen.
const KEEP: u32 = 48;
/// The cards of a hand and the width of one, in pixels: the hand spans five of them, whatever is
/// hidden of it at the moment.
const HAND_CARDS: i32 = 5;
const CARD_WIDTH: i32 = 32;
/// The original's own message for a favourite card ("Attack everyone!", seen with the pick
/// cursor on a card whose kanji is the fighter's class) opens a window to the right of that card
/// that covers the cards after it; its top edge is 7 pixels above the cursor's tile row. The
/// tooltip never reaches into it: it ends this many pixels above the cursor's tile row.
const ABOVE_CURSOR: i32 = 9;
/// The kanji tile is this many pixels under the cursor tile.
const BELOW: i32 = 24;
/// Pixels of the frame (cream edge, orange band, dark red line) and of padding inside it.
const FRAME: i32 = 4;
const PAD: i32 = 3;
const PICTURE: i32 = 14;
/// Pixels of a character cell, and from one line to the next.
const LINE: i32 = 8;
const PITCH: i32 = 9;
/// The height of the two lines.
const TEXT: i32 = PITCH + LINE;

const CREAM: [u8; 3] = [0xFF, 0xEF, 0xCE];
const ORANGE: [u8; 3] = [0xFF, 0x52, 0x00];
const DARK: [u8; 3] = [0x84, 0x00, 0x00];
const INK: [u8; 3] = [0x00, 0x00, 0x31];

/// The tooltip being shown: the card's kanji cell, the cursor's row and the tint of the cursor
/// (a fade darkens it), and how many frames ago the cursor was seen.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Seen {
    kanji: (i32, i32),
    cursor_y: i32,
    tip: usize,
    white: [u8; 3],
    /// The right edge of the hand, in 1x pixels: its five cards from its left edge, or the last
    /// card tile on the screen when that is further right.
    hand_right: i32,
    age: u32,
}

/// The tooltip's window: its left, top, width and height in 1x pixels, in a frame `frame_w` pixels wide.
fn place(seen: &Seen, frame_w: i32) -> (i32, i32, i32, i32) {
    let tip = &TIPS[seen.tip];
    let chars = tip.reading.len().max(tip.meaning.len()) as i32;
    let inner_w = PICTURE + PAD + LINE * chars;
    let (win_w, win_h) = (inner_w + 2 * (FRAME + PAD), TEXT + 2 * (FRAME + PAD));
    // Left edge on the card's, kept within the hand's span (and the picture)
    let hand_right = seen.hand_right.min(frame_w);
    let win_x = (seen.kanji.0 - 8).min(hand_right - win_w).clamp(0, (frame_w - win_w).max(0));
    let win_y = seen.cursor_y - ABOVE_CURSOR - win_h;
    (win_x, win_y, win_w, win_h)
}

/// Whether a window of the game's own is open where the tooltip's `window` would be: a text cell of the frame that
/// is not a card's lies in it (a letter, a mark, or the blank inside of a window: the 2-bit tiles of the font the
/// frame shows, `font::TextCapture`). With the cursor on a card, the picture above the hand is the fight's, with no
/// text in it.
fn under_a_window(cells: &[TextCell], (x, y, w, h): (i32, i32, i32, i32)) -> bool {
    cells.iter().any(|cell| {
        cell.glyph < CARD_BASE
            && cell.mask != 0
            && cell.x < x + w
            && x < cell.x + 8
            && cell.y < y + h
            && y < cell.y + 8
    })
}

/// The tooltip for the cards in `cells`, over the picture `rgba` (`width` pixels wide, `scale`
/// times the 1x frame). Does nothing without card art, a cursor or a card under it, with a window of the game's own
/// where it would be ([`under_a_window`]), or with `overlay`'s translation turning it off.
pub(crate) fn draw(
    font: &Font,
    rgba: &mut [u8],
    width: usize,
    scale: usize,
    cells: &[TextCell],
    overlay: Option<&Overlay>,
) {
    if overlay.is_some_and(Overlay::card_tooltip_off) {
        return;
    }
    let Some(cards) = font.cards() else { return };
    if !(2..=crate::MAX_SCALE as usize).contains(&scale) || cards.tips.is_empty() {
        return;
    }
    let Some(cursor) = cards.glyph_of(CURSOR_CODE) else { return };
    let tip_of = |cell: &TextCell| -> Option<usize> {
        let code = cards.code_of(cell.glyph)?;
        TIPS.iter().position(|tip| tip.code == code)
    };
    let found = cells.iter().filter(|cell| cell.glyph == cursor && cell.mask != 0).find_map(|c| {
        let kanji =
            cells.iter().find(|k| k.x == c.x && k.y == c.y + BELOW && k.glyph >= CARD_BASE)?;
        Some(Seen {
            kanji: (kanji.x, kanji.y),
            cursor_y: c.y,
            tip: tip_of(kanji)?,
            white: c.white?,
            hand_right: {
                let hand = || cells.iter().filter(|k| k.glyph >= CARD_BASE && k.y > c.y);
                let seen = hand().map(|k| k.x + 8).max().unwrap_or(0);
                // The original's message hides the cards after the one under the cursor: the
                // hand is as wide as its five cards, whatever shows
                let full = hand().map(|k| k.x).min().map_or(0, |x| x + HAND_CARDS * CARD_WIDTH);
                seen.max(full)
            },
            age: 0,
        })
    });
    let shown = {
        let mut seen = cards.seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match (found, seen.as_mut()) {
            (Some(now), _) => *seen = Some(now),
            (None, Some(last)) => {
                last.age += 1;
                // The card must still be there, under the same symbol
                let there = cells
                    .iter()
                    .any(|cell| (cell.x, cell.y) == last.kanji && tip_of(cell) == Some(last.tip));
                if last.age > KEEP || !there {
                    *seen = None;
                }
            }
            (None, None) => {}
        }
        *seen
    };
    if let Some(seen) = shown {
        let window = place(&seen, (width / scale) as i32);
        if !under_a_window(cells, window) {
            paint(font, cards, rgba, width, scale, &seen, window);
        }
    }
}

/// A canvas over the picture: coordinates in pixels of the 1x frame (the window's), the colours
/// tinted by the cursor's white.
struct Canvas<'a> {
    rgba: &'a mut [u8],
    width: usize,
    height: usize,
    scale: usize,
    tint: [u8; 3],
}

impl Canvas<'_> {
    fn put(&mut self, x: i32, y: i32, colour: [u8; 3], alpha: u32) {
        if alpha == 0 || x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }
        let at = (y as usize * self.width + x as usize) * 4;
        for (channel, (&colour, &tint)) in colour.iter().zip(&self.tint).enumerate() {
            let ink = u32::from(colour) * u32::from(tint) / 255;
            let under = u32::from(self.rgba[at + channel]);
            self.rgba[at + channel] = ((under * (255 - alpha) + ink * alpha + 127) / 255) as u8;
        }
    }

    /// Fills the 1x pixels `x0..x1`, `y0..y1`.
    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, colour: [u8; 3]) {
        let s = self.scale as i32;
        for y in y0 * s..y1 * s {
            for x in x0 * s..x1 * s {
                self.put(x, y, colour, 255);
            }
        }
    }

    /// A disc of radius `r` (1x pixels) at (`cx`, `cy`), anti-aliased, over `colour`.
    fn disc(&mut self, cx: f32, cy: f32, r: f32, colour: [u8; 3]) {
        let s = self.scale as f32;
        let (x0, x1) = (((cx - r - 1.0) * s) as i32, ((cx + r + 1.0) * s) as i32);
        let (y0, y1) = (((cy - r - 1.0) * s) as i32, ((cy + r + 1.0) * s) as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                let (dx, dy) = ((x as f32 + 0.5) / s - cx, (y as f32 + 0.5) / s - cy);
                let edge = (r - (dx * dx + dy * dy).sqrt()) * s;
                let alpha = (edge + 0.5).clamp(0.0, 1.0);
                self.put(x, y, colour, (alpha * 255.0) as u32);
            }
        }
    }
}

fn paint(
    font: &Font,
    cards: &CardArt,
    rgba: &mut [u8],
    width: usize,
    scale: usize,
    seen: &Seen,
    (win_x, win_y, win_w, win_h): (i32, i32, i32, i32),
) {
    let tip = &TIPS[seen.tip];
    let height = rgba.len() / 4 / width;
    let mut canvas = Canvas { rgba, width, height, scale, tint: seen.white };
    // The frame: a cream edge, the orange band, the dark red line, the cream inside
    canvas.rect(win_x, win_y, win_x + win_w, win_y + win_h, CREAM);
    canvas.rect(win_x + 1, win_y + 1, win_x + win_w - 1, win_y + win_h - 1, ORANGE);
    canvas.rect(win_x + 3, win_y + 3, win_x + win_w - 3, win_y + win_h - 3, DARK);
    canvas.rect(win_x + FRAME, win_y + FRAME, win_x + win_w - FRAME, win_y + win_h - FRAME, CREAM);
    // The corner caps: an orange disc in a dark red ring over each corner
    for (cx, cy) in [
        (win_x + 3, win_y + 3),
        (win_x + win_w - 3, win_y + 3),
        (win_x + 3, win_y + win_h - 3),
        (win_x + win_w - 3, win_y + win_h - 3),
    ] {
        canvas.disc(cx as f32, cy as f32, 3.4, DARK);
        canvas.disc(cx as f32, cy as f32, 2.5, ORANGE);
    }
    let (left, top) = (win_x + FRAME + PAD, win_y + FRAME + PAD);
    // The kanji (or emblem), centred in its 14 x 14 box
    if let Some(picture) = cards.tip_picture(scale, tip.code) {
        let side = PICTURE as usize * scale;
        for py in 0..side {
            for px in 0..side {
                let pixel = &picture[(py * side + px) * 4..][..4];
                let at = (
                    (top + (TEXT - PICTURE) / 2) * scale as i32 + py as i32,
                    left * scale as i32 + px as i32,
                );
                canvas.put(at.1, at.0, [pixel[0], pixel[1], pixel[2]], u32::from(pixel[3]));
            }
        }
    }
    // The two lines of text, a character to a cell of 8 pixels
    let text_x = left + PICTURE + PAD;
    for (row, line) in [tip.reading, tip.meaning].into_iter().enumerate() {
        for (column, ch) in line.chars().enumerate() {
            let Some(mask) = font.char_mask(scale, ch) else { continue };
            let side = (8 + 2 * MARGIN) * scale;
            let origin = (
                (text_x + column as i32 * LINE - MARGIN as i32) * scale as i32,
                (top + row as i32 * PITCH - MARGIN as i32) * scale as i32,
            );
            for my in 0..side {
                for mx in 0..side {
                    let alpha = u32::from(mask[my * side + mx]);
                    canvas.put(origin.0 + mx as i32, origin.1 + my as i32, INK, alpha);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A font over a cartridge image with the UI tiles empty but for their own codes, with card art
    /// for the cursor tile and the kanji of Kami (`0xF8`) and its tooltip picture.
    fn font() -> (Font, u8, u8) {
        let mut rom = vec![0_u8; crate::font::FONT_OFFSET + 384 * 16];
        for code in 0..384 {
            rom[crate::font::FONT_OFFSET + code * 16 + 15] = (code % 250) as u8 + 1;
        }
        let tips =
            vec![(0xF8_u16, [0, 0, 49, 255].repeat(crate::cards::TIP_PX * crate::cards::TIP_PX))];
        let cards = Arc::new(CardArt::flat(&[CURSOR_CODE, 0xF8], tips));
        let font = Font::standard(&rom).unwrap().with_cards(&rom, cards).unwrap();
        (font, CARD_BASE, CARD_BASE + 1)
    }

    fn cell(x: i32, y: i32, glyph: u8) -> TextCell {
        TextCell {
            x,
            y,
            glyph,
            half: 0,
            clear: false,
            bold: false,
            layer: 0,
            fg: [0; 3],
            bg: [255; 3],
            mid: [255; 3],
            white: Some([255; 3]),
            mask: !0,
        }
    }

    const WIDTH: usize = 256 * 2;

    fn picture() -> Vec<u8> {
        vec![7_u8; WIDTH * 224 * 2 * 4]
    }

    /// Whether the picture holds any pixel the tooltip's window drew (its cream).
    fn drawn(rgba: &[u8]) -> bool {
        rgba.chunks(4).any(|p| p[..3] == CREAM)
    }

    #[test]
    fn the_tooltip_shows_over_the_card_under_the_cursor_and_stays_when_the_cursor_blinks() {
        let (font, cursor, kanji) = font();
        let cards = font.cards().unwrap();
        // The cursor at (88, 176), the card's kanji 24 pixels below it
        let both = [cell(88, 176, cursor), cell(88, 200, kanji)];
        let mut rgba = picture();
        draw(&font, &mut rgba, WIDTH, 2, &both, None);
        assert!(drawn(&rgba));
        // The window is above the cursor row and its left edge is the card's
        let rows: Vec<usize> =
            (0..224 * 2).filter(|&y| rgba[(y * WIDTH + 2 * 80 + 2) * 4..][..3] == CREAM).collect();
        assert!(rows.iter().all(|&y| y < 176 * 2), "above the cursor: {rows:?}");
        // The cursor blinks off: the card is still there, so the window stays
        let card_only = [cell(88, 200, kanji)];
        let mut rgba = picture();
        draw(&font, &mut rgba, WIDTH, 2, &card_only, None);
        assert!(drawn(&rgba), "kept while the cursor blinks");
        // ... but not for ever
        let mut gone = false;
        for _ in 0..=KEEP {
            let mut rgba = picture();
            draw(&font, &mut rgba, WIDTH, 2, &card_only, None);
            gone = !drawn(&rgba);
        }
        assert!(gone, "dropped after {KEEP} frames without the cursor");
        assert!(cards.seen.lock().unwrap().is_none());
    }

    #[test]
    fn the_originals_message_for_a_favourite_card_stays_visible_beside_the_tooltip() {
        let (font, cursor, kanji) = font();
        // The first card of the hand is under the cursor (arrow tile row 174, kanji 24 below).
        // The original's message window opens to its right, from x 112 and y 167 to the bottom,
        // and hides the other cards: only the first card is on screen. The tooltip must neither
        // reach into that window nor cover the command window to the left of the hand (x < 80)
        let cells = [cell(88, 174, cursor), cell(88, 198, kanji), cell(80, 198, CARD_BASE + 2)];
        let mut rgba = picture();
        draw(&font, &mut rgba, WIDTH, 2, &cells, None);
        assert!(drawn(&rgba));
        let tinted = |x: usize, y: usize| rgba[(y * WIDTH + x) * 4..][..3] == CREAM;
        let message = (112 * 2..WIDTH).any(|x| (167 * 2..224 * 2).any(|y| tinted(x, y)));
        assert!(!message, "the tooltip stays out of the message window");
        let command = (0..80 * 2).any(|x| (0..224 * 2).any(|y| tinted(x, y)));
        assert!(!command, "the tooltip stays off the command window");
    }

    #[test]
    fn no_tooltip_without_a_cursor_on_a_card_or_when_the_card_is_gone() {
        let (font, cursor, kanji) = font();
        // A cursor with no card under it (a dialogue's arrow), a card with no cursor
        for cells in [vec![cell(88, 176, cursor)], vec![cell(88, 200, kanji)]] {
            let mut rgba = picture();
            draw(&font, &mut rgba, WIDTH, 2, &cells, None);
            assert!(!drawn(&rgba));
        }
        // The cursor was on a card; the card is played, the window goes at once
        let mut rgba = picture();
        draw(&font, &mut rgba, WIDTH, 2, &[cell(88, 176, cursor), cell(88, 200, kanji)], None);
        assert!(drawn(&rgba));
        let mut rgba = picture();
        draw(&font, &mut rgba, WIDTH, 2, &[], None);
        assert!(!drawn(&rgba));
        // At scale 1 nothing is drawn
        let mut rgba = picture();
        draw(&font, &mut rgba, WIDTH, 1, &[cell(88, 176, cursor), cell(88, 200, kanji)], None);
        assert!(!drawn(&rgba));
    }

    #[test]
    fn the_window_stays_within_the_hand_and_the_picture() {
        let (font, cursor, kanji) = font();
        // A hand of five cards from x 80 (their kanji 8 pixels in), the cursor on the last: the
        // window's right edge is the hand's (x 240)
        let mut cells: Vec<TextCell> =
            (0..5).map(|slot| cell(88 + 32 * slot, 198, kanji)).collect();
        cells.push(cell(80, 198, CARD_BASE + 2)); // the first card's frame
        cells.push(cell(88 + 32 * 4, 174, cursor));
        let mut rgba = picture();
        draw(&font, &mut rgba, WIDTH, 2, &cells, None);
        let columns: Vec<usize> = (0..WIDTH)
            .filter(|&x| (0..224 * 2).any(|y| rgba[(y * WIDTH + x) * 4..][..3] == CREAM))
            .collect();
        assert!(*columns.last().unwrap() < 2 * 240 + 2, "{columns:?}");
        assert!(*columns.last().unwrap() >= 2 * 240 - 4, "{columns:?}");
    }

    /// A font with card art for the cursor and the seven symbols (glyph `CARD_BASE + 1 + symbol`), each
    /// tooltip picture a colour of its own.
    fn every_symbol_font() -> Font {
        let mut rom = vec![0_u8; crate::font::FONT_OFFSET + 384 * 16];
        for code in 0..384 {
            rom[crate::font::FONT_OFFSET + code * 16 + 15] = (code % 250) as u8 + 1;
        }
        let mut codes = vec![CURSOR_CODE];
        codes.extend(TIPS.iter().map(|tip| tip.code));
        let tips = TIPS
            .iter()
            .enumerate()
            .map(|(symbol, tip)| {
                let px = crate::cards::TIP_PX * crate::cards::TIP_PX;
                let pixel = [30 * symbol as u8, 200 - 20 * symbol as u8, 90, 255];
                (tip.code, pixel.repeat(px))
            })
            .collect();
        let cards = Arc::new(CardArt::flat(&codes, tips));
        Font::standard(&rom).unwrap().with_cards(&rom, cards).unwrap()
    }

    /// A picture that is not one colour, so that every blend shows.
    fn gradient(scale: usize) -> Vec<u8> {
        let (w, h) = (256 * scale, 224 * scale);
        (0..w * h)
            .flat_map(|at| [(at % w) as u8, (at / w) as u8, (at * 7 % 251) as u8, 255])
            .collect()
    }

    /// FNV-1a of the whole picture.
    fn fnv(rgba: &[u8]) -> u64 {
        rgba.iter()
            .fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
    }

    /// The cells of a full hand, the kanji from x 88, the cursor on the card of `slot`, every card of `symbol`.
    fn hand(symbol: usize, slot: i32) -> Vec<TextCell> {
        let kanji = CARD_BASE + 1 + symbol as u8;
        let mut cells: Vec<TextCell> = (0..5).map(|s| cell(88 + 32 * s, 198, kanji)).collect();
        cells.push(cell(88 + 32 * slot, 174, CARD_BASE));
        cells
    }

    /// The whole picture at `scale` with the tooltip `cells` show, drawn with a font of its own.
    fn shown(cells: &[TextCell], scale: usize, overlay: Option<&Overlay>) -> Vec<u8> {
        let mut rgba = gradient(scale);
        draw(&every_symbol_font(), &mut rgba, 256 * scale, scale, cells, overlay);
        rgba
    }

    /// The whole picture (FNV-1a of every byte) with the English tooltip of each symbol, the cursor on the card of
    /// slot 0, 2 or 4 at a scale of 2, 3 or 4, as the tooltip drew it before its layout for translations.
    const ENGLISH_BEFORE: [(usize, i32, usize, u64); 21] = [
        (0, 0, 2, 0xb3ba_b099_61cf_b9ba),
        (0, 2, 3, 0x07f0_89bb_a378_3c9d),
        (0, 4, 4, 0x38e9_45ac_3d0b_f9d3),
        (1, 0, 2, 0x1e14_1387_33a2_e003),
        (1, 2, 3, 0xed3f_d651_893c_e14a),
        (1, 4, 4, 0x224f_d92c_ef44_b349),
        (2, 0, 2, 0x2ffd_73d4_5bbf_a3ab),
        (2, 2, 3, 0xbf92_b2a7_c584_eada),
        (2, 4, 4, 0x2849_34dc_0d7c_a70d),
        (3, 0, 2, 0x887c_df37_6d63_57fe),
        (3, 2, 3, 0xe0d1_5538_2e70_512f),
        (3, 4, 4, 0xbc41_c477_7408_46b5),
        (4, 0, 2, 0xe35f_676d_3adc_f163),
        (4, 2, 3, 0xfda2_706b_f5db_93cd),
        (4, 4, 4, 0x61dd_2eec_255d_2b09),
        (5, 0, 2, 0xa42f_5353_1e75_9e82),
        (5, 2, 3, 0x50b9_ec85_9d69_2904),
        (5, 4, 4, 0x6698_c6bb_0ab9_e699),
        (6, 0, 2, 0x82ab_08dd_2407_ee19),
        (6, 2, 3, 0xd835_0a78_beb4_83a4),
        (6, 4, 4, 0xce07_481c_b6de_f471),
    ];

    #[test]
    fn the_english_tooltip_is_drawn_as_before_to_the_pixel() {
        for (symbol, slot, scale, before) in ENGLISH_BEFORE {
            let rgba = shown(&hand(symbol, slot), scale, None);
            assert_ne!(rgba, gradient(scale), "symbol {symbol}: a tooltip is drawn");
            assert_eq!(fnv(&rgba), before, "symbol {symbol}, slot {slot}, scale {scale}");
        }
    }

    /// The cells of the game's window from (`x0`, `y0`) to (`x1`, `y1`): a line of letters every 16 pixels from its
    /// top, blanks between, or blanks only.
    fn window_cells((x0, y0): (i32, i32), (x1, y1): (i32, i32), letters: bool) -> Vec<TextCell> {
        let mut cells = Vec::new();
        for y in (y0..y1).step_by(8) {
            for x in (x0..x1).step_by(8) {
                let letter = letters && (y - y0) % 16 == 0 && x < x0 + 80;
                cells.push(cell(x, y, if letter { 0x20 } else { 0 }));
            }
        }
        cells
    }

    /// Once 必 is picked: the card stays in the first slot with its cursor (the hand's other cards gone), and the game
    /// opens the technique list (x 112 to 248, y 80 to 200) or, for a plain attack, the target window (x 112 to 248,
    /// y 136 to 224), both over where the tooltip would be.
    fn picked(window: &str, letters: bool) -> Vec<TextCell> {
        let mut cells = hand(5, 0);
        cells.retain(|c| c.x == 88);
        let (top, bottom) = if window == "list" { (80, 200) } else { (136, 224) };
        cells.extend(window_cells((112, top), (248, bottom), letters));
        cells
    }

    #[test]
    fn no_tooltip_while_a_window_of_the_game_is_where_it_would_be() {
        for window in ["list", "target"] {
            for letters in [true, false] {
                let cells = picked(window, letters);
                assert_eq!(shown(&cells, 2, None), gradient(2), "{window}, letters {letters}");
                // The same frame without the window: the tooltip is there
                let card: Vec<TextCell> =
                    cells.into_iter().filter(|c| c.glyph >= CARD_BASE).collect();
                assert_ne!(shown(&card, 2, None), gradient(2), "{window}: without it");
            }
        }
        // The game's windows beside the tooltip do not hide it: the command window left of the hand (x < 80), the
        // favourite card's message from y 167, the status window at the top
        let mut beside = hand(5, 0);
        beside.extend([
            cell(16, 150, 0x20),
            cell(72, 140, 0),
            cell(120, 168, 0x20),
            cell(200, 176, 0),
            cell(100, 40, 0x20),
        ]);
        assert_eq!(shown(&beside, 2, None), shown(&hand(5, 0), 2, None), "windows beside it");
    }

    /// A translation of `rows` (`key<TAB>text` lines) over a Latin font: the tooltip reads only its switch.
    fn overlay(rows: &str) -> Overlay {
        let table =
            ssd_catalogue::translation::Translation::parse(&format!("key\tzh\n{rows}")).unwrap();
        let face =
            crate::translate::ChineseFace::new(crate::font::SHARE_TECH_MONO.to_vec()).unwrap();
        Overlay::new(Arc::new(table), Arc::new(face))
    }

    #[test]
    fn a_translation_that_turns_the_tooltip_off_draws_none_at_any_step_of_the_pick() {
        let key = crate::translate::CARD_TOOLTIP_KEY;
        let off = overlay(&format!("{key}\toff\n"));
        // Picking a card: the cursor on each card of the hand, every symbol
        for symbol in 0..7 {
            for slot in 0..5 {
                let cells = hand(symbol, slot);
                assert_eq!(
                    shown(&cells, 3, Some(&off)),
                    gradient(3),
                    "symbol {symbol}, slot {slot}"
                );
                assert_ne!(
                    shown(&cells, 3, None),
                    gradient(3),
                    "English: symbol {symbol}, slot {slot}"
                );
            }
        }
        // The technique list open, the target window open
        for window in ["list", "target"] {
            assert_eq!(shown(&picked(window, true), 2, Some(&off)), gradient(2), "{window}");
        }
        // Nor while the cursor blinks off over a card it was on
        let font = every_symbol_font();
        let mut rgba = gradient(2);
        draw(&font, &mut rgba, 512, 2, &hand(5, 0), Some(&off));
        let card_only: Vec<TextCell> = hand(5, 0).into_iter().filter(|c| c.y == 198).collect();
        draw(&font, &mut rgba, 512, 2, &card_only, Some(&off));
        assert_eq!(rgba, gradient(2), "the cursor blinking");
    }

    #[test]
    fn a_translation_without_the_switch_keeps_the_tooltip_as_the_english() {
        // Another language pack (no switch, or another text) shows the tooltip as without a translation
        let key = crate::translate::CARD_TOOLTIP_KEY;
        for rows in [
            String::new(),
            "screen:chase_damage\t傷害\n".to_owned(),
            format!("{key}\ton\n"),
            format!("{key}\toff now\n"),
        ] {
            let other = overlay(&rows);
            for (symbol, slot, scale, before) in ENGLISH_BEFORE {
                let rgba = shown(&hand(symbol, slot), scale, Some(&other));
                assert_eq!(
                    fnv(&rgba),
                    before,
                    "{rows:?}: symbol {symbol}, slot {slot}, scale {scale}"
                );
            }
        }
    }

    #[test]
    fn every_symbol_has_text_the_font_can_draw() {
        let rom = vec![0_u8; crate::font::FONT_OFFSET + 384 * 16];
        let font = Font::standard(&rom).unwrap();
        for tip in &TIPS {
            for ch in tip.reading.chars().chain(tip.meaning.chars()) {
                assert!(font.char_mask(2, ch).is_some(), "{} has no glyph for {ch:?}", tip.reading);
            }
        }
        let mut codes: Vec<u16> = TIPS.iter().map(|tip| tip.code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), 7, "seven distinct symbols");
    }
}
