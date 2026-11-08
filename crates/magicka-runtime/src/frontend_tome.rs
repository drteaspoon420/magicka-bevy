//! The menu Tome (`Magicka.GameLogic.UI.Tome`): desk camera clips, book and
//! page clips, page riffles, the candle light and the submenu stack, as a
//! presentation-independent state machine. A renderer samples the named clips
//! at the reported times; see research/frontend-tome.md.
//!
//! Clip timing follows XNAnimation's AnimationController: time advances by
//! `dt * speed * clip_speed`, a non-looping clip clamps at its duration and
//! finishes unless a cross-fade is running, and a cross-fade blends the
//! running clip toward the *first keyframe* of the next clip before starting
//! it from zero.

/// A named clip and its duration in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clip {
    pub name: &'static str,
    pub duration: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CrossFade {
    clip: Clip,
    looping: bool,
    duration: f32,
    time: f32,
    queued: Option<(Clip, f32)>,
}

/// XNAnimation.Controllers.AnimationController, reduced to clip timing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controller {
    clip: Option<Clip>,
    time: f32,
    pub speed: f32,
    pub clip_speed: f32,
    pub forward: bool,
    looping: bool,
    playing: bool,
    finished: bool,
    cross_fade: Option<CrossFade>,
}
impl Default for Controller {
    fn default() -> Self {
        Self {
            clip: None,
            time: 0.0,
            speed: 1.0,
            clip_speed: 1.0,
            forward: true,
            looping: false,
            playing: false,
            finished: false,
            cross_fade: None,
        }
    }
}

/// What a renderer should pose: `clip` at `time`, blended by `amount` toward
/// the first keyframe of `toward` while a cross-fade runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub clip: &'static str,
    pub time: f32,
    pub toward: Option<(&'static str, f32)>,
}

impl Controller {
    pub fn start(&mut self, clip: Clip, looping: bool) {
        self.clip = Some(clip);
        self.finished = false;
        self.playing = true;
        self.looping = looping;
        self.cross_fade = None;
        self.time = if self.forward { 0.0 } else { clip.duration };
    }
    pub fn cross_fade(&mut self, clip: Clip, duration: f32, looping: bool) {
        self.playing = true;
        if let Some(fade) = &mut self.cross_fade {
            fade.queued = Some((clip, duration));
            fade.looping = looping;
            fade.duration = fade.time + (fade.duration - fade.time) * 0.5;
            return;
        }
        self.cross_fade = Some(CrossFade {
            clip,
            looping,
            duration,
            time: 0.0,
            queued: None,
        });
        self.finished = false;
    }
    pub fn stop(&mut self) {
        self.playing = false;
    }
    pub fn update(&mut self, dt: f32) {
        let Some(clip) = self.clip else { return };
        let dt = dt * self.speed * self.clip_speed;
        if self.playing {
            self.time += if self.forward { dt } else { -dt };
            if self.time < 0.0 || self.time > clip.duration {
                if self.looping {
                    while self.time > clip.duration {
                        self.time -= clip.duration;
                    }
                    while self.time < 0.0 {
                        self.time += clip.duration;
                    }
                } else {
                    self.time = self.time.clamp(0.0, clip.duration);
                    if self.cross_fade.is_none() {
                        self.playing = false;
                        self.finished = true;
                    }
                }
            }
        }
        if let Some(fade) = &mut self.cross_fade {
            fade.time += dt;
            if fade.time > fade.duration {
                let fade = *fade;
                self.start(fade.clip, fade.looping);
                if let Some((next, duration)) = fade.queued {
                    self.cross_fade(next, duration, fade.looping);
                }
            }
        }
    }
    pub fn clip(&self) -> Option<Clip> {
        self.clip
    }
    pub fn time(&self) -> f32 {
        self.time
    }
    pub fn is_playing(&self) -> bool {
        self.playing
    }
    pub fn has_finished(&self) -> bool {
        self.finished
    }
    pub fn cross_fading(&self) -> bool {
        self.cross_fade.is_some()
    }
    pub fn pose(&self) -> Option<Pose> {
        let clip = self.clip?;
        Some(Pose {
            clip: clip.name,
            time: self.time,
            toward: self.cross_fade.map(|f| {
                let amount = if f.duration > f32::EPSILON {
                    f.time / f.duration
                } else {
                    1.0
                };
                (f.clip.name, amount)
            }),
        })
    }
}

/// Tome.CameraAnimation, indexing the background rig's clips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraClip {
    WakeUp,
    Idle,
    LookLeft,
    LookBack,
    DozeOff,
    ZoomedIn,
}
const CAMERA_NAMES: [&str; 6] = [
    "wakeup",
    "idle",
    "lookleft",
    "lookback",
    "dozeoff",
    "exitlevel",
];

/// Durations of every clip the Tome uses, from the converted models.
#[derive(Clone, Copy, Debug)]
pub struct TomeClips {
    camera: [Clip; 6],
    tome_open: Clip,
    tome_close: Clip,
    /// page_flip open_left, open_right, close_left, close_right (posing the
    /// two resting `page` meshes).
    pages: [Clip; 4],
    /// page_flip flip_left, flip_right (the riffled pages).
    flips: [Clip; 2],
}
impl TomeClips {
    /// `background`, `tome` and `page_flip` clip durations by name.
    pub fn from_lookup(
        background: impl Fn(&str) -> Option<f32>,
        tome: impl Fn(&str) -> Option<f32>,
        page_flip: impl Fn(&str) -> Option<f32>,
    ) -> Result<Self, String> {
        fn clip(lookup: &impl Fn(&str) -> Option<f32>, name: &'static str) -> Result<Clip, String> {
            lookup(name)
                .map(|duration| Clip { name, duration })
                .ok_or_else(|| format!("tome clip {name} missing"))
        }
        let mut camera = [Clip {
            name: "",
            duration: 0.0,
        }; 6];
        for (slot, name) in camera.iter_mut().zip(CAMERA_NAMES) {
            *slot = clip(&background, name)?;
        }
        Ok(Self {
            camera,
            tome_open: clip(&tome, "open")?,
            tome_close: clip(&tome, "close")?,
            pages: [
                clip(&page_flip, "open_left")?,
                clip(&page_flip, "open_right")?,
                clip(&page_flip, "close_left")?,
                clip(&page_flip, "close_right")?,
            ],
            flips: [
                clip(&page_flip, "flip_left")?,
                clip(&page_flip, "flip_right")?,
            ],
        })
    }
    pub fn camera(&self, clip: CameraClip) -> Clip {
        self.camera[clip as usize]
    }
}

/// Tome.MaxNrOfPages.
pub const MAX_PAGES: usize = 10;

/// Tome.RiffleController: up to ten staggered page flips.
#[derive(Clone, Debug)]
pub struct Riffle {
    controllers: [Controller; MAX_PAGES],
    clip: Clip,
    offset: f32,
    timer: f32,
    active: usize,
    left_to_activate: usize,
}
impl Riffle {
    fn new(flip_left: Clip) -> Self {
        Self {
            controllers: [Controller::default(); MAX_PAGES],
            clip: flip_left,
            offset: flip_left.duration * 0.625 / MAX_PAGES as f32,
            timer: 0.0,
            active: 0,
            left_to_activate: 0,
        }
    }
    fn start(&mut self, clip: Clip, pages: usize) {
        let pages = pages.min(MAX_PAGES);
        self.clip = clip;
        self.active = pages;
        self.left_to_activate = pages;
        for controller in &mut self.controllers[..pages] {
            controller.speed = 2.0;
            controller.forward = true;
            controller.start(clip, false);
        }
    }
    fn update(&mut self, dt: f32) {
        self.timer -= dt;
        while self.timer <= 0.0 && self.left_to_activate > 0 {
            self.left_to_activate -= 1;
            let controller = &mut self.controllers[self.left_to_activate];
            controller.start(self.clip, false);
            controller.update(-self.timer + 0.01);
            self.timer += self.offset;
        }
        if self.left_to_activate == 0 {
            self.timer = 0.0;
        }
        for controller in &mut self.controllers {
            if controller.is_playing() {
                controller.update(dt);
            }
        }
    }
    pub fn finished(&self) -> bool {
        self.controllers[..self.active]
            .iter()
            .all(Controller::has_finished)
    }
    /// The pages drawn this frame (`ControllersPlaying`).
    pub fn pages(&self) -> &[Controller] {
        &self.controllers[..self.active]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TomeState {
    Closed,
    Opening,
    Open,
    Closing,
    RiffleForward,
    RiffleBackward,
    CloseToBack,
    BackToFront,
    ClosedBack,
}

/// The four 1024-square regions of the 2048-square page render target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sheet {
    TomeLeft,
    TomeRight,
    PageLeft,
    PageRight,
}
impl Sheet {
    /// Top-left pixel of the sheet in the render target.
    pub fn origin(self) -> [f32; 2] {
        match self {
            Sheet::TomeLeft => [0.0, 0.0],
            Sheet::TomeRight => [1024.0, 0.0],
            Sheet::PageLeft => [0.0, 1024.0],
            Sheet::PageRight => [1024.0, 1024.0],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TomeEvent {
    /// `ui_menu_bookopen`.
    BookOpen,
    /// `ui_menu_bookclose`.
    BookClose,
    /// `ui_menu_tomepage` with its Pages variable.
    Pages(usize),
    /// Tome.OnOpen.
    Opened,
    /// Tome.OnClose (ClosedState.OnEnter after a close).
    Closed,
    /// Tome.OnBackClose.
    BackClosed,
}

/// The candle light: Tome.Update's intensity/flicker integration.
#[derive(Clone, Copy, Debug)]
pub struct Light {
    pub intensity: f32,
    pub target: f32,
    pub variation_speed: f32,
    pub variation_amount: f32,
    pub target_variation_speed: f32,
    pub target_variation_amount: f32,
    variation_position: f32,
}
impl Default for Light {
    fn default() -> Self {
        Self {
            intensity: 0.0,
            // The constructor assigns TargetLightIntensity = 0.
            target: 0.0,
            variation_speed: 24.0,
            variation_amount: 1.0,
            target_variation_speed: 6.0,
            target_variation_amount: 0.2,
            variation_position: 0.0,
        }
    }
}
impl Light {
    fn update(&mut self, dt: f32, random: f32) {
        self.variation_speed += (self.target_variation_speed - self.variation_speed) * dt * 5.0;
        self.variation_amount += (self.target_variation_amount - self.variation_amount) * dt * 5.0;
        self.intensity += (self.target - self.intensity) * dt * self.variation_speed;
        self.variation_position += (random - 0.5) * 2.0 * dt * self.variation_speed;
        self.variation_position = self.variation_position.clamp(-1.0, 1.0);
    }
    /// The factor applied to the light colours (ambient 0.3/0.2/0.2,
    /// diffuse 0.7/0.75/0.75, specular 1.3/1/1).
    pub fn factor(&self) -> f32 {
        (self.intensity + self.variation_position * self.variation_amount) * 100.0
    }
}
pub const LIGHT_AMBIENT: [f32; 3] = [0.3, 0.2, 0.2];
pub const LIGHT_DIFFUSE: [f32; 3] = [0.7, 0.75, 0.75];
pub const LIGHT_SPECULAR: [f32; 3] = [1.3, 1.0, 1.0];

/// One submenu to draw and the sheets for its left and right pages.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetDraw<M> {
    pub menu: M,
    pub left: Sheet,
    pub right: Sheet,
    /// The menu's previous content, on the sheets turning away during a
    /// riffle within one menu (SubMenuIntro/SubMenuCutscene.DrawNewAndOld).
    pub old: bool,
}

#[derive(Clone, Debug)]
pub struct Tome<M> {
    clips: TomeClips,
    root: M,
    pub state: TomeState,
    pub camera: Controller,
    pub book: Controller,
    pub left_page: Controller,
    pub right_page: Controller,
    pub riffle: Riffle,
    pub light: Light,
    menus: Vec<M>,
    riffles: Vec<usize>,
    /// The menu just popped; it still draws on the page turning back.
    popped: Option<M>,
    /// The running riffle turns pages within the current menu (Riffle),
    /// not between menus (PushMenu/PopMenu).
    same_menu: bool,
    intro_time: f32,
    hurry: bool,
    input_locked: bool,
    events: Vec<TomeEvent>,
    /// Draw the riffled pages this frame (Riffle states only).
    pub riffling: bool,
}

impl<M: Copy + PartialEq> Tome<M> {
    /// The Tome after CompanyState.OnExit: closed, camera at the start of
    /// wakeup (stopped), light off. `root` is SubMenuMain.
    pub fn new(clips: TomeClips, root: M) -> Self {
        let mut tome = Self {
            clips,
            root,
            state: TomeState::Closed,
            camera: Controller::default(),
            book: Controller::default(),
            left_page: Controller::default(),
            right_page: Controller::default(),
            riffle: Riffle::new(clips.flips[0]),
            light: Light::default(),
            menus: Vec::new(),
            riffles: Vec::new(),
            popped: None,
            same_menu: false,
            intro_time: 0.0,
            hurry: false,
            input_locked: false,
            events: Vec::new(),
            riffling: false,
        };
        tome.enter(TomeState::Closed);
        tome.events.clear();
        tome.set_camera(CameraClip::WakeUp);
        tome
    }

    pub fn set_camera(&mut self, clip: CameraClip) {
        self.camera.start(self.clips.camera(clip), false);
        self.camera.stop();
    }
    pub fn play_camera(&mut self, clip: CameraClip) {
        self.camera.start(self.clips.camera(clip), false);
    }
    pub fn cross_fade_camera(&mut self, clip: CameraClip, duration: f32) {
        self.camera
            .cross_fade(self.clips.camera(clip), duration, false);
    }

    pub fn take_events(&mut self) -> Vec<TomeEvent> {
        std::mem::take(&mut self.events)
    }
    pub fn input_locked(&self) -> bool {
        self.input_locked
    }
    pub fn current(&self) -> Option<M> {
        self.menus.last().copied()
    }
    pub fn depth(&self) -> usize {
        self.menus.len()
    }
    pub fn intro_time(&self) -> f32 {
        self.intro_time
    }

    fn change(&mut self, state: TomeState) {
        self.state = state;
        self.enter(state);
    }

    fn enter(&mut self, state: TomeState) {
        let clips = self.clips;
        self.riffling = matches!(state, TomeState::RiffleForward | TomeState::RiffleBackward);
        match state {
            TomeState::Closed => {
                for (controller, clip) in [
                    (&mut self.book, clips.tome_close),
                    (&mut self.left_page, clips.pages[2]),
                    (&mut self.right_page, clips.pages[3]),
                ] {
                    controller.start(clip, false);
                    controller.update(clip.duration - 0.001);
                    controller.update(0.01);
                }
                self.events.push(TomeEvent::Closed);
            }
            TomeState::Open => {
                for (controller, clip) in [
                    (&mut self.book, clips.tome_open),
                    (&mut self.left_page, clips.pages[0]),
                    (&mut self.right_page, clips.pages[1]),
                ] {
                    controller.forward = true;
                    controller.start(clip, false);
                    controller.update(clip.duration + 1.0);
                }
            }
            TomeState::Opening => {
                for (controller, clip) in [
                    (&mut self.book, clips.tome_open),
                    (&mut self.left_page, clips.pages[0]),
                    (&mut self.right_page, clips.pages[1]),
                ] {
                    controller.forward = true;
                    controller.start(clip, false);
                    controller.update(0.001);
                }
                self.events.push(TomeEvent::BookOpen);
                if self.menus.is_empty() {
                    self.menus.push(self.root);
                }
                self.cross_fade_camera(CameraClip::Idle, 0.75);
                self.events.push(TomeEvent::Opened);
            }
            TomeState::Closing | TomeState::CloseToBack => {
                self.book.start(clips.tome_close, false);
                self.left_page.start(clips.pages[2], false);
                self.right_page.start(clips.pages[3], false);
                self.events.push(TomeEvent::BookClose);
            }
            TomeState::BackToFront => {
                self.book.start(clips.tome_open, false);
                for (controller, clip) in [
                    (&mut self.left_page, clips.pages[0]),
                    (&mut self.right_page, clips.pages[1]),
                ] {
                    controller.forward = true;
                    controller.start(clip, false);
                    controller.update(0.001);
                }
                self.events.push(TomeEvent::BookClose);
            }
            TomeState::RiffleForward | TomeState::RiffleBackward => {
                let pages = self.riffles.last().copied().unwrap_or(0);
                let clip = clips.flips[usize::from(state == TomeState::RiffleBackward)];
                self.riffle.start(clip, pages);
                self.events.push(TomeEvent::Pages(pages));
            }
            TomeState::ClosedBack => self.events.push(TomeEvent::BackClosed),
        }
    }

    /// Tome.Update for one frame. `random` is a uniform sample in [0, 1).
    pub fn update(&mut self, dt: f32, random: f32) {
        self.intro_time += dt;
        let wakeup = self.clips.camera(CameraClip::WakeUp);
        let on_wakeup = self.camera.clip() == Some(wakeup);
        if self.hurry && on_wakeup && self.camera.has_finished() {
            self.change(TomeState::Opening);
        }
        if self.camera.clip() == Some(wakeup) {
            if self.intro_time > 1.5 || self.camera.clip_speed > 1.0 {
                self.light.target = 1.0;
            } else {
                if !self.camera.cross_fading() && !self.camera.is_playing() {
                    self.play_camera(CameraClip::WakeUp);
                }
                self.light.target = 0.0;
            }
        }
        self.light.update(dt, random);
        self.camera.update(dt);
        match self.state {
            TomeState::Opening | TomeState::Closing => {
                if self.book.is_playing() {
                    self.book.update(dt);
                    self.left_page.update(dt);
                    self.right_page.update(dt);
                } else if self.state == TomeState::Opening {
                    self.change(TomeState::Open);
                } else {
                    self.change(TomeState::Closed);
                }
            }
            TomeState::RiffleForward | TomeState::RiffleBackward => {
                if self.riffle.finished() {
                    self.change(TomeState::Open);
                } else {
                    self.riffle.update(dt);
                }
            }
            TomeState::CloseToBack | TomeState::BackToFront => {
                self.book.update(dt);
                self.left_page.update(dt);
                self.right_page.update(dt);
                if self.book.has_finished() && !self.book.cross_fading() {
                    self.change(if self.state == TomeState::CloseToBack {
                        TomeState::ClosedBack
                    } else {
                        TomeState::Open
                    });
                }
            }
            TomeState::Closed | TomeState::Open | TomeState::ClosedBack => {}
        }
        self.input_locked = !self.riffle.finished() || self.book.is_playing();
        if !self.riffle.finished() && !self.riffling {
            self.riffle.update(dt);
        }
    }

    /// Tome.ControllerA while closed: after a second, speed the wake-up
    /// six-fold and open when it ends. Returns whether the press was taken.
    pub fn press_open(&mut self) -> bool {
        if self.input_locked {
            return false;
        }
        match self.state {
            TomeState::ClosedBack => {
                self.change(TomeState::BackToFront);
                true
            }
            TomeState::Closed => {
                if self.intro_time > 1.0 && !self.hurry {
                    self.hurry = true;
                    self.camera.clip_speed *= 6.0;
                }
                true
            }
            _ => false,
        }
    }

    /// Tome.PushMenu: only while open; riffles `pages` pages forward.
    pub fn push(&mut self, menu: M, pages: usize) -> bool {
        if self.state != TomeState::Open {
            return false;
        }
        self.same_menu = false;
        self.menus.push(menu);
        self.riffles.push(pages);
        if pages > 0 {
            self.change(TomeState::RiffleForward);
        }
        true
    }
    /// Tome.PushMenuInstant.
    pub fn push_instant(&mut self, menu: M, pages: usize) {
        self.same_menu = false;
        self.menus.push(menu);
        self.riffles.push(pages);
    }
    /// Tome.PopMenu from the menu state: riffles back, or closes the book at
    /// the root.
    pub fn pop(&mut self) -> bool {
        if self.state != TomeState::Open {
            return false;
        }
        self.same_menu = false;
        if self.menus.len() > 1 {
            // The popped menu still draws on the turning page.
            self.popped = self.menus.pop();
            self.change(TomeState::RiffleBackward);
            self.riffles.pop();
        } else {
            self.change(TomeState::Closing);
        }
        true
    }
    /// Tome.PopMenuInstant.
    pub fn pop_instant(&mut self) {
        self.same_menu = false;
        if self.menus.len() > 1 {
            self.menus.pop();
            self.riffles.pop();
        }
    }
    /// Replace the menu stack below the current page (PopPreviousMenu).
    pub fn pop_previous(&mut self) {
        let n = self.menus.len();
        if n >= 2 {
            self.menus.remove(n - 2);
            self.riffles.pop();
        }
    }
    pub fn riffle(&mut self, forward: bool, pages: usize) {
        self.same_menu = true;
        self.riffles.push(pages);
        self.change(if forward {
            TomeState::RiffleForward
        } else {
            TomeState::RiffleBackward
        });
        self.riffles.pop();
    }
    pub fn close(&mut self) {
        self.change(TomeState::Closing);
    }
    pub fn close_to_back(&mut self) {
        self.change(TomeState::CloseToBack);
    }

    /// The menus to draw and their sheets (TomeState.Draw/DrawNewAndOld).
    pub fn draws(&self) -> Vec<SheetDraw<M>> {
        use Sheet::*;
        let Some(current) = self.current() else {
            return Vec::new();
        };
        let n = self.menus.len();
        let draw = |menu, left, right, old| SheetDraw {
            menu,
            left,
            right,
            old,
        };
        // The content on the sheets turning away: this menu's previous
        // content within a menu, else the menu below (forward) or the one
        // popped (backward).
        let below = if self.same_menu {
            Some((current, true))
        } else {
            (n > 1).then(|| (self.menus[n - 2], false))
        };
        let popped = if self.same_menu {
            Some((current, true))
        } else {
            self.popped.map(|m| (m, false))
        };
        match self.state {
            TomeState::Closed | TomeState::ClosedBack => Vec::new(),
            TomeState::Open => vec![draw(current, TomeLeft, TomeRight, false)],
            TomeState::RiffleForward => {
                let mut draws = vec![draw(current, PageLeft, TomeRight, false)];
                draws.extend(below.map(|(menu, old)| draw(menu, TomeLeft, PageRight, old)));
                draws
            }
            TomeState::RiffleBackward => {
                let mut draws = vec![draw(current, TomeLeft, PageRight, false)];
                draws.extend(popped.map(|(menu, old)| draw(menu, PageLeft, TomeRight, old)));
                draws
            }
            _ => {
                let mut draws = vec![draw(current, TomeLeft, TomeRight, false)];
                if n > 1 {
                    draws.push(draw(self.menus[n - 2], PageLeft, PageRight, false));
                }
                draws
            }
        }
    }
}

/// Tome.MousePickTome's page planes (world space, y = 0.5 with a 0.1 tilt).
struct Triangle {
    origin: [f32; 3],
    edge0: [f32; 3],
    edge1: [f32; 3],
}
const RIGHT_TRIS: [Triangle; 2] = [
    Triangle {
        origin: [-5.3, 0.5, 2.75],
        edge0: [5.3, -0.1, 0.0],
        edge1: [0.0, 0.0, -5.5],
    },
    Triangle {
        origin: [0.0, 0.5, -2.75],
        edge0: [-5.3, 0.1, 0.0],
        edge1: [0.0, 0.0, 5.5],
    },
];
const LEFT_TRIS: [Triangle; 2] = [
    Triangle {
        origin: [0.0, 0.4, 2.75],
        edge0: [5.3, 0.1, 0.0],
        edge1: [0.0, 0.0, -5.5],
    },
    Triangle {
        origin: [5.3, 0.5, -2.75],
        edge0: [-5.3, -0.1, 0.0],
        edge1: [0.0, 0.0, 5.5],
    },
];

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
/// JigLibX SegmentTriangleIntersection: segment parameter and the two edge
/// parameters of the hit (both sides accepted).
fn intersect(origin: [f32; 3], delta: [f32; 3], tri: &Triangle) -> Option<(f32, f32)> {
    let p = cross(delta, tri.edge1);
    let det = dot(tri.edge0, p);
    if det.abs() < 1e-8 {
        return None;
    }
    let inv = 1.0 / det;
    let s = sub(origin, tri.origin);
    let u = dot(s, p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, tri.edge0);
    let v = dot(delta, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot(tri.edge1, q) * inv;
    (0.0..=1.0).contains(&t).then_some((u, v))
}

/// A hit on the open book: pixel position within a 1024-square page sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageHit {
    pub position: [f32; 2],
    pub right_page: bool,
}

/// Tome.MousePickTome: the camera position and the world point under the
/// cursor at the far plane.
pub fn pick(camera: [f32; 3], far_point: [f32; 3]) -> Option<PageHit> {
    let delta = sub(far_point, camera);
    let hit = |tri: &Triangle| intersect(camera, delta, tri);
    let size = 1024.0;
    if let Some((t, t2)) = hit(&RIGHT_TRIS[0]) {
        return Some(PageHit {
            position: [(1.0 - t + 0.03) * 0.97 * size, t2 * size],
            right_page: true,
        });
    }
    if let Some((t, t2)) = hit(&RIGHT_TRIS[1]) {
        return Some(PageHit {
            position: [(t + 0.03) * 0.97 * size, (1.0 - t2) * size],
            right_page: true,
        });
    }
    if let Some((t, t2)) = hit(&LEFT_TRIS[0]) {
        return Some(PageHit {
            position: [(1.0 - t) * 0.97 * size, t2 * size],
            right_page: false,
        });
    }
    if let Some((t, t2)) = hit(&LEFT_TRIS[1]) {
        return Some(PageHit {
            position: [t * 0.97 * size, (1.0 - t2) * size],
            right_page: false,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Durations of the converted retail clips
    /// (output/evidence/frontend-handoff/tome-assets.json).
    fn clips() -> TomeClips {
        TomeClips::from_lookup(
            |n| {
                Some(match n {
                    "wakeup" => 8.333_333,
                    "idle" | "dozeoff" => 4.166_666_5,
                    "lookleft" | "lookback" | "exitlevel" => 2.083_333_3,
                    _ => return None,
                })
            },
            |n| {
                Some(match n {
                    "open" => 1.316_666_6,
                    "close" => 1.066_666_6,
                    _ => return None,
                })
            },
            |n| {
                Some(match n {
                    "flip_left" | "flip_right" => 0.983_333_3,
                    "open_left" | "open_right" | "close_left" | "close_right" => 1.316_666_6,
                    _ => return None,
                })
            },
        )
        .unwrap()
    }

    #[test]
    fn opening_waits_for_hurried_wakeup_then_cross_fades_to_idle() {
        let mut tome = Tome::new(clips(), 0u8);
        assert_eq!(tome.state, TomeState::Closed);
        // Presses during the first second are ignored.
        tome.update(0.5, 0.5);
        tome.press_open();
        assert_eq!(tome.camera.clip_speed, 1.0);
        tome.update(0.6, 0.5);
        assert!(tome.press_open());
        assert_eq!(tome.camera.clip_speed, 6.0);
        // 8.33 s of wakeup at 6x, starting from 1.1 s in: ~1.2 s remain.
        let mut t = 0.0;
        while tome.state == TomeState::Closed {
            tome.update(1.0 / 60.0, 0.5);
            t += 1.0 / 60.0;
            assert!(t < 2.0);
        }
        assert_eq!(tome.state, TomeState::Opening);
        assert!(t > 1.0, "{t}");
        assert!(tome.camera.cross_fading());
        assert_eq!(tome.current(), Some(0));
        let events = tome.take_events();
        assert!(events.contains(&TomeEvent::BookOpen));
        assert!(events.contains(&TomeEvent::Opened));
        while tome.state == TomeState::Opening {
            tome.update(1.0 / 60.0, 0.5);
        }
        assert_eq!(tome.state, TomeState::Open);
        assert_eq!(tome.camera.clip().unwrap().name, "idle");
        // The hurried wake-up raised the light; nothing lowers it afterwards.
        assert_eq!(tome.light.target, 1.0);
    }

    #[test]
    fn push_riffles_staggered_pages_and_pop_returns_with_the_old_page_turning() {
        let mut tome = Tome::new(clips(), 0u8);
        tome.change(TomeState::Opening);
        while tome.state != TomeState::Open {
            tome.update(1.0 / 60.0, 0.5);
        }
        tome.take_events();
        assert!(tome.push(1, 3));
        assert_eq!(tome.state, TomeState::RiffleForward);
        assert_eq!(tome.take_events(), vec![TomeEvent::Pages(3)]);
        assert_eq!(
            tome.draws(),
            vec![
                SheetDraw {
                    menu: 1,
                    left: Sheet::PageLeft,
                    right: Sheet::TomeRight,
                    old: false
                },
                SheetDraw {
                    menu: 0,
                    left: Sheet::TomeLeft,
                    right: Sheet::PageRight,
                    old: false
                },
            ]
        );
        // StartClip sets every page playing; activation (last page first,
        // one offset apart) restarts each from zero, as RiffleController does.
        for _ in 0..9 {
            tome.update(1.0 / 60.0, 0.5);
        }
        let times: Vec<f32> = tome.riffle.pages().iter().map(Controller::time).collect();
        assert!(times[2] > times[1] && times[1] > times[0], "{times:?}");
        assert!(tome.input_locked());
        let mut frames = 0;
        while tome.state != TomeState::Open {
            tome.update(1.0 / 60.0, 0.5);
            frames += 1;
        }
        // flip at speed 2 (0.49 s) plus two 0.06 s staggers.
        // 0.49 s of flip at speed 2 plus two 0.06 s staggers, from the push.
        assert!((34..=40).contains(&(frames + 9)), "{frames}");
        assert!(tome.pop());
        assert_eq!(tome.state, TomeState::RiffleBackward);
        assert_eq!(tome.current(), Some(0));
        assert_eq!(
            tome.draws(),
            vec![
                SheetDraw {
                    menu: 0,
                    left: Sheet::TomeLeft,
                    right: Sheet::PageRight,
                    old: false
                },
                SheetDraw {
                    menu: 1,
                    left: Sheet::PageLeft,
                    right: Sheet::TomeRight,
                    old: false
                },
            ]
        );
        while tome.state != TomeState::Open {
            tome.update(1.0 / 60.0, 0.5);
        }
        // A riffle within one menu turns its old content away.
        tome.riffle(true, 1);
        assert_eq!(tome.state, TomeState::RiffleForward);
        assert_eq!(
            tome.draws(),
            vec![
                SheetDraw {
                    menu: 0,
                    left: Sheet::PageLeft,
                    right: Sheet::TomeRight,
                    old: false
                },
                SheetDraw {
                    menu: 0,
                    left: Sheet::TomeLeft,
                    right: Sheet::PageRight,
                    old: true
                },
            ]
        );
        while tome.state != TomeState::Open {
            tome.update(1.0 / 60.0, 0.5);
        }
        // Popping the root closes the book.
        assert!(tome.pop());
        assert_eq!(tome.state, TomeState::Closing);
    }

    #[test]
    fn cross_fade_blends_toward_the_next_clip_then_starts_it() {
        let a = Clip {
            name: "a",
            duration: 2.0,
        };
        let b = Clip {
            name: "b",
            duration: 3.0,
        };
        let mut c = Controller::default();
        c.start(a, false);
        c.update(0.5);
        c.cross_fade(b, 1.0, false);
        c.update(0.25);
        let pose = c.pose().unwrap();
        assert_eq!((pose.clip, pose.time), ("a", 0.75));
        assert_eq!(pose.toward, Some(("b", 0.25)));
        c.update(1.0);
        let pose = c.pose().unwrap();
        assert_eq!((pose.clip, pose.time, pose.toward), ("b", 0.0, None));
    }

    #[test]
    fn pick_maps_the_right_page_plane_to_sheet_pixels() {
        // Straight down onto the right page's centre.
        let hit = pick([-2.65, 9.0, 0.0], [-2.65, -100.0, 0.0]).unwrap();
        assert!(hit.right_page);
        assert!(
            (hit.position[0] - 512.0 * 0.97 - 0.03 * 0.97 * 1024.0).abs() < 20.0,
            "{hit:?}"
        );
        assert!((hit.position[1] - 512.0).abs() < 2.0, "{hit:?}");
        let left = pick([2.65, 9.0, 0.0], [2.65, -100.0, 0.0]).unwrap();
        assert!(!left.right_page);
        assert!(pick([20.0, 9.0, 0.0], [20.0, -100.0, 0.0]).is_none());
    }
}
