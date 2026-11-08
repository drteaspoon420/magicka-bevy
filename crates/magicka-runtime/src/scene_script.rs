//! Trigger layout decoding. Unsupported behaviors are retained as definitions,
//! never substituted with successful predicates or no-op actions.
use crate::{
    animation_action::{ActionReadError, AnimationAction},
    audio_action::AudioAction,
    avatar_move::AvatarMoveAction,
    dialog_action::DialogAction,
    dialog_state::DialogCondition,
    effect_action::EffectAction,
    entity_command::EntityCommand,
    give_order::GiveOrderAction,
    presence::PresenceCondition,
    scene_command::SceneCommand,
    scene_flow::SceneFlowAction,
    scene_loading::custom_hash,
    script_attributes::{DecodedVariableAction, ScriptReadError},
    script_condition::ScriptCondition,
    spawn_action::SpawnAction,
    spawn_phys::SpawnPhysAction,
    toggle_action::ToggleAction,
    trigger::{compose_action_groups, Trigger},
    trigger_control::TriggerControlAction,
    world_condition::WorldCondition,
};
use quick_xml::{
    events::{BytesStart, Event},
    Reader, XmlVersion,
};
use std::collections::HashSet;
#[derive(Clone, Debug, Default)]
pub struct XmlElement {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<XmlElement>,
    pub raw_text: String,
    /// Decoded descendant text in document order, for XmlNode.InnerText consumers.
    pub inner_text: String,
    preserve_space: bool,
    pending_text: String,
    pub(crate) non_element_children: bool,
    /// Significant text, CDATA or processing instructions; comments excluded.
    pub(crate) non_comment_text_children: bool,
}
impl XmlElement {
    /// Decode shared XML content while preserving native InnerText semantics.
    pub fn parse(bytes: &[u8]) -> Result<Self, LayoutError> {
        xml(bytes)
    }

    /// A childless element with these attributes (synthesized actions).
    pub(crate) fn synthetic(name: &str, attributes: Vec<(String, String)>) -> Self {
        Self {
            name: name.into(),
            attributes,
            children: Vec::new(),
            raw_text: String::new(),
            inner_text: String::new(),
            preserve_space: false,
            pending_text: String::new(),
            non_element_children: false,
            non_comment_text_children: false,
        }
    }
    fn flush_text(&mut self) {
        if !self.pending_text.is_empty()
            && (self.preserve_space
                || !self
                    .pending_text
                    .chars()
                    .all(|c| matches!(c, ' ' | '\t' | '\r' | '\n')))
        {
            self.inner_text.push_str(&self.pending_text);
            self.non_element_children = true;
            self.non_comment_text_children = true;
        }
        self.pending_text.clear();
    }
    fn attributes(&self) -> impl Iterator<Item = (&str, &str)> {
        self.attributes
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }
}
#[derive(Clone, Debug)]
pub enum ConditionDefinition {
    Known(ScriptCondition),
    Presence(PresenceCondition),
    Dialog(DialogCondition),
    World(WorldCondition),
    TutorialDone { invert: bool },
    Unsupported(XmlElement),
}
#[derive(Clone, Debug)]
pub enum ActionDefinition {
    Dispenser(crate::dispenser::DispenserAction),
    Animation(AnimationAction),
    Variable(DecodedVariableAction),
    Control(TriggerControlAction),
    Dialog(DialogAction),
    Tutorial(crate::tutorial::TutorialAction),
    Campaign(crate::campaign_action::CampaignAction),
    Boss(crate::boss_action::BossAction),
    Damage(crate::script_damage::ScriptDamageAction),
    Spawn(Box<SpawnAction>),
    SpawnElemental(crate::spawn_elemental::SpawnElementalAction),
    GiveOrder(Box<GiveOrderAction>),
    Scene(SceneCommand),
    Audio(AudioAction),
    Effect(EffectAction),
    AvatarMove(Box<AvatarMoveAction>),
    SpawnPhys(Box<SpawnPhysAction>),
    SpawnItem(Box<crate::spawn_item::SpawnItemAction>),
    SpawnMagick(crate::magick_books::SpawnMagickAction),
    Entity(EntityCommand),
    Toggle(ToggleAction),
    Flow(Box<SceneFlowAction>),
    PhysAnimation(crate::phys_animation::PhysAnimationAction),
    Unsupported(XmlElement),
}
#[derive(Clone, Debug, Default)]
pub struct SceneScriptLayout {
    pub triggers: Vec<Trigger>,
    pub conditions: Vec<ConditionDefinition>,
    pub actions: Vec<ActionDefinition>,
    pub deferred_interactables: Vec<XmlElement>,
    /// OnInteract entries; each also has a trigger (same id) in `triggers`.
    pub interactables: Vec<InteractableDef>,
    /// GameScene's `<RuleSet>` (and, for Survival, its `<wave>` nodes).
    pub ruleset: Option<crate::ruleset::RulesetLayout>,
}

/// Magicka.Levels.Triggers.InteractType (Defines.INTERACTSTRINGS index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractType {
    None = 0,
    PickUp = 1,
    Examine = 2,
    Talk = 3,
    Interact = 4,
    Activate = 5,
}

impl InteractType {
    fn parse(value: &str) -> Option<Self> {
        Some(match value.to_ascii_lowercase().as_str() {
            "none" => Self::None,
            "pick_up" => Self::PickUp,
            "examine" => Self::Examine,
            "talk" => Self::Talk,
            "interact" => Self::Interact,
            "activate" => Self::Activate,
            _ => return None,
        })
    }
}

/// An `<OnInteract>` (Interactable): a condition-less, non-autorun trigger
/// keyed by its lowercased locator, executed by Interact.
#[derive(Clone, Debug, PartialEq)]
pub struct InteractableDef {
    pub id: i32,
    pub locator: String,
    pub kind: InteractType,
    pub enabled: bool,
    /// animHighlight paths (`part/child/...`, lowercased, hashed per level).
    pub anim_highlight: Vec<Vec<i32>>,
    /// physHighlight entity unique-ID hashes.
    pub phys_highlight: Vec<i32>,
    /// Attributes the native constructor rejects with an exception. Only the
    /// unlisted havindr_s5HBF.xml has one (`repeatitions`); the rewrite keeps
    /// loading and records them.
    pub unknown_attributes: Vec<String>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum LayoutError {
    Xml(String),
    InvalidTriggerAttribute(String),
    DuplicateTrigger(i32),
    MissingThen,
    Condition(ScriptReadError),
    Action(ScriptReadError),
    Animation(ActionReadError),
    Control(ActionReadError),
}
fn element(
    node: &BytesStart<'_>,
    reader: &Reader<&[u8]>,
    inherited_space: bool,
) -> Result<XmlElement, LayoutError> {
    let name = std::str::from_utf8(node.name().as_ref())
        .map_err(|e| LayoutError::Xml(e.to_string()))?
        .to_owned();
    let attributes = node
        .attributes()
        .map(|a| {
            let a = a.map_err(|e| LayoutError::Xml(e.to_string()))?;
            let key = std::str::from_utf8(a.key.as_ref())
                .map_err(|e| LayoutError::Xml(e.to_string()))?
                .to_owned();
            let value = a
                .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
                .map_err(|e| LayoutError::Xml(e.to_string()))?
                .into_owned();
            Ok((key, value))
        })
        .collect::<Result<Vec<_>, LayoutError>>()?;
    let preserve_space = match attributes
        .iter()
        .find(|(key, _)| key == "xml:space")
        .map(|(_, value)| value.as_str())
    {
        Some("preserve") => true,
        Some("default") => false,
        Some(_) => return Err(LayoutError::Xml("invalid xml:space".into())),
        None => inherited_space,
    };
    Ok(XmlElement {
        name,
        attributes,
        children: vec![],
        raw_text: String::new(),
        inner_text: String::new(),
        preserve_space,
        pending_text: String::new(),
        non_element_children: false,
        non_comment_text_children: false,
    })
}
/// Try each recovered condition family; only an unknown tag falls through.
fn read_condition(node: &XmlElement) -> Result<ConditionDefinition, LayoutError> {
    fn attempt<T>(
        result: Result<T, ScriptReadError>,
        wrap: impl FnOnce(T) -> ConditionDefinition,
    ) -> Result<Option<ConditionDefinition>, LayoutError> {
        match result {
            Ok(c) => Ok(Some(wrap(c))),
            Err(ScriptReadError::UnsupportedTag(_)) => Ok(None),
            Err(e) => Err(LayoutError::Condition(e)),
        }
    }
    if node.name.eq_ignore_ascii_case("dialoghintdone") {
        let mut invert = false;
        for (key, value) in node.attributes() {
            if key.eq_ignore_ascii_case("invert") {
                invert = boolean(value, key)?;
            }
        }
        return Ok(ConditionDefinition::TutorialDone { invert });
    }
    let attrs = || node.attributes();
    if let Some(d) = attempt(
        ScriptCondition::read(&node.name, attrs()),
        ConditionDefinition::Known,
    )? {
        return Ok(d);
    }
    if let Some(d) = attempt(
        PresenceCondition::read(&node.name, attrs()),
        ConditionDefinition::Presence,
    )? {
        return Ok(d);
    }
    if let Some(d) = attempt(
        DialogCondition::read(&node.name, attrs()),
        ConditionDefinition::Dialog,
    )? {
        return Ok(d);
    }
    if let Some(d) = attempt(
        WorldCondition::read(&node.name, attrs()),
        ConditionDefinition::World,
    )? {
        return Ok(d);
    }
    Ok(ConditionDefinition::Unsupported(node.clone()))
}
pub(crate) fn xml(bytes: &[u8]) -> Result<XmlElement, LayoutError> {
    let mut reader = Reader::from_reader(bytes);
    let mut stack = Vec::<XmlElement>::new();
    let mut roots = vec![];
    loop {
        match reader
            .read_event()
            .map_err(|e| LayoutError::Xml(e.to_string()))?
        {
            Event::Start(node) => {
                if let Some(parent) = stack.last_mut() {
                    parent.flush_text();
                }
                stack.push(element(
                    &node,
                    &reader,
                    stack.last().is_some_and(|n| n.preserve_space),
                )?);
            }
            Event::Empty(node) => {
                if let Some(parent) = stack.last_mut() {
                    parent.flush_text();
                }
                let node = element(
                    &node,
                    &reader,
                    stack.last().is_some_and(|n| n.preserve_space),
                )?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else {
                    roots.push(node);
                }
            }
            Event::End(_) => {
                let mut node = stack
                    .pop()
                    .ok_or_else(|| LayoutError::Xml("unexpected closing tag".into()))?;
                node.flush_text();
                if let Some(parent) = stack.last_mut() {
                    parent.inner_text.push_str(&node.inner_text);
                    parent.children.push(node);
                } else {
                    roots.push(node);
                }
            }
            Event::Text(text) => {
                if let Some(node) = stack.last_mut() {
                    let decoded = text
                        .xml_content(XmlVersion::Implicit1_0)
                        .map_err(|e| LayoutError::Xml(e.to_string()))?;
                    node.pending_text.push_str(&decoded);
                    node.raw_text.push_str(
                        std::str::from_utf8(text.as_ref())
                            .map_err(|e| LayoutError::Xml(e.to_string()))?,
                    );
                }
            }
            Event::CData(text) => {
                if let Some(node) = stack.last_mut() {
                    node.flush_text();
                    node.non_element_children = true;
                    node.non_comment_text_children = true;
                    node.inner_text.push_str(
                        &text
                            .xml_content(XmlVersion::Implicit1_0)
                            .map_err(|e| LayoutError::Xml(e.to_string()))?,
                    );
                    node.raw_text.push_str(
                        std::str::from_utf8(text.as_ref())
                            .map_err(|e| LayoutError::Xml(e.to_string()))?,
                    );
                }
            }
            Event::DocType(_) => return Err(LayoutError::Xml("DOCTYPE is unsupported".into())),
            Event::GeneralRef(reference) => {
                if let Some(node) = stack.last_mut() {
                    let spelling = format!(
                        "&{};",
                        std::str::from_utf8(reference.as_ref())
                            .map_err(|e| LayoutError::Xml(e.to_string()))?
                    );
                    node.pending_text.push_str(
                        &quick_xml::escape::unescape(&spelling)
                            .map_err(|e| LayoutError::Xml(e.to_string()))?,
                    );
                    node.raw_text.push('&');
                    node.raw_text.push_str(
                        std::str::from_utf8(reference.as_ref())
                            .map_err(|e| LayoutError::Xml(e.to_string()))?,
                    );
                    node.raw_text.push(';');
                }
            }
            Event::Eof => break,
            Event::Comment(_) => {
                if let Some(node) = stack.last_mut() {
                    node.flush_text();
                    node.non_element_children = true;
                }
            }
            Event::PI(_) => {
                if let Some(node) = stack.last_mut() {
                    node.flush_text();
                    node.non_element_children = true;
                    node.non_comment_text_children = true;
                }
            }
            _ => {}
        }
    }
    if !stack.is_empty() || roots.len() != 1 {
        return Err(LayoutError::Xml("expected one complete root".into()));
    }
    Ok(roots.remove(0))
}
fn boolean(value: &str, key: &str) -> Result<bool, LayoutError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(LayoutError::InvalidTriggerAttribute(key.into())),
    }
}
impl SceneScriptLayout {
    pub fn read(bytes: &[u8], looped: bool) -> Result<Self, LayoutError> {
        let root = xml(bytes)?;
        if !root.name.eq_ignore_ascii_case("scene") {
            return Err(LayoutError::Xml("expected Scene root".into()));
        }
        let mut layout = Self::default();
        let mut ids = HashSet::new();
        for node in &root.children {
            match node.name.to_ascii_lowercase().as_str() {
                "trigger" => {
                    let trigger = layout.read_trigger(node, looped)?;
                    if !ids.insert(trigger.id) {
                        return Err(LayoutError::DuplicateTrigger(trigger.id));
                    }
                    layout.triggers.push(trigger);
                }
                "oninteract" => {
                    layout.deferred_interactables.push(node.clone());
                    let (def, trigger) = layout.read_interactable(node, looped)?;
                    if !ids.insert(trigger.id) {
                        return Err(LayoutError::DuplicateTrigger(trigger.id));
                    }
                    layout.triggers.push(trigger);
                    layout.interactables.push(def);
                }
                "ruleset" => {
                    layout.ruleset = crate::ruleset::RulesetLayout::read(node);
                }
                // GameScene reads waves only after a Survival ruleset.
                "wave"
                    if layout
                        .ruleset
                        .as_ref()
                        .is_some_and(|r| r.kind == crate::ruleset::RulesetKind::Survival) =>
                {
                    layout.read_wave(node, looped)?;
                }
                _ => {} // This loader handles the scene's script section only.
            }
        }
        Ok(layout)
    }
    /// Wave.Read: each non-comment child is a WaveActions; its actions
    /// become a hidden trigger whose Area-taking actions are redirected to a
    /// placeholder area, aliased to the WaveActions' spawn area when the
    /// ruleset initializes (WaveActions.Initialize sets every Area
    /// property).
    fn read_wave(&mut self, node: &XmlElement, looped: bool) -> Result<(), LayoutError> {
        let ruleset = self.ruleset.as_mut().expect("survival ruleset");
        let w = ruleset.waves.len();
        ruleset.waves.push(Vec::new());
        for (a, child) in node.children.iter().enumerate() {
            let mut area = None;
            let mut delay = 0.0f32;
            for (key, value) in child.attributes() {
                match key.to_ascii_lowercase().as_str() {
                    "area" if !value.eq_ignore_ascii_case("any") => area = Some(value.to_owned()),
                    "delay" => delay = value.trim().parse().unwrap_or(0.0),
                    _ => {}
                }
            }
            let placeholder = crate::ruleset::wave_area_placeholder(w, a);
            let name = format!("#survivalwave{w}_{a}");
            let id = custom_hash(&name);
            let children: Vec<XmlElement> = child
                .children
                .iter()
                .map(|n| {
                    let mut n = n.clone();
                    if crate::ruleset::AREA_ACTIONS.contains(&n.name.to_ascii_lowercase().as_str())
                    {
                        n.attributes
                            .retain(|(k, _)| !k.eq_ignore_ascii_case("area"));
                        n.attributes.push(("area".into(), placeholder.clone()));
                    }
                    n
                })
                .collect();
            let mut trigger = Trigger::new(id, vec![], vec![]);
            trigger.name = name;
            trigger.autorun = false;
            trigger.repeat = 0.0;
            let (mut random, mut common) = (vec![], vec![]);
            self.read_actions(&children, looped, id, &mut random, &mut common)?;
            if !random.is_empty() {
                // WaveActions.Read: "Can't use RANDOM in survivalmode".
                return Err(LayoutError::Xml("RANDOM in a survival wave".into()));
            }
            trigger.actions = compose_action_groups(random, &common);
            let spawns: Vec<(String, u32)> = trigger
                .actions
                .iter()
                .flatten()
                .filter_map(|&i| match &self.actions[i] {
                    ActionDefinition::Spawn(s) => Some((
                        s.type_name.clone().unwrap_or_default(),
                        s.amount.max(0) as u32,
                    )),
                    _ => None,
                })
                .collect();
            let has_actions = trigger.actions.iter().any(|g| !g.is_empty());
            self.triggers.push(trigger);
            self.ruleset.as_mut().unwrap().waves[w].push(crate::ruleset::WaveActionsLayout {
                trigger: id,
                has_actions,
                area,
                placeholder,
                delay,
                spawns,
            });
        }
        Ok(())
    }
    /// Interactable(XmlNode): locator/type/enabled/animhighlight/
    /// physhighlight attributes (others throw), then ReadActions over the
    /// node's own children; no conditions, no autorun.
    fn read_interactable(
        &mut self,
        node: &XmlElement,
        looped: bool,
    ) -> Result<(InteractableDef, Trigger), LayoutError> {
        let mut def = InteractableDef {
            id: 0,
            locator: String::new(),
            kind: InteractType::None,
            enabled: true,
            anim_highlight: Vec::new(),
            phys_highlight: Vec::new(),
            unknown_attributes: Vec::new(),
        };
        let list = |v: &str| -> Vec<String> {
            v.to_ascii_lowercase()
                .replace(' ', "")
                .split(',')
                .map(str::to_owned)
                .collect()
        };
        for (key, value) in node.attributes() {
            match key.to_ascii_lowercase().as_str() {
                "locator" => {
                    def.locator = value.to_ascii_lowercase();
                    def.id = custom_hash(&def.locator);
                }
                "type" => {
                    def.kind = InteractType::parse(value)
                        .ok_or_else(|| LayoutError::InvalidTriggerAttribute(key.into()))?;
                }
                "enabled" => def.enabled = boolean(value, key)?,
                "animhighlight" => {
                    for path in list(value) {
                        def.anim_highlight
                            .push(path.split('/').map(custom_hash).collect());
                    }
                }
                "physhighlight" => {
                    for name in list(value) {
                        def.phys_highlight.push(custom_hash(&name));
                    }
                }
                _ => def.unknown_attributes.push(key.to_owned()),
            }
        }
        let mut trigger = Trigger::new(def.id, vec![], vec![]);
        trigger.autorun = false;
        let (mut random, mut common) = (vec![], vec![]);
        self.read_actions(&node.children, looped, def.id, &mut random, &mut common)?;
        trigger.actions = compose_action_groups(random, &common);
        Ok((def, trigger))
    }
    fn read_trigger(&mut self, node: &XmlElement, looped: bool) -> Result<Trigger, LayoutError> {
        let mut trigger = Trigger::new(0, vec![], vec![]);
        let mut has_then = false;
        for (key, value) in node.attributes() {
            match key.to_ascii_lowercase().as_str() {
                "id" => {
                    if !value.is_ascii() {
                        return Err(LayoutError::InvalidTriggerAttribute(key.into()));
                    }
                    trigger.id = custom_hash(&value.to_ascii_lowercase());
                    trigger.name = value.to_owned();
                }
                "autorun" => trigger.autorun = boolean(value, key)?,
                "repeat" => {
                    trigger.repeat = match value.trim().to_ascii_lowercase().as_str() {
                        "true" => 0.0,
                        "false" => -1.0,
                        _ => {
                            if value.contains(['e', 'E', ',']) {
                                return Err(LayoutError::InvalidTriggerAttribute(key.into()));
                            }
                            value
                                .trim()
                                .parse()
                                .map_err(|_| LayoutError::InvalidTriggerAttribute(key.into()))?
                        }
                    };
                }
                _ => return Err(LayoutError::InvalidTriggerAttribute(key.into())),
            }
        }
        for child in &node.children {
            match child.name.to_ascii_lowercase().as_str() {
                "if" => {
                    let mut group = vec![];
                    for condition in &child.children {
                        let definition = read_condition(condition)?;
                        group.push(self.conditions.len());
                        self.conditions.push(definition);
                    }
                    trigger.conditions.push(group);
                }
                "then" => {
                    has_then = true;
                    let (mut random, mut common) = (vec![], vec![]);
                    self.read_actions(
                        &child.children,
                        looped,
                        trigger.id,
                        &mut random,
                        &mut common,
                    )?;
                    trigger.actions = compose_action_groups(random, &common);
                }
                _ => {}
            }
        }
        if !has_then {
            return Err(LayoutError::MissingThen);
        }
        Ok(trigger)
    }
    /// Append one action definition, read exactly as a scene `<Then>` child
    /// owned by trigger `owner` would be, and return its pool index.
    pub fn define_action(&mut self, node: &XmlElement, owner: i32) -> Result<usize, LayoutError> {
        self.action(node, owner)
    }
    /// Append one condition definition, read as an `<If>` child would be.
    pub fn define_condition(&mut self, node: &XmlElement) -> Result<usize, LayoutError> {
        let definition = read_condition(node)?;
        self.conditions.push(definition);
        Ok(self.conditions.len() - 1)
    }
    /// Read a `<Trigger>` element (with `If`/`Then` children) into the
    /// pools, as the scene loader does, without registering it.
    pub fn define_trigger(
        &mut self,
        node: &XmlElement,
        looped: bool,
    ) -> Result<Trigger, LayoutError> {
        self.read_trigger(node, looped)
    }
    fn action(&mut self, node: &XmlElement, owner: i32) -> Result<usize, LayoutError> {
        let definition = match node.name.to_ascii_lowercase().as_str() {
            "playanimation" | "stopanimation" | "resumeanimation" => ActionDefinition::Animation(
                AnimationAction::read(&node.name, node.attributes())
                    .map_err(LayoutError::Animation)?,
            ),
            "counter" | "timer" | "addtimer" => ActionDefinition::Variable(
                DecodedVariableAction::read(&node.name, node.attributes())
                    .map_err(LayoutError::Action)?,
            ),
            "executetrigger" | "cleardelayedactions" => ActionDefinition::Control(
                TriggerControlAction::read(&node.name, owner, node.attributes())
                    .map_err(LayoutError::Control)?,
            ),
            "startdialog" | "killdialogs" => ActionDefinition::Dialog(
                DialogAction::read(&node.name, node.attributes()).map_err(LayoutError::Action)?,
            ),
            "spawndispenser" | "activatedispenser" => ActionDefinition::Dispenser(
                crate::dispenser::DispenserAction::read(node).map_err(LayoutError::Action)?,
            ),
            "spawn" => ActionDefinition::Spawn(Box::new(
                SpawnAction::read(node).map_err(LayoutError::Action)?,
            )),
            "spawnelemental" => ActionDefinition::SpawnElemental(
                crate::spawn_elemental::SpawnElementalAction::read(node.attributes())
                    .map_err(LayoutError::Action)?,
            ),
            name if crate::scene_command::TAGS.contains(&name) => ActionDefinition::Scene(
                SceneCommand::read(&node.name, node.attributes()).map_err(LayoutError::Action)?,
            ),
            "sethint" | "setdialoghint" | "removedialoghint" | "enabledark" | "disabledark" => {
                ActionDefinition::Tutorial(
                    crate::tutorial::TutorialAction::read(&node.name, node.attributes())
                        .map_err(LayoutError::Action)?,
                )
            }
            name if crate::boss_action::TAGS.contains(&name) => ActionDefinition::Boss(
                crate::boss_action::BossAction::read(node).map_err(LayoutError::Action)?,
            ),
            "spawnclonewizards" => ActionDefinition::Campaign(
                crate::campaign_action::CampaignAction::read_clone_wizards(node)
                    .map_err(LayoutError::Action)?,
            ),
            name if crate::campaign_action::TAGS.contains(&name) => ActionDefinition::Campaign(
                crate::campaign_action::CampaignAction::read(&node.name, node.attributes())
                    .map_err(LayoutError::Action)?,
            ),
            "damage" => ActionDefinition::Damage(
                crate::script_damage::ScriptDamageAction::read(node)
                    .map_err(LayoutError::Action)?,
            ),
            name if crate::audio_action::TAGS.contains(&name) => ActionDefinition::Audio(
                AudioAction::read(&node.name, node.attributes()).map_err(LayoutError::Action)?,
            ),
            name if crate::effect_action::TAGS.contains(&name) => ActionDefinition::Effect(
                EffectAction::read(&node.name, node.attributes()).map_err(LayoutError::Action)?,
            ),
            "avatarmove" => ActionDefinition::AvatarMove(Box::new(
                AvatarMoveAction::read(node).map_err(LayoutError::Action)?,
            )),
            "spawnmagick" => ActionDefinition::SpawnMagick(
                crate::magick_books::SpawnMagickAction::read(node.attributes())
                    .map_err(LayoutError::Action)?,
            ),
            "spawnitem" => ActionDefinition::SpawnItem(Box::new(
                crate::spawn_item::SpawnItemAction::read(node.attributes())
                    .map_err(LayoutError::Action)?,
            )),
            "spawnphys" => ActionDefinition::SpawnPhys(Box::new(
                SpawnPhysAction::read(node.attributes()).map_err(LayoutError::Action)?,
            )),
            "spawnanimprop" => ActionDefinition::SpawnPhys(Box::new(
                SpawnPhysAction::read_anim_prop(node.attributes()).map_err(LayoutError::Action)?,
            )),
            name if crate::entity_command::TAGS.contains(&name) => ActionDefinition::Entity(
                EntityCommand::read(&node.name, node.attributes()).map_err(LayoutError::Action)?,
            ),
            name if crate::toggle_action::TAGS.contains(&name) => ActionDefinition::Toggle(
                ToggleAction::read(&node.name, node.attributes()).map_err(LayoutError::Action)?,
            ),
            name if crate::scene_flow::TAGS.contains(&name) => ActionDefinition::Flow(Box::new(
                SceneFlowAction::read(node).map_err(LayoutError::Action)?,
            )),
            "physanimation" => ActionDefinition::PhysAnimation(
                crate::phys_animation::PhysAnimationAction::read(node.attributes())
                    .map_err(LayoutError::Action)?,
            ),
            "giveorder" => ActionDefinition::GiveOrder(Box::new(
                GiveOrderAction::read(node).map_err(LayoutError::Action)?,
            )),
            _ => ActionDefinition::Unsupported(node.clone()),
        };
        let index = self.actions.len();
        self.actions.push(definition);
        Ok(index)
    }
    fn read_actions(
        &mut self,
        nodes: &[XmlElement],
        looped: bool,
        owner: i32,
        random: &mut Vec<Vec<usize>>,
        common: &mut Vec<usize>,
    ) -> Result<(), LayoutError> {
        for node in nodes {
            match node.name.to_ascii_lowercase().as_str() {
                "looped" if looped => {
                    for child in &node.children {
                        self.read_action_branch(child, owner, random, common)?;
                    }
                }
                "looped" => {}
                _ => self.read_action_branch(node, owner, random, common)?,
            }
        }
        Ok(())
    }

    fn read_action_branch(
        &mut self,
        node: &XmlElement,
        owner: i32,
        random: &mut Vec<Vec<usize>>,
        common: &mut Vec<usize>,
    ) -> Result<(), LayoutError> {
        if node.name.eq_ignore_ascii_case("random") {
            let mut group = vec![];
            for action in &node.children {
                group.push(self.action(action, owner)?);
            }
            random.push(group);
        } else {
            common.push(self.action(node, owner)?);
        }
        Ok(())
    }
}
