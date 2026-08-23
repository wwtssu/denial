use std::collections::HashSet;
use std::error::Error;
use std::ffi::OsStr;

use denial_core::topology::OutputTransform;
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Device, DeviceCapability, Event,
    GestureBeginEvent, GestureSwipeUpdateEvent, InputEvent, KeyState, KeyboardKeyEvent,
    PointerAxisEvent, PointerButtonEvent, PointerMotionEvent, TouchEvent,
};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::session::libseat::LibSeatSession;
#[cfg(feature = "flutter")]
use smithay::desktop::{
    PopupKind, PopupManager, WindowSurfaceType, utils::under_from_surface_tree,
};
use smithay::input::keyboard::{FilterResult, KeyboardHandle, Keycode};
#[cfg(feature = "flutter")]
use smithay::input::keyboard::{XkbConfig, xkb};
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, MotionEvent, PointerHandle, RelativeMotionEvent,
};
#[cfg(feature = "flutter")]
use smithay::input::pointer::{CursorImageStatus, Focus, GrabStartData};
use smithay::input::touch::{DownEvent, MotionEvent as TouchMotionEvent, UpEvent};
use smithay::reexports::calloop::EventLoop;
#[cfg(feature = "flutter")]
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::input::event::pointer::PointerEventTrait;
use smithay::reexports::input::event::touch::TouchEventTrait;
use smithay::reexports::input::{Device as LibinputDevice, Libinput, TapButtonMap};
#[cfg(feature = "flutter")]
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
#[cfg(feature = "flutter")]
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
#[cfg(feature = "flutter")]
use smithay::utils::Serial;
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER};
use smithay::wayland::pointer_constraints::{PointerConstraint, with_pointer_constraint};
use tracing::{info, warn};

#[cfg(feature = "flutter")]
use super::super::PendingWindowEvent;
use super::super::RuntimeState;
#[cfg(test)]
use super::super::lifecycle::LifecycleState;
use super::super::lifecycle::ShutdownReason;
#[cfg(test)]
use super::super::native_shortcut::NativeEscapeShortcut;
use super::super::native_shortcut::{ShortcutDisposition, ShortcutTarget};
#[cfg(feature = "flutter")]
use super::super::settings::KeyboardSettings;
use super::super::settings::TouchpadSettings;
#[cfg(feature = "flutter")]
use super::super::window_grab::{
    LocalFlutterWindowGrab, MoveSurfaceGrab, ResizeEdges, ResizeSurfaceGrab, X11ResizeSurfaceGrab,
};
#[cfg(feature = "flutter")]
use super::super::wire::{
    InputLayoutSnapshot, InputRect, InputWindowRegion, WindowPlacementChange, WindowPlacementPhase,
};
#[cfg(feature = "flutter")]
use super::FlutterPointerPress;
use super::WaylandFrontend;
#[cfg(feature = "flutter")]
use super::input_source::init_joystick_activity;
use super::input_source::{InputBatchEvent, LibinputBatchSource};
#[cfg(feature = "flutter")]
use super::touch_gestures::{
    self, TouchGestureUpdate, TouchWindowTarget, WINDOW_TOUCH_CORNER_SIZE,
    WINDOW_TOUCH_STRIP_HEIGHT,
};

#[path = "input/flutter_route.rs"]
mod flutter_route;
#[path = "input/wayland_route.rs"]
mod wayland_route;

#[cfg(all(test, feature = "flutter"))]
use flutter_route::flutter_pointer_endpoint_is_synchronized;
#[cfg(feature = "flutter")]
use flutter_route::process_flutter_input_event;
use flutter_route::process_wayland_keyboard_transition;
#[cfg(feature = "flutter")]
pub(crate) use flutter_route::reconcile_flutter_pointer_route;
#[cfg(test)]
use flutter_route::scaled_axis_amount;
use wayland_route::process_wayland_input_event;

fn output_bound_absolute_position<E>(
    event: &E,
    bounds: Rectangle<i32, Logical>,
    transform: OutputTransform,
) -> Point<f64, Logical>
where
    E: AbsolutePositionEvent<LibinputInputBackend>,
{
    let (x, y) = transform.native_to_logical(
        event.x_transformed(1).clamp(0.0, 1.0),
        event.y_transformed(1).clamp(0.0, 1.0),
    );
    Point::from((
        f64::from(bounds.loc.x) + x * f64::from(bounds.size.w),
        f64::from(bounds.loc.y) + y * f64::from(bounds.size.h),
    ))
}

#[cfg(feature = "flutter")]
const BTN_LEFT: u32 = 0x110;
#[cfg(feature = "flutter")]
const BTN_RIGHT: u32 = 0x111;
#[cfg(feature = "flutter")]
const MAX_CLIENT_POINTER_PRESSES: usize = 16;

#[cfg(feature = "flutter")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShellKeyStroke {
    evdev_keycode: u32,
    shift: bool,
}

#[cfg(feature = "flutter")]
fn shell_text_key_stroke(character: char) -> Option<ShellKeyStroke> {
    let (evdev_keycode, shift) = match character {
        'a'..='z' => (
            [
                30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22,
                47, 17, 45, 21, 44,
            ][usize::from(character as u8 - b'a')],
            false,
        ),
        'A'..='Z' => (
            [
                30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22,
                47, 17, 45, 21, 44,
            ][usize::from(character as u8 - b'A')],
            true,
        ),
        '1' => (2, false),
        '2' => (3, false),
        '3' => (4, false),
        '4' => (5, false),
        '5' => (6, false),
        '6' => (7, false),
        '7' => (8, false),
        '8' => (9, false),
        '9' => (10, false),
        '0' => (11, false),
        '!' => (2, true),
        '@' => (3, true),
        '#' => (4, true),
        '$' => (5, true),
        '%' => (6, true),
        '^' => (7, true),
        '&' => (8, true),
        '*' => (9, true),
        '(' => (10, true),
        ')' => (11, true),
        '-' => (12, false),
        '_' => (12, true),
        '=' => (13, false),
        '+' => (13, true),
        '[' => (26, false),
        '{' => (26, true),
        ']' => (27, false),
        '}' => (27, true),
        ';' => (39, false),
        ':' => (39, true),
        '\'' => (40, false),
        '"' => (40, true),
        '`' => (41, false),
        '~' => (41, true),
        '\\' => (43, false),
        '|' => (43, true),
        ',' => (51, false),
        '<' => (51, true),
        '.' => (52, false),
        '>' => (52, true),
        '/' => (53, false),
        '?' => (53, true),
        ' ' => (57, false),
        _ => return None,
    };
    Some(ShellKeyStroke {
        evdev_keycode,
        shift,
    })
}

#[cfg(feature = "flutter")]
fn shell_named_key_stroke(key: &str) -> Option<ShellKeyStroke> {
    let (evdev_keycode, shift) = match key {
        "Escape" => (1, false),
        "BackSpace" | "Backspace" => (14, false),
        "Tab" => (15, false),
        "Return" | "Enter" => (28, false),
        "space" | "Space" => (57, false),
        "Up" => (103, false),
        "Left" => (105, false),
        "Right" => (106, false),
        "Down" => (108, false),
        "Delete" => (111, false),
        "comma" => (51, false),
        "period" => (52, false),
        "slash" => (53, false),
        "backslash" => (43, false),
        "minus" => (12, false),
        "equal" => (13, false),
        "apostrophe" => (40, false),
        "semicolon" => (39, false),
        "colon" => (39, true),
        "bracketleft" => (26, false),
        "bracketright" => (27, false),
        value if value.chars().count() == 1 => shell_text_key_stroke(value.chars().next()?)
            .map(|stroke| (stroke.evdev_keycode, stroke.shift))?,
        _ => return None,
    };
    Some(ShellKeyStroke {
        evdev_keycode,
        shift,
    })
}

#[cfg(feature = "flutter")]
fn inject_shell_key_stroke(
    state: &mut RuntimeState,
    keyboard: &KeyboardHandle<RuntimeState>,
    stroke: ShellKeyStroke,
    ctrl: bool,
    time: u32,
) -> bool {
    const XKB_KEYCODE_OFFSET: u32 = 8;
    const LEFT_CTRL: u32 = 29;
    const LEFT_SHIFT: u32 = 42;

    let keycode = Keycode::new(stroke.evdev_keycode + XKB_KEYCODE_OFFSET);
    if keyboard.pressed_keys().contains(&keycode) {
        warn!(
            keycode = stroke.evdev_keycode,
            "ignored shell keyboard key already held by another input source"
        );
        return false;
    }

    let modifiers = keyboard.modifier_state();
    let inject_ctrl = ctrl && !modifiers.ctrl;
    let inject_shift = stroke.shift && !modifiers.shift;
    let ctrl_keycode = Keycode::new(LEFT_CTRL + XKB_KEYCODE_OFFSET);
    let shift_keycode = Keycode::new(LEFT_SHIFT + XKB_KEYCODE_OFFSET);
    let mut delivered = false;
    let mut send = |state: &mut RuntimeState, keycode: Keycode, key_state: KeyState| {
        delivered |= process_keyboard_transition(state, keycode, key_state, time);
    };

    if inject_ctrl {
        send(state, ctrl_keycode, KeyState::Pressed);
    }
    if inject_shift {
        send(state, shift_keycode, KeyState::Pressed);
    }
    send(state, keycode, KeyState::Pressed);
    send(state, keycode, KeyState::Released);
    if inject_shift {
        send(state, shift_keycode, KeyState::Released);
    }
    if inject_ctrl {
        send(state, ctrl_keycode, KeyState::Released);
    }
    delivered
}

#[cfg(feature = "flutter")]
fn inject_shell_key_transition(
    state: &mut RuntimeState,
    keyboard: &KeyboardHandle<RuntimeState>,
    stroke: ShellKeyStroke,
    key_state: KeyState,
    time: u32,
) -> bool {
    const XKB_KEYCODE_OFFSET: u32 = 8;

    // Held modified keys require explicit modifier ownership. The OSK uses
    // this lifecycle only for unmodified Backspace; complete modified taps
    // continue through inject_shell_key_stroke().
    if stroke.shift {
        return false;
    }
    let keycode = Keycode::new(stroke.evdev_keycode + XKB_KEYCODE_OFFSET);
    let seat_pressed = keyboard.pressed_keys().contains(&keycode);
    let accepted = {
        let frontend = state.wayland.as_mut().expect("missing Wayland frontend");
        route_shell_key_transition(
            &mut frontend.shell_keyboard_keys,
            keycode.raw(),
            key_state,
            seat_pressed,
        )
    };
    accepted && process_keyboard_transition(state, keycode, key_state, time)
}

#[cfg(feature = "flutter")]
/// Turn one shell-keyboard intent into complete virtual key lifecycles.
///
/// These transitions enter the same focus, XKB, shortcut, Flutter and Wayland
/// router as libinput keyboard events. The software keyboard is an input
/// source, not a separate text-delivery protocol.
pub(crate) fn dispatch_shell_keyboard(
    state: &mut RuntimeState,
    command: &super::super::wire::KeyboardCommand,
) -> bool {
    let (keyboard, time) = {
        let frontend = state.wayland.as_ref().expect("missing Wayland frontend");
        (
            frontend.seat.get_keyboard().expect("seat has no keyboard"),
            frontend.start_time.elapsed().as_millis() as u32,
        )
    };
    // Do not gate the shared router on Wayland seat focus. Secure lock
    // deliberately clears client focus, and process_keyboard_transition()
    // routes that same focusless stream to Flutter just as it does for a
    // physical keyboard.
    match command {
        super::super::wire::KeyboardCommand::Text(text) => {
            let mut delivered = false;
            for character in text.chars() {
                let Some(stroke) = shell_text_key_stroke(character) else {
                    warn!(%character, "ignored character unsupported by the shell keyboard keymap");
                    continue;
                };
                delivered |= inject_shell_key_stroke(state, &keyboard, stroke, false, time);
            }
            delivered
        }
        super::super::wire::KeyboardCommand::Key { key, ctrl, phase } => {
            let Some(stroke) = shell_named_key_stroke(key) else {
                warn!(%key, "ignored unsupported shell keyboard key");
                return false;
            };
            match phase {
                super::super::wire::KeyboardKeyPhase::Tap => {
                    inject_shell_key_stroke(state, &keyboard, stroke, *ctrl, time)
                }
                super::super::wire::KeyboardKeyPhase::Pressed => {
                    inject_shell_key_transition(state, &keyboard, stroke, KeyState::Pressed, time)
                }
                super::super::wire::KeyboardKeyPhase::Released => {
                    inject_shell_key_transition(state, &keyboard, stroke, KeyState::Released, time)
                }
            }
        }
    }
}

#[cfg(feature = "flutter")]
#[derive(Clone)]
pub(super) struct ClientInputRoute {
    window: Option<smithay::desktop::Window>,
    pub(super) surface: WlSurface,
    region: InputWindowRegion,
    layout_index: usize,
    scene_origin: Point<f64, Logical>,
}

/// A pointer press serial that was actually delivered to a Wayland client.
///
/// Smithay exposes only the serial of its current click grab. Keeping this
/// tiny, physically-bounded list lets XDG move/resize validate a later button
/// in a multi-button grab, and preserves the atlas-routed focus used when the
/// event was delivered.
#[cfg(feature = "flutter")]
pub(super) struct ClientPointerPress {
    serial: Serial,
    button: u32,
    focus: (WlSurface, Point<f64, Logical>),
    location: Point<f64, Logical>,
}

/// A compositor-forced pointer release remains authoritative until the user
/// deliberately clicks the same toplevel again. Clients may destroy and
/// recreate their protocol constraint after receiving `unlocked`; keying this
/// policy to the window keeps those replacement constraints inactive too.
#[cfg(feature = "flutter")]
#[derive(Debug, Default)]
pub(super) struct PointerConstraintEscape {
    released_window_id: Option<u64>,
}

#[cfg(feature = "flutter")]
impl PointerConstraintEscape {
    fn release_window(&mut self, window_id: u64) {
        self.released_window_id = Some(window_id);
    }

    fn suppresses_window(&self, window_id: u64) -> bool {
        self.released_window_id == Some(window_id)
    }

    fn resume_window(&mut self, window_id: u64) -> bool {
        if !self.suppresses_window(window_id) {
            return false;
        }
        self.released_window_id = None;
        true
    }

    pub(super) fn forget_window(&mut self, window_id: u64) {
        if self.suppresses_window(window_id) {
            self.released_window_id = None;
        }
    }

    fn reset(&mut self) {
        self.released_window_id = None;
    }
}

#[cfg(feature = "flutter")]
impl ClientInputRoute {
    fn focus_at(&self, position: Point<f64, Logical>) -> (WlSurface, Point<f64, Logical>) {
        let scene_position = position - self.scene_origin;
        // Map through the client content area, not the full frame: the
        // frame's top strip is the shell title bar, which has no client
        // buffer. Anchor the content rect at the frame's bottom-left — the
        // decoration sits on top, so the content's bottom edge coincides
        // with the frame's. Its size is the client content texture, making
        // content -> texture a 1:1 mapping.
        let source = self.region.source_rect;
        let content = InputRect {
            x: self.region.rect.x + (self.region.rect.width - source.width) / 2.0,
            y: self.region.rect.y + self.region.rect.height - source.height,
            width: source.width,
            height: source.height,
        };
        let (local_x, local_y) = content.map_to(source, scene_position.x, scene_position.y);
        let local_point = Point::from((local_x, local_y));
        let (surface, local_origin) =
            under_from_surface_tree(&self.surface, local_point, (0, 0), WindowSurfaceType::ALL)
                .unwrap_or_else(|| (self.surface.clone(), (0, 0).into()));
        let scale_x = content.width / source.width;
        let scale_y = content.height / source.height;
        let global_origin = self.scene_origin
            + Point::from((
                content.x + (f64::from(local_origin.x) - source.x) * scale_x,
                content.y + (f64::from(local_origin.y) - source.y) * scale_y,
            ));
        (surface, global_origin)
    }
}

#[cfg(feature = "flutter")]
impl WaylandFrontend {
    pub(super) fn invalidate_window_input_routes(&mut self, window: &smithay::desktop::Window) {
        if self
            .client_input_route_cache
            .as_ref()
            .is_some_and(|route| route.window.as_ref() == Some(window))
        {
            self.client_input_route_cache = None;
        }
        if self
            .client_pointer_capture
            .as_ref()
            .is_some_and(|route| route.window.as_ref() == Some(window))
        {
            self.client_pointer_capture = None;
            self.client_pointer_buttons.clear();
            self.client_pointer_presses.clear();
        }
        self.client_touch_routes
            .retain(|_, route| route.window.as_ref() != Some(window));
    }

    fn window_id_for_input_surface(&self, surface: &WlSurface) -> Option<u64> {
        let root = self.owning_toplevel_surface(surface)?;
        self.surface_id(&root)
    }

    pub(super) fn pointer_constraint_released_for_surface(&self, surface: &WlSurface) -> bool {
        self.window_id_for_input_surface(surface)
            .is_some_and(|window_id| self.pointer_constraint_escape.suppresses_window(window_id))
    }

    fn resume_pointer_constraint_for_route(&mut self, route: &ClientInputRoute) -> bool {
        self.pointer_constraint_escape
            .resume_window(route.region.window_id)
    }

    fn remember_client_pointer_press(
        &mut self,
        route: &ClientInputRoute,
        serial: Serial,
        button: u32,
    ) {
        self.client_pointer_presses
            .retain(|press| press.button != button && press.serial != serial);
        if self.client_pointer_presses.len() == MAX_CLIENT_POINTER_PRESSES {
            self.client_pointer_presses.remove(0);
        }
        self.client_pointer_presses.push(ClientPointerPress {
            serial,
            button,
            focus: route.focus_at(self.pointer_location),
            location: self.pointer_location,
        });
    }

    fn forget_client_pointer_button(&mut self, button: u32) {
        self.client_pointer_presses
            .retain(|press| press.button != button);
    }

    pub(super) fn take_client_pointer_press(
        &mut self,
        surface: &WlSurface,
        serial: Serial,
    ) -> Option<GrabStartData<RuntimeState>> {
        let index = self.client_pointer_presses.iter().position(|press| {
            press.serial == serial && press.focus.0.id().same_client_as(&surface.id())
        })?;
        let press = self.client_pointer_presses.remove(index);
        Some(GrabStartData {
            focus: Some(press.focus),
            button: press.button,
            location: press.location,
        })
    }
}

#[cfg(feature = "flutter")]
enum InputTarget {
    Flutter,
    Client(ClientInputRoute),
}

#[cfg(feature = "flutter")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SuperPointerAction {
    Move,
    Resize,
}

#[cfg(feature = "flutter")]
fn super_pointer_action(logo: bool, button: u32) -> Option<SuperPointerAction> {
    if !logo {
        return None;
    }
    match button {
        BTN_LEFT => Some(SuperPointerAction::Move),
        BTN_RIGHT => Some(SuperPointerAction::Resize),
        _ => None,
    }
}

#[cfg(feature = "flutter")]
fn resize_edge_for_geometry(
    pointer: Point<f64, Logical>,
    geometry: Rectangle<i32, Logical>,
) -> xdg_toplevel::ResizeEdge {
    use xdg_toplevel::ResizeEdge as Edge;
    const EDGE_INSET: f64 = 12.0;

    let left = f64::from(geometry.loc.x);
    let top = f64::from(geometry.loc.y);
    let right = left + f64::from(geometry.size.w);
    let bottom = top + f64::from(geometry.size.h);
    let x = pointer.x;
    let y = pointer.y;

    let near_left = x - left <= EDGE_INSET;
    let near_right = right - x <= EDGE_INSET;
    let near_top = y - top <= EDGE_INSET;
    let near_bottom = bottom - y <= EDGE_INSET;

    // Corners win over edges inside the inset band.
    match (near_left, near_right, near_top, near_bottom) {
        (true, false, true, false) => return Edge::TopLeft,
        (true, false, false, true) => return Edge::BottomLeft,
        (false, true, true, false) => return Edge::TopRight,
        (false, true, false, true) => return Edge::BottomRight,
        (true, _, _, _) => return Edge::Left,
        (_, true, _, _) => return Edge::Right,
        (_, _, true, _) => return Edge::Top,
        (_, _, _, true) => return Edge::Bottom,
        _ => {}
    }

    // Pointer inside the window but away from the border: resize the nearest
    // edge. The midpoint of the top border therefore yields Top (height only)
    // instead of a corner that would also change the width.
    let dist_left = (x - left).abs();
    let dist_right = (right - x).abs();
    let dist_top = (y - top).abs();
    let dist_bottom = (bottom - y).abs();
    let min = dist_left.min(dist_right).min(dist_top).min(dist_bottom);
    if min == dist_top {
        Edge::Top
    } else if min == dist_bottom {
        Edge::Bottom
    } else if min == dist_left {
        Edge::Left
    } else {
        Edge::Right
    }
}

/// Border-band edge detection for unmodified left-press resize.
///
/// Unlike [`resize_edge_for_geometry`] (which falls back to the nearest edge
/// anywhere inside the window for SUPER+RMB), this returns `None` unless the
/// pointer is inside the edge inset band. A plain LMB press on the border
/// therefore resizes only when the cursor is actually on an edge — the
/// desktop-standard interaction, no modifier required. The window's top
/// border is the title bar's top edge, so that strip participates too.
///
/// The geometry is the window's *expanded* input rect: 8px outside the
/// visual frame (the transparent edge band — the window input shape is a
/// slightly larger window whose margin is never rendered) plus the frame
/// itself. The band is the 12px inset of that expanded rect, i.e. an
/// asymmetric safe area: 8px outside the visual edge, 4px inside. One shared
/// value with the shell's cursor hit test, so the cursor never claims a
/// press the compositor would refuse.
#[cfg(feature = "flutter")]
fn resize_edge_at_border(
    pointer: Point<f64, Logical>,
    geometry: Rectangle<i32, Logical>,
) -> Option<xdg_toplevel::ResizeEdge> {
    use xdg_toplevel::ResizeEdge as Edge;
    const EDGE_INSET: f64 = 12.0;

    let left = f64::from(geometry.loc.x);
    let top = f64::from(geometry.loc.y);
    let right = left + f64::from(geometry.size.w);
    let bottom = top + f64::from(geometry.size.h);
    let x = pointer.x;
    let y = pointer.y;

    let near_left = x - left <= EDGE_INSET;
    let near_right = right - x <= EDGE_INSET;
    let near_top = y - top <= EDGE_INSET;
    let near_bottom = bottom - y <= EDGE_INSET;

    match (near_left, near_right, near_top, near_bottom) {
        (true, false, true, false) => Some(Edge::TopLeft),
        (true, false, false, true) => Some(Edge::BottomLeft),
        (false, true, true, false) => Some(Edge::TopRight),
        (false, true, false, true) => Some(Edge::BottomRight),
        (true, _, _, _) => Some(Edge::Left),
        (_, true, _, _) => Some(Edge::Right),
        (_, _, true, _) => Some(Edge::Top),
        (_, _, _, true) => Some(Edge::Bottom),
        _ => None,
    }
}

/// Starts a resize grab for a plain LMB press inside a window's border band.
///
/// This is the desktop-standard interaction: no modifier, just press on the
/// edge and drag. Geometry is the window's full frame — the same rect the
/// publisher uses for both the window input region and the shellRegions
/// subtraction — so the frame includes the title bar and its top border band
/// doubles as the title bar's top edge. Decoration hits (target Flutter with
/// no local region) resolve the owning window through the layout and rebuild
/// its route; local-window hits take the local grab path.
#[cfg(feature = "flutter")]
fn begin_border_resize_grab(
    state: &mut RuntimeState,
    target: &InputTarget,
    local_window_region: &Option<InputWindowRegion>,
    button: u32,
    serial: Serial,
) -> bool {
    let frontend = state.wayland.as_ref().expect("missing Wayland frontend");
    let layout = frontend.input_layout.as_ref();
    let scene_position = frontend.pointer_location - frontend.atlas_origin;

    // Resolve the owning window region: content hits carry a route, local
    // window hits carry the region, decoration hits resolve through the
    // layout (a covered client can never be reached through a higher
    // window's title bar, mirroring input_route). The transparent edge band
    // (8px outside the visual frame) resolves through the same lookup by
    // matching against the expanded input rect.
    let (route, region) = match target {
        InputTarget::Client(route) => (Some(route.clone()), route.region),
        InputTarget::Flutter => {
            if let Some(region) = local_window_region {
                (None, *region)
            } else {
                let Some((layout_index, (region, _))) = layout
                    .expect("missing input layout")
                    .windows
                    .iter()
                    .zip(&layout.expect("missing input layout").window_decorations)
                    .enumerate()
                    .find(|(_, (region, decoration))| {
                        region_accepts_input(region, scene_position)
                            || (decoration.width > 0.0
                                && decoration.height > 0.0
                                && decoration.contains(scene_position.x, scene_position.y)
                                && region.visible()
                                && region.hit_test_enabled())
                            || (region.visible()
                                && region.hit_test_enabled()
                                && scene_position.x >= region.rect.x - 8.0
                                && scene_position.x
                                    <= region.rect.x + region.rect.width + 8.0
                                && scene_position.y >= region.rect.y - 8.0
                                && scene_position.y
                                    <= region.rect.y + region.rect.height + 8.0)
                    })
                else {
                    return false;
                };
                let Some(surface) = frontend.surfaces_by_id.get(&region.surface_id).cloned()
                else {
                    return false;
                };
                let Some(window) = frontend.window_for_id(region.window_id) else {
                    return false;
                };
                let Some(root_surface) = frontend.window_root_surface(&window) else {
                    return false;
                };
                if frontend.owning_toplevel_surface(&surface).as_ref() != Some(&root_surface) {
                    return false;
                }
                (
                    Some(ClientInputRoute {
                        window: Some(window),
                        surface,
                        region: *region,
                        layout_index,
                        scene_origin: frontend.atlas_origin,
                    }),
                    *region,
                )
            }
        }
    };

    if region.geometry_locked() {
        return false;
    }
    // The border band lives on the window's expanded input rect: the visual
    // frame plus the transparent 8px edge band, so the asymmetric safe area
    // (8px outside / 4px inside the visual edge) is a uniform 12px inset.
    let global_geometry = Rectangle::new(
        Point::from((
            (region.rect.x - 8.0 + frontend.atlas_origin.x).round() as i32,
            (region.rect.y - 8.0 + frontend.atlas_origin.y).round() as i32,
        )),
        (
            (region.rect.width + 16.0).round() as i32,
            (region.rect.height + 16.0).round() as i32,
        )
            .into(),
    );
    let Some(edge) = resize_edge_at_border(frontend.pointer_location, global_geometry) else {
        info!(
            x = frontend.pointer_location.x,
            y = frontend.pointer_location.y,
            rect = ?global_geometry,
            "border press outside the edge band — no resize"
        );
        return false;
    };
    info!(
        x = frontend.pointer_location.x,
        y = frontend.pointer_location.y,
        rect = ?global_geometry,
        edge = ?edge,
        has_route = route.is_some(),
        "border press starts resize grab"
    );

    match route {
        Some(route) => flutter_route::begin_super_pointer_grab(
            state,
            &route,
            SuperPointerAction::Resize,
            button,
            serial,
            Some(edge),
        ),
        None => flutter_route::begin_local_super_pointer_grab(
            state,
            region,
            SuperPointerAction::Resize,
            button,
            serial,
            Some(edge),
        ),
    }
}

#[cfg(feature = "flutter")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RoutedPointerTarget {
    Flutter,
    Client(u64),
}

#[cfg(feature = "flutter")]
struct PointerMotionTarget {
    routed: RoutedPointerTarget,
    focus: Option<(WlSurface, Point<f64, Logical>)>,
}

#[cfg(feature = "flutter")]
impl PointerMotionTarget {
    const FLUTTER: Self = Self {
        routed: RoutedPointerTarget::Flutter,
        focus: None,
    };

    fn client(route: &ClientInputRoute, position: Point<f64, Logical>) -> Self {
        Self {
            routed: RoutedPointerTarget::Client(route.region.surface_id),
            focus: Some(route.focus_at(position)),
        }
    }
}

#[cfg(feature = "flutter")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FlutterKeyDisposition {
    Forward,
    Dispatch,
    ConsumeRetired,
}

#[derive(Clone, Copy, Debug, Default)]
struct InputDeviceReset {
    keyboard: bool,
    pointer: bool,
    touch: bool,
}

impl InputDeviceReset {
    const ALL: Self = Self {
        keyboard: true,
        pointer: true,
        touch: true,
    };

    const fn any(self) -> bool {
        self.keyboard || self.pointer || self.touch
    }
}

fn retired_key_consumes_transition(
    retired: &mut HashSet<u32>,
    keycode: u32,
    state: KeyState,
) -> bool {
    match state {
        KeyState::Pressed => retired.contains(&keycode),
        KeyState::Released => retired.remove(&keycode),
    }
}

#[cfg(feature = "flutter")]
fn retired_pointer_button_consumes_transition(
    retired: &mut HashSet<u32>,
    button: u32,
    state: ButtonState,
) -> bool {
    match state {
        ButtonState::Pressed => retired.contains(&button),
        ButtonState::Released => retired.remove(&button),
    }
}

fn update_pressed_buttons(buttons: &mut HashSet<u32>, button: u32, state: ButtonState) {
    match state {
        ButtonState::Pressed => {
            buttons.insert(button);
        }
        ButtonState::Released => {
            buttons.remove(&button);
        }
    }
}

#[cfg(feature = "flutter")]
pub(super) fn retire_flutter_generation_keys(
    active: &mut HashSet<u32>,
    retired: &mut HashSet<u32>,
) {
    retired.extend(active.drain());
}

#[cfg(feature = "flutter")]
fn route_flutter_key_transition(
    active: &mut HashSet<u32>,
    retired: &mut HashSet<u32>,
    keycode: u32,
    state: KeyState,
    capture_new_press: bool,
) -> FlutterKeyDisposition {
    if retired_key_consumes_transition(retired, keycode, state) {
        if state == KeyState::Released {
            active.remove(&keycode);
        }
        return FlutterKeyDisposition::ConsumeRetired;
    }
    match state {
        KeyState::Pressed if active.contains(&keycode) || capture_new_press => {
            active.insert(keycode);
            FlutterKeyDisposition::Dispatch
        }
        KeyState::Pressed => FlutterKeyDisposition::Forward,
        KeyState::Released if active.remove(&keycode) => FlutterKeyDisposition::Dispatch,
        KeyState::Released => FlutterKeyDisposition::Forward,
    }
}

#[cfg(feature = "flutter")]
fn route_input_method_key_transition(
    active: &mut HashSet<u32>,
    retired: &mut HashSet<u32>,
    keycode: u32,
    state: KeyState,
    flutter_editor_active: bool,
) -> FlutterKeyDisposition {
    route_flutter_key_transition(
        active,
        retired,
        keycode,
        state,
        flutter_editor_active && matches!(state, KeyState::Pressed),
    )
}

#[cfg(feature = "flutter")]
fn route_shell_key_transition(
    held: &mut HashSet<u32>,
    keycode: u32,
    state: KeyState,
    seat_pressed: bool,
) -> bool {
    match state {
        KeyState::Pressed => !seat_pressed && held.insert(keycode),
        KeyState::Released => held.remove(&keycode),
    }
}

#[cfg(feature = "flutter")]
fn region_accepts_input(region: &InputWindowRegion, position: Point<f64, Logical>) -> bool {
    region.rect.contains(position.x, position.y)
        && region.visible()
        && region.hit_test_enabled()
        && region.window_id == region.object_id
}

#[cfg(feature = "flutter")]
fn software_keyboard_owns_touch(
    layout: Option<&InputLayoutSnapshot>,
    scene_position: Point<f64, Logical>,
) -> bool {
    layout.is_some_and(|layout| {
        layout
            .software_keyboard_regions
            .iter()
            .any(|region| region.contains(scene_position.x, scene_position.y))
    })
}

#[cfg(feature = "flutter")]
impl WaylandFrontend {
    fn client_input_route_is_live(&self, route: &ClientInputRoute) -> bool {
        // Window unmap paths invalidate their cached routes explicitly. The
        // stable surface map is therefore the only lifecycle check needed at
        // input frequency; avoiding Space's element lookup keeps a cache hit
        // independent of the number of windows.
        self.surfaces_by_id
            .get(&route.region.surface_id)
            .is_some_and(|surface| surface == &route.surface)
            && (route.window.is_some() || self.input_method.owns_popup_surface(&route.surface))
    }

    fn input_route(&mut self, position: Point<f64, Logical>) -> Option<&ClientInputRoute> {
        let layout = self.input_layout.as_ref()?;
        let scene_position = position - self.atlas_origin;
        if layout.exclusive_shell()
            || layout
                .shell_regions
                .iter()
                .any(|region| region.contains(scene_position.x, scene_position.y))
        {
            return None;
        }

        // XDG popups render above their parent window but are absent from
        // the Flutter input layout. Route hits on popup geometry directly
        // to the popup surface so menu items receive events; routing them
        // to the parent toplevel would read as an outside click and
        // dismiss the menu.
        if let Some(route) = self.popup_input_route(scene_position) {
            self.client_input_route_cache = Some(route);
            return self.client_input_route_cache.as_ref();
        }

        // Windows are depth-tested front-to-back as single units: a window's
        // shell-drawn decoration (title bar) and its content share the same
        // z unit. The first hit decides routing - decoration and local
        // Flutter windows route to the shell scene (they have no Smithay
        // input target), content continues to the client route below. A
        // covered client can never receive the event through a higher
        // window's title bar.
        if let Some((hit, decoration)) = layout
            .windows
            .iter()
            .zip(&layout.window_decorations)
            .find(|(region, decoration)| {
                region_accepts_input(region, scene_position)
                    || (decoration.width > 0.0
                        && decoration.height > 0.0
                        && decoration.contains(scene_position.x, scene_position.y)
                        && region.visible()
                        && region.hit_test_enabled())
            })
        {
            let is_local = self.local_windows.contains(hit.window_id);
            let is_decoration = decoration.width > 0.0
                && decoration.height > 0.0
                && decoration.contains(scene_position.x, scene_position.y);
            info!(
                window_id = hit.window_id,
                object_id = hit.object_id,
                surface_id = hit.surface_id,
                z = hit.z,
                is_local,
                is_decoration,
                rect = ?hit.rect,
                "input hit test at {scene_position:?}"
            );
            if is_local || is_decoration {
                self.client_input_route_cache = None;
                return None;
            }
        }

        // Pointer samples commonly arrive much faster than Flutter layout
        // snapshots. Reuse the fully validated route while it remains the
        // topmost candidate instead of walking Space and the surface tree at
        // input frequency. Regions preceding it still need a cheap geometry
        // check because windows are ordered front-to-back and may overlap.
        let cached_is_valid = self.client_input_route_cache.as_ref().is_some_and(|route| {
            region_accepts_input(&route.region, scene_position)
                && layout
                    .windows
                    .get(..route.layout_index)
                    .is_some_and(|higher_regions| {
                        !higher_regions
                            .iter()
                            .any(|region| region_accepts_input(region, scene_position))
                    })
                && self.client_input_route_is_live(route)
        });
        if cached_is_valid {
            return self.client_input_route_cache.as_ref();
        }

        let route = layout
            .windows
            .iter()
            .enumerate()
            .find_map(|(layout_index, region)| {
                if !region_accepts_input(region, scene_position) {
                    return None;
                }
                let surface = self.surfaces_by_id.get(&region.surface_id).cloned()?;
                if self.input_method.owns_popup_surface(&surface) {
                    return Some(ClientInputRoute {
                        window: None,
                        surface,
                        region: *region,
                        layout_index,
                        scene_origin: self.atlas_origin,
                    });
                }
                let window = self.window_for_id(region.window_id)?;
                let root_surface = self.window_root_surface(&window)?;
                if self.owning_toplevel_surface(&surface).as_ref() != Some(&root_surface) {
                    return None;
                }
                Some(ClientInputRoute {
                    window: Some(window.clone()),
                    surface,
                    region: *region,
                    layout_index,
                    scene_origin: self.atlas_origin,
                })
            });

        if let Some(route) = route {
            info!(
                window_id = route.region.window_id,
                surface_id = route.region.surface_id,
                z = route.region.z,
                rect = ?route.region.rect,
                "created client input route at {scene_position:?}"
            );
            self.client_input_route_cache = Some(route);
            return self.client_input_route_cache.as_ref();
        }

        // X11 fallback: the Flutter input layout lags new X11 windows by up
        // to seconds (the layout is a Rust -> Flutter -> Rust round trip),
        // but X11 popups must be clickable the instant they map — a menu
        // whose events are routed to the parent window is interpreted as an
        // outside click and closes immediately. Hit the compositor-owned
        // Space directly for X11 windows so they are interactive from the
        // first frame. Wayland windows stay layout-only: their decoration
        // semantics live entirely in the Flutter scene.
        let x11_fallback = self.space.elements().enumerate().rev().find_map(
            |(z, window)| {
                let x11 = window.x11_surface()?;
                let geometry = self.window_geometry_target(window);
                if geometry.size.w <= 0 || geometry.size.h <= 0 {
                    return None;
                }
                let rect = InputRect {
                    x: geometry.loc.x as f64,
                    y: geometry.loc.y as f64,
                    width: geometry.size.w as f64,
                    height: geometry.size.h as f64,
                };
                if !rect.contains(scene_position.x, scene_position.y) {
                    return None;
                }
                let surface = self.window_root_surface(window)?;
                let surface_id = self.surface_id(&surface)?;
                let _ = x11;
                Some((
                    z,
                    ClientInputRoute {
                        window: Some(window.clone()),
                        surface,
                        region: InputWindowRegion {
                            object_id: surface_id,
                            surface_id,
                            window_id: surface_id,
                            rect,
                            source_rect: rect,
                            z: z as i32,
                            flags: crate::wire::INPUT_WINDOW_VISIBLE,
                        },
                        layout_index: 0,
                        scene_origin: self.atlas_origin,
                    },
                ))
            },
        );
        if let Some((_, route)) = x11_fallback {
            info!(
                window_id = route.region.window_id,
                surface_id = route.region.surface_id,
                z = route.region.z,
                rect = ?route.region.rect,
                "created X11 fallback input route at {scene_position:?}"
            );
            self.client_input_route_cache = Some(route);
            return self.client_input_route_cache.as_ref();
        }

        None
    }

    /// Finds a hit on an XDG popup surface (context menus, dropdowns) and
    /// builds a client route to it. Popups are rendered above their parent
    /// window in the compositor scene but never appear in the Flutter
    /// input layout, so their geometry is queried from the compositor
    /// space directly (same pattern as the X11 fallback below).
    fn popup_input_route(&self, scene_position: Point<f64, Logical>) -> Option<ClientInputRoute> {
        for window in self.space.elements().rev() {
            let Some(root) = self.window_root_surface(window) else {
                continue;
            };
            for (popup, popup_location) in PopupManager::popups_for_surface(&root) {
                let popup_geometry = popup.geometry();
                let PopupKind::Xdg(popup) = popup else {
                    continue;
                };
                let popup_surface = popup.wl_surface();
                if !popup_surface.is_alive() {
                    continue;
                }
                let content = self.window_geometry_target(window);
                let popup_origin = (
                    content
                        .loc
                        .x
                        .saturating_add(popup_location.x)
                        .saturating_sub(popup_geometry.loc.x),
                    content
                        .loc
                        .y
                        .saturating_add(popup_location.y)
                        .saturating_sub(popup_geometry.loc.y),
                );
                // Route on the popup's real content geometry rather than the
                // full render buffer. The compositor draws the buffer with its
                // shadow margins, but the XDG popup geometry places the
                // interactive content offset from the buffer origin (shadow
                // band). Using the render extent as the input+focus region
                // shifts the pointer surface-local coordinates by exactly that
                // offset, so menu hover highlights never land on the client.
                // Anchor the region at the content origin (popup origin plus the
                // geometry offset) with the geometry's content size.
                let geo_size = popup_geometry.size;
                if geo_size.w <= 0 || geo_size.h <= 0 {
                    continue;
                }
                let rect = InputRect {
                    x: (popup_origin.0 + popup_geometry.loc.x) as f64,
                    y: (popup_origin.1 + popup_geometry.loc.y) as f64,
                    width: geo_size.w as f64,
                    height: geo_size.h as f64,
                };
                info!(
                    content_loc = ?content.loc,
                    popup_location = ?popup_location,
                    popup_geometry = ?popup_geometry,
                    popup_origin = ?popup_origin,
                    "XDG popup input geometry"
                );
                if !rect.contains(scene_position.x, scene_position.y) {
                    continue;
                }
                let Some(surface_id) = self.surface_id(&popup_surface) else {
                    continue;
                };
                info!(
                    surface_id,
                    rect = ?rect,
                    "created XDG popup input route at {scene_position:?}"
                );
                return Some(ClientInputRoute {
                    window: None,
                    surface: popup_surface.clone(),
                    region: InputWindowRegion {
                        object_id: surface_id,
                        surface_id,
                        window_id: 0,
                        rect,
                        // Source is the popup buffer's own local coordinate
                        // space (anchor at 0,0), NOT the global rect. Deriving
                        // source from the global rect makes focus_at subtract
                        // the global origin from itself and collapse
                        // global_origin to ~0 (content.x + (0 - source.x)).
                        source_rect: InputRect {
                            x: 0.0,
                            y: 0.0,
                            width: geo_size.w as f64,
                            height: geo_size.h as f64,
                        },
                        z: i32::MAX,
                        flags: crate::wire::INPUT_WINDOW_VISIBLE,
                    },
                    layout_index: 0,
                    scene_origin: self.atlas_origin,
                });
            }
        }
        None
    }

    fn local_flutter_window_region_at(
        &self,
        position: Point<f64, Logical>,
    ) -> Option<InputWindowRegion> {
        let layout = self.input_layout.as_ref()?;
        let scene_position = position - self.atlas_origin;
        if layout.exclusive_shell()
            || layout
                .shell_regions
                .iter()
                .any(|region| region.contains(scene_position.x, scene_position.y))
        {
            return None;
        }
        layout
            .windows
            .iter()
            .find(|region| region_accepts_input(region, scene_position))
            .copied()
            .filter(|region| self.local_windows.contains(region.window_id))
    }

    fn touch_window_target_at(&self, position: Point<f64, Logical>) -> Option<TouchWindowTarget> {
        let layout = self.input_layout.as_ref()?;
        let scene_position = position - self.atlas_origin;
        if layout.exclusive_shell()
            || layout
                .shell_regions
                .iter()
                .any(|region| region.contains(scene_position.x, scene_position.y))
        {
            return None;
        }
        let region = layout
            .windows
            .iter()
            .find(|region| region_accepts_input(region, scene_position))?;
        let geometry = if self.local_windows.contains(region.window_id) {
            self.local_flutter_window_geometry(region.window_id)?
        } else {
            let window = self.window_for_id(region.window_id)?;
            let geometry = self.window_geometry_target(&window);
            super::super::wire::WindowGeometry {
                x: f64::from(geometry.loc.x),
                y: f64::from(geometry.loc.y),
                width: f64::from(geometry.size.w),
                height: f64::from(geometry.size.h),
            }
        };
        Some(TouchWindowTarget {
            window_id: region.window_id,
            geometry,
            in_gesture_strip: scene_position.y
                >= region.rect.y + region.rect.height
                    - WINDOW_TOUCH_STRIP_HEIGHT.min(region.rect.height),
            in_move_corner: {
                let corner_width = WINDOW_TOUCH_CORNER_SIZE.min(region.rect.width * 0.5);
                let corner_height = WINDOW_TOUCH_CORNER_SIZE.min(region.rect.height * 0.5);
                let on_horizontal_edge = scene_position.x < region.rect.x + corner_width
                    || scene_position.x >= region.rect.x + region.rect.width - corner_width;
                let on_vertical_edge = scene_position.y < region.rect.y + corner_height
                    || scene_position.y >= region.rect.y + region.rect.height - corner_height;
                on_horizontal_edge && on_vertical_edge
            },
            geometry_locked: region.geometry_locked(),
        })
    }

    fn input_target(&mut self, position: Point<f64, Logical>) -> InputTarget {
        self.input_route(position)
            .cloned()
            .map_or(InputTarget::Flutter, InputTarget::Client)
    }

    fn pointer_motion_target(&mut self, position: Point<f64, Logical>) -> PointerMotionTarget {
        self.input_route(position)
            .map_or(PointerMotionTarget::FLUTTER, |route| {
                PointerMotionTarget::client(route, position)
            })
    }
}

pub(in super::super) fn init_libinput(
    event_loop: &mut EventLoop<'static, RuntimeState>,
    session: LibSeatSession,
    seat_name: &str,
) -> Result<(), Box<dyn Error>> {
    #[cfg(feature = "flutter")]
    init_joystick_activity(event_loop, session.clone())?;
    let mut context =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.into());
    context
        .udev_assign_seat(seat_name)
        .map_err(|()| "libinput could not assign the active seat")?;
    let backend = LibinputBatchSource::new(LibinputInputBackend::new(context));
    event_loop
        .handle()
        .insert_source(backend, |event, batch, state| {
            match event {
                InputBatchEvent::Input(event) => {
                    batch.flush_clients |= process_input_event(state, event);
                }
                // libinput is independent from the Wayland client socket
                // source. Flush after Smithay has drained the complete batch:
                // clients still observe input immediately, while a burst of
                // samples costs one non-blocking socket flush instead of one
                // syscall per event.
                InputBatchEvent::Complete => {
                    if batch.flush_clients
                        && let Some(frontend) = state.wayland.as_mut()
                        && let Err(error) = frontend.display_handle.flush_clients()
                    {
                        warn!(%error, "could not flush Wayland clients after native input batch");
                    }
                }
            }
        })?;
    Ok(())
}

#[cfg(feature = "flutter")]
fn process_keyboard_transition(
    state: &mut RuntimeState,
    keycode: Keycode,
    key_state: KeyState,
    time: u32,
) -> bool {
    state.note_user_activity();
    if intercept_native_escape(state, keycode.raw(), key_state) {
        return true;
    }
    if let Some(evdev_keycode) = keycode.raw().checked_sub(8) {
        let allow_new = !state.secure_session_locked();
        let routed = state.native_app_plugins.as_mut().map(|manager| {
            manager.route_key(
                evdev_keycode,
                key_state == KeyState::Pressed,
                u64::from(time).saturating_mul(1_000_000),
                allow_new,
            )
        });
        match routed {
            Some(Ok(true)) => return true,
            Some(Err(error)) => {
                warn!(%error, evdev_keycode, "native application key routing failed");
                return true;
            }
            Some(Ok(false)) | None => {}
        }
    }
    if state.flutter_active {
        return process_flutter_keyboard_transition(state, keycode, key_state, time);
    }
    if state.secure_session_locked() {
        // Native lock state remains authoritative if Flutter is restarting or
        // unavailable. No keyboard source may fall through to a client.
        return true;
    }
    process_wayland_keyboard_transition(state, keycode, key_state, time);
    true
}

fn configure_touchpad_device(
    device: &mut LibinputDevice,
    settings: &TouchpadSettings,
) -> Result<(), String> {
    let finger_count = device.config_tap_finger_count();
    if finger_count > 0 {
        if let Err(error) = device.config_tap_set_button_map(TapButtonMap::LeftRightMiddle) {
            // Keep tap-to-click usable if a device rejects explicit remapping;
            // libinput's normal default is the same left/right/middle order.
            warn!(
                ?error,
                device = %device.name(),
                "could not configure multi-finger tap button mapping"
            );
        }

        device
            .config_tap_set_enabled(settings.tap_to_click_enabled)
            .map_err(|error| {
                format!(
                    "could not set tap-to-click on {} to {}: {error:?}",
                    device.name(),
                    settings.tap_to_click_enabled
                )
            })?;
    }

    let natural_scroll_supported = device.config_scroll_has_natural_scroll();
    if natural_scroll_supported {
        device
            .config_scroll_set_natural_scroll_enabled(settings.natural_scroll_enabled)
            .map_err(|error| {
                format!(
                    "could not set natural scrolling on {} to {}: {error:?}",
                    device.name(),
                    settings.natural_scroll_enabled
                )
            })?;
    }

    info!(
        device = %device.name(),
        finger_count,
        tap_to_click_enabled = settings.tap_to_click_enabled,
        natural_scroll_enabled = settings.natural_scroll_enabled,
        natural_scroll_supported,
        two_finger_right_click = finger_count >= 2,
        "configured touchpad"
    );
    Ok(())
}

fn is_touchpad_device(device: &LibinputDevice) -> bool {
    // SAFETY: this device came from the same udev-backed libinput context used
    // to construct the event source, as required by input::Device::udev_device.
    let udev_marks_touchpad = unsafe { device.udev_device() }.and_then(|device| {
        device
            .property_value("ID_INPUT_TOUCHPAD")
            .map(|value| value == OsStr::new("1"))
    });
    udev_marks_touchpad.unwrap_or_else(|| device.config_tap_finger_count() > 0)
}

#[cfg(feature = "flutter")]
pub(in super::super) fn install_touchpad_settings(
    state: &mut RuntimeState,
    settings: &TouchpadSettings,
) -> Result<(), String> {
    for device in state.touchpad_devices.values_mut() {
        configure_touchpad_device(device, settings)?;
    }
    Ok(())
}

#[cfg(feature = "flutter")]
fn touchpad_presence_changed(previous_count: usize, current_count: usize) -> bool {
    (previous_count == 0) != (current_count == 0)
}

#[cfg(feature = "flutter")]
fn process_touchpad_gesture_event(
    state: &mut RuntimeState,
    event: &InputEvent<LibinputInputBackend>,
) -> Option<bool> {
    if !state.flutter_active || state.secure_session_locked() {
        if matches!(
            event,
            InputEvent::GestureSwipeBegin { .. }
                | InputEvent::GestureSwipeUpdate { .. }
                | InputEvent::GestureSwipeEnd { .. }
        ) {
            state.touchpad_gestures.reset();
            return Some(false);
        }
        return None;
    }

    let gesture = match event {
        InputEvent::GestureSwipeBegin { event } => {
            let device = event.device();
            state
                .touchpad_gestures
                .begin_swipe(device.sysname(), event.fingers());
            None
        }
        InputEvent::GestureSwipeUpdate { event } => {
            let device = event.device();
            state
                .touchpad_gestures
                .update_swipe(device.sysname(), event.delta_x(), event.delta_y())
        }
        InputEvent::GestureSwipeEnd { event } => {
            let device = event.device();
            state.touchpad_gestures.end_swipe(device.sysname());
            None
        }
        _ => return None,
    };

    if let Some(gesture) = gesture {
        let disposition = state.native_escape_shortcut.observe_gesture(gesture);
        let handled = execute_shortcut_disposition(state, disposition);
        if handled {
            info!(
                ?gesture,
                "recognized configured compositor shortcut gesture"
            );
        }
        Some(handled)
    } else {
        Some(false)
    }
}

fn process_input_event(
    state: &mut RuntimeState,
    mut event: InputEvent<LibinputInputBackend>,
) -> bool {
    if let InputEvent::DeviceAdded { device } = &mut event {
        if Device::has_capability(device, DeviceCapability::Keyboard) {
            if let Some(led_state) = state
                .wayland
                .as_ref()
                .and_then(|frontend| frontend.seat.get_keyboard())
                .map(|keyboard| keyboard.led_state())
            {
                device.led_update(led_state.into());
            }
            state
                .keyboard_devices
                .insert(device.sysname().to_owned(), device.clone());
        }
        let touchpad = is_touchpad_device(device);
        if touchpad {
            let settings = state
                .wayland
                .as_ref()
                .map(|frontend| frontend.settings.touchpad().clone())
                .unwrap_or_default();
            // libinput recognizes taps and emits ordinary BTN_LEFT transitions.
            // Keeping recognition at the device boundary lets synthesized
            // clicks use the same Flutter/Wayland focus and grab path as
            // physical buttons.
            if let Err(error) = configure_touchpad_device(device, &settings) {
                warn!(%error, "could not apply persisted touchpad settings");
            }
            #[cfg(feature = "flutter")]
            {
                let previous_count = state.touchpad_devices.len();
                state
                    .touchpad_devices
                    .insert(device.sysname().to_owned(), device.clone());
                state.input_device_capabilities_changed |=
                    touchpad_presence_changed(previous_count, state.touchpad_devices.len());
            }
        }
    }

    if let InputEvent::DeviceRemoved { device } = &event {
        if Device::has_capability(device, DeviceCapability::Keyboard) {
            state.keyboard_devices.remove(device.sysname());
        }
        #[cfg(feature = "flutter")]
        {
            let previous_count = state.touchpad_devices.len();
            state.touchpad_devices.remove(device.sysname());
            state.input_device_capabilities_changed |=
                touchpad_presence_changed(previous_count, state.touchpad_devices.len());
        }
        let reset = InputDeviceReset {
            keyboard: Device::has_capability(device, DeviceCapability::Keyboard),
            pointer: Device::has_capability(device, DeviceCapability::Pointer),
            touch: Device::has_capability(device, DeviceCapability::Touch),
        };
        if reset.any() {
            reset_input_devices(state, reset);
        }
        return reset.any();
    }

    #[cfg(feature = "flutter")]
    if let InputEvent::Keyboard {
        event: key_event, ..
    } = &event
    {
        return process_keyboard_transition(
            state,
            key_event.key_code(),
            key_event.state(),
            key_event.time_msec(),
        );
    }

    #[cfg(feature = "flutter")]
    if !matches!(&event, InputEvent::DeviceAdded { .. }) {
        state.note_user_activity();
    }

    #[cfg(feature = "flutter")]
    if let Some(flush_clients) = process_touchpad_gesture_event(state, &event) {
        return flush_clients;
    }

    #[cfg(not(feature = "flutter"))]
    match &event {
        InputEvent::Keyboard {
            event: key_event, ..
        } if intercept_native_escape(state, key_event.key_code().raw(), key_event.state()) => {
            // Native window actions may emit configure/focus messages. This
            // edge is infrequent, so retain the conservative immediate flush.
            return true;
        }
        _ => {}
    }

    #[cfg(feature = "flutter")]
    if state.flutter_active {
        return process_flutter_input_event(state, event);
    }

    #[cfg(feature = "flutter")]
    if state.secure_session_locked() {
        // Native lock state remains authoritative if Flutter is restarting or
        // unavailable. No physical input may fall through to a client.
        return true;
    }

    process_wayland_input_event(state, event);
    true
}

pub(in super::super) fn reset_all_input_devices(state: &mut RuntimeState) {
    reset_input_devices(state, InputDeviceReset::ALL);
}

#[cfg(feature = "flutter")]
pub(in super::super) fn install_keyboard_settings(
    state: &mut RuntimeState,
    settings: &KeyboardSettings,
) -> Result<Vec<String>, Box<dyn Error>> {
    let layout_names = settings.compiled_layout_names()?;
    let names = settings.xkb_names();
    // Retire every key against the old map before replacing it. A later
    // physical release is consumed, so neither Flutter nor a client can keep
    // a modifier logically held across the keymap boundary.
    reset_input_devices(
        state,
        InputDeviceReset {
            keyboard: true,
            pointer: false,
            touch: false,
        },
    );
    let keyboard = state
        .wayland
        .as_ref()
        .and_then(|frontend| frontend.seat.get_keyboard())
        .ok_or("seat has no keyboard")?;
    keyboard.set_xkb_config(
        state,
        XkbConfig {
            rules: "evdev",
            model: "pc105",
            layout: &names.layout,
            variant: &names.variant,
            options: Some(names.options),
        },
    )?;
    keyboard.change_repeat_info(
        i32::try_from(settings.repeat_rate_hz)?,
        i32::try_from(settings.repeat_delay_ms)?,
    );
    {
        let frontend = state.wayland.as_mut().expect("missing Wayland frontend");
        #[cfg(feature = "flutter")]
        if let Some(compose) = frontend.flutter_compose.as_mut() {
            compose.reset();
        }
        frontend.keyboard_layout_names = layout_names.clone();
        frontend.active_keyboard_layout = 0;
        frontend.keyboard_configuration_changed = true;
    }
    super::input_method::refresh_keyboard_grab(
        state,
        i32::try_from(settings.repeat_rate_hz)?,
        i32::try_from(settings.repeat_delay_ms)?,
    );
    Ok(layout_names)
}

fn reset_input_devices(state: &mut RuntimeState, reset: InputDeviceReset) {
    if reset.keyboard {
        state.native_escape_shortcut.reset();
        #[cfg(feature = "flutter")]
        cancel_flutter_repeat(state);
    }
    #[cfg(feature = "flutter")]
    if reset.pointer {
        state.touchpad_gestures.reset();
    }
    #[cfg(feature = "flutter")]
    if reset.touch {
        let actions = state
            .wayland
            .as_mut()
            .map(|frontend| frontend.touch_gestures.cancel_all())
            .unwrap_or_default();
        touch_gestures::apply_actions(state, actions);
    }
    #[cfg(feature = "flutter")]
    if state.flutter_active {
        state
            .flutter_input
            .cancel_device_lifecycles(reset.pointer, reset.touch);
    }
    #[cfg(feature = "flutter")]
    if let Some(manager) = state.native_app_plugins.as_mut()
        && let Err(error) = manager.reset_input(reset.keyboard, reset.touch)
    {
        warn!(%error, "could not reset native application input");
    }

    let Some(frontend) = state.wayland.as_mut() else {
        return;
    };
    let time = frontend.start_time.elapsed().as_millis() as u32;
    let pointer = reset
        .pointer
        .then(|| frontend.seat.get_pointer().expect("seat has no pointer"));
    let touch = reset
        .touch
        .then(|| frontend.seat.get_touch().expect("seat has no touch"));
    let keyboard = reset
        .keyboard
        .then(|| frontend.seat.get_keyboard().expect("seat has no keyboard"));
    let mut pointer_buttons = if reset.pointer {
        let mut buttons = std::mem::take(&mut frontend.wayland_pointer_buttons)
            .into_iter()
            .collect::<Vec<_>>();
        buttons.sort_unstable();
        #[cfg(feature = "flutter")]
        {
            frontend.client_pointer_capture = None;
            frontend.pointer_constraint_escape.reset();
            frontend.client_pointer_buttons.clear();
            frontend.client_pointer_presses.clear();
            frontend.flutter_pointer_press = None;
            frontend.set_clipboard_drag_active(false);
            frontend.retired_pointer_buttons.clear();
            frontend.set_routed_pointer_target(RoutedPointerTarget::Flutter);
        }
        buttons
    } else {
        Vec::new()
    };
    #[cfg(feature = "flutter")]
    let cancel_client_touch = reset.touch
        && (!frontend.client_touch_routes.is_empty()
            || touch.as_ref().is_some_and(|touch| touch.is_grabbed()));
    #[cfg(not(feature = "flutter"))]
    let cancel_client_touch = reset.touch;
    if reset.touch {
        #[cfg(feature = "flutter")]
        {
            frontend.flutter_touch_slots.clear();
            frontend.client_touch_routes.clear();
            frontend.client_touch_frame_pending = false;
        }
    }

    #[cfg(feature = "flutter")]
    let active_flutter_keys = if reset.keyboard {
        std::mem::take(&mut frontend.flutter_keyboard_keys)
    } else {
        HashSet::new()
    };
    #[cfg(feature = "flutter")]
    let active_input_method_keys = if reset.keyboard {
        std::mem::take(&mut frontend.flutter_input_method_keys)
    } else {
        HashSet::new()
    };
    if reset.keyboard {
        frontend.shell_keyboard_keys.clear();
    }
    let previously_retired_keys = if reset.keyboard {
        frontend.retired_keyboard_keys.clone()
    } else {
        HashSet::new()
    };
    if let Some(keyboard) = keyboard.as_ref() {
        for keycode in keyboard.pressed_keys() {
            frontend.retired_keyboard_keys.insert(keycode.raw());
        }
        #[cfg(feature = "flutter")]
        frontend
            .retired_keyboard_keys
            .extend(active_flutter_keys.iter().copied());
        #[cfg(feature = "flutter")]
        frontend
            .retired_input_method_keys
            .extend(active_input_method_keys.iter().copied());
    }
    if let Some(pointer) = pointer {
        let had_buttons = !pointer_buttons.is_empty();
        let had_grab = pointer.is_grabbed();
        for button in pointer_buttons.drain(..) {
            pointer.button(
                state,
                &ButtonEvent {
                    button,
                    state: ButtonState::Released,
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                },
            );
        }
        if pointer.is_grabbed() {
            pointer.unset_grab(state, SERIAL_COUNTER.next_serial(), time);
        }
        if had_buttons || had_grab {
            pointer.frame(state);
        }
    }

    if cancel_client_touch && let Some(touch) = touch {
        touch.cancel(state);
        if touch.is_grabbed() {
            touch.unset_grab(state);
        }
    }

    if let Some(keyboard) = keyboard {
        let mut pressed_keys = keyboard.pressed_keys().into_iter().collect::<Vec<_>>();
        pressed_keys.sort_unstable_by_key(|keycode| keycode.raw());
        for keycode in pressed_keys {
            let raw_keycode = keycode.raw();
            #[cfg(feature = "flutter")]
            let was_flutter = active_flutter_keys.contains(&raw_keycode);
            #[cfg(feature = "flutter")]
            let was_flutter_input_method = active_input_method_keys.contains(&raw_keycode);
            #[cfg(not(feature = "flutter"))]
            let was_flutter = false;
            let was_retired = previously_retired_keys.contains(&raw_keycode);
            keyboard.input::<(), _>(
                state,
                keycode,
                KeyState::Released,
                SERIAL_COUNTER.next_serial(),
                time,
                move |state, modifiers, key| {
                    #[cfg(not(feature = "flutter"))]
                    let _ = (&state, &modifiers, &key);
                    #[cfg(feature = "flutter")]
                    if (was_flutter || was_flutter_input_method) && state.flutter_active {
                        state
                            .flutter_input
                            .handle_keyboard(key, KeyState::Released, modifiers);
                    }
                    if was_flutter || was_retired {
                        FilterResult::Intercept(())
                    } else {
                        FilterResult::Forward
                    }
                },
            );
        }
        if keyboard.is_grabbed() {
            keyboard.unset_grab(state);
        }
    }
    state.scene_sync.mark_dirty();
}

#[cfg(feature = "flutter")]
fn flutter_unicode_for_keysym(
    compose: Option<&mut xkb::compose::State>,
    keysym: xkb::Keysym,
) -> u32 {
    let direct = || keysym.key_char().map(u32::from).unwrap_or(0);
    let Some(compose) = compose else {
        return direct();
    };
    match compose.feed(keysym) {
        xkb::compose::FeedResult::Ignored => direct(),
        xkb::compose::FeedResult::Accepted => match compose.status() {
            xkb::compose::Status::Nothing => direct(),
            xkb::compose::Status::Composing | xkb::compose::Status::Cancelled => 0,
            xkb::compose::Status::Composed => compose
                .utf8()
                .as_deref()
                .and_then(single_unicode_scalar)
                .or_else(|| {
                    compose
                        .keysym()
                        .and_then(|symbol| symbol.key_char().map(u32::from))
                })
                .unwrap_or(0),
        },
    }
}

#[cfg(feature = "flutter")]
fn single_unicode_scalar(value: &str) -> Option<u32> {
    let mut characters = value.chars();
    let character = characters.next()?;
    characters.next().is_none().then(|| u32::from(character))
}

#[cfg(feature = "flutter")]
fn flutter_key_repeats(key: &smithay::input::keyboard::KeysymHandle<'_>) -> bool {
    let xkb = key.xkb().lock().unwrap();
    // SAFETY: the keymap reference is used only while the owning XKB mutex is
    // held and is not retained beyond this call.
    unsafe { xkb.keymap() }.key_repeats(key.raw_code())
}

#[cfg(feature = "flutter")]
fn retained_flutter_xkb_keycode(keycode: u32) -> Keycode {
    // flutter_keyboard_keys retains Smithay/XKB keycodes, which already
    // include XKB's evdev + 8 offset. Replay that value unchanged; adding the
    // offset again turns XKB Backspace (22) into XKB U (30).
    Keycode::new(keycode)
}

#[cfg(feature = "flutter")]
fn cancel_flutter_repeat(state: &mut RuntimeState) {
    let Some(frontend) = state.wayland.as_mut() else {
        return;
    };
    frontend.flutter_repeat_generation = frontend.flutter_repeat_generation.wrapping_add(1);
    frontend.flutter_repeat_key = None;
    if let Some(token) = frontend.flutter_repeat_token.take() {
        frontend.loop_handle.remove(token);
    }
}

#[cfg(feature = "flutter")]
fn start_flutter_repeat(state: &mut RuntimeState, keycode: u32) {
    cancel_flutter_repeat(state);
    let Some(frontend) = state.wayland.as_mut() else {
        return;
    };
    let rate = frontend.settings.keyboard().repeat_rate_hz;
    if rate == 0 {
        return;
    }
    let delay =
        std::time::Duration::from_millis(u64::from(frontend.settings.keyboard().repeat_delay_ms));
    let interval = std::time::Duration::from_secs_f64(1.0 / f64::from(rate));
    frontend.flutter_repeat_generation = frontend.flutter_repeat_generation.wrapping_add(1);
    let generation = frontend.flutter_repeat_generation;
    frontend.flutter_repeat_key = Some(keycode);
    let loop_handle = frontend.loop_handle.clone();
    match loop_handle.insert_source(Timer::from_duration(delay), move |_, _, state| {
        let current = state.wayland.as_ref().is_some_and(|frontend| {
            frontend.flutter_repeat_generation == generation
                && frontend.flutter_repeat_key == Some(keycode)
        });
        if !current || !dispatch_flutter_repeat(state, keycode) {
            return TimeoutAction::Drop;
        }
        TimeoutAction::ToDuration(interval)
    }) {
        Ok(token) => {
            state
                .wayland
                .as_mut()
                .expect("missing Wayland frontend")
                .flutter_repeat_token = Some(token);
        }
        Err(error) => {
            warn!(%error, "could not schedule Flutter keyboard repeat");
            cancel_flutter_repeat(state);
        }
    }
}

#[cfg(feature = "flutter")]
fn dispatch_flutter_repeat(state: &mut RuntimeState, keycode: u32) -> bool {
    if !state.flutter_active {
        return false;
    }
    let Some(keyboard) = state
        .wayland
        .as_ref()
        .and_then(|frontend| frontend.seat.get_keyboard())
    else {
        return false;
    };
    let owned = state
        .wayland
        .as_ref()
        .is_some_and(|frontend| frontend.flutter_keyboard_keys.contains(&keycode));
    if !owned {
        return false;
    }
    let xkb_keycode = retained_flutter_xkb_keycode(keycode);
    let keysym = keyboard.with_xkb_state(state, |context| {
        let xkb = context.xkb().lock().unwrap();
        // SAFETY: the state reference remains inside the XKB mutex guard.
        unsafe { xkb.state() }.key_get_one_sym(xkb_keycode)
    });
    let modifiers = keyboard.modifier_state();
    let unicode = {
        let frontend = state.wayland.as_mut().expect("missing Wayland frontend");
        flutter_unicode_for_keysym(frontend.flutter_compose.as_mut(), keysym)
    };
    state.flutter_input.handle_keyboard_with_unicode(
        xkb_keycode.raw(),
        KeyState::Pressed,
        &modifiers,
        unicode,
    );
    true
}

/// Deliver a key returned by the external input method to its Flutter editor.
///
/// The physical transition has already updated Smithay's XKB state before the
/// input-method grab received it. Reusing that state here avoids a second XKB
/// transition and, critically, does not re-enter the input-method grab. Keys
/// not owned by Flutter remain on the ordinary virtual-keyboard path.
#[cfg(feature = "flutter")]
pub(super) fn dispatch_input_method_key_to_flutter(
    state: &mut RuntimeState,
    keyboard: &KeyboardHandle<RuntimeState>,
    keycode: Keycode,
    key_state: KeyState,
    flutter_editor_active: bool,
) -> bool {
    let disposition = {
        let frontend = state.wayland.as_mut().expect("missing Wayland frontend");
        route_input_method_key_transition(
            &mut frontend.flutter_input_method_keys,
            &mut frontend.retired_input_method_keys,
            keycode.raw(),
            key_state,
            state.flutter_active && flutter_editor_active,
        )
    };
    match disposition {
        FlutterKeyDisposition::Forward => return false,
        FlutterKeyDisposition::ConsumeRetired => return true,
        FlutterKeyDisposition::Dispatch if !state.flutter_active => return true,
        FlutterKeyDisposition::Dispatch => {}
    }

    let keysym = keyboard.with_xkb_state(state, |context| {
        let xkb = context.xkb().lock().unwrap();
        // SAFETY: the state reference remains inside the XKB mutex guard.
        unsafe { xkb.state() }.key_get_one_sym(keycode)
    });
    let modifiers = keyboard.modifier_state();
    let unicode = if matches!(key_state, KeyState::Pressed) {
        let frontend = state.wayland.as_mut().expect("missing Wayland frontend");
        flutter_unicode_for_keysym(frontend.flutter_compose.as_mut(), keysym)
    } else {
        keysym.key_char().map(u32::from).unwrap_or(0)
    };
    state
        .flutter_input
        .handle_keyboard_with_unicode(keycode.raw(), key_state, &modifiers, unicode);
    true
}

fn intercept_native_escape(
    state: &mut RuntimeState,
    xkb_keycode: u32,
    key_state: KeyState,
) -> bool {
    // Smithay/XKB keycodes carry the conventional eight-code offset over the
    // Linux evdev values emitted by libinput.
    let Some(evdev_keycode) = xkb_keycode.checked_sub(8) else {
        return false;
    };
    let disposition = state
        .native_escape_shortcut
        .observe(evdev_keycode, key_state == KeyState::Pressed);
    #[cfg(feature = "flutter")]
    if state.secure_session_locked() {
        return match disposition {
            ShortcutDisposition::Forward => false,
            ShortcutDisposition::RequestLock => {
                if let Some(authentication) = state.authentication.as_ref() {
                    authentication.lock();
                }
                true
            }
            ShortcutDisposition::RequestShutdown => {
                state
                    .lifecycle
                    .request_shutdown(ShutdownReason::NativeEscapeShortcut);
                true
            }
            // Shortcut state still observes every transition so releases stay
            // balanced, but locked sessions cannot trigger client/window or
            // system-control actions.
            _ => true,
        };
    }
    execute_shortcut_disposition(state, disposition)
}

pub(super) fn execute_shortcut_disposition(
    state: &mut RuntimeState,
    disposition: ShortcutDisposition,
) -> bool {
    match disposition {
        ShortcutDisposition::Forward => false,
        ShortcutDisposition::Consume => true,
        ShortcutDisposition::RequestShutdown => {
            state
                .lifecycle
                .request_shutdown(ShutdownReason::NativeEscapeShortcut);
            true
        }
        ShortcutDisposition::RequestApplications => {
            #[cfg(feature = "flutter")]
            state.queue_shell_action(super::super::wire::ShellAction::Applications, None);
            true
        }
        ShortcutDisposition::RequestOverview => {
            #[cfg(feature = "flutter")]
            {
                let monitor_id = prepare_shell_overlay_action(state);
                state.queue_shell_action(super::super::wire::ShellAction::Overview, monitor_id);
            }
            true
        }
        ShortcutDisposition::RequestToggleVerticalMaximize => {
            #[cfg(feature = "flutter")]
            super::window_management::toggle_shell_vertical_maximize_focused_toplevel(state);
            true
        }
        ShortcutDisposition::RequestWindowSwitcherNext => {
            #[cfg(feature = "flutter")]
            {
                let monitor_id = prepare_shell_overlay_action(state);
                state.queue_shell_action(
                    super::super::wire::ShellAction::WindowSwitcherNext,
                    monitor_id,
                );
            }
            true
        }
        ShortcutDisposition::RequestWindowSwitcherEnd { forward } => {
            #[cfg(feature = "flutter")]
            state.queue_shell_action(super::super::wire::ShellAction::WindowSwitcherEnd, None);
            !forward
        }
        ShortcutDisposition::RequestClipboard => {
            #[cfg(feature = "flutter")]
            {
                let monitor_id = prepare_shell_overlay_action(state);
                state.queue_shell_action(super::super::wire::ShellAction::Clipboard, monitor_id);
            }
            true
        }
        ShortcutDisposition::RequestScreenshotRegion => {
            #[cfg(feature = "flutter")]
            {
                let monitor_id = prepare_shell_overlay_action(state);
                state.request_screenshot_selection(monitor_id);
            }
            true
        }
        ShortcutDisposition::RequestMinimize => {
            #[cfg(feature = "flutter")]
            super::window_management::minimize_focused_toplevel(state);
            true
        }
        ShortcutDisposition::RequestClose => {
            #[cfg(feature = "flutter")]
            super::window_management::close_focused_toplevel(state);
            true
        }
        ShortcutDisposition::RequestToggleMaximize => {
            #[cfg(feature = "flutter")]
            super::window_management::toggle_shell_maximize_focused_toplevel(state);
            true
        }
        ShortcutDisposition::RequestToggleFullscreen => {
            #[cfg(feature = "flutter")]
            super::window_management::toggle_shell_fullscreen_focused_toplevel(state);
            true
        }
        ShortcutDisposition::RequestReleasePointer => {
            #[cfg(feature = "flutter")]
            release_pointer_to_shell(state);
            true
        }
        ShortcutDisposition::RequestLock => {
            #[cfg(feature = "flutter")]
            if let Some(authentication) = state.authentication.as_ref() {
                authentication.lock();
            }
            true
        }
        ShortcutDisposition::RequestVolumeUp => {
            if let Some(controls) = state.system_controls.as_ref() {
                controls.volume_up();
            }
            true
        }
        ShortcutDisposition::RequestVolumeDown => {
            if let Some(controls) = state.system_controls.as_ref() {
                controls.volume_down();
            }
            true
        }
        ShortcutDisposition::RequestMute => {
            if let Some(controls) = state.system_controls.as_ref() {
                controls.toggle_mute();
            }
            true
        }
        ShortcutDisposition::RequestBrightnessUp => {
            adjust_brightness_for_pointer_output(state, true);
            true
        }
        ShortcutDisposition::RequestBrightnessDown => {
            adjust_brightness_for_pointer_output(state, false);
            true
        }
        ShortcutDisposition::RequestNextKeyboardLayout => {
            cycle_keyboard_layout(state, true);
            true
        }
        ShortcutDisposition::RequestPreviousKeyboardLayout => {
            cycle_keyboard_layout(state, false);
            true
        }
        ShortcutDisposition::Spawn(arguments) => {
            #[cfg(feature = "flutter")]
            state
                .pending_shortcut_launches
                .push_back(ShortcutTarget::Spawn { command: arguments });
            true
        }
        ShortcutDisposition::SpawnSh(command) => {
            #[cfg(feature = "flutter")]
            state
                .pending_shortcut_launches
                .push_back(ShortcutTarget::SpawnSh { command });
            true
        }
    }
}

fn cycle_keyboard_layout(state: &mut RuntimeState, forward: bool) {
    let Some(keyboard) = state
        .wayland
        .as_ref()
        .and_then(|frontend| frontend.seat.get_keyboard())
    else {
        return;
    };
    let active = keyboard.with_xkb_state(state, |mut context| {
        if forward {
            context.cycle_next_layout();
        } else {
            context.cycle_prev_layout();
        }
        context.xkb().lock().unwrap().active_layout().0 as usize
    });
    publish_active_keyboard_layout(state, active);
}

fn synchronize_active_keyboard_layout(
    state: &mut RuntimeState,
    keyboard: &KeyboardHandle<RuntimeState>,
) {
    let active = keyboard.with_xkb_state(state, |context| {
        context.xkb().lock().unwrap().active_layout().0 as usize
    });
    publish_active_keyboard_layout(state, active);
}

fn publish_active_keyboard_layout(state: &mut RuntimeState, active: usize) {
    let Some(frontend) = state.wayland.as_mut() else {
        return;
    };
    if frontend.active_keyboard_layout == active {
        return;
    }
    frontend.active_keyboard_layout = active;
    frontend.keyboard_configuration_changed = true;
    let name = frontend
        .keyboard_layout_names
        .get(active)
        .map(String::as_str)
        .unwrap_or("unknown");
    info!(
        layout_index = active,
        layout_name = name,
        "switched keyboard layout"
    );
}

#[cfg(feature = "flutter")]
fn release_pointer_to_shell(state: &mut RuntimeState) {
    let Some(pointer) = state
        .wayland
        .as_ref()
        .and_then(|frontend| frontend.seat.get_pointer())
    else {
        return;
    };
    let focused_surface = pointer.current_focus();
    let released_window_id = focused_surface.as_ref().and_then(|surface| {
        state
            .wayland
            .as_ref()
            .and_then(|frontend| frontend.window_id_for_input_surface(surface))
    });
    let had_constraint = focused_surface.as_ref().is_some_and(|surface| {
        with_pointer_constraint(surface, &pointer, |constraint| {
            let Some(constraint) = constraint else {
                return false;
            };
            constraint.deactivate();
            true
        })
    });

    let (mut pressed_buttons, time) = {
        let frontend = state.wayland.as_mut().expect("missing Wayland frontend");
        if let Some(window_id) = released_window_id {
            frontend.pointer_constraint_escape.release_window(window_id);
        }
        frontend.client_pointer_capture = None;
        frontend.client_pointer_buttons.clear();
        frontend.client_pointer_presses.clear();
        frontend.flutter_pointer_press = None;
        frontend.set_clipboard_drag_active(false);
        let mut pressed_buttons = std::mem::take(&mut frontend.wayland_pointer_buttons)
            .into_iter()
            .collect::<Vec<_>>();
        pressed_buttons.sort_unstable();
        frontend
            .retired_pointer_buttons
            .extend(pressed_buttons.iter().copied());
        frontend.update_cursor_image(CursorImageStatus::default_named());
        (
            pressed_buttons,
            frontend.start_time.elapsed().as_millis() as u32,
        )
    };

    let had_grab = pointer.is_grabbed();
    for button in pressed_buttons.drain(..) {
        pointer.button(
            state,
            &ButtonEvent {
                button,
                state: ButtonState::Released,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
    }
    if pointer.is_grabbed() {
        pointer.unset_grab(state, SERIAL_COUNTER.next_serial(), time);
    }
    if had_grab
        || !state
            .wayland
            .as_ref()
            .expect("missing Wayland frontend")
            .retired_pointer_buttons
            .is_empty()
    {
        pointer.frame(state);
    }
    state.scene_sync.mark_dirty();
    info!(
        window_id = ?released_window_id,
        had_constraint,
        had_grab,
        "released pointer capture until the client is clicked again"
    );
}

#[cfg(feature = "flutter")]
fn prepare_shell_overlay_action(state: &mut RuntimeState) -> Option<i64> {
    let pointer = state
        .wayland
        .as_ref()
        .and_then(|frontend| frontend.seat.get_pointer());
    let focused_surface = pointer.as_ref().and_then(PointerHandle::current_focus);
    let released_constraint = match (pointer, focused_surface) {
        (Some(pointer), Some(surface)) => {
            with_pointer_constraint(&surface, &pointer, |constraint| {
                let Some(constraint) = constraint else {
                    return false;
                };
                if !constraint.is_active() {
                    return false;
                }
                constraint.deactivate();
                true
            })
        }
        _ => false,
    };
    if released_constraint {
        if let Some(frontend) = state.wayland.as_mut() {
            frontend.update_cursor_image(CursorImageStatus::default_named());
        }
        state.scene_sync.mark_dirty();
    }

    state
        .wayland
        .as_ref()
        .and_then(WaylandFrontend::control_output_under_pointer)
        .map(|(_, monitor_id)| monitor_id)
}

fn adjust_brightness_for_pointer_output(state: &RuntimeState, increase: bool) {
    let Some((connector, monitor_id)) = state
        .wayland
        .as_ref()
        .and_then(WaylandFrontend::control_output_under_pointer)
    else {
        warn!("brightness shortcut has no output under the pointer");
        return;
    };
    let Some(controls) = state.system_controls.as_ref() else {
        return;
    };
    if increase {
        controls.brightness_up(connector.to_owned(), monitor_id);
    } else {
        controls.brightness_down(connector.to_owned(), monitor_id);
    }
}

#[cfg(feature = "flutter")]
fn process_flutter_keyboard_transition(
    state: &mut RuntimeState,
    keycode: Keycode,
    key_state: KeyState,
    time: u32,
) -> bool {
    let secure_locked = state.secure_session_locked();
    let raw_keycode = keycode.raw();
    let keyboard = state
        .wayland
        .as_ref()
        .expect("missing Wayland frontend")
        .seat
        .get_keyboard()
        .expect("seat has no keyboard");
    let keyboard_grabbed = keyboard.is_grabbed();
    let disposition = {
        let frontend = state.wayland.as_mut().expect("missing Wayland frontend");
        let capture_new_press = matches!(key_state, KeyState::Pressed)
            && (secure_locked
                || (frontend.text_input.shell_captures_keyboard() && !keyboard_grabbed));
        route_flutter_key_transition(
            &mut frontend.flutter_keyboard_keys,
            &mut frontend.retired_keyboard_keys,
            raw_keycode,
            key_state,
            capture_new_press,
        )
    };
    keyboard.input::<(), _>(
        state,
        keycode,
        key_state,
        SERIAL_COUNTER.next_serial(),
        time,
        move |state, modifiers, key| match disposition {
            FlutterKeyDisposition::Dispatch => {
                let repeatable =
                    matches!(key_state, KeyState::Pressed) && flutter_key_repeats(&key);
                let unicode = if matches!(key_state, KeyState::Pressed) {
                    let frontend = state.wayland.as_mut().expect("missing Wayland frontend");
                    flutter_unicode_for_keysym(
                        frontend.flutter_compose.as_mut(),
                        key.modified_sym(),
                    )
                } else {
                    key.modified_sym().key_char().map(u32::from).unwrap_or(0)
                };
                state.flutter_input.handle_keyboard_with_unicode(
                    key.raw_code().raw(),
                    key_state,
                    modifiers,
                    unicode,
                );
                if repeatable {
                    start_flutter_repeat(state, raw_keycode);
                } else if matches!(key_state, KeyState::Released)
                    && state
                        .wayland
                        .as_ref()
                        .is_some_and(|frontend| frontend.flutter_repeat_key == Some(raw_keycode))
                {
                    cancel_flutter_repeat(state);
                }
                FilterResult::Intercept(())
            }
            FlutterKeyDisposition::ConsumeRetired => FilterResult::Intercept(()),
            FlutterKeyDisposition::Forward => FilterResult::Forward,
        },
    );
    synchronize_active_keyboard_layout(state, &keyboard);
    true
}

#[cfg(test)]
#[path = "input/tests.rs"]
mod tests;
