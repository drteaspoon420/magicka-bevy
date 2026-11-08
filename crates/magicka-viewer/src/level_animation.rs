//! Opt-in preview driven by original level channel bits and recovered playback.
use bevy::{gltf::GltfExtras, prelude::*};
use magicka_content::level::Keyframe;
use magicka_runtime::animated_pose::{local_pose, PartPlayback};

pub struct LevelAnimationPlugin;
impl Plugin for LevelAnimationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (attach, advance).chain());
    }
}
#[derive(Component)]
struct NativeLevelAnimation {
    keys: Vec<Keyframe>,
    playback: PartPlayback,
}

fn decode(text: &str) -> Result<Option<(f32, Vec<Keyframe>)>, String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let Some(data) = value.get("magicka_level_animation") else {
        return Ok(None);
    };
    if data["version"].as_u64() != Some(1) {
        return Err("unsupported level animation metadata version".into());
    }
    let bits = |v: &serde_json::Value| -> Result<f32, String> {
        let n = v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("invalid float bits")?;
        let value = f32::from_bits(n);
        if !value.is_finite() {
            return Err("nonfinite level animation component".into());
        }
        Ok(value)
    };
    let duration = bits(&data["duration_bits"])?;
    let source = data["keys"].as_array().ok_or("missing level keyframes")?;
    if source.is_empty() {
        return Err("empty level animation channel".into());
    }
    let mut keys = Vec::with_capacity(source.len());
    for key in source {
        let key = key
            .as_array()
            .filter(|k| k.len() == 11)
            .ok_or("invalid level keyframe")?;
        let values = key.iter().map(bits).collect::<Result<Vec<_>, _>>()?;
        keys.push(Keyframe {
            time: values[0],
            translation: values[1..4].try_into().unwrap(),
            orientation: values[4..8].try_into().unwrap(),
            scale: values[8..11].try_into().unwrap(),
        });
    }
    Ok(Some((duration, keys)))
}
fn attach(
    mut commands: Commands,
    args: Res<crate::Args>,
    nodes: Query<(Entity, &GltfExtras), Added<GltfExtras>>,
) {
    if !args.native_level_animation {
        return;
    }
    for (entity, extras) in &nodes {
        match decode(&extras.value) {
            Ok(Some((duration, keys))) => {
                let mut playback = PartPlayback::new(duration);
                // Preview policy: play every part. Campaign commands will later
                // decide what moves; this is not original level startup behavior.
                playback.play(duration, 0.0, -1.0, 1.0, true, false);
                if args.render.is_some() {
                    playback.time = args.time;
                    playback.stop();
                }
                commands
                    .entity(entity)
                    .insert(NativeLevelAnimation { keys, playback });
            }
            Ok(None) => {}
            Err(error) => error!("level animation metadata: {error}"),
        }
    }
}
fn advance(
    args: Res<crate::Args>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut parts: Query<(&mut NativeLevelAnimation, &mut Transform)>,
) {
    if !args.native_level_animation {
        return;
    }
    for (mut part, mut transform) in &mut parts {
        if args.render.is_none() && keys.just_pressed(KeyCode::Space) {
            part.playback.playing = !part.playback.playing;
        }
        part.playback.advance(time.delta_secs());
        match local_pose(&part.keys, part.playback.time) {
            Ok(matrix) => *transform = Transform::from_matrix(Mat4::from_cols_array(&matrix)),
            Err(error) => error!("level pose sampling: {error:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_preserves_duplicate_times_and_quaternion_bits() {
        let key = [0.0f32, 1.0, 2.0, 3.0, 0.0, 0.0, 0.6, 0.8, 1.0, 1.0, 1.0].map(f32::to_bits);
        let json = serde_json::json!({"magicka_level_animation":{"version":1,"duration_bits":1.0f32.to_bits(),"keys":[key,key]}});
        let (duration, keys) = decode(&json.to_string()).unwrap().unwrap();
        assert_eq!(duration, 1.0);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[1].orientation[2].to_bits(), 0.6f32.to_bits());
        let transform =
            Transform::from_matrix(Mat4::from_cols_array(&local_pose(&keys, 0.0).unwrap()));
        assert_eq!(transform.translation, Vec3::new(1.0, 2.0, 3.0));
    }
    #[test]
    fn unrelated_extras_are_ignored_and_invalid_channels_rejected() {
        assert!(decode("{}").unwrap().is_none());
        assert!(
            decode(r#"{"magicka_level_animation":{"version":1,"duration_bits":0,"keys":[]}}"#)
                .is_err()
        );
        assert!(decode(r#"{"magicka_level_animation":{"version":2}}"#).is_err());
    }

    #[test]
    fn ecs_attachment_and_fixed_time_preview_use_recovered_sampler() {
        let a = [0.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0].map(f32::to_bits);
        let b = [1.0f32, 4.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0].map(f32::to_bits);
        let extras = serde_json::json!({"magicka_level_animation": {
            "version": 1, "duration_bits": 1.0f32.to_bits(), "keys": [a,b]
        }});
        let mut app = App::new();
        app.insert_resource(crate::Args {
            native_level_animation: true,
            root: String::new(),
            scene: String::new(),
            render: Some("unused.png".into()),
            side: false,
            origin: Vec2::ZERO,
            scale: 1.0,
            size: UVec2::ONE,
            anim: None,
            time: 0.25,
            opaque: false,
            snapshot: None,
        })
        .insert_resource(Time::<()>::default())
        .insert_resource(ButtonInput::<KeyCode>::default())
        .add_plugins(LevelAnimationPlugin);
        let entity = app
            .world_mut()
            .spawn((
                GltfExtras {
                    value: extras.to_string(),
                },
                Transform::default(),
            ))
            .id();
        app.update();
        let transform = app.world().get::<Transform>(entity).unwrap();
        assert_eq!(transform.translation, Vec3::X);
        // Normalized lerp gives 2*atan(1/3); glTF slerp would give PI/4.
        assert!((transform.rotation.to_scaled_axis().z - 2.0 * (1.0f32 / 3.0).atan()).abs() < 1e-5);
        assert!(
            !app.world()
                .get::<NativeLevelAnimation>(entity)
                .unwrap()
                .playback
                .playing
        );
    }
}
