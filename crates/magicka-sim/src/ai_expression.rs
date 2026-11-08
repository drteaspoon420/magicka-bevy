//! Magicka.AI.Arithmetics (ability desirability expressions) and the pure
//! parts of FuzzyMath. Parsing follows Expression.Read statement for
//! statement, including the CLR 2.0 quicksort that orders operators of
//! equal priority. World queries are delegated to [`ExpressionHost`].
//! See research/npc-combat.md.

/// Operator priorities (Expression.sPriorities).
fn priority(c: u8) -> Option<u8> {
    match c {
        b'^' => Some(200),
        b'!' => Some(150),
        b'*' | b'/' => Some(100),
        b'+' | b'-' => Some(50),
        _ => None,
    }
}

/// Variable.VariableType names (Enum.GetNames includes the aliases).
const VARIABLE_TYPES: [(&str, u8); 42] = [
    ("DistanceLinear", 0),
    ("DistLin", 0),
    ("DistanceExponential", 1),
    ("DistExp", 1),
    ("Behind", 2),
    ("Danger", 3),
    ("HasStatus", 4),
    ("Status", 4),
    ("Gripped", 5),
    ("Gripping", 6),
    ("Entangled", 7),
    ("Speed", 8),
    ("Health", 9),
    ("FriendlyDensity", 10),
    ("FDensity", 10),
    ("EnemyDensity", 11),
    ("EDensity", 11),
    ("Threat", 12),
    ("LOS", 13),
    ("LineOfSight", 13),
    ("IsCharacter", 14),
    ("Character", 14),
    ("IsC", 14),
    ("IsEthereal", 15),
    ("Ethereal", 15),
    ("Resistance", 16),
    ("Res", 16),
    ("Shielded", 17),
    ("LOSC", 18),
    ("LineOfSightCollision", 18),
    ("IsShield", 19),
    ("IsS", 19),
    ("Shield", 19),
    ("IsBarrier", 20),
    ("IsB", 20),
    ("Barrier", 20),
    ("GripDamageAccumulation", 21),
    ("GripDmgAcc", 21),
    ("GripDamageAccumulationNormalized", 22),
    ("GripDmgAccNrm", 22),
    ("IsOnGround", 23),
    ("Angle", 24),
];
const SCOPES: [(&str, u8); 6] = [
    ("Global", 0),
    ("G", 0),
    ("Self", 1),
    ("S", 1),
    ("Target", 2),
    ("T", 2),
];
const ELEMENTS: [(&str, u32); 26] = [
    ("None", 0),
    ("Earth", 1),
    ("Physical", 1),
    ("Water", 2),
    ("Cold", 4),
    ("Fire", 8),
    ("Lightning", 0x10),
    ("Arcane", 0x20),
    ("Life", 0x40),
    ("Shield", 0x80),
    ("Ice", 0x100),
    ("Steam", 0x200),
    ("Poison", 0x400),
    ("Offensive", 0xFF3F),
    ("Defensive", 0xB0),
    ("All", 0xFFFF),
    ("Magick", 0xFFFF),
    ("Basic", 0xFF),
    ("Instant", 0x371),
    ("InstantPhysical", 0x171),
    ("InstantNonPhysical", 0x270),
    ("StatusEffects", 0x64E),
    ("ShieldElements", 0xE0),
    ("PhysicalElements", 0x101),
    ("Beams", 0x60),
    ("", 0),
];
const STATUS_EFFECTS: [(&str, u32); 13] = [
    ("None", 0),
    ("Burning", 1),
    ("Wet", 2),
    ("Frozen", 4),
    ("Cold", 8),
    ("Poisoned", 0x10),
    ("Healing", 0x20),
    ("Life", 0x20),
    ("Greased", 0x40),
    ("Steamed", 0x80),
    ("Bleeding", 0x100),
    ("NumberOfTypes", 0x200),
    ("", 0),
];

#[derive(Clone, Debug, PartialEq)]
pub enum ExprError {
    /// Expression.Read "Invalid syntax!" (a '(' inside function arguments).
    Syntax(String),
    /// Enum.Parse/float.Parse failures and index errors in Variable(string).
    Variable(String),
    /// GetValue's `throw new Exception()` for an invalid scope.
    Scope(VariableKind, Scope),
}

/// Enum.Parse(type, value, ignoreCase: true): comma-separated names or
/// integers, ORed; surrounding whitespace allowed.
fn enum_parse(table: &[(&str, u32)], text: &str) -> Option<u32> {
    let mut value = 0u32;
    let mut any = false;
    for part in text.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return None;
        }
        let v = if let Ok(n) = part.parse::<i64>() {
            n as u32
        } else {
            table
                .iter()
                .find(|(name, _)| !name.is_empty() && name.eq_ignore_ascii_case(part))?
                .1
        };
        value |= v;
        any = true;
    }
    any.then_some(value)
}

/// float.Parse(InvariantCulture) through double, as .NET Framework does.
fn parse_float(text: &str) -> Option<f32> {
    let t = text.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("inf") || t.eq_ignore_ascii_case("nan") {
        return None;
    }
    t.parse::<f64>().ok().map(|v| v as f32)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Global,
    SelfScope,
    Target,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariableKind {
    DistanceLinear,
    DistanceExponential,
    Behind,
    Danger,
    HasStatus,
    Gripped,
    Gripping,
    Entangled,
    Speed,
    Health,
    FriendlyDensity,
    EnemyDensity,
    Threat,
    LineOfSight,
    IsCharacter,
    IsEthereal,
    Resistance,
    Shielded,
    LineOfSightCollision,
    IsShield,
    IsBarrier,
    GripDamageAccumulation,
    GripDamageAccumulationNormalized,
    IsOnGround,
    Angle,
    /// A numeric or ORed value outside the named members (GetValue default).
    Other(u32),
}

impl VariableKind {
    fn from_value(v: u32) -> Self {
        use VariableKind::*;
        match v {
            0 => DistanceLinear,
            1 => DistanceExponential,
            2 => Behind,
            3 => Danger,
            4 => HasStatus,
            5 => Gripped,
            6 => Gripping,
            7 => Entangled,
            8 => Speed,
            9 => Health,
            10 => FriendlyDensity,
            11 => EnemyDensity,
            12 => Threat,
            13 => LineOfSight,
            14 => IsCharacter,
            15 => IsEthereal,
            16 => Resistance,
            17 => Shielded,
            18 => LineOfSightCollision,
            19 => IsShield,
            20 => IsBarrier,
            21 => GripDamageAccumulation,
            22 => GripDamageAccumulationNormalized,
            23 => IsOnGround,
            24 => Angle,
            other => Other(other),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Variable {
    pub scope: Scope,
    pub kind: VariableKind,
    pub a1: f32,
    pub a2: f32,
    pub a3: f32,
    pub a4: f32,
    pub status: u32,
    pub elements: u32,
}

impl Variable {
    /// Variable(string).
    pub fn read(text: &str) -> Result<Self, ExprError> {
        let bad = || ExprError::Variable(text.to_owned());
        let compact = text.replace(' ', "");
        let mut array: Vec<String> = compact
            .split([',', '/', '(', ')'])
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        if array.is_empty() {
            return Err(bad());
        }
        let mut num = 0;
        let mut scope = Scope::Global;
        let parts: Vec<&str> = array[0].split('.').collect();
        let head = parts[0].replace(['\n', '\t', '\r'], "");
        if SCOPES.iter().any(|(n, _)| n.eq_ignore_ascii_case(&head)) {
            let table: Vec<(&str, u32)> = SCOPES.iter().map(|&(n, v)| (n, u32::from(v))).collect();
            scope = match enum_parse(&table, parts[0]).ok_or_else(bad)? {
                0 => Scope::Global,
                1 => Scope::SelfScope,
                2 => Scope::Target,
                _ => return Err(bad()),
            };
            let second = parts.get(1).ok_or_else(bad)?.to_string();
            let mut rebuilt = vec![parts[0].to_owned(), second];
            rebuilt.extend(array.iter().skip(1).cloned());
            array = rebuilt;
            num += 1;
        }
        let types: Vec<(&str, u32)> = VARIABLE_TYPES
            .iter()
            .map(|&(n, v)| (n, u32::from(v)))
            .collect();
        let kind = VariableKind::from_value(
            enum_parse(&types, array.get(num).ok_or_else(bad)?).ok_or_else(bad)?,
        );
        num += 1;
        let mut v = Self {
            scope,
            kind,
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            a4: 0.0,
            status: 0,
            elements: 0,
        };
        let arg = |i: usize| array.get(i).ok_or_else(bad);
        let float = |i: usize| parse_float(arg(i)?).ok_or_else(bad);
        match kind {
            VariableKind::DistanceLinear | VariableKind::DistanceExponential => {
                v.a2 = float(num)?;
                num += 1;
                if num < array.len() {
                    v.a1 = v.a2;
                    v.a2 = float(num)?;
                } else {
                    v.a1 = -v.a2;
                }
            }
            VariableKind::FriendlyDensity | VariableKind::EnemyDensity => {
                v.a1 = if num < array.len() { float(num)? } else { 6.0 };
            }
            VariableKind::Danger | VariableKind::Threat => {
                v.elements = enum_parse(&ELEMENTS, &arg(num)?.replace('|', ",")).ok_or_else(bad)?;
            }
            VariableKind::HasStatus => {
                v.status = enum_parse(&STATUS_EFFECTS, arg(num)?).ok_or_else(bad)?;
            }
            VariableKind::LineOfSightCollision => {
                // The length tests count the scope prefix element, as written.
                v.a4 = -1.0;
                if array.len() > 1 {
                    v.a1 = float(num)?;
                    num += 1;
                    if array.len() > 2 {
                        v.a2 = float(num)?;
                        num += 1;
                        if array.len() > 3 {
                            v.a3 = float(num)?;
                            num += 1;
                            if array.len() > 4 {
                                v.a4 = float(num)?;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(v)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Constant(f32),
    Variable(Variable),
    Pow(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
}

#[derive(Clone, Copy, Debug)]
struct Operator {
    value: u8,
    position: usize,
    priority: u16,
}
impl Operator {
    fn compare(&self, other: &Self) -> i32 {
        i32::from(self.priority) - i32::from(other.priority)
    }
}

/// CLR 2.0 `GenericArraySortHelper<T>.QuickSort` (median of three, then
/// Hoare partition), which List<T>.Sort uses for an IComparable<T> struct.
/// INFERRED from the .NET Framework reference source (the BCL is not among
/// the target files); it fixes the order of equal-priority operators.
fn clr_quicksort(keys: &mut [Operator], mut left: isize, mut right: isize) {
    fn swap_if_greater(keys: &mut [Operator], a: isize, b: isize) {
        if a != b && keys[a as usize].compare(&keys[b as usize]) > 0 {
            keys.swap(a as usize, b as usize);
        }
    }
    loop {
        let mut i = left;
        let mut j = right;
        let middle = i + ((j - i) >> 1);
        swap_if_greater(keys, i, middle);
        swap_if_greater(keys, i, j);
        swap_if_greater(keys, middle, j);
        let x = keys[middle as usize];
        loop {
            while x.compare(&keys[i as usize]) > 0 {
                i += 1;
            }
            while x.compare(&keys[j as usize]) < 0 {
                j -= 1;
            }
            if i > j {
                break;
            }
            if i < j {
                keys.swap(i as usize, j as usize);
            }
            i += 1;
            j -= 1;
            if i > j {
                break;
            }
        }
        if j - left <= right - i {
            if left < j {
                clr_quicksort(keys, left, j);
            }
            left = i;
        } else {
            if i < right {
                clr_quicksort(keys, i, right);
            }
            right = j;
        }
        if left >= right {
            break;
        }
    }
}

/// IsFunction: does the text ending at `end` (exclusive) end with a
/// VariableType name (case-insensitive)?
fn is_function(text: &[u8], end: usize) -> bool {
    VARIABLE_TYPES.iter().any(|(name, _)| {
        end >= name.len() && text[end - name.len()..end].eq_ignore_ascii_case(name.as_bytes())
    })
}

impl Expr {
    /// Expression.Read.
    pub fn read(text: &str) -> Result<Self, ExprError> {
        let bytes = text.as_bytes();
        let mut operators = Vec::new();
        let mut depth: u8 = 0;
        let mut in_function = false;
        for (i, &c) in bytes.iter().enumerate() {
            match c {
                b'(' => {
                    if in_function {
                        return Err(ExprError::Syntax(text.to_owned()));
                    }
                    if is_function(bytes, i) {
                        in_function = true;
                    }
                    depth = depth.wrapping_add(1);
                }
                b')' => {
                    in_function = false;
                    depth = depth.wrapping_sub(1);
                }
                _ => {
                    if !in_function {
                        if let Some(p) = priority(c) {
                            operators.push(Operator {
                                value: c,
                                position: i,
                                priority: (u16::from(depth) << 8) | u16::from(p),
                            });
                        }
                    }
                }
            }
        }
        if operators.len() > 1 {
            let n = operators.len() as isize;
            clr_quicksort(&mut operators, 0, n - 1);
        }
        Self::read_range(text, &mut operators, 0, text.len())
    }

    fn read_range(
        text: &str,
        operators: &mut Vec<Operator>,
        start: usize,
        count: usize,
    ) -> Result<Self, ExprError> {
        let found = operators
            .iter()
            .position(|o| o.position >= start && o.position < start + count);
        let Some(index) = found else {
            let leaf = &text[start..start + count];
            let stripped = leaf.replace(['(', ')'], "");
            let first = stripped.trim().split(' ').next().unwrap_or("");
            return Ok(match parse_float(first) {
                Some(v) => Expr::Constant(v),
                None => Expr::Variable(Variable::read(leaf)?),
            });
        };
        let op = operators.remove(index);
        let left_count = op.position - start;
        let right_start = op.position + 1;
        let right_count = count - (right_start - start);
        if op.value == b'!' {
            return Ok(Expr::Not(Box::new(Self::read_range(
                text,
                operators,
                right_start,
                right_count,
            )?)));
        }
        let a = Box::new(Self::read_range(text, operators, start, left_count)?);
        let b = Box::new(Self::read_range(text, operators, right_start, right_count)?);
        Ok(match op.value {
            b'^' => Expr::Pow(a, b),
            b'*' => Expr::Mul(a, b),
            b'/' => Expr::Div(a, b),
            b'+' => Expr::Add(a, b),
            _ => Expr::Sub(a, b),
        })
    }

    /// GetValue.
    pub fn value(
        &self,
        args: &ExpressionArgs,
        host: &mut dyn ExpressionHost,
    ) -> Result<f32, ExprError> {
        Ok(match self {
            Expr::Constant(v) => *v,
            Expr::Variable(v) => v.value(args, host)?,
            Expr::Pow(a, b) => {
                let (a, b) = (a.value(args, host)?, b.value(args, host)?);
                f64::from(a).powf(f64::from(b)) as f32
            }
            Expr::Not(a) => 1.0 - a.value(args, host)?,
            Expr::Mul(a, b) => a.value(args, host)? * b.value(args, host)?,
            Expr::Div(a, b) => a.value(args, host)? / b.value(args, host)?,
            Expr::Add(a, b) => a.value(args, host)? + b.value(args, host)?,
            Expr::Sub(a, b) => a.value(args, host)? - b.value(args, host)?,
        })
    }
}

pub type Vec3 = [f32; 3];

/// ExpressionArguments (geometry; the agent and target live in the host).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ExpressionArgs {
    pub delta: Vec3,
    pub delta_normalized: Vec3,
    pub distance: f32,
    pub ai_position: Vec3,
    pub target_position: Vec3,
    pub ai_direction: Vec3,
    pub target_direction: Vec3,
}

impl ExpressionArgs {
    /// NewExpressionArguments: AIDir is the owner's Direction and TargetDir
    /// the target body's orientation Forward.
    pub fn new(
        ai_position: Vec3,
        ai_direction: Vec3,
        target_position: Vec3,
        target_forward: Vec3,
    ) -> Self {
        let delta = std::array::from_fn(|i| target_position[i] - ai_position[i]);
        let distance = length(delta);
        Self {
            delta,
            delta_normalized: delta.map(|d| d / distance),
            distance,
            ai_position,
            target_position,
            ai_direction,
            target_direction: target_forward,
        }
    }
}

fn length(v: Vec3) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// Which character a query is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    Owner,
    Target,
}

/// World facts the variables read. `None` means the target is not of the
/// type the original casts to (the variable then yields its default).
pub trait ExpressionHost {
    fn has_status(&mut self, who: Subject, status: u32) -> Option<bool>;
    fn gripped(&mut self, who: Subject) -> Option<bool>;
    fn gripping(&mut self, who: Subject) -> Option<bool>;
    fn entangled(&mut self, who: Subject) -> Option<bool>;
    /// |CharacterBody.Movement|.
    fn speed(&mut self, who: Subject) -> Option<f32>;
    /// HitPoints and MaxHitPoints.
    fn health(&mut self, who: Subject) -> (f32, f32);
    /// Characters within `range` of the subject, other than it, whose
    /// faction does (friendly) or does not intersect the owner's.
    fn density_count(&mut self, around: Subject, friendly: bool, range: f32) -> u32;
    fn danger(&mut self, elements: u32) -> f32;
    fn threat(&mut self, elements: u32) -> Option<f32>;
    /// Variable LOS: no other entity blocks the owner→target segment.
    fn line_of_sight(&mut self, origin: Vec3, delta: Vec3, distance: f32) -> bool;
    /// Scene segment intersection (LOSC).
    fn scene_hit(&mut self, origin: Vec3, delta: Vec3) -> bool;
    fn is_character(&mut self) -> bool;
    fn ethereal(&mut self, who: Subject) -> Option<bool>;
    /// IDamageable.ResistanceAgainst.
    fn resistance_against(&mut self, who: Subject, elements: u32) -> Option<f32>;
    fn self_shielded(&mut self, who: Subject) -> Option<bool>;
    fn is_shield(&mut self) -> bool;
    fn is_barrier(&mut self) -> bool;
    /// GripDamageAccumulation and HitTolerance of a gripping owner.
    fn grip_damage(&mut self) -> Option<(f32, f32)>;
    fn on_ground(&mut self, who: Subject) -> Option<bool>;
}

impl Variable {
    fn subject(&self) -> Option<Subject> {
        match self.scope {
            Scope::SelfScope => Some(Subject::Owner),
            Scope::Target => Some(Subject::Target),
            Scope::Global => None,
        }
    }

    pub fn value(
        &self,
        a: &ExpressionArgs,
        host: &mut dyn ExpressionHost,
    ) -> Result<f32, ExprError> {
        use VariableKind as K;
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        let scope_error = || ExprError::Scope(self.kind, self.scope);
        Ok(match self.kind {
            K::DistanceLinear => fuzzy_distance_linear(a.distance, self.a1, self.a2),
            K::DistanceExponential => fuzzy_distance_exponential(a.distance, self.a1, self.a2),
            K::Behind => fuzzy_angle(a.delta_normalized, a.target_direction),
            K::Danger => host.danger(self.elements),
            K::Threat => match self.scope {
                Scope::Target => host.threat(self.elements).unwrap_or(0.0),
                _ => return Err(scope_error()),
            },
            K::HasStatus | K::Gripped | K::Gripping | K::Entangled => {
                let who = self.subject().ok_or_else(scope_error)?;
                let fact = match self.kind {
                    K::HasStatus => host.has_status(who, self.status),
                    K::Gripped => host.gripped(who),
                    K::Gripping => host.gripping(who),
                    _ => host.entangled(who),
                };
                flag(fact.unwrap_or(false))
            }
            K::Speed => {
                let who = self.subject().ok_or_else(scope_error)?;
                host.speed(who).unwrap_or(0.0)
            }
            K::Health => {
                let who = if self.scope == Scope::SelfScope {
                    Subject::Owner
                } else {
                    Subject::Target
                };
                let (hp, max) = host.health(who);
                hp / max
            }
            K::FriendlyDensity | K::EnemyDensity => {
                let around = if self.scope == Scope::SelfScope {
                    Subject::Owner
                } else {
                    Subject::Target
                };
                let n = host.density_count(around, self.kind == K::FriendlyDensity, self.a1);
                1.0 - 0.5f64.powf(f64::from(n)) as f32
            }
            K::LineOfSight => flag(host.line_of_sight(a.ai_position, a.delta, a.distance)),
            K::LineOfSightCollision => {
                let origin = [
                    a.ai_position[0] + a.ai_direction[0] * self.a1,
                    a.ai_position[1] + self.a2,
                    a.ai_position[2] + a.ai_direction[2] * self.a3,
                ];
                let delta = if self.a4 <= 0.0 {
                    a.delta
                } else {
                    let reach = if self.a4 > a.distance {
                        a.distance
                    } else {
                        self.a4
                    };
                    a.delta_normalized.map(|d| d * reach)
                };
                flag(!host.scene_hit(origin, delta))
            }
            K::IsCharacter => match self.scope {
                Scope::Target => flag(host.is_character()),
                Scope::SelfScope => 1.0,
                Scope::Global => 0.0,
            },
            K::IsEthereal => {
                let who = if self.scope == Scope::Target {
                    Subject::Target
                } else {
                    Subject::Owner
                };
                flag(host.ethereal(who).unwrap_or(false))
            }
            K::Resistance => {
                let who = if self.scope == Scope::Target {
                    Subject::Target
                } else {
                    Subject::Owner
                };
                host.resistance_against(who, self.elements)
                    .map_or(0.0, |r| 1.0 - r)
            }
            K::Shielded => match self.scope {
                Scope::Global => 0.0,
                _ => flag(host.self_shielded(self.subject().unwrap()).unwrap_or(false)),
            },
            K::IsShield => flag(self.scope == Scope::Target && host.is_shield()),
            K::IsBarrier => flag(self.scope == Scope::Target && host.is_barrier()),
            K::GripDamageAccumulation => host.grip_damage().map_or(0.0, |(d, _)| d),
            K::GripDamageAccumulationNormalized => host.grip_damage().map_or(0.0, |(d, t)| d / t),
            K::IsOnGround => match self.subject() {
                Some(who) => flag(host.on_ground(who).unwrap_or(false)),
                None => 0.0,
            },
            K::Angle => {
                let mut d = a.delta;
                let mut f = a.ai_direction;
                d[1] = 0.0;
                f[1] = 0.0;
                let eps = 1e-6;
                if dot(d, d) > eps && dot(f, f) > eps {
                    let (ld, lf) = (length(d), length(f));
                    dot(d.map(|x| x / ld), f.map(|x| x / lf))
                } else {
                    0.0
                }
            }
            K::Other(_) => 0.0,
        })
    }
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// FuzzyMath.FuzzyDistanceExponential (double precision inside).
pub fn fuzzy_distance_exponential(distance: f32, min: f32, max: f32) -> f32 {
    let mid = (f64::from(min) + f64::from(max)) * 0.5;
    let x = std::f64::consts::E.powf(-std::f64::consts::LN_10 / (mid - f64::from(max)));
    x.powf(-(f64::from(distance) - mid).abs()) as f32
}

/// FuzzyMath.FuzzyDistanceLinear(distance, min, max). The negated test
/// keeps the original's NaN behaviour.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn fuzzy_distance_linear(distance: f32, min: f32, max: f32) -> f32 {
    let mid = (max + min) * 0.5;
    let k = 2.0 / (max - min);
    if !(distance < mid) {
        (max - distance) * k
    } else {
        (distance - min) * k
    }
}

/// FuzzyMath.FuzzyAngle.
pub fn fuzzy_angle(a: Vec3, b: Vec3) -> f32 {
    (dot(a, b) + 1.0) * 0.5
}

/// FuzzyMath.FuzzyAnger from the AttackedBy accumulator.
pub fn fuzzy_anger(attacked: f32) -> f32 {
    1.0 - 0.977237221f64.powf(f64::from(attacked)) as f32
}
