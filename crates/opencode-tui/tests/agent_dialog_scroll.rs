//! Agent-select dialog scrolling.
//!
//! The dialog caps its height at 12 rows, so with 30 agents only ~10 fit. Navigation
//! has to keep the selection inside that window. This asserts the rendered buffer
//! actually contains the selected agent after repeated `move_down`.

use opencode_tui::components::{Agent, AgentSelectDialog};
use ratatui::{backend::TestBackend, Terminal};

const W: u16 = 120;
const H: u16 = 40;

fn dialog_with(n: usize) -> AgentSelectDialog {
    let agents = (0..n)
        .map(|i| Agent {
            name: format!("agent-{i:02}"),
            description: "d".into(),
            color: ratatui::style::Color::Cyan,
        })
        .collect();
    let mut dlg = AgentSelectDialog::new();
    dlg.set_agents(agents);
    dlg
}

fn selected_visible(dlg: &AgentSelectDialog, term: &mut Terminal<TestBackend>) -> bool {
    term.draw(|frame| dlg.render(frame, frame.size(), &opencode_tui::theme::Theme::default()))
        .unwrap();
    let buf = term.backend().buffer();
    let want = dlg.selected_agent().map(|a| a.name.clone()).unwrap_or_default();
    (0..H).any(|y| {
        let row: String = (0..W).map(|x| buf.get(x, y).symbol()).collect();
        row.contains(&want)
    })
}

#[test]
fn selected_agent_stays_visible_after_navigating_past_the_window() {
    let mut dlg = dialog_with(30);
    dlg.open();
    let mut term = Terminal::new(TestBackend::new(W, H)).unwrap();

    assert!(selected_visible(&dlg, &mut term), "agent 0 must render");

    let mut invisible_at = None;
    for _ in 0..29 {
        dlg.move_down();
        if !selected_visible(&dlg, &mut term) {
            invisible_at = Some(dlg.selected_agent().map(|a| a.name.clone()));
            break;
        }
    }
    assert!(
        invisible_at.is_none(),
        "selection scrolled out of the dialog window at {invisible_at:?}"
    );
}

#[test]
fn every_agent_is_visible_at_once_on_a_tall_terminal() {
    let mut dlg = dialog_with(30);
    dlg.open();
    let mut term = Terminal::new(TestBackend::new(W, H)).unwrap();
    term.draw(|frame| dlg.render(frame, frame.size(), &opencode_tui::theme::Theme::default()))
        .unwrap();

    let buf = term.backend().buffer();
    let missing: Vec<String> = (0..30)
        .map(|i| format!("agent-{i:02}"))
        .filter(|name| {
            !(0..H).any(|y| {
                let row: String = (0..W).map(|x| buf.get(x, y).symbol()).collect();
                row.contains(name)
            })
        })
        .collect();
    assert!(missing.is_empty(), "not rendered: {missing:?}");
}

#[test]
fn move_down_reaches_the_last_agent() {
    let mut dlg = dialog_with(30);
    dlg.open();
    for _ in 0..40 {
        dlg.move_down();
    }
    assert_eq!(
        dlg.selected_agent().map(|a| a.name.clone()),
        Some("agent-29".into()),
        "move_down must clamp at the last agent, not stop early"
    );
}