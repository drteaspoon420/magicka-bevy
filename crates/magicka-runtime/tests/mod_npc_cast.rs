//! `Spells.CastElements` on an NPC caster goes through the native NPC
//! spellcasting path (its next attack casts the requested elements).
use magicka_runtime::{mod_runtime::ModRuntime, play_session::PlaySession};
use magicka_sim::spells::Elements;
use std::path::PathBuf;

#[test]
fn scripted_npc_cast_uses_the_native_spellcasting_queue() {
    let dir = std::env::temp_dir().join(format!("magicka-npc-cast-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let write = |p: &str, t: &str| {
        let p = dir.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    };
    write(
        "mods/caster/mod.kv",
        r#""Mod" { "Id" "caster" "Version" "1.0.0" "ApiVersion" "1" "ServerScripts" "1" }"#,
    );
    write(
        "mods/caster/scripts/vscripts/caster/init.lua",
        r#"
        function OnGameStart()
            Timers.Every(0.5, function()
                for _, e in ipairs(Entities.GetAll()) do
                    if e:GetPlayer() == nil and e:IsAlive() then
                        Spells.CastElements(e, { "cold", "cold" }, { delivery = "projectile" })
                        NetTables.Set("caster", "ordered", true)
                        return false
                    end
                end
            end)
        end
        "#,
    );
    let _root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut s = PlaySession::load(
        &magicka_content::retail_content_dir(),
        "Havindr.lvl",
        "Wizard",
    )
    .unwrap();
    s.disable_scripts();
    let mut mods = ModRuntime::load(None, &dir.join("mods"), Some(&["caster".into()]), 1).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    mods.attach(&mut s);
    let at = s.world.physics.bodies[s.avatar().body].position;
    let shaman = s
        .spawn_npc_at("Goblin_shaman", [at[0] + 6.0, at[1], at[2]])
        .unwrap();
    let mut cold = false;
    for _ in 0..600 {
        mods.step(&mut s, 1.0 / 60.0);
        s.step(1.0 / 60.0);
        let Some(npc) = s.world.npcs.get(&shaman) else {
            break;
        };
        cold |= npc
            .combat
            .spellcasting
            .queue
            .iter()
            .any(|sp| sp.element.has(Elements::COLD));
        if cold {
            break;
        }
    }
    assert_eq!(mods.scripts.net_tables()["caster"]["ordered"], true);
    assert!(
        !s.world.log.iter().any(|l| l.contains("mod caster")),
        "{:?}",
        s.world.log.iter().rev().take(5).collect::<Vec<_>>()
    );
    assert!(cold, "the shaman typed the scripted Cold spell");
}
