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
pub const CLAUDE_VERSION: &str = "2.1.299";
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
/// What to do when wizard cannot start a session or has gone away.
pub const SETUP_HINT: &str =
    "Run `wizard setup` to choose a provider and model, then start this look again.";
const HINT_TTL: Duration = Duration::from_millis(800);
/// How long the effort notice stays up after start and after a turn.
/// How long the effort notice stays once the session is up, a turn ends or a model is picked.
const NOTICE_TTL: Duration = Duration::from_secs(30);
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
    /// The result of a local or backend command, hanging under the user row before it. `dim`
    /// is the SGR dim attribute, `muted` the grey of secondary text.
    Output {
        text: String,
        accent: String,
        dim: bool,
        muted: bool,
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
    Help {
        tab: usize,
        sel: usize,
        focus: bool,
    },
    Model {
        sel: usize,
    },
    Status,
    Resume {
        sel: usize,
        query: String,
    },
    Permission {
        req: PermissionRequest,
        sel: usize,
    },
    /// Esc Esc on an empty prompt: pick a prompt to go back to before.
    Rewind {
        sel: usize,
    },
    /// ctrl+r: earlier prompts, filtered by what is typed.
    Search {
        query: String,
        sel: usize,
    },
}

#[derive(Clone, Debug)]
pub struct Busy {
    pub since: Duration,
    pub verb: usize,
    pub chars: usize,
    pub thinking: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModeStep {
    Plan,
    Mode(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    CtrlC,
    CtrlD,
    Esc,
    /// Esc on an empty prompt; a second opens the rewind picker.
    Rewind,
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
    /// Link texts and their targets in the last transcript draw, wrapped in OSC 8 after it.
    pub links: std::cell::RefCell<Vec<(String, String)>>,
    /// Where those links landed in the last frame: column, row, text, url, colour.
    pub link_spots: Vec<(u16, u16, String, String, ratatui::style::Color)>,
    /// When the notice above the prompt goes away; `None` keeps it.
    pub notice_until: Option<Duration>,
    /// Names of the commands that come from `~/.wizard/commands`.
    pub custom: Vec<String>,
    /// Subagents from `~/.wizard/subagents`: name and description.
    pub agents: Vec<(String, String)>,
    /// When each entry of `prompts` was sent, Unix seconds.
    pub prompt_times: Vec<i64>,
    rewind_answer: Option<String>,
    /// A note being typed for a permission answer (Tab on the prompt).
    pub amend: Option<String>,
    /// The prompt a `/rewind` is going back to, put back in the input when it lands.
    rewinding: Option<String>,
    /// shift+tab steps not yet sent, the one wizard has not answered, and where they lead.
    mode_steps: std::collections::VecDeque<ModeStep>,
    mode_waiting: Option<ModeStep>,
    mode_target: Option<(String, bool)>,
    /// The effort the model picker shows; applied only when the pick is confirmed.
    pub effort_preview: Option<String>,
    /// Prompt history kept across sessions, and where.
    history_file: Option<PathBuf>,
    /// The directory each entry of `prompts` was sent from.
    pub prompt_dirs: Vec<String>,
    /// The history search shows this project's prompts only.
    pub search_here: bool,
    /// ctrl+x was pressed: ctrl+s next sends the queued prompts now.
    chord: bool,
    /// The clock is pinned (tests).
    pub fixed_now: bool,
    /// Blocks holding a prompt that waited in the queue.
    pub from_queue: Vec<usize>,
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
        let mut app = App {
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
            links: Default::default(),
            link_spots: Vec::new(),
            notice_until: None,
            custom: Vec::new(),
            agents: Vec::new(),
            prompt_times: Vec::new(),
            amend: None,
            rewind_answer: None,
            rewinding: None,
            silent: 0,
            mode_steps: Default::default(),
            mode_waiting: None,
            mode_target: None,
            effort_preview: None,
            history_file: None,
            prompt_dirs: Vec::new(),
            search_here: false,
            chord: false,
            fixed_now: false,
            from_queue: Vec::new(),
        };
        app.load_home();
        app
    }

    /// Custom commands and subagents from `~/.wizard`. The backend runs a custom command as
    /// typed but does not list it, and keeps subagents to itself.
    fn load_home(&mut self) {
        self.load_history();
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return;
        };
        let wizard = home.join(".wizard");
        for (name, text) in read_dir_sorted(&wizard.join("commands"), "md") {
            let description = front_matter(&text, "description").unwrap_or_else(|| {
                text.lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty() && *l != "---")
                    .unwrap_or("")
                    .trim_start_matches('#')
                    .trim()
                    .to_string()
            });
            if !self.commands.iter().any(|c| c.name == name) {
                self.commands.push(SlashCommand {
                    name: name.clone(),
                    description,
                    input_hint: String::new(),
                });
            }
            self.custom.push(name);
        }
        for (stem, text) in read_dir_sorted(&wizard.join("subagents"), "toml") {
            let name = toml_str(&text, "name").unwrap_or(stem);
            let description = toml_str(&text, "description").unwrap_or_default();
            self.agents.push((name, description));
        }
    }

    /// Prompts sent from any directory before, oldest first, from `history.jsonl` in the state
    /// directory: `CLAUDEW_STATE_DIR`, else `$XDG_STATE_HOME/claudew` or
    /// `~/.local/state/claudew`. The scripted backends keep none unless the variable is set.
    fn load_history(&mut self) {
        let dir = match std::env::var_os("CLAUDEW_STATE_DIR") {
            Some(d) if !d.is_empty() => Some(PathBuf::from(d)),
            _ if self.opts.backend != BackendKind::Wizard => None,
            _ => std::env::var_os("XDG_STATE_HOME")
                .filter(|s| !s.is_empty())
                .map(|s| PathBuf::from(s).join("claudew"))
                .or_else(|| {
                    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state/claudew"))
                }),
        };
        let Some(file) = dir.map(|d| d.join("history.jsonl")) else {
            return;
        };
        if let Ok(text) = std::fs::read_to_string(&file) {
            for line in text.lines() {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue;
                };
                let Some(p) = v.get("text").and_then(|t| t.as_str()) else {
                    continue;
                };
                self.prompts.push(p.to_string());
                self.prompt_times
                    .push(v.get("at").and_then(|t| t.as_i64()).unwrap_or(0));
                self.prompt_dirs.push(
                    v.get("cwd")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string(),
                );
            }
        }
        self.history_file = Some(file);
    }

    fn remember(&mut self, text: &str) {
        let cwd = self.opts.cwd.to_string_lossy().into_owned();
        self.prompts.push(text.to_string());
        self.prompt_times.push(self.now_unix);
        self.prompt_dirs.push(cwd.clone());
        let Some(file) = &self.history_file else {
            return;
        };
        let line = serde_json::json!({ "text": text, "at": self.now_unix, "cwd": cwd });
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
        {
            use std::io::Write;
            let _ = writeln!(f, "{line}");
        }
    }

    /// Wizard's version, as the backend reports it (`wizard 3.8.10`).
    /// The reply's text is coming in. The text is the progress then, so the spinner and the
    /// row above it step aside.
    pub fn streaming(&self) -> bool {
        self.busy.is_some() && matches!(self.blocks.last(), Some(Block::Assistant { .. }))
    }

    /// Empty until wizard has said which it is; never this crate's own version.
    pub fn wizard_version(&self) -> String {
        self.cfg
            .backend
            .strip_prefix("wizard ")
            .unwrap_or("")
            .to_string()
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
        let (mode, plan) = self.shown_mode();
        if plan {
            return None;
        }
        Some(self.cfg.modes.iter().position(|m| *m == mode).unwrap_or(0))
    }

    fn output(&mut self, text: impl Into<String>, accent: impl Into<String>) {
        self.blocks.push(Block::Output {
            text: text.into(),
            accent: accent.into(),
            dim: false,
            muted: false,
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
                    self.notice_until = Some(self.clock.elapsed() + NOTICE_TTL);
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
                for custom in self
                    .commands
                    .iter()
                    .filter(|x| self.custom.contains(&x.name))
                {
                    if !c.iter().any(|x| x.name == custom.name) {
                        c.push(custom.clone());
                    }
                }
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
            Event::ConfigChanged(c) => {
                self.cfg = c;
                if matches!(self.mode_waiting, Some(ModeStep::Mode(_))) {
                    self.mode_waiting = None;
                    self.pump_modes();
                }
            }
            Event::Sessions(s) => self.sessions = s,
            Event::History { session_id, items } => {
                self.session_id = session_id;
                self.blocks.clear();
                self.from_queue.clear();
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
            Event::Tool(mut call) => {
                if let Some(d) = call.diff.take() {
                    call.diff = Some(place_diff(&self.opts.cwd, d));
                }
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
            Event::TurnEnd(_) if self.silent > 0 && self.busy.is_none() => {
                self.silent -= 1;
                if self.mode_waiting == Some(ModeStep::Plan) {
                    self.mode_waiting = None;
                }
                self.pump_modes();
            }
            Event::TurnEnd(reason) => {
                self.end_turn(reason);
                self.pump_modes();
            }
            // wizard turned a mode change down because something was running; try it again
            Event::Notice { text, .. } if text.starts_with("Options can only be changed") => {
                if let Some(step) = self.mode_waiting.take() {
                    self.mode_steps.push_front(step);
                }
            }
            Event::Notice { text, .. }
                if agent_core::switched_look(&text).is_some_and(|l| l != LOOK) =>
            {
                self.quit = true;
            }
            Event::Notice { text, .. } if self.rewinding.is_some() => {
                self.rewind_answer = Some(text);
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
                        muted: false,
                    });
                } else {
                    let setup = text.starts_with("could not start a session");
                    self.blocks.push(Block::Notice { level, text });
                    if setup {
                        self.busy = None;
                        self.blocks.push(Block::Notice {
                            level: NoticeLevel::Info,
                            text: SETUP_HINT.into(),
                        });
                    }
                }
            }
            // wizard is gone: nothing is running any more, and nothing typed can reach it
            Event::Fatal(e) => {
                self.busy = None;
                self.queued.clear();
                self.notice = None;
                self.blocks.push(Block::Notice {
                    level: NoticeLevel::Error,
                    text: e.clone(),
                });
                self.blocks.push(Block::Notice {
                    level: NoticeLevel::Info,
                    text: SETUP_HINT.into(),
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
        effort_notice_text(&self.cfg.effort)
    }

    fn start_busy(&mut self) {
        let n = self.turns + self.blocks.len();
        self.busy = Some(Busy {
            since: self.clock.elapsed(),
            verb: n % VERBS.len(),
            chars: 0,
            thinking: false,
        });
        self.notice = self.effort_notice();
        self.notice_until = None;
    }

    fn end_turn(&mut self, reason: StopReason) {
        if let Some(text) = self.rewinding.take() {
            self.finish_rewind(text);
            return;
        }
        let Some(busy) = self.busy.take() else { return };
        self.notice_until = Some(self.clock.elapsed() + NOTICE_TTL);
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
            StopReason::Cancelled => {
                // calls still out when the turn stopped fold away like finished ones
                for b in &mut self.blocks {
                    if let Block::Tool(t) = b {
                        if matches!(t.status, ToolStatus::Pending | ToolStatus::Running) {
                            t.status = ToolStatus::Failed;
                            t.output = Some(agent_core::INTERRUPTED_OUTPUT.into());
                        }
                    }
                }
                self.blocks.push(Block::Output {
                    text: "Interrupted · What should Wizard do instead?".into(),
                    accent: String::new(),
                    dim: false,
                    muted: true,
                });
                let used_tools = self
                    .blocks
                    .iter()
                    .rev()
                    .take_while(|b| !matches!(b, Block::User { .. }))
                    .any(|b| matches!(b, Block::Tool(_)));
                if used_tools {
                    self.blocks.push(Block::TurnEnd {
                        verb: busy.verb % DONE_VERBS.len(),
                        secs,
                        time: self.now_hm(),
                    });
                }
            }
            _ if command_turn => {}
            _ => self.blocks.push(Block::TurnEnd {
                verb: busy.verb % DONE_VERBS.len(),
                secs,
                time: self.now_hm(),
            }),
        }
        if !self.queued.is_empty() {
            let next = self.queued.remove(0);
            let at = self.blocks.len();
            self.submit(next);
            if matches!(self.blocks.get(at), Some(Block::User { .. })) {
                self.from_queue.push(at);
            }
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
        if self.prompts.last() != Some(&text) {
            self.remember(&text);
        }
        self.history_at = None;
        if self.bash_mode {
            self.bash_mode = false;
            self.run_shell(text);
            return;
        }
        let (name, arg) = match text.strip_prefix('/') {
            Some(rest) => {
                let (n, a) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                (n.to_string(), a.trim().to_string())
            }
            None => (String::new(), String::new()),
        };
        if matches!(name.as_str(), "exit" | "quit") {
            self.quit = true;
            return;
        }
        if self.fatal.is_some() {
            self.blocks.push(Block::User { text, bash: false });
            self.blocks.push(Block::Notice {
                level: NoticeLevel::Error,
                text: "Not sent: there is no session to send it to.".into(),
            });
            return;
        }
        if self.busy.is_some() {
            self.queued.push(text);
            return;
        }
        match name.as_str() {
            "exit" | "quit" => {
                self.quit = true;
                return;
            }
            "clear" | "new" | "reset" => {
                self.notice = None;
                self.from_queue.clear();
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
            let items = mention_matches(&self.files, &self.agents, &q);
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
                let path = match path.strip_prefix(AGENT) {
                    Some(a) => format!("agent-{}", a.split('\t').next().unwrap_or("")),
                    None => path,
                };
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
        let chord = std::mem::take(&mut self.chord);
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
            KeyCode::Char('r') if ctrl => {
                self.dialog = Some(Dialog::Search {
                    query: String::new(),
                    sel: 0,
                })
            }
            KeyCode::End if ctrl => self.scroll = 0,
            KeyCode::Char('l') if ctrl => self.repaint = true,
            KeyCode::Char('z') if ctrl && !shift => self.suspend = true,
            KeyCode::Char('g') if ctrl => self.edit_request = true,
            KeyCode::Char('s') if ctrl && chord => {
                self.chord = false;
                if self.busy.is_some() && !self.queued.is_empty() {
                    self.send(Request::Cancel);
                }
            }
            KeyCode::Char('x') if ctrl => self.chord = true,
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
                    KeyOutcome::AtTop if key.code == KeyCode::Up && !self.queued.is_empty() => {
                        let back = std::mem::take(&mut self.queued).join("\n");
                        self.editor.set_text(&back);
                    }
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
            self.hint = Some((Hint::CtrlC, self.clock.elapsed()));
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
            self.send(Request::Cancel);
        } else if !self.editor.is_empty() {
            if prev == Some(Hint::Esc) {
                self.editor.clear();
                self.menu = Menu::None;
            } else {
                self.hint = Some((Hint::Esc, self.clock.elapsed()));
            }
        } else if prev == Some(Hint::Rewind) {
            if !self.rewind_targets().is_empty() {
                self.dialog = Some(Dialog::Rewind { sel: usize::MAX });
            }
        } else {
            self.hint = Some((Hint::Rewind, self.clock.elapsed()));
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

    /// The mode and plan state the footer shows: where the presses so far lead, not yet all
    /// confirmed by wizard.
    pub fn shown_mode(&self) -> (String, bool) {
        self.mode_target
            .clone()
            .unwrap_or_else(|| (self.cfg.mode.clone(), self.plan))
    }

    /// shift+tab: the next of wizard's modes, then plan, then back. Wizard takes one change at
    /// a time and none during a turn, so the steps queue and each waits for its answer.
    fn cycle_mode(&mut self) {
        let modes = self.cfg.modes.clone();
        let (mode, plan) = self.shown_mode();
        let at = modes.iter().position(|m| *m == mode).unwrap_or(0);
        let next = if plan {
            (modes.first().cloned().unwrap_or(mode.clone()), false)
        } else if at + 1 < modes.len() {
            (modes[at + 1].clone(), false)
        } else {
            (mode.clone(), true)
        };
        if next.1 != plan {
            self.mode_steps.push_back(ModeStep::Plan);
        }
        if next.0 != mode {
            self.mode_steps.push_back(ModeStep::Mode(next.0.clone()));
        }
        self.mode_target = Some(next);
        self.pump_modes();
    }

    fn pump_modes(&mut self) {
        if self.mode_waiting.is_some() || self.busy.is_some() {
            return;
        }
        match self.mode_steps.pop_front() {
            Some(ModeStep::Plan) => {
                self.mode_waiting = Some(ModeStep::Plan);
                self.toggle_plan();
            }
            Some(ModeStep::Mode(m)) => {
                self.mode_waiting = Some(ModeStep::Mode(m.clone()));
                self.send(Request::SetMode(m));
            }
            None => self.mode_target = None,
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
                self.effort_preview = None;
                self.show_effort_notice();
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
            // Esc on a permission prompt turns the call down and stops the turn
            Some(Dialog::Permission { req, .. }) => {
                self.amend = None;
                self.send(Request::Decide {
                    id: req.id,
                    allow: false,
                    scope: DecideScope::Once,
                    note: String::new(),
                });
                self.send(Request::Cancel);
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
        if let (Some(mut note), Dialog::Permission { req, sel }) = (self.amend.clone(), &dialog) {
            match key.code {
                KeyCode::Esc => self.amend = None,
                KeyCode::Enter => {
                    let choice = permission_options(req)[*sel].1;
                    self.decide(req.clone(), choice);
                }
                KeyCode::Backspace => {
                    note.pop();
                    self.amend = Some(note);
                }
                KeyCode::Char(c) => {
                    note.push(c);
                    self.amend = Some(note);
                }
                _ => {}
            }
            return;
        }
        if key.code == KeyCode::Esc {
            return self.close_dialog();
        }
        match dialog {
            Dialog::Help { tab, sel, focus } => {
                let n = crate::ui::dialogs::sorted_commands(self)
                    .iter()
                    .filter(|c| self.custom.contains(&c.name) == (tab == 2))
                    .count();
                let (tab, sel, focus) = match key.code {
                    KeyCode::Tab | KeyCode::Right => ((tab + 1) % 3, 0, false),
                    KeyCode::BackTab | KeyCode::Left => ((tab + 2) % 3, 0, false),
                    KeyCode::Down if tab > 0 && n > 0 && !focus => (tab, sel, true),
                    KeyCode::Down if tab > 0 && n > 0 => (tab, (sel + 1) % n, true),
                    KeyCode::Up if tab > 0 && focus && sel == 0 => (tab, 0, false),
                    KeyCode::Up if tab > 0 && focus => (tab, sel - 1, true),
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
                            self.from_queue.clear();
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
                    KeyCode::Tab => self.amend = Some(String::new()),
                    _ => {}
                }
            }
            Dialog::Rewind { sel } => {
                let n = self.rewind_targets().len() + 1;
                let at = sel.min(n - 1);
                match key.code {
                    KeyCode::Up => {
                        self.dialog = Some(Dialog::Rewind {
                            sel: at.saturating_sub(1),
                        })
                    }
                    KeyCode::Down => {
                        self.dialog = Some(Dialog::Rewind {
                            sel: (at + 1).min(n - 1),
                        })
                    }
                    KeyCode::Enter => {
                        self.dialog = None;
                        if at + 1 < n {
                            self.start_rewind(at);
                        }
                    }
                    _ => {}
                }
            }
            Dialog::Search { mut query, sel } => {
                let n = self.search_hits(&query).len();
                match key.code {
                    KeyCode::Up => {
                        self.dialog = Some(Dialog::Search {
                            query,
                            sel: (sel + 1).min(n.saturating_sub(1)),
                        })
                    }
                    KeyCode::Down => {
                        self.dialog = Some(Dialog::Search {
                            query,
                            sel: sel.saturating_sub(1),
                        })
                    }
                    KeyCode::Enter => {
                        let hit = self
                            .search_hits(&query)
                            .get(sel)
                            .map(|&i| self.prompts[i].clone());
                        self.dialog = None;
                        if let Some(text) = hit {
                            self.editor.set_text(&text);
                            self.refresh_menu();
                        }
                    }
                    KeyCode::Backspace => {
                        query.pop();
                        self.dialog = Some(Dialog::Search { query, sel: 0 });
                    }
                    KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.search_here = !self.search_here;
                        self.dialog = Some(Dialog::Search { query, sel: 0 });
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        query.push(c);
                        self.dialog = Some(Dialog::Search { query, sel: 0 });
                    }
                    _ => {}
                }
            }
        }
    }

    /// Earlier prompts matching `query`, newest first, as indexes into `prompts`.
    pub fn search_hits(&self, query: &str) -> Vec<usize> {
        let q = query.to_lowercase();
        let here = self.opts.cwd.to_string_lossy();
        (0..self.prompts.len())
            .rev()
            .filter(|&i| self.prompts[i].to_lowercase().contains(&q))
            .filter(|&i| !self.search_here || self.prompt_dirs.get(i).is_some_and(|d| *d == here))
            .collect()
    }

    /// The prompts the rewind picker offers, oldest first: what was typed, not commands.
    pub fn rewind_targets(&self) -> Vec<String> {
        self.blocks
            .iter()
            .filter_map(|b| match b {
                Block::User { text, bash: false } if !text.starts_with('/') => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// The files the `nth` rewind prompt's turn changed: name, lines added, lines removed.
    pub fn changes_after(&self, nth: usize) -> Vec<(String, usize, usize)> {
        let mut seen = 0;
        let mut inside = false;
        let mut out: Vec<(String, usize, usize)> = Vec::new();
        for b in &self.blocks {
            match b {
                Block::User { text, bash: false } if !text.starts_with('/') => {
                    if inside {
                        break;
                    }
                    inside = seen == nth;
                    seen += 1;
                }
                Block::Tool(t) if inside => {
                    let Some(d) = &t.diff else { continue };
                    let old = d.old.clone().unwrap_or_default();
                    let diff = similar::TextDiff::from_lines(&old, &d.new);
                    let (mut a, mut r) = (0, 0);
                    for ch in diff.iter_all_changes() {
                        match ch.tag() {
                            similar::ChangeTag::Insert => a += 1,
                            similar::ChangeTag::Delete => r += 1,
                            similar::ChangeTag::Equal => {}
                        }
                    }
                    let name = d.path.rsplit('/').next().unwrap_or(&d.path).to_string();
                    match out.iter_mut().find(|(n, _, _)| *n == name) {
                        Some(e) => {
                            e.1 += a;
                            e.2 += r;
                        }
                        None => out.push((name, a, r)),
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Ask wizard to go back to before the `nth` prompt the picker lists. Wizard numbers its
    /// turns in the session file; the prompt is found there by its first line.
    fn start_rewind(&mut self, nth: usize) {
        let targets = self.rewind_targets();
        let Some(text) = targets.get(nth).cloned() else {
            return;
        };
        let markers = std::env::var_os("HOME")
            .map(|h| {
                rewind_markers(
                    &PathBuf::from(h)
                        .join(".wizard/sessions")
                        .join(format!("{}.jsonl", self.session_id)),
                )
            })
            .unwrap_or_default();
        let first = |s: &str| s.lines().next().unwrap_or("").trim().to_string();
        let earlier = targets[..nth]
            .iter()
            .filter(|t| first(t) == first(&text))
            .count();
        let turn = markers
            .iter()
            .filter(|(_, p)| first(p) == first(&text))
            .nth(earlier)
            .map(|(t, _)| *t);
        match turn {
            Some(turn) => {
                self.rewinding = Some(text);
                self.rewind_answer = None;
                self.start_busy();
                self.send(Request::Prompt(format!("/rewind {turn}")));
            }
            None => self.blocks.push(Block::Notice {
                level: NoticeLevel::Error,
                text: "Cannot rewind: wizard has no checkpoint for that prompt".into(),
            }),
        }
    }

    fn finish_rewind(&mut self, text: String) {
        self.busy = None;
        let answer = self.rewind_answer.take().unwrap_or_default();
        if answer.starts_with("rewound to before turn") {
            self.blocks.clear();
            self.from_queue.clear();
            self.send(Request::LoadSession(self.session_id.clone()));
            self.editor.set_text(&text);
        } else {
            let why = if answer.is_empty() {
                "wizard did not answer".to_string()
            } else {
                answer
            };
            self.blocks.push(Block::Notice {
                level: NoticeLevel::Error,
                text: format!("Rewind failed: {why}"),
            });
        }
    }

    fn decide(&mut self, req: PermissionRequest, choice: Choice) {
        self.dialog = None;
        // an approved plan ends plan mode in wizard
        if is_plan(&req) && choice != Choice::No {
            self.plan = false;
        }
        let note = self.amend.take().unwrap_or_default();
        self.send(Request::Decide {
            id: req.id,
            allow: choice != Choice::No,
            scope: if choice == Choice::Always {
                DecideScope::Always
            } else {
                DecideScope::Once
            },
            note,
        });
    }

    fn show_effort_notice(&mut self) {
        self.notice = self.effort_notice();
        self.notice_until = Some(self.clock.elapsed() + NOTICE_TTL);
    }

    fn pick_model(&mut self, i: usize) {
        let Some(m) = self.cfg.models.get(i).cloned() else {
            return;
        };
        self.dialog = None;
        self.show_effort_notice();
        let effort = self.effort_preview.take().filter(|e| *e != self.cfg.effort);
        self.blocks.push(Block::User {
            text: "/model ".into(),
            bash: false,
        });
        if m.id == self.cfg.model && effort.is_none() {
            self.output(format!("Kept model as {}", m.name), m.name);
            return;
        }
        if m.id != self.cfg.model {
            self.send(Request::SetModel(m.id.clone()));
            self.cfg.model = m.id;
        }
        let mut text = format!("Set model to {} for this session only", m.name);
        if let Some(e) = effort {
            self.send(Request::SetEffort(e.clone()));
            self.cfg.effort = e.clone();
            self.show_effort_notice();
            text.push_str(&format!(" with {e} effort"));
        }
        self.output(text, m.name);
    }

    /// ←/→ in the model picker: only what the picker shows changes until a pick is confirmed.
    fn step_effort(&mut self, step: isize) {
        let n = self.cfg.efforts.len() as isize;
        if n == 0 {
            return;
        }
        let current = self
            .effort_preview
            .clone()
            .unwrap_or_else(|| self.cfg.effort.clone());
        let at = self
            .cfg
            .efforts
            .iter()
            .position(|e| *e == current)
            .unwrap_or(0) as isize;
        self.effort_preview = Some(self.cfg.efforts[(at + step).clamp(0, n - 1) as usize].clone());
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
        if !self.fixed_now {
            self.now_unix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(self.now_unix, |d| d.as_secs() as i64);
        }
        let now = self.clock.elapsed();
        if self.notice_until.is_some_and(|until| now >= until) {
            self.notice = None;
            self.notice_until = None;
            self.dirty = true;
        }
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
        let elapsed = self.clock.elapsed();
        let hint = self.hint.map(|(_, at)| at + HINT_TTL);
        [hint, self.notice_until]
            .into_iter()
            .flatten()
            .min()
            .map(|at| now + at.saturating_sub(elapsed).max(Duration::from_millis(10)))
    }

    /// What is printed to the normal screen after the UI is gone.
    pub fn epilogue(&self) -> String {
        if self.blocks.is_empty() || self.session_id.is_empty() {
            return "\n".to_string();
        }
        format!(
            "\n\x1b[2mResume this session with:\x1b[0m\n\x1b[2m{}\x1b[0m\n",
            self.resume_command()
        )
    }

    /// What the user types to come back. Started by `wizard` (it sets CLAUDEW_WIZARD_BIN),
    /// that is `wizard --resume`, which opens the session just closed, the latest in this
    /// directory. Run on its own, it is this binary with the id.
    pub fn resume_command(&self) -> String {
        if std::env::var_os("CLAUDEW_WIZARD_BIN").is_some_and(|b| !b.is_empty()) {
            "wizard --resume".to_string()
        } else {
            format!("wizard-ui-claude --resume {}", self.session_id)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Yes,
    Always,
    No,
}

/// The numbered answers a permission prompt offers.
/// Wizard's one question: a finished plan from `exit_plan`.
pub fn is_plan(req: &PermissionRequest) -> bool {
    req.input.get("plan").is_some_and(|p| p.is_string())
}

pub fn permission_options(req: &PermissionRequest) -> Vec<(String, Choice)> {
    if is_plan(req) {
        return vec![
            ("Yes, proceed".to_string(), Choice::Yes),
            ("No, keep planning".to_string(), Choice::No),
        ];
    }
    let mut v = vec![("Yes".to_string(), Choice::Yes)];
    if !req.rule.is_empty() {
        let what = match req.kind {
            ToolKind::Edit => "Yes, allow all edits during this session".to_string(),
            _ => format!("Yes, and don\u{2019}t ask again for: {}", req.rule),
        };
        v.push((what, Choice::Always));
    }
    v.push(("No".to_string(), Choice::No));
    v
}

/// An edit's diff with the whole file around it. Wizard sends only the replaced snippet, so
/// it is found in the file: the new text once the edit is made, the old text before. With the
/// file on both sides the diff shows real line numbers and the lines around the change.
pub fn place_diff(cwd: &Path, d: agent_core::FileDiff) -> agent_core::FileDiff {
    let Some(old) = d.old.clone().filter(|o| !o.is_empty() && !d.new.is_empty()) else {
        return d;
    };
    let path = if Path::new(&d.path).is_absolute() {
        PathBuf::from(&d.path)
    } else {
        cwd.join(&d.path)
    };
    let Ok(file) = std::fs::read_to_string(&path) else {
        return d;
    };
    if file == d.new || file == old {
        return d;
    }
    let (before, after) = if file.contains(&d.new) && !file.contains(&old) {
        (file.replacen(&d.new, &old, 1), file)
    } else if file.contains(&old) {
        let after = file.replacen(&old, &d.new, 1);
        (file, after)
    } else {
        return d;
    };
    agent_core::FileDiff {
        path: d.path,
        old: Some(before),
        new: after,
    }
}

/// `(turn, prompt)` pairs from a wizard session file, for `/rewind <turn>`.
pub fn rewind_markers(path: &Path) -> Vec<(u64, String)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).ok()?;
            Some((
                v.get("turn")?.as_u64()?,
                v.get("prompt")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

/// Files in `dir` ending in `.{ext}`, by name: (stem, contents).
fn read_dir_sorted(dir: &Path, ext: &str) -> Vec<(String, String)> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<(String, String)> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension()? != ext {
                return None;
            }
            let stem = p.file_stem()?.to_string_lossy().into_owned();
            Some((stem, std::fs::read_to_string(&p).ok()?))
        })
        .collect();
    v.sort();
    v
}

/// `key: value` from a markdown file's front matter.
fn front_matter(text: &str, key: &str) -> Option<String> {
    let body = text.strip_prefix("---")?;
    let end = body.find("\n---")?;
    body[..end].lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

/// A top-level `key = "value"` from a TOML file.
fn toml_str(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        let v = v.trim();
        (k.trim() == key && v.starts_with('"')).then(|| v.trim_matches('"').to_string())
    })
}

/// `◐ medium · /effort`, the notice over the prompt; none for wizard's unnamed default.
pub fn effort_notice_text(effort: &str) -> Option<String> {
    if matches!(effort, "" | "default") {
        return None;
    }
    Some(format!("{} {effort} · /effort", effort_glyph(effort)))
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
/// A mention entry that names a subagent: the marker, then name and description joined by a
/// tab.
pub const AGENT: char = '\u{1}';

/// Paths and subagents for an `@` query: the top level and then the agents when nothing is
/// typed, otherwise directories that start with it, up to two agents that match, then the
/// other paths.
pub fn mention_matches(files: &[String], agents: &[(String, String)], query: &str) -> Vec<String> {
    const MAX: usize = 15;
    let agent = |(n, d): &(String, String)| format!("{AGENT}{n}\t{d}");
    if query.is_empty() {
        let mut top: Vec<&String> = files
            .iter()
            .filter(|f| !f.trim_end_matches('/').contains('/'))
            .collect();
        top.sort_by_key(|f| (!f.ends_with('/'), f.to_lowercase()));
        let mut out: Vec<String> = top.into_iter().cloned().collect();
        out.extend(agents.iter().map(agent));
        out.truncate(MAX);
        return out;
    }
    let q = query.to_lowercase();
    let mut ranked: Vec<(u8, usize, String)> = files
        .iter()
        .filter_map(|f| {
            let low = f.to_lowercase();
            let base = low.trim_end_matches('/').rsplit('/').next().unwrap_or("");
            let rank = if low.starts_with(&q) && f.ends_with('/') {
                0
            } else if low.starts_with(&q) {
                3
            } else if base.starts_with(&q) {
                4
            } else if low.contains(&q) {
                5
            } else {
                return None;
            };
            Some((rank, f.len(), f.clone()))
        })
        .collect();
    // at most two agents between the paths that start with the query and the rest
    let mut found: Vec<(u8, String)> = agents
        .iter()
        .filter_map(|a| {
            let low = a.0.to_lowercase();
            let rank = if low.starts_with(&q) {
                1
            } else if low.contains(&q) {
                2
            } else {
                return None;
            };
            Some((rank, agent(a)))
        })
        .collect();
    found.sort_by_key(|(rank, _)| *rank);
    ranked.extend(found.into_iter().take(2).map(|(rank, a)| (rank, 0, a)));
    ranked.sort_by_key(|x| (x.0, x.1));
    ranked.into_iter().take(MAX).map(|(_, _, f)| f).collect()
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

/// The editor as Claude Code names it in its hints: `Nvim` for nvim, `nano` as it is.
pub fn editor_display() -> String {
    let name = editor_name();
    match name.as_str() {
        "nvim" => "Nvim".into(),
        "vim" => "Vim".into(),
        "emacs" | "emacsclient" => "Emacs".into(),
        "code" => "VS Code".into(),
        "hx" => "Helix".into(),
        _ => name,
    }
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
        BackendKind::Wizard => backend_wizard::WizardBackend::spawn_with(cwd, resume, true)?,
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
    if !app.link_spots.is_empty() {
        use crossterm::cursor::{MoveTo, RestorePosition, SavePosition};
        use crossterm::style::{Print, ResetColor, SetForegroundColor};
        let mut out = std::io::stdout();
        let _ = crossterm::queue!(out, SavePosition);
        for (x, y, text, url, color) in &app.link_spots {
            let _ = crossterm::queue!(out, MoveTo(*x, *y), Print(format!("\x1b]8;;{url}\x1b\\")));
            if let ratatui::style::Color::Indexed(n) = color {
                let _ = crossterm::queue!(
                    out,
                    SetForegroundColor(crossterm::style::Color::AnsiValue(*n))
                );
            }
            let _ = crossterm::queue!(out, Print(text), ResetColor, Print("\x1b]8;;\x1b\\"));
        }
        let _ = crossterm::queue!(out, RestorePosition);
        let _ = std::io::Write::flush(&mut out);
    }
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
