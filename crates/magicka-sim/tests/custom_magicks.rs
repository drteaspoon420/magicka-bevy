use magicka_sim::{
    magicks::{allowed, animation},
    spells::{
        try_add_to_queue, Elements, MagickTree, MagickType, Spell, ALL_MAGICKS_MASK, MAGICK_COMBOS,
        SPELL_QUEUE_CAP,
    },
};

#[test]
fn custom_combinations_survive_real_queue_rules_and_resolve_uniquely() {
    let tree = MagickTree::default();
    let mut count = 0;
    for &(magick, combo) in MAGICK_COMBOS.iter().filter(|(m, _)| *m as u8 >= 36) {
        count += 1;
        assert!(combo.len() <= SPELL_QUEUE_CAP);
        assert!(combo.iter().filter(|&&e| e == Elements::SHIELD).count() <= 1);
        assert_eq!(MAGICK_COMBOS.iter().filter(|(_, c)| *c == combo).count(), 1);
        let mut queue = Vec::new();
        let mut events = Vec::new();
        for &element in combo {
            // Only basic keyboard elements, no fabricated merged element.
            assert!(Elements::BY_INDEX[..8].contains(&element), "{magick:?}");
            try_add_to_queue(
                &mut queue,
                SPELL_QUEUE_CAP,
                Spell::default_for(element),
                &mut events,
            );
        }
        assert_eq!(
            queue.iter().map(|s| s.element).collect::<Vec<_>>(),
            combo,
            "{magick:?} cancels or merges"
        );
        assert_eq!(tree.exact_magick(&queue), Some(magick));
        assert!(!tree.combine_magick(&mut queue, |_| false));
        assert!(tree.combine_magick(&mut queue, |m| allowed(ALL_MAGICKS_MASK, m)));
        assert_eq!(queue, [Spell::magick(magick)]);
        assert_eq!(MagickType::try_from(magick as u8), Ok(magick));
        assert!(animation(magick).is_some());
    }
    assert_eq!(count, 15);
    assert!(MagickType::try_from(35).is_err());
    assert_eq!(ALL_MAGICKS_MASK & (1 << 35), 0);
}
