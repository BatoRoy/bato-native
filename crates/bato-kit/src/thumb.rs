//! Images: thumbnails, previews and avatars.
//!
//! - [`Thumbs`]: a small LRU of decoded images keyed by the caller's id (a
//!   download id, `(conn, path, mtime, size)`), with the request bookkeeping
//!   React gets from `<img loading="lazy">`: ask once, remember "no preview",
//!   retry a failed transfer, cancel what scrolled away.
//! - [`load`]: fetch + decode on a worker thread (at most [`WORKERS`] at
//!   once), so the UI thread only uploads RGBA. [`decode`] is the blocking
//!   decoder (JPEG, PNG, GIF's first frame, WebP, BMP, ICO).
//! - [`thumb`]: the view: the image when it is ready, the app's glyph while
//!   loading, on error and for "no preview" (BatoBrowse's tile fallback).
//! - [`picture`] / [`vector`]: a raster or SVG image fitted into its box
//!   (contain / cover), rounded, honouring the kit's group opacity. SVG goes
//!   through resvg (iced's `svg`), drawn in its own colours.
//!
//! ```ignore
//! // state: thumbs: Thumbs<String> = Thumbs::new(256)
//! // view: request what is visible, show what is there
//! thumb(&cx, self.thumbs.get(&id), Ico::FileImage, 128.0)
//! // update: start the loads the view asked for
//! let tasks = ids.into_iter().filter(|id| self.thumbs.want(id.clone())).map(|id| {
//!     let token = self.thumbs.token(&id).unwrap();
//!     let api = self.api.clone();
//!     thumb::load(id.clone(), token, move || api.thumbnail_bytes(&id), Msg::Thumb)
//! });
//! Msg::Thumb(id, result) => self.thumbs.finish(id, result),
//! ```

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};

use iced::advanced::image::{self as adv_image, Renderer as _};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::svg::{self, Renderer as _};
use iced::advanced::widget::{Tree, Widget};
use iced::widget::image::Handle;
use iced::{ContentFit, Element, Length, Radians, Rectangle, Size, Task, mouse};

use crate::anim::current_opacity;
use crate::ctx::Ctx;
use crate::icon::{Icon, icon};
use crate::style::c;

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

// ── Decoding ────────────────────────────────────────────────────────────

/// Decode encoded image bytes (JPEG, PNG, GIF, WebP, BMP, ICO; the format
/// is sniffed) into an RGBA handle. Blocking: call it from a worker
/// ([`load`], [`decode_task`]). `max_side` downsizes larger images
/// (Triangle filter) so a 6000 px photo is not uploaded whole.
pub fn decode(bytes: &[u8], max_side: Option<u32>) -> Result<Handle, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let img = match max_side {
        Some(m) if img.width().max(img.height()) > m => {
            img.resize(m, m, image::imageops::FilterType::Triangle)
        }
        _ => img,
    };
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    Ok(Handle::from_rgba(w, h, rgba.into_raw()))
}

/// SVG bytes as a handle for [`vector`] (rasterised by resvg on draw, at
/// the device scale, and cached by iced).
pub fn svg_handle(bytes: impl Into<std::borrow::Cow<'static, [u8]>>) -> svg::Handle {
    svg::Handle::from_memory(bytes)
}

/// Concurrent loads ([`load`]): BatoBrowse's "cap concurrency (4–6)".
pub const WORKERS: usize = 4;

type Job = Box<dyn FnOnce() + Send>;

/// The worker pool, spawned on first use. Idle workers block on the queue:
/// no wake-ups, nothing for an idle app to pay.
fn pool() -> &'static Mutex<Sender<Job>> {
    static POOL: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    POOL.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..WORKERS {
            let rx = rx.clone();
            let _ = std::thread::Builder::new()
                .name(format!("bato-thumb-{i}"))
                .spawn(move || {
                    loop {
                        let job = match rx.lock() {
                            Ok(r) => r.recv(),
                            Err(_) => return,
                        };
                        match job {
                            Ok(job) => job(),
                            Err(_) => return,
                        }
                    }
                });
        }
        Mutex::new(tx)
    })
}

/// Run `work` on the pool and deliver its result as a message.
fn on_pool<T: Send + 'static, M: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T) -> M + Send + 'static,
) -> Task<M> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    let job: Job = Box::new(move || {
        let _ = tx.send(work());
    });
    if let Ok(q) = pool().lock() {
        let _ = q.send(job);
    }
    Task::future(async move { rx.await.ok().map(done) }).and_then(Task::done)
}

/// Why a load produced no image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The source has no preview (404, audio without art) or the bytes did
    /// not decode: remembered, the glyph shows for good.
    NoPreview(String),
    /// A transfer error worth retrying (server restarting): forgotten.
    Transient(String),
    /// The load was cancelled before it ran.
    Cancelled,
}

/// What [`load`] delivers.
pub type Loaded = Result<Handle, Failure>;

/// Fetch with `fetch` and decode on a worker, then publish
/// `done(key, result)`. `fetch` returns the encoded bytes, `Ok(None)` for
/// "no preview", or `Err` for a transient failure. A load whose `token`
/// was cancelled ([`Thumbs::cancel_unless`]) before a worker picked it up
/// does not run.
pub fn load<K, M>(
    key: K,
    token: CancelToken,
    fetch: impl FnOnce() -> Result<Option<Vec<u8>>, String> + Send + 'static,
    done: fn(K, Loaded) -> M,
) -> Task<M>
where
    K: Send + 'static,
    M: Send + 'static,
{
    on_pool(
        move || {
            if token.is_cancelled() {
                return Err(Failure::Cancelled);
            }
            match fetch() {
                Ok(Some(bytes)) => decode(&bytes, None).map_err(Failure::NoPreview),
                Ok(None) => Err(Failure::NoPreview(String::new())),
                Err(e) => Err(Failure::Transient(e)),
            }
        },
        move |r| done(key, r),
    )
}

/// Decode bytes the app already has (a data URL's payload) on a worker.
pub fn decode_task<K, M>(
    key: K,
    bytes: Vec<u8>,
    max_side: Option<u32>,
    done: fn(K, Loaded) -> M,
) -> Task<M>
where
    K: Send + 'static,
    M: Send + 'static,
{
    on_pool(
        move || decode(&bytes, max_side).map_err(Failure::NoPreview),
        move |r| done(key, r),
    )
}

/// Shared with a queued [`load`]; cancelling skips the work if it has not
/// started.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

// ── The cache ───────────────────────────────────────────────────────────

/// One cached entry.
#[derive(Debug, Clone)]
pub enum Thumb {
    /// Requested, no answer yet.
    Loading,
    /// No preview (or undecodable): the glyph shows; not asked again.
    None,
    Ready(Handle),
}

#[derive(Debug)]
struct Entry {
    thumb: Thumb,
    used: u64,
    token: Option<CancelToken>,
}

/// An LRU of decoded images keyed by the caller's id. `capacity` bounds the
/// decoded images ([`Thumb::Ready`]); loading and "no preview" entries are
/// tiny and kept until [`Thumbs::forget`] or eviction by age.
#[derive(Debug)]
pub struct Thumbs<K> {
    map: HashMap<K, Entry>,
    capacity: usize,
    clock: u64,
}

impl<K: Eq + Hash + Clone> Thumbs<K> {
    pub fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            capacity: capacity.max(1),
            clock: 0,
        }
    }

    /// The entry for `key` (for `view`; [`Thumbs::touch`] marks use).
    pub fn get(&self, key: &K) -> Option<&Thumb> {
        self.map.get(key).map(|e| &e.thumb)
    }

    /// The decoded image, if ready.
    pub fn image(&self, key: &K) -> Option<&Handle> {
        match self.get(key) {
            Some(Thumb::Ready(h)) => Some(h),
            _ => None,
        }
    }

    /// Mark `keys` as recently used (the visible tiles), so scrolling keeps
    /// what is on screen and evicts what is far away.
    pub fn touch<'k>(&mut self, keys: impl IntoIterator<Item = &'k K>)
    where
        K: 'k,
    {
        for k in keys {
            self.clock += 1;
            if let Some(e) = self.map.get_mut(k) {
                e.used = self.clock;
            }
        }
    }

    /// Ask for `key`: `true` if a load should start now (not cached, not
    /// loading). The entry becomes [`Thumb::Loading`] with a fresh
    /// [`CancelToken`] ([`Thumbs::token`]).
    pub fn want(&mut self, key: K) -> bool {
        self.clock += 1;
        if let Some(e) = self.map.get_mut(&key) {
            e.used = self.clock;
            return false;
        }
        self.map.insert(
            key,
            Entry {
                thumb: Thumb::Loading,
                used: self.clock,
                token: Some(CancelToken::default()),
            },
        );
        true
    }

    /// The cancel token of a loading entry, for [`load`].
    pub fn token(&self, key: &K) -> Option<CancelToken> {
        self.map.get(key).and_then(|e| e.token.clone())
    }

    /// Store a result from [`load`]. A transient failure or a cancelled
    /// load forgets the entry, so the next [`Thumbs::want`] asks again.
    pub fn finish(&mut self, key: K, result: Loaded) {
        self.clock += 1;
        let thumb = match result {
            Ok(h) => Thumb::Ready(h),
            Err(Failure::NoPreview(_)) => Thumb::None,
            Err(Failure::Transient(_) | Failure::Cancelled) => {
                self.map.remove(&key);
                return;
            }
        };
        self.map.insert(
            key,
            Entry {
                thumb,
                used: self.clock,
                token: None,
            },
        );
        self.evict();
    }

    /// Cancel every queued load whose key fails `keep` (scrolled away,
    /// navigated off); the entries are forgotten.
    pub fn cancel_unless(&mut self, keep: impl Fn(&K) -> bool) {
        self.map.retain(|k, e| {
            if matches!(e.thumb, Thumb::Loading) && !keep(k) {
                if let Some(t) = &e.token {
                    t.cancel();
                }
                false
            } else {
                true
            }
        });
    }

    /// Forget `key` (deleted, or its file changed).
    pub fn forget(&mut self, key: &K) {
        if let Some(e) = self.map.remove(key)
            && let Some(t) = e.token
        {
            t.cancel();
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Decoded images held.
    pub fn ready(&self) -> usize {
        self.map
            .values()
            .filter(|e| matches!(e.thumb, Thumb::Ready(_)))
            .count()
    }

    fn evict(&mut self) {
        // Images over capacity, oldest first; small entries beyond 8× the
        // capacity too, so a long session does not grow without bound.
        while self.ready() > self.capacity {
            self.drop_oldest(|t| matches!(t, Thumb::Ready(_)));
        }
        while self.map.len() > self.capacity * 8 {
            self.drop_oldest(|t| !matches!(t, Thumb::Loading));
        }
    }

    fn drop_oldest(&mut self, which: impl Fn(&Thumb) -> bool) {
        let oldest = self
            .map
            .iter()
            .filter(|(_, e)| which(&e.thumb))
            .min_by_key(|(_, e)| e.used)
            .map(|(k, _)| k.clone());
        match oldest {
            Some(k) => {
                self.map.remove(&k);
            }
            None => self.capacity = usize::MAX,
        }
    }
}

// ── Views ───────────────────────────────────────────────────────────────

/// A thumbnail `size` px square: the image (cover, radius 6) when ready,
/// else `fallback` at half the size in `muted` (loading, no preview).
pub fn thumb<'a, M: 'a>(
    cx: &Ctx,
    state: Option<&Thumb>,
    fallback: impl Into<Icon>,
    size: f32,
) -> El<'a, M> {
    match state {
        Some(Thumb::Ready(h)) => picture(h.clone())
            .width(size)
            .height(size)
            .fit(ContentFit::Cover)
            .radius(6.0)
            .into(),
        _ => iced::widget::container(icon(fallback, (size * 0.5).round()).color(c(cx.p().muted)))
            .center(size)
            .into(),
    }
}

#[derive(Debug, Clone)]
enum Source {
    Raster(Handle),
    Vector(svg::Handle),
}

/// A raster or SVG image fitted into its box ([`picture`], [`vector`]).
pub struct Picture {
    src: Source,
    width: Length,
    height: Length,
    fit: ContentFit,
    radius: f32,
}

/// A raster image (`<img>`, `object-fit: contain` by default).
pub fn picture(handle: Handle) -> Picture {
    Picture {
        src: Source::Raster(handle),
        width: Length::Shrink,
        height: Length::Shrink,
        fit: ContentFit::Contain,
        radius: 0.0,
    }
}

/// An SVG image in its own colours (a file preview, a logo).
pub fn vector(handle: svg::Handle) -> Picture {
    Picture {
        src: Source::Vector(handle),
        width: Length::Shrink,
        height: Length::Shrink,
        fit: ContentFit::Contain,
        radius: 0.0,
    }
}

impl Picture {
    pub fn width(mut self, w: impl Into<Length>) -> Self {
        self.width = w.into();
        self
    }
    pub fn height(mut self, h: impl Into<Length>) -> Self {
        self.height = h.into();
        self
    }
    /// `object-fit` (`Contain`, `Cover`, `Fill`, `ScaleDown`, `None`).
    pub fn fit(mut self, f: ContentFit) -> Self {
        self.fit = f;
        self
    }
    /// Rounded corners (clips the image).
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }

    fn natural(&self, renderer: &iced::Renderer) -> Size {
        let s = match &self.src {
            Source::Raster(h) => renderer.measure_image(h).unwrap_or_default(),
            Source::Vector(h) => renderer.measure_svg(h),
        };
        Size::new(s.width as f32, s.height as f32)
    }
}

/// Where `natural` lands inside `bounds` for `fit`, centred.
pub fn fitted(natural: Size, bounds: Rectangle, fit: ContentFit) -> Rectangle {
    if natural.width <= 0.0 || natural.height <= 0.0 {
        return bounds;
    }
    let s = fit.fit(natural, bounds.size());
    Rectangle {
        x: bounds.x + (bounds.width - s.width) / 2.0,
        y: bounds.y + (bounds.height - s.height) / 2.0,
        width: s.width,
        height: s.height,
    }
}

impl<M> Widget<M, iced::Theme, iced::Renderer> for Picture {
    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let natural = self.natural(renderer);
        // Like iced's `image`: a Shrink side takes the natural size scaled
        // to what the other side allows.
        let full = limits.resolve(self.width, self.height, natural);
        let ratio = if natural.height > 0.0 {
            natural.width / natural.height
        } else {
            1.0
        };
        let size = match (self.width, self.height) {
            (Length::Shrink, Length::Shrink) => {
                let s = ContentFit::ScaleDown.fit(natural, limits.max());
                Size::new(s.width, s.height)
            }
            (Length::Shrink, _) => {
                Size::new((full.height * ratio).min(limits.max().width), full.height)
            }
            (_, Length::Shrink) => {
                Size::new(full.width, (full.width / ratio).min(limits.max().height))
            }
            _ => full,
        };
        layout::Node::new(size)
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let b = layout.bounds();
        let Some(clip) = b.intersection(viewport) else {
            return;
        };
        let target = fitted(self.natural(renderer), b, self.fit);
        let opacity = current_opacity();
        match &self.src {
            Source::Raster(h) => renderer.draw_image(
                adv_image::Image {
                    handle: h.clone(),
                    filter_method: adv_image::FilterMethod::Linear,
                    rotation: Radians(0.0),
                    border_radius: self.radius.into(),
                    opacity,
                    snap: true,
                },
                target,
                clip,
            ),
            Source::Vector(h) => renderer.draw_svg(
                svg::Svg {
                    handle: h.clone(),
                    color: None,
                    rotation: Radians(0.0),
                    opacity,
                },
                target,
                clip,
            ),
        }
    }
}

impl<'a, M: 'a> From<Picture> for El<'a, M> {
    fn from(p: Picture) -> Self {
        Element::new(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, _| image::Rgba([x as u8, 0, 200, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    fn dims(h: &Handle) -> (u32, u32) {
        match h {
            Handle::Rgba { width, height, .. } => (*width, *height),
            _ => panic!("not rgba"),
        }
    }

    #[test]
    fn decodes_and_downsizes() {
        let h = decode(&png(40, 20), None).unwrap();
        assert_eq!(dims(&h), (40, 20));
        let h = decode(&png(400, 200), Some(100)).unwrap();
        assert_eq!(dims(&h), (100, 50));
        assert!(decode(b"not an image", None).is_err());
    }

    #[test]
    fn asks_once_and_remembers_no_preview() {
        let mut t: Thumbs<&str> = Thumbs::new(4);
        assert!(t.want("a"));
        assert!(!t.want("a"), "loading");
        assert!(matches!(t.get(&"a"), Some(Thumb::Loading)));
        t.finish("a", Err(Failure::NoPreview("404".into())));
        assert!(!t.want("a"), "no preview is cached");
        assert!(t.want("b"));
        t.finish("b", Err(Failure::Transient("refused".into())));
        assert!(t.want("b"), "a transient failure is asked again");
        let h = decode(&png(2, 2), None).unwrap();
        t.finish("b", Ok(h));
        assert!(t.image(&"b").is_some());
        t.forget(&"b");
        assert!(t.get(&"b").is_none());
    }

    #[test]
    fn evicts_the_least_recently_used_image() {
        let mut t: Thumbs<u32> = Thumbs::new(2);
        let h = decode(&png(1, 1), None).unwrap();
        for k in 0..2 {
            t.want(k);
            t.finish(k, Ok(h.clone()));
        }
        t.touch([&0]);
        t.want(2);
        t.finish(2, Ok(h.clone()));
        assert_eq!(t.ready(), 2);
        assert!(t.image(&0).is_some(), "touched, kept");
        assert!(t.get(&1).is_none(), "oldest evicted");
        assert!(t.image(&2).is_some());
    }

    #[test]
    fn cancels_what_scrolled_away() {
        let mut t: Thumbs<u32> = Thumbs::new(8);
        t.want(1);
        t.want(2);
        let (one, two) = (t.token(&1).unwrap(), t.token(&2).unwrap());
        t.cancel_unless(|k| *k == 2);
        assert!(one.is_cancelled());
        assert!(!two.is_cancelled());
        assert!(t.get(&1).is_none());
        assert!(t.want(1), "asked again when it comes back");
    }

    #[test]
    fn fit_centres_contain_and_cover() {
        let b = Rectangle::new(iced::Point::new(10.0, 10.0), Size::new(100.0, 50.0));
        let r = fitted(Size::new(200.0, 200.0), b, ContentFit::Contain);
        assert_eq!(
            r,
            Rectangle::new(iced::Point::new(35.0, 10.0), Size::new(50.0, 50.0))
        );
        let r = fitted(Size::new(200.0, 200.0), b, ContentFit::Cover);
        assert_eq!(
            r,
            Rectangle::new(iced::Point::new(10.0, -15.0), Size::new(100.0, 100.0))
        );
    }

    #[test]
    fn worker_pool_runs_off_the_ui_thread() {
        let here = std::thread::current().id();
        let (tx, rx) = channel();
        let job: Job = Box::new(move || {
            let _ = tx.send(std::thread::current().id());
        });
        pool().lock().unwrap().send(job).unwrap();
        let there = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_ne!(here, there);
    }
}
