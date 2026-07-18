use vte::{Params, Perform};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalColor {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cell {
    pub(crate) character: char,
    pub(crate) foreground: TerminalColor,
    pub(crate) background: TerminalColor,
    pub(crate) inverse: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            character: ' ',
            foreground: TerminalColor::Default,
            background: TerminalColor::Default,
            inverse: false,
        }
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
    cursor_visible: bool,
    scroll_top: usize,
    history: Vec<Vec<Cell>>,
    scrollback_offset: usize,
    scroll_bottom: usize,
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
            cursor_visible: true,
            scroll_top: 0,
            scroll_bottom: rows.max(1) - 1,
            history: Vec::new(),
            scrollback_offset: 0,
        };
        grid.cells.resize(grid.cols * grid.rows, Cell::default());
        grid
    }

    pub(super) fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        if self.cols == cols && self.rows == rows {
            return;
        }
        let mut cells = vec![Cell::default(); cols * rows];
        for row in 0..self.rows.min(rows) {
            let source = row * self.cols;
            let destination = row * cols;
            cells[destination..destination + self.cols.min(cols)]
                .copy_from_slice(&self.cells[source..source + self.cols.min(cols)]);
        }
        self.cols = cols;
        self.rows = rows;
        self.cells = cells;
        self.cursor_col = self.cursor_col.min(cols - 1);
        self.cursor_row = self.cursor_row.min(rows - 1);
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
    }

    pub(super) fn snapshot(&self, exited: bool) -> TerminalSnapshot {
        let mut all_rows = self.history.clone();
        all_rows.extend(self.cells.chunks(self.cols).map(<[Cell]>::to_vec));
        let start = all_rows
            .len()
            .saturating_sub(self.rows + self.scrollback_offset);
        let mut cells =
            all_rows[start..all_rows.len().saturating_sub(self.scrollback_offset)].concat();
        cells.resize(self.cols * self.rows, Cell::default());
        TerminalSnapshot {
            cols: self.cols,
            rows: self.rows,
            cells,
            cursor_col: self.cursor_col,
            cursor_row: self.cursor_row,
            cursor_visible: self.cursor_visible && self.scrollback_offset == 0,
            exited,
        }
    }

    fn blank(&self) -> Cell {
        Cell {
            character: ' ',
            foreground: self.foreground,
            background: self.background,
            inverse: self.inverse,
        }
    }

    pub(super) fn scroll(&mut self, delta: i32) {
        if delta < 0 {
            self.scrollback_offset = self
                .scrollback_offset
                .saturating_add(delta.unsigned_abs() as usize)
                .min(self.history.len());
        } else {
            self.scrollback_offset = self.scrollback_offset.saturating_sub(delta as usize);
        }
    }
    fn index(&self, row: usize, col: usize) -> usize {
        row * self.cols + col
    }
    fn clear_row(&mut self, row: usize) {
        let blank = self.blank();
        for col in 0..self.cols {
            let index = self.index(row, col);
            self.cells[index] = blank;
        }
    }
    fn scroll_up(&mut self, top: usize, bottom: usize, count: usize) {
        for _ in 0..count {
            if top == 0 && bottom + 1 == self.rows {
                self.history.push(self.cells[..self.cols].to_vec());
                if self.history.len() > 10_000 {
                    self.history.remove(0);
                }
            }
            for row in top..bottom {
                for col in 0..self.cols {
                    let destination = self.index(row, col);
                    let source = self.index(row + 1, col);
                    self.cells[destination] = self.cells[source];
                }
            }
            self.clear_row(bottom);
        }
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
                let cursor = self.index(self.cursor_row, self.cursor_col);
                let blank = self.blank();
                self.cells[cursor..].fill(blank);
            }
            1 => {
                let cursor = self.index(self.cursor_row, self.cursor_col);
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
        let blank = self.blank();
        match mode {
            0 => self.cells[start + self.cursor_col..start + self.cols].fill(blank),
            1 => self.cells[start..=start + self.cursor_col].fill(blank),
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
                }
                7 => self.inverse = true,
                27 => self.inverse = false,
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
}

impl Perform for Grid {
    fn print(&mut self, character: char) {
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.linefeed();
        }
        let index = self.index(self.cursor_row, self.cursor_col);
        self.cells[index] = Cell {
            character,
            foreground: self.foreground,
            background: self.background,
            inverse: self.inverse,
        };
        self.cursor_col += 1;
    }
    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' | 0x0b | 0x0c => self.linefeed(),
            b'\r' => self.cursor_col = 0,
            0x08 => self.cursor_col = self.cursor_col.saturating_sub(1),
            b'\t' => {
                self.cursor_col = ((self.cursor_col / 8) + 1)
                    .saturating_mul(8)
                    .min(self.cols - 1)
            }
            _ => {}
        }
    }
    fn csi_dispatch(&mut self, params: &Params, _: &[u8], _: bool, action: char) {
        let amount = Self::param(params, 0, 1);
        match action {
            'A' => self.cursor_row = self.cursor_row.saturating_sub(amount),
            'B' => self.cursor_row = (self.cursor_row + amount).min(self.rows - 1),
            'C' => self.cursor_col = (self.cursor_col + amount).min(self.cols - 1),
            'D' => self.cursor_col = self.cursor_col.saturating_sub(amount),
            'G' => self.cursor_col = amount.saturating_sub(1).min(self.cols - 1),
            'H' | 'f' => {
                self.cursor_row = Self::param(params, 0, 1)
                    .saturating_sub(1)
                    .min(self.rows - 1);
                self.cursor_col = Self::param(params, 1, 1)
                    .saturating_sub(1)
                    .min(self.cols - 1);
            }
            'J' => self.erase_display(Self::param(params, 0, 0)),
            'K' => self.erase_line(Self::param(params, 0, 0)),
            'm' => self.sgr(params),
            'r' => {
                self.scroll_top = Self::param(params, 0, 1)
                    .saturating_sub(1)
                    .min(self.rows - 1);
                self.scroll_bottom = Self::param(params, 1, self.rows)
                    .saturating_sub(1)
                    .min(self.rows - 1);
                self.cursor_col = 0;
                self.cursor_row = self.scroll_top;
            }
            's' => self.saved_cursor = (self.cursor_col, self.cursor_row),
            'u' => (self.cursor_col, self.cursor_row) = self.saved_cursor,
            'h' | 'l' => {
                if action == 'l'
                    && params
                        .iter()
                        .any(|parameter| parameter.first() == Some(&25))
                {
                    self.cursor_visible = false;
                }
                if action == 'h'
                    && params
                        .iter()
                        .any(|parameter| parameter.first() == Some(&25))
                {
                    self.cursor_visible = true;
                }
            }
            _ => {}
        }
    }
    fn esc_dispatch(&mut self, _: &[u8], _: bool, byte: u8) {
        match byte {
            b'7' => self.saved_cursor = (self.cursor_col, self.cursor_row),
            b'8' => (self.cursor_col, self.cursor_row) = self.saved_cursor,
            b'D' => self.linefeed(),
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
