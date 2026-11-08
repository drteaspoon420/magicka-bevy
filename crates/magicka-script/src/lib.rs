//! Server-side Lua for mods (rewrite extension, plan workstream E).
//!
//! One [`ScriptHost`] VM per match runs on the host only. Each mod gets its
//! own environment over a shared, sandboxed set of globals; `require` and
//! handler files resolve only through the mod filesystem. Scripts read an
//! owned [`WorldSnapshot`] and queue typed [`Command`]s; the runtime adapter
//! validates and applies them at a fixed phase. Lua is Lua 5.4 through
//! `mlua` (vendored), not Valve's VScript; see `docs/MODDING.md`.
pub mod convert;
pub mod modifiers;
pub mod types;

use convert::{from_lua, to_lua};
use magicka_definitions::{DefinitionId, Definitions, Handler};
use magicka_mods::{vfs::LogicalPath, ModSet};
use mlua::{
    Function, HookTriggers, Lua, LuaOptions, MultiValue, RegistryKey, StdLib, Table, UserData,
    UserDataMethods, Value, Variadic, VmState,
};
use modifiers::{ModifierEvent, ModifierStore};
use serde_json::{json, Value as Json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
pub use types::*;

/// The scripting API version mods target (`ApiVersion` in `mod.kv`).
pub const API_VERSION: u32 = 1;
/// Hook granularity for the instruction budget.
const HOOK_STEP: u32 = 1000;

struct Subscription {
    token: u64,
    owner: usize,
    event: String,
    priority: i64,
    filter: bool,
    func: RegistryKey,
}
struct Timer {
    owner: usize,
    due: f64,
    interval: Option<f64>,
    func: RegistryKey,
}

/// Mutable host state reachable from API functions (Lua app data). Never
/// borrowed across a call back into Lua.
struct State {
    snapshot: WorldSnapshot,
    commands: Vec<Issued>,
    current: Option<usize>,
    owners: Vec<String>,
    rng: u64,
    budget: u64,
    limits: Limits,
    subscriptions: Vec<Subscription>,
    next_token: u64,
    timers: BTreeMap<u64, Timer>,
    next_timer: u64,
    fired: Vec<(usize, String, Json)>,
    net_tables: BTreeMap<String, BTreeMap<String, (usize, Json)>>,
    net_dirty: BTreeSet<String>,
    modifiers: ModifierStore,
    modifier_events: Vec<ModifierEvent>,
    defs: Arc<Definitions>,
    set: Arc<ModSet>,
    loaded: BTreeMap<(usize, String), RegistryKey>,
    loading: BTreeSet<(usize, String)>,
    log: Vec<String>,
    spawns: usize,
    time: f64,
    next_condition: u64,
}

fn err(message: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(message.into())
}

impl State {
    fn owner(&self) -> mlua::Result<usize> {
        self.current
            .ok_or_else(|| err("API used outside a script callback"))
    }
    fn issue(&mut self, command: Command) -> mlua::Result<()> {
        let owner = self.owner()?;
        if self.commands.len() >= self.limits.commands_per_tick {
            return Err(err("command limit for this tick exceeded"));
        }
        let spawns = match &command {
            Command::Spawn { .. } => true,
            Command::World { op, .. } => world_ops::spawns(op),
            _ => false,
        };
        if spawns {
            if self.spawns >= self.limits.spawns_per_tick {
                return Err(err("spawn limit for this tick exceeded"));
            }
            self.spawns += 1;
        }
        self.commands.push(Issued {
            owner: self.owners[owner].clone(),
            command,
        });
        Ok(())
    }
    /// SplitMix64: the server's seeded gameplay RNG (recordable seed).
    fn next_random(&mut self) -> u64 {
        self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn owned_count(&self, owner: usize, timers: bool) -> usize {
        if timers {
            self.timers.values().filter(|t| t.owner == owner).count()
        } else {
            self.subscriptions
                .iter()
                .filter(|s| s.owner == owner)
                .count()
        }
    }
}

fn with_state<R>(lua: &Lua, f: impl FnOnce(&mut State) -> mlua::Result<R>) -> mlua::Result<R> {
    let mut state = lua
        .app_data_mut::<State>()
        .ok_or_else(|| err("script host state unavailable"))?;
    f(&mut state)
}

impl UserData for EntityRef {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        fn info<R>(
            lua: &Lua,
            e: &EntityRef,
            f: impl FnOnce(&EntityInfo) -> R,
        ) -> mlua::Result<Option<R>> {
            with_state(lua, |s| Ok(s.snapshot.entity(*e).map(f)))
        }
        methods.add_method("IsValid", |lua, e, ()| Ok(info(lua, e, |_| ())?.is_some()));
        methods.add_method("GetHandle", |_, e, ()| Ok((e.handle, e.generation)));
        methods.add_method("IsAlive", |lua, e, ()| {
            Ok(info(lua, e, |i| i.alive)?.unwrap_or(false))
        });
        methods.add_method("GetHealth", |lua, e, ()| info(lua, e, |i| i.health));
        methods.add_method("GetMaxHealth", |lua, e, ()| info(lua, e, |i| i.max_health));
        methods.add_method("GetName", |lua, e, ()| info(lua, e, |i| i.name.clone()));
        methods.add_method("GetPlayer", |lua, e, ()| {
            Ok(info(lua, e, |i| i.player)?.flatten())
        });
        methods.add_method("GetFaction", |lua, e, ()| info(lua, e, |i| i.faction));
        methods.add_method("GetPosition", |lua, e, ()| {
            match info(lua, e, |i| i.position)? {
                Some(p) => Ok(Value::Table(vec3(lua, p)?)),
                None => Ok(Value::Nil),
            }
        });
        methods.add_method("GetForward", |lua, e, ()| {
            match info(lua, e, |i| i.facing)? {
                Some(p) => Ok(Value::Table(vec3(lua, p)?)),
                None => Ok(Value::Nil),
            }
        });
        methods.add_method("HasStatus", |lua, e, name: String| {
            let name = name.to_ascii_lowercase();
            Ok(info(lua, e, |i| i.statuses.contains(&name))?.unwrap_or(false))
        });
        methods.add_method("IsEthereal", |lua, e, ()| {
            Ok(info(lua, e, |i| i.ethereal)?.unwrap_or(false))
        });
        methods.add_method("HasModifier", |lua, e, id: String| {
            let id = DefinitionId::parse(&id, "base").map_err(err)?;
            with_state(lua, |s| Ok(s.modifiers.has(*e, &id)))
        });
        methods.add_method("GetModifierStacks", |lua, e, id: String| {
            let id = DefinitionId::parse(&id, "base").map_err(err)?;
            with_state(lua, |s| Ok(s.modifiers.stacks(*e, &id)))
        });
        methods.add_meta_method("__eq", |_, a, b: mlua::AnyUserData| {
            Ok(b.borrow::<EntityRef>().is_ok_and(|b| *a == *b))
        });
        methods.add_meta_method("__tostring", |_, e, ()| {
            Ok(format!("Entity({}#{})", e.handle, e.generation))
        });
    }
}

fn vec3(lua: &Lua, p: [f32; 3]) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("x", p[0])?;
    t.set("y", p[1])?;
    t.set("z", p[2])?;
    Ok(t)
}
fn read_vec3(v: &Value) -> mlua::Result<[f32; 3]> {
    let Value::Table(t) = v else {
        return Err(err("a position must be {x=,y=,z=} or {x,y,z}"));
    };
    let get = |named: &str, index: usize| -> mlua::Result<f32> {
        let v: Option<f64> = t.get(named)?;
        let v = match v {
            Some(v) => v,
            None => t
                .get::<Option<f64>>(index)?
                .ok_or_else(|| err("a position needs x, y and z"))?,
        };
        if !v.is_finite() {
            return Err(err("positions must be finite"));
        }
        Ok(v as f32)
    };
    Ok([get("x", 1)?, get("y", 2)?, get("z", 3)?])
}
fn read_entity(v: &Value) -> mlua::Result<EntityRef> {
    match v {
        Value::UserData(u) => Ok(*u.borrow::<EntityRef>()?),
        _ => Err(err("expected an entity")),
    }
}
fn read_number(v: &Value, what: &str) -> mlua::Result<f64> {
    let n = match v {
        Value::Integer(i) => *i as f64,
        Value::Number(n) => *n,
        _ => return Err(err(format!("{what} must be a number"))),
    };
    if !n.is_finite() {
        return Err(err(format!("{what} must be finite")));
    }
    Ok(n)
}
fn read_string(v: &Value, what: &str) -> mlua::Result<String> {
    match v {
        Value::String(s) => Ok(s.to_str()?.to_string()),
        _ => Err(err(format!("{what} must be a string"))),
    }
}
fn read_id(v: &Value, what: &str) -> mlua::Result<DefinitionId> {
    DefinitionId::parse(&read_string(v, what)?, "base").map_err(err)
}
fn opt(t: Option<&Table>, key: &str) -> mlua::Result<Value> {
    match t {
        Some(t) => t.get(key),
        None => Ok(Value::Nil),
    }
}

/// Arguments after dropping a leading namespace table (`NS:Func()` style).
fn args(mv: Variadic<Value>) -> Vec<Value> {
    let mut v: Vec<Value> = mv.into_iter().collect();
    if let Some(Value::Table(t)) = v.first() {
        if t.raw_get::<Option<String>>("__namespace")
            .ok()
            .flatten()
            .is_some()
        {
            v.remove(0);
        }
    }
    v
}
fn arg(v: &[Value], i: usize) -> Value {
    v.get(i).cloned().unwrap_or(Value::Nil)
}

/// One loaded mod (or the base layer's scripts).
struct ModRuntime {
    id: String,
    env: RegistryKey,
    failed: bool,
}

pub struct ScriptHost {
    lua: Lua,
    mods: Vec<ModRuntime>,
    pub errors: Vec<ScriptError>,
    /// Handler file environments by (owner, logical path).
    handlers: BTreeMap<(usize, String), RegistryKey>,
}

impl ScriptHost {
    /// Create the match VM and load every mod that ships server scripts
    /// (`scripts/vscripts/<mod id>/init.lua`), in load order, then run
    /// `OnModLoad`. A failing mod is recorded and skipped.
    pub fn new(
        set: Arc<ModSet>,
        defs: Arc<Definitions>,
        limits: Limits,
        seed: u64,
    ) -> Result<Self, ScriptError> {
        let fail = |e: mlua::Error| ScriptError {
            owner: "engine".into(),
            callback: "create VM".into(),
            entity: None,
            message: e.to_string(),
        };
        let lua = Lua::new_with(
            StdLib::COROUTINE | StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH,
            LuaOptions::default(),
        )
        .map_err(fail)?;
        lua.set_memory_limit(limits.memory_bytes).map_err(fail)?;
        let mut owners: Vec<String> = Vec::new();
        let mut entries = Vec::new();
        for layer in set.vfs.layers() {
            let owner = if layer.name == magicka_mods::BASE_ID {
                "base".to_string()
            } else {
                layer.name.clone()
            };
            let server = layer.name == magicka_mods::BASE_ID
                || set
                    .mods
                    .iter()
                    .any(|m| m.id == layer.name && m.server_scripts);
            owners.push(owner.clone());
            if server {
                if let Ok(path) = LogicalPath::new(&format!("scripts/vscripts/{owner}/init.lua")) {
                    if set.vfs.resolve(&path).is_some() {
                        entries.push((owners.len() - 1, path));
                    }
                }
            }
        }
        lua.set_app_data(State {
            snapshot: WorldSnapshot::default(),
            commands: Vec::new(),
            current: None,
            owners: owners.clone(),
            rng: seed,
            budget: limits.instructions,
            limits,
            subscriptions: Vec::new(),
            next_token: 0,
            timers: BTreeMap::new(),
            next_timer: 0,
            fired: Vec::new(),
            net_tables: BTreeMap::new(),
            net_dirty: BTreeSet::new(),
            modifiers: ModifierStore::default(),
            modifier_events: Vec::new(),
            defs,
            set,
            loaded: BTreeMap::new(),
            loading: BTreeSet::new(),
            log: Vec::new(),
            spawns: 0,
            time: 0.0,
            next_condition: 0,
        });
        lua.set_global_hook(
            HookTriggers::new().every_nth_instruction(HOOK_STEP),
            |lua, _| {
                with_state(lua, |s| {
                    if s.budget <= u64::from(HOOK_STEP) {
                        s.budget = 0;
                        return Err(err("instruction budget exceeded"));
                    }
                    s.budget -= u64::from(HOOK_STEP);
                    Ok(VmState::Continue)
                })
            },
        )
        .map_err(fail)?;
        let safe = Self::sandbox(&lua).map_err(fail)?;
        let mut mods = Vec::new();
        for (i, owner) in owners.iter().enumerate() {
            let env = lua.create_table().map_err(fail)?;
            let meta = lua.create_table().map_err(fail)?;
            meta.set("__index", safe.clone()).map_err(fail)?;
            env.set_metatable(Some(meta)).map_err(fail)?;
            lua.set_named_registry_value(&format!("magicka.env.{i}"), env.clone())
                .map_err(fail)?;
            mods.push(ModRuntime {
                id: owner.clone(),
                env: lua.create_registry_value(env).map_err(fail)?,
                failed: false,
            });
        }
        let mut host = Self {
            lua,
            mods,
            errors: Vec::new(),
            handlers: BTreeMap::new(),
        };
        for (owner, path) in entries {
            if let Err(e) = host.load_module(owner, &path) {
                host.mods[owner].failed = true;
                host.errors.push(ScriptError {
                    owner: host.mods[owner].id.clone(),
                    callback: format!("load {path}"),
                    entity: None,
                    message: e.to_string(),
                });
            }
        }
        host.lifecycle(Lifecycle::ModLoad, Json::Null);
        Ok(host)
    }

    /// The shared, read-only globals: safe base functions, the standard
    /// tables, and the engine API namespaces.
    fn sandbox(lua: &Lua) -> mlua::Result<Table> {
        let g = lua.globals();
        let safe = lua.create_table()?;
        for name in [
            "assert",
            "error",
            "ipairs",
            "next",
            "pairs",
            "pcall",
            "select",
            "tonumber",
            "tostring",
            "type",
            "xpcall",
            "rawequal",
            "rawget",
            "rawset",
            "rawlen",
            "setmetatable",
            "getmetatable",
        ] {
            safe.set(name, g.get::<Value>(name)?)?;
        }
        safe.set("_VERSION", "Lua 5.4 (Magicka rewrite sandbox)")?;
        // Per-VM copies of the libraries; no string.dump, seeded random.
        for lib in ["string", "table", "math", "utf8", "coroutine"] {
            let original: Table = g.get(lib)?;
            let copy = lua.create_table()?;
            for pair in original.pairs::<Value, Value>() {
                let (k, v) = pair?;
                copy.set(k, v)?;
            }
            safe.set(lib, copy)?;
        }
        let string: Table = safe.get("string")?;
        string.set("dump", Value::Nil)?;
        // Strings' metatable is reachable from any script: hide it so no
        // mod can alter the shared string library through it.
        lua.load("getmetatable('').__metatable = false").exec()?;
        let math: Table = safe.get("math")?;
        let random = lua.create_function(|lua, mv: Variadic<Value>| {
            let v: Vec<Value> = mv.into_iter().collect();
            let r = with_state(lua, |s| Ok(s.next_random()))?;
            let unit = (r >> 11) as f64 / (1u64 << 53) as f64;
            match (v.first(), v.get(1)) {
                (None, _) => Ok(Value::Number(unit)),
                (Some(a), b) => {
                    let (lo, hi) = match b {
                        Some(b) => (read_number(a, "m")? as i64, read_number(b, "n")? as i64),
                        None => (1, read_number(a, "m")? as i64),
                    };
                    if lo > hi {
                        return Err(err("interval is empty"));
                    }
                    let span = (hi - lo) as u64 + 1;
                    Ok(Value::Integer(lo + (r % span) as i64))
                }
            }
        })?;
        math.set("random", random.clone())?;
        math.set(
            "randomseed",
            lua.create_function(|_, _: MultiValue| Ok(()))?,
        )?;
        safe.set(
            "print",
            lua.create_function(|lua, mv: Variadic<Value>| {
                let text: Vec<String> = mv
                    .iter()
                    .map(|v| v.to_string().unwrap_or_else(|_| v.type_name().into()))
                    .collect();
                with_state(lua, |s| {
                    let owner = s.current.map_or("engine".into(), |o| s.owners[o].clone());
                    s.log.push(format!("[{owner}] {}", text.join("\t")));
                    Ok(())
                })
            })?,
        )?;
        safe.set(
            "require",
            lua.create_function(|lua, name: String| require(lua, &name))?,
        )?;
        api::install(lua, &safe, random)?;
        world_ops::install(lua, &safe)?;
        Ok(safe)
    }

    fn mod_index(&self, id: &str) -> Option<usize> {
        self.mods.iter().position(|m| m.id == id)
    }

    /// Run a chunk in a mod's environment, once per (mod, path).
    fn load_module(&mut self, owner: usize, path: &LogicalPath) -> mlua::Result<Value> {
        let env: Table = self.lua.registry_value(&self.mods[owner].env)?;
        let text = with_state(&self.lua, |s| {
            s.set
                .vfs
                .read_string(path)
                .ok_or_else(|| err(format!("{path} not found")))?
                .map_err(err)
        })?;
        let f = self
            .lua
            .load(&text)
            .set_name(format!("@{path}"))
            .set_environment(env)
            .into_function()?;
        self.call(owner, f, ())
    }

    /// Invoke Lua as `owner` under a fresh instruction budget.
    fn call(&self, owner: usize, f: Function, a: impl mlua::IntoLuaMulti) -> mlua::Result<Value> {
        let previous = with_state(&self.lua, |s| {
            let p = (s.current, s.budget);
            s.current = Some(owner);
            s.budget = s.limits.instructions;
            Ok(p)
        })?;
        let result = f.call::<Value>(a);
        with_state(&self.lua, |s| {
            s.current = previous.0;
            if previous.0.is_some() {
                s.budget = previous.1;
            }
            Ok(())
        })?;
        result
    }

    fn report(&mut self, owner: usize, callback: &str, entity: Option<EntityRef>, e: mlua::Error) {
        self.errors.push(ScriptError {
            owner: self.mods[owner].id.clone(),
            callback: callback.into(),
            entity,
            message: e.to_string(),
        });
    }

    pub fn mod_ids(&self) -> Vec<String> {
        self.mods.iter().map(|m| m.id.clone()).collect()
    }
    pub fn loaded(&self, id: &str) -> bool {
        self.mod_index(id).is_some_and(|i| !self.mods[i].failed)
    }

    /// Call a lifecycle function in every loaded mod, in load order.
    pub fn lifecycle(&mut self, phase: Lifecycle, payload: Json) {
        for owner in 0..self.mods.len() {
            if self.mods[owner].failed {
                continue;
            }
            let result = (|| {
                let env: Table = self.lua.registry_value(&self.mods[owner].env)?;
                let Some(f) = env.raw_get::<Option<Function>>(phase.function())? else {
                    return Ok(());
                };
                let arg = to_lua(&self.lua, &payload)?;
                self.call(owner, f, arg).map(drop)
            })();
            if let Err(e) = result {
                self.report(owner, phase.function(), None, e);
            }
        }
        self.drain_fired();
    }

    /// Call a global function by name in every loaded mod (level and game
    /// callbacks such as `OnNpcKilled`), and notify subscribers of the
    /// event of the same name.
    pub fn call_global(&mut self, function: &str, payload: &Json) {
        for owner in 0..self.mods.len() {
            if self.mods[owner].failed {
                continue;
            }
            let result = (|| {
                let env: Table = self.lua.registry_value(&self.mods[owner].env)?;
                let Some(f) = env.raw_get::<Option<Function>>(function)? else {
                    return Ok(());
                };
                let arg = to_lua(&self.lua, payload)?;
                self.call(owner, f, arg).map(drop)
            })();
            if let Err(e) = result {
                self.report(owner, function, None, e);
            }
        }
        self.emit(function, payload);
    }

    /// A client's request to a mod (`OnModRequest(request)` in the mod named
    /// by the channel prefix, e.g. `boosted.buy`). Returns the handler's
    /// result. The engine stores results by request ID for exactly-once.
    pub fn request(&mut self, channel: &str, payload: &Json) -> Result<Json, ScriptError> {
        let id = channel.split('.').next().unwrap_or_default();
        let fail = |message: String| ScriptError {
            owner: id.into(),
            callback: format!("OnModRequest {channel}"),
            entity: None,
            message,
        };
        let owner = self
            .mod_index(id)
            .filter(|o| !self.mods[*o].failed)
            .ok_or_else(|| fail("no such mod".into()))?;
        let result = (|| {
            let env: Table = self.lua.registry_value(&self.mods[owner].env)?;
            let Some(f) = env.raw_get::<Option<Function>>("OnModRequest")? else {
                return Err(err("the mod takes no requests"));
            };
            let arg = to_lua(&self.lua, payload)?;
            let v = self.call(owner, f, arg)?;
            from_lua(&v)
        })()
        .map_err(|e| fail(e.to_string()));
        self.drain_fired();
        result
    }

    /// What scripts see this tick.
    pub fn set_snapshot(&mut self, snapshot: WorldSnapshot) {
        let _ = with_state(&self.lua, |s| {
            s.time = snapshot.time;
            s.snapshot = snapshot;
            s.spawns = 0;
            Ok(())
        });
    }

    /// Notify subscribers (post-action; cannot change the outcome).
    pub fn emit(&mut self, event: &str, payload: &Json) {
        self.dispatch(event, payload, false);
        self.drain_fired();
    }

    fn dispatch(
        &mut self,
        event: &str,
        payload: &Json,
        filters: bool,
    ) -> Vec<(usize, Result<Value, mlua::Error>)> {
        let mut order: Vec<(i64, u64, usize)> = with_state(&self.lua, |s| {
            Ok(s.subscriptions
                .iter()
                .filter(|sub| sub.event == event && sub.filter == filters)
                .map(|sub| (-sub.priority, sub.token, sub.owner))
                .collect())
        })
        .unwrap_or_default();
        order.sort();
        let mut out = Vec::new();
        for (_, token, owner) in order {
            if self.mods[owner].failed {
                continue;
            }
            let f = with_state(&self.lua, |s| {
                let sub = s.subscriptions.iter().find(|x| x.token == token);
                match sub {
                    Some(sub) => Ok(Some(self.lua.registry_value::<Function>(&sub.func)?)),
                    None => Ok(None),
                }
            });
            let Ok(Some(f)) = f else { continue };
            let result = to_lua(&self.lua, payload).and_then(|arg| self.call(owner, f, arg));
            if let Err(e) = &result {
                if !filters {
                    self.report(owner, &format!("event {event}"), None, e.clone());
                }
            }
            out.push((owner, result));
        }
        out
    }

    /// Run pre-action filters: each sees the (possibly modified) action and
    /// returns `true`/`nil` (allow), `false` (deny) or a table of changed
    /// fields. Only the filter's mutable fields may change.
    pub fn filter(&mut self, name: &str, action: &Json) -> Verdict {
        let Some(spec) = FILTERS.iter().find(|f| f.name == name) else {
            return Verdict::Allow;
        };
        let mut current = action.clone();
        let mut modified = false;
        let mut order: Vec<(i64, u64, usize)> = with_state(&self.lua, |s| {
            Ok(s.subscriptions
                .iter()
                .filter(|sub| sub.event == name && sub.filter)
                .map(|sub| (-sub.priority, sub.token, sub.owner))
                .collect())
        })
        .unwrap_or_default();
        order.sort();
        for (_, token, owner) in order {
            if self.mods[owner].failed {
                continue;
            }
            let f = with_state(&self.lua, |s| {
                match s.subscriptions.iter().find(|x| x.token == token) {
                    Some(sub) => Ok(Some(self.lua.registry_value::<Function>(&sub.func)?)),
                    None => Ok(None),
                }
            });
            let Ok(Some(f)) = f else { continue };
            let result = to_lua(&self.lua, &current).and_then(|arg| self.call(owner, f, arg));
            match result {
                Ok(Value::Nil) | Ok(Value::Boolean(true)) => {}
                Ok(Value::Boolean(false)) => return Verdict::Deny,
                Ok(v @ Value::Table(_)) => {
                    if let Ok(Json::Object(changes)) = from_lua(&v) {
                        for (k, v) in changes {
                            if spec.mutable.contains(&k.as_str()) && v.is_number() {
                                current[k.as_str()] = v;
                                modified = true;
                            } else {
                                self.errors.push(ScriptError {
                                    owner: self.mods[owner].id.clone(),
                                    callback: format!("filter {name}"),
                                    entity: None,
                                    message: format!("field {k} is not modifiable"),
                                });
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    self.report(owner, &format!("filter {name}"), None, e);
                    if spec.on_error == OnError::FailClosed {
                        return Verdict::Deny;
                    }
                }
            }
        }
        if modified {
            let Json::Object(map) = current else {
                return Verdict::Allow;
            };
            Verdict::Modified(map)
        } else {
            Verdict::Allow
        }
    }

    /// Advance simulation time: due timers (by due time, then id), then
    /// modifier thinks and expirations, then script-fired events.
    pub fn tick(&mut self, time: f64) {
        let _ = with_state(&self.lua, |s| {
            s.time = time;
            Ok(())
        });
        loop {
            let due = with_state(&self.lua, |s| {
                Ok(s.timers
                    .iter()
                    .filter(|(_, t)| t.due <= time)
                    .min_by(|a, b| a.1.due.total_cmp(&b.1.due).then(a.0.cmp(b.0)))
                    .map(|(id, t)| (*id, t.owner)))
            })
            .ok()
            .flatten();
            let Some((id, owner)) = due else { break };
            let f = with_state(&self.lua, |s| {
                let t = s.timers.get_mut(&id).unwrap();
                let f = self.lua.registry_value::<Function>(&t.func)?;
                match t.interval {
                    Some(i) => t.due += i,
                    None => {
                        s.timers.remove(&id);
                    }
                }
                Ok(f)
            });
            let Ok(f) = f else { break };
            if self.mods[owner].failed {
                continue;
            }
            match self.call(owner, f, ()) {
                // A repeating timer returning false cancels itself.
                Ok(Value::Boolean(false)) => {
                    let _ = with_state(&self.lua, |s| {
                        s.timers.remove(&id);
                        Ok(())
                    });
                }
                Ok(_) => {}
                Err(e) => {
                    self.report(owner, "timer", None, e);
                    let _ = with_state(&self.lua, |s| {
                        s.timers.remove(&id);
                        Ok(())
                    });
                }
            }
        }
        self.drain_fired();
    }

    /// Advance modifiers by `dt` and run their Lua callbacks.
    pub fn update_modifiers(&mut self, dt: f64) {
        let _ = with_state(&self.lua, |s| {
            let defs = s.defs.clone();
            let mut events = std::mem::take(&mut s.modifier_events);
            s.modifiers.update(&defs, dt, &mut events);
            s.modifier_events = events;
            Ok(())
        });
        self.run_modifier_events();
    }

    fn run_modifier_events(&mut self) {
        loop {
            let events = with_state(&self.lua, |s| Ok(std::mem::take(&mut s.modifier_events)))
                .unwrap_or_default();
            if events.is_empty() {
                break;
            }
            let defs = with_state(&self.lua, |s| Ok(s.defs.clone())).unwrap();
            for event in events {
                let (m, callback) = match &event {
                    ModifierEvent::Created(m) => (m, "OnCreated"),
                    ModifierEvent::Think(m) => (m, "OnIntervalThink"),
                    ModifierEvent::Destroyed(m) => (m, "OnDestroy"),
                };
                let Some(def) = defs.modifiers.get(&m.modifier) else {
                    continue;
                };
                let Handler::Lua { file, .. } = &def.handler else {
                    continue;
                };
                let ctx = json!({
                    "modifier": m.modifier.as_str(),
                    "instance": m.id,
                    "target": convert::entity_json(m.target),
                    "caster": m.caster.map(convert::entity_json),
                    "stacks": m.stacks,
                    "remaining": m.remaining,
                });
                let handler = Handler::Lua {
                    file: file.clone(),
                    function: callback.into(),
                };
                if let Err(e) = self.call_handler(&handler, &ctx, Some(m.target), true) {
                    self.errors.push(e);
                }
            }
        }
        self.drain_fired();
    }

    fn drain_fired(&mut self) {
        for _ in 0..16 {
            let fired =
                with_state(&self.lua, |s| Ok(std::mem::take(&mut s.fired))).unwrap_or_default();
            if fired.is_empty() {
                return;
            }
            for (_, name, payload) in fired {
                self.dispatch(&name, &payload, false);
            }
        }
        let _ = with_state(&self.lua, |s| {
            s.fired.clear();
            Ok(())
        });
        self.errors.push(ScriptError {
            owner: "engine".into(),
            callback: "Events.Fire".into(),
            entity: None,
            message: "event cascade deeper than 16 rounds was cut".into(),
        });
    }

    /// Call a definition's Lua handler with a context table. The handler
    /// file runs once in its own environment over its owning mod's. A
    /// missing function is an error unless `optional`.
    pub fn call_handler(
        &mut self,
        handler: &Handler,
        ctx: &Json,
        entity: Option<EntityRef>,
        optional: bool,
    ) -> Result<Json, ScriptError> {
        let Handler::Lua { file, function } = handler else {
            return Ok(Json::Null);
        };
        let fail = |owner: &str, message: String| ScriptError {
            owner: owner.into(),
            callback: format!("{file}:{function}"),
            entity,
            message,
        };
        let path = LogicalPath::new(&format!("scripts/vscripts/{file}"))
            .map_err(|e| fail("engine", e.to_string()))?;
        let layer = with_state(&self.lua, |s| Ok(s.set.vfs.resolve(&path).map(|r| r.layer)))
            .ok()
            .flatten()
            .ok_or_else(|| fail("engine", format!("{path} not found")))?;
        let owner_id = if layer == magicka_mods::BASE_ID {
            "base".to_string()
        } else {
            layer
        };
        let owner = self
            .mod_index(&owner_id)
            .ok_or_else(|| fail(&owner_id, "owner not loaded".into()))?;
        if self.mods[owner].failed {
            return Err(fail(&owner_id, "mod disabled after a load error".into()));
        }
        let key = (owner, path.to_string());
        if !self.handlers.contains_key(&key) {
            let env = (|| {
                let modenv: Table = self.lua.registry_value(&self.mods[owner].env)?;
                let env = self.lua.create_table()?;
                let meta = self.lua.create_table()?;
                meta.set("__index", modenv)?;
                env.set_metatable(Some(meta))?;
                let text = with_state(&self.lua, |s| {
                    s.set.vfs.read_string(&path).unwrap().map_err(err)
                })?;
                let f = self
                    .lua
                    .load(&text)
                    .set_name(format!("@{path}"))
                    .set_environment(env.clone())
                    .into_function()?;
                self.call(owner, f, ())?;
                self.lua.create_registry_value(env)
            })()
            .map_err(|e| fail(&owner_id, e.to_string()))?;
            self.handlers.insert(key.clone(), env);
        }
        let result = (|| {
            let env: Table = self.lua.registry_value(&self.handlers[&key])?;
            let Some(f) = env.get::<Option<Function>>(function.as_str())? else {
                return if optional {
                    Ok(Json::Null)
                } else {
                    Err(err(format!("{file} defines no function {function}")))
                };
            };
            let arg = to_lua(&self.lua, ctx)?;
            let v = self.call(owner, f, arg)?;
            from_lua(&v).or(Ok(Json::Null))
        })()
        .map_err(|e| fail(&owner_id, e.to_string()));
        self.drain_fired();
        result
    }

    /// Engine-side modifier application (item intrinsics, KV abilities).
    pub fn apply_modifier(
        &mut self,
        target: EntityRef,
        modifier: &DefinitionId,
        duration: Option<f64>,
        caster: Option<EntityRef>,
        owner: &str,
    ) -> Result<u64, String> {
        let r = with_state(&self.lua, |s| {
            let defs = s.defs.clone();
            let mut events = std::mem::take(&mut s.modifier_events);
            let r = s.modifiers.apply(
                &defs,
                target,
                modifier,
                duration,
                1,
                caster,
                owner,
                &mut events,
            );
            s.modifier_events = events;
            Ok(r)
        })
        .map_err(|e| e.to_string())?;
        self.run_modifier_events();
        r
    }
    pub fn remove_modifier(&mut self, target: EntityRef, modifier: &DefinitionId) {
        let _ = with_state(&self.lua, |s| {
            let mut events = std::mem::take(&mut s.modifier_events);
            s.modifiers.remove(target, modifier, &mut events);
            s.modifier_events = events;
            Ok(())
        });
        self.run_modifier_events();
    }
    /// Drop modifiers on entities the snapshot no longer contains.
    pub fn prune_modifiers(&mut self) {
        let _ = with_state(&self.lua, |s| {
            let live: BTreeSet<EntityRef> = s
                .snapshot
                .entities
                .iter()
                .filter_map(|e| e.entity)
                .collect();
            let mut events = std::mem::take(&mut s.modifier_events);
            s.modifiers
                .retain_targets(|t| live.contains(&t), &mut events);
            s.modifier_events = events;
            Ok(())
        });
        self.run_modifier_events();
    }
    pub fn modifiers(&self) -> ModifierStore {
        with_state(&self.lua, |s| Ok(s.modifiers.clone())).unwrap_or_default()
    }
    pub fn modifier_properties(&self, target: EntityRef) -> BTreeMap<String, f64> {
        with_state(&self.lua, |s| Ok(s.modifiers.properties(&s.defs, target))).unwrap_or_default()
    }

    /// Commands queued since the last call, in issue order.
    pub fn take_commands(&mut self) -> Vec<Issued> {
        with_state(&self.lua, |s| Ok(std::mem::take(&mut s.commands))).unwrap_or_default()
    }
    pub fn take_log(&mut self) -> Vec<String> {
        with_state(&self.lua, |s| Ok(std::mem::take(&mut s.log))).unwrap_or_default()
    }
    pub fn take_errors(&mut self) -> Vec<ScriptError> {
        std::mem::take(&mut self.errors)
    }

    /// Server-owned replicated mod UI state: every table, for baselines.
    pub fn net_tables(&self) -> BTreeMap<String, BTreeMap<String, Json>> {
        with_state(&self.lua, |s| {
            Ok(s.net_tables
                .iter()
                .map(|(t, m)| {
                    (
                        t.clone(),
                        m.iter().map(|(k, (_, v))| (k.clone(), v.clone())).collect(),
                    )
                })
                .collect())
        })
        .unwrap_or_default()
    }
    /// Tables changed since the last call (complete table contents).
    pub fn take_net_table_changes(&mut self) -> BTreeMap<String, BTreeMap<String, Json>> {
        let dirty =
            with_state(&self.lua, |s| Ok(std::mem::take(&mut s.net_dirty))).unwrap_or_default();
        let all = self.net_tables();
        dirty
            .into_iter()
            .map(|t| (t.clone(), all.get(&t).cloned().unwrap_or_default()))
            .collect()
    }

    /// Disable a mod: `OnModUnload`, then remove its subscriptions,
    /// filters, timers, net table entries and modifiers. Nothing it
    /// registered stays live.
    pub fn unload_mod(&mut self, id: &str) {
        let Some(owner) = self.mod_index(id) else {
            return;
        };
        if !self.mods[owner].failed {
            let result = (|| {
                let env: Table = self.lua.registry_value(&self.mods[owner].env)?;
                if let Some(f) = env.raw_get::<Option<Function>>("OnModUnload")? {
                    self.call(owner, f, ())?;
                }
                Ok::<_, mlua::Error>(())
            })();
            if let Err(e) = result {
                self.report(owner, "OnModUnload", None, e);
            }
        }
        self.mods[owner].failed = true;
        let _ = with_state(&self.lua, |s| {
            s.subscriptions.retain(|x| x.owner != owner);
            s.timers.retain(|_, t| t.owner != owner);
            for (table, entries) in &mut s.net_tables {
                let before = entries.len();
                entries.retain(|_, (o, _)| *o != owner);
                if entries.len() != before {
                    s.net_dirty.insert(table.clone());
                }
            }
            s.fired.retain(|(o, _, _)| *o != owner);
            let name = s.owners[owner].clone();
            s.commands.retain(|c| c.owner != name);
            let mut events = std::mem::take(&mut s.modifier_events);
            s.modifiers.remove_owner(&name, &mut events);
            s.modifier_events = events;
            Ok(())
        });
        self.handlers.retain(|(o, _), _| *o != owner);
        self.run_modifier_events();
    }
    /// Counts of live registrations owned by a mod (diagnostics, tests).
    pub fn owned(&self, id: &str) -> (usize, usize, usize, usize) {
        let Some(owner) = self.mod_index(id) else {
            return (0, 0, 0, 0);
        };
        with_state(&self.lua, |s| {
            Ok((
                s.owned_count(owner, false),
                s.owned_count(owner, true),
                s.net_tables
                    .values()
                    .map(|t| t.values().filter(|(o, _)| *o == owner).count())
                    .sum(),
                s.modifiers
                    .active()
                    .iter()
                    .filter(|m| m.owner == s.owners[owner])
                    .count(),
            ))
        })
        .unwrap_or_default()
    }

    /// Serializable mod state for saves: each mod's `SaveState()` result,
    /// net tables and modifiers. Closures and VM objects are never saved.
    pub fn save_state(&mut self) -> Json {
        let mut mods = serde_json::Map::new();
        for owner in 0..self.mods.len() {
            if self.mods[owner].failed {
                continue;
            }
            let result = (|| {
                let env: Table = self.lua.registry_value(&self.mods[owner].env)?;
                let Some(f) = env.raw_get::<Option<Function>>("SaveState")? else {
                    return Ok(None);
                };
                let v = self.call(owner, f, ())?;
                Ok(Some(from_lua(&v)?))
            })();
            match result {
                Ok(Some(v)) => {
                    mods.insert(self.mods[owner].id.clone(), v);
                }
                Ok(None) => {}
                Err(e) => self.report(owner, "SaveState", None, e),
            }
        }
        let modifiers = self.modifiers();
        let tables = self.net_tables();
        json!({ "api": API_VERSION, "mods": mods, "net_tables": tables, "modifiers": modifiers })
    }
    /// Restore [`ScriptHost::save_state`] output: `LoadState(state)` per mod.
    pub fn load_state(&mut self, state: &Json) {
        if let Ok(store) = serde_json::from_value::<ModifierStore>(state["modifiers"].clone()) {
            let _ = with_state(&self.lua, |s| {
                s.modifiers = store;
                Ok(())
            });
        }
        if let Some(tables) = state["net_tables"].as_object() {
            let owners: Vec<String> = self.mod_ids();
            let _ = with_state(&self.lua, |s| {
                for (table, entries) in tables {
                    let Some(entries) = entries.as_object() else {
                        continue;
                    };
                    for (k, v) in entries {
                        // Ownership follows the table name's mod prefix.
                        let owner = owners
                            .iter()
                            .position(|o| table.starts_with(o.as_str()))
                            .unwrap_or(0);
                        s.net_tables
                            .entry(table.clone())
                            .or_default()
                            .insert(k.clone(), (owner, v.clone()));
                    }
                    s.net_dirty.insert(table.clone());
                }
                Ok(())
            });
        }
        for owner in 0..self.mods.len() {
            let Some(saved) = state["mods"].get(&self.mods[owner].id).cloned() else {
                continue;
            };
            if self.mods[owner].failed {
                continue;
            }
            let result = (|| {
                let env: Table = self.lua.registry_value(&self.mods[owner].env)?;
                if let Some(f) = env.raw_get::<Option<Function>>("LoadState")? {
                    let arg = to_lua(&self.lua, &saved)?;
                    self.call(owner, f, arg)?;
                }
                Ok::<_, mlua::Error>(())
            })();
            if let Err(e) = result {
                self.report(owner, "LoadState", None, e);
            }
        }
    }

    pub fn used_memory(&self) -> usize {
        self.lua.used_memory()
    }
}

/// A full author check of a mod set (`magicka-modtool validate`).
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub summary: Vec<String>,
}

/// Validate everything a match would load: definitions, every Lua file's
/// syntax, and the server VM booting each mod (`OnModLoad`).
pub fn validate(set: Arc<ModSet>) -> Report {
    let mut report = Report::default();
    let defs = match Definitions::load(&set) {
        Ok(d) => d,
        Err(e) => {
            report.errors.extend(e.0);
            return report;
        }
    };
    report.warnings.extend(defs.warnings.iter().cloned());
    let checker = Lua::new_with(StdLib::NONE, LuaOptions::default());
    if let Ok(checker) = &checker {
        for path in set.vfs.list("scripts/vscripts") {
            if path.extension() != Some("lua") {
                continue;
            }
            match set.vfs.read_string(&path) {
                Some(Ok(text)) => {
                    if let Err(e) = checker
                        .load(&text)
                        .set_name(format!("@{path}"))
                        .into_function()
                    {
                        report.errors.push(format!("{path}: {e}"));
                    }
                }
                Some(Err(e)) => report.errors.push(e),
                None => {}
            }
        }
    }
    match ScriptHost::new(set.clone(), Arc::new(defs.clone()), Limits::default(), 1) {
        Ok(mut host) => report
            .errors
            .extend(host.take_errors().iter().map(|e| e.to_string())),
        Err(e) => report.errors.push(e.to_string()),
    }
    report.summary.push(format!(
        "{} mods; {} items, {} abilities, {} modifiers, {} magicks, {} spells, {} units, {} effects, {} levels, {} panels",
        set.mods.len(),
        defs.items.len(),
        defs.abilities.len(),
        defs.modifiers.len(),
        defs.magicks.len(),
        defs.spells.len(),
        defs.units.len(),
        defs.effects.len(),
        defs.levels.len(),
        defs.panels.len()
    ));
    match set.content_manifest() {
        Ok(m) => report.summary.push(format!(
            "rules hash: {}",
            m.rules_hash
                .map(|h| magicka_mods::sha256_hex(format!("{h}{}", defs.rules_hash()).as_bytes()))
                .unwrap_or_else(|| "none (unmodded)".into())
        )),
        Err(e) => report.errors.push(e),
    }
    report
}

/// `require("a.b")`: `scripts/vscripts/a/b.lua` through the VFS, cached
/// per mod, cycles rejected.
fn require(lua: &Lua, name: &str) -> mlua::Result<Value> {
    if name.is_empty() || name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err(err(format!("invalid module name {name:?}")));
    }
    // Script folders are named after the mod Id, which usually contains
    // dots: "<own id>.a.b" is the requiring mod's own `a/b.lua`.
    let own = with_state(lua, |s| Ok(s.owners[s.owner()?].clone()))?;
    let relative = match name.strip_prefix(&format!("{own}.")) {
        Some(rest) => format!("{own}/{}", rest.replace('.', "/")),
        None => name.replace('.', "/"),
    };
    let path = LogicalPath::new(&format!("scripts/vscripts/{relative}.lua"))
        .map_err(|e| err(e.to_string()))?;
    let (owner, cached) = with_state(lua, |s| {
        let owner = s.owner()?;
        let key = (owner, path.to_string());
        let cached = match s.loaded.get(&key) {
            Some(k) => Some(lua.registry_value::<Value>(k)?),
            None => None,
        };
        if cached.is_none() && !s.loading.insert(key) {
            return Err(err(format!("require cycle through {name}")));
        }
        Ok((owner, cached))
    })?;
    if let Some(v) = cached {
        return Ok(v);
    }
    let result = (|| {
        let text = with_state(lua, |s| {
            s.set
                .vfs
                .read_string(&path)
                .ok_or_else(|| err(format!("module {name} not found ({path})")))?
                .map_err(err)
        })?;
        // Modules run in the requiring mod's environment.
        let env: Table = lua.named_registry_value(&format!("magicka.env.{owner}"))?;
        let f = lua
            .load(&text)
            .set_name(format!("@{path}"))
            .set_environment(env)
            .into_function()?;
        let v: Value = f.call(())?;
        Ok(if v.is_nil() { Value::Boolean(true) } else { v })
    })();
    with_state(lua, |s| {
        let key = (owner, path.to_string());
        s.loading.remove(&key);
        if let Ok(v) = &result {
            s.loaded.insert(key, lua.create_registry_value(v.clone())?);
        }
        Ok(())
    })?;
    result
}

mod api;
pub mod world_ops;
