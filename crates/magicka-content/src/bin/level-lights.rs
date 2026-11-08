//! Print the static light list of LevelModel XNBs, one tab-separated row per light.
//!
//!     level-lights [--effects] <level.xnb>...
//!
//! With `--effects`, prints the static and animated-part effect placements
//! instead: file, part ("-" for static), id, effect, range, position, forward.
//!
//! Columns: file, name, type (0 point, 1 directional, 2 spot), variation type,
//! range, attenuation, cutoff, sharpness, diffuse rgb, ambient rgb, specular,
//! variation speed/amount, shadow map size, cast shadows, position, direction.

use magicka_content::{parse_primary, Content};
use magicka_xnb::xnb;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut status = ExitCode::SUCCESS;
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let effects = args.first().is_some_and(|a| a == "--effects");
    if effects {
        args.remove(0);
    }
    for file in args {
        let parsed = std::fs::read(&file)
            .map_err(|e| e.to_string())
            .and_then(|bytes| xnb::parse(&bytes).map_err(|e| e.to_string()))
            .and_then(|x| match parse_primary(&x) {
                Some(Ok(p)) => match p.content {
                    Content::Level(level) => Ok(level),
                    _ => Err("not a LevelModel".into()),
                },
                Some(Err(e)) => Err(e.to_string()),
                None => Err("no primary reader".into()),
            });
        let level = match parsed {
            Ok(level) => level,
            Err(e) => {
                eprintln!("{file}: {e}");
                status = ExitCode::FAILURE;
                continue;
            }
        };
        let v = |a: [f32; 3]| format!("{} {} {}", a[0], a[1], a[2]);
        if effects {
            let mut stack: Vec<(&str, &[magicka_content::level::EffectPlacement])> =
                vec![("-", &level.effects)];
            let mut parts: Vec<&magicka_content::level::AnimatedLevelPart> =
                level.animated_parts.iter().collect();
            while let Some(part) = parts.pop() {
                stack.push((&part.name, &part.effects));
                parts.extend(part.children.iter());
            }
            for (part, list) in stack {
                for e in list {
                    println!(
                        "{file}\t{part}\t{}\t{}\t{}\t{}\t{}",
                        e.id,
                        e.effect,
                        e.range,
                        v(e.position),
                        v(e.forward)
                    );
                }
            }
            continue;
        }
        for l in &level.lights {
            println!(
                "{file}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                l.name,
                l.light_type,
                l.variation_type,
                l.range,
                l.use_attenuation,
                l.cutoff_angle,
                l.sharpness,
                v(l.diffuse),
                v(l.ambient),
                l.specular,
                l.variation_speed,
                l.variation_amount,
                l.shadow_map_size,
                l.cast_shadows,
                v(l.position),
                v(l.direction),
            );
        }
    }
    status
}
