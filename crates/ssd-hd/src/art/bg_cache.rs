//! The art of the background layers read at the picture's scale, kept by their last use within a
//! budget of bytes (see [`BgImages`]).

use crate::bgart::{BgArt, BgImage};
use rustc_hash::FxHashMap;
use std::sync::Arc;

/// The memory budget of the background layers' art at the picture's scale, in bytes: the one limit
/// of their cache. Past it the layer used longest ago goes, one at a time. A frame binds two layers
/// at most (`bgart::MAX_SLOTS`), and a scene can show three or more in turn: the budget holds them
/// all. The layers of the shared pack (v0.3.9, 77 layers) are 256 x 176 to 512 x 512 game pixels;
/// at 8x one of 256 x 176 (art file 1024 x 704) is 2048 x 1408 RGBA, 11 MB, so the budget holds 23
/// of them, and one of 512 x 512 is 64 MB, so it holds 4, as many as the cache held before it was
/// a budget (it was emptied whole past four layers). At 4x it holds 16 of the biggest. A layer kept
/// by the colour transfer of the last pictures (`ArtState::numbered`) stays in memory until that
/// lets it go too.
pub(crate) const BG_BUDGET: usize = 256 << 20;

/// The background layers' art kept: by the index of the entry, the art (`None` for files that
/// cannot be read or are not the layer's size) and its last use.
#[derive(Default)]
pub(crate) struct BgImages {
    held: FxHashMap<u32, Held>,
    /// The bytes of the pixels of `held`.
    bytes: usize,
    /// The count of the uses: the last use of a layer is its `used`.
    clock: u64,
    /// The layers read since the cache was made (a read is a miss of [`BgImages::get`]).
    pub(crate) reads: u64,
    /// The layers let go to keep within the budget since the cache was made (not those of [`BgImages::clear`]).
    pub(crate) evicted: u64,
}

struct Held {
    art: Option<Arc<BgImage>>,
    used: u64,
}

/// The bytes of the pixels of a layer's art: the art at the picture's scale and its source.
fn bytes_of(art: Option<&Arc<BgImage>>) -> usize {
    art.map_or(0, |image| {
        let art = match &image.art {
            BgArt::Whole(art) => art.rgba.len(),
            BgArt::Tiled(tiles) => tiles.values().map(|tile| tile.rgba.len()).sum(),
        };
        art + image.source.as_ref().map_or(0, |source| source.rgba.len())
    })
}

impl BgImages {
    /// Drops every layer (the picture's scale changed).
    pub(crate) fn clear(&mut self) {
        self.held.clear();
        self.bytes = 0;
    }

    /// The art of entry `at` if it is kept (`Some(None)`: kept as unreadable), now its last use.
    pub(crate) fn get(&mut self, at: u32) -> Option<&Option<Arc<BgImage>>> {
        let held = self.held.get_mut(&at)?;
        self.clock += 1;
        held.used = self.clock;
        Some(&held.art)
    }

    /// Whether entry `at` is kept.
    pub(crate) fn contains(&self, at: u32) -> bool {
        self.held.contains_key(&at)
    }

    /// Keeps `art` for entry `at`, the layers used longest ago going first while the layers would
    /// hold more than `budget` bytes.
    pub(crate) fn insert(&mut self, at: u32, art: Option<Arc<BgImage>>, budget: usize) {
        let size = bytes_of(art.as_ref());
        if let Some(old) = self.held.remove(&at) {
            self.bytes -= bytes_of(old.art.as_ref());
        }
        while self.bytes + size > budget
            && let Some(oldest) =
                self.held.iter().min_by_key(|(_, held)| held.used).map(|(at, _)| *at)
        {
            if let Some(gone) = self.held.remove(&oldest) {
                self.bytes -= bytes_of(gone.art.as_ref());
                self.evicted += 1;
            }
        }
        self.clock += 1;
        self.bytes += size;
        self.held.insert(at, Held { art, used: self.clock });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::Image;

    fn layer(bytes: usize) -> Option<Arc<BgImage>> {
        let art = Image::new(bytes / 4, 1, vec![0; bytes]);
        Arc::new(BgImage { art: BgArt::Whole(art), base: None, source: None }).into()
    }

    #[test]
    fn past_the_budget_the_layer_used_longest_ago_goes_and_only_it() {
        let mut cache = BgImages::default();
        for at in 0..4 {
            cache.insert(at, layer(100), 400);
        }
        // Layer 0 is used again: layer 1 is now the one used longest ago
        assert!(cache.get(0).is_some());
        cache.insert(4, layer(100), 400);
        let kept: Vec<u32> = (0..5).filter(|&at| cache.contains(at)).collect();
        assert_eq!(kept, [0, 2, 3, 4]);
        assert_eq!(cache.bytes, 400);
        assert_eq!(cache.evicted, 1, "one layer let go");
        // An unreadable layer takes no room and goes nothing
        cache.insert(5, None, 400);
        assert!(matches!(cache.get(5), Some(None)));
        assert_eq!((0..6).filter(|&at| cache.contains(at)).count(), 5);
        assert_eq!(cache.evicted, 1);
        cache.clear();
        assert_eq!(cache.evicted, 1, "emptied for another scale is not an eviction");
    }

    #[test]
    fn the_budget_holds_at_least_six_layers_of_256_x_176_at_8x_and_four_of_the_biggest() {
        let at_8x = |w: usize, h: usize| w * 8 * h * 8 * 4;
        assert!(BG_BUDGET / at_8x(256, 176) >= 6);
        assert!(BG_BUDGET / at_8x(512, 512) >= 4);
    }
}
