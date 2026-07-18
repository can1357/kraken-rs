use crate::{
    app::state::AppState,
    term::{Cell, TerminalColor},
    ui::{
        Color, FontFace, RADIUS_LG, Rect, Scene, Theme,
        action::{CursorHint, ResizeTarget, UiAction},
    },
};

const PADDING: f32 = 12.0;

pub(super) fn build(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    scene.rect(0, rect, scene.viewport(), theme.window);
    let splitter = Rect::new(rect.x, rect.y - 2.0, rect.width, 4.0);
    scene.rect(
        3,
        splitter,
        scene.viewport(),
        if splitter.contains(state.hover()) {
            theme.accent
        } else {
            theme.border
        },
    );
    scene.hit(rect, UiAction::FocusTerminal, CursorHint::Text, None);
    scene.hit(
        splitter,
        UiAction::BeginResize(ResizeTarget::TerminalPane),
        CursorHint::ResizeVertical,
        Some("Resize terminal"),
    );
    let Some(snapshot) = state.terminal_snapshot() else {
        scene.text(
            "Terminal unavailable",
            [rect.x + PADDING, rect.y + PADDING],
            rect,
            theme.text_muted,
            12.0,
            16.0,
            FontFace::Sans,
        );
        return;
    };
    let font_size = f32::from(state.settings.terminal_font_size.max(8));
    let cell_width = font_size * 0.6;
    let cell_height = font_size * 1.2;
    let clip = Rect::new(
        rect.x + PADDING,
        rect.y + PADDING,
        (rect.width - PADDING * 2.0).max(0.0),
        (rect.height - PADDING * 2.0).max(0.0),
    );
    scene.rounded_rect(0, clip, rect, theme.input, theme.border, RADIUS_LG, 1.0);
    for row in 0..snapshot.rows {
        if row as f32 * cell_height >= clip.height {
            break;
        }
        let start = row * snapshot.cols;
        let end = (start + snapshot.cols).min(snapshot.cells.len());
        let cells = &snapshot.cells[start..end];
        for (column, cell) in cells.iter().enumerate() {
            let background = cell_color(cell.background, theme, theme.input);
            let foreground = cell_color(cell.foreground, theme, theme.text);
            let (background, _) = if cell.inverse {
                (foreground, background)
            } else {
                (background, foreground)
            };
            if background != theme.input {
                let cell_rect = Rect::new(
                    clip.x + column as f32 * cell_width,
                    clip.y + row as f32 * cell_height,
                    cell_width,
                    cell_height,
                );
                scene.rounded_rect(1, cell_rect, clip, background, background, 0.0, 0.0);
            }
        }
        draw_row(
            scene,
            cells,
            row,
            clip,
            cell_width,
            cell_height,
            font_size,
            theme,
        );
    }
    if snapshot.cursor_visible && !snapshot.exited && snapshot.cursor_row < snapshot.rows {
        let cursor = Rect::new(
            clip.x + snapshot.cursor_col as f32 * cell_width,
            clip.y + snapshot.cursor_row as f32 * cell_height + 1.0,
            2.0,
            (cell_height - 2.0).max(1.0),
        );
        scene.rounded_rect(2, cursor, clip, theme.accent, theme.accent, 0.0, 0.0);
    }
    if snapshot.exited {
        scene.text(
            "process exited — toggle Terminal to start a new shell",
            [clip.x + 8.0, clip.bottom() - cell_height],
            clip,
            theme.text_muted,
            font_size,
            cell_height,
            FontFace::Terminal,
        );
    }
}

fn draw_row(
    scene: &mut Scene,
    cells: &[Cell],
    row: usize,
    clip: Rect,
    cell_width: f32,
    cell_height: f32,
    font_size: f32,
    theme: &Theme,
) {
    let mut start = 0;
    while start < cells.len() {
        let cell = cells[start];
        let color = if cell.inverse {
            cell_color(cell.background, theme, theme.input)
        } else {
            cell_color(cell.foreground, theme, theme.text)
        };
        let mut end = start + 1;
        while end < cells.len() {
            let next = cells[end];
            let next_color = if next.inverse {
                cell_color(next.background, theme, theme.input)
            } else {
                cell_color(next.foreground, theme, theme.text)
            };
            if next_color != color {
                break;
            }
            end += 1;
        }
        let text: String = cells[start..end]
            .iter()
            .map(|cell| cell.character)
            .collect();
        scene.text(
            text,
            [
                clip.x + start as f32 * cell_width,
                clip.y + row as f32 * cell_height,
            ],
            clip,
            color,
            font_size,
            cell_height,
            FontFace::Terminal,
        );
        start = end;
    }
}

fn cell_color(color: TerminalColor, theme: &Theme, default: Color) -> Color {
    match color {
        TerminalColor::Default => default,
        TerminalColor::Rgb(red, green, blue) => Color::rgb(red, green, blue),
        TerminalColor::Indexed(index) if index < 16 => ansi_color(index, theme),
        TerminalColor::Indexed(index) if index < 232 => {
            let value = index - 16;
            let level = |component| {
                if component == 0 {
                    0
                } else {
                    55 + component * 40
                }
            };
            Color::rgb(level(value / 36), level((value / 6) % 6), level(value % 6))
        }
        TerminalColor::Indexed(index) => {
            let gray = 8 + (index - 232) * 10;
            Color::rgb(gray, gray, gray)
        }
    }
}

fn ansi_color(index: u8, theme: &Theme) -> Color {
    match index {
        0 => theme.text_dim,
        1 => theme.red,
        2 => theme.green,
        3 => theme.orange,
        4 => theme.accent_active,
        5 => theme.purple,
        6 => theme.accent,
        7 => theme.text_muted,
        8 => theme.text_disabled,
        9 => theme.red,
        10 => theme.green,
        11 => theme.orange,
        12 => theme.accent_hover,
        13 => theme.purple,
        14 => theme.accent_hover,
        _ => theme.text,
    }
}
