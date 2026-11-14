//! The **Data & Viz** tab: its cards in page order (docs/CARDS.md). One file
//! per card; a batch edits only its own card files and its line here.

crate::showcase::tab_cards! {
    table: Table => "table",
    pagination: Pagination => "pagination",
    charts: Charts => "charts",
    timeline: Timeline => "timeline",
    sortable: Sortable => "sortable",
    kanban: Kanban => "kanban",
    node_graph: NodeGraph => "node-graph",
}
