use std::collections::VecDeque;
use unicode_width::UnicodeWidthChar;
use vte::{Params, Perform};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalColor {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Cell {
    pub(crate) character: char,
    pub(crate) combining: String,
    pub(crate) foreground: TerminalColor,
    pub(crate) background: TerminalColor,
    pub(crate) inverse: bool,
    pub(crate) bold: bool,
    pub(crate) dim: bool,
    pub(crate) italic: bool,
    pub(crate) underline: bool,
    pub(crate) strikethrough: bool,
    pub(crate) width: u8,
    pub(crate) continuation: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            character: ' ',
            combining: String::new(),
            foreground: TerminalColor::Default,
            background: TerminalColor::Default,
            inverse: false,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            strikethrough: false,
            width: 1,
            continuation: false,
        }
    }
}

impl Cell {
    pub(crate) fn push_text(&self, output: &mut String) {
        if self.continuation {
            return;
        }
        output.push(self.character);
        output.push_str(&self.combining);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TerminalSnapshot {
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    pub(crate) cells: Vec<Cell>,
    pub(crate) cursor_col: usize,
    pub(crate) cursor_row: usize,
    pub(crate) cursor_visible: bool,
    pub(crate) exited: bool,
    /// Stable absolute line number of viewport row zero.
    pub(crate) viewport_top: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MouseTracking {
    #[default]
    None,
    Press,
    Button,
    Any,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TerminalModes {
    pub(crate) mouse_tracking: MouseTracking,
    pub(crate) sgr_mouse: bool,
    pub(crate) bracketed_paste: bool,
}

#[derive(Debug)]
pub(super) struct Grid {
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
    cursor_col: usize,
    cursor_row: usize,
    saved_cursor: (usize, usize),
    foreground: TerminalColor,
    background: TerminalColor,
    inverse: bool,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    cursor_visible: bool,
    scroll_top: usize,
    history: VecDeque<Vec<Cell>>,
    history_base: usize,
    scrollback_offset: usize,
    scroll_bottom: usize,
    modes: TerminalModes,
}

impl Grid {
    pub(super) fn new(cols: usize, rows: usize) -> Self {
        let mut grid = Self {
            cols: cols.max(1),
            rows: rows.max(1),
            cells: Vec::new(),
            cursor_col: 0,
            cursor_row: 0,
            saved_cursor: (0, 0),
            foreground: TerminalColor::Default,
            background: TerminalColor::Default,
            inverse: false,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            strikethrough: false,
            cursor_visible: true,
            scroll_top: 0,
            scroll_bottom: rows.max(1) - 1,
            history: VecDeque::new(),
            history_base: 0,
            scrollback_offset: 0,
            modes: TerminalModes::default(),
        };
        grid.cells.resize(grid.cols * grid.rows, Cell::default());
        grid
    }

    pub(super) fn resize(&mut self, cols: usize, rows: usize) -> bool {
        let cols = cols.max(1);
        let rows = rows.max(1);
        if self.cols == cols && self.rows == rows {
            return false;
        }
        let mut cells = vec![Cell::default(); cols * rows];
        for row in 0..self.rows.min(rows) {
            let source = row * self.cols;
            let destination = row * cols;
            cells[destination..destination + self.cols.min(cols)]
                .clone_from_slice(&self.cells[source..source + self.cols.min(cols)]);
        }
        for history_row in &mut self.history {
            history_row.resize(cols, Cell::default());
            history_row.truncate(cols);
        }
        self.cols = cols;
        self.rows = rows;
        self.cells = cells;
        self.cursor_col = self.cursor_col.min(cols - 1);
        self.cursor_row = self.cursor_row.min(rows - 1);
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
        self.scrollback_offset = self.scrollback_offset.min(self.history.len());
        true
    }

    pub(super) fn snapshot(&self, exited: bool) -> TerminalSnapshot {
        let total_rows = self.history.len() + self.rows;
        let end = total_rows.saturating_sub(self.scrollback_offset);
        let start = end.saturating_sub(self.rows);
        let mut cells = Vec::with_capacity(self.cols * self.rows);
        for line in start..end {
            if line < self.history.len() {
                cells.extend(self.history[line].iter().cloned());
            } else {
                let screen_row = line - self.history.len();
                let offset = screen_row * self.cols;
                cells.extend(self.cells[offset..offset + self.cols].iter().cloned());
            }
        }
        cells.resize(self.cols * self.rows, Cell::default());
        TerminalSnapshot {
            cols: self.cols,
            rows: self.rows,
            cells,
            cursor_col: self.cursor_col,
            cursor_row: self.cursor_row,
            cursor_visible: self.cursor_visible && self.scrollback_offset == 0,
            exited,
            viewport_top: self.history_base + start,
        }
    }

    fn blank(&self) -> Cell {
        Cell {
            foreground: self.foreground,
            background: self.background,
            inverse: self.inverse,
            bold: self.bold,
            dim: self.dim,
            italic: self.italic,
            underline: self.underline,
            strikethrough: self.strikethrough,
            ..Cell::default()
        }
    }

    pub(super) fn scroll(&mut self, delta: i32) -> bool {
        let previous = self.scrollback_offset;
        if delta < 0 {
            self.scrollback_offset = self
                .scrollback_offset
                .saturating_add(delta.unsigned_abs() as usize)
                .min(self.history.len());
        } else {
            self.scrollback_offset = self.scrollback_offset.saturating_sub(delta as usize);
        }
        previous != self.scrollback_offset
    }

    pub(super) fn modes(&self) -> TerminalModes {
        self.modes
    }

    pub(super) fn selection_text(&self, start: (usize, usize), end: (usize, usize)) -> String {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let mut selected = String::new();
        for line in start.0..=end.0 {
            let Some(cells) = self.absolute_row(line) else {
                continue;
            };
            let first_col = if line == start.0 { start.1 } else { 0 }.min(self.cols - 1);
            let last_col = if line == end.0 {
                end.1.min(self.cols - 1)
            } else {
                self.cols - 1
            };
            let line_start = selected.len();
            for cell in &cells[first_col..=last_col] {
                if cell.continuation {
                    continue;
                }
                cell.push_text(&mut selected);
            }
            while selected.len() > line_start && selected.ends_with(' ') {
                selected.pop();
            }
            if line != end.0 {
                selected.push('\n');
            }
        }
        selected
    }

    fn absolute_row(&self, line: usize) -> Option<&[Cell]> {
        let relative = line.checked_sub(self.history_base)?;
        if relative < self.history.len() {
            return self.history.get(relative).map(Vec::as_slice);
        }
        let screen_row = relative.checked_sub(self.history.len())?;
        if screen_row >= self.rows {
            return None;
        }
        let start = screen_row * self.cols;
        Some(&self.cells[start..start + self.cols])
    }
    fn index(&self, row: usize, col: usize) -> usize {
        row * self.cols + col
    }
    fn clear_row(&mut self, row: usize) {
        let blank = self.blank();
        for col in 0..self.cols {
            let index = self.index(row, col);
            self.cells[index] = blank.clone();
        }
    }
    fn scroll_up(&mut self, top: usize, bottom: usize, count: usize) {
        for _ in 0..count {
            if top == 0 && bottom + 1 == self.rows {
                self.history.push_back(self.cells[..self.cols].to_vec());
                if self.scrollback_offset > 0 {
                    self.scrollback_offset = self
                        .scrollback_offset
                        .saturating_add(1)
                        .min(self.history.len());
                }
                if self.history.len() > 10_000 {
                    self.history.pop_front();
                    self.history_base = self.history_base.saturating_add(1);
                }
                self.scrollback_offset = self.scrollback_offset.min(self.history.len());
            }
            for row in top..bottom {
                for col in 0..self.cols {
                    let destination = self.index(row, col);
                    let source = self.index(row + 1, col);
                    let cell = self.cells[source].clone();
                    self.cells[destination] = cell;
                }
            }
            self.clear_row(bottom);
        }
    }
    fn scroll_down(&mut self, top: usize, bottom: usize, count: usize) {
        for _ in 0..count {
            for row in (top + 1..=bottom).rev() {
                for col in 0..self.cols {
                    let destination = self.index(row, col);
                    let source = self.index(row - 1, col);
                    let cell = self.cells[source].clone();
                    self.cells[destination] = cell;
                }
            }
            self.clear_row(top);
        }
    }

    fn clear_wide_at(&mut self, row: usize, col: usize) {
        let index = self.index(row, col);
        let blank = self.blank();
        if self.cells[index].continuation && col > 0 {
            let leading = self.index(row, col - 1);
            self.cells[leading] = blank.clone();
        } else if self.cells[index].width == 2 && col + 1 < self.cols {
            let trailing = self.index(row, col + 1);
            self.cells[trailing] = blank.clone();
        }
        self.cells[index] = blank;
    }
    fn linefeed(&mut self) {
        if self.cursor_row == self.scroll_bottom {
            self.scroll_up(self.scroll_top, self.scroll_bottom, 1);
        } else {
            self.cursor_row = (self.cursor_row + 1).min(self.rows - 1);
        }
    }
    fn param(params: &Params, index: usize, default: usize) -> usize {
        params
            .iter()
            .nth(index)
            .and_then(|parameter| parameter.first())
            .map_or(default, |value| usize::from(*value))
    }
    fn erase_display(&mut self, mode: usize) {
        match mode {
            0 => {
                let cursor = self.index(self.cursor_row, self.cursor_col.min(self.cols - 1));
                let blank = self.blank();
                self.cells[cursor..].fill(blank);
            }
            1 => {
                let cursor = self.index(self.cursor_row, self.cursor_col.min(self.cols - 1));
                let blank = self.blank();
                self.cells[..=cursor].fill(blank);
            }
            _ => {
                let blank = self.blank();
                self.cells.fill(blank);
            }
        }
    }
    fn erase_line(&mut self, mode: usize) {
        let start = self.cursor_row * self.cols;
        let column = self.cursor_col.min(self.cols - 1);
        let blank = self.blank();
        match mode {
            0 => self.cells[start + column..start + self.cols].fill(blank),
            1 => self.cells[start..=start + column].fill(blank),
            _ => self.cells[start..start + self.cols].fill(blank),
        }
    }
    fn sgr(&mut self, params: &Params) {
        let values: Vec<usize> = params
            .iter()
            .map(|parameter| parameter.first().map_or(0, |value| usize::from(*value)))
            .collect();
        let values = if values.is_empty() { vec![0] } else { values };
        let mut index = 0;
        while index < values.len() {
            match values[index] {
                0 => {
                    self.foreground = TerminalColor::Default;
                    self.background = TerminalColor::Default;
                    self.inverse = false;
                    self.bold = false;
                    self.dim = false;
                    self.italic = false;
                    self.underline = false;
                    self.strikethrough = false;
                }
                1 => self.bold = true,
                2 => self.dim = true,
                3 => self.italic = true,
                4 | 21 => self.underline = true,
                7 => self.inverse = true,
                9 => self.strikethrough = true,
                22 => {
                    self.bold = false;
                    self.dim = false;
                }
                23 => self.italic = false,
                24 => self.underline = false,
                27 => self.inverse = false,
                29 => self.strikethrough = false,
                30..=37 => self.foreground = TerminalColor::Indexed((values[index] - 30) as u8),
                90..=97 => self.foreground = TerminalColor::Indexed((values[index] - 90 + 8) as u8),
                40..=47 => self.background = TerminalColor::Indexed((values[index] - 40) as u8),
                100..=107 => {
                    self.background = TerminalColor::Indexed((values[index] - 100 + 8) as u8)
                }
                39 => self.foreground = TerminalColor::Default,
                49 => self.background = TerminalColor::Default,
                38 | 48 if index + 1 < values.len() => {
                    let foreground = values[index] == 38;
                    match values[index + 1] {
                        5 if index + 2 < values.len() => {
                            let color = TerminalColor::Indexed(values[index + 2] as u8);
                            if foreground {
                                self.foreground = color;
                            } else {
                                self.background = color;
                            }
                            index += 2;
                        }
                        2 if index + 4 < values.len() => {
                            let color = TerminalColor::Rgb(
                                values[index + 2] as u8,
                                values[index + 3] as u8,
                                values[index + 4] as u8,
                            );
                            if foreground {
                                self.foreground = color;
                            } else {
                                self.background = color;
                            }
                            index += 4;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }

    fn set_private_modes(&mut self, params: &Params, enabled: bool) {
        for parameter in params.iter() {
            let Some(mode) = parameter.first().copied() else {
                continue;
            };
            match mode {
                25 => self.cursor_visible = enabled,
                1000 if enabled => self.modes.mouse_tracking = MouseTracking::Press,
                1002 if enabled => self.modes.mouse_tracking = MouseTracking::Button,
                1003 if enabled => self.modes.mouse_tracking = MouseTracking::Any,
                1000 | 1002 | 1003 => self.modes.mouse_tracking = MouseTracking::None,
                1006 => self.modes.sgr_mouse = enabled,
                2004 => self.modes.bracketed_paste = enabled,
                _ => {}
            }
        }
    }
}

impl Perform for Grid {
    fn print(&mut self, character: char) {
        let width = UnicodeWidthChar::width(character).unwrap_or(1).min(2);
        if width == 0 {
            if self.cursor_col > 0 {
                let mut column = self.cursor_col.min(self.cols) - 1;
                if self.cells[self.index(self.cursor_row, column)].continuation && column > 0 {
                    column -= 1;
                }
                let index = self.index(self.cursor_row, column);
                self.cells[index].combining.push(character);
                return;
            }
        }
        let width = if self.cols == 1 { 1 } else { width.max(1) };
        if self.cursor_col >= self.cols || (width == 2 && self.cursor_col + 1 >= self.cols) {
            self.cursor_col = 0;
            self.linefeed();
        }
        let column = self.cursor_col;
        self.clear_wide_at(self.cursor_row, column);
        if width == 2 {
            self.clear_wide_at(self.cursor_row, column + 1);
        }
        let mut cell = self.blank();
        cell.character = character;
        cell.width = width as u8;
        let index = self.index(self.cursor_row, column);
        self.cells[index] = cell.clone();
        if width == 2 {
            let mut continuation = cell;
            continuation.character = ' ';
            continuation.combining.clear();
            continuation.width = 0;
            continuation.continuation = true;
            let trailing = self.index(self.cursor_row, column + 1);
            self.cells[trailing] = continuation;
        }
        self.cursor_col += width;
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' | 0x0b | 0x0c => self.linefeed(),
            b'\r' => self.cursor_col = 0,
            0x08 => self.cursor_col = self.cursor_col.saturating_sub(1),
            b'\t' => {
                self.cursor_col = ((self.cursor_col / 8) + 1)
                    .saturating_mul(8)
                    .min(self.cols - 1);
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _: bool, action: char) {
        let amount = Self::param(params, 0, 1).max(1);
        match action {
            'A' => self.cursor_row = self.cursor_row.saturating_sub(amount),
            'B' => self.cursor_row = (self.cursor_row + amount).min(self.rows - 1),
            'C' => {
                self.cursor_col = (self.cursor_col.min(self.cols - 1) + amount).min(self.cols - 1)
            }
            'D' => self.cursor_col = self.cursor_col.saturating_sub(amount),
            'E' => {
                self.cursor_row = (self.cursor_row + amount).min(self.rows - 1);
                self.cursor_col = 0;
            }
            'F' => {
                self.cursor_row = self.cursor_row.saturating_sub(amount);
                self.cursor_col = 0;
            }
            'G' | '`' => self.cursor_col = amount.saturating_sub(1).min(self.cols - 1),
            'H' | 'f' => {
                self.cursor_row = Self::param(params, 0, 1)
                    .saturating_sub(1)
                    .min(self.rows - 1);
                self.cursor_col = Self::param(params, 1, 1)
                    .saturating_sub(1)
                    .min(self.cols - 1);
            }
            'd' => self.cursor_row = amount.saturating_sub(1).min(self.rows - 1),
            'J' => self.erase_display(Self::param(params, 0, 0)),
            'K' => self.erase_line(Self::param(params, 0, 0)),
            'X' => {
                let column = self.cursor_col.min(self.cols - 1);
                let end = (column + amount).min(self.cols);
                let blank = self.blank();
                self.cells[self.cursor_row * self.cols + column..self.cursor_row * self.cols + end]
                    .fill(blank);
            }
            '@' => {
                let column = self.cursor_col.min(self.cols - 1);
                let count = amount.min(self.cols - column);
                let blank = self.blank();
                let row =
                    &mut self.cells[self.cursor_row * self.cols..(self.cursor_row + 1) * self.cols];
                row[column..].rotate_right(count);
                row[column..column + count].fill(blank);
            }
            'P' => {
                let column = self.cursor_col.min(self.cols - 1);
                let count = amount.min(self.cols - column);
                let blank = self.blank();
                let row =
                    &mut self.cells[self.cursor_row * self.cols..(self.cursor_row + 1) * self.cols];
                row[column..].rotate_left(count);
                row[self.cols - count..].fill(blank);
            }
            'L' if self.cursor_row >= self.scroll_top && self.cursor_row <= self.scroll_bottom => {
                self.scroll_down(self.cursor_row, self.scroll_bottom, amount);
            }
            'M' if self.cursor_row >= self.scroll_top && self.cursor_row <= self.scroll_bottom => {
                self.scroll_up(self.cursor_row, self.scroll_bottom, amount);
            }
            'S' => self.scroll_up(self.scroll_top, self.scroll_bottom, amount),
            'T' => self.scroll_down(self.scroll_top, self.scroll_bottom, amount),
            'm' => self.sgr(params),
            'r' => {
                self.scroll_top = Self::param(params, 0, 1)
                    .saturating_sub(1)
                    .min(self.rows - 1);
                self.scroll_bottom = Self::param(params, 1, self.rows)
                    .saturating_sub(1)
                    .min(self.rows - 1)
                    .max(self.scroll_top);
                self.cursor_col = 0;
                self.cursor_row = self.scroll_top;
            }
            's' => self.saved_cursor = (self.cursor_col, self.cursor_row),
            'u' => (self.cursor_col, self.cursor_row) = self.saved_cursor,
            'h' | 'l' if intermediates.contains(&b'?') => {
                self.set_private_modes(params, action == 'h');
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, _: &[u8], _: bool, byte: u8) {
        match byte {
            b'7' => self.saved_cursor = (self.cursor_col, self.cursor_row),
            b'8' => (self.cursor_col, self.cursor_row) = self.saved_cursor,
            b'D' => self.linefeed(),
            b'M' if self.cursor_row == self.scroll_top => {
                self.scroll_down(self.scroll_top, self.scroll_bottom, 1);
            }
            b'M' => self.cursor_row = self.cursor_row.saturating_sub(1),
            b'c' => *self = Self::new(self.cols, self.rows),
            _ => {}
        }
    }

    fn hook(&mut self, _: &Params, _: &[u8], _: bool, _: char) {}
    fn put(&mut self, _: u8) {}
    fn unhook(&mut self) {}
    fn osc_dispatch(&mut self, _: &[&[u8]], _: bool) {}
}
