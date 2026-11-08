use magicka_sim::amalgameddon::*;

/// Rising by 0.015 per Update until the first unit is above Y 5, a 0.2 s
/// push apart (0.25 per Update), then smashing together (0.6 per Update)
/// until the capsules touch: the second unit is absorbed with its HP.
#[test]
fn rise_push_apart_smash_and_merge() {
    let mut a = Amalgameddon::new(1, 10, 11);
    let (mut p, mut q) = ([0.0, 4.98, 0.0], [2.0, 4.98, 0.0]);
    let mut steps = vec![];
    for _ in 0..200 {
        let Some(s) = a.step(1.0 / 60.0, (p, 0.5), (q, 0.5), 300.0) else {
            break;
        };
        steps.push(s);
        match s {
            Step::Move { first, second, .. } => {
                p = first;
                q = second;
            }
            Step::Merge { .. } => break,
        }
    }
    assert!(matches!(steps[0], Step::Move { stun: true, .. }));
    assert_eq!(a.phase, Phase::SmashTogether);
    assert_eq!(
        *steps.last().unwrap(),
        Step::Merge {
            happy: 10,
            sorry: 11,
            life: 300.0
        }
    );
    assert_eq!(a.ttl, 0.0);
    // Two rising steps (4.98 -> 4.995 -> 5.01), then the 0.2 s push.
    assert!(steps.len() > 12 && steps.len() < 30, "{}", steps.len());
}
