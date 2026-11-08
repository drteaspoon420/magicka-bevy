//! Lua values ↔ JSON values: the only structured data that crosses the VM
//! boundary (events, filters, net tables, saved mod state). Functions,
//! userdata other than entities, cycles and non-string/integer keys are
//! rejected rather than silently dropped.
use crate::types::EntityRef;
use mlua::{Lua, Result, Table, Value};
use serde_json::{Map, Number, Value as Json};

pub const MAX_DEPTH: usize = 32;

pub fn to_lua(lua: &Lua, json: &Json) -> Result<Value> {
    Ok(match json {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Integer(i),
            None => Value::Number(n.as_f64().unwrap_or(0.0)),
        },
        Json::String(s) => Value::String(lua.create_string(s)?),
        Json::Array(items) => {
            let t = lua.create_table_with_capacity(items.len(), 0)?;
            for (i, v) in items.iter().enumerate() {
                t.raw_set(i + 1, to_lua(lua, v)?)?;
            }
            Value::Table(t)
        }
        Json::Object(map) => {
            // An entity reference keeps its userdata identity.
            if let (Some(h), Some(g), 2) = (
                map.get("__entity").and_then(Json::as_u64),
                map.get("generation").and_then(Json::as_u64),
                map.len(),
            ) {
                return Ok(Value::UserData(lua.create_userdata(EntityRef {
                    handle: h as u16,
                    generation: g as u32,
                })?));
            }
            let t = lua.create_table_with_capacity(0, map.len())?;
            for (k, v) in map {
                t.raw_set(k.as_str(), to_lua(lua, v)?)?;
            }
            Value::Table(t)
        }
    })
}

pub fn entity_json(e: EntityRef) -> Json {
    serde_json::json!({ "__entity": e.handle, "generation": e.generation })
}

pub fn from_lua(value: &Value) -> Result<Json> {
    from_lua_at(value, 0)
}

fn from_lua_at(value: &Value, depth: usize) -> Result<Json> {
    if depth > MAX_DEPTH {
        return Err(mlua::Error::RuntimeError(format!(
            "value nests deeper than {MAX_DEPTH} (cycle?)"
        )));
    }
    Ok(match value {
        Value::Nil => Json::Null,
        Value::Boolean(b) => Json::Bool(*b),
        Value::Integer(i) => Json::Number((*i).into()),
        Value::Number(n) => Number::from_f64(*n)
            .map(Json::Number)
            .ok_or_else(|| mlua::Error::RuntimeError("numbers must be finite".into()))?,
        Value::String(s) => Json::String(s.to_str()?.to_string()),
        Value::Table(t) => table(t, depth)?,
        Value::UserData(u) => match u.borrow::<EntityRef>() {
            Ok(e) => entity_json(*e),
            Err(_) => return Err(mlua::Error::RuntimeError("unsupported userdata".into())),
        },
        other => {
            return Err(mlua::Error::RuntimeError(format!(
                "a {} cannot be stored or sent",
                other.type_name()
            )))
        }
    })
}

fn table(t: &Table, depth: usize) -> Result<Json> {
    let len = t.raw_len();
    let mut count = 0;
    let mut map = Map::new();
    let mut array = true;
    for pair in t.clone().pairs::<Value, Value>() {
        let (k, v) = pair?;
        count += 1;
        let key = match &k {
            Value::Integer(i) => {
                if *i < 1 || *i as usize > len {
                    array = false;
                }
                i.to_string()
            }
            Value::String(s) => {
                array = false;
                s.to_str()?.to_string()
            }
            other => {
                return Err(mlua::Error::RuntimeError(format!(
                    "table keys must be strings or integers, not {}",
                    other.type_name()
                )))
            }
        };
        map.insert(key, from_lua_at(&v, depth + 1)?);
    }
    if array && count == len && len > 0 {
        let mut out = Vec::with_capacity(len);
        for i in 1..=len {
            out.push(map.remove(&i.to_string()).unwrap_or(Json::Null));
        }
        return Ok(Json::Array(out));
    }
    Ok(Json::Object(map))
}
