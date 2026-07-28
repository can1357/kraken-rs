//! Off-thread syntax highlighting for diff rows.
//!
//! Highlighting one line runs syntect's full regex set against the pure-Rust
//! `fancy-regex` backend, costing on the order of a millisecond. Doing that
//! for every row scrolled into a diff window pinned the event loop (a 40k-row
//! diff spent ~65% of the main thread here), so the work moved to worker
//! threads, following the avatar pattern in [`crate::graph::avatars`].
//!
//! A projection looks up each line, paints plain text for the misses, and
//! queues them as one batch. The batch publishes a single version bump and a
//! single wake, so a scrolled window costs one repaint rather than one per
//! line.
//!
//! Only the newest batch is kept: during a fast scroll the window the user is
//! actually looking at must not queue behind the windows they flew past, so
//! [`queue`] discards any superseded batch and releases its lines for a later
//! request.
//!
//! ```ignore
//! let mut pending = Vec::new();
//! let runs = match highlight::spans(revision, row, slot) {
//!     Some(spans) => paint(spans),
//!     None => {
//!         pending.push((row, slot, text.to_owned()));
//!         paint_plain(text)
//!     }
//! };
//! highlight::queue(revision, path, pending);
//! ```

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Condvar, LazyLock, Mutex},
    thread,
};

use syntect::{easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet};
use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;

/// Highlighted lines retained across scrolling. Each entry is one line of one
/// pane; the cap bounds memory on documents far larger than any window.
const CACHE_CAP: usize = 8_192;

/// One highlighted run of a diff line.
#[derive(Clone)]
pub(crate) struct Span {
    pub(crate) content: String,
    /// Packed SLIR RGBA.
    pub(crate) tone: u32,
}

/// One line awaiting highlighting: row index, pane slot, text.
pub(crate) type PendingLine = (usize, u8, String);

/// Identifies one highlighted line: diff revision, row index, pane slot.
type Key = (u64, usize, u8);

#[derive(Default)]
struct Store {
    spans: HashMap<Key, Vec<Span>>,
    /// Insertion order backing the FIFO eviction that enforces [`CACHE_CAP`].
    order: VecDeque<Key>,
    /// In-flight keys, so a row queued while scrolling is requested once.
    queued: HashSet<Key>,
    /// Revision the cache holds; a newer one drops every entry.
    revision: Option<u64>,
    /// Bumped once per completed batch so projections know to rebuild.
    version: u64,
}

struct Request {
    revision: u64,
    path: PathBuf,
    lines: Vec<PendingLine>,
}

static STORE: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(Store::default()));
/// Single-slot mailbox holding only the newest batch, plus its wakeup.
///
/// Lock order is always `STORE` then `WORK`; no path holds `WORK` while
/// taking `STORE`.
static WORK: LazyLock<(Mutex<Option<Request>>, Condvar)> =
    LazyLock::new(|| (Mutex::new(None), Condvar::new()));
static WORKERS: LazyLock<()> = LazyLock::new(start_workers);
static EVENT_LOOP_PROXY: LazyLock<Mutex<Option<EventLoopProxy<UserEvent>>>> =
    LazyLock::new(|| Mutex::new(None));

/// Connects highlight completions to the native event loop.
pub(crate) fn set_event_loop_proxy(proxy: EventLoopProxy<UserEvent>) {
    *EVENT_LOOP_PROXY.lock().expect("highlight proxy lock") = Some(proxy);
}

/// Revision of the highlight cache; changes when a batch lands.
///
/// Projections fold this into their cache key so completed highlights rebuild
/// the window that was showing those lines as plain text.
pub(crate) fn version() -> u64 {
    STORE.lock().expect("highlight store lock").version
}

/// Returns cached spans for one line, or `None` when it is not highlighted
/// yet. Pure lookup: pass the misses to [`queue`].
///
/// Requesting a newer revision drops every entry from the previous one.
pub(crate) fn spans(revision: u64, row: usize, slot: u8) -> Option<Vec<Span>> {
    let mut store = STORE.lock().expect("highlight store lock");
    if store.revision != Some(revision) {
        store.revision = Some(revision);
        store.spans.clear();
        store.order.clear();
        store.queued.clear();
        return None;
    }
    store.spans.get(&(revision, row, slot)).cloned()
}

/// Publishes one batch of unhighlighted lines as the newest work.
///
/// Lines already in flight are skipped. Any batch still waiting is dropped
/// and its lines released, so the visible window never queues behind windows
/// scrolled past. The batch completes as a single version bump and wake.
pub(crate) fn queue(revision: u64, path: &Path, lines: Vec<PendingLine>) {
    if lines.is_empty() {
        return;
    }
    let fresh = {
        let mut store = STORE.lock().expect("highlight store lock");
        if store.revision != Some(revision) {
            return;
        }
        lines
            .into_iter()
            .filter(|&(row, slot, _)| store.queued.insert((revision, row, slot)))
            .collect::<Vec<_>>()
    };
    if fresh.is_empty() {
        return;
    }
    LazyLock::force(&WORKERS);
    let (mailbox, ready) = &*WORK;
    let superseded = {
        let mut slot = mailbox.lock().expect("highlight mailbox lock");
        slot.replace(Request {
            revision,
            path: path.to_path_buf(),
            lines: fresh,
        })
    };
    ready.notify_one();
    if let Some(stale) = superseded {
        release(&stale);
    }
}

/// Returns superseded or abandoned lines to the unqueued state so a later
/// window can request them again.
fn release(request: &Request) {
    let mut store = STORE.lock().expect("highlight store lock");
    for &(row, slot, _) in &request.lines {
        store.queued.remove(&(request.revision, row, slot));
    }
}

fn start_workers() {
    for index in 0..2 {
        thread::Builder::new()
            .name(format!("diff-highlight-{index}"))
            .spawn(move || {
                loop {
                    let (mailbox, ready) = &*WORK;
                    let request = {
                        let mut slot = mailbox.lock().expect("highlight mailbox lock");
                        while slot.is_none() {
                            slot = ready.wait(slot).expect("highlight mailbox wait");
                        }
                        slot.take()
                    };
                    let Some(request) = request else { continue };
                    highlight(request);
                }
            })
            .expect("start diff highlight worker");
    }
}

fn highlight(request: Request) {
    let highlighted = request
        .lines
        .iter()
        .map(|(row, slot, text)| ((*row, *slot), compute(&request.path, text)))
        .collect::<Vec<_>>();

    let mut store = STORE.lock().expect("highlight store lock");
    // A revision that advanced while the batch was in flight already cleared
    // the cache; dropping the result keeps the store single-revision.
    if store.revision != Some(request.revision) {
        return;
    }
    for ((row, slot), spans) in highlighted {
        let key = (request.revision, row, slot);
        store.queued.remove(&key);
        if store.spans.insert(key, spans).is_none() {
            store.order.push_back(key);
        }
    }
    while store.order.len() > CACHE_CAP {
        let Some(evicted) = store.order.pop_front() else {
            break;
        };
        store.spans.remove(&evicted);
    }
    store.version = store.version.wrapping_add(1);
    drop(store);
    if let Some(proxy) = EVENT_LOOP_PROXY
        .lock()
        .expect("highlight proxy lock")
        .as_ref()
    {
        let _ = proxy.send_event(UserEvent::Highlight);
    }
}

fn compute(path: &Path, line: &str) -> Vec<Span> {
    static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
    static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("txt");
    let syntax = SYNTAXES
        .find_syntax_by_extension(extension)
        .unwrap_or_else(|| SYNTAXES.find_syntax_plain_text());
    let Some(theme) = THEMES
        .themes
        .get("base16-ocean.dark")
        .or_else(|| THEMES.themes.values().next())
    else {
        return Vec::new();
    };
    let mut highlighter = HighlightLines::new(syntax, theme);
    highlighter.highlight_line(line, &SYNTAXES).map_or_else(
        |_| Vec::new(),
        |ranges| {
            ranges
                .into_iter()
                .map(|(style, token)| Span {
                    content: token.to_owned(),
                    tone: u32::from_le_bytes([
                        style.foreground.r,
                        style.foreground.g,
                        style.foreground.b,
                        style.foreground.a,
                    ]),
                })
                .collect()
        },
    )
}
