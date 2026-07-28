//! Headless Slab Drive Protocol mount for the application-owned document.
//!
//! The listener only forwards NDJSON; [`slab_drive::RequestPump`] remains the
//! protocol implementation and drives the same retained Slab instance that
//! the application projects into and renders.

use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use num_traits::ToPrimitive;
use serde_json::Value;

use crate::{app::state::AppState, gpu::offscreen::OffscreenRenderer};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const QUIT_FLUSH_TIMEOUT: Duration = Duration::from_secs(1);

/// Runs a loopback Slab Drive Protocol session without creating a native window.
pub(crate) fn run(repo: Option<PathBuf>, width: u32, height: u32, port: u16) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("bind SDP listener on port {port}"))?;
    let address = listener.local_addr().context("read SDP listener address")?;
    let (request_sender, request_receiver) = mpsc::channel();
    let _listener = spawn_listener(listener, request_sender)?;
    let mut state = AppState::new(repo, width, height, None);
    let mut renderer = pollster::block_on(OffscreenRenderer::new())?;

    eprintln!("sdp: listening on {address}");

    loop {
        state.process_events();
        let request = match request_receiver.recv_timeout(EVENT_POLL_INTERVAL) {
            Ok(request) => request,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        };
        let result = renderer.drive_request(&mut state, &request.line);
        sync_viewport(&mut state, &request.line, &result.response);
        let quit = result.quit;
        let (acknowledged, written) = mpsc::channel();
        let delivered = request
            .reply
            .send(DriveResponse {
                json: result.response.to_string(),
                acknowledged,
            })
            .is_ok();
        if quit {
            if delivered {
                let _ = written.recv_timeout(QUIT_FLUSH_TIMEOUT);
            }
            return Ok(());
        }
    }
}

struct DriveRequest {
    line: String,
    reply: Sender<DriveResponse>,
}

struct DriveResponse {
    json: String,
    acknowledged: Sender<()>,
}

/// Starts the acceptor that enforces SDP's one-client session rule.
fn spawn_listener(
    listener: TcpListener,
    requests: Sender<DriveRequest>,
) -> Result<thread::JoinHandle<()>> {
    let active = Arc::new(AtomicBool::new(false));
    thread::Builder::new()
        .name("kraken-sdp-listener".to_owned())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                if active
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    slab_drive::reject_busy(stream);
                    continue;
                }
                let connection_active = Arc::clone(&active);
                let spawn_active = Arc::clone(&active);
                let requests = requests.clone();
                if thread::Builder::new()
                    .name("kraken-sdp-client".to_owned())
                    .spawn(move || {
                        serve_connection(stream, requests);
                        connection_active.store(false, Ordering::Release);
                    })
                    .is_err()
                {
                    spawn_active.store(false, Ordering::Release);
                }
            }
        })
        .context("spawn SDP listener")
}

/// Sends one connection's ordered requests to the state-owning thread.
fn serve_connection(stream: TcpStream, requests: Sender<DriveRequest>) {
    let _ = stream.set_nodelay(true);
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut write_half = stream;
    for line in BufReader::new(read_half).lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let (reply, responses) = mpsc::channel();
        if requests.send(DriveRequest { line, reply }).is_err() {
            break;
        }
        let Ok(response) = responses.recv() else {
            break;
        };
        if write_half
            .write_all(response.json.as_bytes())
            .and_then(|()| write_half.write_all(b"\n"))
            .and_then(|()| write_half.flush())
            .is_err()
        {
            break;
        }
        let _ = response.acknowledged.send(());
    }
}

/// Mirrors an accepted SDP viewport onto the application's host-owned layout.
fn sync_viewport(state: &mut AppState, line: &str, response: &Value) {
    let Ok(request) = serde_json::from_str::<Value>(line) else {
        return;
    };
    if request.get("method").and_then(Value::as_str) != Some("env.set") {
        return;
    }
    let Some(result) = response.get("result") else {
        return;
    };
    let (Some(width), Some(height)) = (
        result.get("width").and_then(Value::as_f64),
        result.get("height").and_then(Value::as_f64),
    ) else {
        return;
    };
    let (Some(width), Some(height)) = (width.round().to_u32(), height.round().to_u32()) else {
        return;
    };
    state.resize(width, height);
}
