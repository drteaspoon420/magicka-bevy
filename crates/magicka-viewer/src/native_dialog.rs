//! Persistent manager/host resource and native-ordered box-to-GPU synchronization.
use crate::{
    native_text::{geometry_mesh, NativeBitmapText, NativeTextDelta, NativeTextSet},
    text_box_visual::background_mesh,
};
use bevy::prelude::*;
use magicka_runtime::{
    dialog_frame::DialogFrame,
    dialog_manager::{DialogCollectionState, DialogManager},
    dialog_text_host::{DialogTextFrameWorld, DialogTextHost},
    interaction_playback::InteractionLocation,
    message_playback::MessageHost,
    text_geometry::{GeometryFont, TextGeometry, TextStyle},
};
use magicka_sim::damage_delivery::NetworkRole;
use std::{collections::VecDeque, marker::PhantomData, sync::Arc};

// TextBox.get_ZIndex: dialogs follow health bars (98) and precede the
// keyboard guide (600). Keep text just above its own background in Bevy.
const DIALOG_LAYER: f32 = 201.;
const DIALOG_TEXT_LAYER: f32 = DIALOG_LAYER + 0.01;

pub enum NativeDialogCommand {
    Start {
        dialog: i32,
        location: InteractionLocation,
        controller: Option<i32>,
    },
    Advance {
        controller: Option<i32>,
        role: NetworkRole,
    },
    EndAll,
    EndDialog(i32),
    EndController(Option<i32>),
    SetDialogs(DialogCollectionState),
}
#[derive(Resource)]
pub struct NativeDialogSession<W: Send + Sync + 'static> {
    pub manager: DialogManager,
    pub host: DialogTextHost<W>,
    pub commands: VecDeque<NativeDialogCommand>,
    pub screen: [f32; 2],
    pub matrix: [[f32; 4]; 4],
    pub channel: usize,
    pub failure: Option<String>,
    pub replaced_collections: Vec<DialogCollectionState>,
    dt: f32,
}
impl<W: magicka_runtime::dialog_text_host::DialogTextWorld + Send + Sync + 'static>
    NativeDialogSession<W>
{
    pub fn new(manager: DialogManager, mut host: DialogTextHost<W>, screen: [f32; 2]) -> Self {
        host.select_collection(manager.collection_token());
        Self {
            manager,
            host,
            commands: VecDeque::new(),
            screen,
            matrix: Mat4::IDENTITY.to_cols_array_2d(),
            channel: 0,
            failure: None,
            replaced_collections: vec![],
            dt: 0.,
        }
    }
}
/// Borrowed dialog state the box renderer reads and writes back (typing).
pub struct DialogViewMut<'a, W> {
    pub host: &'a mut DialogTextHost<W>,
    pub channel: usize,
    pub dt: f32,
    pub screen: [f32; 2],
    pub matrix: [[f32; 4]; 4],
    pub failure: &'a mut Option<String>,
}
/// A resource owning dialog text boxes to render (preview session or game).
pub trait DialogView<W>:
    Resource + bevy::ecs::component::Component<Mutability = bevy::ecs::component::Mutable>
{
    fn view(&mut self) -> Option<DialogViewMut<'_, W>>;
}
impl<W: magicka_runtime::dialog_text_host::DialogTextWorld + Send + Sync + 'static> DialogView<W>
    for NativeDialogSession<W>
{
    fn view(&mut self) -> Option<DialogViewMut<'_, W>> {
        Some(DialogViewMut {
            host: &mut self.host,
            channel: self.channel,
            dt: self.dt,
            screen: self.screen,
            matrix: self.matrix,
            failure: &mut self.failure,
        })
    }
}
/// Renders boxes from any DialogView resource (no manager updates).
pub struct NativeDialogRenderPlugin<R, W>(PhantomData<fn() -> (R, W)>);
impl<R, W> Default for NativeDialogRenderPlugin<R, W> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
impl<R, W> Plugin for NativeDialogRenderPlugin<R, W>
where
    R: DialogView<W>,
    W: DialogTextFrameWorld + Send + Sync + 'static,
    <W as MessageHost>::Error: std::fmt::Debug,
{
    fn build(&self, app: &mut App) {
        app.configure_sets(
            PostUpdate,
            (
                DialogRenderSet::Prepare,
                NativeTextSet::Mesh,
                DialogRenderSet::Publish,
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            prepare_boxes::<R, W>
                .in_set(DialogRenderSet::Prepare)
                .before(bevy::transform::TransformSystems::Propagate),
        )
        .add_systems(
            PostUpdate,
            publish_typing::<R, W>.in_set(DialogRenderSet::Publish),
        );
    }
}
/// Attach to the body NativeBitmapText entity. All meshes/materials are supplied
/// by the caller from original private atlases; this adapter does not reload them.
#[derive(Component)]
pub struct NativeDialogBox {
    pub slot: usize,
    pub cutscene: bool,
    pub additional: bool,
    pub background: Entity,
    pub background_mesh: Handle<Mesh>,
    pub owner_name: Entity,
    pub owner_mesh: Handle<Mesh>,
    pub name_font: Arc<GeometryFont>,
    name_cache: Option<Vec<u16>>,
}
impl NativeDialogBox {
    pub fn new(
        slot: usize,
        background: Entity,
        background_mesh: Handle<Mesh>,
        owner_name: Entity,
        owner_mesh: Handle<Mesh>,
        name_font: Arc<GeometryFont>,
    ) -> Self {
        Self {
            slot,
            cutscene: false,
            additional: false,
            background,
            background_mesh,
            owner_name,
            owner_mesh,
            name_font,
            name_cache: None,
        }
    }
    pub fn for_cutscene(
        background: Entity,
        background_mesh: Handle<Mesh>,
        owner_name: Entity,
        owner_mesh: Handle<Mesh>,
        name_font: Arc<GeometryFont>,
    ) -> Self {
        let mut binding = Self::new(
            0,
            background,
            background_mesh,
            owner_name,
            owner_mesh,
            name_font,
        );
        binding.cutscene = true;
        binding
    }
}
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
enum DialogRenderSet {
    Prepare,
    Publish,
}
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum NativeDialogFrameSet {
    Update,
}
pub struct NativeDialogPlugin<W>(PhantomData<fn() -> W>);
impl<W> Default for NativeDialogPlugin<W> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
impl<W> Plugin for NativeDialogPlugin<W>
where
    W: DialogTextFrameWorld + Send + Sync + 'static,
    <W as MessageHost>::Error: std::fmt::Debug,
{
    fn build(&self, app: &mut App) {
        app.configure_sets(
            PostUpdate,
            (
                DialogRenderSet::Prepare,
                NativeTextSet::Mesh,
                DialogRenderSet::Publish,
            )
                .chain(),
        )
        .add_systems(
            Update,
            update_session::<W>.in_set(NativeDialogFrameSet::Update),
        )
        .add_systems(
            PostUpdate,
            prepare_boxes::<NativeDialogSession<W>, W>
                .in_set(DialogRenderSet::Prepare)
                .before(bevy::transform::TransformSystems::Propagate),
        )
        .add_systems(
            PostUpdate,
            publish_typing::<NativeDialogSession<W>, W>.in_set(DialogRenderSet::Publish),
        );
    }
}
fn update_session<W>(
    session: Option<ResMut<NativeDialogSession<W>>>,
    clock: Res<NativeTextDelta>,
    time: Res<Time<Real>>,
) where
    W: DialogTextFrameWorld + Send + Sync + 'static,
    <W as MessageHost>::Error: std::fmt::Debug,
{
    let Some(mut session) = session else { return };
    if session.failure.is_some() {
        return;
    }
    let session = &mut *session;
    session.dt = clock.0.unwrap_or(time.delta_secs());
    while let Some(command) = session.commands.pop_front() {
        let cutscene = session.host.world.in_cutscene();
        let result = match command {
            NativeDialogCommand::Start {
                dialog,
                location,
                controller,
            } => session
                .manager
                .start(dialog, location, controller, cutscene, &mut session.host)
                .map(|_| ()),
            NativeDialogCommand::Advance { controller, role } => session
                .manager
                .advance_controller(controller, cutscene, role, &mut session.host),
            NativeDialogCommand::EndAll => session.manager.end_all(&mut session.host),
            NativeDialogCommand::EndDialog(id) => session.manager.end_dialog(id, &mut session.host),
            NativeDialogCommand::EndController(controller) => session
                .manager
                .end_for_controller(controller, &mut session.host),
            NativeDialogCommand::SetDialogs(collection) => {
                let token = collection.token;
                session
                    .replaced_collections
                    .push(session.manager.set_dialogs(collection));
                session.host.select_collection(token);
                Ok(())
            }
        };
        if let Err(error) = result {
            session.failure = Some(format!("dialog command: {error:?}"));
            return;
        }
    }
    if let Err(error) = session.manager.update(
        DialogFrame {
            channel: session.channel,
            dt: session.dt,
        },
        &mut session.matrix,
        &mut session.host,
    ) {
        session.failure = Some(format!("dialog frame: {error:?}"));
    }
}
fn prepare_boxes<R: DialogView<W>, W>(
    session: Option<ResMut<R>>,
    mut bindings: Query<(Entity, &mut NativeDialogBox)>,
    mut texts: Query<&mut NativeBitmapText>,
    mut placements: Query<(&mut Transform, &mut Visibility)>,
    mut meshes: ResMut<Assets<Mesh>>,
) where
    W: DialogTextFrameWorld + Send + Sync + 'static,
    <W as MessageHost>::Error: std::fmt::Debug,
{
    let Some(mut session) = session else { return };
    let Some(session) = session.view() else {
        return;
    };
    if session.failure.is_some() {
        return;
    }
    let mut seen = [false; 8];
    let mut seen_additional = vec![false; session.host.additional.len()];
    let mut seen_cutscene = false;
    for (body_entity, mut binding) in &mut bindings {
        if binding.additional && binding.slot >= session.host.additional.len() {
            if let Ok(mut body) = texts.get_mut(body_entity) {
                body.suspended = true;
            }
            for entity in [body_entity, binding.background, binding.owner_name] {
                if let Ok((_, mut visibility)) = placements.get_mut(entity) {
                    *visibility = Visibility::Hidden;
                }
            }
            continue;
        }
        let result = (|| -> Result<(), String> {
            let used = if binding.cutscene {
                &mut seen_cutscene
            } else if binding.additional {
                &mut seen_additional[binding.slot]
            } else {
                seen.get_mut(binding.slot).ok_or("invalid dialog slot")?
            };
            if *used {
                return Err("duplicate dialog slot binding".into());
            }
            *used = true;
            let submitted = session
                .host
                .cutscene_submitted
                .get(session.channel)
                .copied()
                .unwrap_or(false);
            let widget = if binding.cutscene {
                &mut session.host.cutscene.widget
            } else if binding.additional {
                &mut session.host.additional[binding.slot]
            } else {
                session
                    .host
                    .boxes
                    .get_mut(binding.slot)
                    .ok_or("invalid dialog slot")?
            };
            let visible = widget.state.scene_initialized
                && widget.state.visible()
                && (!binding.cutscene || submitted);
            let mut body = texts
                .get_mut(body_entity)
                .map_err(|_| "missing body text")?;
            body.suspended = !visible;
            body.reveal_enabled = false;
            if let Some(error) = &body.failure {
                return Err(error.clone());
            }
            for entity in [body_entity, binding.background, binding.owner_name] {
                let (_, mut visibility) = placements
                    .get_mut(entity)
                    .map_err(|_| "missing box placement")?;
                *visibility = if visible {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                };
            }
            if !visible {
                return Ok(());
            }
            placements
                .get_mut(binding.background)
                .map_err(|_| "missing background placement")?
                .0
                .translation
                .z = DIALOG_LAYER;
            let center = if binding.cutscene {
                magicka_runtime::cutscene_text::pre_render(
                    &mut widget.state,
                    session.channel,
                    session.dt,
                    session.screen,
                )
            } else {
                widget
                    .state
                    .pre_render(session.channel, session.dt, session.screen, session.matrix)
            }
            .map_err(|e| format!("box pre-render: {e:?}"))?;
            let state = widget.state.channels[session.channel];
            let origins =
                widget.draw_origins(center, state.scale, binding.name_font.metrics.line_height);
            let transform = |p: [f32; 2]| {
                Transform::from_xyz(
                    p[0] - session.screen[0] * 0.5,
                    session.screen[1] * 0.5 - p[1],
                    DIALOG_TEXT_LAYER,
                )
                .with_scale(Vec3::new(state.scale, state.scale, 1.))
            };
            placements
                .get_mut(body_entity)
                .map_err(|_| "missing body placement")?
                .0
                .set_if_neq(transform(origins.body));
            let (mut position, mut visibility) = placements
                .get_mut(binding.owner_name)
                .map_err(|_| "missing name placement")?;
            if let Some(origin) = origins.owner_name {
                position.set_if_neq(transform(origin));
            } else {
                *visibility = Visibility::Hidden;
            }
            *meshes
                .get_mut(&binding.background_mesh)
                .ok_or("missing background mesh")? = background_mesh(
                center,
                state.size.map(|v| v as f32),
                state.scale,
                session.screen,
            );
            if widget.show_name && binding.name_cache.as_ref() != Some(&widget.name_characters) {
                let geometry = TextGeometry::build(
                    &binding.name_font,
                    &widget.name_characters,
                    TextStyle::dialogue(),
                )
                .map_err(|e| format!("owner geometry: {e:?}"))?;
                *meshes
                    .get_mut(&binding.owner_mesh)
                    .ok_or("missing owner mesh")? =
                    geometry_mesh(&geometry, geometry.quads.len() as i32)?;
                binding.name_cache = Some(widget.name_characters.clone());
            }
            body.typing = widget.state.typing.clone();
            Ok(())
        })();
        if let Err(error) = result {
            *session.failure = Some(error);
            return;
        }
    }
}
fn publish_typing<R: DialogView<W>, W>(
    session: Option<ResMut<R>>,
    bindings: Query<(&NativeDialogBox, &NativeBitmapText)>,
) where
    W: DialogTextFrameWorld + Send + Sync + 'static,
    <W as MessageHost>::Error: std::fmt::Debug,
{
    let Some(mut session) = session else { return };
    let Some(session) = session.view() else {
        return;
    };
    if session.failure.is_some() {
        return;
    }
    for (binding, text) in &bindings {
        if let Some(error) = &text.failure {
            *session.failure = Some(error.clone());
            return;
        }
        if !text.suspended {
            let widget = if binding.cutscene {
                Some(&mut session.host.cutscene.widget)
            } else if binding.additional {
                session.host.additional.get_mut(binding.slot)
            } else {
                session.host.boxes.get_mut(binding.slot)
            };
            if let Some(widget) = widget {
                widget.state.typing = text.typing.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_text::NativeTextPlugin;
    use magicka_content::assets::{BitmapFont, Glyph};
    use magicka_runtime::{
        dialog_definition::{DialogDefinition, MessageDefinition},
        dialog_manager::DialogManagerEffect,
        dialog_text_host::{DialogMessageKey, DialogTextWorld},
        localization::LocalizationTable,
        message_playback::{MessageEffect, MessageOwner},
        message_textbox::MessageTextBoxWorld,
        scene_loading::custom_hash,
        text_box::TextBoxOwner,
        text_box_initialize::TextBoxInitHost,
    };
    #[derive(Default)]
    struct World {
        cutscene: bool,
    }
    impl MessageHost for World {
        type Error = &'static str;
        fn resolve_owner(&mut self, _: i32) -> Result<Option<MessageOwner>, Self::Error> {
            Ok(None)
        }
        fn cue_stopping(&self) -> Option<bool> {
            None
        }
        fn effect(&mut self, _: &MessageDefinition, _: MessageEffect) -> Result<(), Self::Error> {
            Err("no world effects")
        }
    }
    impl TextBoxInitHost for World {
        type Error = &'static str;
        fn load_background(&mut self, _: &str) -> Result<(), Self::Error> {
            Ok(())
        }
    }
    impl MessageTextBoxWorld for World {
        fn entity_scene(&mut self, _: i32) -> Result<Option<i32>, &'static str> {
            Ok(Some(1))
        }
    }
    impl DialogTextWorld for World {
        fn bind_message(&mut self, _: DialogMessageKey) -> Result<(), &'static str> {
            Ok(())
        }
        fn lock_controller(&mut self, _: i32) -> Result<(), &'static str> {
            Ok(())
        }
        fn unlock_controller(&mut self, _: i32) -> Result<(), &'static str> {
            Ok(())
        }
        fn stop_message_cue(&mut self) -> Result<(), &'static str> {
            Ok(())
        }
        fn manager_effect(&mut self, _: DialogManagerEffect) -> Result<(), &'static str> {
            Ok(())
        }
    }
    impl DialogTextFrameWorld for World {
        fn text_owner(&self, _: i32) -> Option<TextBoxOwner> {
            None
        }
        fn owner_dead(&self, _: i32) -> bool {
            false
        }
        fn in_cutscene(&self) -> bool {
            self.cutscene
        }
        fn message_box_count(&self) -> usize {
            0
        }
        fn update_message_box(&mut self, _: usize, _: DialogFrame) -> Result<(), &'static str> {
            Err("no message boxes")
        }
        fn message_box_dead(&self, _: usize) -> bool {
            false
        }
        fn remove_message_box(&mut self, _: usize) -> Result<(), &'static str> {
            Err("no message boxes")
        }
        fn submit_subtitles(&mut self, _: usize, _: f32) -> Result<(), &'static str> {
            Err("no subtitles")
        }
    }
    fn fixture() -> (App, Entity, Handle<Mesh>) {
        let content = BitmapFont {
            line_height: 10,
            base_line: 8,
            default_character: '?',
            texture: None,
            glyphs: ['A', 'B', '?']
                .into_iter()
                .map(|character| Glyph {
                    character,
                    origin: [0; 2],
                    size: [4, 5],
                    advance_width: 5,
                    left_side_bearing: 0,
                    force_white: false,
                })
                .collect(),
            kerning: vec![],
        };
        let font = Arc::new(GeometryFont::with_atlas_size(&content, [16, 16]).unwrap());
        let definitions = DialogDefinition::read_collection(&[br##"<Dialogs><Dialog id="test" repeatFrom="-1"><Interact><Message character="#owner"><Text ttl="0.1">#one</Text><Text ttl="0.2">#two</Text></Message></Interact></Dialog></Dialogs>"##]).unwrap();
        let mut strings = LocalizationTable::default();
        for (id, text) in [("#owner", "A"), ("#one", "AA"), ("#two", "BB")] {
            strings.strings.insert(custom_hash(id), text.into());
        }
        let mut host = DialogTextHost::new(
            World::default(),
            strings,
            font.metrics.clone(),
            font.metrics.clone(),
            100,
        );
        host.scene = Some(1);
        host.initialize_layouts(&definitions).unwrap();
        let mut session =
            NativeDialogSession::new(DialogManager::new(definitions).unwrap(), host, [800., 600.]);
        session.commands.push_back(NativeDialogCommand::Start {
            dialog: custom_hash("test"),
            location: InteractionLocation::Screen([0.; 2]),
            controller: None,
        });
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_resource::<Assets<Mesh>>()
            .add_plugins((NativeTextPlugin, NativeDialogPlugin::<World>::default()))
            .insert_resource(session);
        app.world_mut().resource_mut::<NativeTextDelta>().0 = Some(0.);
        let mut mesh_asset = app.world_mut().resource_mut::<Assets<Mesh>>();
        let bg = mesh_asset.add(geometry_mesh(&TextGeometry::default(), 0).unwrap());
        let name = mesh_asset.add(geometry_mesh(&TextGeometry::default(), 0).unwrap());
        let body = mesh_asset.add(geometry_mesh(&TextGeometry::default(), 0).unwrap());
        let background = app
            .world_mut()
            .spawn((Transform::default(), Visibility::Hidden))
            .id();
        let owner = app
            .world_mut()
            .spawn((Transform::default(), Visibility::Hidden))
            .id();
        let entity = app
            .world_mut()
            .spawn((
                NativeBitmapText::new(font.clone(), "", TextStyle::dialogue()),
                Mesh2d(body.clone()),
                Transform::default(),
                Visibility::Hidden,
                NativeDialogBox::new(0, background, bg, owner, name, font),
            ))
            .id();
        (app, entity, body)
    }
    #[test]
    fn retained_session_orders_geometry_publication_reveal_ttl_and_hide() {
        let (mut app, entity, mesh) = fixture();
        app.update();
        let session = app.world().resource::<NativeDialogSession<World>>();
        assert!(session.failure.is_none());
        assert_eq!(session.host.boxes[0].state.typing.primitive_count, 4);
        assert_eq!(session.host.boxes[0].state.typing.visible_characters, 0);
        let binding = app.world().get::<NativeDialogBox>(entity).unwrap();
        let background_z = app
            .world()
            .get::<Transform>(binding.background)
            .unwrap()
            .translation
            .z;
        let text_z = app.world().get::<Transform>(entity).unwrap().translation.z;
        assert!(98. < background_z && background_z < text_z && text_z < 600.);
        assert_eq!(
            app.world()
                .get::<Transform>(binding.owner_name)
                .unwrap()
                .translation
                .z,
            text_z
        );
        app.world_mut().resource_mut::<NativeTextDelta>().0 = Some(0.25);
        app.update();
        let session = app.world().resource::<NativeDialogSession<World>>();
        assert!(session.failure.is_none());
        assert_eq!(session.host.boxes[0].state.typing.visible_characters, 2);
        assert_eq!(session.host.boxes[0].state.ttl, 0.1); // unfinished at Update entry
        assert_eq!(
            app.world()
                .resource::<Assets<Mesh>>()
                .get(&mesh)
                .unwrap()
                .indices()
                .unwrap()
                .len(),
            12
        );
        app.world_mut().resource_mut::<NativeTextDelta>().0 = Some(0.11);
        app.update();
        let session = app.world().resource::<NativeDialogSession<World>>();
        assert!(session.failure.is_none());
        assert_eq!(session.host.boxes[0].state.text.as_deref(), Some("BB"));
        assert_eq!(session.host.boxes[0].state.ttl, 0.2);
        app.world_mut()
            .resource_mut::<NativeDialogSession<World>>()
            .commands
            .push_back(NativeDialogCommand::EndAll);
        app.world_mut().resource_mut::<NativeTextDelta>().0 = Some(0.125);
        app.update();
        let session = app.world().resource::<NativeDialogSession<World>>();
        assert!(session.failure.is_none());
        assert!(session.manager.active().iter().all(Option::is_none));
        assert!(!session.host.boxes[0].state.visible());
        assert!(
            app.world()
                .get::<NativeBitmapText>(entity)
                .unwrap()
                .suspended
        );
        assert_eq!(
            *app.world().get::<Visibility>(entity).unwrap(),
            Visibility::Hidden
        );
    }
    #[test]
    fn missing_render_asset_records_failure_without_fabricating_mesh() {
        let (mut app, entity, _) = fixture();
        let handle = app
            .world()
            .get::<NativeDialogBox>(entity)
            .unwrap()
            .background_mesh
            .clone();
        app.world_mut()
            .resource_mut::<Assets<Mesh>>()
            .remove(&handle);
        app.update();
        let session = app.world().resource::<NativeDialogSession<World>>();
        assert_eq!(session.failure.as_deref(), Some("missing background mesh"));
        assert!(session.manager.dialog_active(custom_hash("test")));
        assert_eq!(session.host.boxes[0].state.typing.primitive_count, 0);
    }
    #[test]
    fn queued_collection_switch_preserves_old_box_and_retains_previous_progress() {
        let (mut app, entity, _) = fixture();
        app.update();
        let collection = magicka_runtime::dialog_manager::DialogCollectionState::new(5,
            DialogDefinition::read_collection(&[b"<Dialogs><Dialog id=\"other\"><Interact><Message><Text>#one</Text></Message></Interact></Dialog></Dialogs>"]).unwrap()).unwrap();
        app.world_mut()
            .resource_mut::<NativeDialogSession<World>>()
            .commands
            .push_back(NativeDialogCommand::SetDialogs(collection));
        app.update();
        let session = app.world().resource::<NativeDialogSession<World>>();
        assert!(session.failure.is_none());
        assert_eq!(session.manager.collection_token(), 5);
        assert_eq!(session.host.collection_token, 5);
        assert!(session.manager.active().iter().all(Option::is_none));
        assert_eq!(session.replaced_collections.len(), 1);
        assert_eq!(session.replaced_collections[0].token, 0);
        assert!(session.host.boxes[0].state.grow);
        assert!(
            !app.world()
                .get::<NativeBitmapText>(entity)
                .unwrap()
                .suspended
        );
        assert_eq!(session.host.boxes[0].state.text.as_deref(), Some("AA"));
    }
    #[test]
    fn cutscene_binding_draws_final_submission_then_suspends_without_further_update() {
        let (mut app, entity, _) = fixture();
        app.world_mut()
            .get_mut::<NativeDialogBox>(entity)
            .unwrap()
            .cutscene = true;
        app.world_mut()
            .resource_mut::<NativeDialogSession<World>>()
            .host
            .world
            .cutscene = true;
        app.update();
        let session = app.world().resource::<NativeDialogSession<World>>();
        assert!(session.failure.is_none());
        assert_eq!(session.host.cutscene.widget.state.size[0], 640);
        assert_eq!(session.host.cutscene.widget.state.typing.primitive_count, 4);
        assert!(!session.host.boxes[0].state.scene_initialized);
        app.world_mut().resource_mut::<NativeTextDelta>().0 = Some(0.25);
        app.update();
        app.world_mut().resource_mut::<NativeTextDelta>().0 = Some(0.11);
        app.update();
        assert_eq!(
            app.world()
                .resource::<NativeDialogSession<World>>()
                .host
                .cutscene
                .widget
                .state
                .text
                .as_deref(),
            Some("BB")
        );
        app.world_mut().resource_mut::<NativeTextDelta>().0 = Some(0.21);
        app.update();
        let session = app.world().resource::<NativeDialogSession<World>>();
        assert!(session.failure.is_none());
        assert!(!session.manager.draw_cutscene_text);
        assert!(session.host.cutscene_submitted[0]);
        assert!(
            !app.world()
                .get::<NativeBitmapText>(entity)
                .unwrap()
                .suspended
        );
        app.world_mut().resource_mut::<NativeTextDelta>().0 = Some(0.);
        app.update();
        let session = app.world().resource::<NativeDialogSession<World>>();
        assert!(!session.host.cutscene_submitted[0]);
        assert_eq!(session.host.cutscene.widget.state.scale, 1.);
        assert!(
            app.world()
                .get::<NativeBitmapText>(entity)
                .unwrap()
                .suspended
        );
    }
}
