// OWNER: pickers (model picker, resume picker, selection list)
//! Selection popups and the session pickers (spec C.7, A.15).
//!
//! `selection` is the list view every popup shares, `resume` the full-screen session picker,
//! `status_card` the `/status` cell. This file wires them to the app: `/model` and its effort
//! step, `/resume`, the startup resume flows.

pub mod hooks;
pub mod keymap;
pub mod multi_select;
pub mod names;
pub mod prompt;
pub mod resume;
pub mod selection;
pub mod status_card;
pub mod theme;

use std::io::Write;
use std::sync::Arc;

use agent_core::{Config, ModelOption, Request};

use crate::app::App;
use crate::ui::AppAction;
use crate::ui::history_cells::{ErrorCell, InfoCell};
use selection::{ItemAction, ListSelectionView, SelectionItem, SelectionParams};

/// A cell of ready-made lines, wrapped to the width without a hanging indent.
#[derive(Debug)]
pub struct PlainLinesCell {
    pub lines: Vec<ratatui::text::Line<'static>>,
}

impl crate::ui::HistoryCell for PlainLinesCell {
    fn display_lines(&self, width: u16) -> Vec<ratatui::text::Line<'static>> {
        let opts = crate::wrap::WrapOpts::new(width.max(1) as usize);
        self.lines
            .iter()
            .flat_map(|l| crate::wrap::word_wrap_line(l, &opts))
            .collect()
    }
}

/// What `codexw resume` asked for before the chat starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartPick {
    Picker,
    Last,
}

/// What Codex's catalog says about each effort; wizard has no per-model list, so this is the
/// text for the five levels it offers.
fn effort_label(effort: &str) -> String {
    match effort {
        "default" => "Default".into(),
        "none" => "None".into(),
        "minimal" => "Minimal".into(),
        "low" => "Low".into(),
        "medium" => "Medium".into(),
        "high" => "High".into(),
        "xhigh" => "Extra high".into(),
        other => other.to_string(),
    }
}

fn effort_description(effort: &str) -> Option<&'static str> {
    Some(match effort {
        "default" => "Use the model's own reasoning level",
        "low" => "Fast responses with lighter reasoning",
        "medium" => "Balances speed and reasoning depth for everyday tasks",
        "high" => "Greater reasoning depth for complex problems",
        "xhigh" => "Extra high reasoning depth for complex problems",
        _ => return None,
    })
}

pub const EFFORT_NOTE: &str =
    "Wizard sends the effort to models that accept one and ignores it on the rest.";

/// Step 2 of `/model`: the effort for `model` (spec C.7.3 step 3).
pub fn effort_params(cfg: &Config, model: &ModelOption) -> SelectionParams {
    let on_current_model = cfg.model == model.id;
    let highlight = if on_current_model {
        cfg.effort.clone()
    } else {
        "default".to_string()
    };
    let items: Vec<SelectionItem> = cfg
        .efforts
        .iter()
        .map(|e| {
            let mut it = SelectionItem::new(
                effort_label(e),
                ItemAction::Close(AppAction::ChangeModel {
                    model: Some(model.id.clone()),
                    effort: Some(e.clone()),
                }),
            );
            it.description = effort_description(e).map(String::from);
            it.is_current = on_current_model && *e == highlight;
            it
        })
        .collect();
    let initial = cfg.efforts.iter().position(|e| *e == highlight);
    SelectionParams {
        title: Some(format!("Select Reasoning Level for {}", model.name)),
        footer_note: Some(ratatui::text::Line::from(ratatui::style::Stylize::dim(
            EFFORT_NOTE,
        ))),
        items,
        initial_selected_idx: initial,
        ..Default::default()
    }
}

/// Step 1 of `/model`: every model the backend lists (spec C.7.3 step 2).
pub fn model_params(cfg: &Config) -> SelectionParams {
    let has_efforts = !cfg.efforts.is_empty();
    let items = cfg
        .models
        .iter()
        .map(|m| {
            let action = if has_efforts {
                let cfg = cfg.clone();
                let m = m.clone();
                ItemAction::Child(Arc::new(move || effort_params(&cfg, &m)))
            } else {
                ItemAction::Close(AppAction::ChangeModel {
                    model: Some(m.id.clone()),
                    effort: None,
                })
            };
            let mut it = SelectionItem::new(m.name.clone(), action);
            if !m.provider.is_empty() {
                it.description = Some(m.provider.clone());
            }
            it.is_current = m.id == cfg.model;
            it
        })
        .collect();
    SelectionParams {
        title: Some("Select Model and Effort".into()),
        subtitle: Some("Start with a different one by running codexw -m <provider>/<model>".into()),
        items,
        ..Default::default()
    }
}

/// One row of wizard's `/ui` listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Look {
    pub name: String,
    pub description: String,
}

/// This look's name in wizard's list.
pub const THIS_LOOK: &str = "codex";

/// The looks in wizard's answer to bare `/ui`, or `None` when `text` is not that listing:
///
/// ```text
/// ui: codex — the codex layout: >_ banner, › prompts, a bare composer
///   · wizard  the house look: one rule, no boxes, nothing decorative (default)
///   ● codex  the codex layout: >_ banner, › prompts, a bare composer
/// switch with /ui <name>. a full look restarts into that UI
/// ```
pub fn parse_looks(text: &str) -> Option<Vec<Look>> {
    if !text.trim_start().starts_with("ui:") {
        return None;
    }
    let looks: Vec<Look> = text
        .lines()
        .filter_map(|l| {
            // `●` marks the saved look, `·` the rest
            let row = l.trim_start().strip_prefix(['●', '·'])?.trim();
            let (name, description) = row.split_once(char::is_whitespace).unwrap_or((row, ""));
            (!name.is_empty()).then(|| Look {
                name: name.to_string(),
                description: description.trim().to_string(),
            })
        })
        .collect();
    (!looks.is_empty()).then_some(looks)
}

/// `/ui`: the looks wizard lists. Enter on one runs `/ui <name>`. The row marked current is
/// this look, whatever wizard has saved: started with a one-off override, the saved look is
/// another one and has to stay pickable.
pub fn look_params(looks: &[Look]) -> SelectionParams {
    let items = looks
        .iter()
        .map(|l| {
            let current = l.name == THIS_LOOK;
            let action = if current {
                ItemAction::None
            } else {
                ItemAction::Close(AppAction::Submit(format!("/ui {}", l.name)))
            };
            let mut it = SelectionItem::new(l.name.clone(), action);
            if !l.description.is_empty() {
                it.description = Some(l.description.clone());
            }
            it.is_current = current;
            it
        })
        .collect();
    SelectionParams {
        title: Some("Select a Look".into()),
        subtitle: Some("Wizard leaves this look and starts the one you pick.".into()),
        items,
        ..Default::default()
    }
}

impl<W: Write> App<W> {
    /// Bare `/ui`: ask wizard for its looks; `show_look_picker` opens on the answer.
    pub fn open_look_picker(&mut self) {
        if !self.ready {
            self.push_cell(Box::new(InfoCell {
                text: "Look selection is disabled until startup completes.".into(),
                hint: None,
            }));
            return;
        }
        self.look_pick = true;
        self.wizard_command("/ui".into());
    }

    pub fn show_look_picker(&mut self, looks: &[Look]) {
        self.pane
            .push_view(Box::new(ListSelectionView::new(look_params(looks))));
        self.request_draw();
    }

    /// `/model [id]`: the model picker, or switch straight to a named model.
    pub fn open_model_picker(&mut self) {
        self.open_model_picker_with("");
    }

    pub fn open_model_picker_with(&mut self, rest: &str) {
        if !self.ready {
            self.push_cell(Box::new(InfoCell {
                text: "Model selection is disabled until startup completes.".into(),
                hint: None,
            }));
            return;
        }
        if self.config.models.is_empty() {
            self.push_cell(Box::new(InfoCell {
                text: "No additional models are available right now.".into(),
                hint: None,
            }));
            return;
        }
        if !rest.is_empty() {
            let found = self
                .config
                .models
                .iter()
                .find(|m| m.id == rest || m.name == rest)
                .map(|m| m.id.clone());
            match found {
                Some(id) => self.change_model(Some(id), None),
                None => self.push_cell(Box::new(ErrorCell {
                    text: format!("No model named '{rest}'. Run /model to see the list."),
                })),
            }
            return;
        }
        let params = model_params(&self.config);
        self.pane
            .push_view(Box::new(ListSelectionView::new(params)));
        self.request_draw();
    }

    /// Apply a pick from the model picker. Wizard refuses option changes mid-turn, so say so
    /// instead of letting the request fail quietly.
    pub fn change_model(&mut self, model: Option<String>, effort: Option<String>) {
        if self.running {
            self.push_cell(Box::new(ErrorCell {
                text: "Wizard changes the model between turns. Wait for the turn to end, then try again.".into(),
            }));
            return;
        }
        let name = match &model {
            Some(id) => self
                .config
                .models
                .iter()
                .find(|m| m.id == *id)
                .map_or_else(|| id.clone(), |m| m.name.clone()),
            None => self.model_name(),
        };
        if let Some(m) = model {
            let _ = self.tx.send(Request::SetModel(m));
        }
        let label = match &effort {
            Some(e) => {
                let _ = self.tx.send(Request::SetEffort(e.clone()));
                e.clone()
            }
            None => self.config.effort.clone(),
        };
        let mut text = format!("Model changed to {name}");
        if !label.is_empty() {
            text.push(' ');
            text.push_str(&label);
        }
        self.push_cell(Box::new(InfoCell { text, hint: None }));
    }

    /// `/resume [id]`: ask for the session list; `show_sessions` opens the picker.
    pub fn open_resume(&mut self, rest: &str) {
        if !rest.is_empty() {
            let id = self
                .home
                .as_deref()
                .and_then(|h| names::id_for(h, rest))
                .unwrap_or_else(|| rest.to_string());
            self.apply_action(AppAction::LoadSession(id));
            return;
        }
        self.want_sessions = Some(String::new());
        let _ = self.tx.send(Request::ListSessions);
    }

    /// The session list arrived: open the picker, or resolve `codexw resume --last`.
    pub fn show_sessions(&mut self, _query: &str) {
        match self.start_pick {
            Some(StartPick::Last) => {
                let cwd = self.opts.cwd.display().to_string();
                let newest = self
                    .sessions
                    .iter()
                    .filter(|s| s.cwd.trim_end_matches('/') == cwd.trim_end_matches('/'))
                    .max_by_key(|s| s.updated)
                    .map(|s| s.id.clone());
                match newest {
                    Some(id) => self.load_session(id),
                    None => self.start_fresh(),
                }
            }
            _ => {
                let startup = self.start_pick.is_some();
                let picker = resume::ResumePicker::new(&self.sessions, self.resume_opts(startup));
                self.open_overlay(Box::new(picker));
            }
        }
    }

    fn resume_opts(&self, startup: bool) -> resume::ResumeOpts {
        let home = self.home.as_deref().map(std::path::PathBuf::from);
        let config_path = home.as_ref().map(|h| h.join(".config/codexw/config.toml"));
        let density = config_path
            .as_ref()
            .and_then(|p| crate::config::get(p, "session_picker_view"))
            .filter(|v| v == "comfortable")
            .map_or(resume::Density::Dense, |_| resume::Density::Comfortable);
        resume::ResumeOpts {
            startup,
            cwd: self.opts.cwd.display().to_string(),
            now: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64),
            sessions_dir: home.map(|h| h.join(".wizard/sessions")),
            density,
            names: self.home.as_deref().map(names::load).unwrap_or_default(),
            config_path,
        }
    }

    /// Open a saved session. The old one's summary lands after the replay once the backend
    /// confirms (spec A.15.5); at startup there is nothing to summarise.
    pub fn load_session(&mut self, id: String) {
        let startup = self.start_pick.take().is_some();
        if !startup {
            self.switch = Some(crate::app::SessionSwitch {
                clear: false,
                load: true,
            });
        }
        let _ = self.tx.send(Request::LoadSession(id));
    }

    /// `/rename [name]`: name this session (client-only, kept in a sidecar file).
    pub fn rename_command(&mut self, rest: &str) {
        if rest.is_empty() {
            let view = prompt::PromptView::new(
                "Name thread",
                "Type a name and press Enter",
                None,
                Arc::new(AppAction::RenameSession),
            );
            self.pane.push_view(Box::new(view));
            self.request_draw();
        } else {
            self.rename_session(rest.to_string());
        }
    }

    pub fn rename_session(&mut self, name: String) {
        use ratatui::style::Stylize;
        use ratatui::text::{Line, Span};
        let name = name.trim().to_string();
        if name.is_empty() {
            self.push_cell(Box::new(ErrorCell {
                text: "Thread name cannot be empty.".into(),
            }));
            return;
        }
        let Some(home) = self.home.clone() else {
            self.push_cell(Box::new(ErrorCell {
                text: "Could not find a home directory to keep the name in.".into(),
            }));
            return;
        };
        if self.session_id.is_empty() {
            self.push_cell(Box::new(ErrorCell {
                text: "Rename is available once the session has started.".into(),
            }));
            return;
        }
        if let Err(e) = names::set(&home, &self.session_id, &name) {
            self.push_cell(Box::new(ErrorCell {
                text: format!("Failed to rename the session: {e}"),
            }));
            return;
        }
        let hint = names::resume_hint(Some(&name), &self.session_id);
        let line = Line::from(vec![
            Span::from("• Session renamed to "),
            Span::from(name).cyan(),
            Span::from(". To resume this session run "),
            Span::from(hint).cyan(),
        ]);
        self.push_cell(Box::new(PlainLinesCell { lines: vec![line] }));
    }

    /// `/status`: the card (spec B.13.4).
    pub fn push_status_card(&mut self) {
        let cfg = &self.config;
        let provider = cfg
            .models
            .iter()
            .find(|m| m.id == cfg.model)
            .map(|m| m.provider.clone())
            .filter(|p| !p.is_empty());
        let chat = cfg.mode == "chat";
        let data = status_card::StatusData {
            model: self.model_name(),
            effort: cfg.effort.clone(),
            provider,
            directory: self.cwd_display(),
            permissions: if chat {
                "Chat mode (no file or shell tools)".into()
            } else {
                "Full Access".into()
            },
            approval: "never (wizard does not ask)".into(),
            agents: status_card::agents_summary(&self.opts.cwd),
            mode: Some(cfg.mode.clone()).filter(|m| !m.is_empty()),
            session: Some(self.session_id.clone()).filter(|s| !s.is_empty()),
            usage: self.usage.clone(),
        };
        self.push_cell(Box::new(status_card::StatusCell { data }));
    }

    /// The startup picker was dismissed, or `--last` found nothing: carry on with the new session.
    pub fn start_fresh(&mut self) {
        self.start_pick = None;
        self.finish_ready();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{BottomView, ViewResult};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn cfg() -> Config {
        let m = |p: &str, n: &str| ModelOption {
            id: format!("{p}/{n}"),
            name: n.into(),
            provider: p.into(),
        };
        Config {
            model: "xai-oauth/grok-4.6".into(),
            models: vec![m("xai-oauth", "grok-4.6"), m("chatgpt", "gpt-5.6-sol")],
            effort: "medium".into(),
            efforts: ["default", "low", "medium", "high", "xhigh"]
                .map(String::from)
                .to_vec(),
            ..Default::default()
        }
    }

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    const LISTING: &str = "ui: codex \u{2014} the codex layout: >_ banner, \u{203a} prompts, a bare composer\n  \u{b7} wizard  the house look: one rule, no boxes, nothing decorative (default)\n  \u{25cf} codex  the codex layout: >_ banner, \u{203a} prompts, a bare composer\n  \u{b7} grok  the grok layout: a colored bar down every block, boxed composer\nswitch with /ui <name>. a full look restarts into that UI";

    #[test]
    fn look_listing_becomes_rows() {
        let looks = parse_looks(LISTING).unwrap();
        let names: Vec<&str> = looks.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["wizard", "codex", "grok"]);
        assert_eq!(
            looks[0].description,
            "the house look: one rule, no boxes, nothing decorative (default)"
        );
        assert_eq!(parse_looks("saved [ui] skin = \"grok\"."), None);
        assert_eq!(parse_looks("error: could not read config"), None);
        assert_eq!(parse_looks("ui: nothing listed"), None);
    }

    #[test]
    fn look_picker_starts_on_this_look_and_enter_sends_the_switch() {
        // Wizard has `wizard` saved here; the row marked current is still codex.
        let listing = LISTING
            .replace('\u{25cf}', "\u{b7}")
            .replacen('\u{b7}', "\u{25cf}", 1);
        assert_eq!(parse_looks(&listing), parse_looks(LISTING));
        let mut v = ListSelectionView::new(look_params(&parse_looks(LISTING).unwrap()));
        assert_eq!(v.selected_index(), Some(1));
        // The look already running: Enter only closes.
        assert_eq!(v.handle_key(key(KeyCode::Enter)), ViewResult::Close);
        let mut v = ListSelectionView::new(look_params(&parse_looks(LISTING).unwrap()));
        v.handle_key(key(KeyCode::Down));
        assert_eq!(
            v.handle_key(key(KeyCode::Enter)),
            ViewResult::CloseWith(AppAction::Submit("/ui grok".into()))
        );
    }

    #[test]
    fn model_list_marks_current_and_starts_there() {
        let v = ListSelectionView::new(model_params(&cfg()));
        assert_eq!(v.selected_index(), Some(0));
        let mut c = cfg();
        c.model = "chatgpt/gpt-5.6-sol".into();
        let v = ListSelectionView::new(model_params(&c));
        assert_eq!(v.selected_index(), Some(1));
    }

    #[test]
    fn enter_on_a_model_opens_the_effort_step_then_applies_both() {
        let mut v = ListSelectionView::new(model_params(&cfg()));
        v.handle_key(key(KeyCode::Down));
        assert_eq!(v.handle_key(key(KeyCode::Enter)), ViewResult::Pending);
        // Another model: the highlight starts on Default.
        assert_eq!(v.selected_index(), Some(0));
        v.handle_key(key(KeyCode::Down));
        v.handle_key(key(KeyCode::Down));
        let r = v.handle_key(key(KeyCode::Enter));
        assert_eq!(
            r,
            ViewResult::CloseWith(AppAction::ChangeModel {
                model: Some("chatgpt/gpt-5.6-sol".into()),
                effort: Some("medium".into()),
            })
        );
    }

    #[test]
    fn effort_step_on_the_current_model_highlights_the_current_effort() {
        let c = cfg();
        let p = effort_params(&c, &c.models[0]);
        let v = ListSelectionView::new(p);
        assert_eq!(v.selected_index(), Some(2));
        assert_eq!(v.desired_height(100), 11);
    }
}
