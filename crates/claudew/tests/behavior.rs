//! Keys and backend events that change state rather than pixels.

mod common;

use agent_core::{Event, NoticeLevel, Request, StopReason};
use claudew::app::{Block, Dialog};
use common::Harness;
use crossterm::event::KeyCode;

fn harness() -> Harness {
    Harness::new("static-120x40.jsonl", 120, 40)
}

#[test]
fn shift_tab_walks_the_modes_then_plan_and_leaves_no_trace() {
    let mut hn = harness();
    hn.press(KeyCode::BackTab);
    assert_eq!(hn.sent(), [Request::SetMode("accept edits".into())]);
    hn.press(KeyCode::BackTab);
    assert_eq!(hn.sent(), [Request::Prompt("/plan".into())]);
    for ev in [
        Event::TurnStart,
        Event::Notice {
            level: NoticeLevel::Info,
            text: "plan mode on".into(),
        },
        Event::TurnEnd(StopReason::EndTurn),
    ] {
        hn.event(ev);
    }
    assert!(hn.app.plan);
    assert!(hn.app.blocks.is_empty(), "{:?}", hn.app.blocks);
    assert!(hn.app.busy.is_none());
    hn.press(KeyCode::BackTab);
    let sent = hn.sent();
    assert!(sent.contains(&Request::Prompt("/plan".into())), "{sent:?}");
    assert!(
        sent.contains(&Request::SetMode("manual".into())),
        "{sent:?}"
    );
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
