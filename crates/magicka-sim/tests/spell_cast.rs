use magicka_sim::{
    damage::CastType as C,
    spell_cast::*,
    spells::{Elements as E, Spell},
};
#[derive(Debug, PartialEq)]
enum Op {
    Notice(CastNotice),
    Acquire(EffectKind),
    Set(u32),
    Cast(C, bool),
}
#[derive(Default)]
struct Host {
    ops: Vec<Op>,
    current: Option<u32>,
}
impl SpellCastHost for Host {
    type Handle = u32;
    fn notice(&mut self, n: CastNotice) {
        self.ops.push(Op::Notice(n));
    }
    fn acquire(&mut self, k: EffectKind) -> u32 {
        self.ops.push(Op::Acquire(k));
        42
    }
    fn set_current(&mut self, h: u32) {
        self.current = Some(h);
        self.ops.push(Op::Set(h));
    }
    fn cast(&mut self, h: u32, _: Spell, c: C, f: bool) {
        assert_eq!(self.current, Some(h));
        self.ops.push(Op::Cast(c, f));
    }
}
#[test]
fn all_effect_families_assign_owner_before_casting() {
    for (e, k) in [
        (E::NONE, EffectKind::Push),
        (E::FIRE, EffectKind::Spray),
        (E::EARTH, EffectKind::Projectile),
        (E::SHIELD, EffectKind::Shield),
        (E::ARCANE, EffectKind::RailGun),
        (E::LIGHTNING, EffectKind::Lightning),
    ] {
        for c in [C::Force, C::Area, C::SelfCast, C::Weapon] {
            let mut h = Host::default();
            Spell::default_for(e).cast_with(c, true, CastOwner::default(), &mut h);
            assert_eq!(h.ops, [Op::Acquire(k), Op::Set(42), Op::Cast(c, true)]);
        }
    }
}
#[test]
fn none_returns_before_notices_magick_mode_can_acquire_without_casting() {
    let mut h = Host::default();
    let local = CastOwner {
        avatar: true,
        remote: false,
    };
    Spell::default_for(E::ICE).cast_with(C::Blast, false, local, &mut h);
    assert!(h.ops.is_empty());
    Spell::default_for(E::ICE).cast_with(C::Magick, false, local, &mut h);
    assert_eq!(
        h.ops,
        [
            Op::Notice(CastNotice::VanillaIce),
            Op::Acquire(EffectKind::Projectile)
        ]
    );
}
#[test]
fn achievement_thresholds_and_remote_beam_tip_are_independent() {
    let mut h = Host::default();
    Spell::default_for(E::EARTH | E::WATER | E::COLD | E::FIRE | E::STEAM).cast_with(
        C::Area,
        false,
        CastOwner {
            avatar: true,
            remote: false,
        },
        &mut h,
    );
    assert_eq!(
        &h.ops[..3],
        &[
            Op::Notice(CastNotice::LetOffSomeSteam),
            Op::Notice(CastNotice::StateAlchemist),
            Op::Notice(CastNotice::WizardKing)
        ]
    );
    h.ops.clear();
    Spell::default_for(E::ARCANE).cast_with(
        C::Force,
        false,
        CastOwner {
            avatar: true,
            remote: true,
        },
        &mut h,
    );
    assert_eq!(h.ops[0], Op::Notice(CastNotice::BeamsTip));
}
#[test]
fn shared_initialization_and_release_minimum() {
    let mut s = EffectLifecycle::begin(C::Force, true, 12.0).unwrap();
    assert_eq!((s.min_ttl, s.max_ttl, s.time_stamp), (0.5, 1.5, 12.0));
    let owner = EffectOwner {
        avatar: true,
        button_none: true,
        ..Default::default()
    };
    assert_eq!(
        s.update(0.25, EffectKind::Spray, owner, |_| panic!()),
        (true, 0.75)
    );
    let mut calls = 0;
    assert_eq!(
        s.update(0.25, EffectKind::Spray, owner, |_| calls += 1),
        (false, 0.75)
    );
    assert_eq!(calls, 1);
    assert!(s.active);
    let s = EffectLifecycle::begin(C::SelfCast, false, 0.0).unwrap();
    assert_eq!((s.min_ttl, s.max_ttl), (0.2, 0.2));
}
#[test]
fn projectile_ignores_release_and_selected_none_but_not_death() {
    let mut s = EffectLifecycle::begin(C::Force, false, 0.0).unwrap();
    let mut owner = EffectOwner {
        avatar: true,
        button_none: true,
        cast_none: true,
        ..Default::default()
    };
    assert!(s.update(2.0, EffectKind::Projectile, owner, |_| panic!()).0);
    assert!(s.max_ttl < 0.0);
    owner.dead = true;
    assert!(
        !s.update(0.0, EffectKind::Projectile, owner, |s| s.active = false)
            .0
    );
    assert!(!s.active);
}
#[test]
fn selected_none_ends_before_minimum_and_stop_clears_active_after_callback() {
    let mut s = EffectLifecycle::begin(C::Area, false, 0.0).unwrap();
    assert!(
        !s.update(
            0.0,
            EffectKind::Shield,
            EffectOwner {
                avatar: true,
                cast_none: true,
                ..Default::default()
            },
            |_| {}
        )
        .0
    );
    s.stop(|s| assert!(s.active));
    assert!(!s.active);
    assert!(
        !s.update(0.0, EffectKind::Projectile, EffectOwner::default(), |_| {})
            .0
    );
}
