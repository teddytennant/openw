//! The transcript as rows: the header, then one block per message, tool call or notice with a
//! blank row before each. Finished reads, searches and commands fold into one summary row; the
//! detailed view (ctrl+o) shows every call with its output.

use agent_core::{NoticeLevel, TodoStatus, ToolCall, ToolKind, ToolStatus};
use ratatui::style::{Modifier, Style};

use crate::app::{clock_text, App, Block, DONE_VERBS, VERSION};
use crate::text::{wrap, wrap_str, Row, Span};
use crate::theme::Theme;
use crate::ui::diff;
use crate::ui::prompt::fmt_secs;

/// The Wizard mark: a hat, in the nine columns Claude Code's mascot takes.
pub const MARK: [&str; 3] = ["    ▟▙   ", "   ▟██▙  ", " ▝▀▀▀▀▀▀▘"];

pub fn rows(app: &App, w: usize, detail: bool) -> Vec<Row> {
    let th = app.theme;
    let mut out = header(app, w);
    let blocks = &app.blocks;
    let mut i = 0;
    while i < blocks.len() {
        match &blocks[i] {
            Block::User { text, bash } => {
                out.push(Row::new());
                out.extend(user(&th, text, *bash, w));
            }
            Block::Output { text, accent, dim } => out.extend(output(&th, text, accent, *dim, w)),
            Block::Assistant { text, time } => {
                if text.trim().is_empty() {
                    i += 1;
                    continue;
                }
                out.push(Row::new());
                if detail {
                    let stamp = format!("{} {}", clock_text(*time, true), app.cfg.model);
                    let pad = w.saturating_sub(stamp.chars().count() + 8);
                    out.push(Row::of(" ".repeat(pad), Style::new()).with(stamp, th.dim()));
                }
                out.extend(assistant(&th, text, w));
            }
            Block::Thought(text) => {
                if detail && !text.trim().is_empty() {
                    out.push(Row::new());
                    let st = th.dim().add_modifier(Modifier::ITALIC);
                    out.push(Row::of("✻ Thinking…", st));
                    for r in wrap_str(text.trim(), st, w.saturating_sub(4)) {
                        out.push(Row::of("  ", Style::new()).with(r.text(), st));
                    }
                }
            }
            Block::Tool(_) => {
                let mut j = i;
                let mut group: Vec<&ToolCall> = Vec::new();
                while let Some(Block::Tool(t)) = blocks.get(j) {
                    if detail || !folds(t) {
                        break;
                    }
                    group.push(t);
                    j += 1;
                }
                if group.is_empty() {
                    let Block::Tool(t) = &blocks[i] else {
                        unreachable!()
                    };
                    out.push(Row::new());
                    out.extend(tool(app, t, w, detail));
                } else {
                    out.push(Row::new());
                    out.extend(folded(&th, &group, w));
                    i = j - 1;
                }
            }
            Block::Todos(todos) => {
                if app.show_todos && !todos.is_empty() {
                    out.push(Row::new());
                    out.push(Row::of("● ", th.fg(th.success)).with("Update Todos", th.bold()));
                    for (n, t) in todos.iter().enumerate() {
                        let lead = if n == 0 { ELBOW } else { "     " };
                        let (mark, st) = match t.status {
                            TodoStatus::Completed => {
                                ("☒ ", th.dim().add_modifier(Modifier::CROSSED_OUT))
                            }
                            TodoStatus::InProgress => ("☐ ", th.bold()),
                            TodoStatus::Pending => ("☐ ", Style::new()),
                        };
                        out.push(
                            Row::of(lead, th.dim())
                                .with(mark, Style::new())
                                .with(t.text.clone(), st),
                        );
                    }
                }
            }
            Block::TurnEnd { verb, secs, time } => {
                out.push(Row::new());
                out.push(Row::of(
                    format!(
                        "✻ {} for {} · done {}",
                        DONE_VERBS[*verb % DONE_VERBS.len()],
                        fmt_secs(*secs),
                        clock_text(*time, false)
                    ),
                    th.dim(),
                ));
            }
            Block::Notice { level, text } => {
                let color = match level {
                    NoticeLevel::Info => th.secondary,
                    NoticeLevel::Warn => th.warning,
                    NoticeLevel::Error => th.error,
                };
                out.extend(hanging(&th, text, th.fg(color), w));
            }
        }
        i += 1;
    }
    let asking = matches!(app.dialog, Some(crate::app::Dialog::Permission { .. }));
    if app.busy.is_some() && !asking {
        out.push(Row::new());
    }
    out
}

fn header(app: &App, w: usize) -> Vec<Row> {
    let th = app.theme;
    let brand = th.fg(th.brand);
    let mut model = app.model_name();
    if !app.cfg.effort.is_empty() && app.cfg.effort != "medium" {
        model.push_str(&format!(" with {} effort", app.cfg.effort));
    }
    let provider = app.provider();
    if !provider.is_empty() {
        model.push_str(&format!(" · {provider}"));
    }
    let room = w.saturating_sub(11);
    vec![
        Row::new(),
        Row::of(MARK[0], brand)
            .with("  ", Style::new())
            .with("Wizard", th.bold())
            .with(" ", Style::new())
            .with(format!("v{VERSION}"), th.dim()),
        Row::of(MARK[1], brand)
            .with("  ", Style::new())
            .with(crate::text::truncate(&model, room), th.dim()),
        Row::of(MARK[2], brand)
            .with("  ", Style::new())
            .with(short_cwd(&app.opts.cwd.to_string_lossy(), w), th.dim()),
        Row::new(),
    ]
}

/// The working directory for the header: `~` for home, and leading components dropped behind
/// `/…/` until it fits.
pub fn short_cwd(cwd: &str, w: usize) -> String {
    let mut path = cwd.to_string();
    if let Some(home) = std::env::var_os("HOME").and_then(|h| h.into_string().ok()) {
        if !home.is_empty() && (path == home || path.starts_with(&format!("{home}/"))) {
            path = format!("~{}", &path[home.len()..]);
        }
    }
    let limit = w.saturating_sub(14);
    if path.chars().count() <= limit {
        return path;
    }
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    for skip in 1..parts.len() {
        let cand = format!("/…/{}", parts[skip..].join("/"));
        if cand.chars().count() <= limit {
            return cand;
        }
    }
    format!("/…/{}", parts.last().copied().unwrap_or(""))
}

/// Inline markdown: `code`, **bold** and *italic*. Anything unbalanced is left as typed.
pub fn inline(th: &Theme, text: &str, base: Style, code_only: bool) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut plain = String::new();
    let chars: Vec<char> = text.chars().collect();
    let find = |from: usize, pat: &[char]| -> Option<usize> {
        (from..chars.len().saturating_sub(pat.len() - 1)).find(|&k| chars[k..k + pat.len()] == *pat)
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let (open, style): (&[char], Style) = if c == '`' {
            (&['`'], base.fg(th.suggestion))
        } else if code_only {
            (&[], base)
        } else if c == '*' && chars.get(i + 1) == Some(&'*') {
            (&['*', '*'], base.add_modifier(Modifier::BOLD))
        } else if c == '*' && chars.get(i + 1).is_some_and(|n| !n.is_whitespace()) {
            (&['*'], base.add_modifier(Modifier::ITALIC))
        } else {
            (&[], base)
        };
        if !open.is_empty() {
            if let Some(end) = find(i + open.len(), open) {
                if end > i + open.len() && !chars[i + open.len()..end].contains(&'\n') {
                    if !plain.is_empty() {
                        out.push((std::mem::take(&mut plain), base));
                    }
                    out.push((chars[i + open.len()..end].iter().collect(), style));
                    i = end + open.len();
                    continue;
                }
            }
        }
        plain.push(c);
        i += 1;
    }
    if !plain.is_empty() {
        out.push((plain, base));
    }
    out
}

fn user(th: &Theme, text: &str, bash: bool, w: usize) -> Vec<Row> {
    let bg = Style::new().bg(if bash { th.bash_bg } else { th.user_bg });
    let body = bg.fg(th.user_fg);
    let spans = inline(th, text.trim_end(), body, true);
    let lines = wrap(&spans, w.saturating_sub(4));
    let last = lines.len() - 1;
    lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let mut row = match (i, bash) {
                (0, true) => Row::of("! ", bg.fg(th.bash)),
                (0, false) => Row::of("❯", bg.fg(th.secondary)).with(" ", bg),
                _ => Row::of("  ", bg),
            };
            row.extend(line);
            if i == last && !bash && text.ends_with(' ') && text.starts_with('/') {
                row.push(" ", bg);
            }
            row
        })
        .collect()
}

fn assistant(th: &Theme, text: &str, w: usize) -> Vec<Row> {
    let width = w.saturating_sub(3);
    let mut body: Vec<Row> = Vec::new();
    let mut fence = false;
    for line in text.trim().lines() {
        let t = line.trim_end();
        if t.trim_start().starts_with("```") {
            fence = !fence;
            continue;
        }
        if fence {
            body.push(Row::of(t.to_string(), Style::new()));
            continue;
        }
        let (lead, rest, base) = if let Some(h) = t.strip_prefix("# ") {
            let st =
                Style::new().add_modifier(Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED);
            ("", h, st)
        } else if let Some(h) = t.strip_prefix("## ").or_else(|| t.strip_prefix("### ")) {
            ("", h, Style::new().add_modifier(Modifier::BOLD))
        } else if let Some(q) = t.strip_prefix("> ") {
            ("▎ ", q, th.dim().add_modifier(Modifier::ITALIC))
        } else {
            ("", t, Style::new())
        };
        let hang = list_indent(rest);
        for (k, r) in wrap(
            &inline(th, rest, base, false),
            width.saturating_sub(hang).max(1),
        )
        .into_iter()
        .enumerate()
        {
            let pad = if k == 0 {
                String::new()
            } else {
                " ".repeat(hang)
            };
            let mut row = Row::of(lead, th.dim()).with(pad, Style::new());
            row.extend(r);
            body.push(row);
        }
    }
    body.into_iter()
        .enumerate()
        .map(|(i, r)| {
            let mut row = if i == 0 {
                Row::of("●", th.fg(th.bullet)).with(" ", Style::new())
            } else {
                Row::of("  ", Style::new())
            };
            row.extend(r);
            row
        })
        .collect()
}

/// How far a list item's wrapped rows hang under its marker.
fn list_indent(line: &str) -> usize {
    let t = line.trim_start();
    let lead = line.len() - t.len();
    if t.starts_with("- ") || t.starts_with("* ") {
        return lead + 2;
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && t[digits..].starts_with(". ") {
        return lead + digits + 2;
    }
    0
}

/// The elbow a tool's result hangs from. The last cell is a no-break space, as Claude Code
/// writes it.
pub const ELBOW: &str = "  ⎿ \u{a0}";

/// `  ⎿  text`, later lines under the text.
fn hanging(th: &Theme, text: &str, style: Style, w: usize) -> Vec<Row> {
    wrap_str(text.trim_end(), style, w.saturating_sub(10))
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let mut row = if i == 0 {
                Row::of(ELBOW, th.dim())
            } else {
                Row::of("     ", Style::new())
            };
            row.extend(r);
            row
        })
        .collect()
}

fn output(th: &Theme, text: &str, accent: &str, dim: bool, w: usize) -> Vec<Row> {
    let base = if dim {
        Style::new().add_modifier(Modifier::DIM)
    } else {
        Style::new()
    };
    let mut spans: Vec<Span> = Vec::new();
    match text.find(accent) {
        Some(at) if !accent.is_empty() => {
            spans.push((text[..at].to_string(), base));
            spans.push((accent.to_string(), base.fg(th.suggestion)));
            spans.push((text[at + accent.len()..].to_string(), base));
        }
        _ => spans.push((text.to_string(), base)),
    }
    wrap(&spans, w.saturating_sub(7))
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let mut row = if i == 0 {
                Row::of("  ⎿", th.dim()).with("  ", Style::new())
            } else {
                Row::of("     ", Style::new())
            };
            row.extend(r);
            row
        })
        .collect()
}

fn folds(t: &ToolCall) -> bool {
    let done = matches!(t.status, ToolStatus::Completed | ToolStatus::Failed);
    let local = t.input.get("local").is_some();
    done && !local && t.kind != ToolKind::Edit && t.diff.is_none()
}

/// What a shell command does, for the folded summary: `find` and `grep` search, `cat` reads.
fn shell_kind(t: &ToolCall) -> ToolKind {
    let cmd = input_str(t, &["command", "cmd", "script"]).unwrap_or("");
    let first = cmd.split_whitespace().next().unwrap_or("");
    let first = first.rsplit('/').next().unwrap_or(first);
    match first {
        "find" | "grep" | "rg" | "ag" | "ack" | "fd" | "egrep" | "fgrep" => ToolKind::Search,
        "cat" | "head" | "tail" | "less" | "more" | "bat" => ToolKind::Read,
        _ => ToolKind::Execute,
    }
}

fn bucket(t: &ToolCall) -> (&'static str, &'static str, &'static str) {
    let kind = if t.kind == ToolKind::Execute {
        shell_kind(t)
    } else {
        t.kind
    };
    match kind {
        ToolKind::Execute => ("ran", "shell command", "shell commands"),
        ToolKind::Search => ("searched for", "pattern", "patterns"),
        ToolKind::Read => ("read", "file", "files"),
        ToolKind::Fetch => ("fetched", "URL", "URLs"),
        _ => ("used", "tool", "tools"),
    }
}

/// `  Searched for 1 pattern, read 1 file`: one phrase per kind, in the order first used.
fn folded(th: &Theme, group: &[&ToolCall], w: usize) -> Vec<Row> {
    let mut counts: Vec<((&str, &str, &str), usize)> = Vec::new();
    for t in group {
        let b = bucket(t);
        match counts.iter_mut().find(|(k, _)| *k == b) {
            Some((_, n)) => *n += 1,
            None => counts.push((b, 1)),
        }
    }
    let st = th.dim();
    let mut spans: Vec<Span> = Vec::new();
    for (i, ((verb, one, many), n)) in counts.iter().enumerate() {
        let mut verb = verb.to_string();
        if i == 0 {
            verb[..1].make_ascii_uppercase();
        } else {
            spans.push((", ".into(), st));
        }
        spans.push((format!("{verb} "), st));
        spans.push((n.to_string(), st.add_modifier(Modifier::BOLD)));
        spans.push((format!(" {}", if *n == 1 { one } else { many }), st));
    }
    wrap(&spans, w.saturating_sub(4))
        .into_iter()
        .map(|r| {
            let mut row = Row::of("  ", Style::new());
            row.extend(r);
            row
        })
        .collect()
}

fn input_str<'a>(t: &'a ToolCall, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| t.input.get(*k).and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
}

/// The tool's display name and argument, as in `Bash(echo hi)`.
pub fn call_label(app: &App, t: &ToolCall, absolute: bool) -> (String, String) {
    let path = |p: &str| -> String {
        if absolute {
            return p.to_string();
        }
        let cwd = app.opts.cwd.to_string_lossy();
        p.strip_prefix(&format!("{cwd}/")).unwrap_or(p).to_string()
    };
    let file = t
        .diff
        .as_ref()
        .map(|d| d.path.as_str())
        .or_else(|| input_str(t, &["path", "file_path", "file", "filePath"]));
    match t.kind {
        ToolKind::Execute => (
            "Bash".into(),
            input_str(t, &["command", "cmd", "script"])
                .unwrap_or(&t.title)
                .to_string(),
        ),
        ToolKind::Read => ("Read".into(), path(file.unwrap_or(&t.title))),
        ToolKind::Edit => {
            let new_file = t.diff.as_ref().is_some_and(|d| d.old.is_none());
            (
                if new_file { "Write" } else { "Update" }.into(),
                path(file.unwrap_or(&t.title)),
            )
        }
        ToolKind::Search => (
            "Search".into(),
            input_str(t, &["pattern", "query", "regex"])
                .unwrap_or(&t.title)
                .to_string(),
        ),
        ToolKind::Fetch => (
            "Fetch".into(),
            input_str(t, &["url", "query"])
                .unwrap_or(&t.title)
                .to_string(),
        ),
        _ => (t.name.clone(), t.title.clone()),
    }
}

fn tool(app: &App, t: &ToolCall, w: usize, detail: bool) -> Vec<Row> {
    let th = app.theme;
    let running = matches!(t.status, ToolStatus::Pending | ToolStatus::Running);
    let dot = match t.status {
        ToolStatus::Completed => th.success,
        ToolStatus::Failed => th.error,
        _ => th.secondary,
    };
    let (name, arg) = call_label(app, t, detail);
    let command = input_str(t, &["command", "cmd", "script"]);
    let mut out: Vec<Row> = Vec::new();
    let editing = t.kind == ToolKind::Edit || t.diff.is_some();

    if running && !editing && !detail {
        let title = if t.title.is_empty() {
            format!("{name}({arg})")
        } else {
            t.title.clone()
        };
        out.push(
            Row::of("●", th.fg(dot))
                .with(" ", Style::new())
                .with(title, Style::new()),
        );
        if let Some(c) = command {
            out.push(Row::of(format!("{ELBOW}$ {c}"), th.dim()));
        }
        return out;
    }

    // the arguments wrap in a column that starts right after the name
    let col = 2 + crate::text::width(&name);
    let args = if arg.is_empty() {
        Vec::new()
    } else {
        wrap_str(&format!("({arg})"), Style::new(), w.saturating_sub(col))
    };
    let mut first = Row::of("●", th.fg(dot))
        .with(" ", Style::new())
        .with(name, th.bold());
    let mut args = args.into_iter();
    if let Some(r) = args.next() {
        first.extend(r);
    }
    out.push(first);
    for r in args {
        let mut row = Row::of(" ".repeat(col), Style::new());
        row.extend(r);
        out.push(row);
    }
    if running {
        return out;
    }
    if t.status == ToolStatus::Failed {
        let text = t.output.as_deref().unwrap_or("failed").trim();
        let text = if text == agent_core::INTERRUPTED_OUTPUT {
            "Interrupted"
        } else {
            text
        };
        out.extend(hanging(&th, &format!("Error: {text}"), th.fg(th.error), w));
        return out;
    }
    if let Some(d) = &t.diff {
        let rendered = diff::render(&th, d, 5, w);
        let mut row = Row::of(ELBOW, th.dim());
        row.0.extend(diff::summary(&rendered, d.old.is_none(), &th));
        out.push(row);
        out.extend(rendered.rows);
        return out;
    }
    let text = t.output.as_deref().unwrap_or("").trim_end();
    match t.kind {
        ToolKind::Read => {
            // counted the way Claude Code counts: a final newline starts one more line
            let n = t.output.as_deref().unwrap_or("").split('\n').count();
            out.push(
                Row::of(ELBOW, th.dim())
                    .with("Read ", Style::new())
                    .with(n.to_string(), th.bold())
                    .with(if n == 1 { " line" } else { " lines" }, Style::new()),
            );
        }
        _ if text.is_empty() => out.extend(hanging(&th, "(No output)", Style::new(), w)),
        _ => {
            let lines: Vec<&str> = text.lines().collect();
            let shown = if detail {
                lines.len()
            } else {
                lines.len().min(3)
            };
            let mut body = lines[..shown].join("\n");
            if shown < lines.len() {
                body.push_str(&format!(
                    "\n… +{} lines (ctrl+o to expand)",
                    lines.len() - shown
                ));
            }
            out.extend(hanging(&th, &body, Style::new(), w));
        }
    }
    out
}
