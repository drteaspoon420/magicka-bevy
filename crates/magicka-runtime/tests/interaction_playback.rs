use magicka_runtime::{dialog_definition::*, interaction_playback::*, message_playback::*};
#[derive(Default)]
struct Host {
    index: usize,
    events: Vec<(usize, &'static str)>,
    locks: Vec<i32>,
    unlocks: Vec<i32>,
    fail_lock: bool,
    fail_unlock: bool,
}
impl MessageHost for Host {
    type Error = &'static str;
    fn resolve_owner(&mut self, _: i32) -> Result<Option<MessageOwner>, Self::Error> {
        Ok(None)
    }
    fn cue_stopping(&self) -> Option<bool> {
        None
    }
    fn effect(&mut self, _: &MessageDefinition, effect: MessageEffect) -> Result<(), Self::Error> {
        self.events.push((
            self.index,
            match effect {
                MessageEffect::Show { .. } => "show",
                MessageEffect::Hide => "hide",
                MessageEffect::PlayCue { .. } => "play",
                _ => "other",
            },
        ));
        Ok(())
    }
}
impl InteractionHost for Host {
    type Error = MessagePlaybackError<&'static str>;
    fn advance_message(
        &mut self,
        index: usize,
        definition: &MessageDefinition,
        playback: &mut MessagePlayback,
        placement: MessagePlacement,
    ) -> Result<bool, Self::Error> {
        self.index = index;
        playback.advance(definition, placement, self)
    }
    fn lock_input(&mut self, controller: i32) -> Result<(), Self::Error> {
        self.locks.push(controller);
        if self.fail_lock {
            Err(MessagePlaybackError::Host("lock failed"))
        } else {
            Ok(())
        }
    }
    fn unlock_input(&mut self, controller: i32) -> Result<(), Self::Error> {
        self.unlocks.push(controller);
        if self.fail_unlock {
            Err(MessagePlaybackError::Host("unlock failed"))
        } else {
            Ok(())
        }
    }
    fn end_message(&mut self, index: usize, _: &MessageDefinition) -> Result<(), Self::Error> {
        self.events.push((index, "end"));
        Ok(())
    }
}
fn definition() -> InteractionDefinition {
    DialogDefinition::read_collection(&[br#"<Dialogs><dialog repeatFrom="-1"><interact count="1"><message><text>#one</text></message><message/><message><text>#three</text></message></interact></dialog></Dialogs>"#]).unwrap().remove(0).interactions.remove(0)
}
#[test]
fn decoded_interaction_sequences_real_messages_and_ignores_next_message_completion() {
    let definition = definition();
    let mut playback = InteractionPlayback::new(&definition);
    let mut host = Host::default();
    assert!(!playback
        .begin(
            &definition,
            InteractionLocation::Screen([10.0, 20.0]),
            Some(17),
            &mut host
        )
        .unwrap());
    assert_eq!(host.events, [(0, "show")]);
    assert!(!playback.advance(&definition, Some(18), &mut host).unwrap());
    assert_eq!(playback.current, 1);
    assert_eq!(playback.completed, 0);
    assert_eq!(host.events, [(0, "show"), (0, "hide"), (1, "hide")]);
    assert!(!playback.advance(&definition, None, &mut host).unwrap());
    assert_eq!(playback.current, 2);
    assert_eq!(host.events.last(), Some(&(2, "show")));
    assert!(playback.advance(&definition, None, &mut host).unwrap());
    assert_eq!(
        (playback.current, playback.completed, playback.controller),
        (0, 1, None)
    );
    assert_eq!(host.locks, [17]);
    assert_eq!(host.unlocks, [17]);
    assert_eq!(playback.repetition(&definition).completed, 1);
    let events = host.events.len();
    assert!(playback
        .begin(
            &definition,
            InteractionLocation::World([1.0, 2.0, 3.0]),
            Some(99),
            &mut host
        )
        .unwrap());
    assert_eq!(playback.world_position, [1.0, 2.0, 3.0]);
    assert!(!playback.screen_position);
    assert_eq!(host.events.len(), events);
    assert_eq!(host.locks, [17]);
}
#[test]
fn end_and_reset_preserve_controller_and_placement() {
    let definition = definition();
    let mut playback = InteractionPlayback::new(&definition);
    let mut host = Host::default();
    playback
        .begin(
            &definition,
            InteractionLocation::World([4.0, 5.0, 6.0]),
            Some(17),
            &mut host,
        )
        .unwrap();
    playback.last_owner = Some(42);
    playback.completed = 10;
    playback.end(&definition, &mut host).unwrap();
    playback.end(&definition, &mut host).unwrap();
    assert_eq!(host.unlocks, [17, 17]);
    assert_eq!(playback.controller, Some(17));
    playback.reset();
    assert_eq!((playback.current, playback.completed), (0, 0));
    assert_eq!(playback.controller, Some(17));
    assert_eq!(playback.last_owner, Some(42));
    assert_eq!(playback.world_position, [4.0, 5.0, 6.0]);
    assert_eq!(playback.messages[0].current, 0);
    playback
        .begin(
            &definition,
            InteractionLocation::Entity(None),
            None,
            &mut host,
        )
        .unwrap();
    assert_eq!(playback.last_owner, None);
    assert!(!playback.screen_position);
    assert_eq!(playback.world_position, [4.0, 5.0, 6.0]);
}
#[test]
fn lock_and_unlock_failures_preserve_original_mutation_points() {
    let mut definition = definition();
    definition.messages.truncate(1);
    let mut playback = InteractionPlayback::new(&definition);
    let mut host = Host {
        fail_lock: true,
        ..Default::default()
    };
    assert_eq!(
        playback.begin(
            &definition,
            InteractionLocation::Screen([0.0; 2]),
            Some(17),
            &mut host
        ),
        Err(InteractionError::Host(MessagePlaybackError::Host(
            "lock failed"
        )))
    );
    assert_eq!(playback.controller, Some(17));
    assert!(host.events.is_empty());
    host.fail_lock = false;
    playback.advance(&definition, Some(18), &mut host).unwrap();
    host.fail_unlock = true;
    assert_eq!(
        playback.advance(&definition, None, &mut host),
        Err(InteractionError::Host(MessagePlaybackError::Host(
            "unlock failed"
        )))
    );
    assert_eq!(
        (playback.current, playback.completed, playback.controller),
        (0, 1, Some(17))
    );
    assert!(playback.advance(&definition, None, &mut host).unwrap());
    assert_eq!(host.unlocks, [17]);
    assert_eq!(host.locks, [17]);
}
#[test]
fn infinite_repetition_keeps_advancing_and_bad_cursor_fails_after_acquiring_input() {
    let mut definition = definition();
    definition.messages.truncate(1);
    definition.repeat = 0;
    let mut playback = InteractionPlayback::new(&definition);
    let mut host = Host::default();
    for completed in 1..=2 {
        assert!(!playback
            .begin(
                &definition,
                InteractionLocation::Screen([0.0; 2]),
                Some(17),
                &mut host
            )
            .unwrap());
        assert!(playback.advance(&definition, None, &mut host).unwrap());
        assert_eq!(playback.completed, completed);
    }
    playback.current = -1;
    assert_eq!(
        playback.advance(&definition, Some(19), &mut host),
        Err(InteractionError::MissingMessage(-1))
    );
    assert_eq!(playback.controller, Some(19));
}

#[test]
fn owned_dialog_pop_observes_live_repetitions_and_resets_all_on_wrap() {
    let definition = DialogDefinition::read_collection(&[br#"<Dialogs><dialog repeatFrom="0"><interact><message><text>#one</text></message></interact><interact><message><text>#two</text></message></interact></dialog></Dialogs>"#]).unwrap().remove(0);
    let mut dialog = DialogPlayback::new(definition);
    let mut host = Host::default();
    let selected = dialog.pop().unwrap().unwrap();
    assert_eq!(selected.interaction, 0);
    dialog
        .begin(
            0,
            InteractionLocation::Screen([0.0; 2]),
            Some(17),
            &mut host,
        )
        .unwrap();
    assert!(dialog.advance(0, None, &mut host).unwrap());
    assert_eq!(dialog.interaction(0).unwrap().completed, 1);
    assert_eq!(dialog.cursor().interactions[0].completed, 1);
    dialog.reset();
    assert_eq!(dialog.cursor().next, 0);
    assert_eq!(dialog.cursor().interacted, 1);
    assert_eq!(dialog.interaction(0).unwrap().completed, 1);
    dialog.pop().unwrap();
    let selected = dialog.pop().unwrap().unwrap();
    assert_eq!(selected.interaction, 1);
    assert!(selected.reset_all);
    assert_eq!(dialog.cursor().next, 0);
    assert_eq!(dialog.cursor().interacted, 2);
    assert_eq!(dialog.interaction(0).unwrap().completed, 0);
    assert!(!dialog
        .begin(
            0,
            InteractionLocation::Screen([0.0; 2]),
            Some(19),
            &mut host
        )
        .unwrap());
    assert_eq!(host.locks, [17, 19]);
}
