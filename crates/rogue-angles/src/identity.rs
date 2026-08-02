// Generic roguelike item-identification framework: a per-run shuffled bijection between an
// "appearance" (what the player sees before identifying something — a potion color, a scroll's
// made-up title) and an "effect" (what it actually does), plus which appearances the player has
// identified so far. The engine owns this shape; it has no idea what a potion or a scroll is —
// a game instantiates one `IdentityTable<A, E>` per identifiable item category and keeps its
// inventory model (which slots hold which appearances, how many of each) entirely to itself.
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use rand::Rng;
use rand::seq::SliceRandom;

/// Produces a derangement of `0..n`: a permutation where no index maps to itself (for n >= 2),
/// so every chosen appearance ends up with a *different* effect than it started with. Used by
/// `IdentityTable::forget_some` to scramble a subset of known appearances' hidden effects among
/// themselves, keeping the overall appearance→effect bijection intact.
pub fn derange_indices(n: usize, rng: &mut impl Rng) -> Vec<usize> {
    let mut perm: Vec<usize> = (0..n).collect();
    if n < 2 {
        return perm;
    }
    loop {
        perm.shuffle(rng);
        if perm.iter().enumerate().all(|(i, &j)| i != j) {
            return perm;
        }
    }
}

/// A per-run shuffled `A` → `E` bijection, plus which `A`s have been identified.
pub struct IdentityTable<A, E> {
    mapping: HashMap<A, E>,
    identified: HashSet<A>,
}

impl<A, E> Default for IdentityTable<A, E> {
    fn default() -> Self { Self { mapping: HashMap::new(), identified: HashSet::new() } }
}

impl<A: Copy + Eq + Hash, E: Copy> IdentityTable<A, E> {
    /// Shuffles `effects` and assigns one to each of `appearances`, forgetting all prior
    /// identifications. `effects` and `appearances` must be the same length — call once per new
    /// game, not on every level (a mid-game reshuffle would retroactively change the effect of
    /// items the player already identified).
    pub fn randomize(&mut self, appearances: &[A], effects: &[E], rng: &mut impl Rng) {
        self.identified.clear();
        let mut shuffled = effects.to_vec();
        shuffled.shuffle(rng);
        self.mapping = appearances.iter().copied().zip(shuffled).collect();
    }

    pub fn effect_of(&self, appearance: A) -> Option<E> { self.mapping.get(&appearance).copied() }

    pub fn is_identified(&self, appearance: A) -> bool { self.identified.contains(&appearance) }

    pub fn identify(&mut self, appearance: A) { self.identified.insert(appearance); }

    pub fn known_count(&self) -> usize { self.identified.len() }

    /// Implements a Scroll-of-Forgetting-style scramble: choose up to `count` known appearances
    /// at random, forget their identifications, and permute (derange) their hidden effects among
    /// one another. Returns the number of appearances scrambled, or `None` if fewer than two
    /// appearances are known (a derangement needs at least two to move between).
    pub fn forget_some(&mut self, count: usize, rng: &mut impl Rng) -> Option<usize> {
        let known: Vec<A> = self.identified.iter().copied().collect();
        if known.len() < 2 {
            return None;
        }
        let chosen: Vec<A> = known.choose_multiple(rng, count).copied().collect();
        let effects: Vec<E> = chosen.iter().map(|a| self.mapping[a]).collect();
        let perm = derange_indices(chosen.len(), rng);
        for (i, &a) in chosen.iter().enumerate() {
            self.mapping.insert(a, effects[perm[i]]);
            self.identified.remove(&a);
        }
        Some(chosen.len())
    }
}
