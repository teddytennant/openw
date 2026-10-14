//! Keys and backend events that change state rather than pixels.

mod common;

use agent_core::{Event, NoticeLevel, Request, StopReason};
use claudew::app::{Block, Dialog};
use common::Harness;
use crossterm::event::KeyCode;

fn harness() -> Harness {
    Harness::new("static-120x40.jsonl", 120, 40)
}

fn plan_answer(hn: &mut common::Harness, text: &str) {
    for ev in [
        Event::TurnStart,
        Event::Notice {
            level: NoticeLevel::Info,
            text: text.into(),
        },
        Event::TurnEnd(StopReason::EndTurn),
    ] {
        hn.event(ev);
    }
}

fn mode_confirmed(hn: &mut common::Harness, mode: &str) {
    let mut cfg = hn.app.cfg.clone();
    cfg.mode = mode.into();
    hn.event(Event::ConfigChanged(cfg));
}

#[test]
fn shift_tab_walks_the_modes_then_plan_and_leaves_no_trace() {
    let mut hn = harness();
    hn.press(KeyCode::BackTab);
    assert_eq!(hn.sent(), [Request::SetMode("accept edits".into())]);
    hn.press(KeyCode::BackTab);
    assert!(
        hn.sent().is_empty(),
        "the second step waits for wizard's answer"
    );
    assert_eq!(hn.app.mode_index(), None, "the footer already shows plan");
    mode_confirmed(&mut hn, "accept edits");
    assert_eq!(hn.sent(), [Request::Prompt("/plan".into())]);
    plan_answer(&mut hn, "plan mode on");
    assert!(hn.app.plan);
    assert!(hn.app.blocks.is_empty(), "{:?}", hn.app.blocks);
    assert!(hn.app.busy.is_none());
    hn.press(KeyCode::BackTab);
    assert_eq!(hn.sent(), [Request::Prompt("/plan".into())]);
    plan_answer(&mut hn, "plan mode off");
    assert_eq!(hn.sent(), [Request::SetMode("manual".into())]);
    mode_confirmed(&mut hn, "manual");
    assert_eq!(hn.app.mode_index(), Some(0));
    assert!(hn.app.blocks.is_empty(), "{:?}", hn.app.blocks);
}

#[test]
fn a_refused_mode_change_is_retried_once_the_turn_ends() {
    let mut hn = harness();
    hn.press(KeyCode::BackTab);
    assert_eq!(hn.sent(), [Request::SetMode("accept edits".into())]);
    hn.event(Event::Notice {
        level: NoticeLevel::Info,
        text: "Options can only be changed between turns.".into(),
    });
    assert!(hn.app.blocks.is_empty(), "wizard's refusal is not printed");
    hn.event(Event::TurnStart);
    hn.event(Event::TurnEnd(StopReason::EndTurn));
    assert_eq!(hn.sent(), [Request::SetMode("accept edits".into())]);
    assert_eq!(hn.app.mode_index(), Some(1));
}

#[test]
fn ctrl_c_clears_then_exits_and_the_window_closes() {
    let mut hn = harness();
    hn.type_str("draft");
    hn.ctrl('c');
    assert!(hn.app.editor.is_empty() && !hn.app.quit);
    hn.advance(900);
    hn.ctrl('c');
    assert!(!hn.app.quit, "the first press's window had closed");
    hn.advance(300);
    hn.ctrl('c');
    assert!(hn.app.quit);
}

#[test]
fn escape_twice_quickly_clears_the_prompt() {
    let mut hn = harness();
    hn.type_str("abc");
    hn.press(KeyCode::Esc);
    hn.advance(900);
    hn.press(KeyCode::Esc);
    assert_eq!(
        hn.app.editor.text(),
        "abc",
        "too slow to count as a double press"
    );
    hn.advance(300);
    hn.press(KeyCode::Esc);
    assert!(hn.app.editor.is_empty());
}

#[test]
fn enter_in_the_slash_menu_runs_the_selection() {
    let mut hn = harness();
    hn.type_str("/mo");
    hn.press(KeyCode::Enter);
    assert!(matches!(hn.app.dialog, Some(Dialog::Model { .. })));
    hn.press(KeyCode::Esc);
    assert!(matches!(
        hn.app.blocks.last(),
        Some(Block::Output { text, .. }) if text == "Kept model as Haiku 5.5"
    ));
}

#[test]
fn a_backend_command_goes_out_and_its_answer_hangs_under_it() {
    let mut hn = harness();
    hn.type_str("/compact");
    hn.press(KeyCode::Enter);
    assert_eq!(hn.sent(), [Request::Prompt("/compact".into())]);
    hn.event(Event::TurnStart);
    hn.event(Event::Notice {
        level: NoticeLevel::Info,
        text: "Compacted (ctrl+o to see full summary)".into(),
    });
    hn.event(Event::TurnEnd(StopReason::EndTurn));
    assert!(matches!(
        &hn.app.blocks[..],
        [Block::User { .. }, Block::Output { dim: true, .. }]
    ));
}

#[test]
fn switching_to_another_look_quits_this_one() {
    let mut hn = harness();
    hn.event(Event::Notice {
        level: NoticeLevel::Info,
        text: "saved [ui] skin = \"grok\". Quit this look to switch now.".into(),
    });
    assert!(hn.app.quit);
}

#[test]
fn a_permission_answer_goes_back_with_its_id() {
    let mut hn = Harness::new("turn-120x40.jsonl", 120, 40);
    hn.type_str("go");
    hn.press(KeyCode::Enter);
    hn.play();
    hn.sent();
    hn.type_str("edit it");
    hn.press(KeyCode::Enter);
    hn.play();
    assert!(matches!(hn.app.dialog, Some(Dialog::Permission { .. })));
    hn.press(KeyCode::Char('3'));
    let sent = hn.sent();
    assert!(
        sent.iter()
            .any(|r| matches!(r, Request::Decide { id, allow: false, .. } if id == "p1")),
        "{sent:?}"
    );
}

#[test]
fn the_commands_the_look_answers_are_listed_before_the_backend_is_up() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let (mtx, _mrx) = tokio::sync::mpsc::unbounded_channel();
    let app = claudew::app::App::new(Default::default(), tx, mtx);
    let names: Vec<&str> = app.commands.iter().map(|c| c.name.as_str()).collect();
    assert!(
        names.contains(&"help") && names.contains(&"resume"),
        "{names:?}"
    );
}

#[test]
fn esc_on_a_permission_prompt_turns_it_down_and_stops_the_turn() {
    let mut hn = Harness::new("turn2-120x40.jsonl", 120, 40);
    hn.type_str("md");
    hn.press(KeyCode::Enter);
    hn.play();
    hn.type_str("git");
    hn.press(KeyCode::Enter);
    hn.play();
    assert!(matches!(hn.app.dialog, Some(Dialog::Permission { .. })));
    hn.sent();
    hn.press(KeyCode::Esc);
    let sent = hn.sent();
    assert!(
        matches!(sent.first(), Some(Request::Decide { allow: false, .. })),
        "{sent:?}"
    );
    assert_eq!(sent.last(), Some(&Request::Cancel));
    hn.event(Event::TurnEnd(StopReason::Cancelled));
    let n = hn.app.blocks.len();
    assert!(matches!(
        &hn.app.blocks[n - 2],
        Block::Output { muted: true, text, .. } if text.starts_with("Interrupted")
    ));
    assert!(matches!(hn.app.blocks[n - 1], Block::TurnEnd { .. }));
}

#[test]
fn tab_on_a_permission_prompt_sends_a_note_with_the_answer() {
    let mut hn = Harness::new("turn2-120x40.jsonl", 120, 40);
    hn.type_str("md");
    hn.press(KeyCode::Enter);
    hn.play();
    hn.type_str("git");
    hn.press(KeyCode::Enter);
    hn.play();
    hn.sent();
    hn.press(KeyCode::Tab);
    hn.type_str("use -b main");
    hn.press(KeyCode::Enter);
    let sent = hn.sent();
    assert!(
        matches!(sent.first(), Some(Request::Decide { allow: true, note, .. }) if note == "use -b main"),
        "{sent:?}"
    );
}

#[test]
fn the_effort_notice_goes_after_a_while_and_comes_back_for_a_turn() {
    let mut hn = harness();
    assert!(hn
        .app
        .notice
        .as_deref()
        .is_some_and(|n| n.contains("/effort")));
    hn.advance(31_000);
    assert_eq!(hn.app.notice, None);
    hn.type_str("hi");
    hn.press(KeyCode::Enter);
    assert!(hn.app.notice.is_some(), "shown again while the turn runs");
}

#[test]
fn the_header_and_status_show_wizards_version() {
    let mut hn = harness();
    let v = common::replay_version();
    assert_eq!(hn.app.wizard_version(), v);
    let screen = hn.render().plain();
    assert!(screen.contains(&format!("Wizard v{v}")), "{screen}");
    hn.type_str("/status");
    hn.press(KeyCode::Enter);
    let screen = hn.render().plain();
    assert!(
        screen.contains(&format!("Version:           {v}")),
        "{screen}"
    );
}

#[test]
fn before_wizard_answers_the_header_names_no_version() {
    let mut hn = harness();
    hn.app.cfg.backend.clear();
    assert_eq!(hn.app.wizard_version(), "");
    let screen = hn.render().plain();
    assert!(
        !screen.contains(&format!("v{}", claudew::app::VERSION)),
        "{screen}"
    );
}

#[test]
fn ctrl_r_finds_an_earlier_prompt() {
    let mut hn = harness();
    for p in ["alpha one", "beta two"] {
        hn.app.submit(p.to_string());
        hn.event(Event::TurnEnd(StopReason::EndTurn));
    }
    hn.ctrl('r');
    hn.type_str("alp");
    hn.press(KeyCode::Enter);
    assert_eq!(hn.app.editor.text(), "alpha one");
}

#[test]
fn completing_a_command_shows_what_it_takes() {
    let mut hn = Harness::new("wizard-120x40.jsonl", 120, 40);
    hn.type_str("/mod");
    hn.press(KeyCode::Tab);
    assert_eq!(hn.app.editor.text(), "/model ");
    let screen = hn.render().plain();
    assert!(screen.contains("❯\u{a0}/model  [tag]"), "{screen}");
}

#[test]
fn page_up_shows_the_way_back_down() {
    let mut hn = harness();
    for n in 0..30 {
        hn.app.submit(format!("prompt {n}"));
        hn.event(Event::TurnEnd(StopReason::EndTurn));
    }
    hn.render();
    hn.press(KeyCode::PageUp);
    assert!(hn.render().plain().contains("Jump to bottom (ctrl+End) ↓"));
    hn.key(KeyCode::End, crossterm::event::KeyModifiers::CONTROL);
    assert!(!hn.render().plain().contains("Jump to bottom"));
}

#[test]
fn after_a_fatal_error_nothing_looks_busy_and_the_exits_work() {
    let mut hn = harness();
    hn.type_str("hello");
    hn.press(KeyCode::Enter);
    hn.sent();
    hn.event(Event::Fatal("wizard exited".into()));
    assert!(hn.app.busy.is_none());
    let notices: Vec<_> = hn
        .app
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Notice { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(
        notices.iter().any(|t| t.contains("wizard setup")),
        "{notices:?}"
    );
    hn.type_str("again");
    hn.press(KeyCode::Enter);
    assert!(hn.sent().is_empty(), "nothing goes to a dead backend");
    assert!(hn.app.busy.is_none());
    hn.type_str("/exit");
    hn.press(KeyCode::Enter);
    assert!(hn.app.quit);
}

#[test]
fn ctrl_c_during_a_turn_stops_it_and_a_second_press_exits() {
    let mut hn = harness();
    hn.type_str("hello");
    hn.press(KeyCode::Enter);
    hn.event(Event::TurnStart);
    hn.sent();
    hn.ctrl('c');
    assert_eq!(hn.sent(), [Request::Cancel]);
    hn.advance(200);
    hn.ctrl('c');
    assert!(hn.app.quit);
}

#[test]
fn exit_is_not_queued_behind_a_turn() {
    let mut hn = harness();
    hn.type_str("hello");
    hn.press(KeyCode::Enter);
    hn.event(Event::TurnStart);
    hn.type_str("/exit");
    hn.press(KeyCode::Enter);
    assert!(hn.app.quit);
}

#[test]
fn an_interrupt_without_tools_leaves_no_done_line() {
    let mut hn = harness();
    hn.type_str("hello");
    hn.press(KeyCode::Enter);
    hn.event(Event::TurnStart);
    hn.event(Event::TextDelta("partial".into()));
    hn.press(KeyCode::Esc);
    hn.event(Event::TurnEnd(StopReason::Cancelled));
    assert!(
        !hn.app
            .blocks
            .iter()
            .any(|b| matches!(b, Block::TurnEnd { .. })),
        "{:?}",
        hn.app.blocks
    );
}

#[test]
fn a_queued_prompt_shows_survives_esc_and_comes_back_on_up() {
    let mut hn = harness();
    hn.type_str("first");
    hn.press(KeyCode::Enter);
    hn.event(Event::TurnStart);
    hn.type_str("queued one");
    hn.press(KeyCode::Enter);
    let screen = hn.render().plain();
    assert!(screen.contains("❯ queued one"), "{screen}");
    assert!(screen.contains("ctrl+x ctrl+s to send now"), "{screen}");
    assert!(
        screen.contains("Press up to edit queued messages"),
        "{screen}"
    );
    hn.press(KeyCode::Up);
    assert_eq!(hn.app.editor.text(), "queued one");
    assert!(hn.app.queued.is_empty());
    hn.press(KeyCode::Enter);
    hn.sent();
    hn.press(KeyCode::Esc);
    assert_eq!(hn.sent(), [Request::Cancel]);
    hn.event(Event::TurnEnd(StopReason::Cancelled));
    assert_eq!(hn.sent(), [Request::Prompt("queued one".into())]);
}

#[test]
fn effort_arrows_only_preview_until_the_pick_is_confirmed() {
    let mut hn = harness();
    hn.app.cfg.efforts = vec!["low".into(), "medium".into(), "high".into()];
    hn.app.cfg.effort = "medium".into();
    hn.type_str("/model");
    hn.press(KeyCode::Enter);
    hn.sent();
    hn.press(KeyCode::Right);
    assert!(hn.sent().is_empty());
    assert_eq!(hn.app.cfg.effort, "medium");
    hn.press(KeyCode::Esc);
    assert_eq!(hn.app.cfg.effort, "medium");
    hn.type_str("/model");
    hn.press(KeyCode::Enter);
    hn.press(KeyCode::Right);
    hn.press(KeyCode::Enter);
    assert!(hn.sent().contains(&Request::SetEffort("high".into())));
    let screen = hn.render().plain();
    assert!(
        screen.contains("for this session only with high effort"),
        "{screen}"
    );
}

#[test]
fn a_finished_plan_asks_and_the_answer_goes_back() {
    let mut hn = harness();
    hn.app.plan = true;
    hn.event(Event::TurnStart);
    hn.event(Event::Permission(agent_core::PermissionRequest {
        id: "9".into(),
        tool: "exit_plan".into(),
        kind: agent_core::ToolKind::Other,
        title: "Ready to code?".into(),
        input: serde_json::json!({ "plan": "1. Add `// entry point` to main.rs." }),
        diff: None,
        rule: String::new(),
    }));
    let screen = hn.render().plain();
    assert!(screen.contains("Ready to code?"), "{screen}");
    assert!(screen.contains("// entry point"), "{screen}");
    assert!(screen.contains("❯ 1. Yes, proceed"), "{screen}");
    assert!(screen.contains("2. No, keep planning"), "{screen}");
    hn.sent();
    hn.press(KeyCode::Enter);
    assert!(matches!(
        hn.sent().as_slice(),
        [Request::Decide { id, allow: true, .. }] if id == "9"
    ));
    assert!(!hn.app.plan, "an approved plan leaves plan mode");
}

#[test]
fn streaming_text_hides_the_spinner() {
    let mut hn = harness();
    hn.type_str("count");
    hn.press(KeyCode::Enter);
    hn.event(Event::TurnStart);
    assert!(
        hn.render().plain().contains("…"),
        "the spinner shows before any text"
    );
    hn.event(Event::TextDelta("1\n2\n3".into()));
    let screen = hn.render().plain();
    assert!(
        !screen.contains("esc to interrupt)") && !screen.contains("…"),
        "{screen}"
    );
}

#[test]
fn an_edit_snippet_is_placed_in_its_file() {
    let dir = std::env::temp_dir().join(format!("claudew-place-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("main.rs"),
        "fn main() {\n    println!(\"hi\");\n}\n",
    )
    .unwrap();
    let placed = claudew::app::place_diff(
        &dir,
        agent_core::FileDiff {
            path: "main.rs".into(),
            old: Some("hello".into()),
            new: "hi".into(),
        },
    );
    assert_eq!(
        placed.old.as_deref(),
        Some("fn main() {\n    println!(\"hello\");\n}\n")
    );
    assert_eq!(placed.new, "fn main() {\n    println!(\"hi\");\n}\n");
    std::fs::remove_dir_all(&dir).unwrap();
}
