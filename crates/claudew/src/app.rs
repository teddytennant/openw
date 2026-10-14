//! App state, backend events and keys.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use agent_core::{
    Backend, BackendHandle, Config, DecideScope, Event, HistoryItem, NoticeLevel,
    PermissionRequest, Request, SessionInfo, SlashCommand, StopReason, Todo, ToolCall, ToolKind,
    ToolStatus, Usage,
};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tuikit::editor::{Editor, KeyOutcome};
use tuikit::term::{Event as TermEvent, EventPump, TermGuard, Terminal};

use crate::theme::Theme;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The Claude Code release this look reproduces.
pub const CLAUDE_VERSION: &str = "2.1.296";
pub const LOOK: &str = "claude";

pub const SPINNER_FRAMES: [&str; 6] = ["·", "✢", "*", "✶", "✻", "✽"];
pub const VERBS: [&str; 12] = [
    "Deliberating",
    "Sublimating",
    "Considering",
    "Imagining",
    "Combobulating",
    "Lollygagging",
    "Cogitating",
    "Brewing",
    "Pondering",
    "Simmering",
    "Conjuring",
    "Percolating",
];
pub const DONE_VERBS: [&str; 6] = [
    "Worked",
    "Brewed",
    "Baked",
    "Cogitated",
    "Sautéed",
    "Churned",
];
pub const PLACEHOLDERS: [&str; 4] = [
    "Try \"fix lint errors\"",
    "Try \"how does <filepath> work?\"",
    "Try \"create a util logging.py that...\"",
    "Try \"write a test for <filepath>\"",
];
/// Commands the look answers itself, listed in the menu when the backend does not name them.
pub const LOCAL_COMMANDS: &[(&str, &str)] = &[
    ("clear", "Clear conversation history and free up context"),
    ("exit", "Exit the REPL"),
    ("help", "Show help and available commands"),
    ("model", "Set the AI model for Wizard"),
    ("resume", "Resume a previous conversation"),
    (
        "status",
        "Show Wizard status including version, model and session",
    ),
];
/// How long a footer hint stays up, and how quickly a second press must follow to count.
const HINT_TTL: Duration = Duration::from_millis(800);
const DOUBLE_PRESS: Duration = Duration::from_millis(800);
const FRAME: Duration = Duration::from_millis(120);

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Msg {
    Backend(Event),
    Files(Vec<String>),
    Shell {
        command: String,
        output: String,
        ok: bool,
    },
}

#[derive(Clone, Debug, Default)]
pub struct AppOpts {
    pub cwd: PathBuf,
    pub resume: Option<String>,
    pub continue_latest: bool,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub plan: bool,
    pub prompts: Vec<String>,
    pub backend: BackendKind,
    pub light: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum BackendKind {
    #[default]
    Wizard,
    Mock,
    Replay(PathBuf),
}

/// Monotonic time the tests can pin.
#[derive(Debug)]
pub struct Clock {
    start: Instant,
    frozen: Option<Duration>,
}

impl Default for Clock {
    fn default() -> Self {
        Clock {
            start: Instant::now(),
            frozen: None,
        }
    }
}

impl Clock {
    pub fn elapsed(&self) -> Duration {
        self.frozen.unwrap_or_else(|| self.start.elapsed())
    }

    pub fn freeze(&mut self, at: Duration) {
        self.frozen = Some(at);
    }
}

#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Block {
    User {
        text: String,
        bash: bool,
    },
    /// The result of a local or backend command, hanging under the user row before it.
    Output {
        text: String,
        accent: String,
        dim: bool,
    },
    Assistant {
        text: String,
        time: (u32, u32),
    },
    Thought(String),
    Tool(ToolCall),
    Todos(Vec<Todo>),
    TurnEnd {
        verb: usize,
        secs: u64,
        time: (u32, u32),
    },
    Notice {
        level: NoticeLevel,
        text: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Menu {
    None,
    /// Indexes into `App::commands`.
    Slash {
        items: Vec<usize>,
        sel: usize,
    },
    Mention {
        items: Vec<String>,
        sel: Option<usize>,
    },
}

#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Dialog {
    Help { tab: usize, sel: usize, focus: bool },
    Model { sel: usize },
    Status,
    Resume { sel: usize, query: String },
    Permission { req: PermissionRequest, sel: usize },
}

#[derive(Clone, Debug)]
pub struct Busy {
    pub since: Duration,
    pub verb: usize,
    pub chars: usize,
    pub thinking: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    CtrlC,
    CtrlD,
    Esc,
}

pub struct App {
    pub opts: AppOpts,
    pub tx: UnboundedSender<Request>,
    pub msg_tx: UnboundedSender<Msg>,
    pub size: (u16, u16),
    pub theme: Theme,
    pub clock: Clock,
    /// Wall clock override for tests: hour and minute.
    pub fixed_hm: Option<(u32, u32)>,
    /// Unix seconds for relative times; tests pin it.
    pub now_unix: i64,
    pub cfg: Config,
    pub session_id: String,
    pub commands: Vec<SlashCommand>,
    pub sessions: Vec<SessionInfo>,
    pub files: Vec<String>,
    pub blocks: Vec<Block>,
    pub todos: Vec<Todo>,
    pub show_todos: bool,
    pub usage: Usage,
    pub editor: Editor,
    pub bash_mode: bool,
    pub stash: Option<String>,
    pub prompts: Vec<String>,
    pub history_at: Option<usize>,
    pub draft: String,
    pub busy: Option<Busy>,
    pub queued: Vec<String>,
    pub plan: bool,
    pub menu: Menu,
    pub shortcuts: bool,
    pub dialog: Option<Dialog>,
    pub detail: bool,
    /// Rows the transcript is scrolled up from its end.
    pub scroll: usize,
    pub hint: Option<(Hint, Duration)>,
    pub notice: Option<String>,
    pub placeholder: usize,
    pub ready: bool,
    pub turns: usize,
    pub fatal: Option<String>,
    pub quit: bool,
    pub dirty: bool,
    pub repaint: bool,
    pub suspend: bool,
    pub edit_request: bool,
    /// Open the transcript in the editor (`v` in the detailed view).
    pub view_request: bool,
    /// The resume picker lists every project's sessions, not just this directory's.
    pub resume_all: bool,
    /// Where the terminal cursor goes, set by the last draw.
    pub cursor: Option<(u16, u16)>,
    /// Rows of the last transcript draw: content height and viewport height.
    pub last_view: (usize, usize),
    pending_prompts: Vec<String>,
    /// `/plan` toggles sent for shift+tab whose turn and answer stay out of the transcript.
    silent: usize,
}

impl App {
    pub fn new(opts: AppOpts, tx: UnboundedSender<Request>, msg_tx: UnboundedSender<Msg>) -> App {
        let theme = if opts.light {
            Theme::light()
        } else {
            Theme::dark()
        };
        let plan = opts.plan;
        let pending_prompts = opts.prompts.clone();
        App {
            opts,
            tx,
            msg_tx,
            size: (80, 24),
            theme,
            clock: Clock::default(),
            fixed_hm: None,
            now_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64),
            cfg: Config::default(),
            session_id: String::new(),
            commands: LOCAL_COMMANDS
                .iter()
                .map(|(name, description)| SlashCommand {
                    name: name.to_string(),
                    description: description.to_string(),
                    input_hint: String::new(),
                })
                .collect(),
            sessions: Vec::new(),
            files: Vec::new(),
            blocks: Vec::new(),
            todos: Vec::new(),
            show_todos: true,
            usage: Usage::default(),
            editor: Editor::new(),
            bash_mode: false,
            stash: None,
            prompts: Vec::new(),
            history_at: None,
            draft: String::new(),
            busy: None,
            queued: Vec::new(),
            plan,
            menu: Menu::None,
            shortcuts: false,
            dialog: None,
            detail: false,
            scroll: 0,
            hint: None,
            notice: None,
            placeholder: 0,
            ready: false,
            turns: 0,
            fatal: None,
            quit: false,
            dirty: true,
            repaint: false,
            suspend: false,
            edit_request: false,
            view_request: false,
            resume_all: false,
            cursor: None,
            last_view: (0, 0),
            pending_prompts,
            silent: 0,
        }
    }

    pub fn send(&self, req: Request) {
        let _ = self.tx.send(req);
    }

    pub fn now_hm(&self) -> (u32, u32) {
        self.fixed_hm.unwrap_or_else(local_hm)
    }

    /// The model's display name, as the backend lists it.
    pub fn model_name(&self) -> String {
        self.cfg
            .models
            .iter()
            .find(|m| m.id == self.cfg.model)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| self.cfg.model.clone())
    }

    pub fn provider(&self) -> String {
        self.cfg
            .models
            .iter()
            .find(|m| m.id == self.cfg.model)
            .map(|m| m.provider.clone())
            .unwrap_or_default()
    }

    /// Which entry of the shift+tab cycle is on: an index into the backend's modes, or `None`
    /// for plan mode.
    pub fn mode_index(&self) -> Option<usize> {
        if self.plan {
            return None;
        }
        Some(
            self.cfg
                .modes
                .iter()
                .position(|m| *m == self.cfg.mode)
                .unwrap_or(0),
        )
    }

    fn output(&mut self, text: impl Into<String>, accent: impl Into<String>) {
        self.blocks.push(Block::Output {
            text: text.into(),
            accent: accent.into(),
            dim: false,
        });
        self.scroll = 0;
    }

    // ---- backend ------------------------------------------------------------------------

    pub fn update(&mut self, msg: Msg) {
        self.dirty = true;
        match msg {
            Msg::Backend(ev) => self.on_event(ev),
            Msg::Files(f) => {
                self.files = f;
                self.refresh_menu();
            }
            Msg::Shell {
                command,
                output,
                ok,
            } => {
                self.blocks.push(Block::Tool(ToolCall {
                    id: format!("shell-{}", self.blocks.len()),
                    name: "Bash".into(),
                    kind: ToolKind::Execute,
                    title: command.clone(),
                    input: serde_json::json!({ "command": command, "local": true }),
                    status: if ok {
                        ToolStatus::Completed
                    } else {
                        ToolStatus::Failed
                    },
                    output: Some(output),
                    ..Default::default()
                }));
                self.busy = None;
            }
        }
    }

    pub fn on_event(&mut self, ev: Event) {
        self.dirty = true;
        match ev {
            Event::Ready { session_id, config } => {
                self.session_id = session_id;
                self.cfg = config;
                if !self.ready {
                    self.ready = true;
                    self.notice = self.effort_notice();
                    if let Some(m) = self.opts.model.clone() {
                        self.send(Request::SetModel(m));
                    }
                    if let Some(e) = self.opts.effort.clone() {
                        self.send(Request::SetEffort(e));
                    }
                    if self.plan {
                        self.toggle_plan();
                    }
                    for p in std::mem::take(&mut self.pending_prompts) {
                        self.submit(p);
                    }
                }
            }
            Event::Commands(mut c) => {
                for (name, description) in LOCAL_COMMANDS {
                    if !c.iter().any(|x| x.name == *name) {
                        c.push(SlashCommand {
                            name: name.to_string(),
                            description: description.to_string(),
                            input_hint: String::new(),
                        });
                    }
                }
                self.commands = c;
                self.refresh_menu();
            }
            Event::ConfigChanged(c) => self.cfg = c,
            Event::Sessions(s) => self.sessions = s,
            Event::History { session_id, items } => {
                self.session_id = session_id;
                self.blocks.clear();
                let time = self.now_hm();
                for item in items {
                    match item {
                        HistoryItem::User(text) => {
                            self.turns += 1;
                            self.blocks.push(Block::User { text, bash: false })
                        }
                        HistoryItem::Assistant(text) => {
                            self.blocks.push(Block::Assistant { text, time })
                        }
                        HistoryItem::Thought(t) => self.blocks.push(Block::Thought(t)),
                        HistoryItem::Tool(t) => self.blocks.push(Block::Tool(t)),
                    }
                }
                self.scroll = 0;
            }
            Event::TurnStart if self.silent > 0 && self.busy.is_none() => {}
            Event::TurnStart => {
                if self.busy.is_none() {
                    self.start_busy();
                }
            }
            Event::TextDelta(t) => {
                if let Some(b) = &mut self.busy {
                    b.chars += t.len();
                    b.thinking = false;
                }
                let time = self.now_hm();
                match self.blocks.last_mut() {
                    Some(Block::Assistant { text, .. }) => text.push_str(&t),
                    _ => self.blocks.push(Block::Assistant { text: t, time }),
                }
            }
            Event::ThoughtDelta(t) => {
                if let Some(b) = &mut self.busy {
                    b.chars += t.len();
                    b.thinking = true;
                }
                match self.blocks.last_mut() {
                    Some(Block::Thought(text)) => text.push_str(&t),
                    _ => self.blocks.push(Block::Thought(t)),
                }
            }
            Event::Tool(call) => {
                if let Some(b) = &mut self.busy {
                    b.thinking = false;
                }
                let at = self.blocks.iter().position(|b| match b {
                    Block::Tool(t) => t.id == call.id,
                    _ => false,
                });
                match at {
                    Some(i) => self.blocks[i] = Block::Tool(call),
                    None => self.blocks.push(Block::Tool(call)),
                }
            }
            Event::Todos(t) => {
                self.todos = t.clone();
                match self.blocks.last_mut() {
                    Some(Block::Todos(old)) => *old = t,
                    _ => self.blocks.push(Block::Todos(t)),
                }
            }
            Event::Usage(u) => self.usage = u,
            Event::TurnEnd(_) if self.silent > 0 && self.busy.is_none() => self.silent -= 1,
            Event::TurnEnd(reason) => self.end_turn(reason),
            Event::Notice { text, .. }
                if agent_core::switched_look(&text).is_some_and(|l| l != LOOK) =>
            {
                self.quit = true;
            }
            Event::Notice { level, text } => {
                let low = text.to_lowercase();
                if low.contains("plan mode") {
                    self.plan = low.contains(" on") || low.contains("enabled");
                }
                if self.silent > 0 && level == NoticeLevel::Info {
                    return;
                }
                let after_command = matches!(
                    self.blocks.last(),
                    Some(Block::User { text, .. }) if text.starts_with('/')
                );
                if after_command && level == NoticeLevel::Info {
                    let dim = low.starts_with("compact");
                    self.blocks.push(Block::Output {
                        text,
                        accent: String::new(),
                        dim,
                    });
                } else {
                    self.blocks.push(Block::Notice { level, text });
                }
            }
            Event::Fatal(e) => {
                self.busy = None;
                self.blocks.push(Block::Notice {
                    level: NoticeLevel::Error,
                    text: e.clone(),
                });
                self.fatal = Some(e);
            }
            Event::Permission(req) => {
                self.dialog = Some(Dialog::Permission { req, sel: 0 });
            }
            Event::RewindPreview(_) => {}
        }
    }

    fn effort_notice(&self) -> Option<String> {
        let e = &self.cfg.effort;
        if e.is_empty() {
            return None;
        }
        Some(format!("{} {e} · /effort", effort_glyph(e)))
    }

    fn start_busy(&mut self) {
        let n = self.turns + self.blocks.len();
        self.busy = Some(Busy {
            since: self.clock.elapsed(),
            verb: n % VERBS.len(),
            chars: 0,
            thinking: false,
        });
        self.notice = None;
    }

    fn end_turn(&mut self, reason: StopReason) {
        let Some(busy) = self.busy.take() else { return };
        let secs = (self.clock.elapsed().saturating_sub(busy.since)).as_secs();
        let command_turn = self
            .blocks
            .iter()
            .rev()
            .find_map(|b| match b {
                Block::User { text, bash } => Some(text.starts_with('/') && !bash),
                _ => None,
            })
            .unwrap_or(false);
        match reason {
            StopReason::Cancelled => self.blocks.push(Block::Output {
                text: "Interrupted · What should Wizard do instead?".into(),
                accent: String::new(),
                dim: true,
            }),
            _ if command_turn => {}
            _ => self.blocks.push(Block::TurnEnd {
                verb: busy.verb % DONE_VERBS.len(),
                secs,
                time: self.now_hm(),
            }),
        }
        if !self.queued.is_empty() {
            let next = self.queued.remove(0);
            self.submit(next);
        }
    }

    // ---- prompts ------------------------------------------------------------------------

    /// Send `text` as the user typed it: a local command, a shell line or a prompt.
    pub fn submit(&mut self, text: String) {
        let text = text.trim_end().to_string();
        if text.is_empty() {
            return;
        }
        self.scroll = 0;
        self.notice = None;
        if self.prompts.last() != Some(&text) {
            self.prompts.push(text.clone());
        }
        self.history_at = None;
        if self.bash_mode {
            self.bash_mode = false;
            self.run_shell(text);
            return;
        }
        if self.busy.is_some() {
            self.queued.push(text);
            return;
        }
        let (name, arg) = match text.strip_prefix('/') {
            Some(rest) => {
                let (n, a) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                (n.to_string(), a.trim().to_string())
            }
            None => (String::new(), String::new()),
        };
        match name.as_str() {
            "exit" | "quit" => {
                self.quit = true;
                return;
            }
            "clear" | "new" | "reset" => {
                self.blocks.clear();
                self.todos.clear();
                self.turns = 0;
                self.usage = Usage::default();
                self.blocks.push(Block::User { text, bash: false });
                self.send(Request::NewSession);
                return;
            }
            "help" => {
                self.dialog = Some(Dialog::Help {
                    tab: 0,
                    sel: 0,
                    focus: false,
                });
                return;
            }
            "status" => {
                self.dialog = Some(Dialog::Status);
                return;
            }
            "model" if arg.is_empty() && !self.cfg.models.is_empty() => {
                self.open_model();
                return;
            }
            "resume" | "continue" if arg.is_empty() => {
                self.sessions.clear();
                self.send(Request::ListSessions);
                self.dialog = Some(Dialog::Resume {
                    sel: 0,
                    query: String::new(),
                });
                return;
            }
            "resume" | "continue" => {
                self.blocks.push(Block::User { text, bash: false });
                self.send(Request::LoadSession(arg));
                return;
            }
            _ => {}
        }
        self.blocks.push(Block::User {
            text: text.clone(),
            bash: false,
        });
        self.turns += 1;
        self.start_busy();
        self.send(Request::Prompt(text));
    }

    fn run_shell(&mut self, command: String) {
        self.blocks.push(Block::User {
            text: command.clone(),
            bash: true,
        });
        self.start_busy();
        let tx = self.msg_tx.clone();
        let cwd = self.opts.cwd.clone();
        std::thread::spawn(move || {
            let out = std::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .current_dir(cwd)
                .stdin(std::process::Stdio::null())
                .output();
            let (output, ok) = match out {
                Ok(o) => {
                    let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
                    s.push_str(&String::from_utf8_lossy(&o.stderr));
                    (s.trim_end().to_string(), o.status.success())
                }
                Err(e) => (e.to_string(), false),
            };
            let _ = tx.send(Msg::Shell {
                command,
                output,
                ok,
            });
        });
    }

    fn open_model(&mut self) {
        let sel = self
            .cfg
            .models
            .iter()
            .position(|m| m.id == self.cfg.model)
            .unwrap_or(0);
        self.dialog = Some(Dialog::Model { sel });
    }

    // ---- menus --------------------------------------------------------------------------

    /// The `@token` the cursor is in or just after, as (byte start, query).
    fn mention_at_cursor(&self) -> Option<(usize, String)> {
        let text = self.editor.text();
        let upto = &text[..self.editor.cursor().min(text.len())];
        let start = upto.rfind(|c: char| c.is_whitespace()).map_or(0, |i| i + 1);
        let token = &upto[start..];
        token.strip_prefix('@').map(|q| (start, q.to_string()))
    }

    pub fn refresh_menu(&mut self) {
        let text = self.editor.text().to_string();
        if self.bash_mode {
            self.menu = Menu::None;
            return;
        }
        if let Some(q) = text.strip_prefix('/') {
            if !q.contains(char::is_whitespace) {
                let items = slash_matches(&self.commands, q);
                let sel = match &self.menu {
                    Menu::Slash { sel, items: old } if *old == items => *sel,
                    _ => 0,
                };
                self.menu = Menu::Slash { items, sel };
                return;
            }
        }
        if let Some((_, q)) = self.mention_at_cursor() {
            let items = mention_matches(&self.files, &q);
            let sel = match &self.menu {
                Menu::Mention { sel, items: old } if *old == items => *sel,
                _ => None,
            };
            self.menu = Menu::Mention { items, sel };
            return;
        }
        self.menu = Menu::None;
    }

    fn menu_move(&mut self, down: bool) {
        match &mut self.menu {
            Menu::Slash { items, sel } if !items.is_empty() => {
                let n = items.len();
                *sel = if down {
                    (*sel + 1) % n
                } else {
                    (*sel + n - 1) % n
                };
            }
            Menu::Mention { items, sel } if !items.is_empty() => {
                let n = items.len();
                *sel = Some(match *sel {
                    None if down => 0,
                    None => n - 1,
                    Some(i) if down => (i + 1) % n,
                    Some(i) => (i + n - 1) % n,
                });
            }
            _ => {}
        }
    }

    /// Put the selected menu entry into the input. Returns whether there was one to take.
    fn menu_accept(&mut self, first_if_none: bool) -> bool {
        match self.menu.clone() {
            Menu::Slash { items, sel } => {
                let Some(cmd) = items.get(sel).and_then(|i| self.commands.get(*i)) else {
                    return false;
                };
                self.editor.set_text(&format!("/{} ", cmd.name));
                self.menu = Menu::None;
                true
            }
            Menu::Mention { items, sel } => {
                let pick = match sel {
                    Some(i) => items.get(i),
                    None if first_if_none => items.first(),
                    None => None,
                };
                let (Some(path), Some((start, _))) = (pick.cloned(), self.mention_at_cursor())
                else {
                    return false;
                };
                let text = self.editor.text().to_string();
                let cursor = self.editor.cursor().min(text.len());
                let dir = path.ends_with('/');
                let insert = if dir {
                    format!("@{path}")
                } else {
                    format!("@{path} ")
                };
                let new = format!("{}{}{}", &text[..start], insert, &text[cursor..]);
                self.editor.set_text(&new);
                self.editor.set_cursor(start + insert.len());
                self.menu = Menu::None;
                if dir {
                    self.refresh_menu();
                }
                true
            }
            Menu::None => false,
        }
    }

    // ---- keys ---------------------------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) {
        self.dirty = true;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let now = self.clock.elapsed();
        let prev_hint = self
            .hint
            .filter(|(_, at)| now.saturating_sub(*at) < DOUBLE_PRESS)
            .map(|(h, _)| h);
        if matches!(key.code, KeyCode::Esc)
            || (ctrl && matches!(key.code, KeyCode::Char('c' | 'd')))
        {
            self.hint = None;
        }

        if ctrl && key.code == KeyCode::Char('c') {
            return self.ctrl_c(prev_hint);
        }
        if self.detail {
            return self.detail_key(key);
        }
        if self.dialog.is_some() {
            return self.dialog_key(key);
        }
        match key.code {
            KeyCode::Char('d') if ctrl && self.editor.is_empty() => {
                if prev_hint == Some(Hint::CtrlD) {
                    self.quit = true;
                } else {
                    self.hint = Some((Hint::CtrlD, self.clock.elapsed()));
                }
            }
            KeyCode::Char('o') if ctrl => {
                self.detail = true;
                self.scroll = 0;
            }
            KeyCode::Char('t') if ctrl => self.show_todos = !self.show_todos,
            KeyCode::Char('l') if ctrl => self.repaint = true,
            KeyCode::Char('z') if ctrl && !shift => self.suspend = true,
            KeyCode::Char('g') if ctrl => self.edit_request = true,
            KeyCode::Char('s') if ctrl => self.toggle_stash(),
            KeyCode::Char('p') if alt => self.open_model(),
            KeyCode::BackTab => self.cycle_mode(),
            KeyCode::PageUp => self.scroll_by(self.page() as isize),
            KeyCode::PageDown => self.scroll_by(-(self.page() as isize)),
            KeyCode::Esc => self.escape(prev_hint),
            KeyCode::Enter if shift || alt => self.insert_newline(),
            KeyCode::Char('j') if ctrl => self.insert_newline(),
            KeyCode::Enter => self.enter(),
            KeyCode::Tab => {
                self.menu_accept(true);
            }
            KeyCode::Up if self.menu_open() => self.menu_move(false),
            KeyCode::Down if self.menu_open() => self.menu_move(true),
            KeyCode::Char('?') if self.editor.is_empty() && !self.bash_mode => {
                self.shortcuts = !self.shortcuts;
            }
            KeyCode::Char('!') if self.editor.is_empty() && !self.bash_mode => {
                self.bash_mode = true;
                self.shortcuts = false;
            }
            KeyCode::Backspace if self.editor.is_empty() && (self.bash_mode || self.shortcuts) => {
                self.bash_mode = false;
                self.shortcuts = false;
            }
            _ => {
                self.shortcuts = false;
                match self.editor.apply_key(key) {
                    KeyOutcome::AtTop if key.code == KeyCode::Up => self.history(-1),
                    KeyOutcome::AtBottom if key.code == KeyCode::Down => self.history(1),
                    _ => {
                        if !matches!(key.code, KeyCode::Up | KeyCode::Down) {
                            self.history_at = None;
                        }
                    }
                }
                self.refresh_menu();
            }
        }
    }

    fn menu_open(&self) -> bool {
        match &self.menu {
            Menu::None => false,
            Menu::Slash { items, .. } => !items.is_empty(),
            Menu::Mention { items, .. } => !items.is_empty(),
        }
    }

    fn page(&self) -> usize {
        (self.last_view.1 / 2).max(1)
    }

    pub fn scroll_by(&mut self, up: isize) {
        let max = self.last_view.0.saturating_sub(self.last_view.1) as isize;
        self.scroll = (self.scroll as isize + up).clamp(0, max.max(0)) as usize;
    }

    fn ctrl_c(&mut self, prev: Option<Hint>) {
        if self.dialog.is_some() {
            self.close_dialog();
            return;
        }
        if self.detail {
            self.detail = false;
            return;
        }
        if prev == Some(Hint::CtrlC) && self.editor.is_empty() {
            self.quit = true;
            return;
        }
        if self.editor.is_empty() && self.busy.is_some() {
            self.send(Request::Cancel);
            return;
        }
        self.editor.clear();
        self.bash_mode = false;
        self.shortcuts = false;
        self.history_at = None;
        self.menu = Menu::None;
        self.hint = Some((Hint::CtrlC, self.clock.elapsed()));
    }

    fn escape(&mut self, prev: Option<Hint>) {
        if self.shortcuts {
            self.shortcuts = false;
        } else if self.menu_open() {
            self.menu = Menu::None;
        } else if self.bash_mode && self.editor.is_empty() {
            self.bash_mode = false;
        } else if self.busy.is_some() {
            self.queued.clear();
            self.send(Request::Cancel);
        } else if !self.editor.is_empty() {
            if prev == Some(Hint::Esc) {
                self.editor.clear();
                self.menu = Menu::None;
            } else {
                self.hint = Some((Hint::Esc, self.clock.elapsed()));
            }
        }
    }

    fn insert_newline(&mut self) {
        self.editor.insert_newline();
        self.refresh_menu();
    }

    fn enter(&mut self) {
        if self.editor.text().ends_with('\\') && self.editor.cursor() == self.editor.text().len() {
            self.editor.backspace();
            self.editor.insert_newline();
            return;
        }
        if let Menu::Slash { items, sel } = &self.menu {
            if let Some(cmd) = items.get(*sel).and_then(|i| self.commands.get(*i)) {
                let text = format!("/{}", cmd.name);
                self.editor.clear();
                self.menu = Menu::None;
                self.submit(text);
                return;
            }
        }
        if matches!(self.menu, Menu::Mention { sel: Some(_), .. }) && self.menu_accept(false) {
            return;
        }
        let text = self.editor.text().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.editor.clear();
        self.menu = Menu::None;
        self.submit(text);
        if let Some(s) = self.stash.take() {
            self.editor.set_text(&s);
        }
    }

    fn toggle_stash(&mut self) {
        if self.editor.is_empty() {
            if let Some(s) = self.stash.take() {
                self.editor.set_text(&s);
            }
        } else {
            self.stash = Some(self.editor.text().to_string());
            self.editor.clear();
        }
        self.refresh_menu();
    }

    fn history(&mut self, step: isize) {
        if self.prompts.is_empty() {
            return;
        }
        let n = self.prompts.len();
        let next = match (self.history_at, step) {
            (None, s) if s < 0 => {
                self.draft = self.editor.text().to_string();
                Some(n - 1)
            }
            (None, _) => None,
            (Some(0), s) if s < 0 => Some(0),
            (Some(i), s) if s < 0 => Some(i - 1),
            (Some(i), _) if i + 1 >= n => None,
            (Some(i), _) => Some(i + 1),
        };
        if next == self.history_at {
            return;
        }
        self.history_at = next;
        match next {
            Some(i) => {
                let t = self.prompts[i].clone();
                self.editor.set_text(&t);
            }
            None => {
                let d = std::mem::take(&mut self.draft);
                self.editor.set_text(&d);
            }
        }
        self.menu = Menu::None;
    }

    /// Flip wizard's plan mode without a transcript entry.
    fn toggle_plan(&mut self) {
        self.silent += 1;
        self.send(Request::Prompt("/plan".into()));
    }

    fn cycle_mode(&mut self) {
        let modes = self.cfg.modes.clone();
        match self.mode_index() {
            None => {
                self.plan = false;
                self.toggle_plan();
                if let Some(first) = modes.first() {
                    if *first != self.cfg.mode {
                        self.cfg.mode = first.clone();
                        self.send(Request::SetMode(first.clone()));
                    }
                }
            }
            Some(i) if i + 1 < modes.len() => {
                self.cfg.mode = modes[i + 1].clone();
                self.send(Request::SetMode(modes[i + 1].clone()));
            }
            Some(_) => {
                self.plan = true;
                self.toggle_plan();
            }
        }
    }

    fn detail_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('o') if ctrl => self.detail = false,
            KeyCode::Esc | KeyCode::Char('q') => self.detail = false,
            KeyCode::Char('?') => self.shortcuts = !self.shortcuts,
            KeyCode::Char('v') => self.view_request = true,
            KeyCode::Up | KeyCode::Char('k') => self.scroll_by(1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_by(-1),
            KeyCode::PageUp => self.scroll_by(self.page() as isize),
            KeyCode::PageDown => self.scroll_by(-(self.page() as isize)),
            _ => {}
        }
        if !self.detail {
            self.scroll = 0;
        }
    }

    fn close_dialog(&mut self) {
        match self.dialog.take() {
            Some(Dialog::Model { .. }) => {
                self.blocks.push(Block::User {
                    text: "/model ".into(),
                    bash: false,
                });
                let name = self.model_name();
                self.output(format!("Kept model as {name}"), name);
            }
            Some(Dialog::Resume { .. }) => {
                self.resume_all = false;
                self.blocks.push(Block::User {
                    text: "/resume ".into(),
                    bash: false,
                });
                self.output("Resume cancelled", "");
            }
            Some(Dialog::Permission { req, .. }) => {
                self.send(Request::Decide {
                    id: req.id,
                    allow: false,
                    scope: DecideScope::Once,
                    note: String::new(),
                });
            }
            _ => {}
        }
    }

    /// Sessions the resume picker lists for `query`: this directory's first.
    pub fn resume_list(&self, query: &str) -> Vec<&SessionInfo> {
        let q = query.to_lowercase();
        let cwd = self.opts.cwd.to_string_lossy();
        let mut v: Vec<&SessionInfo> = self
            .sessions
            .iter()
            .filter(|s| self.resume_all || s.cwd.is_empty() || s.cwd == cwd)
            .filter(|s| q.is_empty() || s.title.to_lowercase().contains(&q))
            .collect();
        v.sort_by_key(|s| std::cmp::Reverse(s.updated));
        v
    }

    fn dialog_key(&mut self, key: KeyEvent) {
        let Some(dialog) = self.dialog.clone() else {
            return;
        };
        if key.code == KeyCode::Esc {
            return self.close_dialog();
        }
        match dialog {
            Dialog::Help { tab, sel, focus } => {
                let n = self.commands.len();
                let (tab, sel, focus) = match key.code {
                    KeyCode::Tab | KeyCode::Right | KeyCode::BackTab | KeyCode::Left => {
                        ((tab + 1) % 2, 0, false)
                    }
                    KeyCode::Down if tab == 1 && n > 0 && !focus => (tab, sel, true),
                    KeyCode::Down if tab == 1 && n > 0 => (tab, (sel + 1) % n, true),
                    KeyCode::Up if tab == 1 && focus && sel == 0 => (tab, 0, false),
                    KeyCode::Up if tab == 1 && focus => (tab, sel - 1, true),
                    _ => (tab, sel, focus),
                };
                self.dialog = Some(Dialog::Help { tab, sel, focus });
            }
            Dialog::Model { sel } => {
                let n = self.cfg.models.len().max(1);
                match key.code {
                    KeyCode::Down => self.dialog = Some(Dialog::Model { sel: (sel + 1) % n }),
                    KeyCode::Up => {
                        self.dialog = Some(Dialog::Model {
                            sel: (sel + n - 1) % n,
                        })
                    }
                    KeyCode::Left => self.step_effort(-1),
                    KeyCode::Right => self.step_effort(1),
                    KeyCode::Char(c @ '1'..='9') if (c as usize - '1' as usize) < n => {
                        self.pick_model(c as usize - '1' as usize)
                    }
                    KeyCode::Enter => self.pick_model(sel),
                    _ => {}
                }
            }
            Dialog::Status => {
                if key.code == KeyCode::Enter {
                    self.dialog = None;
                }
            }
            Dialog::Resume { sel, mut query } => {
                let ids: Vec<String> = self
                    .resume_list(&query)
                    .iter()
                    .map(|s| s.id.clone())
                    .collect();
                let n = ids.len().max(1);
                let mut sel = sel.min(n - 1);
                match key.code {
                    KeyCode::Down => sel = (sel + 1) % n,
                    KeyCode::Up => sel = (sel + n - 1) % n,
                    KeyCode::Backspace => {
                        query.pop();
                        sel = 0;
                    }
                    KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.resume_all = !self.resume_all;
                        sel = 0;
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        query.push(c);
                        sel = 0;
                    }
                    KeyCode::Enter => {
                        if let Some(id) = ids.get(sel) {
                            self.dialog = None;
                            self.blocks.clear();
                            self.send(Request::LoadSession(id.clone()));
                        }
                        return;
                    }
                    _ => {}
                }
                self.dialog = Some(Dialog::Resume { sel, query });
            }
            Dialog::Permission { req, sel } => {
                let options = permission_options(&req);
                let n = options.len();
                match key.code {
                    KeyCode::Down => {
                        self.dialog = Some(Dialog::Permission {
                            req,
                            sel: (sel + 1) % n,
                        })
                    }
                    KeyCode::Up => {
                        self.dialog = Some(Dialog::Permission {
                            req,
                            sel: (sel + n - 1) % n,
                        })
                    }
                    KeyCode::Char(c @ '1'..='3') if (c as usize - '1' as usize) < n => {
                        self.decide(req, options[c as usize - '1' as usize].1)
                    }
                    KeyCode::Enter => self.decide(req, options[sel].1),
                    _ => {}
                }
            }
        }
    }

    fn decide(&mut self, req: PermissionRequest, choice: Choice) {
        self.dialog = None;
        self.send(Request::Decide {
            id: req.id,
            allow: choice != Choice::No,
            scope: if choice == Choice::Always {
                DecideScope::Always
            } else {
                DecideScope::Once
            },
            note: String::new(),
        });
    }

    fn pick_model(&mut self, i: usize) {
        let Some(m) = self.cfg.models.get(i).cloned() else {
            return;
        };
        self.dialog = None;
        self.blocks.push(Block::User {
            text: "/model ".into(),
            bash: false,
        });
        if m.id == self.cfg.model {
            self.output(format!("Kept model as {}", m.name), m.name);
        } else {
            self.send(Request::SetModel(m.id.clone()));
            self.cfg.model = m.id;
            self.output(format!("Set model to {}", m.name), m.name);
        }
    }

    fn step_effort(&mut self, step: isize) {
        let n = self.cfg.efforts.len() as isize;
        if n == 0 {
            return;
        }
        let at = self
            .cfg
            .efforts
            .iter()
            .position(|e| *e == self.cfg.effort)
            .unwrap_or(0) as isize;
        let next = self.cfg.efforts[(at + step).clamp(0, n - 1) as usize].clone();
        if next != self.cfg.effort {
            self.cfg.effort = next.clone();
            self.send(Request::SetEffort(next));
        }
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::ScrollUp => self.scroll_by(3),
            MouseEventKind::ScrollDown => self.scroll_by(-3),
            _ => return,
        }
        self.dirty = true;
    }

    pub fn on_paste(&mut self, s: &str) {
        if self.dialog.is_some() || self.detail {
            return;
        }
        self.editor.paste(s);
        self.shortcuts = false;
        self.refresh_menu();
        self.dirty = true;
    }

    pub fn on_resize(&mut self, w: u16, h: u16) {
        self.size = (w, h);
        self.scroll = 0;
        self.dirty = true;
    }

    /// Expire the footer hint and advance the spinner. Returns nothing; sets `dirty`.
    pub fn on_tick(&mut self) {
        let now = self.clock.elapsed();
        if let Some((_, at)) = self.hint {
            if now.saturating_sub(at) >= HINT_TTL {
                self.hint = None;
                self.dirty = true;
            }
        }
        if self.busy.is_some() && self.dialog.is_none() {
            self.dirty = true;
        }
    }

    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        if self.busy.is_some() && self.dialog.is_none() {
            return Some(now + FRAME);
        }
        self.hint.map(|(_, at)| {
            let left = (at + HINT_TTL).saturating_sub(self.clock.elapsed());
            now + left.max(Duration::from_millis(10))
        })
    }

    /// What is printed to the normal screen after the UI is gone.
    pub fn epilogue(&self) -> String {
        if self.blocks.is_empty() || self.session_id.is_empty() {
            return "\n".to_string();
        }
        format!(
            "\n\x1b[2mResume this session with:\x1b[0m\n\x1b[2mwizard-ui-claude --resume {}\x1b[0m\n",
            self.session_id
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Yes,
    Always,
    No,
}

/// The numbered answers a permission prompt offers.
pub fn permission_options(req: &PermissionRequest) -> Vec<(String, Choice)> {
    let mut v = vec![("Yes".to_string(), Choice::Yes)];
    if !req.rule.is_empty() {
        let what = match req.kind {
            ToolKind::Edit => "Yes, allow all edits during this session".to_string(),
            _ => format!("Yes, and don't ask again for {}", req.rule),
        };
        v.push((what, Choice::Always));
    }
    v.push(("No".to_string(), Choice::No));
    v
}

pub fn effort_glyph(effort: &str) -> &'static str {
    match effort {
        "low" | "minimal" | "none" => "○",
        "high" => "●",
        "xhigh" | "max" => "◉",
        _ => "◐",
    }
}

/// Commands for what is typed after the slash: names that start with it, then names that
/// contain it, then descriptions that do.
pub fn slash_matches(commands: &[SlashCommand], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..commands.len()).collect();
    }
    let q = query.to_lowercase();
    let mut ranked: Vec<(u8, usize)> = commands
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let name = c.name.to_lowercase();
            if name.starts_with(&q) {
                Some((0, i))
            } else if name.contains(&q) {
                Some((1, i))
            } else if c.description.to_lowercase().contains(&q) {
                Some((2, i))
            } else {
                None
            }
        })
        .collect();
    ranked.sort();
    ranked.into_iter().map(|(_, i)| i).collect()
}

/// Paths for an `@` query: the top level when nothing is typed, otherwise every path that
/// matches, best first.
pub fn mention_matches(files: &[String], query: &str) -> Vec<String> {
    const MAX: usize = 15;
    if query.is_empty() {
        let mut top: Vec<&String> = files
            .iter()
            .filter(|f| !f.trim_end_matches('/').contains('/'))
            .collect();
        top.sort_by_key(|f| (!f.ends_with('/'), f.to_lowercase()));
        return top.into_iter().take(MAX).cloned().collect();
    }
    let q = query.to_lowercase();
    let mut ranked: Vec<(u8, usize, &String)> = files
        .iter()
        .filter_map(|f| {
            let low = f.to_lowercase();
            let base = low.trim_end_matches('/').rsplit('/').next().unwrap_or("");
            let rank = if low.starts_with(&q) {
                0
            } else if base.starts_with(&q) {
                1
            } else if low.contains(&q) {
                2
            } else {
                return None;
            };
            Some((rank, f.len(), f))
        })
        .collect();
    ranked.sort();
    ranked
        .into_iter()
        .take(MAX)
        .map(|(_, _, f)| f.clone())
        .collect()
}

/// Files and directories under `cwd`, relative, directories with a trailing slash. Hidden
/// entries and build output are left out.
pub fn list_files(cwd: &Path) -> Vec<String> {
    const LIMIT: usize = 5000;
    let mut out = Vec::new();
    let mut stack = vec![cwd.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            let path = e.path();
            let Ok(rel) = path.strip_prefix(cwd) else {
                continue;
            };
            let rel = rel.to_string_lossy().into_owned();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                out.push(format!("{rel}/"));
                stack.push(path);
            } else {
                out.push(rel);
            }
            if out.len() >= LIMIT {
                return out;
            }
        }
    }
    out
}

/// The editor `v` and ctrl+g open, by its short name.
pub fn editor_name() -> String {
    ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.trim().is_empty())
        .and_then(|v| {
            let cmd = v.split_whitespace().next()?.to_string();
            Some(cmd.rsplit('/').next().unwrap_or(&cmd).to_string())
        })
        .unwrap_or_else(|| "nano".to_string())
}

fn local_hm() -> (u32, u32) {
    // SAFETY: `time` and `localtime_r` only write to the values passed in.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        (tm.tm_hour as u32, tm.tm_min as u32)
    }
}

/// `9:31 PM`; `pad` gives `09:31 PM` as the detailed transcript prints it.
pub fn clock_text((h, m): (u32, u32), pad: bool) -> String {
    let h12 = match h % 12 {
        0 => 12,
        x => x,
    };
    let ap = if h < 12 { "AM" } else { "PM" };
    if pad {
        format!("{h12:02}:{m:02} {ap}")
    } else {
        format!("{h12}:{m:02} {ap}")
    }
}

/// Start the chosen backend and forward its events into `msg_tx`. Returns the request sender
/// and the forwarding task, which ends when the backend does.
pub fn spawn_backend(
    opts: &AppOpts,
    msg_tx: UnboundedSender<Msg>,
) -> Result<(UnboundedSender<Request>, tokio::task::JoinHandle<()>)> {
    let cwd = opts.cwd.clone();
    let resume = opts.resume.clone();
    let BackendHandle { tx, mut rx } = match &opts.backend {
        BackendKind::Mock => agent_core::mock::MockBackend::spawn(cwd, resume)?,
        BackendKind::Replay(path) => crate::replay::spawn(path)?,
        BackendKind::Wizard => backend_wizard::WizardBackend::spawn(cwd, resume)?,
    };
    let fwd = tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            if msg_tx.send(Msg::Backend(ev)).is_err() {
                break;
            }
        }
    });
    Ok((tx, fwd))
}

fn draw_frame(terminal: &mut Terminal, app: &mut App) -> Result<()> {
    terminal.draw(|f| {
        let area = f.area();
        app.size = (area.width, area.height);
        crate::ui::draw(f.buffer_mut(), app);
        if let Some(c) = app.cursor {
            f.set_cursor_position(c);
        }
    })?;
    Ok(())
}

/// Run until the app quits or the terminal closes.
pub async fn run(
    app: &mut App,
    terminal: &mut Terminal,
    guard: &mut TermGuard,
    msg_rx: UnboundedReceiver<Msg>,
) -> Result<()> {
    use futures_util::FutureExt;
    let input = tuikit::input::Input::spawn()?;
    let input_ctl = input.control();
    let mut pump = EventPump::<Msg>::spawn_raw(input, Duration::ZERO, msg_rx);
    loop {
        if app.repaint {
            app.repaint = false;
            terminal.clear()?;
            app.dirty = true;
        }
        if app.dirty {
            app.dirty = false;
            draw_frame(terminal, app)?;
        }
        if app.quit {
            break;
        }
        let first = match app.next_wake(Instant::now()) {
            Some(d) => tokio::select! {
                e = pump.next() => e,
                _ = tokio::time::sleep_until(d.into()) => Some(TermEvent::Tick),
            },
            None => pump.next().await,
        };
        let Some(first) = first else { break };
        handle(app, first);
        while !app.quit {
            match pump.next().now_or_never() {
                Some(Some(e)) => handle(app, e),
                Some(None) => app.quit = true,
                None => break,
            }
        }
        if std::mem::take(&mut app.suspend) {
            guard.suspend();
            // SAFETY: raising a signal on our own process.
            unsafe {
                libc::raise(libc::SIGTSTP);
            }
            let _ = guard.resume();
            app.repaint = true;
        }
        if std::mem::take(&mut app.view_request) {
            let w = app.size.0 as usize;
            let text: Vec<String> = crate::ui::transcript::rows(app, w, true)
                .iter()
                .map(|r| r.text().trim_end().to_string())
                .collect();
            let cwd = app.opts.cwd.clone();
            let _ = tuikit::input::external_editor(
                guard,
                &input_ctl,
                &cwd,
                "claudew-transcript",
                &text.join("\n"),
                Some("nano"),
            );
            app.repaint = true;
        }
        if std::mem::take(&mut app.edit_request) {
            let text = app.editor.text().to_string();
            let cwd = app.opts.cwd.clone();
            let out = tuikit::input::external_editor(
                guard,
                &input_ctl,
                &cwd,
                "claudew",
                &text,
                Some("nano"),
            );
            match out {
                tuikit::input::EditorOutcome::Edited(t) => {
                    app.editor.set_text(t.trim_end_matches('\n'));
                    app.refresh_menu();
                }
                tuikit::input::EditorOutcome::NoEditor => {
                    app.notice = Some("set $EDITOR to edit the prompt in it".into())
                }
                tuikit::input::EditorOutcome::Failed(e) => app.notice = Some(e),
            }
            app.repaint = true;
        }
    }
    Ok(())
}

fn handle(app: &mut App, ev: TermEvent<Msg>) {
    match ev {
        TermEvent::Key(k) => app.on_key(k),
        TermEvent::Mouse(m) => app.on_mouse(m),
        TermEvent::Paste(s) => app.on_paste(&s),
        TermEvent::Resize(w, h) => app.on_resize(w, h),
        TermEvent::FocusGained => app.repaint = true,
        TermEvent::FocusLost | TermEvent::ColorScheme(_) | TermEvent::Tick => {}
        TermEvent::Msg(m) => app.update(m),
    }
    app.on_tick();
}
