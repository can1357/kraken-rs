use std::{
    io::{Read, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
};

use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use vte::Parser;
use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;

use super::{
    Cell, TerminalSnapshot,
    grid::{Grid, TerminalModes},
};

/// Live shell process plus a VT-compatible screen model shared with rendering.
pub(crate) struct Terminal {
    grid: Arc<Mutex<Grid>>,
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    exited: Arc<AtomicBool>,
    revision: Arc<AtomicU64>,
}

impl Terminal {
    /// Starts a shell whose output wakes the native event loop.
    pub(crate) fn spawn(
        cwd: &Path,
        cols: usize,
        rows: usize,
        event_loop_proxy: Option<EventLoopProxy<UserEvent>>,
    ) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: rows.max(1) as u16,
                cols: cols.max(1) as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("open terminal pty")?;
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_owned());
        let mut command = CommandBuilder::new(shell);
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        let child = pair
            .slave
            .spawn_command(command)
            .context("spawn terminal shell")?;
        drop(pair.slave);
        let reader = pair
            .master
            .try_clone_reader()
            .context("clone terminal pty reader")?;
        let writer = pair
            .master
            .take_writer()
            .context("take terminal pty writer")?;
        let grid = Arc::new(Mutex::new(Grid::new(cols, rows)));
        let reader_grid = Arc::clone(&grid);
        let exited = Arc::new(AtomicBool::new(false));
        let reader_exited = Arc::clone(&exited);
        let revision = Arc::new(AtomicU64::new(1));
        let reader_revision = Arc::clone(&revision);
        thread::spawn(move || {
            let mut reader = reader;
            let mut parser = Parser::new();
            let mut buffer = [0_u8; 4096];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                let updated = if let Ok(mut grid) = reader_grid.lock() {
                    parser.advance(&mut *grid, &buffer[..read]);
                    true
                } else {
                    false
                };
                if updated {
                    reader_revision.fetch_add(1, Ordering::Release);
                }
                if updated && let Some(proxy) = &event_loop_proxy {
                    let _ = proxy.send_event(UserEvent::Terminal);
                }
            }
            reader_exited.store(true, Ordering::Release);
            reader_revision.fetch_add(1, Ordering::Release);
            if let Some(proxy) = &event_loop_proxy {
                let _ = proxy.send_event(UserEvent::Terminal);
            }
        });
        Ok(Self {
            grid,
            writer: Mutex::new(writer),
            master: Mutex::new(pair.master),
            child: Mutex::new(child),
            exited,
            revision,
        })
    }

    pub(crate) fn write(&self, bytes: &[u8]) {
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writer.write_all(bytes);
            let _ = writer.flush();
        }
    }

    pub(crate) fn resize(&self, cols: usize, rows: usize) {
        self.resize_viewport(cols, rows, 0, 0);
    }

    pub(crate) fn resize_viewport(
        &self,
        cols: usize,
        rows: usize,
        pixel_width: usize,
        pixel_height: usize,
    ) {
        let changed = self
            .grid
            .lock()
            .is_ok_and(|mut grid| grid.resize(cols, rows));
        if let Ok(master) = self.master.lock() {
            let _ = master.resize(PtySize {
                rows: pty_dimension(rows),
                cols: pty_dimension(cols),
                pixel_width: pty_dimension_or_zero(pixel_width),
                pixel_height: pty_dimension_or_zero(pixel_height),
            });
        }
        if changed {
            self.revision.fetch_add(1, Ordering::Release);
        }
    }

    pub(crate) fn snapshot(&self) -> TerminalSnapshot {
        let exited = self.exited.load(Ordering::Acquire);
        self.grid.lock().map_or_else(
            |_| TerminalSnapshot {
                cols: 1,
                rows: 1,
                cells: vec![Cell::default()],
                cursor_col: 0,
                cursor_row: 0,
                cursor_visible: false,
                exited,
                viewport_top: 0,
            },
            |grid| grid.snapshot(exited),
        )
    }

    pub(crate) fn scroll(&self, delta: i32) {
        if self.grid.lock().is_ok_and(|mut grid| grid.scroll(delta)) {
            self.revision.fetch_add(1, Ordering::Release);
        }
    }

    pub(crate) fn modes(&self) -> TerminalModes {
        self.grid
            .lock()
            .map_or_else(|_| TerminalModes::default(), |grid| grid.modes())
    }

    pub(crate) fn selection_text(&self, start: (usize, usize), end: (usize, usize)) -> String {
        self.grid
            .lock()
            .map_or_else(|_| String::new(), |grid| grid.selection_text(start, end))
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
}

fn pty_dimension(value: usize) -> u16 {
    u16::try_from(value.max(1)).unwrap_or(u16::MAX)
}

fn pty_dimension_or_zero(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }
}
