//! `Icon`: lucide icons (0.383.0, `assets/icons`, exported by
//! `scripts/export-icons.cjs`), drawn through iced's SVG pipeline (resvg).
//!
//! Colour goes through iced's colour filter, which replaces RGB and keeps
//! alpha: that also turns resvg's premultiplied pixels into the straight alpha
//! iced's image pipeline blends with (`SrcAlpha`). Writing the colour into the
//! SVG instead leaves premultiplied data that iced multiplies by alpha a second
//! time, which thins every anti-aliased edge (measured: coverage ≈ a², strokes
//! ~0.3 px thinner than Chromium's). Bounds are snapped to device pixels so the
//! raster is never resampled.
//!
//! Adding an icon: export it with `scripts/export-icons.cjs` and add one line
//! to the `icons!` list below.

use std::cell::RefCell;
use std::collections::HashMap;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::svg::{self, Renderer as _};
use iced::advanced::widget::{Tree, Widget};
use iced::{Color, Element, Length, Radians, Rectangle, Size, mouse};

use crate::anim::{current_opacity, epoch};
use crate::scale::snap_rect;

macro_rules! icons {
    ($($name:ident => $file:literal,)*) => {
        /// Every lucide icon the Hybrid components use.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Ico {
            $($name,)*
        }

        impl Ico {
            pub const ALL: &[Ico] = &[$(Ico::$name,)*];

            /// The lucide file name (`chevron-down`).
            pub fn name(self) -> &'static str {
                match self {
                    $(Ico::$name => $file,)*
                }
            }

            /// The SVG source (24-unit box, stroke 2, `#000000`).
            pub fn src(self) -> &'static str {
                match self {
                    $(Ico::$name => include_str!(concat!("../../../assets/icons/", $file, ".svg")),)*
                }
            }
        }
    };
}

icons! {
    Activity => "activity",
    AlertCircle => "alert-circle",
    AlertTriangle => "alert-triangle",
    BarChart3 => "bar-chart-3",
    Bell => "bell",
    Blocks => "blocks",
    Box => "box",
    Calendar => "calendar",
    CalendarClock => "calendar-clock",
    Check => "check",
    CheckCircle => "check-circle",
    ChevronDown => "chevron-down",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    ChevronUp => "chevron-up",
    ChevronsUpDown => "chevrons-up-down",
    Clipboard => "clipboard",
    CloudMoon => "cloud-moon",
    Command => "command",
    Copy => "copy",
    Download => "download",
    Eye => "eye",
    EyeOff => "eye-off",
    FileCode => "file-code",
    FileText => "file-text",
    GripVertical => "grip-vertical",
    Home => "home",
    Info => "info",
    LayoutGrid => "layout-grid",
    ListChecks => "list-checks",
    Mail => "mail",
    Minus => "minus",
    Moon => "moon",
    MoreHorizontal => "more-horizontal",
    Palette => "palette",
    PanelLeftClose => "panel-left-close",
    PanelLeftOpen => "panel-left-open",
    Pencil => "pencil",
    Pipette => "pipette",
    Plug => "plug",
    Plus => "plus",
    RefreshCw => "refresh-cw",
    Rows3 => "rows-3",
    Save => "save",
    Scissors => "scissors",
    Search => "search",
    Server => "server",
    Settings => "settings",
    Star => "star",
    SunMedium => "sun-medium",
    Trash2 => "trash-2",
    Upload => "upload",
    Volume1 => "volume-1",
    Volume2 => "volume-2",
    VolumeX => "volume-x",
    X => "x",
    XCircle => "x-circle",
}

/// The SVG source with `fill`/`stroke-width` overrides applied.
fn source(ico: Ico, filled: bool, stroke: Option<f32>) -> String {
    let mut s = ico.src().to_string();
    if filled {
        s = s.replacen("fill=\"none\"", "fill=\"#000000\"", 1);
    }
    if let Some(w) = stroke {
        // Only the root's stroke-width (lucide sets it once, on <svg>).
        s = s.replacen("stroke-width=\"2\"", &format!("stroke-width=\"{w}\""), 1);
    }
    s
}

type Key = (Ico, bool, Option<u32>);

thread_local! {
    static HANDLES: RefCell<HashMap<Key, svg::Handle>> = RefCell::new(HashMap::new());
}

fn handle(ico: Ico, filled: bool, stroke: Option<f32>) -> svg::Handle {
    HANDLES.with(|h| {
        h.borrow_mut()
            .entry((ico, filled, stroke.map(f32::to_bits)))
            .or_insert_with(|| svg::Handle::from_memory(source(ico, filled, stroke).into_bytes()))
            .clone()
    })
}

pub struct Icon {
    ico: Ico,
    size: f32,
    color: Option<Color>,
    filled: bool,
    stroke: Option<f32>,
    rotation: f32,
    spin: bool,
}

/// A lucide icon `size` px square, in the inherited text colour.
pub fn icon(ico: Ico, size: f32) -> Icon {
    Icon {
        ico,
        size,
        color: None,
        filled: false,
        stroke: None,
        rotation: 0.0,
        spin: false,
    }
}

impl Icon {
    pub fn color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }
    pub fn color_maybe(mut self, c: Option<Color>) -> Self {
        self.color = c;
        self
    }
    /// `fill="currentColor"` (the filled star).
    pub fn filled(mut self) -> Self {
        self.filled = true;
        self
    }
    /// lucide `strokeWidth` (default 2), e.g. 3 for the Checkbox tick.
    pub fn stroke_width(mut self, w: f32) -> Self {
        self.stroke = (w != 2.0).then_some(w);
        self
    }
    /// Static rotation in radians (a chevron turned 180°).
    pub fn rotation(mut self, r: f32) -> Self {
        self.rotation = r;
        self
    }
    /// `animate-spin`: one turn per second, linear. Asks for frames only
    /// while on.
    pub fn spin(mut self, on: bool) -> Self {
        self.spin = on;
        self
    }
}

impl<Message, Theme> Widget<Message, Theme, iced::Renderer> for Icon {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(self.size), Length::Fixed(self.size))
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        _limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(Size::new(self.size, self.size))
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &iced::Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut iced::advanced::Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        if self.spin
            && matches!(
                event,
                iced::Event::Window(iced::window::Event::RedrawRequested(_))
            )
        {
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let color = self.color.unwrap_or(style.text_color);
        let b = snap_rect(layout.bounds());
        let rotation = if self.spin {
            (epoch().elapsed().as_secs_f32() % 1.0) * std::f32::consts::TAU
        } else {
            self.rotation
        };
        renderer.draw_svg(
            svg::Svg {
                handle: handle(self.ico, self.filled, self.stroke),
                color: Some(Color { a: 1.0, ..color }),
                rotation: Radians(rotation),
                opacity: color.a * current_opacity(),
            },
            b,
            viewport.expand(2.0),
        );
    }
}

impl<'a, Message: 'a, Theme: 'a> From<Icon> for Element<'a, Message, Theme, iced::Renderer> {
    fn from(i: Icon) -> Self {
        Element::new(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_lucide_svg() {
        assert_eq!(Ico::ALL.len(), 57);
        for i in Ico::ALL {
            let s = i.src();
            assert!(
                s.starts_with(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\""
                ),
                "{}",
                i.name()
            );
            assert!(
                s.contains("stroke=\"#000000\" stroke-width=\"2\""),
                "{}",
                i.name()
            );
        }
    }

    #[test]
    fn stroke_and_fill_overrides() {
        let s = source(Ico::Check, false, Some(3.0));
        assert!(s.contains("stroke-width=\"3\""));
        assert!(!s.contains("stroke-width=\"2\""));
        let s = source(Ico::Star, true, None);
        assert!(s.contains("fill=\"#000000\""));
    }
}
