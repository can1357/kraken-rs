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
use serde_json::{Value, json};

use crate::{
    app::state::{AppState, EditKey, EditKeyKind, FocusField},
    gpu::offscreen::OffscreenRenderer,
    ui::{Rect, Scene, Theme, action::UiAction},
    views,
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
        theme: Theme::dark(),
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
    theme: Theme,
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
                let scene = self.refresh();
                Ok(scene_snapshot(&scene))
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

    fn refresh(&mut self) -> Scene {
        self.state.process_events();
        let scene = views::build_scene(&self.state, &self.theme);
        self.state.adopt_scene(&scene);
        scene
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
        let scene = self.refresh();
        self.renderer
            .render_png(&scene, self.theme.window, &output)?;
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

    fn dispatch_mouse(&mut self, params: &Value) -> Result<Value> {
        let event_type = required_string(params, "type")?;
        let x = optional_f32(params, "x").unwrap_or(self.state.mouse[0]);
        let y = optional_f32(params, "y").unwrap_or(self.state.mouse[1]);
        self.state.mouse = [x, y];
        // Rebuild the scene with the pointer already at the event position so
        // position-derived hit payloads match, exactly like the winit path
        // (CursorMoved renders before the press arrives).
        self.refresh();
        let target = hit_at(&self.state, [x, y]);

        match event_type {
            "mouseMoved" => {
                if self.state.is_dragging() {
                    self.state.drag_to(x, y);
                }
            }
            "mousePressed" | "mouseClicked" => {
                if optional_string(params, "button").unwrap_or("left") == "right" {
                    self.state.right_click();
                } else {
                    self.state.click();
                    if event_type == "mouseClicked" {
                        self.state.end_drag();
                    }
                }
            }
            "mouseReleased" => self.state.end_drag(),
            "mouseWheel" => {
                let delta_y = optional_f32(params, "deltaY").unwrap_or(0.0);
                self.state.scroll(delta_y);
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
        let text = required_string(params, "text")?;
        // Mirror the winit IME path: a focused terminal owns typed text.
        if self.state.terminal_accepts_input() {
            self.state
                .terminal_input(text.replace('\n', "\r").as_bytes());
        } else {
            self.state.insert_text(text);
        }
        self.refresh();
        Ok(json!({ "inserted": text }))
    }

    fn dispatch_key(&mut self, params: &Value) -> Result<Value> {
        if optional_string(params, "type").is_some_and(|kind| kind == "keyUp") {
            return Ok(json!({ "ignored": "keyUp" }));
        }
        let raw_key = required_string(params, "key")?;
        let mut command = optional_bool(params, "command").unwrap_or(false);
        let mut shift = optional_bool(params, "shift").unwrap_or(false);
        let mut alt = optional_bool(params, "alt").unwrap_or(false);
        let mut control = optional_bool(params, "control").unwrap_or(false);
        // Accept "Alt+ArrowLeft"-style modifier prefixes so QA drivers can
        // express chords the transport schema has no flags for.
        let mut key = raw_key;
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
        // Terminal owns non-command keys, mirroring the winit routing.
        if self.state.terminal_accepts_input() && !command {
            let bytes: Option<&[u8]> = match key {
                "Backspace" => Some(b"\x7f"),
                "Enter" => Some(b"\r"),
                "Tab" => Some(b"\t"),
                "Escape" => Some(b"\x1b"),
                "ArrowUp" => Some(b"\x1b[A"),
                "ArrowDown" => Some(b"\x1b[B"),
                "ArrowRight" => Some(b"\x1b[C"),
                "ArrowLeft" => Some(b"\x1b[D"),
                _ => None,
            };
            if let Some(bytes) = bytes {
                self.state.terminal_input(bytes);
            } else if key.chars().count() == 1 {
                self.state.terminal_input(key.as_bytes());
            } else if let Some(text) = optional_string(params, "text") {
                self.state.terminal_input(text.as_bytes());
            }
            self.refresh();
            return Ok(json!({ "key": key, "routed": "terminal" }));
        }
        let edit_kind = match key {
            "ArrowLeft" => Some(EditKeyKind::Left),
            "ArrowRight" => Some(EditKeyKind::Right),
            "ArrowUp" => Some(EditKeyKind::Up),
            "ArrowDown" => Some(EditKeyKind::Down),
            "Home" => Some(EditKeyKind::Home),
            "End" => Some(EditKeyKind::End),
            "Backspace" => Some(EditKeyKind::Backspace),
            "Delete" => Some(EditKeyKind::Delete),
            _ if primary && key.chars().count() == 1 => key.chars().next().map(EditKeyKind::Char),
            _ => None,
        };
        if let Some(kind) = edit_kind
            && self.state.edit_key(EditKey {
                kind,
                shift,
                alt,
                command: primary,
            })
        {
            self.refresh();
            return Ok(json!({ "key": raw_key, "routed": "text" }));
        }
        match key {
            "F1" => {
                self.state.dispatch(
                    if self.state.main_view == crate::app::state::MainView::Diff {
                        UiAction::ToggleEditorPalette
                    } else {
                        UiAction::ToggleCommandPalette
                    },
                );
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
                self.state.dispatch(
                    if self.state.main_view == crate::app::state::MainView::Diff {
                        UiAction::ToggleEditorPalette
                    } else {
                        UiAction::ToggleCommandPalette
                    },
                );
            }
            key if command
                && key.eq_ignore_ascii_case("c")
                && self.state.main_view == crate::app::state::MainView::Diff =>
            {
                self.state.dispatch(UiAction::CopyDiffText);
            }
            key if command && key.eq_ignore_ascii_case("f") => {
                self.state.dispatch(
                    if self.state.main_view == crate::app::state::MainView::Diff {
                        UiAction::ToggleDiffSearch
                    } else {
                        UiAction::ToggleSearch
                    },
                );
            }
            "," if command => self.state.dispatch(UiAction::OpenPreferences),
            key if command && shift && key.eq_ignore_ascii_case("a") => {
                self.state.dispatch(UiAction::ToggleTabSwitcher);
            }
            _ => {
                if let Some(text) = optional_string(params, "text") {
                    self.state.insert_text(text);
                } else if !command && key.chars().count() == 1 {
                    self.state.insert_text(key);
                }
            }
        }
        self.refresh();
        Ok(json!({ "key": key }))
    }

    fn click(&mut self, params: &Value) -> Result<Value> {
        let scene = self.refresh();
        let (point, target) = if let Some(selector) = optional_string(params, "selector") {
            let (point, target) = find_target(&scene, selector)
                .ok_or_else(|| anyhow!("no visible UI target matches `{selector}`"))?;
            (point, Some(target))
        } else {
            let point = [required_f32(params, "x")?, required_f32(params, "y")?];
            let target = hit_at(&self.state, point);
            (point, target)
        };
        self.state.mouse = point;
        self.state.click();
        // A synthetic click is press + release; releasing settles any deferred
        // ref click that the press turned into a potential drag.
        self.state.end_drag();
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
            "hitTargetCount": self.state.hits.len(),
        })
    }
}

fn scene_snapshot(scene: &Scene) -> Value {
    let text = scene
        .layers
        .iter()
        .flat_map(|layer| layer.text.iter())
        .map(|item| {
            json!({
                "text": item.text,
                "origin": { "x": item.origin[0], "y": item.origin[1] },
                "bounds": rect_value(item.bounds),
                "size": item.size,
                "lineHeight": item.line_height,
                "font": format!("{:?}", item.face),
            })
        })
        .collect::<Vec<_>>();
    let hits = scene
        .hits
        .iter()
        .map(|hit| {
            json!({
                "rect": rect_value(hit.rect),
                "action": format!("{:?}", hit.action),
                "cursor": format!("{:?}", hit.cursor),
                "tooltip": hit.tooltip,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "viewport": { "width": scene.width, "height": scene.height },
        "text": text,
        "hits": hits,
    })
}

fn find_target(scene: &Scene, selector: &str) -> Option<([f32; 2], String)> {
    let selector = selector.to_lowercase();
    let direct = scene.hits.iter().rev().find(|hit| {
        let action = format!("{:?}", hit.action).to_lowercase();
        let tooltip = hit.tooltip.as_deref().unwrap_or_default().to_lowercase();
        action == selector
            || tooltip == selector
            || action.contains(&selector)
            || tooltip.contains(&selector)
    });
    if let Some(hit) = direct {
        return Some((center(hit.rect), format!("{:?}", hit.action)));
    }

    scene
        .layers
        .iter()
        .flat_map(|layer| layer.text.iter())
        .find_map(|text| {
            text.text
                .to_lowercase()
                .contains(&selector)
                .then(|| {
                    scene
                        .hits
                        .iter()
                        .rev()
                        .filter(|hit| !matches!(hit.action, UiAction::DismissOverlay))
                        .filter(|hit| {
                            hit.rect.intersection(text.bounds).is_some()
                                || hit.rect.contains(text.origin)
                        })
                        .min_by(|left, right| {
                            distance_squared(center(left.rect), text.origin)
                                .total_cmp(&distance_squared(center(right.rect), text.origin))
                        })
                })
                .flatten()
                .map(|hit| (center(hit.rect), format!("{:?}", hit.action)))
        })
}

fn hit_at(state: &AppState, point: [f32; 2]) -> Option<String> {
    state
        .hits
        .iter()
        .rev()
        .find(|hit| hit.rect.contains(point))
        .map(|hit| format!("{:?}", hit.action))
}

fn center(rect: Rect) -> [f32; 2] {
    [rect.x + rect.width * 0.5, rect.y + rect.height * 0.5]
}

fn distance_squared(left: [f32; 2], right: [f32; 2]) -> f32 {
    let x = left[0] - right[0];
    let y = left[1] - right[1];
    x.mul_add(x, y * y)
}

fn rect_value(rect: Rect) -> Value {
    json!({
        "x": rect.x,
        "y": rect.y,
        "width": rect.width,
        "height": rect.height,
    })
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
    use super::*;
    use crate::ui::{FontFace, action::CursorHint};

    fn assert_point(actual: [f32; 2], expected: [f32; 2]) {
        assert!((actual[0] - expected[0]).abs() < f32::EPSILON);
        assert!((actual[1] - expected[1]).abs() < f32::EPSILON);
    }

    #[test]
    fn semantic_target_prefers_the_topmost_action_match() {
        let mut scene = Scene::new(800, 600);
        scene.hit(
            scene.viewport(),
            UiAction::DismissOverlay,
            CursorHint::Default,
            None,
        );
        let search = Rect::new(680.0, 20.0, 90.0, 30.0);
        scene.hit(
            search,
            UiAction::ToggleSearch,
            CursorHint::Pointer,
            Some("Search"),
        );

        let (point, action) = find_target(&scene, "ToggleSearch").expect("search target");

        assert_point(point, [725.0, 35.0]);
        assert_eq!(action, "ToggleSearch");
    }

    #[test]
    fn semantic_target_maps_visible_text_to_its_control() {
        let mut scene = Scene::new(800, 600);
        scene.hit(
            scene.viewport(),
            UiAction::DismissOverlay,
            CursorHint::Default,
            None,
        );
        let control = Rect::new(100.0, 80.0, 120.0, 32.0);
        scene.hit(
            control,
            UiAction::ToggleActionsMenu,
            CursorHint::Pointer,
            None,
        );
        let theme = Theme::dark();
        scene.text(
            "General",
            [112.0, 100.0],
            control,
            theme.text,
            12.0,
            16.0,
            FontFace::Sans,
        );

        let (point, action) = find_target(&scene, "general").expect("text target");

        assert_point(point, [160.0, 96.0]);
        assert_eq!(action, "ToggleActionsMenu");
    }
}
