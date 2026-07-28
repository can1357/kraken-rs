pub(crate) mod ai;
pub(crate) mod drive;
#[cfg(target_os = "macos")]
pub(crate) mod native_menu;
pub(crate) mod palette;
pub(crate) mod state;

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use num_traits::ToPrimitive;
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalSize},
    event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{Key, ModifiersState, NamedKey},
    window::Window,
};

use crate::{
    app::state::AppState,
    gpu::{offscreen::OffscreenRenderer, window::WindowRenderer},
    ui::{action::UiAction, slab::SlabHostCommand},
};

pub(crate) use state::ScreenshotView;

/// Cross-thread completions that make the native event loop process fresh state.
#[derive(Clone, Copy, Debug)]
pub(crate) enum UserEvent {
    /// A queued Git operation completed.
    Git,
    /// An AI provider request completed.
    Ai,
    /// An avatar fetch completed.
    Avatar,
    /// A repository filesystem refresh completed.
    Filesystem,
    /// The embedded terminal produced output or exited.
    Terminal,
}

/// Process-level launch configuration shared by windowed and screenshot modes.
#[derive(Clone, Debug)]
pub(crate) struct LaunchOptions {
    pub(crate) repo: Option<PathBuf>,
    pub(crate) screenshot: Option<ScreenshotView>,
    pub(crate) drive_port: Option<u16>,
    pub(crate) output: PathBuf,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

const ANIMATION_FRAME_INTERVAL: Duration = Duration::from_millis(16);

const MULTI_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const MULTI_CLICK_DISTANCE_SQ: f64 = 4.0 * 4.0;

struct Click {
    at: Instant,
    x: f64,
    y: f64,
    button: u32,
    count: u32,
}

#[derive(Default)]
struct ClickCounter {
    last: Option<Click>,
}

impl ClickCounter {
    fn pointer_down(&mut self, button: u32, x: f64, y: f64) -> u32 {
        let now = Instant::now();
        let count = self.last.as_ref().map_or(1, |last| {
            let dx = x - last.x;
            let dy = y - last.y;
            if last.button == button
                && now.duration_since(last.at) <= MULTI_CLICK_INTERVAL
                && dx * dx + dy * dy <= MULTI_CLICK_DISTANCE_SQ
            {
                last.count.saturating_add(1)
            } else {
                1
            }
        });
        self.last = Some(Click {
            at: now,
            x,
            y,
            button,
            count,
        });
        count
    }
}

fn modifier_bits(modifiers: ModifiersState) -> u32 {
    let mut bits = 0;
    if modifiers.shift_key() {
        bits |= slab_kernel::dispatch::M_SHIFT;
    }
    if modifiers.alt_key() {
        bits |= slab_kernel::dispatch::M_ALT;
    }
    if modifiers.control_key() {
        bits |= slab_kernel::dispatch::M_CTRL;
    }
    if modifiers.super_key() {
        bits |= slab_kernel::dispatch::M_META;
    }
    bits
}

fn base_event(
    etype: u32,
    state: &AppState,
    modifiers: ModifiersState,
) -> slab_kernel::dispatch::Event {
    slab_kernel::dispatch::Event {
        etype,
        x: f64::from(state.mouse[0]),
        y: f64::from(state.mouse[1]),
        dx: 0.0,
        dy: 0.0,
        button: 0,
        clicks: 0,
        key: String::new(),
        text: String::new(),
        mods: modifier_bits(modifiers),
    }
}

fn pointer_button(button: MouseButton) -> Option<u32> {
    match button {
        MouseButton::Left => Some(0),
        MouseButton::Middle => Some(1),
        MouseButton::Right => Some(2),
        _ => None,
    }
}

fn key_name(key: &Key) -> Option<String> {
    let name = match key {
        Key::Named(named) => match named {
            NamedKey::Enter => "Enter",
            NamedKey::Tab => "Tab",
            NamedKey::Space => " ",
            NamedKey::Escape => "Escape",
            NamedKey::Backspace => "Backspace",
            NamedKey::Delete => "Delete",
            NamedKey::Insert => "Insert",
            NamedKey::Home => "Home",
            NamedKey::End => "End",
            NamedKey::PageUp => "PageUp",
            NamedKey::PageDown => "PageDown",
            NamedKey::ArrowLeft => "ArrowLeft",
            NamedKey::ArrowRight => "ArrowRight",
            NamedKey::ArrowUp => "ArrowUp",
            NamedKey::ArrowDown => "ArrowDown",
            NamedKey::F1 => "F1",
            NamedKey::F2 => "F2",
            NamedKey::F3 => "F3",
            NamedKey::F4 => "F4",
            NamedKey::F5 => "F5",
            NamedKey::F6 => "F6",
            NamedKey::F7 => "F7",
            NamedKey::F8 => "F8",
            NamedKey::F9 => "F9",
            NamedKey::F10 => "F10",
            NamedKey::F11 => "F11",
            NamedKey::F12 => "F12",
            NamedKey::F13 => "F13",
            NamedKey::F14 => "F14",
            NamedKey::F15 => "F15",
            NamedKey::F16 => "F16",
            NamedKey::F17 => "F17",
            NamedKey::F18 => "F18",
            NamedKey::F19 => "F19",
            NamedKey::F20 => "F20",
            NamedKey::F21 => "F21",
            NamedKey::F22 => "F22",
            NamedKey::F23 => "F23",
            NamedKey::F24 => "F24",
            _ => return None,
        },
        Key::Character(character) => return Some(character.to_string()),
        _ => return None,
    };
    Some(name.to_owned())
}

fn apply_host_commands(
    event_loop: &ActiveEventLoop,
    renderer: &WindowRenderer,
    commands: Vec<SlabHostCommand>,
) {
    for command in commands {
        match command {
            SlabHostCommand::Close => event_loop.exit(),
            SlabHostCommand::Minimize => renderer.window().set_minimized(true),
            SlabHostCommand::ToggleMaximize => {
                renderer
                    .window()
                    .set_maximized(!renderer.window().is_maximized());
            }
            SlabHostCommand::DragWindow => {
                let _ = renderer.window().drag_window();
            }
        }
    }
}

/// Renders and presents synchronously so the new drawable is committed inside
/// the `CATransaction` that resized the window (see `presentsWithTransaction` in
/// `WindowRenderer::new`); deferring via `request_redraw` lets the compositor
/// stretch the previous frame to the new bounds.
fn render_now(state: &mut AppState, renderer: &mut WindowRenderer) {
    state.advance_animations();
    state.process_events();
    renderer.render(state);
}

fn clipboard_text() -> Option<String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.get_text())
        .ok()
}

fn set_clipboard_text(text: String) {
    if text.is_empty() {
        return;
    }
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(text);
    }
}

/// Runs one headless frame or enters the native winit event loop.
pub(crate) fn run(options: LaunchOptions) -> Result<()> {
    if let Some(port) = options.drive_port {
        return drive::run(options.repo, options.width, options.height, port);
    }
    if let Some(view) = options.screenshot {
        let state = AppState::for_screenshot(options.repo, view, options.width, options.height)?;
        let mut renderer = pollster::block_on(OffscreenRenderer::new())?;
        renderer.render_png(&state, &options.output)?;
        println!("wrote {}", options.output.display());
        return Ok(());
    }

    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .context("create native event loop")?;
    let mut application = NativeApplication::new(options, event_loop.create_proxy());
    event_loop
        .run_app(&mut application)
        .context("run native application")
}

struct NativeApplication {
    repo: Option<PathBuf>,
    requested_width: u32,
    requested_height: u32,
    state: Option<AppState>,
    renderer: Option<WindowRenderer>,
    modifiers: ModifiersState,
    clicks: ClickCounter,
    composing: bool,
    event_loop_proxy: EventLoopProxy<UserEvent>,
    next_animation_frame: Option<Instant>,
}

impl NativeApplication {
    fn new(options: LaunchOptions, event_loop_proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            repo: options.repo,
            requested_width: options.width,
            requested_height: options.height,
            state: None,
            renderer: None,
            modifiers: ModifiersState::default(),
            clicks: ClickCounter::default(),
            composing: false,
            event_loop_proxy,
            next_animation_frame: None,
        }
    }

    fn request_redraw(&self) {
        if let Some(renderer) = &self.renderer {
            renderer.window().request_redraw();
        }
    }
}

fn logical_size(size: PhysicalSize<u32>, scale_factor: f64) -> (u32, u32) {
    let size = size.to_logical::<f64>(scale_factor);
    (
        size.width.round().to_u32().unwrap_or(1).max(1),
        size.height.round().to_u32().unwrap_or(1).max(1),
    )
}

fn logical_window_size(window: &Window) -> (u32, u32) {
    logical_size(window.inner_size(), window.scale_factor())
}

impl ApplicationHandler<UserEvent> for NativeApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        let width = self.requested_width.clamp(960, 1_800);
        let height = self.requested_height.clamp(640, 1_100);
        let attributes = Window::default_attributes()
            .with_title("Kraken Native")
            .with_inner_size(LogicalSize::new(f64::from(width), f64::from(height)))
            .with_min_inner_size(LogicalSize::new(960.0, 640.0))
            .with_decorations(false);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("failed to create window: {error}");
                event_loop.exit();
                return;
            }
        };
        let renderer = match pollster::block_on(WindowRenderer::new(window, event_loop)) {
            Ok(renderer) => renderer,
            Err(error) => {
                eprintln!("failed to initialize GPU: {error:#}");
                event_loop.exit();
                return;
            }
        };
        let (width, height) = logical_window_size(renderer.window());
        self.state = Some(AppState::new(
            self.repo.take(),
            width,
            height,
            Some(self.event_loop_proxy.clone()),
        ));
        self.renderer = Some(renderer);
        self.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let (Some(state), Some(renderer)) = (&mut self.state, &mut self.renderer) else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => {
                let event = base_event(slab_kernel::dispatch::E_CLOSE, state, self.modifiers);
                let outcome = renderer.dispatch(state, &event);
                apply_host_commands(event_loop, renderer, outcome.host_commands);
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                renderer.resize(size.width, size.height);
                let (width, height) = logical_window_size(renderer.window());
                state.resize(width, height);
                let mut event = base_event(slab_kernel::dispatch::E_RESIZE, state, self.modifiers);
                event.dx = f64::from(width);
                event.dy = f64::from(height);
                // Render synchronously inside the resize transaction; the
                // repaint effect must not queue a second render (see
                // `dispatch_without_redraw`).
                let outcome = renderer.dispatch_without_redraw(state, &event);
                apply_host_commands(event_loop, renderer, outcome.host_commands);
                render_now(state, renderer);
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let size = renderer.window().inner_size();
                renderer.resize(size.width, size.height);
                let (width, height) = logical_window_size(renderer.window());
                state.resize(width, height);
                let mut event = base_event(slab_kernel::dispatch::E_RESIZE, state, self.modifiers);
                event.dx = f64::from(width);
                event.dy = f64::from(height);
                let outcome = renderer.dispatch_without_redraw(state, &event);
                apply_host_commands(event_loop, renderer, outcome.host_commands);
                render_now(state, renderer);
            }
            WindowEvent::Occluded(false) => {
                // Renders skipped while hidden never queue retries; repaint
                // as soon as the window is visible again.
                renderer.window().request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let position = position.to_logical::<f64>(renderer.window().scale_factor());
                state.mouse = [
                    position.x.to_f32().unwrap_or(0.0),
                    position.y.to_f32().unwrap_or(0.0),
                ];
                let event =
                    base_event(slab_kernel::dispatch::E_POINTER_MOVE, state, self.modifiers);
                let outcome = renderer.dispatch(state, &event);
                apply_host_commands(event_loop, renderer, outcome.host_commands);
            }
            WindowEvent::MouseInput {
                state: button_state,
                button,
                ..
            } => {
                let Some(button) = pointer_button(button) else {
                    return;
                };
                let pressed = button_state == ElementState::Pressed;
                let mut event = base_event(
                    if pressed {
                        slab_kernel::dispatch::E_POINTER_DOWN
                    } else {
                        slab_kernel::dispatch::E_POINTER_UP
                    },
                    state,
                    self.modifiers,
                );
                event.button = button;
                if pressed {
                    event.clicks = self.clicks.pointer_down(event.button, event.x, event.y);
                }
                let outcome = renderer.dispatch(state, &event);
                apply_host_commands(event_loop, renderer, outcome.host_commands);
                #[cfg(target_os = "macos")]
                if pressed && button == 2 && state.context_menu().is_some() {
                    present_native_menu(state, renderer.window());
                }
                #[cfg(target_os = "macos")]
                if !pressed
                    && button == 0
                    && matches!(state.overlay, state::Overlay::DropMenu { .. })
                {
                    present_native_menu(state, renderer.window());
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        (-f64::from(x) * 40.0, -f64::from(y) * 40.0)
                    }
                    MouseScrollDelta::PixelDelta(position) => {
                        let position = position.to_logical::<f64>(renderer.window().scale_factor());
                        (-position.x, -position.y)
                    }
                };
                let mut event = base_event(slab_kernel::dispatch::E_WHEEL, state, self.modifiers);
                event.dx = dx;
                event.dy = dy;
                let outcome = renderer.dispatch(state, &event);
                apply_host_commands(event_loop, renderer, outcome.host_commands);
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
                state.modifier_shift = self.modifiers.shift_key();
                state.modifier_primary = self.modifiers.super_key() || self.modifiers.control_key();
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !self.composing =>
            {
                let command = self.modifiers.super_key();
                let control = self.modifiers.control_key();
                let primary = command || control;
                let shift = self.modifiers.shift_key();
                #[cfg(target_os = "macos")]
                if control
                    && !command
                    && !state.terminal_accepts_input()
                    && matches!(
                        &event.logical_key,
                        Key::Character(character) if character.eq_ignore_ascii_case("c")
                    )
                {
                    event_loop.exit();
                    return;
                }
                let clipboard_modifier = if state.terminal_accepts_input() {
                    command || (!cfg!(target_os = "macos") && control && shift)
                } else {
                    primary
                };
                let shortcut = match &event.logical_key {
                    Key::Character(character)
                        if clipboard_modifier && character.chars().count() == 1 =>
                    {
                        character
                            .chars()
                            .next()
                            .map(|character| character.to_ascii_lowercase())
                    }
                    _ => None,
                };
                let key = key_name(&event.logical_key);
                if !matches!(shortcut, Some('c' | 'x' | 'v'))
                    && let Some(name) = &key
                {
                    let mut kernel =
                        base_event(slab_kernel::dispatch::E_KEY_DOWN, state, self.modifiers);
                    kernel.key.clone_from(name);
                    let outcome = renderer.dispatch(state, &kernel);
                    apply_host_commands(event_loop, renderer, outcome.host_commands);
                }
                match shortcut {
                    Some('c') => {
                        let copy = base_event(slab_kernel::dispatch::E_COPY, state, self.modifiers);
                        let outcome = renderer.dispatch(state, &copy);
                        apply_host_commands(event_loop, renderer, outcome.host_commands);
                        if let Some(text) = renderer.take_copy_text() {
                            set_clipboard_text(text);
                        } else if state.main_view == state::MainView::Diff {
                            state.dispatch(UiAction::CopyDiffText);
                        }
                        return;
                    }
                    Some('x') => {
                        let copy = base_event(slab_kernel::dispatch::E_COPY, state, self.modifiers);
                        let outcome = renderer.dispatch(state, &copy);
                        apply_host_commands(event_loop, renderer, outcome.host_commands);
                        if let Some(text) = renderer.take_copy_text() {
                            set_clipboard_text(text);
                        }
                        let cut = base_event(slab_kernel::dispatch::E_CUT, state, self.modifiers);
                        let outcome = renderer.dispatch(state, &cut);
                        apply_host_commands(event_loop, renderer, outcome.host_commands);
                        return;
                    }
                    Some('v') => {
                        if let Some(text) = clipboard_text() {
                            let mut paste =
                                base_event(slab_kernel::dispatch::E_PASTE, state, self.modifiers);
                            paste.text = text;
                            let outcome = renderer.dispatch(state, &paste);
                            apply_host_commands(event_loop, renderer, outcome.host_commands);
                        }
                        return;
                    }
                    _ => {}
                }

                let insertable = matches!(event.logical_key, Key::Character(_))
                    || event.logical_key == Key::Named(NamedKey::Space);
                if insertable
                    && !primary
                    && let Some(text) = &event.text
                {
                    let mut input =
                        base_event(slab_kernel::dispatch::E_TEXT, state, self.modifiers);
                    input.text = text.to_string();
                    let outcome = renderer.dispatch(state, &input);
                    apply_host_commands(event_loop, renderer, outcome.host_commands);
                }

                if let Some(key) = key {
                    state.handle_key_shortcut(&key, command, primary, shift);
                }
                renderer.window().request_redraw();
            }
            WindowEvent::Ime(ime) => match ime {
                Ime::Enabled => {}
                Ime::Preedit(text, _) if !text.is_empty() => {
                    if !self.composing {
                        self.composing = true;
                        let start = base_event(
                            slab_kernel::dispatch::E_COMPOSITION_START,
                            state,
                            self.modifiers,
                        );
                        let outcome = renderer.dispatch(state, &start);
                        apply_host_commands(event_loop, renderer, outcome.host_commands);
                    }
                    let mut update = base_event(
                        slab_kernel::dispatch::E_COMPOSITION_UPDATE,
                        state,
                        self.modifiers,
                    );
                    update.text = text;
                    let outcome = renderer.dispatch(state, &update);
                    apply_host_commands(event_loop, renderer, outcome.host_commands);
                }
                Ime::Commit(text) => {
                    let mut commit = base_event(
                        if self.composing {
                            self.composing = false;
                            slab_kernel::dispatch::E_COMPOSITION_END
                        } else {
                            slab_kernel::dispatch::E_TEXT
                        },
                        state,
                        self.modifiers,
                    );
                    commit.text = text;
                    let outcome = renderer.dispatch(state, &commit);
                    apply_host_commands(event_loop, renderer, outcome.host_commands);
                }
                Ime::Disabled if self.composing => {
                    self.composing = false;
                    let end = base_event(
                        slab_kernel::dispatch::E_COMPOSITION_END,
                        state,
                        self.modifiers,
                    );
                    let outcome = renderer.dispatch(state, &end);
                    apply_host_commands(event_loop, renderer, outcome.host_commands);
                }
                Ime::Preedit(_, _) | Ime::Disabled => {}
            },
            WindowEvent::Focused(false) => {
                self.composing = false;
                let blur = base_event(slab_kernel::dispatch::E_BLUR, state, self.modifiers);
                let outcome = renderer.dispatch(state, &blur);
                apply_host_commands(event_loop, renderer, outcome.host_commands);
            }
            WindowEvent::RedrawRequested => {
                render_now(state, renderer);
                self.next_animation_frame = state
                    .diff_scroll_animating()
                    .then(|| Instant::now() + ANIMATION_FRAME_INTERVAL);
            }
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: UserEvent) {
        if let Some(state) = &mut self.state {
            state.process_events();
        }
        self.request_redraw();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let state_changed = self.state.as_mut().is_some_and(AppState::process_events);
        let now = Instant::now();
        let animation_due = self
            .next_animation_frame
            .is_some_and(|deadline| deadline <= now);
        if animation_due {
            self.next_animation_frame = None;
        }
        if state_changed || animation_due {
            self.request_redraw();
        }

        if self
            .state
            .as_ref()
            .is_none_or(|state| !state.diff_scroll_animating())
        {
            self.next_animation_frame = None;
        }
        let auto_fetch = self
            .state
            .as_ref()
            .and_then(AppState::next_auto_fetch_deadline);
        let deadline = match (self.next_animation_frame, auto_fetch) {
            (Some(animation), Some(fetch)) => Some(animation.min(fetch)),
            (Some(animation), None) => Some(animation),
            (None, Some(fetch)) => Some(fetch),
            (None, None) => None,
        };
        event_loop.set_control_flow(deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}

/// Presents the state's active context menu natively and dispatches the pick;
/// dismisses the backing overlay when the menu closes without a selection.
#[cfg(target_os = "macos")]
fn present_native_menu(state: &mut AppState, window: &Window) {
    let Some(spec) = state.context_menu() else {
        return;
    };
    let context = state.overlay.clone();
    match native_menu::show(window, &spec) {
        Some(action) => {
            state.dispatch(action);
            if state.overlay == context {
                state.dispatch(UiAction::DismissOverlay);
            }
        }
        None => state.dispatch(UiAction::DismissOverlay),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_window_extent_is_converted_to_logical_pixels() {
        assert_eq!(
            logical_size(PhysicalSize::new(1_600, 1_000), 1.0),
            (1_600, 1_000)
        );
        assert_eq!(
            logical_size(PhysicalSize::new(3_200, 2_000), 2.0),
            (1_600, 1_000)
        );
        assert_eq!(
            logical_size(PhysicalSize::new(2_400, 1_500), 1.5),
            (1_600, 1_000)
        );
    }
}
