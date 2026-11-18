//! Layout helpers (INF-13): what CSS grid/flex give React and iced's
//! `row`/`column` cannot express.
//!
//! | CSS | Kit |
//! |---|---|
//! | `grid grid-cols-N gap-G`, `col-span-2`, `grid-cols-[3rem_repeat(7,1fr)]`, `auto-rows-fr` | [`grid()`] / [`Grid`] |
//! | `grid-cols-[repeat(auto-fill,minmax(140px,1fr))]` (AutoGrid) | [`Grid::auto_fill`] |
//! | `columns-3` + `break-inside-avoid` (Masonry) | [`masonry()`] |
//! | `flex-col items-stretch`, shrink-wrapped (the toast stack) | [`stretch_column()`] |
//! | `aspect-video` / `aspect-[4/3]` | [`aspect_ratio()`] |
//! | `absolute left-… top-… w-… h-…` (calendar events, timeline bars, dashboard) | [`absolute()`] |
//! | `border-t` / `border-l` hairlines, fixed gaps | [`hline`], [`vline`], [`gap`], [`vgap`] |
//! | the page container (`mx-auto max-w-[--app-content-width] p-…`) | [`page()`] |
//! | `Stack`/`HStack`/`VStack`, `Center`, `Container`, `Divider`, `Spacer` | [`hstack`], [`vstack`], [`center`], [`sized_container`], [`divider`], [`divider_v`], [`divider_label`], [`spacer`] |
//! | `Collapsible`'s `grid-rows-[0fr]` → `[1fr]` height animation | [`collapse()`] |
//! | `rounded-lg overflow-hidden` around content with its own fills | [`rounded_clip()`] |
//! | `AppShell` (header, sidebar, footer, scrolling main) | [`app_shell()`] |
//! | `PanelGroup` / `ResizablePanels` (draggable splits) | [`panel_group()`], [`resizable_panels()`] |
//! | `EditorTabs` (sortable, closable tab strip) | [`editor_tabs()`] |
//! | `Dashboard` (editable widget grid, Snap / Free) | [`dashboard()`] |
//! | `flex flex-col` + `mt-auto` (a card's footer pinned in a stretched grid row) | [`fill_end()`] |
//! | `sticky top-0` (a bar pinned to the top of the page scroller) | [`sticky()`] |
//!
//! Every cell is stretched like CSS's default `align-items: stretch`.

mod absolute;
mod app_shell;
mod aspect;
mod clip;
mod collapse;
pub mod dashboard;
mod editor_tabs;
mod fill_end;
mod grid;
mod lines;
mod masonry;
mod panels;
mod primitives;
mod sticky;
mod stretch;

pub use absolute::{Absolute, absolute};
pub use aspect::{AspectRatio, aspect_ratio};
pub use fill_end::{FillEnd, fill_end};
pub use grid::{Grid, Track, grid};
pub use lines::{gap, hline, page, vgap, vline};
pub use masonry::{Masonry, masonry};
pub use sticky::{Sticky, sticky};
pub use stretch::{StretchColumn, stretch_column};

pub use app_shell::{AppShell, app_shell};
pub use clip::{RoundedClip, rounded_clip};
pub use collapse::{COLLAPSE, Collapse, collapse};
pub use dashboard::{Dashboard, DashboardItem, DashboardMsg, Mode, WidgetType, dashboard};
pub use editor_tabs::{EditorTab, EditorTabs, editor_tabs};
pub use panels::{
    Direction, PanelGroup, bar_size, drag_sizes, initial_sizes, panel_group, resizable_panels,
};
pub use primitives::{
    ContainerSize, center, divider, divider_label, divider_v, hstack, sized_container, spacer,
    units, vstack,
};

/// The child-forwarding parts of a widget whose children are
/// `self.cells: Vec<Element>` laid out one node per cell, in order.
macro_rules! forward_cells {
    () => {
        fn children(&self) -> Vec<iced::advanced::widget::Tree> {
            self.cells
                .iter()
                .map(iced::advanced::widget::Tree::new)
                .collect()
        }
        fn diff(&self, tree: &mut iced::advanced::widget::Tree) {
            tree.diff_children(&self.cells);
        }
        fn update(
            &mut self,
            tree: &mut iced::advanced::widget::Tree,
            event: &iced::Event,
            layout: iced::advanced::Layout<'_>,
            cursor: iced::mouse::Cursor,
            renderer: &iced::Renderer,
            clipboard: &mut dyn iced::advanced::Clipboard,
            shell: &mut iced::advanced::Shell<'_, Message>,
            viewport: &iced::Rectangle,
        ) {
            for ((cell, t), l) in self
                .cells
                .iter_mut()
                .zip(tree.children.iter_mut())
                .zip(layout.children())
            {
                cell.as_widget_mut()
                    .update(t, event, l, cursor, renderer, clipboard, shell, viewport);
            }
        }
        fn mouse_interaction(
            &self,
            tree: &iced::advanced::widget::Tree,
            layout: iced::advanced::Layout<'_>,
            cursor: iced::mouse::Cursor,
            viewport: &iced::Rectangle,
            renderer: &iced::Renderer,
        ) -> iced::mouse::Interaction {
            self.cells
                .iter()
                .zip(&tree.children)
                .zip(layout.children())
                .map(|((c, t), l)| {
                    c.as_widget()
                        .mouse_interaction(t, l, cursor, viewport, renderer)
                })
                .max()
                .unwrap_or_default()
        }
        fn operate(
            &mut self,
            tree: &mut iced::advanced::widget::Tree,
            layout: iced::advanced::Layout<'_>,
            renderer: &iced::Renderer,
            operation: &mut dyn iced::advanced::widget::Operation,
        ) {
            operation.traverse(&mut |operation| {
                for ((cell, t), l) in self
                    .cells
                    .iter_mut()
                    .zip(tree.children.iter_mut())
                    .zip(layout.children())
                {
                    cell.as_widget_mut().operate(t, l, renderer, operation);
                }
            });
        }
        fn draw(
            &self,
            tree: &iced::advanced::widget::Tree,
            renderer: &mut iced::Renderer,
            theme: &iced::Theme,
            style: &iced::advanced::renderer::Style,
            layout: iced::advanced::Layout<'_>,
            cursor: iced::mouse::Cursor,
            viewport: &iced::Rectangle,
        ) {
            for ((cell, t), l) in self.cells.iter().zip(&tree.children).zip(layout.children()) {
                cell.as_widget()
                    .draw(t, renderer, theme, style, l, cursor, viewport);
            }
        }
        fn overlay<'b>(
            &'b mut self,
            tree: &'b mut iced::advanced::widget::Tree,
            layout: iced::advanced::Layout<'b>,
            renderer: &iced::Renderer,
            viewport: &iced::Rectangle,
            translation: iced::Vector,
        ) -> Option<iced::advanced::overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
            iced::advanced::overlay::from_children(
                &mut self.cells,
                tree,
                layout,
                renderer,
                viewport,
                translation,
            )
        }
    };
}
pub(crate) use forward_cells;

/// `impl From<$ty> for Element`.
macro_rules! into_element {
    ($ty:ident) => {
        impl<'a, Message: 'a> From<$ty<'a, Message>>
            for iced::Element<'a, Message, iced::Theme, iced::Renderer>
        {
            fn from(w: $ty<'a, Message>) -> Self {
                iced::Element::new(w)
            }
        }
    };
}
pub(crate) use into_element;
