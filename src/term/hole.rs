use std::sync::Arc;

use slab_kernel::{
    dispatch::{
        self, E_BLUR, E_COMPOSITION_END, E_COMPOSITION_START, E_COMPOSITION_UPDATE, E_COPY, E_CUT,
        E_KEY_DOWN, E_PASTE, E_POINTER_DOWN, E_POINTER_MOVE, E_POINTER_UP, E_RESIZE, E_TEXT,
        E_WHEEL, Effects, Event, M_ALT, M_CTRL, M_META, M_SHIFT,
    },
    flatten::{Frame, FrameOp, OpClip, OpRect, OpText, frame_new},
    frame::{self as kframe, Instance},
};
use slab_native::{RegisteredFont, holes::HoleContent};

use super::{
    Cell, Terminal, TerminalColor, TerminalSnapshot,
    grid::{MouseTracking, TerminalModes},
};

const FONT_FAMILY: &str = "JetBrainsMono Nerd Font Mono";
const DEFAULT_FONT_SIZE: f64 = 12.0;
const DEFAULT_COLS: usize = 80;
const DEFAULT_ROWS: usize = 24;
const BACKGROUND: u32 = rgba(5, 5, 5, 255);
const FOREGROUND: u32 = rgba(229, 229, 229, 255);
const MUTED: u32 = rgba(163, 163, 163, 255);
const SELECTION: u32 = rgba(70, 70, 70, 255);
const CURSOR: u32 = rgba(237, 237, 237, 255);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SelectionPoint {
    line: usize,
    column: usize,
}

#[derive(Clone, Copy, Debug)]
struct Selection {
    anchor: SelectionPoint,
    focus: SelectionPoint,
    dragged: bool,
}

/// Native terminal content mounted into Slab's `hole terminal` viewport.
///
/// Slab owns panel and splitter geometry. This object owns the PTY-backed cell
/// grid, glyph resources, selection, cursor, and every pixel inside the hole.
pub(crate) struct TerminalHole {
    inst: Instance,
    fonts: Vec<RegisteredFont>,
    regular_font: i32,
    bold_font: i32,
    terminal: Option<Arc<Terminal>>,
    snapshot: Option<TerminalSnapshot>,
    viewport: (f64, f64),
    font_size: f64,
    focused: bool,
    dirty: bool,
    painted_revision: u64,
    selection: Option<Selection>,
    selecting: bool,
    pressed_button: Option<u32>,
    pending_copy: Option<String>,
    composition: Option<String>,
    alt_prefix_pending: bool,
}

impl TerminalHole {
    pub(crate) fn new() -> Self {
        let mut inst = kframe::inst_shell();
        inst.doc.ok = true;
        kframe::inst_init(&mut inst);
        let (regular_font, regular) = register_font(&mut inst, 400);
        let (bold_font, bold) = register_font(&mut inst, 700);
        Self {
            inst,
            fonts: vec![regular, bold],
            regular_font,
            bold_font,
            terminal: None,
            snapshot: None,
            viewport: (0.0, 0.0),
            font_size: DEFAULT_FONT_SIZE,
            focused: false,
            dirty: true,
            painted_revision: 0,
            selection: None,
            selecting: false,
            pressed_button: None,
            pending_copy: None,
            composition: None,
            alt_prefix_pending: false,
        }
    }

    pub(crate) fn registered_fonts(&self) -> &[RegisteredFont] {
        &self.fonts
    }

    pub(crate) fn sync(&mut self, terminal: Option<&Arc<Terminal>>, font_size: f64, focused: bool) {
        let changed_terminal = match (&self.terminal, terminal) {
            (Some(current), Some(next)) => !Arc::ptr_eq(current, next),
            (None, None) => false,
            _ => true,
        };
        if changed_terminal {
            self.terminal = terminal.cloned();
            self.snapshot = None;
            self.selection = None;
            self.selecting = false;
            self.pressed_button = None;
            self.composition = None;
            self.alt_prefix_pending = false;
            self.pending_copy = None;
            self.painted_revision = 0;
            self.dirty = true;
        }
        let font_size = font_size.max(8.0);
        if (self.font_size - font_size).abs() > f64::EPSILON {
            self.font_size = font_size;
            self.dirty = true;
            self.resize_terminal();
        }
        if self.focused != focused {
            self.focused = focused;
            if !focused {
                self.composition = None;
                self.selecting = false;
                self.pressed_button = None;
                self.alt_prefix_pending = false;
                self.pending_copy = None;
            }
            self.dirty = true;
        }
    }

    pub(crate) fn take_copy_text(&mut self) -> Option<String> {
        self.pending_copy.take()
    }

    pub(crate) fn unmount(&mut self) {
        self.viewport = (0.0, 0.0);
        self.selecting = false;
        self.pressed_button = None;
        self.composition = None;
        self.alt_prefix_pending = false;
        self.pending_copy = None;
    }

    fn cell_width(&self) -> f64 {
        let index = self.regular_font.max(0) as usize;
        let upem = self.inst.doc.font_upem[index].max(1) as f64;
        self.inst.doc.font_default_adv[index] as f64 * self.font_size / upem
    }

    fn cell_height(&self) -> f64 {
        self.font_size * 1.2
    }

    fn resize_terminal(&self) {
        let Some(terminal) = &self.terminal else {
            return;
        };
        let cols = (self.viewport.0 / self.cell_width()).floor().max(1.0) as usize;
        let rows = (self.viewport.1 / self.cell_height()).floor().max(1.0) as usize;
        terminal.resize_viewport(
            cols,
            rows,
            self.viewport.0.round().max(0.0) as usize,
            self.viewport.1.round().max(0.0) as usize,
        );
    }

    fn refresh_snapshot(&mut self) {
        let Some(terminal) = self.terminal.as_ref() else {
            self.snapshot = None;
            self.painted_revision = 0;
            return;
        };
        let revision = terminal.revision();
        if self.snapshot.is_some() && revision == self.painted_revision {
            return;
        }
        self.snapshot = Some(terminal.snapshot());
        self.painted_revision = revision;
    }

    fn point_at(&self, x: f64, y: f64) -> Option<SelectionPoint> {
        let snapshot = self.snapshot.as_ref()?;
        let mut column = (x / self.cell_width()).floor().max(0.0) as usize;
        let row = (y / self.cell_height()).floor().max(0.0) as usize;
        column = column.min(snapshot.cols.saturating_sub(1));
        let visible_row = row.min(snapshot.rows.saturating_sub(1));
        if snapshot.cells[visible_row * snapshot.cols + column].continuation && column > 0 {
            column -= 1;
        }
        Some(SelectionPoint {
            line: snapshot.viewport_top + visible_row,
            column,
        })
    }

    fn select_word(&self, point: SelectionPoint) -> Option<Selection> {
        let snapshot = self.snapshot.as_ref()?;
        let row = point.line.checked_sub(snapshot.viewport_top)?;
        if row >= snapshot.rows {
            return None;
        }
        let cells = &snapshot.cells[row * snapshot.cols..(row + 1) * snapshot.cols];
        let mut column = point.column.min(snapshot.cols - 1);
        if cells[column].continuation && column > 0 {
            column -= 1;
        }
        let class = |index: usize| {
            let leading = if cells[index].continuation && index > 0 {
                index - 1
            } else {
                index
            };
            word_character(cells[leading].character)
        };
        let word = class(column);
        let mut first = column;
        while first > 0 && class(first - 1) == word {
            first -= 1;
        }
        let mut last = column;
        while last + 1 < snapshot.cols && class(last + 1) == word {
            last += 1;
        }
        if cells[last].width == 2 && last + 1 < snapshot.cols {
            last += 1;
        }
        Some(Selection {
            anchor: SelectionPoint {
                line: point.line,
                column: first,
            },
            focus: SelectionPoint {
                line: point.line,
                column: last,
            },
            dragged: true,
        })
    }

    fn set_copy_from_selection(&mut self) {
        let (Some(terminal), Some(selection)) = (&self.terminal, self.selection) else {
            self.pending_copy = None;
            return;
        };
        let text = terminal.selection_text(
            (selection.anchor.line, selection.anchor.column),
            (selection.focus.line, selection.focus.column),
        );
        self.pending_copy = (!text.is_empty()).then_some(text);
    }

    fn pointer_dispatch(&mut self, event: &Event, effects: &mut Effects) {
        if event.etype == E_POINTER_DOWN {
            self.focused = true;
            self.dirty = true;
        }
        let modes = self
            .terminal
            .as_ref()
            .map_or_else(TerminalModes::default, |terminal| terminal.modes());
        let reporting = modes.mouse_tracking != MouseTracking::None && event.mods & M_SHIFT == 0;
        if reporting {
            self.report_mouse(event, modes);
            effects.repaint = true;
            effects.cursor = dispatch::CUR_DEFAULT;
            return;
        }

        effects.cursor = dispatch::CUR_TEXT;
        match event.etype {
            E_POINTER_DOWN if event.button == 0 => {
                self.focused = true;
                let Some(point) = self.point_at(event.x, event.y) else {
                    return;
                };
                self.selecting = true;
                self.selection = if event.clicks >= 3 {
                    self.snapshot.as_ref().map(|snapshot| Selection {
                        anchor: SelectionPoint {
                            line: point.line,
                            column: 0,
                        },
                        focus: SelectionPoint {
                            line: point.line,
                            column: snapshot.cols.saturating_sub(1),
                        },
                        dragged: true,
                    })
                } else if event.clicks == 2 {
                    self.select_word(point)
                } else {
                    Some(Selection {
                        anchor: point,
                        focus: point,
                        dragged: false,
                    })
                };
                effects.repaint = true;
            }
            E_POINTER_MOVE if self.selecting => {
                let Some(point) = self.point_at(event.x, event.y) else {
                    return;
                };
                if let Some(selection) = &mut self.selection {
                    selection.dragged |= selection.focus != point;
                    selection.focus = point;
                    effects.repaint = true;
                }
            }
            E_POINTER_UP if event.button == 0 => {
                if self.selecting {
                    if let Some(point) = self.point_at(event.x, event.y)
                        && let Some(selection) = &mut self.selection
                    {
                        selection.dragged |= selection.focus != point;
                        selection.focus = point;
                    }
                    self.selecting = false;
                    if self.selection.is_some_and(|selection| !selection.dragged) {
                        self.selection = None;
                    }
                    effects.repaint = true;
                }
            }
            _ => {}
        }
    }

    fn wheel_dispatch(&mut self, event: &Event, effects: &mut Effects) {
        let Some(terminal) = &self.terminal else {
            return;
        };
        let modes = terminal.modes();
        if modes.mouse_tracking != MouseTracking::None && event.mods & M_SHIFT == 0 {
            self.report_mouse(event, modes);
            effects.repaint = true;
            return;
        }
        if event.dy == 0.0 {
            return;
        }
        let lines = (event.dy.abs() / self.cell_height()).ceil().max(1.0) as i32;
        terminal.scroll(if event.dy < 0.0 { -lines } else { lines });
        effects.repaint = true;
        effects.cursor = dispatch::CUR_TEXT;
    }

    fn report_mouse(&mut self, event: &Event, modes: TerminalModes) {
        let Some(terminal) = &self.terminal else {
            return;
        };
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let column = ((event.x / self.cell_width()).floor().max(0.0) as usize)
            .min(snapshot.cols.saturating_sub(1))
            + 1;
        let row = ((event.y / self.cell_height()).floor().max(0.0) as usize)
            .min(snapshot.rows.saturating_sub(1))
            + 1;
        let modifier = ((event.mods & M_SHIFT != 0) as u32) * 4
            + ((event.mods & M_ALT != 0) as u32) * 8
            + ((event.mods & M_CTRL != 0) as u32) * 16;
        let (button, release) = match event.etype {
            E_WHEEL if event.dy != 0.0 => (if event.dy < 0.0 { 64 } else { 65 }, false),
            E_WHEEL if event.dx != 0.0 => (if event.dx < 0.0 { 66 } else { 67 }, false),
            E_WHEEL => return,
            E_POINTER_DOWN => {
                self.pressed_button = Some(event.button.min(2));
                (event.button.min(2), false)
            }
            E_POINTER_UP => {
                let button = self.pressed_button.take().unwrap_or(event.button.min(2));
                (button, true)
            }
            E_POINTER_MOVE => {
                let button = match self.pressed_button {
                    Some(button) => button,
                    None if modes.mouse_tracking == MouseTracking::Any => 3,
                    None => return,
                };
                if modes.mouse_tracking == MouseTracking::Press {
                    return;
                }
                (button + 32, false)
            }
            _ => return,
        };
        let code = button + modifier;
        if modes.sgr_mouse {
            let suffix = if release { 'm' } else { 'M' };
            terminal.write(format!("\x1b[<{code};{column};{row}{suffix}").as_bytes());
        } else {
            let legacy_code = if release { 3 + modifier } else { code };
            let bytes = [
                0x1b,
                b'[',
                b'M',
                (32 + legacy_code.min(223)) as u8,
                (32 + column.min(223) as u32) as u8,
                (32 + row.min(223) as u32) as u8,
            ];
            terminal.write(&bytes);
        }
    }

    fn dispatch_key(&mut self, event: &Event) {
        self.alt_prefix_pending = false;
        let Some(terminal) = &self.terminal else {
            return;
        };
        if event.mods & M_META != 0 {
            return;
        }
        let modifier = 1
            + (event.mods & M_SHIFT != 0) as u32
            + ((event.mods & M_ALT != 0) as u32) * 2
            + ((event.mods & M_CTRL != 0) as u32) * 4;
        let sequence = match event.key.as_str() {
            "Backspace" => Some(
                if event.mods & M_ALT != 0 {
                    "\x1b\x7f"
                } else {
                    "\x7f"
                }
                .to_owned(),
            ),
            "Enter" => Some(
                if event.mods & M_ALT != 0 {
                    "\x1b\r"
                } else {
                    "\r"
                }
                .to_owned(),
            ),
            "Tab" if event.mods & M_SHIFT != 0 => Some("\x1b[Z".to_owned()),
            "Tab" => Some("\t".to_owned()),
            "Escape" => Some("\x1b".to_owned()),
            "ArrowUp" => Some(modified_csi('A', modifier)),
            "ArrowDown" => Some(modified_csi('B', modifier)),
            "ArrowRight" => Some(modified_csi('C', modifier)),
            "ArrowLeft" => Some(modified_csi('D', modifier)),
            "Home" => Some(modified_csi('H', modifier)),
            "End" => Some(modified_csi('F', modifier)),
            "Insert" => Some(modified_tilde(2, modifier)),
            "Delete" => Some(modified_tilde(3, modifier)),
            "PageUp" => Some(modified_tilde(5, modifier)),
            "PageDown" => Some(modified_tilde(6, modifier)),
            "F1" => Some(modified_ss3('P', modifier)),
            "F2" => Some(modified_ss3('Q', modifier)),
            "F3" => Some(modified_ss3('R', modifier)),
            "F4" => Some(modified_ss3('S', modifier)),
            "F5" => Some(modified_tilde(15, modifier)),
            "F6" => Some(modified_tilde(17, modifier)),
            "F7" => Some(modified_tilde(18, modifier)),
            "F8" => Some(modified_tilde(19, modifier)),
            "F9" => Some(modified_tilde(20, modifier)),
            "F10" => Some(modified_tilde(21, modifier)),
            "F11" => Some(modified_tilde(23, modifier)),
            "F12" => Some(modified_tilde(24, modifier)),
            key if event.mods & M_CTRL != 0 => control_character(key).map(|byte| {
                let mut bytes = String::new();
                if event.mods & M_ALT != 0 {
                    bytes.push('\x1b');
                }
                bytes.push(char::from(byte));
                bytes
            }),
            key if event.mods & M_ALT != 0 && key.chars().count() == 1 => {
                self.alt_prefix_pending = true;
                None
            }
            _ => None,
        };
        if let Some(sequence) = sequence {
            terminal.write(sequence.as_bytes());
            self.selection = None;
            self.dirty = true;
        }
    }

    fn paint_snapshot(&self, frame: &mut Frame, snapshot: &TerminalSnapshot) {
        let cell_width = self.cell_width();
        let cell_height = self.cell_height();
        let visible_rows = snapshot
            .rows
            .min((self.viewport.1 / cell_height).ceil().max(0.0) as usize);
        let visible_cols = snapshot
            .cols
            .min((self.viewport.0 / cell_width).ceil().max(0.0) as usize);

        for row in 0..visible_rows {
            let cells = &snapshot.cells[row * snapshot.cols..(row + 1) * snapshot.cols];
            let mut column = 0;
            while column < visible_cols {
                let background = resolved_background(&cells[column]);
                let mut end = column + 1;
                while end < visible_cols && resolved_background(&cells[end]) == background {
                    end += 1;
                }
                if background != BACKGROUND {
                    push_rect(
                        frame,
                        column as f64 * cell_width,
                        row as f64 * cell_height,
                        (end - column) as f64 * cell_width,
                        cell_height,
                        background,
                    );
                }
                column = end;
            }
        }

        if let Some(selection) = self.selection {
            let (start, end) = ordered_selection(selection);
            for row in 0..visible_rows {
                let line = snapshot.viewport_top + row;
                if line < start.line || line > end.line {
                    continue;
                }
                let first = if line == start.line { start.column } else { 0 }.min(visible_cols);
                let last = if line == end.line {
                    let end_column = end.column.min(snapshot.cols.saturating_sub(1));
                    end_column.saturating_add(usize::from(
                        snapshot.cells[row * snapshot.cols + end_column]
                            .width
                            .max(1),
                    ))
                } else {
                    visible_cols
                }
                .min(visible_cols);
                if last > first {
                    push_rect(
                        frame,
                        first as f64 * cell_width,
                        row as f64 * cell_height,
                        (last - first) as f64 * cell_width,
                        cell_height,
                        SELECTION,
                    );
                }
            }
        }

        for row in 0..visible_rows {
            let cells = &snapshot.cells[row * snapshot.cols..(row + 1) * snapshot.cols];
            let mut column = 0;
            while column < visible_cols {
                if cells[column].continuation {
                    column += 1;
                    continue;
                }
                let foreground = resolved_foreground(&cells[column]);
                let bold = cells[column].bold;
                let wide = cells[column].width == 2;
                let mut end = column + usize::from(cells[column].width.max(1));
                if !wide {
                    while end < visible_cols {
                        let cell = &cells[end];
                        if cell.continuation
                            || cell.width != 1
                            || cell.bold != bold
                            || resolved_foreground(cell) != foreground
                        {
                            break;
                        }
                        end += 1;
                    }
                }
                let mut text = String::new();
                for cell in &cells[column..end.min(visible_cols)] {
                    cell.push_text(&mut text);
                }
                if text.chars().any(|character| character != ' ') {
                    let string_ref = frame.strings.len() as i32;
                    frame.strings.push(text);
                    let font = if bold {
                        self.bold_font
                    } else {
                        self.regular_font
                    };
                    frame.ops.push(FrameOp::Text(OpText {
                        node: 0,
                        x: column as f64 * cell_width,
                        y_baseline: row as f64 * cell_height + self.baseline(font),
                        str_ref: string_ref,
                        measured_w: (end - column) as f64 * cell_width,
                        font,
                        size: self.font_size,
                        weight: if bold { 700 } else { 400 },
                        tracking: 0.0,
                        color: foreground,
                        opacity: 1.0,
                        strike: false,
                        color_kind: 1,
                        gx: 0.0,
                        gy: 0.0,
                        gw: 0.0,
                        gh: 0.0,
                    }));
                }
                column = end.max(column + 1);
            }

            for (column, cell) in cells.iter().take(visible_cols).enumerate() {
                if cell.continuation || (!cell.underline && !cell.strikethrough) {
                    continue;
                }
                let color = resolved_foreground(cell);
                if cell.underline {
                    push_rect(
                        frame,
                        column as f64 * cell_width,
                        (row + 1) as f64 * cell_height - 2.0,
                        f64::from(cell.width.max(1)) * cell_width,
                        1.0,
                        color,
                    );
                }
                if cell.strikethrough {
                    push_rect(
                        frame,
                        column as f64 * cell_width,
                        row as f64 * cell_height + cell_height * 0.56,
                        f64::from(cell.width.max(1)) * cell_width,
                        1.0,
                        color,
                    );
                }
            }
        }

        if snapshot.cursor_visible
            && !snapshot.exited
            && snapshot.cursor_row < visible_rows
            && snapshot.cursor_col < visible_cols
        {
            push_rect(
                frame,
                snapshot.cursor_col as f64 * cell_width,
                snapshot.cursor_row as f64 * cell_height + 1.0,
                2.0,
                (cell_height - 2.0).max(1.0),
                CURSOR,
            );
        }

        if snapshot.exited {
            self.push_text(
                frame,
                "process exited — toggle Terminal to start a new shell",
                8.0,
                (self.viewport.1 - cell_height).max(0.0),
                MUTED,
                false,
            );
        }
    }

    fn baseline(&self, font: i32) -> f64 {
        let index = font.max(0) as usize;
        let upem = self.inst.doc.font_upem[index].max(1) as f64;
        let ascent = self.inst.doc.font_ascent[index] as f64 * self.font_size / upem;
        let descent = self.inst.doc.font_descent[index] as f64 * self.font_size / upem;
        let ink_height = ascent - descent;
        (self.cell_height() - ink_height) * 0.5 + ascent
    }

    fn push_text(&self, frame: &mut Frame, text: &str, x: f64, y: f64, color: u32, bold: bool) {
        let string_ref = frame.strings.len() as i32;
        frame.strings.push(text.to_owned());
        let font = if bold {
            self.bold_font
        } else {
            self.regular_font
        };
        frame.ops.push(FrameOp::Text(OpText {
            node: 0,
            x,
            y_baseline: y + self.baseline(font),
            str_ref: string_ref,
            measured_w: text.chars().count() as f64 * self.cell_width(),
            font,
            size: self.font_size,
            weight: if bold { 700 } else { 400 },
            tracking: 0.0,
            color,
            opacity: 1.0,
            strike: false,
            color_kind: 1,
            gx: 0.0,
            gy: 0.0,
            gw: 0.0,
            gh: 0.0,
        }));
    }

    /// Rebuilds terminal paint output without discarding frame allocations.
    pub(crate) fn update_frame(&mut self, frame: &mut Frame) {
        frame.clear();
        frame.width = self.viewport.0;
        frame.height = self.viewport.1;
        frame.ops.push(FrameOp::ClipPush(OpClip {
            x: 0.0,
            y: 0.0,
            w: self.viewport.0,
            h: self.viewport.1,
            radius: 0.0,
            smooth: 0.0,
        }));
        push_rect(
            frame,
            0.0,
            0.0,
            self.viewport.0,
            self.viewport.1,
            BACKGROUND,
        );
        self.refresh_snapshot();
        if let Some(snapshot) = self.snapshot.as_ref() {
            self.paint_snapshot(frame, snapshot);
        } else {
            self.push_text(frame, "Terminal unavailable", 0.0, 0.0, MUTED, false);
        }
        frame.ops.push(FrameOp::ClipPop);
        self.dirty = false;
    }
}

impl HoleContent for TerminalHole {
    fn resize(&mut self, width: f64, height: f64, _: bool, _: bool) {
        let viewport = (width.max(0.0), height.max(0.0));
        if self.viewport != viewport {
            self.viewport = viewport;
            self.resize_terminal();
            self.dirty = true;
        }
    }

    fn natural(&mut self) -> (f64, f64) {
        self.refresh_snapshot();
        let (cols, rows) = self
            .snapshot
            .as_ref()
            .map_or((DEFAULT_COLS, DEFAULT_ROWS), |snapshot| {
                (snapshot.cols, snapshot.rows)
            });
        (
            cols as f64 * self.cell_width(),
            rows as f64 * self.cell_height(),
        )
    }

    fn frame(&mut self, _: f64) -> Frame {
        let mut frame = frame_new();
        self.update_frame(&mut frame);
        frame
    }

    fn instance(&self) -> &Instance {
        &self.inst
    }

    fn dispatch(&mut self, event: &Event) -> Effects {
        let mut effects = dispatch::effects_new();
        match event.etype {
            E_POINTER_DOWN | E_POINTER_MOVE | E_POINTER_UP => {
                self.pointer_dispatch(event, &mut effects);
            }
            E_WHEEL => self.wheel_dispatch(event, &mut effects),
            E_KEY_DOWN if self.focused => self.dispatch_key(event),
            E_TEXT if self.focused && self.composition.is_none() && event.mods & M_META == 0 => {
                if let Some(terminal) = &self.terminal {
                    if self.alt_prefix_pending {
                        terminal.write(b"\x1b");
                        self.alt_prefix_pending = false;
                    }
                    terminal.write(event.text.as_bytes());
                    self.selection = None;
                    self.dirty = true;
                }
            }
            E_COMPOSITION_START if self.focused => {
                self.composition = Some(String::new());
            }
            E_COMPOSITION_UPDATE if self.focused => {
                self.composition = Some(event.text.clone());
            }
            E_COMPOSITION_END if self.focused => {
                let composed = self.composition.take();
                let text = if event.text.is_empty() {
                    composed.as_deref().unwrap_or_default()
                } else {
                    event.text.as_str()
                };
                if let Some(terminal) = &self.terminal {
                    if self.alt_prefix_pending {
                        terminal.write(b"\x1b");
                        self.alt_prefix_pending = false;
                    }
                    terminal.write(text.as_bytes());
                    self.selection = None;
                    self.dirty = true;
                }
            }
            E_PASTE if self.focused => {
                self.alt_prefix_pending = false;
                if let Some(terminal) = &self.terminal {
                    if terminal.modes().bracketed_paste {
                        terminal.write(b"\x1b[200~");
                        terminal.write(event.text.as_bytes());
                        terminal.write(b"\x1b[201~");
                    } else {
                        terminal.write(event.text.as_bytes());
                    }
                    self.selection = None;
                    self.dirty = true;
                }
            }
            E_COPY | E_CUT if self.focused => self.set_copy_from_selection(),
            E_BLUR => {
                self.focused = false;
                self.selecting = false;
                self.pressed_button = None;
                self.composition = None;
                self.alt_prefix_pending = false;
                self.dirty = true;
            }
            E_RESIZE => self.resize(event.dx, event.dy, true, false),
            _ => {}
        }
        effects.repaint |= self.dirty;
        effects
    }

    fn needs_frame(&self) -> bool {
        self.viewport.0 > 0.0
            && self.viewport.1 > 0.0
            && (self.dirty
                || self
                    .terminal
                    .as_ref()
                    .is_some_and(|terminal| terminal.revision() != self.painted_revision))
    }
}

fn register_font(inst: &mut Instance, weight: u16) -> (i32, RegisteredFont) {
    let asset = slab_fonts::asset(slab_fonts::CLASS_MONO, weight);
    let metrics = slab_fonts::parse_metrics(asset.bytes).expect("bundled terminal font metrics");
    let index = kframe::inst_font_register(
        inst,
        FONT_FAMILY,
        u32::from(metrics.weight),
        u32::from(metrics.upem),
        i32::from(metrics.ascent),
        i32::from(metrics.descent),
        i32::from(metrics.line_gap),
        u32::from(metrics.default_advance),
        &metrics.cps,
        &metrics.gids,
        &metrics.advances,
    );
    (
        index,
        RegisteredFont {
            name: FONT_FAMILY.to_owned(),
            weight: u32::from(metrics.weight),
            bytes: asset.bytes.to_vec(),
        },
    )
}

fn modified_csi(final_byte: char, modifier: u32) -> String {
    if modifier == 1 {
        format!("\x1b[{final_byte}")
    } else {
        format!("\x1b[1;{modifier}{final_byte}")
    }
}

fn modified_ss3(final_byte: char, modifier: u32) -> String {
    if modifier == 1 {
        format!("\x1bO{final_byte}")
    } else {
        format!("\x1b[1;{modifier}{final_byte}")
    }
}

fn modified_tilde(number: u32, modifier: u32) -> String {
    if modifier == 1 {
        format!("\x1b[{number}~")
    } else {
        format!("\x1b[{number};{modifier}~")
    }
}

fn control_character(key: &str) -> Option<u8> {
    let character = key.chars().next()?;
    if key.chars().count() != 1 {
        return None;
    }
    match character {
        '@' | ' ' => Some(0),
        'a'..='z' | 'A'..='Z' => Some(character.to_ascii_uppercase() as u8 - b'@'),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '?' => Some(0x7f),
        _ => None,
    }
}

fn ordered_selection(selection: Selection) -> (SelectionPoint, SelectionPoint) {
    if selection.anchor <= selection.focus {
        (selection.anchor, selection.focus)
    } else {
        (selection.focus, selection.anchor)
    }
}

fn word_character(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '_' | '-' | '.' | '/' | '~')
}

fn resolved_foreground(cell: &Cell) -> u32 {
    let color = if cell.inverse {
        terminal_color(cell.background, BACKGROUND)
    } else {
        terminal_color(cell.foreground, FOREGROUND)
    };
    if cell.dim { dim(color) } else { color }
}

fn resolved_background(cell: &Cell) -> u32 {
    if cell.inverse {
        terminal_color(cell.foreground, FOREGROUND)
    } else {
        terminal_color(cell.background, BACKGROUND)
    }
}

fn terminal_color(color: TerminalColor, default: u32) -> u32 {
    match color {
        TerminalColor::Default => default,
        TerminalColor::Rgb(red, green, blue) => rgba(red, green, blue, 255),
        TerminalColor::Indexed(index) if index < 16 => ansi_color(index),
        TerminalColor::Indexed(index) if index < 232 => {
            let value = index - 16;
            let level = |component| {
                if component == 0 {
                    0
                } else {
                    55 + component * 40
                }
            };
            rgba(
                level(value / 36),
                level((value / 6) % 6),
                level(value % 6),
                255,
            )
        }
        TerminalColor::Indexed(index) => {
            let gray = 8 + (index - 232) * 10;
            rgba(gray, gray, gray, 255)
        }
    }
}

fn ansi_color(index: u8) -> u32 {
    match index {
        0 => rgba(115, 115, 115, 255),
        1 | 9 => rgba(235, 87, 87, 255),
        2 | 10 => rgba(76, 183, 130, 255),
        3 | 11 => rgba(247, 165, 80, 255),
        4 => rgba(212, 212, 212, 255),
        5 | 13 => rgba(168, 120, 245, 255),
        6 => rgba(237, 237, 237, 255),
        7 => rgba(163, 163, 163, 255),
        8 => rgba(82, 82, 82, 255),
        12 | 14 => rgba(255, 255, 255, 255),
        _ => FOREGROUND,
    }
}

fn dim(color: u32) -> u32 {
    let [red, green, blue, alpha] = color.to_le_bytes();
    let scale = |component: u8| (u16::from(component) * 3 / 5) as u8;
    rgba(scale(red), scale(green), scale(blue), alpha)
}

fn push_rect(frame: &mut Frame, x: f64, y: f64, width: f64, height: f64, color: u32) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    frame.ops.push(FrameOp::Rect(OpRect {
        node: 0,
        x,
        y,
        w: width,
        h: height,
        radius: 0.0,
        bg_kind: 1,
        bg: color,
        stroke_kind: 0,
        stroke: 0,
        stroke_w: 0.0,
        stroke_align: 0,
        stroke_sides: 0,
        dash_on: 0.0,
        dash_off: 0.0,
        has_dash: false,
        shadow_off: 0,
        shadow_len: 0,
        opacity: 1.0,
        smooth: 0.0,
        grain_amount: 0.0,
        grain_size: 1.0,
    }));
}

const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> u32 {
    u32::from_le_bytes([red, green, blue, alpha])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_mounted_terminal_holes_request_frames() {
        let mut hole = TerminalHole::new();
        assert!(!hole.needs_frame());

        hole.resize(640.0, 480.0, true, false);
        assert!(hole.needs_frame());

        hole.unmount();
        assert!(!hole.needs_frame());
    }
}
