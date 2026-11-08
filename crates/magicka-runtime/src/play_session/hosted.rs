//! Rewrite extension: a session for an external host game that supplies its
//! own collision and keeps its own scripts (the Postal 2 adapter). No level
//! file, scene scripts or dialogs are loaded; spells, effects and audio
//! requests run exactly as in a campaign scene.
use super::*;
use magicka_content::level::LevelModel;

impl PlaySession {
    /// One avatar of `character` at `spawn` (feet dropped onto collision
    /// within 5 units, as for a checkpoint), facing `forward` on the ground plane.
    pub fn hosted(
        content: &Path,
        model: LevelModel,
        character: &str,
        spawn: [f32; 3],
        forward: [f32; 3],
        kill_plane: f32,
    ) -> Result<Self, SessionError> {
        let (body, make) = Self::prepare_player(content, character, 0)?;
        let start = crate::play_content::StartScene {
            start: crate::play_content::LevelStart {
                scene: "hosted".into(),
                scene_hash: custom_hash("hosted"),
                area: None,
                locations: [0; 4],
                spawn_players: true,
                spawn_fairy: true,
                no_items: false,
                scenes: Vec::new(),
            },
            scene: crate::play_content::SceneFile {
                name: "hosted".into(),
                path: PathBuf::new(),
                model: PathBuf::new(),
                kill_plane,
                post: Default::default(),
                indoor: false,
            },
            model_path: PathBuf::new(),
            model,
        };
        let f = [forward[0], 0.0, forward[2]];
        let len = (f[0] * f[0] + f[2] * f[2]).sqrt();
        let f = if len > 1e-4 {
            [f[0] / len, 0.0, f[2] / len]
        } else {
            [0.0, 0.0, -1.0]
        };
        // XNA World matrix rows: right, up, backward, translation.
        let right = [-f[2], 0.0, f[0]];
        let m = [
            right[0], right[1], right[2], 0.0, 0.0, 1.0, 0.0, 0.0, -f[0], -f[1], -f[2], 0.0,
            spawn[0], spawn[1], spawn[2], 1.0,
        ];
        Self::build(
            content,
            "",
            start,
            [0; 4],
            body,
            make,
            LevelVariables::default(),
            0.0,
            Persistent {
                spawn_override: Some([m; 4]),
                spawn_players: true,
                ..Default::default()
            },
            character,
        )
    }
}
