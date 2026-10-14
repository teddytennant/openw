//! Drawing. `draw` lays out one frame: the transcript on top, and under it either the prompt
//! box with its footer or a dialog. Ctrl+O swaps the whole screen for the detailed transcript.

pub mod dialogs;
pub mod diff;
pub mod prompt;
pub mod transcript;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::app::{App, Dialog};
use crate::text::{blit, put, Row};

pub fn draw(buf: &mut Buffer, app: &mut App) {
    buf.reset();
    app.cursor = None;
    let area = buf.area;
    if area.width < 4 || area.height < 2 {
        return;
    }
    if app.detail {
        return draw_detail(buf, app);
    }
    let w = area.width as usize;
    let mut rows = transcript::rows(app, w, false);
    let top = match app.dialog.clone() {
        Some(Dialog::Permission { req, sel }) => {
            rows.push(Row::new());
            rows.extend(dialogs::permission(app, &req, sel, w));
            area.height
        }
        Some(d) => dialogs::draw(buf, app, &d),
        None => prompt::draw(buf, app),
    };
    // the status panel sits under one blank row and the transcript's last
    let skip = u16::from(matches!(app.dialog, Some(Dialog::Status)) && top > 1);
    draw_rows(buf, app, &rows, Rect::new(0, skip, area.width, top - skip));
}

/// Draw the tail of `rows` into `area`, `app.scroll` rows up from the end.
fn draw_rows(buf: &mut Buffer, app: &mut App, rows: &[Row], area: Rect) {
    let view = area.height as usize;
    let max_scroll = rows.len().saturating_sub(view);
    app.scroll = app.scroll.min(max_scroll);
    app.last_view = (rows.len(), view);
    let start = max_scroll - app.scroll;
    for (i, row) in rows.iter().skip(start).take(view).enumerate() {
        blit(buf, area.x, area.y + i as u16, row);
    }
}

fn draw_detail(buf: &mut Buffer, app: &mut App) {
    let area = buf.area;
    let (w, h) = (area.width, area.height);
    let rows = transcript::rows(app, w as usize, true);
    draw_rows(buf, app, &rows, Rect::new(0, 0, w, h.saturating_sub(3)));
    let th = app.theme;
    put(
        buf,
        0,
        h - 2,
        &"─".repeat(w as usize),
        Style::new().add_modifier(Modifier::DIM),
    );
    if app.shortcuts {
        let table = prompt::shortcut_table(app, w as usize, false, th.dim());
        let top = (h - 2).saturating_sub(table.len() as u16);
        for (i, row) in table.iter().enumerate() {
            blit(buf, 0, top + i as u16, row);
        }
    }
    let mut hint = format!(
        "Showing detailed transcript · ctrl+o to toggle · ↑↓ scroll · v to open in {} · ? for shortcuts",
        crate::app::editor_name()
    );
    if crate::text::width(&hint) + 12 > w as usize {
        hint = "Showing detailed transcript · ctrl+o to toggle · ? for shortcuts".to_string();
    }
    put(buf, 2, h - 1, &hint, th.dim());
    put(buf, w.saturating_sub(8), h - 1, "verbose", th.dim());
}
