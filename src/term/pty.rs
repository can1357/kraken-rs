use std::{
    io::{Read, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use vte::Parser;
use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;

use super::{TerminalSnapshot, grid::Grid};

/// Live shell process plus a VT-compatible screen model shared with rendering.
pub(crate) struct Terminal {
    grid: Arc<Mutex<Grid>>,
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    exited: Arc<AtomicBool>,
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
                if updated && let Some(proxy) = &event_loop_proxy {
                    let _ = proxy.send_event(UserEvent::Terminal);
                }
            }
            reader_exited.store(true, Ordering::Release);
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
        })
    }

    pub(crate) fn write(&self, bytes: &[u8]) {
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writer.write_all(bytes);
            let _ = writer.flush();
        }
    }

    pub(crate) fn resize(&self, cols: usize, rows: usize) {
        if let Ok(mut grid) = self.grid.lock() {
            grid.resize(cols, rows);
        }
        if let Ok(master) = self.master.lock() {
            let _ = master.resize(PtySize {
                rows: rows.max(1) as u16,
                cols: cols.max(1) as u16,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }

    pub(crate) fn snapshot(&self) -> TerminalSnapshot {
        let exited = self.exited.load(Ordering::Acquire);
        self.grid.lock().map_or_else(
            |_| TerminalSnapshot {
                cols: 1,
                rows: 1,
                cells: Vec::new(),
                cursor_col: 0,
                cursor_row: 0,
                cursor_visible: false,
                exited,
            },
            |grid| grid.snapshot(exited),
        )
    }

    pub(crate) fn scroll(&self, delta: i32) {
        if let Ok(mut grid) = self.grid.lock() {
            grid.scroll(delta);
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }
}
