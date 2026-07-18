pub(crate) mod ai;
pub(crate) mod automation;
#[cfg(target_os = "macos")]
pub(crate) mod native_menu;
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
    dpi::LogicalSize,
    event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{CursorIcon, Window},
};

use crate::{
    app::state::{AppState, EditKey, EditKeyKind},
    gpu::{offscreen::OffscreenRenderer, window::WindowRenderer},
    ui::{
        Theme,
        action::{CursorHint, UiAction},
    },
    views,
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
    pub(crate) automation_port: Option<u16>,
    pub(crate) output: PathBuf,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

const ANIMATION_FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Runs one headless frame or enters the native winit event loop.
pub(crate) fn run(options: LaunchOptions) -> Result<()> {
    if let Some(port) = options.automation_port {
        return automation::run(options.repo, options.width, options.height, port);
    }
    if let Some(view) = options.screenshot {
        let state = AppState::for_screenshot(options.repo, view, options.width, options.height)?;
        let theme = Theme::dark();
        let scene = views::build_scene(&state, &theme);
        let mut renderer = pollster::block_on(OffscreenRenderer::new())?;
        renderer.render_png(&scene, theme.window, &options.output)?;
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
    theme: Theme,
    modifiers: ModifiersState,
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
            theme: Theme::dark(),
            modifiers: ModifiersState::default(),
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
        let size = window.inner_size();
        let renderer = match pollster::block_on(WindowRenderer::new(window, event_loop)) {
            Ok(renderer) => renderer,
            Err(error) => {
                eprintln!("failed to initialize GPU: {error:#}");
                event_loop.exit();
                return;
            }
        };
        self.state = Some(AppState::new(
            self.repo.take(),
            size.width,
            size.height,
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
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                renderer.resize(size.width, size.height);
                state.resize(size.width, size.height);
                renderer.window().request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let size = renderer.window().inner_size();
                renderer.resize(size.width, size.height);
                state.resize(size.width, size.height);
                renderer.window().request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                state.mouse = [
                    position.x.to_f32().unwrap_or(0.0),
                    position.y.to_f32().unwrap_or(0.0),
                ];
                if state.is_dragging() {
                    state.drag_to(state.mouse[0], state.mouse[1]);
                }
                renderer.window().request_redraw();
            }
            WindowEvent::MouseInput {
                state: button_state,
                button,
                ..
            } => match (button_state, button) {
                (ElementState::Pressed, MouseButton::Left) => {
                    if state.mouse[1] <= 32.0 && state.mouse[0] <= 70.0 {
                        if state.mouse[0] <= 26.0 {
                            event_loop.exit();
                        } else if state.mouse[0] <= 46.0 {
                            renderer.window().set_minimized(true);
                        } else {
                            let fullscreen = renderer.window().fullscreen().is_none();
                            renderer.window().set_fullscreen(
                                fullscreen.then(|| winit::window::Fullscreen::Borderless(None)),
                            );
                        }
                    } else if state.mouse[1] <= 32.0
                        && state.hits.iter().all(|hit| !hit.rect.contains(state.mouse))
                    {
                        let _ = renderer.window().drag_window();
                    } else {
                        state.click();
                    }
                    renderer.window().request_redraw();
                }
                (ElementState::Pressed, MouseButton::Right) => {
                    state.right_click();
                    #[cfg(target_os = "macos")]
                    present_native_menu(state, renderer.window());
                    renderer.window().request_redraw();
                }
                (ElementState::Released, MouseButton::Left) => {
                    state.end_drag();
                    #[cfg(target_os = "macos")]
                    if matches!(state.overlay, state::Overlay::DropMenu { .. }) {
                        present_native_menu(state, renderer.window());
                    }
                    renderer.window().request_redraw();
                }
                _ => {}
            },
            WindowEvent::MouseWheel { delta, .. } => {
                match delta {
                    MouseScrollDelta::LineDelta(_, vertical)
                        if state.main_view == state::MainView::Diff =>
                    {
                        state.scroll_diff_lines(-vertical * 42.0);
                    }
                    MouseScrollDelta::PixelDelta(position)
                        if state.main_view == state::MainView::Diff =>
                    {
                        state.scroll_diff_pixels(-position.y.to_f32().unwrap_or(0.0));
                    }
                    MouseScrollDelta::LineDelta(_, vertical) => state.scroll(-vertical * 42.0),
                    MouseScrollDelta::PixelDelta(position) => {
                        state.scroll(-position.y.to_f32().unwrap_or(0.0));
                    }
                }
                renderer.window().request_redraw();
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let command = self.modifiers.super_key();
                let shift = self.modifiers.shift_key();
                if state.terminal_accepts_input() && !command {
                    let bytes = match &event.logical_key {
                        Key::Named(NamedKey::Backspace) => Some(b"\x7f".as_slice()),
                        Key::Named(NamedKey::Enter) => Some(b"\r".as_slice()),
                        Key::Named(NamedKey::Tab) => Some(b"\t".as_slice()),
                        Key::Named(NamedKey::Escape) => Some(b"\x1b".as_slice()),
                        Key::Named(NamedKey::ArrowUp) => Some(b"\x1b[A".as_slice()),
                        Key::Named(NamedKey::ArrowDown) => Some(b"\x1b[B".as_slice()),
                        Key::Named(NamedKey::ArrowRight) => Some(b"\x1b[C".as_slice()),
                        Key::Named(NamedKey::ArrowLeft) => Some(b"\x1b[D".as_slice()),
                        Key::Character(character) if self.modifiers.control_key() => {
                            let byte = character
                                .as_bytes()
                                .first()
                                .copied()
                                .map(|byte| byte.to_ascii_lowercase() & 0x1f);
                            if let Some(byte) = byte {
                                state.terminal_input(&[byte]);
                            }
                            None
                        }
                        Key::Character(_) => event.text.as_ref().map(|text| text.as_bytes()),
                        _ => None,
                    };
                    if let Some(bytes) = bytes {
                        state.terminal_input(bytes);
                    }
                    renderer.window().request_redraw();
                    return;
                }
                let alt = self.modifiers.alt_key();
                let primary = command || self.modifiers.control_key();
                let edit_kind = match &event.logical_key {
                    Key::Named(NamedKey::ArrowLeft) => Some(EditKeyKind::Left),
                    Key::Named(NamedKey::ArrowRight) => Some(EditKeyKind::Right),
                    Key::Named(NamedKey::ArrowUp) => Some(EditKeyKind::Up),
                    Key::Named(NamedKey::ArrowDown) => Some(EditKeyKind::Down),
                    Key::Named(NamedKey::Home) => Some(EditKeyKind::Home),
                    Key::Named(NamedKey::End) => Some(EditKeyKind::End),
                    Key::Named(NamedKey::Backspace) => Some(EditKeyKind::Backspace),
                    Key::Named(NamedKey::Delete) => Some(EditKeyKind::Delete),
                    Key::Character(text) if primary && text.chars().count() == 1 => {
                        text.chars().next().map(EditKeyKind::Char)
                    }
                    _ => None,
                };
                if let Some(kind) = edit_kind
                    && state.edit_key(EditKey {
                        kind,
                        shift,
                        alt,
                        command: primary,
                    })
                {
                    renderer.window().request_redraw();
                    return;
                }
                match &event.logical_key {
                    Key::Named(NamedKey::F1) => {
                        state.dispatch(if state.main_view == state::MainView::Diff {
                            UiAction::ToggleEditorPalette
                        } else {
                            UiAction::ToggleCommandPalette
                        })
                    }
                    Key::Named(NamedKey::Enter) if state.focus == state::FocusField::Palette => {
                        state.enter(command);
                    }
                    Key::Named(NamedKey::ArrowUp) if state.focus == state::FocusField::Palette => {
                        state.dispatch(UiAction::PalettePrevious);
                    }
                    Key::Named(NamedKey::ArrowDown)
                        if state.focus == state::FocusField::Palette =>
                    {
                        state.dispatch(UiAction::PaletteNext);
                    }
                    Key::Named(NamedKey::Enter)
                        if shift && state.focus == state::FocusField::DiffSearch =>
                    {
                        state.dispatch(UiAction::PreviousDiffSearch);
                    }
                    Key::Named(NamedKey::ArrowUp) if state.focus == state::FocusField::Search => {
                        state.dispatch(UiAction::PreviousSearchResult);
                    }
                    Key::Named(NamedKey::ArrowDown) if state.focus == state::FocusField::Search => {
                        state.dispatch(UiAction::NextSearchResult);
                    }
                    Key::Named(NamedKey::ArrowUp)
                        if state.focus == state::FocusField::DiffSearch =>
                    {
                        state.dispatch(UiAction::PreviousDiffSearch);
                    }
                    Key::Named(NamedKey::ArrowDown)
                        if state.focus == state::FocusField::DiffSearch =>
                    {
                        state.dispatch(UiAction::NextDiffSearch);
                    }
                    Key::Named(NamedKey::Enter) => state.enter(command),
                    Key::Named(NamedKey::Escape) => state.escape(),
                    Key::Character(character)
                        if (command || self.modifiers.control_key())
                            && shift
                            && character.eq_ignore_ascii_case("p") =>
                    {
                        state.dispatch(if state.main_view == state::MainView::Diff {
                            UiAction::ToggleEditorPalette
                        } else {
                            UiAction::ToggleCommandPalette
                        });
                    }
                    Key::Character(character)
                        if command
                            && character.eq_ignore_ascii_case("c")
                            && state.main_view == state::MainView::Diff =>
                    {
                        state.dispatch(UiAction::CopyDiffText);
                    }
                    Key::Character(character) if command && character.eq_ignore_ascii_case("f") => {
                        state.dispatch(if state.main_view == state::MainView::Diff {
                            UiAction::ToggleDiffSearch
                        } else {
                            UiAction::ToggleSearch
                        });
                    }
                    Key::Character(character) if command && character == "," => {
                        state.dispatch(UiAction::OpenPreferences);
                    }
                    Key::Character(character)
                        if command && shift && character.eq_ignore_ascii_case("a") =>
                    {
                        state.dispatch(UiAction::ToggleTabSwitcher);
                    }
                    Key::Character(_) | Key::Named(_) | Key::Dead(_) | Key::Unidentified(_) => {
                        if !command && let Some(text) = &event.text {
                            state.insert_text(text);
                        }
                    }
                }
                renderer.window().request_redraw();
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                if state.terminal_accepts_input() {
                    state.terminal_input(text.as_bytes());
                } else {
                    state.insert_text(&text);
                }
                renderer.window().request_redraw();
            }
            WindowEvent::RedrawRequested => {
                state.advance_animations();
                state.process_events();
                let scene = views::build_scene(state, &self.theme);
                state.adopt_scene(&scene);
                let cursor = match state.cursor_hint() {
                    CursorHint::Default => CursorIcon::Default,
                    CursorHint::Pointer => CursorIcon::Pointer,
                    CursorHint::ResizeVertical => CursorIcon::NsResize,
                    CursorHint::ResizeHorizontal => CursorIcon::EwResize,
                    CursorHint::Text => CursorIcon::Text,
                };
                renderer.window().set_cursor(cursor);
                if let Err(error) = renderer.render(&scene, self.theme.window) {
                    state.error = Some(format!("{error:#}"));
                }
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
