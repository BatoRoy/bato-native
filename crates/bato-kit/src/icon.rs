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
    AppWindow => "app-window",
    Archive => "archive",
    ArrowDown => "arrow-down",
    ArrowDownAZ => "arrow-down-a-z",
    ArrowRightLeft => "arrow-right-left",
    ArrowUp => "arrow-up",
    ArrowUpAZ => "arrow-up-a-z",
    BarChart3 => "bar-chart-3",
    Bell => "bell",
    Blocks => "blocks",
    BookOpen => "book-open",
    Box => "box",
    Calendar => "calendar",
    CalendarClock => "calendar-clock",
    Check => "check",
    CheckCircle => "check-circle",
    CheckCircle2 => "check-circle-2",
    CheckSquare => "check-square",
    ChevronDown => "chevron-down",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    ChevronUp => "chevron-up",
    ChevronsUpDown => "chevrons-up-down",
    Clipboard => "clipboard",
    ClipboardCopy => "clipboard-copy",
    ClipboardPaste => "clipboard-paste",
    CloudMoon => "cloud-moon",
    Columns2 => "columns-2",
    Command => "command",
    Cookie => "cookie",
    Copy => "copy",
    Cpu => "cpu",
    Database => "database",
    Disc => "disc",
    Download => "download",
    ExternalLink => "external-link",
    Eye => "eye",
    EyeOff => "eye-off",
    File => "file",
    FileArchive => "file-archive",
    FileAudio => "file-audio",
    FileCode => "file-code",
    FileCog => "file-cog",
    FileImage => "file-image",
    FileJson => "file-json",
    FileKey => "file-key",
    FilePlus => "file-plus",
    FileQuestion => "file-question",
    FileSpreadsheet => "file-spreadsheet",
    FileText => "file-text",
    FileType => "file-type",
    FileVideo => "file-video",
    Film => "film",
    Folder => "folder",
    FolderCheck => "folder-check",
    FolderDown => "folder-down",
    FolderGit2 => "folder-git-2",
    FolderInput => "folder-input",
    FolderOpen => "folder-open",
    FolderPlus => "folder-plus",
    FolderRoot => "folder-root",
    FolderTree => "folder-tree",
    GitBranch => "git-branch",
    Grid2x2 => "grid-2x2",
    Grid3x3 => "grid-3x3",
    GripVertical => "grip-vertical",
    HardDrive => "hard-drive",
    History => "history",
    Home => "home",
    Image => "image",
    Info => "info",
    KeyRound => "key-round",
    LayoutGrid => "layout-grid",
    Link => "link",
    Link2 => "link-2",
    List => "list",
    ListChecks => "list-checks",
    ListFilter => "list-filter",
    Loader2 => "loader-2",
    Mail => "mail",
    Minimize2 => "minimize-2",
    Minus => "minus",
    Monitor => "monitor",
    Moon => "moon",
    MoreHorizontal => "more-horizontal",
    Music => "music",
    Network => "network",
    Package => "package",
    PackageOpen => "package-open",
    Palette => "palette",
    PanelLeftClose => "panel-left-close",
    PanelLeftOpen => "panel-left-open",
    PanelRight => "panel-right",
    PenLine => "pen-line",
    Pencil => "pencil",
    Pin => "pin",
    PinOff => "pin-off",
    Pipette => "pipette",
    Play => "play",
    Plug => "plug",
    Plus => "plus",
    Presentation => "presentation",
    Redo2 => "redo-2",
    RefreshCw => "refresh-cw",
    RotateCw => "rotate-cw",
    Rows3 => "rows-3",
    Ruler => "ruler",
    Save => "save",
    Scissors => "scissors",
    Search => "search",
    Server => "server",
    Settings => "settings",
    Settings2 => "settings-2",
    Share2 => "share-2",
    SlidersHorizontal => "sliders-horizontal",
    Smartphone => "smartphone",
    Square => "square",
    SquarePen => "square-pen",
    SquarePlus => "square-plus",
    SquareTerminal => "square-terminal",
    Star => "star",
    SunMedium => "sun-medium",
    Terminal => "terminal",
    TextSelect => "text-select",
    Trash => "trash",
    Trash2 => "trash-2",
    Undo2 => "undo-2",
    Unplug => "unplug",
    Upload => "upload",
    Video => "video",
    Volume1 => "volume-1",
    Volume2 => "volume-2",
    VolumeX => "volume-x",
    Wallpaper => "wallpaper",
    X => "x",
    XCircle => "x-circle",
}

/// What an icon draws: one of the kit's lucide [`Ico`]s or an app's own SVG.
/// Every kit component that takes an icon takes `impl Into<Icon>`, so
/// `Ico::Folder` and `Icon::svg(include_bytes!("../assets/icons/zip.svg"))`
/// both work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Kit(Ico),
    App(AppSvg),
}

/// An app-supplied SVG (`include_bytes!`). Compared and cached by address,
/// so keep each one in a `const`/`static` or pass the same `include_bytes!`.
#[derive(Debug, Clone, Copy)]
pub struct AppSvg {
    src: &'static [u8],
    tint: bool,
}

impl PartialEq for AppSvg {
    fn eq(&self, o: &Self) -> bool {
        std::ptr::eq(self.src, o.src) && self.tint == o.tint
    }
}
impl Eq for AppSvg {}
impl std::hash::Hash for AppSvg {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        (self.src.as_ptr() as usize, self.src.len(), self.tint).hash(h);
    }
}

impl AppSvg {
    pub fn src(&self) -> &'static [u8] {
        self.src
    }
    /// Drawn in the text colour (a lucide-style monochrome icon).
    pub fn is_tinted(&self) -> bool {
        self.tint
    }
}

impl Icon {
    /// A monochrome app icon in lucide's format (24-unit box, `stroke` in
    /// any one colour), tinted with the text colour like an [`Ico`].
    pub const fn svg(src: &'static [u8]) -> Self {
        Icon::App(AppSvg { src, tint: true })
    }
    /// An app SVG drawn in its own colours (a brand logo): never tinted,
    /// `fill`/`stroke_width` do not apply.
    pub const fn svg_colored(src: &'static [u8]) -> Self {
        Icon::App(AppSvg { src, tint: false })
    }
    /// Whether the icon takes the text colour.
    pub fn is_tinted(self) -> bool {
        match self {
            Icon::Kit(_) => true,
            Icon::App(a) => a.tint,
        }
    }
}

impl From<Ico> for Icon {
    fn from(i: Ico) -> Self {
        Icon::Kit(i)
    }
}

/// The SVG source with `fill`/`stroke-width` overrides applied.
fn source(ico: Ico, filled: bool, stroke: Option<f32>) -> String {
    overrides(ico.src(), filled, stroke)
}

fn overrides(src: &str, filled: bool, stroke: Option<f32>) -> String {
    let mut s = src.to_string();
    if filled {
        s = s.replacen("fill=\"none\"", "fill=\"#000000\"", 1);
    }
    if let Some(w) = stroke {
        // Only the root's stroke-width (lucide sets it once, on <svg>).
        s = s.replacen("stroke-width=\"2\"", &format!("stroke-width=\"{w}\""), 1);
    }
    s
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Kit(Ico, bool, Option<u32>),
    App(usize, usize, bool, Option<u32>),
}

thread_local! {
    static HANDLES: RefCell<HashMap<Key, svg::Handle>> = RefCell::new(HashMap::new());
}

/// One handle per icon variant for the thread: iced keys its raster cache by
/// the handle's id, so a fresh handle per frame would re-rasterise.
pub fn handle(ico: impl Into<Icon>, filled: bool, stroke: Option<f32>) -> svg::Handle {
    let ico = ico.into();
    let stroke_key = stroke.map(f32::to_bits);
    let key = match ico {
        Icon::Kit(i) => Key::Kit(i, filled, stroke_key),
        Icon::App(a) if a.tint => {
            Key::App(a.src.as_ptr() as usize, a.src.len(), filled, stroke_key)
        }
        Icon::App(a) => Key::App(a.src.as_ptr() as usize, a.src.len(), false, None),
    };
    HANDLES.with(|h| {
        h.borrow_mut()
            .entry(key)
            .or_insert_with(|| match ico {
                Icon::Kit(i) => svg::Handle::from_memory(source(i, filled, stroke).into_bytes()),
                Icon::App(a) => match std::str::from_utf8(a.src) {
                    Ok(s) if a.tint && (filled || stroke.is_some()) => {
                        svg::Handle::from_memory(overrides(s, filled, stroke).into_bytes())
                    }
                    _ => svg::Handle::from_memory(a.src),
                },
            })
            .clone()
    })
}

/// The icon widget ([`icon`]).
pub struct IconWidget {
    ico: Icon,
    size: f32,
    color: Option<Color>,
    filled: bool,
    stroke: Option<f32>,
    rotation: f32,
    spin: bool,
}

/// An icon `size` px square, in the inherited text colour: a lucide [`Ico`]
/// or an app [`Icon`].
pub fn icon(ico: impl Into<Icon>, size: f32) -> IconWidget {
    IconWidget {
        ico: ico.into(),
        size,
        color: None,
        filled: false,
        stroke: None,
        rotation: 0.0,
        spin: false,
    }
}

impl IconWidget {
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

impl<Message, Theme> Widget<Message, Theme, iced::Renderer> for IconWidget {
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
        let tint = self.ico.is_tinted();
        renderer.draw_svg(
            svg::Svg {
                handle: handle(self.ico, self.filled, self.stroke),
                color: tint.then_some(Color { a: 1.0, ..color }),
                rotation: Radians(rotation),
                opacity: if tint { color.a } else { 1.0 } * current_opacity(),
            },
            b,
            viewport.expand(2.0),
        );
    }
}

impl<'a, Message: 'a, Theme: 'a> From<IconWidget> for Element<'a, Message, Theme, iced::Renderer> {
    fn from(i: IconWidget) -> Self {
        Element::new(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_lucide_svg() {
        assert_eq!(Ico::ALL.len(), 139);
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

    static LOGO: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><rect width=\"24\" height=\"24\" fill=\"#eab308\"/></svg>";
    static ZIP: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\" fill=\"none\" stroke=\"#000000\" stroke-width=\"2\"><path d=\"M4 4h16\"/></svg>";

    #[test]
    fn app_icons_convert_compare_and_cache() {
        let kit: Icon = Ico::Folder.into();
        assert_eq!(kit, Icon::Kit(Ico::Folder));
        assert!(kit.is_tinted());
        let logo = Icon::svg_colored(LOGO);
        assert!(!logo.is_tinted());
        assert_eq!(logo, Icon::svg_colored(LOGO), "same bytes, same icon");
        assert_ne!(logo, Icon::svg(LOGO), "tint is part of the identity");
        assert_ne!(Icon::svg(ZIP), Icon::svg(LOGO));
        // One handle per variant: iced caches rasters by handle id.
        assert_eq!(
            handle(logo, false, None).id(),
            handle(logo, false, None).id()
        );
        assert_eq!(
            handle(Ico::Star, true, None).id(),
            handle(Ico::Star, true, None).id()
        );
        assert_ne!(
            handle(Ico::Star, true, None).id(),
            handle(Ico::Star, false, None).id()
        );
        // Overrides apply to tinted app icons, never to coloured ones.
        assert_ne!(
            handle(Icon::svg(ZIP), false, Some(3.0)).id(),
            handle(Icon::svg(ZIP), false, None).id()
        );
        assert_eq!(
            handle(logo, true, Some(3.0)).id(),
            handle(logo, false, None).id()
        );
    }
}
