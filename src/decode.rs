//! Image decoding (the QImageReader part of QVImageCore::readFile). Runs on worker threads.
//!
//! Output is always Cairo's ARGB32 premultiplied layout so the UI thread only has to wrap it
//! in a surface.

use std::io::{BufReader, Read, Seek};
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ErrorData {
    pub has_error: bool,
    pub error_num: i32,
    pub error_string: String,
}

impl ErrorData {
    fn new(num: i32) -> Self {
        let s = match num {
            1 => "File not found",
            2 => "Device error",
            3 => "Unsupported image format",
            4 => "Unable to read image data",
            _ => "Unknown error",
        };
        ErrorData { has_error: true, error_num: num, error_string: s.to_string() }
    }
}

/// Premultiplied ARGB32 (native endian) pixels, tightly packed (stride = width * 4).
#[derive(Clone)]
pub struct RawImage {
    pub width: i32,
    pub height: i32,
    pub data: Arc<Vec<u8>>,
}

impl RawImage {
    pub fn bytes(&self) -> usize {
        self.data.len()
    }

    /// Rotate by a multiple of 90 degrees (clockwise).
    pub fn rotated(&self, degrees: i32) -> RawImage {
        let deg = ((degrees % 360) + 360) % 360;
        if deg == 0 {
            return self.clone();
        }
        let (w, h) = (self.width as usize, self.height as usize);
        let src = &self.data[..];
        let (nw, nh) = if deg == 180 { (w, h) } else { (h, w) };
        let mut out = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = match deg {
                    90 => (h - 1 - y, x),
                    180 => (w - 1 - x, h - 1 - y),
                    _ => (y, w - 1 - x),
                };
                let s = (y * w + x) * 4;
                let d = (dy * nw + dx) * 4;
                out[d..d + 4].copy_from_slice(&src[s..s + 4]);
            }
        }
        RawImage { width: nw as i32, height: nh as i32, data: Arc::new(out) }
    }
}

#[derive(Clone)]
pub struct Frame {
    pub image: RawImage,
    pub delay_ms: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
/// Images are always shown in sRGB (what normal monitors use).
pub enum TargetColorSpace {
    Srgb,
}

#[derive(Clone)]
pub struct ReadData {
    pub image: Option<RawImage>,
    pub frames: Option<Arc<Vec<Frame>>>,
    pub absolute_file_path: String,
    pub file_size: u64,
    pub image_size: (i32, i32),
    pub target_color_space: TargetColorSpace,
    pub error_data: ErrorData,
}

impl ReadData {
    pub fn cost_kib(&self) -> usize {
        let mut b = self.image.as_ref().map(|i| i.bytes()).unwrap_or(0);
        if let Some(f) = &self.frames {
            b += f.iter().map(|f| f.image.bytes()).sum::<usize>();
        }
        (b / 1024).max(1)
    }
}

// ---- format support ------------------------------------------------------------------------

/// Extensions we can decode natively (lowercase, with dot).
const NATIVE_EXTS: &[&str] = &[
    ".bmp", ".dib", ".dds", ".exr", ".ff", ".gif", ".hdr", ".ico", ".cur", ".jpeg", ".jpg", ".jpe", ".jfi",
    ".jfif", ".pam", ".pbm", ".pgm", ".png", ".apng", ".pnm", ".ppm", ".qoi", ".svg", ".svgz", ".tga",
    ".tif", ".tiff", ".webp",
];

const NATIVE_MIMES: &[&str] = &[
    "image/bmp", "image/x-bmp", "image/x-ms-bmp", "image/vnd.ms-dds", "image/x-dds", "image/x-exr",
    "image/aces", "image/gif", "image/vnd.radiance", "image/x-hdr", "image/vnd.microsoft.icon",
    "image/x-icon", "image/x-win-bitmap", "image/jpeg", "image/pjpeg", "image/x-portable-anymap",
    "image/x-portable-bitmap", "image/x-portable-graymap", "image/x-portable-pixmap", "image/png",
    "image/apng", "image/vnd.mozilla.apng", "image/qoi", "image/x-qoi", "image/svg+xml",
    "image/svg+xml-compressed", "image/x-tga", "image/x-targa", "image/tiff", "image/webp",
];

pub struct FormatLists {
    pub extensions: Vec<String>,
    pub mime_types: Vec<String>,
}

/// QVApplication::defineFilterLists equivalent
pub fn format_lists() -> FormatLists {
    let mut extensions: Vec<String> = NATIVE_EXTS.iter().map(|s| s.to_string()).collect();
    let mut mime_types: Vec<String> = NATIVE_MIMES.iter().map(|s| s.to_string()).collect();
    for f in gdk_pixbuf::Pixbuf::formats() {
        if f.is_disabled() {
            continue;
        }
        for e in f.extensions() {
            let e = format!(".{}", e.to_ascii_lowercase());
            if e == ".pdf" {
                continue;
            }
            if !extensions.contains(&e) {
                extensions.push(e);
            }
        }
        for m in f.mime_types() {
            let m = m.to_string();
            if m == "application/pdf" {
                continue;
            }
            if !mime_types.contains(&m) {
                mime_types.push(m);
            }
        }
    }
    extensions.sort();
    mime_types.sort();
    FormatLists { extensions, mime_types }
}

// ---- color management ------------------------------------------------------------------------

pub struct ColorContext {
    pub target: TargetColorSpace,
}

/// True if the image carries a color profile that isn't plain sRGB (e.g. Display P3 from
/// iPhones, Adobe RGB from cameras), i.e. its colors need converting for a normal screen.
fn needs_conversion(embedded_icc: Option<&[u8]>) -> Option<lcms2::Profile> {
    let p = lcms2::Profile::new_icc(embedded_icc?).ok()?;
    if p.color_space() != lcms2::ColorSpaceSignature::RgbData {
        return None;
    }
    let desc = p.info(lcms2::InfoType::Description, lcms2::Locale::none()).unwrap_or_default();
    if desc.to_ascii_lowercase().contains("srgb") {
        return None;
    }
    Some(p)
}

/// Convert RGBA8 pixels in place from the embedded profile to sRGB. Untagged and
/// sRGB-tagged images are left alone.
fn convert_color_space(rgba: &mut [u8], embedded_icc: Option<&[u8]>, _ctx: &ColorContext) {
    let Some(src) = needs_conversion(embedded_icc) else { return };
    let dst = lcms2::Profile::new_srgb();
    if let Ok(t) = lcms2::Transform::<[u8; 4], [u8; 4]>::new(
        &src,
        lcms2::PixelFormat::RGBA_8,
        &dst,
        lcms2::PixelFormat::RGBA_8,
        lcms2::Intent::Perceptual,
    ) {
        let px: &mut [[u8; 4]] = unsafe { std::slice::from_raw_parts_mut(rgba.as_mut_ptr() as *mut [u8; 4], rgba.len() / 4) };
        t.transform_in_place(px);
    }
}

/// RGBA8 straight alpha -> Cairo ARGB32 premultiplied (native endian), parallel by rows
pub fn rgba_to_cairo(rgba: &[u8]) -> Vec<u8> {
    use rayon::prelude::*;
    let mut out = vec![0u8; rgba.len()];
    const CHUNK: usize = 64 * 1024 * 4;
    out.par_chunks_mut(CHUNK).zip(rgba.par_chunks(CHUNK)).for_each(|(o, i)| {
        for (d, p) in o.chunks_exact_mut(4).zip(i.chunks_exact(4)) {
            let (r, g, b, a) = (p[0] as u32, p[1] as u32, p[2] as u32, p[3] as u32);
            let px = if a == 255 {
                0xff00_0000 | (r << 16) | (g << 8) | b
            } else if a == 0 {
                0
            } else {
                let m = |c: u32| (c * a + 127) / 255;
                (a << 24) | (m(r) << 16) | (m(g) << 8) | m(b)
            };
            d.copy_from_slice(&px.to_ne_bytes());
        }
    });
    out
}

/// RGB8 -> Cairo ARGB32 (opaque), parallel
pub fn rgb_to_cairo(rgb: &[u8]) -> Vec<u8> {
    use rayon::prelude::*;
    let n = rgb.len() / 3;
    let mut out = vec![0u8; n * 4];
    const PX: usize = 64 * 1024;
    out.par_chunks_mut(PX * 4).zip(rgb.par_chunks(PX * 3)).for_each(|(o, i)| {
        for (d, p) in o.chunks_exact_mut(4).zip(i.chunks_exact(3)) {
            let px = 0xff00_0000 | ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32;
            d.copy_from_slice(&px.to_ne_bytes());
        }
    });
    out
}

/// Cairo ARGB32 premultiplied -> RGBA8 straight (for saving / clipboard)
pub fn cairo_to_rgba(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for p in data.chunks_exact(4) {
        let px = u32::from_ne_bytes([p[0], p[1], p[2], p[3]]);
        let a = px >> 24;
        let (r, g, b) = ((px >> 16) & 0xff, (px >> 8) & 0xff, px & 0xff);
        if a == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else if a == 255 {
            out.extend_from_slice(&[r as u8, g as u8, b as u8, 255]);
        } else {
            let u = |c: u32| ((c * 255 + a / 2) / a).min(255) as u8;
            out.extend_from_slice(&[u(r), u(g), u(b), a as u8]);
        }
    }
    out
}

fn raw_from_rgba(w: u32, h: u32, rgba: &[u8]) -> RawImage {
    RawImage { width: w as i32, height: h as i32, data: Arc::new(rgba_to_cairo(rgba)) }
}

// ---- decoding -------------------------------------------------------------------------------

fn ext_of(path: &str) -> String {
    Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn looks_like_svg(head: &[u8]) -> bool {
    if head.starts_with(&[0x1f, 0x8b]) {
        return false; // gzip; only treated as svgz by extension
    }
    let s = String::from_utf8_lossy(&head[..head.len().min(1024)]).to_ascii_lowercase();
    s.contains("<svg")
}

/// Port of QVImageCore::readFile.
pub fn read_file(path: &str, ctx: &ColorContext, largest_dimension: i32) -> ReadData {
    let _t = crate::util::Trace::new("decode");
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut rd = ReadData {
        image: None,
        frames: None,
        absolute_file_path: path.to_string(),
        file_size,
        image_size: (-1, -1),
        target_color_space: ctx.target,
        error_data: ErrorData::default(),
    };

    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            rd.error_data =
                ErrorData::new(if e.kind() == std::io::ErrorKind::NotFound { 1 } else { 2 });
            return rd;
        }
    };
    let mut head = vec![0u8; 1024];
    let n = file.read(&mut head).unwrap_or(0);
    head.truncate(n);
    let _ = file.rewind();

    let ext = ext_of(path);
    if ext == "svg" || ext == "svgz" || looks_like_svg(&head) {
        match read_svg(path, largest_dimension) {
            Ok((img, size)) => {
                rd.image = Some(img);
                rd.image_size = size;
            }
            Err(e) => rd.error_data = e,
        }
        return rd;
    }

    match read_raster(file, path, ctx) {
        Ok((img, frames, size)) => {
            rd.image = Some(img);
            rd.frames = frames.map(Arc::new);
            rd.image_size = size;
        }
        Err(e) => {
            // Fall back to gdk-pixbuf for formats the image crate doesn't know
            match read_pixbuf(path) {
                Some((img, size)) => {
                    rd.image = Some(img);
                    rd.image_size = size;
                }
                None => rd.error_data = e,
            }
        }
    }
    rd
}

type RasterResult = Result<(RawImage, Option<Vec<Frame>>, (i32, i32)), ErrorData>;

fn map_err(e: image::ImageError) -> ErrorData {
    match e {
        image::ImageError::Unsupported(_) => ErrorData::new(3),
        image::ImageError::IoError(io) if io.kind() == std::io::ErrorKind::NotFound => ErrorData::new(1),
        image::ImageError::IoError(_) => ErrorData::new(4),
        image::ImageError::Decoding(_) | image::ImageError::Limits(_) => ErrorData::new(4),
        _ => ErrorData::new(0),
    }
}

fn read_raster(file: std::fs::File, path: &str, ctx: &ColorContext) -> RasterResult {
    use image::{AnimationDecoder, ImageDecoder, ImageFormat};
    let reader = image::ImageReader::new(BufReader::new(file)).with_guessed_format().map_err(|_| ErrorData::new(4))?;
    let format = reader.format().or_else(|| ImageFormat::from_path(path).ok()).ok_or_else(|| ErrorData::new(3))?;

    // Animated formats
    let frames_from = |frames: image::Frames| -> Result<Vec<Frame>, ErrorData> {
        let mut out = vec![];
        let mut total: usize = 0;
        for f in frames {
            let f = f.map_err(map_err)?;
            let (n, d) = f.delay().numer_denom_ms();
            let delay = if d == 0 { 0 } else { n / d };
            let buf = f.into_buffer();
            let (w, h) = buf.dimensions();
            total += (w as usize) * (h as usize) * 4;
            if total > 2 * 1024 * 1024 * 1024 {
                break; // keep memory sane for gigantic animations
            }
            let mut rgba = buf.into_raw();
            convert_color_space(&mut rgba, None, ctx);
            out.push(Frame { image: raw_from_rgba(w, h, &rgba), delay_ms: delay });
        }
        Ok(out)
    };

    let open = || std::fs::File::open(path).map(BufReader::new).map_err(|_| ErrorData::new(2));
    match format {
        ImageFormat::Gif => {
            let dec = image::codecs::gif::GifDecoder::new(open()?).map_err(map_err)?;
            let (w, h) = dec.dimensions();
            let frames = frames_from(dec.into_frames())?;
            return finish_animation(frames, (w as i32, h as i32));
        }
        ImageFormat::WebP => {
            let dec = image::codecs::webp::WebPDecoder::new(open()?).map_err(map_err)?;
            if dec.has_animation() {
                let (w, h) = dec.dimensions();
                let frames = frames_from(dec.into_frames())?;
                return finish_animation(frames, (w as i32, h as i32));
            }
        }
        ImageFormat::Png => {
            let dec = image::codecs::png::PngDecoder::new(open()?).map_err(map_err)?;
            if dec.is_apng().unwrap_or(false) {
                let (w, h) = dec.dimensions();
                let frames = frames_from(dec.apng().map_err(map_err)?.into_frames())?;
                return finish_animation(frames, (w as i32, h as i32));
            }
        }
        _ => {}
    }

    // Static image with orientation + ICC
    let mut dec = reader.into_decoder().map_err(map_err)?;
    let (w, h) = dec.dimensions();
    let icc = dec.icc_profile().ok().flatten();
    let orientation = dec.orientation().ok();
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(8 * 1024 * 1024 * 1024);
    dec.set_limits(limits).map_err(map_err)?;
    let mut img = image::DynamicImage::from_decoder(dec).map_err(map_err)?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    // Fast path: opaque 8-bit RGB that needs no color conversion (most photos)
    let needs_color = needs_conversion(icc.as_deref()).is_some();
    if let (image::DynamicImage::ImageRgb8(rgb), false) = (&img, needs_color) {
        let (iw, ih) = rgb.dimensions();
        let data = rgb_to_cairo(rgb.as_raw());
        return Ok((RawImage { width: iw as i32, height: ih as i32, data: Arc::new(data) }, None, (w as i32, h as i32)));
    }
    let rgba = img.into_rgba8();
    let (iw, ih) = rgba.dimensions();
    let mut raw = rgba.into_raw();
    convert_color_space(&mut raw, icc.as_deref(), ctx);
    Ok((raw_from_rgba(iw, ih, &raw), None, (w as i32, h as i32)))
}

fn finish_animation(frames: Vec<Frame>, size: (i32, i32)) -> RasterResult {
    if frames.is_empty() {
        return Err(ErrorData::new(4));
    }
    let first = frames[0].image.clone();
    if frames.len() == 1 {
        return Ok((first, None, size));
    }
    Ok((first, Some(frames), size))
}

fn read_svg(path: &str, largest: i32) -> Result<(RawImage, (i32, i32)), ErrorData> {
    let data = std::fs::read(path).map_err(|_| ErrorData::new(1))?;
    let mut opt = resvg::usvg::Options { resources_dir: Path::new(path).parent().map(|p| p.to_path_buf()), ..Default::default() };
    opt.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_data(&data, &opt).map_err(|_| ErrorData::new(4))?;
    let size = tree.size();
    let (sw, sh) = (size.width(), size.height());
    // QIcon::pixmap(largestDimension): fit inside an L x L square, keeping aspect ratio
    let l = largest.max(1) as f32;
    let scale = (l / sw).min(l / sh);
    let w = (sw * scale).round().max(1.0) as u32;
    let h = (sh * scale).round().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or_else(|| ErrorData::new(4))?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(w as f32 / sw, h as f32 / sh), &mut pixmap.as_mut());
    // tiny-skia is premultiplied RGBA -> convert to premultiplied BGRA
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for p in pixmap.data().chunks_exact(4) {
        let px = ((p[3] as u32) << 24) | ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32;
        out.extend_from_slice(&px.to_ne_bytes());
    }
    Ok((RawImage { width: w as i32, height: h as i32, data: Arc::new(out) }, (sw.round() as i32, sh.round() as i32)))
}

fn read_pixbuf(path: &str) -> Option<(RawImage, (i32, i32))> {
    let pb = gdk_pixbuf::Pixbuf::from_file(path).ok()?;
    let pb = pb.apply_embedded_orientation().unwrap_or(pb);
    let (w, h) = (pb.width(), pb.height());
    let pb = if pb.has_alpha() { pb } else { pb.add_alpha(false, 0, 0, 0).ok()? };
    let stride = pb.rowstride() as usize;
    let bytes = pb.read_pixel_bytes();
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        rgba.extend_from_slice(&bytes[y * stride..y * stride + w as usize * 4]);
    }
    let size = gdk_pixbuf::Pixbuf::file_info(path).map(|(_, w, h)| (w, h)).unwrap_or((w, h));
    Some((raw_from_rgba(w as u32, h as u32, &rgba), size))
}

/// Smooth scale, like QPixmap::scaled(..., Qt::SmoothTransformation): area averaging when
/// shrinking, bilinear when enlarging. SIMD + multithreaded; data is already premultiplied.
pub fn scale_smooth(src: &RawImage, w: i32, h: i32) -> RawImage {
    use fast_image_resize as fr;
    let (w, h) = (w.max(1) as u32, h.max(1) as u32);
    let Ok(src_img) = fr::images::ImageRef::new(src.width as u32, src.height as u32, &src.data, fr::PixelType::U8x4) else {
        return src.clone();
    };
    let mut dst = fr::images::Image::new(w, h, fr::PixelType::U8x4);
    let filter = if w < src.width as u32 || h < src.height as u32 { fr::FilterType::Box } else { fr::FilterType::Bilinear };
    let opts = fr::ResizeOptions::new().resize_alg(fr::ResizeAlg::Convolution(filter)).use_alpha(false);
    if fr::Resizer::new().resize(&src_img, &mut dst, &opts).is_err() {
        return src.clone();
    }
    RawImage { width: w as i32, height: h as i32, data: Arc::new(dst.into_vec()) }
}

#[cfg(test)]
mod color_tests {
    use super::*;

    fn p3_icc() -> Vec<u8> {
        let white = lcms2::CIExyY { x: 0.3127, y: 0.3290, Y: 1.0 };
        let prim = lcms2::CIExyYTRIPLE {
            Red: lcms2::CIExyY { x: 0.680, y: 0.320, Y: 1.0 },
            Green: lcms2::CIExyY { x: 0.265, y: 0.690, Y: 1.0 },
            Blue: lcms2::CIExyY { x: 0.150, y: 0.060, Y: 1.0 },
        };
        let curve = lcms2::ToneCurve::new_parametric(4, &[2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045]).unwrap();
        let p = lcms2::Profile::new_rgb(&white, &prim, &[&curve, &curve, &curve]).unwrap();
        p.icc().unwrap()
    }

    #[test]
    fn p3_is_converted_srgb_is_not() {
        let ctx = ColorContext { target: TargetColorSpace::Srgb };
        let mut px = vec![200u8, 100, 50, 255];
        convert_color_space(&mut px, Some(&p3_icc()), &ctx);
        assert_ne!(px, vec![200, 100, 50, 255], "P3 pixel should change");
        assert!(px[0] > 200, "P3 orange is more saturated than sRGB: {:?}", px);

        let srgb = lcms2::Profile::new_srgb().icc().unwrap();
        let mut px = vec![200u8, 100, 50, 255];
        convert_color_space(&mut px, Some(&srgb), &ctx);
        assert_eq!(px, vec![200, 100, 50, 255]);

        let mut px = vec![200u8, 100, 50, 255];
        convert_color_space(&mut px, None, &ctx);
        assert_eq!(px, vec![200, 100, 50, 255]);
    }
}
