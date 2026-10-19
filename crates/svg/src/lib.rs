//! DeckCraft SVG pictures: tells SVG from other picture bytes, reads its natural size, and draws it
//! to straight RGBA or to a PNG through `resvg` (pure Rust, so it builds for wasm).
//!
//! PowerPoint stores an SVG picture as an `asvg:svgBlip` next to a PNG fallback; some files carry
//! the SVG alone. The renderer draws SVGs at the size the slide needs, and the `.pptx` writer makes
//! the PNG fallback from here when a picture has none.
//!
//! Hostile input: only images embedded in the SVG (`data:` URLs) are loaded, never files or URLs,
//! so a picture can't pull anything from the machine into what is drawn or saved. Input size,
//! output size and pixel count are capped, and the parser and renderer run behind a panic guard;
//! anything unreadable is `None`. Text isn't drawn (no fonts are loaded); [`has_text`] lets a caller
//! prefer a fallback picture that shows it.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{ImageHrefResolver, Options, Tree};

/// Largest SVG read, in bytes.
pub const MAX_BYTES: usize = 64 << 20;
/// Longest side drawn, in pixels.
pub const MAX_SIDE: u32 = 8192;
/// Most pixels drawn at once (64 MP, 256 MB of RGBA).
const MAX_PIXELS: u64 = 64 << 20;

/// A drawn SVG: straight (not premultiplied) RGBA, row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Do these bytes look like an SVG document (`<svg` near the start, after any XML declaration,
/// comments or doctype)? Compressed SVG (`.svgz`) isn't read.
pub fn is_svg(bytes: &[u8]) -> bool {
    let head = bytes.get(..bytes.len().min(4096)).unwrap_or(bytes);
    let head = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    let Some(start) = head.iter().position(|b| !b.is_ascii_whitespace()) else { return false };
    head.get(start) == Some(&b'<') && find(head, b"<svg").is_some()
}

/// Does the SVG have text? It isn't drawn here, so a fallback picture shows it better.
pub fn has_text(bytes: &[u8]) -> bool {
    is_svg(bytes) && find(bytes, b"<text").is_some()
}

/// The SVG's own size in CSS pixels (96 per inch), as `width` and `height` or the `viewBox` give it.
pub fn natural_size(bytes: &[u8]) -> Option<(f64, f64)> {
    let size = tree(bytes)?.size();
    let (w, h) = (f64::from(size.width()), f64::from(size.height()));
    (w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0).then_some((w, h))
}

/// Draw the SVG with its longer side `side` pixels (at most [`MAX_SIDE`]), transparent where it
/// draws nothing.
pub fn rasterize(bytes: &[u8], side: u32) -> Option<Raster> {
    let pm = pixmap(bytes, side)?;
    let rgba = pm
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Some(Raster { width: pm.width(), height: pm.height(), rgba })
}

/// The SVG drawn as a PNG with its longer side `side` pixels: the fallback PowerPoint keeps next to
/// an SVG picture for readers that don't draw SVG.
pub fn png(bytes: &[u8], side: u32) -> Option<Vec<u8>> {
    let pm = pixmap(bytes, side)?;
    catch_unwind(AssertUnwindSafe(|| pm.encode_png().ok())).ok().flatten()
}

fn pixmap(bytes: &[u8], side: u32) -> Option<Pixmap> {
    let tree = tree(bytes)?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return None;
    }
    let k = side.clamp(1, MAX_SIDE) as f32 / w.max(h);
    let px = |v: f32| (v * k).ceil().clamp(1.0, MAX_SIDE as f32) as u32;
    let (pw, ph) = (px(w), px(h));
    if u64::from(pw) * u64::from(ph) > MAX_PIXELS {
        return None;
    }
    let mut pm = Pixmap::new(pw, ph)?;
    let t = Transform::from_scale(pw as f32 / w, ph as f32 / h);
    catch_unwind(AssertUnwindSafe(|| resvg::render(&tree, t, &mut pm.as_mut()))).ok()?;
    Some(pm)
}

fn tree(bytes: &[u8]) -> Option<Tree> {
    if bytes.len() > MAX_BYTES || !is_svg(bytes) {
        return None;
    }
    let opt = Options {
        image_href_resolver: ImageHrefResolver {
            resolve_data: ImageHrefResolver::default_data_resolver(),
            // A file path or URL in `href`: never followed.
            resolve_string: Box::new(|_, _| None),
        },
        ..Options::default()
    };
    catch_unwind(AssertUnwindSafe(|| Tree::from_data(bytes, &opt).ok())).ok().flatten()
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests;
