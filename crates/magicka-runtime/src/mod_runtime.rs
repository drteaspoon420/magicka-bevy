//! Server-side mod runtime (rewrite extension, plan workstreams D–F): the
//! adapter between the Lua host and the authoritative [`PlaySession`].
//!
//! Each simulation tick, before [`PlaySession::step`]:
//! 1. entity generations are refreshed and a [`WorldSnapshot`] is published;
//! 2. lifecycle callbacks fire (match/game/level start, player join);
//! 3. intrinsic item modifiers follow held items;
//! 4. timers, modifier thinks and expirations run;
//! 5. queued script commands are validated and applied through the same
//!    native paths players and level scripts use;
//! 6. modifier stat properties feed native per-frame values.
//!
//! Clients never run this: they receive the resulting world state.
use crate::play_session::PlaySession;
use magicka_definitions::{DefinitionId, Definitions, Handler};
use magicka_mods::{ContentManifest, ModSet};
use magicka_network::control::Controls;
use magicka_script::{
    convert::entity_json, Command, EntityInfo, EntityRef, Issued, Lifecycle, Limits, PlayerInfo,
    ScriptHost, Verdict, WorldSnapshot,
};
use serde_json::{json, Value as Json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::Path,
    sync::Arc,
};

/// A presentation event for clients to simulate locally (never gameplay).
#[derive(Clone, Debug, PartialEq)]
pub struct PresentationEvent {
    pub sound: bool,
    pub name: String,
    pub position: Option<[f32; 3]>,
    pub owner: Option<u16>,
}

/// Identity of whatever currently holds a handle; a change starts a new
/// generation so stale script handles stop resolving.
#[derive(Clone, PartialEq, Eq)]
struct Identity {
    kind: u8,
    template: String,
    slot: Option<usize>,
}

pub struct ModRuntime {
    pub set: Arc<ModSet>,
    pub definitions: Arc<Definitions>,
    pub scripts: ScriptHost,
    pub content: ContentManifest,
    generations: BTreeMap<u16, (u32, Identity)>,
    /// Intrinsic item modifiers applied: (holder, modifier).
    item_modifiers: BTreeSet<(EntityRef, DefinitionId)>,
    /// Scripted input sequences per player slot (CastElements); `true`
    /// once started (a sequence waits until the wizard is free).
    casts: BTreeMap<usize, (bool, VecDeque<Controls>)>,
    /// Registry Magicks unlocked per slot beyond the native legacy mask.
    pub custom_unlocks: BTreeMap<usize, BTreeSet<DefinitionId>>,
    pub presentation: Vec<PresentationEvent>,
    pub messages: Vec<(Option<usize>, String)>,
    tick: u64,
    time: f64,
    started: bool,
    /// Saved mod state was restored from the session (campaign load).
    restored: bool,
    /// Lua-driven level state (actions, conditions, areas, waves).
    pub levels: crate::mod_levels::LevelBindings,
    /// Upgrade overlays by ID (per player; never global).
    pub upgrades: BTreeMap<String, magicka_script::Upgrade>,
    /// `MaxHealthBonus` currently added to each character (reverted exactly
    /// when the modifier ends).
    max_health_bonus: BTreeMap<u16, f32>,
    /// Exactly-once mod request results by (player, request ID).
    transactions: BTreeMap<(usize, u64), Json>,
    level: String,
    players: BTreeSet<usize>,
}

fn element_bits(name: &str) -> Option<magicka_sim::spells::Elements> {
    magicka_definitions::defs::element(name)
}

impl ModRuntime {
    /// Resolve mods, compile definitions and start the match VM.
    pub fn load(
        base: Option<&Path>,
        mods_root: &Path,
        enabled: Option<&[String]>,
        seed: u64,
    ) -> Result<Self, String> {
        let set = ModSet::load(base, mods_root, enabled).map_err(|e| e.to_string())?;
        Self::from_set(set, seed)
    }
    pub fn from_set(set: ModSet, seed: u64) -> Result<Self, String> {
        let definitions = Arc::new(Definitions::load(&set).map_err(|e| e.to_string())?);
        let mut content = set.content_manifest()?;
        // The rules hash also covers the compiled definition values.
        if let Some(hash) = &mut content.rules_hash {
            *hash =
                magicka_mods::sha256_hex(format!("{hash}{}", definitions.rules_hash()).as_bytes());
        }
        let set = Arc::new(set);
        let scripts = ScriptHost::new(set.clone(), definitions.clone(), Limits::default(), seed)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            set,
            definitions,
            scripts,
            content,
            generations: BTreeMap::new(),
            item_modifiers: BTreeSet::new(),
            casts: BTreeMap::new(),
            custom_unlocks: BTreeMap::new(),
            presentation: Vec::new(),
            messages: Vec::new(),
            tick: 0,
            time: 0.0,
            started: false,
            restored: false,
            levels: Default::default(),
            upgrades: BTreeMap::new(),
            max_health_bonus: BTreeMap::new(),
            transactions: BTreeMap::new(),
            level: String::new(),
            players: BTreeSet::new(),
        })
    }

    /// This tick's presentation events as frame records, for the host's
    /// own screen (ids are local; the network publishes its own).
    pub fn local_presentation(&self) -> Vec<magicka_network::session::PresentationEventState> {
        self.presentation
            .iter()
            .map(|e| magicka_network::session::PresentationEventState {
                id: 0,
                sound: e.sound,
                name: e.name.clone(),
                position: e.position,
                owner: e.owner,
                tick: self.tick,
            })
            .collect()
    }
    /// Install the definitions into a session (call once, before play).
    pub fn attach(&self, session: &mut PlaySession) {
        session.set_definitions(self.definitions.clone());
    }

    /// The current generation-checked reference for a handle.
    pub fn entity(&self, handle: u16) -> Option<EntityRef> {
        self.generations
            .get(&handle)
            .map(|(generation, _)| EntityRef {
                handle,
                generation: *generation,
            })
    }
    fn resolve(&self, e: EntityRef) -> Option<u16> {
        (self.entity(e.handle) == Some(e)).then_some(e.handle)
    }

    fn refresh(&mut self, session: &PlaySession) -> WorldSnapshot {
        let world = &session.world;
        let mut live: BTreeMap<u16, Identity> = BTreeMap::new();
        for (slot, p) in world.players.iter().enumerate() {
            if let Some(p) = p {
                live.insert(
                    p.handle,
                    Identity {
                        kind: 0,
                        template: p.character.definition.id.clone(),
                        slot: Some(slot),
                    },
                );
            }
        }
        for e in &world.entities {
            if e.added && !e.removed && world.npcs.contains_key(&e.handle) {
                live.insert(
                    e.handle,
                    Identity {
                        kind: 1,
                        template: e.template_name.clone().unwrap_or_default(),
                        slot: None,
                    },
                );
            }
        }
        for (handle, identity) in &live {
            let entry = self
                .generations
                .entry(*handle)
                .or_insert((0, identity.clone()));
            if entry.0 == 0 || entry.1 != *identity {
                entry.0 += 1;
                entry.1 = identity.clone();
            }
        }
        // A handle that vanished gets a new generation if it ever returns.
        for (handle, entry) in &mut self.generations {
            if !live.contains_key(handle) && entry.1.kind != 255 {
                entry.1.kind = 255;
            }
        }
        let info = |handle: u16,
                    body: usize,
                    character: &crate::character::RuntimeCharacter,
                    dead: bool,
                    player: Option<usize>| {
            let b = &world.physics.bodies[body];
            let entity = world.entities.iter().find(|e| e.handle == handle);
            EntityInfo {
                entity: self.entity(handle),
                name: live
                    .get(&handle)
                    .map(|i| i.template.clone())
                    .unwrap_or_default(),
                position: b.position,
                facing: b.direction(),
                health: character.combat.health.hit_points,
                max_health: character.combat.health.max_hit_points,
                alive: !dead,
                player,
                faction: entity.map_or(0, |e| e.faction),
                statuses: crate::scene_world::script_ops::STATUSES
                    .iter()
                    .filter(|(_, s)| character.combat.statuses.has(*s))
                    .map(|(n, _)| n.to_string())
                    .collect(),
                ethereal: character.ethereal,
            }
        };
        let mut entities = Vec::new();
        let mut players = Vec::new();
        for (slot, p) in world.players.iter().enumerate() {
            let Some(p) = p else { continue };
            entities.push(info(p.handle, p.body, &p.character, p.dead(), Some(slot)));
            let mut magicks: Vec<String> = self
                .definitions
                .magicks
                .iter()
                .filter(|m| {
                    m.legacy
                        .is_some_and(|l| session.unlocked_magicks[slot] & (1u64 << l) != 0)
                })
                .map(|m| m.common.id.to_string())
                .collect();
            magicks.extend(
                self.custom_unlocks
                    .get(&slot)
                    .into_iter()
                    .flatten()
                    .map(|m| m.to_string()),
            );
            players.push(PlayerInfo {
                slot,
                name: session.player_profiles[slot]
                    .as_ref()
                    .map_or_else(|| format!("Player {}", slot + 1), |p| p.0.clone()),
                profile_id: session.player_ids[slot].clone(),
                entity: self.entity(p.handle),
                magicks,
                items: p
                    .items
                    .iter()
                    .map(|(slot, item)| {
                        let base = item.base_definition.as_deref().unwrap_or(&item.definition);
                        magicka_script::ItemInfo {
                            slot: *slot,
                            name: base.name.clone(),
                            numbers: magicka_definitions::ITEM_FIELDS
                                .iter()
                                .filter_map(|s| {
                                    crate::item_definitions::number(base, s.key)
                                        .map(|v| (s.key.to_string(), v))
                                })
                                .collect(),
                        }
                    })
                    .collect(),
            });
        }
        for (handle, npc) in &world.npcs {
            if live.contains_key(handle) {
                entities.push(info(*handle, npc.body, &npc.character, npc.dead(), None));
            }
        }
        WorldSnapshot {
            tick: self.tick,
            time: self.time,
            level: session.level_file().to_string(),
            scene: session.scene_name().to_string(),
            players,
            entities,
            conditions: Default::default(),
            upgrades: self.upgrades.values().cloned().collect(),
            indoors: world.scene_indoors,
        }
    }

    /// One server tick of mod logic (call before `PlaySession::step`).
    pub fn step(&mut self, session: &mut PlaySession, dt: f32) {
        self.tick += 1;
        self.time += f64::from(dt);
        // KV patches land before upgrade bases are captured.
        session.apply_item_definitions();
        self.levels.sync_scene(session);
        let mut snapshot = self.refresh(session);
        snapshot.conditions = self.levels.conditions(session);
        self.scripts.set_snapshot(snapshot.clone());
        if !self.restored {
            self.restored = true;
            if let Some(state) = session.mod_state.clone() {
                self.load_state(&state);
            }
        }
        if !self.started {
            self.started = true;
            self.scripts.lifecycle(Lifecycle::MatchSetup, Json::Null);
            self.scripts.lifecycle(Lifecycle::GameStart, Json::Null);
        }
        if self.level != snapshot.level {
            if !self.level.is_empty() {
                self.scripts
                    .lifecycle(Lifecycle::LevelEnd, json!({ "level": self.level }));
            }
            self.level = snapshot.level.clone();
            self.scripts
                .lifecycle(Lifecycle::LevelStart, json!({ "level": self.level }));
            self.level_handler("OnLevelStart", &json!({ "level": self.level }));
        }
        for event in self.levels.observe(session, &snapshot) {
            use crate::mod_levels::LevelEvent as E;
            let (function, payload) = match event {
                E::TriggerEnter { area, entity } => (
                    "OnTriggerEnter",
                    json!({ "area": area, "entity": entity_json(entity) }),
                ),
                E::NpcKilled { entity, name } => (
                    "OnNpcKilled",
                    json!({ "entity": entity_json(entity), "name": name }),
                ),
                E::Checkpoint { generation } => {
                    ("OnCheckpoint", json!({ "generation": generation }))
                }
                E::WaveStart { wave } => ("OnWaveStart", json!({ "wave": wave })),
                E::WaveComplete { wave } => ("OnWaveComplete", json!({ "wave": wave })),
            };
            self.scripts.call_global(function, &payload);
            self.level_handler(function, &payload);
        }
        let present: BTreeSet<usize> = snapshot.players.iter().map(|p| p.slot).collect();
        for p in &snapshot.players {
            if !self.players.contains(&p.slot) {
                self.scripts.lifecycle(
                    Lifecycle::PlayerJoin,
                    json!({ "slot": p.slot, "name": p.name, "entity": p.entity.map(entity_json) }),
                );
            }
        }
        self.players = present;
        self.sync_item_modifiers(session);
        self.scripts.tick(self.time);
        self.scripts.update_modifiers(f64::from(dt));
        self.scripts.prune_modifiers();
        for issued in self.scripts.take_commands() {
            self.apply(session, issued);
        }
        // Commands issued by handlers run during application.
        for issued in self.scripts.take_commands() {
            self.apply(session, issued);
        }
        self.apply_properties(session, dt);
        self.apply_upgrades(session);
        self.drive_casts(session);
        // Native interactions since last step (OnInteract).
        for (id, handle) in std::mem::take(&mut session.interactions) {
            let payload = json!({
                "interactable": id,
                "entity": self.entity(handle).map(entity_json),
            });
            self.scripts.call_global("OnInteract", &payload);
            self.level_handler("OnInteract", &payload);
        }
        // Built-in Magicks a mod replaced, cast by anyone last step.
        for (handle, legacy) in std::mem::take(&mut session.world.scripted_magick_casts) {
            let Some(id) = self
                .definitions
                .magicks
                .iter()
                .find(|m| m.legacy == Some(legacy))
                .map(|m| m.common.id.clone())
            else {
                continue;
            };
            if let Err(e) = self.invoke_magick(session, handle, &id, None) {
                session.world.log.push(format!("magick {id}: {e}"));
            }
        }
        // Registered Magicks combined and cast by wizards last step.
        for (handle, index) in std::mem::take(&mut session.world.registered_casts) {
            let Some(id) = self
                .definitions
                .magicks
                .at(magicka_definitions::RegistryIndex(index))
                .map(|m| m.common.id.clone())
            else {
                continue;
            };
            if let Err(e) = self.invoke_magick(session, handle, &id, None) {
                session.world.log.push(format!("magick {id}: {e}"));
            }
        }
        self.sync_registered_unlocks(session);
        if self.tick.is_multiple_of(30) {
            session.mod_state = Some(self.save_state());
        }
        for e in self.scripts.take_errors() {
            session.world.log.push(format!("mod error: {e}"));
        }
        for line in self.scripts.take_log() {
            session.world.log.push(line);
        }
    }

    /// Resolve every player's upgrade overlays onto their held items (from
    /// each item's captured base, through the stat resolver).
    fn apply_upgrades(&mut self, session: &mut PlaySession) {
        let defs = self.definitions.clone();
        // Modifier CooldownPercent shortens every item its holder carries.
        let cooldown: BTreeMap<usize, f64> = session
            .world
            .players
            .iter()
            .enumerate()
            .filter_map(|(slot, p)| {
                let e = self.entity(p.as_ref()?.handle)?;
                let pct = self
                    .scripts
                    .modifier_properties(e)
                    .get("CooldownPercent")
                    .copied()?;
                Some((slot, pct))
            })
            .collect();
        for (slot, p) in session.world.players.iter_mut().enumerate() {
            let Some(p) = p else { continue };
            for (_, item) in &mut p.items {
                let name = item
                    .base_definition
                    .as_deref()
                    .unwrap_or(&item.definition)
                    .name
                    .to_ascii_lowercase();
                let def_id = defs.retail_item(&name).map(|d| d.common.id.to_string());
                let mut overlays: Vec<(&str, magicka_definitions::schema::Change, &str)> = self
                    .upgrades
                    .values()
                    .filter(|u| {
                        u.player == slot && (u.target == name || Some(&u.target) == def_id.as_ref())
                    })
                    .map(|u| (u.key.as_str(), u.change, u.owner.as_str()))
                    .collect();
                if let Some(pct) = cooldown.get(&slot).filter(|p| **p != 0.0) {
                    overlays.push((
                        "CooldownTime",
                        magicka_definitions::schema::Change::Percent(*pct),
                        "modifier",
                    ));
                }
                crate::item_definitions::apply_upgrades(item, &overlays);
            }
        }
    }

    /// A client's (or the local player's) request to a mod. Exactly once:
    /// a repeated (player, request ID) returns the stored result without
    /// running the handler again.
    pub fn request(
        &mut self,
        session: &mut PlaySession,
        player: usize,
        request_id: u64,
        channel: &str,
        data: &Json,
    ) -> Json {
        if let Some(done) = self.transactions.get(&(player, request_id)) {
            return done.clone();
        }
        let payload = json!({
            "player": player, "request_id": request_id, "channel": channel, "data": data
        });
        let result = match self.scripts.request(channel, &payload) {
            Ok(v) => v,
            Err(e) => json!({ "error": e.message }),
        };
        for issued in self.scripts.take_commands() {
            self.apply(session, issued);
        }
        self.apply_upgrades(session);
        self.transactions
            .insert((player, request_id), result.clone());
        // Bounded memory: keep the newest results per player.
        while self
            .transactions
            .range((player, 0)..=(player, u64::MAX))
            .count()
            > 1024
        {
            let oldest = *self.transactions.range((player, 0)..).next().unwrap().0;
            self.transactions.remove(&oldest);
        }
        result
    }

    /// Disable a mod mid-match: scripts unload and every upgrade it owns
    /// goes; items return to their base values on the next tick.
    pub fn disable_mod(&mut self, session: &mut PlaySession, id: &str) {
        self.scripts.unload_mod(id);
        self.upgrades.retain(|_, u| u.owner != id);
        self.apply_upgrades(session);
        self.apply_properties(session, 0.0);
    }

    /// Call `function` in the Lua file of the KV level definition matching
    /// the current level, if any (`Levels` registry, `ScriptFile`).
    fn level_handler(&mut self, function: &str, payload: &Json) {
        let level = self
            .level
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let handlers: Vec<Handler> = self
            .definitions
            .levels
            .iter()
            .filter(|l| {
                l.level
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or_default()
                    .eq_ignore_ascii_case(&level)
            })
            .filter_map(|l| match &l.script {
                Handler::Lua { file, .. } => Some(Handler::Lua {
                    file: file.clone(),
                    function: function.into(),
                }),
                _ => None,
            })
            .collect();
        for h in handlers {
            if let Err(e) = self.scripts.call_handler(&h, payload, None, true) {
                self.scripts.errors.push(e);
            }
        }
    }
    /// Registered Magicks each player may combine: unlocked by default or
    /// unlocked for that player (by script or pickup).
    fn sync_registered_unlocks(&self, session: &mut PlaySession) {
        for slot in 0..session.world.players.len() {
            let unlocked: Vec<u16> = self
                .definitions
                .magicks
                .iter()
                .filter(|m| m.legacy.is_none())
                .filter(|m| {
                    m.unlocked_by_default
                        || self
                            .custom_unlocks
                            .get(&slot)
                            .is_some_and(|s| s.contains(&m.common.id))
                })
                .filter_map(|m| self.definitions.magicks.index_of(&m.common.id).map(|i| i.0))
                .collect();
            if session.registered_unlocked[slot] != unlocked {
                session.registered_unlocked[slot] = unlocked;
            }
        }
    }
    /// Item definitions' intrinsic `Modifiers` last while the item is held.
    fn sync_item_modifiers(&mut self, session: &PlaySession) {
        let mut wanted = BTreeSet::new();
        for p in session.world.players.iter().flatten() {
            let Some(holder) = self.entity(p.handle) else {
                continue;
            };
            for (_, item) in &p.items {
                if !item.attached {
                    continue;
                }
                if let Some(def) = self.definitions.retail_item(&item.definition.name) {
                    for m in &def.modifiers {
                        wanted.insert((holder, m.clone()));
                    }
                }
            }
        }
        for (holder, m) in self
            .item_modifiers
            .difference(&wanted)
            .cloned()
            .collect::<Vec<_>>()
        {
            self.scripts.remove_modifier(holder, &m);
        }
        for (holder, m) in wanted
            .difference(&self.item_modifiers)
            .cloned()
            .collect::<Vec<_>>()
        {
            if let Err(e) =
                self.scripts
                    .apply_modifier(holder, &m, None, Some(holder), m.namespace())
            {
                session_log(&mut self.messages, &format!("item modifier {m}: {e}"));
            }
        }
        self.item_modifiers = wanted;
    }

    /// Modifier properties into native per-frame values.
    fn apply_properties(&mut self, session: &mut PlaySession, dt: f32) {
        let handles: Vec<u16> = self.generations.keys().copied().collect();
        for handle in handles {
            let Some(e) = self.entity(handle) else {
                continue;
            };
            let props = self.scripts.modifier_properties(e);
            let get = |k: &str| props.get(k).copied().unwrap_or(0.0) as f32;
            let speed = 1.0 + get("MoveSpeedPercent") / 100.0;
            let character = match session
                .world
                .players
                .iter_mut()
                .flatten()
                .find(|p| p.handle == handle)
            {
                Some(p) => Some((&mut p.character, p.body)),
                None => session
                    .world
                    .npcs
                    .get_mut(&handle)
                    .map(|n| (&mut n.character, n.body)),
            };
            if let Some((c, body)) = character {
                c.mod_speed = speed.max(0.0);
                let health = &mut c.combat.health;
                health.damage_taken_percent = get("DamageTakenPercent");
                // DamagePercent rides the native BoostDamage list, which
                // every spell, melee and projectile delivery already reads.
                let dealt = get("DamagePercent");
                c.buffs.mod_boost = (dealt != 0.0).then(|| magicka_sim::damage::Damage {
                    attack: 0,
                    element: magicka_sim::spells::Elements::ALL.0,
                    amount: 0.0,
                    magnitude: (1.0 + dealt / 100.0).max(0.0),
                });
                // MaxHealthBonus joins the native per-frame maximum; a rise
                // also raises current health by the same amount once.
                let bonus = get("MaxHealthBonus");
                let old = self.max_health_bonus.insert(handle, bonus).unwrap_or(0.0);
                health.max_health_bonus = bonus;
                if bonus > old && !c.combat.dead {
                    health.max_hit_points += bonus - old;
                    health.hit_points += bonus - old;
                    session.world.physics.bodies[body].owner.hit_points = health.hit_points;
                }
            }
            if let Some(regen) = props.get("HealthRegen").copied().filter(|r| *r != 0.0) {
                let amount = regen as f32 * dt;
                if amount > 0.0 {
                    session.world.mod_heal(handle, amount);
                } else {
                    let from = session
                        .world
                        .mod_character_body(handle)
                        .map(|b| session.world.physics.bodies[b].position)
                        .unwrap_or_default();
                    session
                        .world
                        .mod_damage(handle, damage(-amount, None), from);
                }
            }
        }
    }

    fn apply(&mut self, session: &mut PlaySession, issued: Issued) {
        let owner = issued.owner.clone();
        let result = self.execute(session, issued.command);
        if let Err(e) = result {
            session.world.log.push(format!("mod {owner}: {e}"));
        }
    }

    fn execute(&mut self, session: &mut PlaySession, command: Command) -> Result<(), String> {
        let stale = || "stale or unknown entity".to_string();
        match command {
            Command::Damage {
                target,
                amount,
                element,
                source,
            } => {
                let handle = self.resolve(target).ok_or_else(stale)?;
                let verdict = self.scripts.filter(
                    "damage",
                    &json!({ "target": entity_json(target), "amount": amount, "element": element,
                             "source": source.map(entity_json) }),
                );
                let amount = match verdict {
                    Verdict::Deny => return Ok(()),
                    Verdict::Allow => amount,
                    Verdict::Modified(m) => {
                        m["amount"].as_f64().unwrap_or(f64::from(amount)) as f32
                    }
                };
                let from = source
                    .and_then(|s| self.resolve(s))
                    .and_then(|h| session.world.mod_character_body(h))
                    .map(|b| session.world.physics.bodies[b].position)
                    .unwrap_or_default();
                if !session
                    .world
                    .mod_damage(handle, damage(amount, element.as_deref()), from)
                {
                    return Err(stale());
                }
                self.scripts.emit(
                    "entity_hurt",
                    &json!({ "target": entity_json(target), "amount": amount }),
                );
                Ok(())
            }
            Command::Heal { target, amount } => {
                let handle = self.resolve(target).ok_or_else(stale)?;
                let amount = match self.scripts.filter(
                    "heal",
                    &json!({ "target": entity_json(target), "amount": amount }),
                ) {
                    Verdict::Deny => return Ok(()),
                    Verdict::Allow => amount,
                    Verdict::Modified(m) => {
                        m["amount"].as_f64().unwrap_or(f64::from(amount)) as f32
                    }
                };
                session.world.mod_heal(handle, amount);
                Ok(())
            }
            Command::Teleport { target, position } => {
                let handle = self.resolve(target).ok_or_else(stale)?;
                let body = session.world.mod_character_body(handle).ok_or_else(stale)?;
                let b = &mut session.world.physics.bodies[body];
                let orientation = b.orientation;
                b.move_to(position, orientation);
                Ok(())
            }
            Command::Spawn {
                unit,
                position,
                tag,
            } => {
                let template = DefinitionId::parse(&unit, "base")
                    .ok()
                    .and_then(|id| self.definitions.units.get(&id))
                    .and_then(|u| u.template.clone())
                    .unwrap_or(unit);
                let handle = session.spawn_npc_at(&template, position)?;
                let snapshot = self.refresh(session);
                self.scripts.set_snapshot(snapshot);
                let entity = self.entity(handle);
                self.scripts.emit(
                    "entity_spawned",
                    &json!({ "tag": tag, "template": template, "entity": entity.map(entity_json) }),
                );
                Ok(())
            }
            Command::Remove { target } => {
                let handle = self.resolve(target).ok_or_else(stale)?;
                session.smite(handle)
            }
            Command::CastElements {
                caster,
                elements,
                delivery,
                target_position,
            } => {
                let handle = self.resolve(caster).ok_or_else(stale)?;
                let Some(slot) = session
                    .world
                    .players
                    .iter()
                    .position(|p| p.as_ref().is_some_and(|p| p.handle == handle))
                else {
                    // An NPC caster: its next attack casts these elements
                    // through the native NPC spellcasting path.
                    let npc = session.world.npcs.get_mut(&handle).ok_or_else(stale)?;
                    let combo: Vec<_> = elements
                        .iter()
                        .map(|e| element_bits(e).ok_or_else(|| format!("unknown element {e}")))
                        .collect::<Result<_, _>>()?;
                    let cast_type = match magicka_definitions::Delivery::parse(&delivery) {
                        Some(magicka_definitions::Delivery::Area) => {
                            magicka_sim::damage::CastType::Area
                        }
                        Some(magicka_definitions::Delivery::SelfCast) => {
                            magicka_sim::damage::CastType::SelfCast
                        }
                        Some(magicka_definitions::Delivery::Weapon) => {
                            magicka_sim::damage::CastType::Weapon
                        }
                        Some(_) => magicka_sim::damage::CastType::Force,
                        None => return Err(format!("unknown delivery {delivery}")),
                    };
                    let _ = target_position;
                    if !npc.combat.script_cast(combo, cast_type) {
                        return Err("this character has no spell abilities to cast with".into());
                    }
                    return Ok(());
                };
                self.scripts.filter(
                    "cast",
                    &json!({ "caster": entity_json(caster), "elements": elements }),
                );
                self.casts.insert(
                    slot,
                    (false, cast_sequence(&elements, &delivery, target_position)?),
                );
                Ok(())
            }
            Command::InvokeMagick {
                caster,
                magick,
                target_position,
            } => {
                let handle = self.resolve(caster).ok_or_else(stale)?;
                self.invoke_magick(session, handle, &magick, target_position)
            }
            Command::UnlockMagick { player, magick } => {
                let def = self
                    .definitions
                    .magicks
                    .get(&magick)
                    .ok_or("unknown magick")?;
                match def.legacy {
                    Some(bit) => {
                        if player >= session.unlocked_magicks.len() {
                            return Err("no such player".into());
                        }
                        session.unlocked_magicks[player] |= 1u64 << bit;
                    }
                    None => {
                        self.custom_unlocks
                            .entry(player)
                            .or_default()
                            .insert(magick);
                    }
                }
                Ok(())
            }
            Command::Particle {
                effect,
                position,
                owner,
            } => {
                let owner = owner.and_then(|o| self.resolve(o));
                self.presentation.push(PresentationEvent {
                    sound: false,
                    name: effect,
                    position,
                    owner,
                });
                Ok(())
            }
            Command::Sound {
                sound: effect,
                position,
                owner,
            } => {
                let sound = true;
                let owner = owner.and_then(|o| self.resolve(o));
                self.presentation.push(PresentationEvent {
                    sound,
                    name: effect,
                    position,
                    owner,
                });
                Ok(())
            }
            Command::World { op, args } => {
                let resolve = |e| self.resolve(e);
                let op = crate::mod_world::parse(&op, &args, &resolve)?;
                session.world.script_op(op)
            }
            Command::Message { text, player } => {
                session.world.log.push(text.clone());
                self.messages.push((player, text));
                Ok(())
            }
            Command::Upgrade(upgrade) => {
                let valid = session
                    .world
                    .players
                    .get(upgrade.player)
                    .is_some_and(Option::is_some)
                    && match upgrade.change {
                        magicka_definitions::schema::Change::Add(v)
                        | magicka_definitions::schema::Change::Percent(v)
                        | magicka_definitions::schema::Change::Multiply(v)
                        | magicka_definitions::schema::Change::Set(v) => v.is_finite(),
                    };
                if !valid {
                    return Err("invalid upgrade".into());
                }
                self.upgrades.insert(upgrade.id.clone(), upgrade);
                Ok(())
            }
            Command::RemoveUpgrade { id } => {
                self.upgrades.remove(&id);
                Ok(())
            }
            Command::LevelAction { name, arguments } => {
                self.levels.run_action(session, &name, &arguments)
            }
            Command::DefineCondition {
                id,
                name,
                arguments,
            } => {
                self.levels.define_condition(id, &name, &arguments);
                Ok(())
            }
            Command::AddTrigger { xml } => session.add_script_trigger(&xml).map(drop),
            Command::WatchArea {
                name,
                position,
                radius,
            } => {
                self.levels
                    .areas
                    .insert(name, (position, radius, BTreeSet::new()));
                Ok(())
            }
            Command::SpawnWave { name, units } => {
                let mut wave = crate::mod_levels::Wave {
                    expected: units.len(),
                    ..Default::default()
                };
                for (unit, position) in units {
                    let template = DefinitionId::parse(&unit, "base")
                        .ok()
                        .and_then(|id| self.definitions.units.get(&id))
                        .and_then(|u| u.template.clone())
                        .unwrap_or(unit);
                    let handle = session.spawn_npc_at(&template, position)?;
                    let snapshot = self.refresh(session);
                    self.scripts.set_snapshot(snapshot);
                    if let Some(e) = self.entity(handle) {
                        wave.members.insert(e);
                    }
                }
                wave.expected = wave.members.len();
                self.levels.waves.insert(name.clone(), wave);
                let payload = json!({ "wave": name });
                self.scripts.call_global("OnWaveStart", &payload);
                self.level_handler("OnWaveStart", &payload);
                Ok(())
            }
        }
    }

    /// Cast a registry Magick for `handle`: native built-ins through the
    /// original effect dispatch, mod Magicks through their Lua handler.
    pub fn invoke_magick(
        &mut self,
        session: &mut PlaySession,
        handle: u16,
        magick: &DefinitionId,
        target_position: Option<[f32; 3]>,
    ) -> Result<(), String> {
        let def = self
            .definitions
            .magicks
            .get(magick)
            .ok_or_else(|| format!("unknown magick {magick}"))?
            .clone();
        let Handler::Lua { .. } = &def.handler else {
            let Some(legacy) = def.legacy else {
                return Err(format!("{magick} has no handler"));
            };
            let m = magicka_sim::spells::MagickType::try_from(legacy)
                .map_err(|_| "bad legacy magick")?;
            session.world.cast_magick_from(handle, m);
            return Ok(());
        };
        let caster = self.entity(handle).ok_or("stale caster")?;
        // The native aim: a player's mouse pick, else the target's position.
        let aim = session.world.magick_aim(handle);
        let target_entity = aim.and_then(|(_, _, t)| (t != handle).then_some(t));
        let target_body = target_entity.and_then(|t| {
            session
                .world
                .players
                .iter()
                .flatten()
                .find(|p| p.handle == t)
                .map(|p| p.body)
                .or_else(|| session.world.npcs.get(&t).map(|n| n.body))
        });
        let target = target_position
            .or_else(|| aim.and_then(|(_, pick, _)| pick))
            .or_else(|| target_body.map(|b| session.world.physics.bodies[b].position));
        let ctx = json!({
            "caster": entity_json(caster),
            "magick": magick.as_str(),
            "target_position": target.map(|p| json!({"x": p[0], "y": p[1], "z": p[2]})),
            "target": target_entity.and_then(|t| self.entity(t)).map(entity_json),
        });
        self.scripts
            .call_handler(&def.handler, &ctx, Some(caster), false)
            .map_err(|e| e.to_string())?;
        if let Some(effect) = &def.particle {
            self.presentation.push(PresentationEvent {
                sound: false,
                name: effect.clone(),
                position: target,
                owner: Some(handle),
            });
        }
        if let Some(sound) = &def.sound {
            self.presentation.push(PresentationEvent {
                sound: true,
                name: sound.clone(),
                position: target,
                owner: Some(handle),
            });
        }
        for issued in self.scripts.take_commands() {
            self.apply(session, issued);
        }
        Ok(())
    }

    /// Scripted wizard input (CastElements) for this tick, after network
    /// input: it holds the player's controls until the cast is released.
    fn drive_casts(&mut self, session: &mut PlaySession) {
        let slots: Vec<usize> = self.casts.keys().copied().collect();
        for slot in slots {
            // Like a player, a script waits for the wizard to finish its
            // current cast or animation before typing a new spell.
            let free = session
                .world
                .players
                .get(slot)
                .and_then(Option::as_ref)
                .is_some_and(|p| {
                    matches!(
                        p.motion.state,
                        crate::avatar::AvatarState::Idle | crate::avatar::AvatarState::Move
                    )
                });
            let entry = self.casts.get_mut(&slot).unwrap();
            if !entry.0 && !free {
                continue;
            }
            entry.0 = true;
            let Some(controls) = entry.1.pop_front() else {
                self.casts.remove(&slot);
                session.scripted_input[slot] = false;
                continue;
            };
            session.scripted_input[slot] = true;
            if session.handle_remote_input(slot, controls).is_err() {
                self.casts.remove(&slot);
                session.scripted_input[slot] = false;
            }
        }
    }

    /// Saved mod state (scripts, modifiers, net tables, custom unlocks).
    pub fn save_state(&mut self) -> Json {
        let mut state = self.scripts.save_state();
        state["upgrades"] = json!(self.upgrades.values().collect::<Vec<_>>());
        state["transactions"] = json!(self
            .transactions
            .iter()
            .map(|((p, r), v)| json!([p, r, v]))
            .collect::<Vec<_>>());
        state["custom_unlocks"] = json!(self
            .custom_unlocks
            .iter()
            .map(|(slot, set)| (
                slot.to_string(),
                set.iter().map(|m| m.to_string()).collect::<Vec<_>>()
            ))
            .collect::<BTreeMap<_, _>>());
        state
    }
    pub fn load_state(&mut self, state: &Json) {
        self.scripts.load_state(state);
        if let Ok(list) =
            serde_json::from_value::<Vec<magicka_script::Upgrade>>(state["upgrades"].clone())
        {
            self.upgrades = list.into_iter().map(|u| (u.id.clone(), u)).collect();
        }
        for t in state["transactions"].as_array().into_iter().flatten() {
            if let (Some(p), Some(r)) = (t[0].as_u64(), t[1].as_u64()) {
                self.transactions.insert((p as usize, r), t[2].clone());
            }
        }
        if let Some(map) = state["custom_unlocks"].as_object() {
            for (slot, list) in map {
                let (Ok(slot), Some(list)) = (slot.parse::<usize>(), list.as_array()) else {
                    continue;
                };
                for id in list.iter().filter_map(Json::as_str) {
                    if let Ok(id) = DefinitionId::parse(id, "base") {
                        if self.definitions.magicks.contains(&id) {
                            self.custom_unlocks.entry(slot).or_default().insert(id);
                        }
                    }
                }
            }
        }
    }
}

fn session_log(messages: &mut Vec<(Option<usize>, String)>, text: &str) {
    messages.push((None, text.to_string()));
}

fn damage(amount: f32, element: Option<&str>) -> magicka_sim::damage::Damage {
    magicka_sim::damage::Damage {
        attack: magicka_sim::damage::attack::DAMAGE,
        element: element.and_then(element_bits).map_or(0, |e| e.0),
        amount,
        magnitude: 1.0,
    }
}

/// Element key presses (one tick down, one up each), then the delivery's
/// cast button held long enough to release a spell, then released.
fn cast_sequence(
    elements: &[String],
    delivery: &str,
    aim: Option<[f32; 3]>,
) -> Result<VecDeque<Controls>, String> {
    let keys = crate::avatar::ELEMENT_KEYS;
    let mut out = VecDeque::new();
    let base = Controls {
        aim,
        ..Default::default()
    };
    out.push_back(base);
    for name in elements {
        let e = element_bits(name).ok_or_else(|| format!("unknown element {name}"))?;
        let key = keys
            .iter()
            .position(|k| *k == e)
            .ok_or_else(|| format!("{name} has no key (combine it from basic elements)"))?;
        let mut down = base;
        down.elements[key] = true;
        out.push_back(down);
        out.push_back(base);
    }
    let held = match magicka_definitions::Delivery::parse(delivery) {
        Some(magicka_definitions::Delivery::Area) => Controls {
            shift_down: true,
            cast_down: true,
            ..base
        },
        Some(magicka_definitions::Delivery::SelfCast) => Controls {
            cast_self_down: true,
            ..base
        },
        Some(magicka_definitions::Delivery::Weapon) => {
            return Err("weapon imbue is not a scripted delivery".into())
        }
        Some(_) => Controls {
            cast_down: true,
            ..base
        },
        None => return Err(format!("unknown delivery {delivery}")),
    };
    for _ in 0..20 {
        out.push_back(held);
    }
    out.push_back(base);
    Ok(out)
}
