//! Native dispenser actions and finite spawning state (research/dispensers.md).
use crate::{
    ai_event::{clr_enum, AiEvent},
    scene_script::XmlElement,
    script_attributes::{boolean, identifier, number, ScriptReadError},
    spawn_action::{SpawnLevel, ORDERS, REACT_TO},
};
use magicka_sim::{damage::Damage, spray_pulse::Segment};

pub const MODELS: [&str; 3] = [
    "Models/Effects/SpiderHole",
    "Models/Effects/GoblinTent",
    "Models/Effects/SpiderEggs",
];
pub const CLIPS: [&str; 4] = ["spawn", "execute", "idle", "destroyed"];
const KINDS: &[(&str, i32)] = &[
    ("spiderhole", 0),
    ("goblintent", 1),
    ("spidereggs", 2),
    ("nrofmodels", 3),
];

#[derive(Clone, Debug)]
pub enum DispenserCommand {
    Spawn {
        model: usize,
        area: i32,
        types: Option<Vec<String>>,
        amounts: Option<Vec<i32>>,
        interval: f32,
        active: bool,
        id: i32,
    },
    Activate {
        id: Option<i32>,
        model: usize,
        area: i32,
        node: XmlElement,
    },
}
#[derive(Clone, Debug)]
pub struct DispenserAction {
    pub delay: f32,
    pub command: DispenserCommand,
}
impl DispenserAction {
    pub fn read(node: &XmlElement) -> Result<Self, ScriptReadError> {
        let spawn = node.name.eq_ignore_ascii_case("spawndispenser");
        let (mut delay, mut model, mut area, mut id) = (0.0, 0, 0, None);
        let (mut types, mut amounts, mut interval, mut active) = (None, None, 0.0, true);
        for (k, v) in &node.attributes {
            match k.to_ascii_lowercase().as_str() {
                "delay" => delay = number(v, k)?,
                "model" if spawn => model = clr_enum(v, KINDS, k, i32::MAX)? as usize,
                "type" if !spawn => model = clr_enum(v, KINDS, k, i32::MAX)? as usize,
                "area" => area = identifier(v)?,
                "id" => {
                    id = if v.is_empty() {
                        None
                    } else {
                        Some(identifier(v)?)
                    }
                }
                "types" if spawn => {
                    types = Some(
                        v.to_ascii_lowercase()
                            .split(',')
                            .map(str::to_owned)
                            .collect(),
                    )
                }
                "amount" if spawn => {
                    amounts = Some(
                        v.split(',')
                            .map(|n| number(n, k))
                            .collect::<Result<Vec<_>, _>>()?,
                    )
                }
                "time" if spawn => interval = number(v, k)?,
                "active" if spawn => active = boolean(v, k)?,
                "order" if !spawn => {
                    clr_enum(v, ORDERS, k, 255)?;
                }
                "reactto" if !spawn => {
                    clr_enum(v, REACT_TO, k, 255)?;
                }
                _ => {}
            }
        }
        if !spawn {
            AiEvent::read_children(node)?;
        }
        Ok(Self {
            delay,
            command: if spawn {
                DispenserCommand::Spawn {
                    model,
                    area,
                    types,
                    amounts,
                    interval,
                    active,
                    id: id.unwrap_or(0),
                }
            } else {
                DispenserCommand::Activate {
                    id,
                    model,
                    area,
                    node: node.clone(),
                }
            },
        })
    }
    pub fn initialize<L: SpawnLevel + ?Sized>(
        &self,
        level: &mut L,
    ) -> Result<(), crate::ai_event::ResolveError<L::Error>> {
        match &self.command {
            DispenserCommand::Spawn {
                types: Some(types), ..
            } => {
                for name in types {
                    level
                        .load_character_template(Some(name))
                        .map_err(crate::ai_event::ResolveError::Level)?;
                }
            }
            DispenserCommand::Activate { node, .. } => {
                for child in &node.children {
                    AiEvent::resolve(child, level)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Dispenser {
    pub model: usize,
    pub types: Vec<String>,
    pub amounts: Vec<i32>,
    pub interval: f32,
    pub timer: f32,
    pub active: bool,
    pub health: f32,
    /// Body transform; render orientation lowers its translation by 1.75.
    pub transform: [f32; 16],
    pub durations: [Option<f32>; 4],
    pub clip: Option<usize>,
    pub clip_time: f32,
}
impl Dispenser {
    pub fn new(
        model: usize,
        transform: [f32; 16],
        types: Vec<String>,
        mut amounts: Vec<i32>,
        interval: f32,
        active: bool,
        durations: [Option<f32>; 4],
    ) -> Self {
        amounts.truncate(types.len());
        Self {
            model,
            types,
            amounts,
            interval,
            timer: 0.0,
            active,
            health: 500.0,
            transform,
            durations,
            clip: durations[0].map(|_| 0),
            clip_time: 0.0,
        }
    }
    pub fn position(&self) -> [f32; 3] {
        [self.transform[12], self.transform[13], self.transform[14]]
    }
    pub fn render_transform(&self) -> [f32; 16] {
        let mut m = self.transform;
        m[13] -= 1.75;
        m
    }
    pub fn finished(&self) -> bool {
        self.clip
            .and_then(|c| self.durations[c])
            .is_none_or(|d| self.clip_time >= d)
    }
    pub fn removable(&self) -> bool {
        self.health <= 0.0 && self.finished()
    }
    fn start(&mut self, clip: usize) {
        if self.durations[clip].is_some() {
            self.clip = Some(clip);
            self.clip_time = 0.0;
        }
    }
    /// Advance animation first; timers only run after an animation finishes.
    pub fn update(
        &mut self,
        dt: f32,
        choice: impl FnOnce(usize) -> usize,
    ) -> Result<Option<String>, String> {
        self.clip_time += dt;
        if !self.finished() || !self.active {
            return Ok(None);
        }
        self.timer -= dt;
        if self.timer > 0.0 {
            return Ok(None);
        }
        self.timer = self.interval;
        let result = self.spawn(choice)?;
        if result.is_none() {
            self.health = 0.0;
            self.start(3);
        }
        Ok(result)
    }
    fn spawn(&mut self, choice: impl FnOnce(usize) -> usize) -> Result<Option<String>, String> {
        if self.types.is_empty() {
            return Ok(None);
        }
        if self.amounts.len() != 1 {
            let mut i = 0;
            while i < self.amounts.len() {
                if self.amounts[i] == 0 {
                    self.amounts.remove(i);
                    if i >= self.types.len() {
                        return Err("dispenser type/amount index mismatch".into());
                    }
                    self.types.remove(i);
                } else {
                    i += 1;
                }
            }
        }
        if self.types.is_empty() {
            return Err("dispenser native Random.Next/index on empty type list".into());
        }
        let index = choice(self.types.len());
        // Native's count >= index comparison permits the equality case to
        // reach an out-of-range index; do not repair mismatched author arrays.
        let amount = if self.amounts.len() >= index {
            *self
                .amounts
                .get(index)
                .ok_or("dispenser native amount index outside authored array")?
        } else if self.amounts.len() == 1 {
            self.amounts[0]
        } else {
            return Ok(None);
        };
        if amount <= 0 {
            return Ok(None);
        }
        let name = self.types[index].clone();
        self.start(1);
        let remaining = self
            .amounts
            .get_mut(index)
            .ok_or("dispenser native decrement index outside authored array")?;
        *remaining -= 1;
        Ok(Some(name))
    }
    /// Identity resistances are constructed for all eleven elements. Native
    /// damage truncates Amount*Magnitude, regardless of attack-property flags.
    pub fn damage(&mut self, damage: Damage, enabled: bool) -> u16 {
        let amount = (damage.amount * damage.magnitude) as i32 as f32;
        if enabled {
            self.health -= amount;
        }
        if amount <= 0.0 {
            256
        } else {
            1
        }
    }
    /// Primitive-local Down position and Forward*Length, transformed by body.
    pub fn axis(&self) -> Segment {
        let p = self.position();
        Segment {
            origin: std::array::from_fn(|i| p[i] - self.transform[4 + i]),
            delta: [self.transform[4], self.transform[5], self.transform[6]],
        }
    }
    // `!(d > r²)` transcribes the native comparison, NaN included.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    pub fn segment(&self, seg: Segment, radius: f32) -> Option<[f32; 3]> {
        let axis = self.axis();
        let (d, a, _) = magicka_sim::geometry::jiglib_segment_segment(
            axis.origin,
            axis.delta,
            seg.origin,
            seg.delta,
        );
        // Native IL_00e6..00f0 overwrites the earlier surface calculation
        // with Origin + (Origin + Delta * axisParameter).
        (!(d > (1.25 + radius).powi(2)))
            .then(|| std::array::from_fn(|i| axis.origin[i] + (axis.origin[i] + axis.delta[i] * a)))
    }
}
