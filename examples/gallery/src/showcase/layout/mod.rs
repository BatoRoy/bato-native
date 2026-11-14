//! The **Layout** tab: its cards in page order (docs/CARDS.md). One file
//! per card; a batch edits only its own card files and its line here.

crate::showcase::tab_cards! {
    primitives: Primitives => "primitives",
    responsive: Responsive => "responsive",
    scrollable: Scrollable => "scrollable",
    editor_tabs: EditorTabs => "editor-tabs",
    resizable: Resizable => "resizable",
    app_shell: AppShell => "app-shell",
}

pub mod selftest;
