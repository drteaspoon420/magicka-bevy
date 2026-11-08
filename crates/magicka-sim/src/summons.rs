//! Character.mCurrentSummons bookkeeping (research/summons.md): at most one
//! undead, four flamers and sixteen summons per master.

/// What NonPlayerCharacter.Summoned recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SummonKind {
    Normal,
    Undead,
    Flamer,
}

/// Character.MAX summons array (16 slots) with the undead/flamer counts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SummonList {
    entries: Vec<(u16, SummonKind)>,
    undead: u32,
    flamers: u32,
}

pub const MAX_SUMMONS: usize = 16;
pub const MAX_UNDEAD: u32 = 1;
pub const MAX_FLAMERS: u32 = 4;

impl SummonList {
    pub fn summons(&self) -> impl Iterator<Item = (u16, SummonKind)> + '_ {
        self.entries.iter().copied()
    }

    /// RemoveUndead / RemoveFlamer: the first summon of that kind is
    /// removed and returned for OverKill.
    fn remove_first(&mut self, kind: SummonKind) -> Option<u16> {
        let i = self.entries.iter().position(|(_, k)| *k == kind)?;
        let (handle, _) = self.entries.remove(i);
        match kind {
            SummonKind::Undead => self.undead -= 1,
            SummonKind::Flamer => self.flamers -= 1,
            SummonKind::Normal => {}
        }
        Some(handle)
    }

    /// Character.OnSpawnedSummon: make room (the oldest undead past 1,
    /// flamers past 4, then the oldest summon past 16 are overkilled), then
    /// append. Returns the handles to OverKill, in order.
    pub fn on_spawned(&mut self, npc: u16, kind: SummonKind) -> Vec<u16> {
        let mut overkill = Vec::new();
        while self.undead >= MAX_UNDEAD {
            match self.remove_first(SummonKind::Undead) {
                Some(h) => overkill.push(h),
                None => break,
            }
        }
        while self.flamers >= MAX_FLAMERS {
            match self.remove_first(SummonKind::Flamer) {
                Some(h) => overkill.push(h),
                None => break,
            }
        }
        while self.entries.len() >= MAX_SUMMONS {
            let (h, k) = self.entries.remove(0);
            match k {
                SummonKind::Undead => self.undead -= 1,
                SummonKind::Flamer => self.flamers -= 1,
                SummonKind::Normal => {}
            }
            overkill.push(h);
        }
        self.entries.push((npc, kind));
        match kind {
            SummonKind::Undead => self.undead += 1,
            SummonKind::Flamer => self.flamers += 1,
            SummonKind::Normal => {}
        }
        overkill
    }

    /// Character.OnDespawnedSummon: a listed undead or flamer removes the
    /// *first* of its kind (RemoveUndead/RemoveFlamer, which also OverKills
    /// it); a normal summon is just removed. Returns the handles to OverKill.
    pub fn on_despawned(&mut self, npc: u16) -> Vec<u16> {
        let Some(i) = self.entries.iter().position(|(h, _)| *h == npc) else {
            return Vec::new();
        };
        match self.entries[i].1 {
            SummonKind::Normal => {
                self.entries.remove(i);
                Vec::new()
            }
            kind => self.remove_first(kind).into_iter().collect(),
        }
    }
}
