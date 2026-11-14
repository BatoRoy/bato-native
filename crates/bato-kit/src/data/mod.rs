//! Data and charts (B6): Pagination, DataTable, Timeline, Line/Area/Bar/Sparkline; SortableList
//! and KanbanBoard (B7).
//!
//! Phase 3 batches add one file per component here and one `mod` line below.

// B6 data and charts.
pub mod charts;
pub mod draw;
pub mod pagination;
pub mod table;
pub mod timeline;

pub use charts::{Bar, Series, XType, area_chart, bar_chart, line_chart, sparkline};
pub use pagination::pagination;
pub use table::{ActionTone, RowAction, TableMsg, TableState, Value, column_def, data_table};
pub use timeline::{TimelineItem, timeline};
