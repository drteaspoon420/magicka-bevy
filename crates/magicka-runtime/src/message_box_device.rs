//! KeyboardMouseController's selected message-box branch; no device dependency.
use crate::dialog_input::DialogInputKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxDirection {
    Up,
    Down,
    Left,
    Right,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BoxDeviceSample {
    pub focused: bool,
    pub escape_down: bool,
    pub left_down: bool,
    pub point: [i32; 2],
    /// Producer compared cumulative vertical wheel values since its last poll.
    pub wheel_changed: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxDeviceEvent {
    pub kind: DialogInputKind,
    pub point: [i32; 2],
    pub left_released: bool,
    pub show_mouse: bool,
    pub direction: Option<BoxDirection>,
}
#[derive(Default)]
pub struct BoxDeviceState {
    last: BoxDeviceSample,
}
impl BoxDeviceState {
    /// Call even while unfocused or a higher-priority UI consumer owns input.
    pub fn update(
        &mut self,
        sample: BoxDeviceSample,
        box_branch_enabled: bool,
    ) -> Option<BoxDeviceEvent> {
        let old = self.last;
        self.last = sample;
        if !sample.focused || !box_branch_enabled {
            return None;
        }
        let (kind, show_mouse) = if sample.escape_down && !old.escape_down {
            (DialogInputKind::Escape, false)
        } else if sample.left_down != old.left_down || sample.wheel_changed {
            (DialogInputKind::MouseClick, false)
        } else if sample.point != old.point {
            (DialogInputKind::MouseMove, true)
        } else {
            return None;
        };
        Some(BoxDeviceEvent {
            kind,
            point: sample.point,
            left_released: !sample.left_down,
            show_mouse,
            direction: None,
        })
    }
    /// Native window KeyDown callback, including auto-repeat. Higher-priority
    /// key capture/account widgets are represented by the caller's enabled fact.
    pub fn key_down(vk: i32, box_key_branch_enabled: bool) -> Option<BoxDeviceEvent> {
        if !box_key_branch_enabled {
            return None;
        }
        let direction = match vk {
            13 => None,
            38 => Some(BoxDirection::Up),
            40 => Some(BoxDirection::Down),
            37 => Some(BoxDirection::Left),
            39 => Some(BoxDirection::Right),
            _ => return None,
        };
        Some(BoxDeviceEvent {
            kind: if direction.is_some() {
                DialogInputKind::Move
            } else {
                DialogInputKind::Select
            },
            point: [0; 2],
            left_released: false,
            show_mouse: false,
            direction,
        })
    }
}
