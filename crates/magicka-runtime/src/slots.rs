//! Per-player-slot storage without the original game's four-slot ceiling.
//!
//! The original game fixes every per-player table at four entries
//! (research/multiplayer-scaling.md). The rewrite keeps four as the default
//! player limit and grows these tables on demand when a larger lobby is
//! configured. Reading an absent slot yields the default value, so code
//! written for the original four slots stays valid for any slot index.

use std::ops::{Index, IndexMut};

/// The original game's player count. Lobbies default to this.
pub const ORIGINAL_PLAYER_LIMIT: usize = 4;

/// The party size encounter rules read. Extra monsters and boss attacks stop
/// growing at the original's four-player values, however large the lobby is
/// (project decision; research/rewrite-state-assessment.md).
pub fn encounter_players(players: usize) -> usize {
    players.min(ORIGINAL_PLAYER_LIMIT)
}

/// The count a script compares when it asks about the party. Retail scripts
/// list one branch per party size and test the largest with "exactly four", so
/// a larger party counts as four there; without this, five living wizards
/// match no branch and chs_necro summons nothing. Every other comparison sees
/// the true count, so `greater 4` still lets a mod detect a larger party.
pub fn scripted_party_count(actual: i32, expected: i32, method: i8) -> i32 {
    if method == 0 && expected == ORIGINAL_PLAYER_LIMIT as i32 {
        actual.min(expected)
    } else {
        actual
    }
}

/// Growable per-slot values. Reads beyond the stored length return `T`'s
/// default; writes grow the table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerSlot<T> {
    values: Vec<T>,
    absent: T,
}

impl<T: Default> Default for PerSlot<T> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            absent: T::default(),
        }
    }
}

impl<T: Clone + Default> PerSlot<T> {
    /// `count` copies of `value` (reads beyond still return the default).
    pub fn filled(count: usize, value: T) -> Self {
        Self {
            values: vec![value; count],
            absent: T::default(),
        }
    }
    pub fn from_array<const N: usize>(values: [T; N]) -> Self {
        Self {
            values: values.into(),
            absent: T::default(),
        }
    }
    /// Replace the stored values with `values`.
    pub fn with_values(mut self, values: &[T]) -> Self {
        self.values = values.to_vec();
        self
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    pub fn get(&self, slot: usize) -> Option<&T> {
        self.values.get(slot)
    }
    /// Grow to at least `count` entries.
    pub fn reserve_slots(&mut self, count: usize) {
        if self.values.len() < count {
            self.values.resize(count, T::default());
        }
    }
    /// Set every stored entry (and at least `count`) to `value`.
    pub fn fill(&mut self, count: usize, value: T) {
        self.reserve_slots(count);
        self.values.iter_mut().for_each(|v| *v = value.clone());
    }
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.values.iter()
    }
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.values.iter_mut()
    }
    pub fn as_slice(&self) -> &[T] {
        &self.values
    }
    /// The first `count` values, padded with defaults.
    pub fn to_vec(&self, count: usize) -> Vec<T> {
        (0..count).map(|i| self[i].clone()).collect()
    }
    /// The first `N` values, padded with defaults (for fixed original tables).
    pub fn to_array<const N: usize>(&self) -> [T; N] {
        std::array::from_fn(|i| self[i].clone())
    }
}

impl<T> Index<usize> for PerSlot<T> {
    type Output = T;
    fn index(&self, slot: usize) -> &T {
        self.values.get(slot).unwrap_or(&self.absent)
    }
}

impl<T: Clone + Default> IndexMut<usize> for PerSlot<T> {
    fn index_mut(&mut self, slot: usize) -> &mut T {
        self.reserve_slots(slot + 1);
        &mut self.values[slot]
    }
}

impl<'a, T> IntoIterator for &'a PerSlot<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}

impl<T> IntoIterator for PerSlot<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a mut PerSlot<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter_mut()
    }
}

impl<T: Clone + Default, const N: usize> From<[T; N]> for PerSlot<T> {
    fn from(values: [T; N]) -> Self {
        Self::from_array(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_beyond_length_are_default_and_writes_grow() {
        let mut s = PerSlot::filled(ORIGINAL_PLAYER_LIMIT, 1u64);
        assert_eq!(s[3], 1);
        assert_eq!(s[40], 0);
        assert_eq!(s.len(), 4);
        s[9] = 7;
        assert_eq!(s.len(), 10);
        assert_eq!(s[9], 7);
        assert_eq!(s[5], 0);
        assert_eq!(s.to_array::<2>(), [1, 1]);
        s.fill(12, 3);
        assert_eq!(s.len(), 12);
        assert!(s.iter().all(|v| *v == 3));
    }
}
