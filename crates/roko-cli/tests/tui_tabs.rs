//! Verify F8 Marketplace and F9 Atelier tabs are fully wired.

use roko_cli::tui::tabs::Tab;
use roko_cli::tui::views::{SubView, ViewState};

#[test]
fn tab_all_has_nine_entries() {
    assert_eq!(Tab::ALL.len(), 9);
}

#[test]
fn marketplace_tab_basics() {
    assert_eq!(Tab::Marketplace.fkey(), crossterm::event::KeyCode::F(8));
    assert_eq!(
        Tab::from_key(crossterm::event::KeyCode::F(8)),
        Some(Tab::Marketplace)
    );
    assert_eq!(Tab::Marketplace.label(), "Marketplace");
    assert_eq!(Tab::Marketplace.index(), 7);
}

#[test]
fn atelier_tab_basics() {
    assert_eq!(Tab::Atelier.fkey(), crossterm::event::KeyCode::F(9));
    assert_eq!(
        Tab::from_key(crossterm::event::KeyCode::F(9)),
        Some(Tab::Atelier)
    );
    assert_eq!(Tab::Atelier.label(), "Atelier");
    assert_eq!(Tab::Atelier.index(), 8);
}

#[test]
fn next_prev_cycle_nine_tabs() {
    let mut t = Tab::Dashboard;
    for _ in 0..9 {
        t = t.next();
    }
    assert_eq!(t, Tab::Dashboard);

    for _ in 0..9 {
        t = t.prev();
    }
    assert_eq!(t, Tab::Dashboard);
}

#[test]
fn marketplace_has_subviews() {
    let subs = SubView::for_tab(Tab::Marketplace);
    assert!(!subs.is_empty());
    assert!(subs.iter().any(|s| s.label() == "Jobs"));
}

#[test]
fn atelier_has_subviews() {
    let subs = SubView::for_tab(Tab::Atelier);
    assert!(!subs.is_empty());
    assert!(subs.iter().any(|s| s.label() == "PRDs"));
}

#[test]
fn fkey_roundtrip_all_nine() {
    for tab in Tab::ALL {
        assert_eq!(Tab::from_key(tab.fkey()), Some(tab));
    }
}

#[test]
fn index_sequential_all_nine() {
    for (i, tab) in Tab::ALL.iter().enumerate() {
        assert_eq!(tab.index(), i);
    }
}

#[test]
fn view_state_resolves_marketplace_subview() {
    let vs = ViewState {
        sub_tab: 0,
        ..Default::default()
    };
    let sub = vs.active_sub_view(Tab::Marketplace);
    assert_eq!(sub.label(), "Jobs");
}

#[test]
fn view_state_resolves_atelier_subview() {
    let vs = ViewState {
        sub_tab: 0,
        ..Default::default()
    };
    let sub = vs.active_sub_view(Tab::Atelier);
    assert_eq!(sub.label(), "PRDs");
}
