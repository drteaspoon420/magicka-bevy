//! Native abstract MessageBox lifecycle, before subclass input/text drawing.
//! Fade belongs to Draw, not Update. See research/message-box.md.
pub const MESSAGE_BOX_SIZE: [f32; 2] = [448., 560.];
pub const MESSAGE_BOX_Z_INDEX: i32 = 2000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MessageBoxState {
    pub center: [f32; 2],
    pub alpha: f32,
    pub killed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MessageBoxDraw {
    pub center: [f32; 2],
    pub color: [f32; 4],
}

impl MessageBoxState {
    pub fn new(screen: [f32; 2]) -> Self {
        Self {
            center: [screen[0] * 0.5, screen[1] * 0.5],
            alpha: 0.,
            killed: true,
        }
    }
    /// Caller must also perform DialogManager.AddMessageBox registration.
    pub fn show(&mut self) {
        self.alpha = 0.;
        self.killed = false;
    }
    pub fn kill(&mut self) {
        self.killed = true;
    }
    /// System.Single.Epsilon is the smallest positive subnormal, not Rust EPSILON.
    pub fn dead(&self) -> bool {
        self.killed & (self.alpha < f32::from_bits(1))
    }
    /// True means submit this object to the channel's GUI list. No fading here.
    pub fn update(&mut self, screen: [f32; 2], daisy_wheel_displaying: bool) -> bool {
        if daisy_wheel_displaying {
            return false;
        }
        self.center = [screen[0] * 0.5, screen[1] * 0.5];
        true
    }
    pub fn draw(
        &mut self,
        dt: f32,
        screen: [f32; 2],
        daisy_wheel_displaying: bool,
    ) -> Option<MessageBoxDraw> {
        if daisy_wheel_displaying {
            return None;
        }
        self.center = [screen[0] * 0.5, screen[1] * 0.5];
        self.advance_alpha(dt);
        Some(MessageBoxDraw {
            center: self.center,
            color: [1., 1., 1., self.alpha],
        })
    }
    /// Shared arithmetic only; derived WaitingMessageBox owns its own draw gate
    /// and does not refresh the retained text center during Draw.
    pub fn advance_alpha(&mut self, dt: f32) {
        let next = if self.killed {
            self.alpha - dt * 4.
        } else {
            self.alpha + dt * 4.
        };
        // CLR Math.Min/Max propagate NaN. Preserve one-sided native clamping,
        // including negative deltas, rather than imposing a generic [0,1] clamp.
        self.alpha = if next.is_nan() {
            next
        } else if self.killed {
            next.max(0.)
        } else {
            next.min(1.)
        };
    }
}

/// Constructor's four position/UV vertices; original UI/Menu/Pages atlas.
pub fn background_vertices(texture_size: [f32; 2]) -> [[f32; 4]; 4] {
    let [w, h] = texture_size;
    let x = 1. / w;
    let y = 1. / h;
    [
        [-224., -280., 0., 464. * y],
        [224., -280., 448. * x, 464. * y],
        [224., 280., 448. * x, 1024. * y],
        [-224., 280., 0., 1024. * y],
    ]
}
