use magicka_sim::summons::{SummonKind as K, SummonList};

/// Character.OnSpawnedSummon makes room before appending, whatever the new
/// summon's kind (native quirk): any summon replaces the existing undead,
/// any summon trims the flamers below four first, and a seventeenth summon
/// overkills the oldest one.
#[test]
fn any_summon_trims_undead_and_flamers_and_all_cap_at_sixteen() {
    let mut l = SummonList::default();
    assert!(l.on_spawned(1, K::Undead).is_empty());
    assert_eq!(
        l.on_spawned(2, K::Undead),
        [1],
        "a new undead replaces the old"
    );
    assert_eq!(l.on_spawned(10, K::Flamer), [2], "so does any other summon");
    for h in 11..14 {
        assert!(l.on_spawned(h, K::Flamer).is_empty());
    }
    assert_eq!(l.on_spawned(14, K::Flamer), [10], "fifth flamer");
    assert_eq!(
        l.on_spawned(20, K::Normal),
        [11],
        "four flamers: any summon trims one"
    );
    for h in 21..33 {
        assert!(l.on_spawned(h, K::Normal).is_empty(), "{h}");
    }
    // 3 flamers + 13 normal fill the 16 slots: the next overkills the
    // oldest (flamer 12).
    assert_eq!(l.on_spawned(33, K::Normal), [12]);
    assert_eq!(l.summons().count(), 16);
    // A despawned normal summon just leaves; a despawned flamer removes the
    // first flamer (with OverKill) even when that is another one.
    assert!(l.on_despawned(25).is_empty());
    assert_eq!(l.on_despawned(14), [13]);
    assert!(l.on_despawned(999).is_empty());
}
