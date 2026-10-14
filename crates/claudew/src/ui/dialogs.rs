//! The panels that take the prompt's place: help, the model picker, status and the session
//! picker. A permission prompt is not one of them; it flows at the end of the transcript.

use agent_core::{PermissionRequest, ToolKind};
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

use crate::app::{effort_glyph, permission_options, App, Dialog, VERSION};
use crate::text::{blit, put, truncate, width, wrap, wrap_str, Row, Span};
use crate::ui::{diff, prompt};

const MODEL_ROWS: usize = 10;
/// Rows from which the help panel spaces its parts out, as the reference does at 50 rows and
/// not at 40.
const ROOMY_HELP: usize = 48;

struct Panel {
    top: Vec<Row>,
    /// Rows held against the bottom edge.
    bottom: Vec<Row>,
    /// Rows left for the transcript when the panel takes all the height it can.
    fill: Option<u16>,
}

/// Draws the dialog against the bottom edge and returns its first row.
pub fn draw(buf: &mut Buffer, app: &mut App, dialog: &Dialog) -> u16 {
    let (w, h) = (buf.area.width, buf.area.height);
    let th = app.theme;
    let panel = match dialog {
        Dialog::Help { tab, sel, focus } => help(app, *tab, *sel, *focus, w as usize, h as usize),
        Dialog::Model { sel } => model(app, *sel, w as usize, h as usize),
        Dialog::Status => status(app, w as usize),
        Dialog::Resume { sel, query } => resume(app, *sel, query, w as usize, h as usize),
        Dialog::Permission { .. } => return h,
    };
    let need = (panel.top.len() + panel.bottom.len() + 1) as u16;
    let height = match panel.fill {
        Some(keep) => h.saturating_sub(keep),
        None => need.min(h.saturating_sub(2)),
    }
    .max(2)
    .min(h);
    let top = h - height;
    put(buf, 0, top, &"▔".repeat(w as usize), th.fg(th.suggestion));
    if let Some(n) = &app.notice {
        let x = (w as usize).saturating_sub(width(n) + 3);
        put(buf, x as u16, top, &format!(" {n} "), th.dim());
    }
    let room = height as usize - 1;
    if panel.fill.is_some() {
        let bottom_n = panel.bottom.len().min(room);
        for (i, row) in panel.top.iter().take(room - bottom_n).enumerate() {
            blit(buf, 0, top + 1 + i as u16, row);
        }
        for (i, row) in panel.bottom.iter().rev().take(bottom_n).enumerate() {
            blit(buf, 0, h - 1 - i as u16, row);
        }
    } else {
        // a panel taller than the room it gets is cut at the bottom
        for (i, row) in panel.top.iter().chain(&panel.bottom).take(room).enumerate() {
            blit(buf, 0, top + 1 + i as u16, row);
        }
    }
    top
}

fn indent(row: Row, n: usize) -> Row {
    let mut out = Row::of(" ".repeat(n), Style::new());
    out.extend(row);
    out
}

fn tabs(app: &App, title: &str, names: &[&str], on: usize, color: Color, focus: bool) -> Row {
    let th = app.theme;
    let mut row =
        Row::of("   ", Style::new()).with(title, th.fg(color).add_modifier(Modifier::BOLD));
    for (i, name) in names.iter().enumerate() {
        row.push(" ", Style::new());
        if i == on {
            let st = if focus {
                Style::new().add_modifier(Modifier::REVERSED)
            } else {
                Style::new().bg(color)
            };
            let text = if focus {
                st.add_modifier(Modifier::BOLD)
            } else {
                st.fg(th.tab_fg).add_modifier(Modifier::BOLD)
            };
            row.push(" ", st);
            row.push(*name, text);
            row.push(" ", st);
        } else {
            row.push(format!(" {name} "), Style::new());
        }
    }
    row
}

const HELP_BLUE: Color = Color::Indexed(110);

fn help(app: &App, tab: usize, sel: usize, focus: bool, w: usize, h: usize) -> Panel {
    let th = app.theme;
    let mut top = vec![
        tabs(app, "Help", &["General", "Commands"], tab, HELP_BLUE, focus),
        Row::new(),
    ];
    let bottom = vec![
        Row::of(
            "   For more help: https://github.com/teddytennant/wizard",
            Style::new(),
        ),
        Row::new(),
        Row::of("   Esc to cancel", th.dim().add_modifier(Modifier::ITALIC)),
    ];
    if tab == 0 {
        // a tall terminal gets a blank row around each part
        let roomy = h >= ROOMY_HELP;
        if roomy {
            top.push(Row::new());
        }
        let about = "Wizard understands your codebase, makes edits with your permission, and \
                     executes commands — right from your terminal.";
        for r in wrap_str(about, Style::new(), w.saturating_sub(6)) {
            top.push(indent(r, 3));
        }
        if roomy {
            top.push(Row::new());
            top.push(Row::new());
        }
        top.push(Row::of("   ", Style::new()).with("Shortcuts", th.bold()));
        top.extend(prompt::shortcut_table(app, w, true, Style::new()));
        top.push(Row::new());
        let mut bottom = bottom;
        if roomy {
            top.push(Row::new());
            bottom.insert(1, Row::new());
        }
        return Panel {
            top,
            bottom,
            fill: None,
        };
    }
    top.push(Row::new());
    top.push(Row::of("   Browse default commands", Style::new()));
    top.push(Row::new());
    let room = h.saturating_sub(3 + 1 + top.len() + bottom.len() + 4);
    let shown = (room / 2).max(1);
    let commands = sorted_commands(app);
    let n = commands.len();
    let first = if sel >= shown { sel + 1 - shown } else { 0 };
    for (i, cmd) in commands.iter().enumerate().skip(first).take(shown) {
        let on = focus && i == sel;
        let name = format!("/{}", cmd.name);
        let row = if on {
            Row::of("   ❯ ", th.fg(th.suggestion)).with(name, th.fg(th.suggestion))
        } else if i == sel {
            Row::of("   ❯ ", th.dim()).with(name, Style::new())
        } else {
            Row::of("     ", Style::new()).with(name, Style::new())
        };
        top.push(row);
        top.push(Row::of(
            format!("     {}", truncate(&cmd.description, w.saturating_sub(10))),
            th.dim(),
        ));
    }
    top.push(Row::new());
    let below = n.saturating_sub(first + shown);
    if below > 0 {
        top.push(Row::of(format!("     ↓ {below} more"), th.dim()));
    } else {
        top.push(Row::new());
    }
    top.push(Row::new());
    top.push(Row::new());
    Panel {
        top,
        bottom,
        fill: Some(3),
    }
}

/// The commands as the help panel lists them: by name.
pub fn sorted_commands(app: &App) -> Vec<&agent_core::SlashCommand> {
    let mut v: Vec<_> = app.commands.iter().collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

fn model(app: &App, sel: usize, w: usize, h: usize) -> Panel {
    let th = app.theme;
    let models = &app.cfg.models;
    let mut top = vec![Row::of("   ", Style::new())
        .with("Select model", th.fg(th.title).add_modifier(Modifier::BOLD))];
    let about = "Switch between Wizard models. Your pick applies to this session only. For \
                 other/previous model names, specify with --model.";
    for r in wrap_str(about, th.dim(), w.saturating_sub(6)) {
        top.push(indent(r, 3));
    }
    top.push(Row::new());
    // the panel takes at most half the screen and a row; the list gives way first
    let effort_rows = if app.cfg.effort.is_empty() { 0 } else { 2 };
    let fixed = 1 + top.len() + 1 + 1 + effort_rows + 1;
    let rows = (h / 2 + 1).saturating_sub(fixed).clamp(1, MODEL_ROWS);
    let shown = rows;
    let name_w = models.iter().map(|m| width(&m.name)).max().unwrap_or(0) + 2;
    let first = if sel >= shown { sel + 1 - shown } else { 0 };
    let last = (first + shown).min(models.len());
    for (i, m) in models.iter().enumerate().take(last).skip(first) {
        let current = m.id == app.cfg.model;
        let more_below = i + 1 == last && last < models.len();
        let more_above = i == first && first > 0;
        let mut row = Row::new();
        if i == sel {
            row.push("   ❯ ", th.fg(th.suggestion));
        } else if more_below {
            row.push("   ↓ ", th.dim());
        } else if more_above {
            row.push("   ↑ ", th.dim());
        } else {
            row.push("     ", Style::new());
        }
        row.push(format!("{:<4}", format!("{}.", i + 1)), th.dim());
        let label = if current {
            format!("{} ✔", m.name)
        } else {
            m.name.clone()
        };
        let style = if current {
            th.fg(th.success)
        } else if i == sel {
            th.fg(th.suggestion)
        } else {
            Style::new()
        };
        let pad = name_w.saturating_sub(width(&label));
        row.push(label, style);
        if !m.provider.is_empty() {
            row.push(" ".repeat(pad), Style::new());
            row.push(
                truncate(&m.provider, w.saturating_sub(9 + name_w + 2)),
                th.dim(),
            );
        }
        top.push(row);
    }
    if last < models.len() {
        top.push(Row::of(
            format!("      … +{} models", models.len() - last),
            th.dim(),
        ));
    }
    top.push(Row::new());
    if !app.cfg.effort.is_empty() {
        let e = &app.cfg.effort;
        let mut name = e.clone();
        name[..1].make_ascii_uppercase();
        let mut row = Row::of("   ", Style::new())
            .with(effort_glyph(e), th.fg(th.brand))
            .with(format!(" {name} effort"), th.dim());
        if app.cfg.efforts.len() > 1 {
            row.push(" ←/→ to adjust", th.fg(th.subtle));
        }
        top.push(row);
        top.push(Row::new());
    }
    Panel {
        top,
        bottom: vec![Row::of("   Enter to confirm · Esc to cancel", th.dim())],
        fill: None,
    }
}

fn status(app: &App, _w: usize) -> Panel {
    let th = app.theme;
    let mut top = vec![
        tabs(app, "Settings", &["Status"], 0, th.suggestion, false),
        Row::new(),
    ];
    let model = format!("{} ({})", app.model_name(), app.cfg.model);
    let fields: Vec<(&str, String)> = vec![
        ("Version:", VERSION.to_string()),
        ("Session ID:", app.session_id.clone()),
        ("cwd:", app.opts.cwd.to_string_lossy().into_owned()),
        ("Model:", model),
        ("Effort:", app.cfg.effort.clone()),
        ("Mode:", app.cfg.mode.clone()),
        ("Backend:", app.cfg.backend.clone()),
    ];
    for (label, value) in fields {
        if value.is_empty() {
            continue;
        }
        top.push(
            Row::of("   ", Style::new())
                .with(format!("{label:<19}"), th.bold())
                .with(value, Style::new()),
        );
    }
    top.push(Row::new());
    top.push(Row::of("   Esc to cancel", th.dim()));
    Panel {
        top,
        bottom: Vec::new(),
        fill: Some(2),
    }
}

pub fn ago(now: i64, then: i64) -> String {
    let s = (now - then).max(0);
    let plural = |n: i64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    if then <= 0 {
        String::new()
    } else if s < 60 {
        plural(s.max(1), "second")
    } else if s < 3600 {
        plural(s / 60, "minute")
    } else if s < 86400 {
        plural(s / 3600, "hour")
    } else {
        plural(s / 86400, "day")
    }
}

fn resume(app: &App, sel: usize, query: &str, w: usize, h: usize) -> Panel {
    let th = app.theme;
    let dim = Style::new().add_modifier(Modifier::DIM);
    let inner = w.saturating_sub(8);
    let list = app.resume_list(query);
    let footer: Vec<Row> = wrap_str(
        "Ctrl+A to show all projects · Type to search · Esc to cancel",
        th.dim(),
        w.saturating_sub(8),
    )
    .into_iter()
    .map(|r| indent(r, 5))
    .collect();
    // the transcript keeps six rows; the list gets what is left, in whole entries
    let room = h.saturating_sub(6 + 7 + footer.len());
    let shown = (room / 3).max(1);
    let mut title = Row::of("   ", Style::new()).with(
        "Resume session",
        th.fg(th.title).add_modifier(Modifier::BOLD),
    );
    if list.len() > shown {
        title.push(
            format!(" ({} of {})", sel.min(list.len() - 1) + 1, list.len()),
            th.dim().add_modifier(Modifier::BOLD),
        );
    }
    let mut top = vec![title, Row::of(format!("   ╭{}╮", "─".repeat(inner)), dim)];
    let mut search = Row::of("   │", dim).with(" ", Style::new());
    if query.is_empty() {
        search.push("⌕ Search…", th.dim());
    } else {
        search.push(format!("⌕ {query}"), Style::new());
    }
    let pad = (inner + 4).saturating_sub(search.width());
    search.push(" ".repeat(pad), Style::new());
    search.push("│", dim);
    top.push(search);
    top.push(Row::of(format!("   ╰{}╯", "─".repeat(inner)), dim));
    let scope = if app.resume_all {
        "all projects".to_string()
    } else {
        app.opts
            .cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    top.push(Row::of(format!("     {scope}"), th.dim()));
    top.push(Row::new());
    if list.is_empty() {
        top.push(Row::of(
            "    No conversations found in this project.",
            th.dim(),
        ));
        top.push(Row::of("    Ctrl+A to show all projects", th.dim()));
        top.push(Row::new());
        return Panel {
            top,
            bottom: footer,
            fill: None,
        };
    }
    let sel = sel.min(list.len() - 1);
    let first = if sel >= shown { sel + 1 - shown } else { 0 };
    let last = (first + shown).min(list.len());
    let now = app.now_unix;
    for (i, s) in list.iter().enumerate().take(last).skip(first) {
        let on = i == sel;
        let title = if s.title.is_empty() {
            s.id.chars().take(8).collect()
        } else {
            s.title.clone()
        };
        let title = truncate(&title, w.saturating_sub(10));
        let lead = if on {
            Row::of("   ❯ ", th.fg(th.suggestion))
        } else if i + 1 == last && last < list.len() {
            Row::of("   ↓ ", th.dim())
        } else if i == first && first > 0 {
            Row::of("   ↑ ", th.dim())
        } else {
            Row::of("     ", Style::new())
        };
        let style = if on {
            th.fg(th.suggestion)
        } else {
            Style::new()
        };
        top.push(lead.with(title, style));
        top.push(Row::of(format!("     {}", ago(now, s.updated)), th.dim()));
        top.push(Row::new());
    }
    let height = top.len() + footer.len();
    top.truncate(height.min(h).saturating_sub(footer.len()));
    Panel {
        top,
        bottom: footer,
        fill: None,
    }
}

/// A permission prompt as rows for the end of the transcript.
pub fn permission(app: &App, req: &PermissionRequest, sel: usize, w: usize) -> Vec<Row> {
    let th = app.theme;
    let accent = th.fg(th.suggestion);
    let mut rows = vec![Row::of("─".repeat(w), accent)];
    let name = req
        .diff
        .as_ref()
        .map(|d| d.path.clone())
        .unwrap_or_else(|| req.title.clone());
    let cwd = app.opts.cwd.to_string_lossy();
    let short = name
        .strip_prefix(&format!("{cwd}/"))
        .unwrap_or(&name)
        .to_string();
    let question: Vec<Span> = match (&req.diff, req.kind) {
        (Some(d), _) => {
            let title = if d.old.is_none() {
                "Create file"
            } else {
                "Edit file"
            };
            rows.push(Row::of(" ", Style::new()).with(title, accent.add_modifier(Modifier::BOLD)));
            rows.push(Row::of(format!(" {short}"), th.dim()));
            rows.push(Row::of("╌".repeat(w), th.fg(th.subtle)));
            rows.extend(diff::render(&th, d, 0, w).rows);
            rows.push(Row::of("╌".repeat(w), th.fg(th.subtle)));
            let verb = if d.old.is_none() {
                "create"
            } else {
                "make this edit to"
            };
            vec![
                (format!(" Do you want to {verb} "), Style::new()),
                (short, th.bold()),
                ("?".into(), Style::new()),
            ]
        }
        (None, kind) => {
            let title = match kind {
                ToolKind::Execute => "Bash command".to_string(),
                ToolKind::Fetch => "Fetch".to_string(),
                _ => req.tool.clone(),
            };
            rows.push(Row::of(" ", Style::new()).with(title, accent.add_modifier(Modifier::BOLD)));
            rows.push(Row::new());
            let body = req
                .input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or(&req.title);
            for r in wrap_str(body, Style::new(), w.saturating_sub(6)) {
                rows.push(indent(r, 3));
            }
            rows.push(Row::new());
            vec![(" Do you want to proceed?".into(), Style::new())]
        }
    };
    rows.extend(wrap(&question, w.saturating_sub(1)));
    for (i, (label, _)) in permission_options(req).iter().enumerate() {
        let on = i == sel;
        let st = if on { accent } else { Style::new() };
        for (k, r) in wrap_str(label, st, w.saturating_sub(8))
            .into_iter()
            .enumerate()
        {
            let mut row = Row::new();
            if k > 0 {
                row.push("      ", Style::new());
            } else {
                row.push(if on { " ❯ " } else { "   " }, accent);
                row.push(format!("{}.", i + 1), th.dim());
                row.push(" ", Style::new());
            }
            row.extend(r);
            rows.push(row);
        }
    }
    rows.push(Row::new());
    rows.push(Row::of(" Esc to cancel", th.dim()));
    rows
}
