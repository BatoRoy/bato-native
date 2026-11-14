// bato patch: rasterize outline glyphs with FreeType "light" hinting (the
// auto-hinter, vertical snapping only), the way Chromium/Skia renders text on
// Linux with fontconfig `hintstyle=hintslight`. swash is unhinted, which makes
// x-heights and cap heights straddle pixel rows and look blurry next to the
// Electron client. Subpixel x positioning is kept by translating the outline
// by the cache key's x bin before rendering (also what Skia does).
//
// Set BATO_TEXT_RASTER=swash to get the stock rasterizer back.

use crate::{CacheKey, FontSystem, HashMap};
use alloc::sync::Arc;
use freetype_sys as ft;
use swash::scale::image::{Content, Image};
use swash::scale::Source;
use swash::zeno::Placement;

struct Face {
    face: ft::FT_Face,
    _font: Arc<crate::Font>,
    wght_axis: Option<(usize, usize)>, // (axis index, axis count)
    defaults: alloc::vec::Vec<ft::FT_Fixed>,
    weight: u16,
    size: i64,
}

pub(crate) struct FtRaster {
    lib: ft::FT_Library,
    faces: HashMap<(usize, u32), Face>,
    pub(crate) enabled: bool,
}

// FreeType objects are only touched through `&mut self`.
unsafe impl Send for FtRaster {}
unsafe impl Sync for FtRaster {}

impl FtRaster {
    pub(crate) fn new() -> Self {
        let mut lib = core::ptr::null_mut();
        let ok = unsafe { ft::FT_Init_FreeType(&mut lib) } == 0;
        let enabled = ok && std::env::var("BATO_TEXT_RASTER").map_or(true, |v| v != "swash");
        Self { lib, faces: HashMap::default(), enabled }
    }

    pub(crate) fn render(&mut self, font_system: &mut FontSystem, key: CacheKey) -> Option<Image> {
        if !self.enabled {
            return None;
        }
        let font = font_system.get_font(key.font_id, key.font_weight)?;
        let index = font_system.db().face(key.font_id)?.index;
        let data = font.data();
        let fkey = (data.as_ptr() as usize, index);
        let lib = self.lib;
        if !self.faces.contains_key(&fkey) {
            let mut face = core::ptr::null_mut();
            let err = unsafe {
                ft::FT_New_Memory_Face(lib, data.as_ptr(), data.len() as ft::FT_Long, index as ft::FT_Long, &mut face)
            };
            if err != 0 {
                return None;
            }
            let flags = unsafe { (*face).face_flags };
            if flags & ft::FT_FACE_FLAG_COLOR as ft::FT_Long != 0 {
                unsafe { ft::FT_Done_Face(face) };
                return None;
            }
            let mut wght_axis = None;
            let mut defaults = alloc::vec::Vec::new();
            if flags & ft::FT_FACE_FLAG_MULTIPLE_MASTERS as ft::FT_Long != 0 {
                let mut mm: *mut ft::FT_MM_Var = core::ptr::null_mut();
                if unsafe { ft::FT_Get_MM_Var(face, &mut mm) } == 0 {
                    let n = unsafe { (*mm).num_axis } as usize;
                    for i in 0..n {
                        let axis = unsafe { &*(*mm).axis.add(i) };
                        defaults.push(axis.def);
                        if axis.tag == u32::from_be_bytes(*b"wght") as ft::FT_ULong {
                            wght_axis = Some((i, n));
                        }
                    }
                    unsafe { ft::FT_Done_MM_Var(lib, mm) };
                }
            }
            self.faces.insert(fkey, Face { face, _font: font.clone(), wght_axis, defaults, weight: 0, size: 0 });
        }
        let f = self.faces.get_mut(&fkey)?;
        if let Some((i, _n)) = f.wght_axis {
            if f.weight != key.font_weight.0 {
                let mut coords = f.defaults.clone();
                coords[i] = (key.font_weight.0 as ft::FT_Fixed) << 16;
                unsafe { ft::FT_Set_Var_Design_Coordinates(f.face, coords.len() as u32, coords.as_mut_ptr()) };
                f.weight = key.font_weight.0;
                f.size = 0; // force size re-apply after a variation change
            }
        }
        let size = (f32::from_bits(key.font_size_bits) * 64.0).round() as i64;
        if f.size != size {
            if unsafe { ft::FT_Set_Char_Size(f.face, 0, size as ft::FT_F26Dot6, 72, 72) } != 0 {
                return None;
            }
            f.size = size;
        }
        let load = ft::FT_LOAD_TARGET_LIGHT | ft::FT_LOAD_NO_BITMAP;
        if unsafe { ft::FT_Load_Glyph(f.face, key.glyph_id as u32, load as i32) } != 0 {
            return None;
        }
        let slot = unsafe { (*f.face).glyph };
        if unsafe { (*slot).format } != ft::FT_GLYPH_FORMAT_OUTLINE {
            return None;
        }
        // Synthetic italic (no italic face, e.g. a CodeMirror comment in a
        // monospace font): Skia's fake italic shears x by 1/4 of y.
        if key.flags.contains(crate::CacheKeyFlags::FAKE_ITALIC) {
            let m = ft::FT_Matrix { xx: 0x10000, xy: 0x4000, yx: 0, yy: 0x10000 };
            unsafe { ft::FT_Outline_Transform(&(*slot).outline, &m) };
        }
        let dx = (key.x_bin.as_float() * 64.0).round() as ft::FT_Pos;
        let dy = (key.y_bin.as_float() * 64.0).round() as ft::FT_Pos;
        unsafe { ft::FT_Outline_Translate(&(*slot).outline, dx, -dy) };
        if unsafe { ft::FT_Render_Glyph(slot, ft::FT_RENDER_MODE_LIGHT) } != 0 {
            return None;
        }
        let bm = unsafe { &(*slot).bitmap };
        let (w, h) = (bm.width as usize, bm.rows as usize);
        let mut data = alloc::vec::Vec::with_capacity(w * h);
        for y in 0..h {
            let row = unsafe { core::slice::from_raw_parts(bm.buffer.offset(y as isize * bm.pitch as isize), w) };
            data.extend_from_slice(row);
        }
        let mut img = Image::new();
        img.source = Source::Outline;
        img.content = Content::Mask;
        img.placement = Placement {
            left: unsafe { (*slot).bitmap_left },
            top: unsafe { (*slot).bitmap_top },
            width: w as u32,
            height: h as u32,
        };
        img.data = data;
        Some(img)
    }
}

impl Drop for FtRaster {
    fn drop(&mut self) {
        for (_, f) in self.faces.drain() {
            unsafe { ft::FT_Done_Face(f.face) };
        }
        if !self.lib.is_null() {
            unsafe { ft::FT_Done_FreeType(self.lib) };
        }
    }
}
