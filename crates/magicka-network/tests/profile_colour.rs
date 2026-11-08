use magicka_network::session::{valid_player_colour, PlayerProfile};

#[test]
fn robe_tints_preserve_palette_highlights_and_reject_invalid_components() {
    // Defines.PLAYERCOLORS[7], linear multipliers above one.
    let mut profile = PlayerProfile {
        colour: [363.0 / 255.0, 298.0 / 255.0, 0.0],
        ..Default::default()
    };
    assert!(profile.valid());
    assert!(valid_player_colour(&profile.colour));
    for bad in [f32::NAN, f32::INFINITY, -0.01] {
        profile.colour[0] = bad;
        assert!(!profile.valid());
        assert!(!valid_player_colour(&profile.colour));
    }
    assert!(!valid_player_colour(&[1.0, 1.0]));
    assert!(!valid_player_colour(&[1.0; 4]));
}
