use std::{
    io::{BufRead, BufReader, BufWriter, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use num_traits::ToPrimitive;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use slab_kernel::{
    dispatch::{
        E_KEY_DOWN, E_POINTER_DOWN, E_POINTER_MOVE, E_POINTER_UP, E_TEXT, E_WHEEL, Event, M_ALT,
        M_CTRL, M_META, M_SHIFT,
    },
    flatten::{Frame, FrameOp, SceneNode},
};

use crate::{
    app::state::{AppState, FocusField, MainView},
    gpu::offscreen::OffscreenRenderer,
    ui::action::UiAction,
};

const PROTOCOL_VERSION: &str = "1.0";

/// Runs the loopback automation endpoint without creating a native window.
pub(crate) fn run(repo: Option<PathBuf>, width: u32, height: u32, port: u16) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("bind automation endpoint on port {port}"))?;
    let address = listener
        .local_addr()
        .context("read automation endpoint address")?;
    let mut server = AutomationServer {
        state: AppState::new(repo, width, height, None),
        renderer: pollster::block_on(OffscreenRenderer::new())?,
    };
    server.refresh();

    println!(
        "{}",
        json!({
            "method": "Automation.ready",
            "params": {
                "host": address.ip().to_string(),
                "port": address.port(),
                "protocolVersion": PROTOCOL_VERSION,
            }
        })
    );
    std::io::stdout()
        .flush()
        .context("publish automation endpoint")?;

    for connection in listener.incoming() {
        let stream = connection.context("accept automation connection")?;
        if server.serve(stream)? {
            break;
        }
    }
    Ok(())
}

struct AutomationServer {
    state: AppState,
    renderer: OffscreenRenderer,
}

#[derive(Debug, Deserialize)]
struct Request {
    id: u64,
    method: String,
    #[serde(default)]
    params: Value,
}

impl AutomationServer {
    fn serve(&mut self, stream: TcpStream) -> Result<bool> {
        stream
            .set_nodelay(true)
            .context("configure automation connection")?;
        let reader_stream = stream.try_clone().context("clone automation connection")?;
        let reader = BufReader::new(reader_stream);
        let mut writer = BufWriter::new(stream);

        for line in reader.lines() {
            let line = line.context("read automation request")?;
            if line.trim().is_empty() {
                continue;
            }
            let request = match serde_json::from_str::<Request>(&line) {
                Ok(request) => request,
                Err(error) => {
                    Self::write_response(
                        &mut writer,
                        &json!({
                            "id": Value::Null,
                            "error": {
                                "code": -32700,
                                "message": format!("invalid request: {error}"),
                            }
                        }),
                    )?;
                    continue;
                }
            };
            let id = request.id;
            let should_close = request.method == "Browser.close";
            let response = match self.execute(&request.method, &request.params) {
                Ok(result) => json!({ "id": id, "result": result }),
                Err(error) => json!({
                    "id": id,
                    "error": {
                        "code": -32000,
                        "message": format!("{error:#}"),
                    }
                }),
            };
            Self::write_response(&mut writer, &response)?;
            if should_close {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn write_response(writer: &mut BufWriter<TcpStream>, response: &Value) -> Result<()> {
        serde_json::to_writer(&mut *writer, response).context("encode automation response")?;
        writer
            .write_all(b"\n")
            .context("terminate automation response")?;
        writer.flush().context("flush automation response")
    }

    fn execute(&mut self, method: &str, params: &Value) -> Result<Value> {
        match method {
            "Protocol.getVersion" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "product": format!("Kraken Native/{}", env!("CARGO_PKG_VERSION")),
            })),
            "App.getState" => {
                self.refresh();
                Ok(self.state_snapshot())
            }
            "App.waitForIdle" => self.wait_for_idle(params),
            "Page.getSnapshot" => {
                self.refresh();
                let frame = self.renderer.semantic_frame(&self.state);
                Ok(frame_snapshot(
                    &frame,
                    self.renderer.scene_strings(),
                    &|node| self.renderer.node_key(node),
                ))
            }
            "Page.captureScreenshot" => self.capture_screenshot(params),
            "Page.setViewport" => self.set_viewport(params),
            "Input.dispatchMouseEvent" => self.dispatch_mouse(params),
            "Input.insertText" => self.insert_text(params),
            "Input.dispatchKeyEvent" => self.dispatch_key(params),
            "UI.click" => self.click(params),
            "Browser.close" => Ok(json!({ "closed": true })),
            _ => bail!("unknown automation method `{method}`"),
        }
    }

    /// Drains background job results into state; the slab document solves on
    /// demand whenever a frame or dispatch needs it.
    fn refresh(&mut self) {
        self.state.process_events();
    }

    /// Routes one synthesized kernel event through the shared terminal-then-
    /// root routing the window uses.
    fn send_event(&mut self, event: &Event) {
        self.renderer.dispatch(&mut self.state, event);
    }

    /// Semantic node under a viewport point in the freshly solved frame.
    fn target_at(&mut self, point: [f32; 2]) -> Option<String> {
        let frame = self.renderer.semantic_frame(&self.state);
        node_under_point(
            &frame,
            self.renderer.scene_strings(),
            &|node| self.renderer.node_key(node),
            point,
        )
    }

    fn wait_for_idle(&mut self, params: &Value) -> Result<Value> {
        let timeout = optional_u64(params, "timeoutMs").unwrap_or(10_000);
        let deadline = Instant::now() + Duration::from_millis(timeout);
        loop {
            self.refresh();
            if self.state.busy_jobs == 0 && !self.state.loading_history && !self.state.ai_loading {
                return Ok(self.state_snapshot());
            }
            if Instant::now() >= deadline {
                bail!("application did not become idle within {timeout} ms");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn capture_screenshot(&mut self, params: &Value) -> Result<Value> {
        let output = PathBuf::from(required_string(params, "path")?);
        self.refresh();
        self.renderer.render_png(&self.state, &output)?;
        Ok(json!({
            "path": output,
            "width": self.state.width,
            "height": self.state.height,
        }))
    }

    fn set_viewport(&mut self, params: &Value) -> Result<Value> {
        let width = required_u32(params, "width")?.max(640);
        let height = required_u32(params, "height")?.max(480);
        self.state.resize(width, height);
        self.refresh();
        Ok(json!({ "width": width, "height": height }))
    }

    /// Mirrors winit's `ModifiersChanged` for synthetic pointer input; absent
    /// params mean an unmodified click.
    fn apply_click_modifiers(&mut self, params: &Value) {
        self.state.modifier_shift = optional_bool(params, "shift").unwrap_or(false);
        self.state.modifier_primary = optional_bool(params, "command").unwrap_or(false)
            || optional_bool(params, "control").unwrap_or(false);
    }

    fn dispatch_mouse(&mut self, params: &Value) -> Result<Value> {
        let event_type = required_string(params, "type")?;
        let x = optional_f32(params, "x").unwrap_or(self.state.mouse[0]);
        let y = optional_f32(params, "y").unwrap_or(self.state.mouse[1]);
        self.apply_click_modifiers(params);
        let modifiers = Modifiers::from_params(params);
        self.state.mouse = [x, y];
        // Solve with the pointer already at the event position so the
        // reported target matches what the press will hit, exactly like the
        // winit path (CursorMoved renders before the press arrives).
        self.refresh();
        let target = self.target_at([x, y]);
        let button = match optional_string(params, "button").unwrap_or("left") {
            "right" => 2,
            _ => 0,
        };

        match event_type {
            "mouseMoved" => {
                let event = base_event(E_POINTER_MOVE, &self.state, modifiers);
                self.send_event(&event);
            }
            "mousePressed" | "mouseClicked" => {
                let mut down = base_event(E_POINTER_DOWN, &self.state, modifiers);
                down.button = button;
                down.clicks = optional_u64(params, "clickCount")
                    .and_then(|count| u32::try_from(count).ok())
                    .unwrap_or(1);
                self.send_event(&down);
                if event_type == "mouseClicked" {
                    let mut up = base_event(E_POINTER_UP, &self.state, modifiers);
                    up.button = button;
                    self.send_event(&up);
                }
            }
            "mouseReleased" => {
                let mut up = base_event(E_POINTER_UP, &self.state, modifiers);
                up.button = button;
                self.send_event(&up);
            }
            "mouseWheel" => {
                let mut wheel = base_event(E_WHEEL, &self.state, modifiers);
                wheel.dx = f64::from(optional_f32(params, "deltaX").unwrap_or(0.0));
                wheel.dy = f64::from(optional_f32(params, "deltaY").unwrap_or(0.0));
                self.send_event(&wheel);
            }
            _ => bail!("unsupported mouse event type `{event_type}`"),
        }
        self.refresh();
        Ok(json!({
            "x": x,
            "y": y,
            "target": target,
        }))
    }

    fn insert_text(&mut self, params: &Value) -> Result<Value> {
        let text = required_string(params, "text")?.to_owned();
        // The shared routing hands typed text to a focused terminal, exactly
        // like the winit IME path.
        let mut event = base_event(E_TEXT, &self.state, Modifiers::default());
        event.text.clone_from(&text);
        self.send_event(&event);
        self.refresh();
        Ok(json!({ "inserted": text }))
    }

    fn dispatch_key(&mut self, params: &Value) -> Result<Value> {
        if optional_string(params, "type").is_some_and(|kind| kind == "keyUp") {
            return Ok(json!({ "ignored": "keyUp" }));
        }
        let mut key = required_string(params, "key")?;
        let mut command = optional_bool(params, "command").unwrap_or(false);
        let mut shift = optional_bool(params, "shift").unwrap_or(false);
        let mut alt = optional_bool(params, "alt").unwrap_or(false);
        let mut control = optional_bool(params, "control").unwrap_or(false);
        // Accept "Alt+ArrowLeft"-style modifier prefixes so QA drivers can
        // express chords the transport schema has no flags for.
        while let Some((prefix, rest)) = key.split_once('+') {
            if rest.is_empty() {
                break;
            }
            match prefix.to_ascii_lowercase().as_str() {
                "cmd" | "meta" | "super" | "command" => command = true,
                "ctrl" | "control" => control = true,
                "alt" | "option" => alt = true,
                "shift" => shift = true,
                _ => break,
            }
            key = rest;
        }
        let primary = command || control;
        let modifiers = Modifiers {
            shift,
            alt,
            control,
            command,
        };
        // Primary clipboard chords never reach the kernel as key events,
        // mirroring the winit routing; everything else goes kernel-first.
        let shortcut = (primary && key.chars().count() == 1)
            .then(|| {
                key.chars()
                    .next()
                    .map(|character| character.to_ascii_lowercase())
            })
            .flatten();
        if !matches!(shortcut, Some('c' | 'x' | 'v')) {
            let mut kernel = base_event(E_KEY_DOWN, &self.state, modifiers);
            key.clone_into(&mut kernel.key);
            self.send_event(&kernel);
        }
        // Plain printable keys insert text, mirroring the window's insertable
        // path fed by winit's `event.text`.
        if !primary && key.chars().count() == 1 {
            let mut input = base_event(E_TEXT, &self.state, modifiers);
            optional_string(params, "text")
                .unwrap_or(key)
                .clone_into(&mut input.text);
            self.send_event(&input);
        }
        match key {
            "F1" => {
                self.state
                    .dispatch(if self.state.main_view == MainView::Diff {
                        UiAction::ToggleEditorPalette
                    } else {
                        UiAction::ToggleCommandPalette
                    });
            }
            "Enter" if self.state.focus == FocusField::Palette => self.state.enter(command),
            "ArrowUp" if self.state.focus == FocusField::Palette => {
                self.state.dispatch(UiAction::PalettePrevious);
            }
            "ArrowDown" if self.state.focus == FocusField::Palette => {
                self.state.dispatch(UiAction::PaletteNext);
            }
            "Enter" if shift && self.state.focus == FocusField::DiffSearch => {
                self.state.dispatch(UiAction::PreviousDiffSearch);
            }
            "Enter" => self.state.enter(command),
            "Escape" => self.state.escape(),
            "ArrowUp" if self.state.focus == FocusField::Search => {
                self.state.dispatch(UiAction::PreviousSearchResult);
            }
            "ArrowDown" if self.state.focus == FocusField::Search => {
                self.state.dispatch(UiAction::NextSearchResult);
            }
            "ArrowUp" if self.state.focus == FocusField::DiffSearch => {
                self.state.dispatch(UiAction::PreviousDiffSearch);
            }
            "ArrowDown" if self.state.focus == FocusField::DiffSearch => {
                self.state.dispatch(UiAction::NextDiffSearch);
            }
            key if command && shift && key.eq_ignore_ascii_case("p") => {
                self.state
                    .dispatch(if self.state.main_view == MainView::Diff {
                        UiAction::ToggleEditorPalette
                    } else {
                        UiAction::ToggleCommandPalette
                    });
            }
            key if command
                && key.eq_ignore_ascii_case("c")
                && self.state.main_view == MainView::Diff =>
            {
                self.state.dispatch(UiAction::CopyDiffText);
            }
            key if command && key.eq_ignore_ascii_case("f") => {
                self.state
                    .dispatch(if self.state.main_view == MainView::Diff {
                        UiAction::ToggleDiffSearch
                    } else {
                        UiAction::ToggleSearch
                    });
            }
            "," if command => self.state.dispatch(UiAction::OpenPreferences),
            key if command && shift && key.eq_ignore_ascii_case("a") => {
                self.state.dispatch(UiAction::ToggleTabSwitcher);
            }
            _ => {}
        }
        self.refresh();
        Ok(json!({ "key": key }))
    }

    fn click(&mut self, params: &Value) -> Result<Value> {
        self.refresh();
        let frame = self.renderer.semantic_frame(&self.state);
        let (point, target) = if let Some(selector) = optional_string(params, "selector") {
            let (point, target) = find_frame_target(
                &frame,
                self.renderer.scene_strings(),
                &|node| self.renderer.node_key(node),
                selector,
            )
            .ok_or_else(|| anyhow!("no visible UI target matches `{selector}`"))?;
            (point, Some(target))
        } else {
            let point = [required_f32(params, "x")?, required_f32(params, "y")?];
            let target = node_under_point(
                &frame,
                self.renderer.scene_strings(),
                &|node| self.renderer.node_key(node),
                point,
            );
            (point, target)
        };
        self.apply_click_modifiers(params);
        let modifiers = Modifiers::from_params(params);
        self.state.mouse = point;
        // A synthetic click is hover + press + release, exactly the sequence
        // the winit path produces for a real click.
        let hover = base_event(E_POINTER_MOVE, &self.state, modifiers);
        self.send_event(&hover);
        let mut down = base_event(E_POINTER_DOWN, &self.state, modifiers);
        down.clicks = 1;
        self.send_event(&down);
        let up = base_event(E_POINTER_UP, &self.state, modifiers);
        self.send_event(&up);
        self.refresh();
        Ok(json!({
            "x": point[0],
            "y": point[1],
            "target": target,
        }))
    }

    fn state_snapshot(&self) -> Value {
        let snapshot = self.state.snapshot.as_ref();
        json!({
            "ready": snapshot.is_some() && self.state.busy_jobs == 0,
            "viewport": {
                "width": self.state.width,
                "height": self.state.height,
            },
            "repository": snapshot.map(|repo| repo.name.as_str()),
            "repositoryPath": snapshot.map(|repo| repo.path.display().to_string()),
            "head": snapshot.map(|repo| repo.head.as_str()),
            "commitCount": snapshot.map_or(0, |repo| repo.commits.len()),
            "workingTree": snapshot.map(|repo| json!({
                "staged": repo.working.staged_count(),
                "unstaged": repo.working.unstaged_count(),
            })),
            "mainView": format!("{:?}", self.state.main_view),
            "overlay": format!("{:?}", self.state.overlay),
            "focus": format!("{:?}", self.state.focus),
            "preferencesOpen": self.state.preferences_open,
            "selectedCommit": self.state.selected_commit,
            "selectedCommits": self.state.selected_commits.len(),
            "selectionRange": self.state.selection_endpoints().map(|(oldest, newest)| json!({
                "oldest": oldest,
                "newest": newest,
            })),
            "selectedFile": self.state.selected_file.as_ref().map(|request| request.path.display().to_string()),
            "busyJobs": self.state.busy_jobs,
            "loadingHistory": self.state.loading_history,
            "error": self.state.error,
            "toast": self.state.toast,
            "scroll": {
                "graph": self.state.graph_scroll,
                "sidebar": self.state.sidebar_scroll,
                "detail": self.state.detail_scroll,
                "wipUnstaged": self.state.wip_unstaged_scroll,
                "wipStaged": self.state.wip_staged_scroll,
                "diff": self.state.diff_scroll,
                "preferences": self.state.preferences_scroll,
            },
        })
    }
}

/// Modifier booleans parsed from request params, mirroring the winit
/// `ModifiersState` the windowed path tracks.
#[derive(Clone, Copy, Default)]
struct Modifiers {
    shift: bool,
    alt: bool,
    control: bool,
    command: bool,
}

impl Modifiers {
    /// Reads the optional modifier flags carried by a transport request.
    fn from_params(params: &Value) -> Self {
        Self {
            shift: optional_bool(params, "shift").unwrap_or(false),
            alt: optional_bool(params, "alt").unwrap_or(false),
            control: optional_bool(params, "control").unwrap_or(false),
            command: optional_bool(params, "command").unwrap_or(false),
        }
    }

    /// Packs the booleans into kernel modifier bits.
    fn bits(self) -> u32 {
        let mut bits = 0;
        if self.shift {
            bits |= M_SHIFT;
        }
        if self.alt {
            bits |= M_ALT;
        }
        if self.control {
            bits |= M_CTRL;
        }
        if self.command {
            bits |= M_META;
        }
        bits
    }
}

/// Mirrors the windowed `base_event`: pointer position from tracked state and
/// modifier bits from the request-supplied booleans.
fn base_event(etype: u32, state: &AppState, modifiers: Modifiers) -> Event {
    Event {
        etype,
        x: f64::from(state.mouse[0]),
        y: f64::from(state.mouse[1]),
        dx: 0.0,
        dy: 0.0,
        button: 0,
        clicks: 0,
        key: String::new(),
        text: String::new(),
        mods: modifiers.bits(),
    }
}

/// Resolves a scene-string reference; zero and out-of-range are absent.
fn scene_str(scene_strs: &[String], index: u32) -> Option<&str> {
    if index == 0 {
        return None;
    }
    scene_strs
        .get(usize::try_from(index).ok()?)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}

/// Whether a scene node occupies visible space.
fn visible(node: &SceneNode) -> bool {
    node.w > 0.0 && node.h > 0.0
}

/// Center of a scene node in viewport coordinates.
fn node_center(node: &SceneNode) -> [f32; 2] {
    [
        (node.x + node.w * 0.5).to_f32().unwrap_or(0.0),
        (node.y + node.h * 0.5).to_f32().unwrap_or(0.0),
    ]
}

/// Whether a scene node's border box contains the point.
fn node_contains(node: &SceneNode, x: f64, y: f64) -> bool {
    x >= node.x && x < node.x + node.w && y >= node.y && y < node.y + node.h
}

/// Topmost visible labeled or keyed scene node containing `point`, formatted
/// as its label when present, otherwise its authored key.
fn node_under_point(
    frame: &Frame,
    scene_strs: &[String],
    node_key: &impl Fn(u32) -> String,
    point: [f32; 2],
) -> Option<String> {
    let x = f64::from(point[0]);
    let y = f64::from(point[1]);
    frame.scene.iter().rev().find_map(|node| {
        if !visible(node) || !node_contains(node, x, y) {
            return None;
        }
        if let Some(label) = scene_str(scene_strs, node.label) {
            return Some(label.to_owned());
        }
        let key = node_key(node.node);
        (!key.is_empty()).then_some(key)
    })
}

/// Resolves a UI selector against the solved frame.
///
/// Precedence: case-insensitive exact label match, label substring,
/// authored-key suffix, then visible text-run content substring (using the
/// run's owning scene node for geometry). Only nodes with positive extent
/// participate. Returns the target center and the matched label or key.
fn find_frame_target(
    frame: &Frame,
    scene_strs: &[String],
    node_key: &impl Fn(u32) -> String,
    selector: &str,
) -> Option<([f32; 2], String)> {
    let needle = selector.to_lowercase();
    let labeled = |exact: bool| {
        frame.scene.iter().rev().find_map(|node| {
            if !visible(node) {
                return None;
            }
            let label = scene_str(scene_strs, node.label)?;
            let lowered = label.to_lowercase();
            let matched = if exact {
                lowered == needle
            } else {
                lowered.contains(&needle)
            };
            matched.then(|| (node_center(node), label.to_owned()))
        })
    };
    if let Some(target) = labeled(true) {
        return Some(target);
    }
    if let Some(target) = labeled(false) {
        return Some(target);
    }
    if let Some(target) = frame.scene.iter().rev().find_map(|node| {
        if !visible(node) {
            return None;
        }
        let key = node_key(node.node);
        (!key.is_empty() && key.ends_with(selector)).then(|| (node_center(node), key))
    }) {
        return Some(target);
    }
    frame.ops.iter().rev().find_map(|op| {
        let FrameOp::Text(text) = op else {
            return None;
        };
        let content = frame.strings.get(usize::try_from(text.str_ref).ok()?)?;
        if !content.to_lowercase().contains(&needle) {
            return None;
        }
        let owner = frame
            .scene
            .iter()
            .rev()
            .find(|node| node.node == text.node && visible(node))?;
        let target =
            scene_str(scene_strs, owner.label).map_or_else(|| node_key(owner.node), str::to_owned);
        Some((node_center(owner), target))
    })
}

/// Builds the `Page.getSnapshot` payload from a solved frame: every scene
/// node carrying accessibility semantics plus every visible text run.
fn frame_snapshot(
    frame: &Frame,
    scene_strs: &[String],
    node_key: &impl Fn(u32) -> String,
) -> Value {
    let nodes = frame
        .scene
        .iter()
        .filter(|node| visible(node) && is_semantic(node))
        .map(|node| semantic_node_json(node, scene_strs, node_key))
        .collect::<Vec<_>>();
    let text = frame
        .ops
        .iter()
        .filter_map(|op| {
            let FrameOp::Text(text) = op else {
                return None;
            };
            let content = frame.strings.get(usize::try_from(text.str_ref).ok()?)?;
            if content.is_empty() {
                return None;
            }
            Some(json!({
                "text": content,
                "x": text.x,
                "y": text.y_baseline,
                "size": text.size,
            }))
        })
        .collect::<Vec<_>>();
    json!({
        "viewport": { "width": frame.width, "height": frame.height },
        "nodes": nodes,
        "text": text,
    })
}

/// Whether a scene node carries any accessibility semantics worth reporting.
fn is_semantic(node: &SceneNode) -> bool {
    node.role != 0
        || node.label != 0
        || node.desc != 0
        || node.checked != 0
        || node.expanded != 0
        || node.selected != 0
        || node.disabled
        || node.focused
}

/// Serializes one semantic scene node, omitting absent optional states.
fn semantic_node_json(
    node: &SceneNode,
    scene_strs: &[String],
    node_key: &impl Fn(u32) -> String,
) -> Value {
    let mut object = Map::new();
    object.insert("key".to_owned(), json!(node_key(node.node)));
    if let Some(role) = scene_str(scene_strs, node.role) {
        object.insert("role".to_owned(), json!(role));
    }
    if let Some(label) = scene_str(scene_strs, node.label) {
        object.insert("label".to_owned(), json!(label));
    }
    if let Some(desc) = scene_str(scene_strs, node.desc) {
        object.insert("desc".to_owned(), json!(desc));
    }
    object.insert(
        "rect".to_owned(),
        json!({ "x": node.x, "y": node.y, "width": node.w, "height": node.h }),
    );
    if let Some(checked) = tri_state(node.checked) {
        object.insert("checked".to_owned(), checked);
    }
    if let Some(expanded) = tri_state(node.expanded) {
        object.insert("expanded".to_owned(), expanded);
    }
    if let Some(selected) = tri_state(node.selected) {
        object.insert("selected".to_owned(), selected);
    }
    if node.disabled {
        object.insert("disabled".to_owned(), json!(true));
    }
    if node.focused {
        object.insert("focused".to_owned(), json!(true));
    }
    if let Some(value) = node.value_now {
        object.insert("valueNow".to_owned(), json!(value));
    }
    if let Some(value_text) = scene_str(scene_strs, node.value_text) {
        object.insert("valueText".to_owned(), json!(value_text));
    }
    Value::Object(object)
}

/// Maps an optional kernel state: `0` absent, `1` false, `2` true, `3` mixed.
fn tri_state(value: u32) -> Option<Value> {
    match value {
        1 => Some(json!(false)),
        2 => Some(json!(true)),
        3 => Some(json!("mixed")),
        _ => None,
    }
}

fn required_string<'a>(params: &'a Value, name: &str) -> Result<&'a str> {
    optional_string(params, name).ok_or_else(|| anyhow!("missing string parameter `{name}`"))
}

fn optional_string<'a>(params: &'a Value, name: &str) -> Option<&'a str> {
    params.get(name).and_then(Value::as_str)
}

fn required_f32(params: &Value, name: &str) -> Result<f32> {
    optional_f32(params, name).ok_or_else(|| anyhow!("missing numeric parameter `{name}`"))
}

fn optional_f32(params: &Value, name: &str) -> Option<f32> {
    params
        .get(name)
        .and_then(Value::as_f64)
        .and_then(|value| value.to_f32())
}

fn required_u32(params: &Value, name: &str) -> Result<u32> {
    params
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| anyhow!("missing unsigned integer parameter `{name}`"))
}

fn optional_u64(params: &Value, name: &str) -> Option<u64> {
    params.get(name).and_then(Value::as_u64)
}

fn optional_bool(params: &Value, name: &str) -> Option<bool> {
    params.get(name).and_then(Value::as_bool)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        git::models::{
            BranchInfo, ChangeKind, CommitSummary, RepoSnapshot, WorkingFile, WorkingTree,
        },
        ui::slab::{SlabDocument, generated},
    };

    fn workspace_state() -> AppState {
        let mut state = AppState::new(None, 1_600, 1_000, None);
        let path = PathBuf::from("/tmp/kraken-automation-test");
        state.settings.sidebar_collapsed = false;
        state.tabs[0].title = "kraken-automation-test".to_owned();
        state.tabs[0].path = Some(path.clone());
        state.snapshot = Some(RepoSnapshot {
            path,
            name: "kraken-automation-test".to_owned(),
            head: "main".to_owned(),
            head_id: Some("deadbeef".to_owned()),
            branches: vec![BranchInfo {
                name: "main".to_owned(),
                target: "deadbeef".to_owned(),
                current: true,
                remote: false,
                upstream: None,
            }],
            tags: Vec::new(),
            stashes: Vec::new(),
            worktrees: Vec::new(),
            commits: vec![CommitSummary {
                id: "deadbeef".to_owned(),
                short_id: "deadbee".to_owned(),
                subject: "Initial commit".to_owned(),
                description: String::new(),
                author: "Kraken".to_owned(),
                email: "kraken@example.com".to_owned(),
                authored_seconds: 0,
                parents: Vec::new(),
                is_local: false,
                refs: Vec::new(),
                branch_refs: Vec::new(),
            }],
            working: WorkingTree {
                files: vec![WorkingFile {
                    path: PathBuf::from("src/main.rs"),
                    old_path: None,
                    staged: None,
                    unstaged: Some(ChangeKind::Modified),
                }],
            },
            loaded_limit: 200,
            has_more: false,
            refs_sig: 0,
        });
        state
    }

    /// Solves one frame from a fresh document so finder and snapshot code
    /// can be exercised without a GPU.
    fn solved_frame(state: &AppState) -> (SlabDocument, Frame) {
        let mut document = SlabDocument::new(generated::Doc::new());
        let frame = document.frame(state);
        (document, frame)
    }

    #[test]
    fn selector_resolves_a_labeled_control() {
        let state = workspace_state();
        let (document, frame) = solved_frame(&state);
        let inst = &document.doc.inst;
        let key_of = |node: u32| slab_kernel::scene::key_of(&inst.doc, &inst.st.lists, node);

        let (point, target) =
            find_frame_target(&frame, &inst.st.scene_strs, &key_of, "preferences")
                .expect("selector resolves the Preferences button");
        assert_eq!(target, "Preferences");
        assert!(point[0] > 0.0 && point[1] > 0.0);

        let under = node_under_point(&frame, &inst.st.scene_strs, &key_of, point)
            .expect("the resolved center hits a semantic node");
        assert!(!under.is_empty());
    }

    #[test]
    fn selector_resolves_an_authored_key_suffix() {
        let state = workspace_state();
        let (document, frame) = solved_frame(&state);
        let inst = &document.doc.inst;
        let key_of = |node: u32| slab_kernel::scene::key_of(&inst.doc, &inst.st.lists, node);

        // The graph divider has no label; the finder must fall through the
        // label stages to the authored-key suffix match.
        let (point, target) =
            find_frame_target(&frame, &inst.st.scene_strs, &key_of, "#graph-ref-divider")
                .expect("selector resolves the keyed graph divider");
        assert!(target.ends_with("#graph-ref-divider"));
        assert!(point[0] > 0.0 && point[1] > 0.0);
    }

    #[test]
    fn snapshot_reports_labeled_nodes_and_text_runs() {
        let state = workspace_state();
        let (document, frame) = solved_frame(&state);
        let inst = &document.doc.inst;
        let key_of = |node: u32| slab_kernel::scene::key_of(&inst.doc, &inst.st.lists, node);

        let snapshot = frame_snapshot(&frame, &inst.st.scene_strs, &key_of);

        assert_eq!(snapshot["viewport"]["width"], json!(frame.width));
        let nodes = snapshot["nodes"].as_array().expect("snapshot nodes array");
        assert!(
            nodes
                .iter()
                .any(|node| node["label"] == json!("Preferences")),
            "snapshot lists the labeled Preferences control"
        );
        assert!(
            nodes.iter().all(|node| node["rect"]["width"]
                .as_f64()
                .is_some_and(|width| width > 0.0)),
            "semantic nodes carry positive extents"
        );
        let text = snapshot["text"].as_array().expect("snapshot text array");
        assert!(
            text.iter().any(|run| run["text"]
                .as_str()
                .is_some_and(|content| content.contains("Initial commit"))),
            "snapshot lists the painted commit subject"
        );
    }
}
