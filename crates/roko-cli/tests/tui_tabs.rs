//! Verify F8 Marketplace, F9 Atelier, and F10 Learning tabs are fully wired.

use roko_cli::tui::tabs::Tab;
use roko_cli::tui::views::{SubView, ViewState};

#[test]
fn tab_all_has_ten_entries() {
    assert_eq!(Tab::ALL.len(), 10);
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
fn next_prev_cycle_ten_tabs() {
    let mut t = Tab::Dashboard;
    for _ in 0..10 {
        t = t.next();
    }
    assert_eq!(t, Tab::Dashboard);

    for _ in 0..10 {
        t = t.prev();
    }
    assert_eq!(t, Tab::Dashboard);
}

#[test]
fn learning_tab_basics() {
    assert_eq!(Tab::Learning.fkey(), crossterm::event::KeyCode::F(10));
    assert_eq!(
        Tab::from_key(crossterm::event::KeyCode::F(10)),
        Some(Tab::Learning)
    );
    assert_eq!(Tab::Learning.label(), "Learning");
    assert_eq!(Tab::Learning.index(), 9);
}

#[test]
fn learning_has_subviews() {
    let subs = SubView::for_tab(Tab::Learning);
    assert!(!subs.is_empty());
    assert!(subs.iter().any(|s| s.label() == "Router"));
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
fn fkey_roundtrip_all_ten() {
    for tab in Tab::ALL {
        assert_eq!(Tab::from_key(tab.fkey()), Some(tab));
    }
}

#[test]
fn index_sequential_all_ten() {
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
