//! Pure-Rust visualization for NASA OPERA layers: PNG previews, before/after comparison
//! panels, animated time-series GIFs, and self-contained interactive MapLibre GL JS HTML viewers.

use std::io::Cursor;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{OperaError, Result};
use crate::flood_extent::RgbaImage;
#[cfg(feature = "http")]
use crate::flood_extent::{TILE_SIZE, choose_zoom, decode_png_rgba, tile_grid_for_bbox};
use crate::products::{LayerStyle, legend_entries};
use crate::titiler::TitilerQuery;
#[cfg(feature = "http")]
use crate::titiler::build_tile_url_template;
use crate::types::BBox;

/// Public CARTO Positron XYZ basemap template (no API key required).
pub const CARTO_POSITRON_TEMPLATE: &str =
    "https://a.basemaps.cartocdn.com/light_all/{z}/{x}/{y}.png";

/// Public CARTO Voyager XYZ basemap template (no API key required).
pub const CARTO_VOYAGER_TEMPLATE: &str =
    "https://a.basemaps.cartocdn.com/rastertiles/voyager/{z}/{x}/{y}.png";

/// Options for rendering a static PNG preview of an OPERA layer over an AOI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewOptions {
    /// Target AOI bounding box `[west, south, east, north]`.
    pub bbox: BBox,
    /// Product short name (used for legend and title).
    pub product: String,
    /// Band token (e.g. `B01_WTR`, `VV`, `VEG-DIST-STATUS`).
    pub band: Option<String>,
    /// Water-only mode for DSWx WTR bands.
    pub water_only: bool,
    /// Maximum width/height of the output PNG in pixels (default: 1024).
    pub max_dim: usize,
    /// Maximum tiles across the longer AOI dimension (default: 4).
    pub max_tiles_across: usize,
    /// Whether to composite CARTO Voyager basemap tiles underneath the OPERA layer.
    pub basemap: bool,
    /// Whether to draw a metric scale bar.
    pub scale_bar: bool,
    /// Whether to draw a categorical legend (when applicable).
    pub legend: bool,
    /// Optional title banner drawn at top-left.
    pub title: Option<String>,
}

impl PreviewOptions {
    /// Creates preview options with sensible agent-ready defaults (basemap + scale bar + legend).
    pub fn new(bbox: BBox, product: impl Into<String>, band: Option<String>) -> Self {
        Self {
            bbox,
            product: product.into(),
            band,
            water_only: false,
            max_dim: 1024,
            max_tiles_across: 4,
            basemap: true,
            scale_bar: true,
            legend: true,
            title: None,
        }
    }
}

/// Encodes an [`RgbaImage`] as PNG bytes in pure Rust.
pub fn encode_png_rgba(img: &RgbaImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder =
            png::Encoder::new(Cursor::new(&mut out), img.width as u32, img.height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| OperaError::Image(format!("PNG header encode failed: {e}")))?;
        writer
            .write_image_data(&img.data)
            .map_err(|e| OperaError::Image(format!("PNG data encode failed: {e}")))?;
    }
    Ok(out)
}

/// Alpha-composites `fg` over `bg` in place at offset `(ox, oy)`.
pub fn composite_over(bg: &mut RgbaImage, fg: &RgbaImage, ox: usize, oy: usize) {
    let copy_h = fg.height.min(bg.height.saturating_sub(oy));
    let copy_w = fg.width.min(bg.width.saturating_sub(ox));
    for y in 0..copy_h {
        for x in 0..copy_w {
            let fi = (y * fg.width + x) * 4;
            let sa = fg.data[fi + 3] as u32;
            if sa == 0 {
                continue;
            }
            let bi = ((oy + y) * bg.width + (ox + x)) * 4;
            if sa == 255 {
                bg.data[bi..bi + 4].copy_from_slice(&fg.data[fi..fi + 4]);
            } else {
                let da = bg.data[bi + 3] as u32;
                let inv_sa = 255 - sa;
                let out_a = sa + (da * inv_sa) / 255;
                if out_a == 0 {
                    continue;
                }
                for c in 0..3 {
                    let s = fg.data[fi + c] as u32;
                    let d = bg.data[bi + c] as u32;
                    let num = s * sa * 255 + d * da * inv_sa;
                    bg.data[bi + c] = (num / (out_a * 255)).min(255) as u8;
                }
                bg.data[bi + 3] = out_a.min(255) as u8;
            }
        }
    }
}

/// Crops `img` to `[x0..x1, y0..y1]` and downscales via nearest-neighbor if either dimension exceeds `max_dim`.
pub fn crop_and_resize(
    img: &RgbaImage,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    max_dim: usize,
) -> RgbaImage {
    let cx0 = x0.min(img.width.saturating_sub(1));
    let cy0 = y0.min(img.height.saturating_sub(1));
    let cx1 = x1.clamp(cx0 + 1, img.width);
    let cy1 = y1.clamp(cy0 + 1, img.height);
    let cw = cx1 - cx0;
    let ch = cy1 - cy0;
    let max_d = max_dim.max(64);

    let (tw, th) = if cw > max_d || ch > max_d {
        let scale = (max_d as f64) / (cw.max(ch) as f64);
        (
            ((cw as f64) * scale).round().max(1.0) as usize,
            ((ch as f64) * scale).round().max(1.0) as usize,
        )
    } else {
        (cw, ch)
    };

    let mut data = vec![0u8; tw * th * 4];
    for y in 0..th {
        let sy = cy0 + (y * ch) / th;
        for x in 0..tw {
            let sx = cx0 + (x * cw) / tw;
            let si = (sy * img.width + sx) * 4;
            let di = (y * tw + x) * 4;
            data[di..di + 4].copy_from_slice(&img.data[si..si + 4]);
        }
    }
    RgbaImage {
        width: tw,
        height: th,
        data,
    }
}

fn fill_rect(img: &mut RgbaImage, x: usize, y: usize, w: usize, h: usize, rgba: [u8; 4]) {
    let x1 = (x + w).min(img.width);
    let y1 = (y + h).min(img.height);
    let sa = rgba[3] as u32;
    let inv = 255 - sa;
    for py in y.min(img.height)..y1 {
        for px in x.min(img.width)..x1 {
            let i = (py * img.width + px) * 4;
            if sa == 255 {
                img.data[i..i + 4].copy_from_slice(&rgba);
            } else {
                for c in 0..3 {
                    img.data[i + c] =
                        ((rgba[c] as u32 * sa + img.data[i + c] as u32 * inv) / 255) as u8;
                }
                img.data[i + 3] = 255;
            }
        }
    }
}

/// Minimal 5x7 uppercase/numeric bitmap glyph lookup for crisp zero-dependency HUD labels.
fn glyph_5x7(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x12, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1F],
        '3' => [0x1E, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x1E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x0E, 0x10, 0x1E, 0x11, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
        '/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        '%' => [0x19, 0x19, 0x02, 0x04, 0x08, 0x13, 0x13],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '<' => [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
        '>' => [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08],
        '=' => [0x00, 0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        _ => [0x00; 7],
    }
}

fn draw_text(img: &mut RgbaImage, x: usize, y: usize, text: &str, color: [u8; 4], scale: usize) {
    let s = scale.max(1);
    let mut cx = x;
    for ch in text.chars() {
        let rows = glyph_5x7(ch);
        for (ry, &bits) in rows.iter().enumerate() {
            for rx in 0..5 {
                if (bits & (1 << (4 - rx))) != 0 {
                    fill_rect(img, cx + rx * s, y + ry * s, s, s, color);
                }
            }
        }
        cx += 6 * s;
    }
}

/// Overlays optional title banner, metric scale bar, and categorical legend onto `img`.
pub fn decorate_preview(img: &mut RgbaImage, opts: &PreviewOptions) {
    if let Some(title) = opts.title.as_deref().filter(|s| !s.trim().is_empty()) {
        let pad = 8usize;
        let box_w = (title.len() * 6 + pad * 2).min(img.width.saturating_sub(16));
        fill_rect(img, 8, 8, box_w, 24, [15, 23, 42, 210]);
        draw_text(img, 14, 16, title, [255, 255, 255, 255], 1);
    }

    if opts.scale_bar && img.width >= 160 && img.height >= 80 {
        let mean_lat = 0.5 * (opts.bbox.south + opts.bbox.north);
        let span_m =
            (opts.bbox.east - opts.bbox.west) * 111_320.0 * mean_lat.to_radians().cos().abs();
        let target_m = span_m * 0.2;
        let nice_steps = [
            100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0, 50_000.0,
            100_000.0, 200_000.0, 500_000.0,
        ];
        let bar_m = nice_steps
            .iter()
            .copied()
            .min_by(|a, b| (a - target_m).abs().total_cmp(&(b - target_m).abs()))
            .unwrap_or(1_000.0);
        let bar_px = ((bar_m / span_m.max(1.0)) * img.width as f64)
            .round()
            .clamp(32.0, (img.width as f64 * 0.35).max(32.0)) as usize;
        let label = if bar_m >= 1_000.0 {
            format!("{} KM", (bar_m / 1_000.0).round() as u64)
        } else {
            format!("{} M", bar_m.round() as u64)
        };
        let bx = 12;
        let by = img.height.saturating_sub(36);
        let bw = bar_px.max(label.len() * 6) + 16;
        fill_rect(img, bx, by, bw, 28, [15, 23, 42, 205]);
        draw_text(img, bx + 8, by + 5, &label, [255, 255, 255, 255], 1);
        fill_rect(img, bx + 8, by + 17, bar_px, 5, [255, 255, 255, 255]);
        fill_rect(img, bx + 8, by + 17, bar_px / 2, 5, [56, 189, 248, 255]);
    }

    let mut legend_height = 0usize;
    if opts.legend {
        let entries = legend_entries(&opts.product, opts.band.as_deref(), opts.water_only);
        if !entries.is_empty() && img.width >= 200 && img.height >= 120 {
            let max_chars = entries.iter().map(|(l, _)| l.len()).max().unwrap_or(8);
            let box_w = (max_chars * 6 + 34).min(img.width.saturating_sub(16));
            let box_h = entries.len() * 16 + 12;
            let lx = img.width.saturating_sub(box_w + 12);
            let ly = img.height.saturating_sub(box_h + 12);
            fill_rect(img, lx, ly, box_w, box_h, [15, 23, 42, 210]);
            for (i, (label, color)) in entries.iter().enumerate() {
                let row_y = ly + 8 + i * 16;
                fill_rect(img, lx + 8, row_y, 12, 10, *color);
                draw_text(img, lx + 26, row_y + 1, label, [255, 255, 255, 255], 1);
            }
            legend_height = box_h + 16;
        }
    }

    if opts.basemap && img.width >= 160 && img.height >= 64 {
        let attr = "(C) OSM & CARTO";
        let box_w = attr.len() * 6 + 12;
        let box_h = 15;
        let ax = img.width.saturating_sub(box_w + 12);
        let ay = img.height.saturating_sub(box_h + 8 + legend_height).max(8);
        fill_rect(img, ax, ay, box_w, box_h, [15, 23, 42, 175]);
        draw_text(img, ax + 6, ay + 4, attr, [226, 232, 240, 255], 1);
    }
}

/// Stitches one or more XYZ tile templates (in bottom-to-top order) into an [`RgbaImage`]
/// cropped to `bbox` and resized to `max_dim`.
#[cfg(feature = "http")]
pub fn stitch_tile_templates(
    bbox: BBox,
    tile_templates: &[String],
    max_tiles_across: usize,
    max_dim: usize,
    basemap: bool,
) -> Result<RgbaImage> {
    let zoom = choose_zoom(bbox, max_tiles_across)?;
    let grid = tile_grid_for_bbox(bbox, zoom)?;
    let (tx0, ty0, tx1, ty1) = (grid.tx0, grid.ty0, grid.tx1, grid.ty1);
    let (gx0, gy0, gx1, gy1) = (grid.gx0, grid.gy0, grid.gx1, grid.gy1);
    let cols = (tx1 - tx0 + 1) as usize;
    let rows = (ty1 - ty0 + 1) as usize;
    let stitch_w = cols * TILE_SIZE;
    let stitch_h = rows * TILE_SIZE;

    let mut canvas = RgbaImage {
        width: stitch_w,
        height: stitch_h,
        data: [241, 245, 249, 255].repeat(stitch_w * stitch_h),
    };

    let mut layers: Vec<(bool, String)> = Vec::new();
    if basemap {
        layers.push((true, CARTO_VOYAGER_TEMPLATE.to_string()));
    }
    for tpl in tile_templates {
        layers.push((false, tpl.clone()));
    }

    let mut opera_succeeded = 0usize;
    let mut opera_failed = 0usize;
    let mut first_opera_error: Option<String> = None;
    const BATCH_SIZE: usize = 16;

    for (is_basemap, template) in &layers {
        let mut jobs = Vec::new();
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let url = template
                    .replace("{z}", &zoom.to_string())
                    .replace("{x}", &tx.to_string())
                    .replace("{y}", &ty.to_string());
                jobs.push((tx, ty, url));
            }
        }

        for chunk in jobs.chunks(BATCH_SIZE) {
            #[cfg(feature = "parallel")]
            let tiles: Vec<(u32, u32, Result<RgbaImage>)> = {
                use rayon::prelude::*;
                chunk
                    .par_iter()
                    .map(|(tx, ty, url)| {
                        let res = crate::cmr::http_get_bytes(url).and_then(|b| decode_png_rgba(&b));
                        (*tx, *ty, res)
                    })
                    .collect()
            };

            #[cfg(not(feature = "parallel"))]
            let tiles: Vec<(u32, u32, Result<RgbaImage>)> = chunk
                .iter()
                .map(|(tx, ty, url)| {
                    let res = crate::cmr::http_get_bytes(url).and_then(|b| decode_png_rgba(&b));
                    (*tx, *ty, res)
                })
                .collect();

            for (tx, ty, res) in tiles {
                match res {
                    Ok(tile) => {
                        if !*is_basemap {
                            opera_succeeded += 1;
                        }
                        let ox = (tx - tx0) as usize * TILE_SIZE;
                        let oy = (ty - ty0) as usize * TILE_SIZE;
                        composite_over(&mut canvas, &tile, ox, oy);
                    }
                    Err(err) => {
                        if !*is_basemap {
                            opera_failed += 1;
                            if first_opera_error.is_none() {
                                first_opera_error = Some(err.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    if !tile_templates.is_empty() && opera_succeeded == 0 {
        return Err(OperaError::Http(format!(
            "all {opera_failed} OPERA tile request(s) failed{}",
            first_opera_error
                .map(|e| format!(" (first error: {e})"))
                .unwrap_or_default()
        )));
    }

    let origin_px = (tx0 as usize * TILE_SIZE) as f64;
    let origin_py = (ty0 as usize * TILE_SIZE) as f64;
    let cx0 = (gx0 - origin_px).floor().max(0.0) as usize;
    let cy0 = (gy0 - origin_py).floor().max(0.0) as usize;
    let cx1 = (gx1 - origin_px).ceil().max(1.0) as usize;
    let cy1 = (gy1 - origin_py).ceil().max(1.0) as usize;

    Ok(crop_and_resize(&canvas, cx0, cy0, cx1, cy1, max_dim))
}

/// Renders a PNG preview of one or more `titiler-cmr` queries over `opts.bbox`.
#[cfg(feature = "http")]
pub fn render_preview_png(queries: &[TitilerQuery], opts: &PreviewOptions) -> Result<Vec<u8>> {
    if queries.is_empty() {
        return Err(OperaError::InvalidInput(
            "at least one titiler query is required to render a preview".into(),
        ));
    }
    let templates: Vec<String> = queries
        .iter()
        .map(|q| build_tile_url_template(q, TILE_SIZE as u32))
        .collect();
    let mut img = stitch_tile_templates(
        opts.bbox,
        &templates,
        opts.max_tiles_across,
        opts.max_dim,
        opts.basemap,
    )?;
    decorate_preview(&mut img, opts);
    encode_png_rgba(&img)
}

#[cfg(not(feature = "http"))]
pub fn render_preview_png(_queries: &[TitilerQuery], _opts: &PreviewOptions) -> Result<Vec<u8>> {
    Err(OperaError::HttpDisabled)
}

/// Places two [`RgbaImage`] panels side by side with a thin dark divider.
pub fn combine_side_by_side(left: &RgbaImage, right: &RgbaImage) -> RgbaImage {
    let gap = 6usize;
    let h = left.height.max(right.height);
    let w = left.width + gap + right.width;
    let mut out = RgbaImage {
        width: w,
        height: h,
        data: [15, 23, 42, 255].repeat(w * h),
    };
    composite_over(&mut out, left, 0, 0);
    composite_over(&mut out, right, left.width + gap, 0);
    out
}

/// Encodes a sequence of equal-sized [`RgbaImage`] frames into an animated GIF byte vector.
pub fn encode_animated_gif(frames: &[RgbaImage], delay_cs: u16) -> Result<Vec<u8>> {
    let first = frames.first().ok_or_else(|| {
        OperaError::InvalidInput("at least one frame is required to encode an animated GIF".into())
    })?;
    if first.width == 0 || first.height == 0 {
        return Err(OperaError::InvalidInput(
            "GIF frame dimensions must be non-zero".into(),
        ));
    }
    let w = u16::try_from(first.width)
        .map_err(|_| OperaError::InvalidInput("GIF frame width exceeds u16::MAX".into()))?;
    let h = u16::try_from(first.height)
        .map_err(|_| OperaError::InvalidInput("GIF frame height exceeds u16::MAX".into()))?;
    let expected_bytes = first
        .width
        .checked_mul(first.height)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| OperaError::InvalidInput("GIF frame buffer size overflow".into()))?;

    for (idx, frame_img) in frames.iter().enumerate() {
        if frame_img.width != first.width || frame_img.height != first.height {
            return Err(OperaError::InvalidInput(format!(
                "GIF frame {idx} has dimensions {}x{}, expected {}x{}",
                frame_img.width, frame_img.height, first.width, first.height
            )));
        }
        if frame_img.data.len() != expected_bytes {
            return Err(OperaError::InvalidInput(format!(
                "GIF frame {idx} RGBA buffer length is {}, expected {expected_bytes}",
                frame_img.data.len()
            )));
        }
    }

    let mut out = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut out, w, h, &[])
            .map_err(|e| OperaError::Image(format!("GIF encoder init failed: {e}")))?;
        encoder
            .set_repeat(gif::Repeat::Infinite)
            .map_err(|e| OperaError::Image(format!("GIF repeat failed: {e}")))?;

        for frame_img in frames {
            let mut rgba = frame_img.data.clone();
            let mut frame = gif::Frame::from_rgba_speed(w, h, &mut rgba, 10);
            frame.delay = delay_cs.max(10);
            encoder
                .write_frame(&frame)
                .map_err(|e| OperaError::Image(format!("GIF frame encode failed: {e}")))?;
        }
    }
    Ok(out)
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn escape_json_for_script(json: &str) -> String {
    json.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Layer descriptor for [`export_interactive_html`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HtmlTileLayer {
    pub id: String,
    pub label: String,
    pub tile_url: String,
    pub opacity: f32,
}

/// Generates a self-contained single-file MapLibre GL JS interactive HTML map showing
/// OPERA `titiler-cmr` raster layers, optional GeoJSON footprints/flood polygons, and layer toggles.
pub fn build_interactive_html(
    title: &str,
    bbox: BBox,
    layers: &[HtmlTileLayer],
    overlay_geojson: Option<&Value>,
    style_hint: Option<&LayerStyle>,
) -> Result<String> {
    let layers_json = escape_json_for_script(&serde_json::to_string(layers)?);
    let overlay_json = match overlay_geojson {
        Some(v) => escape_json_for_script(&serde_json::to_string(v)?),
        None => "null".to_string(),
    };
    let bounds_json = serde_json::to_string(&[[bbox.west, bbox.south], [bbox.east, bbox.north]])?;
    let style_note = escape_html(
        style_hint
            .and_then(|s| s.colormap_name.as_deref().or(s.rescale.as_deref()))
            .unwrap_or("OPERA default"),
    );

    let safe_title = escape_html(title);

    Ok(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0" />
<title>{safe_title}</title>
<link href="https://unpkg.com/maplibre-gl@5.2.0/dist/maplibre-gl.css" rel="stylesheet" integrity="sha384-Nq6PQ+9vJPvw7U/VfDELyrWoGQMsy0gi6QShhaSrGzkpF5KkM40csg2leky+YMTd" crossorigin="anonymous" />
<script src="https://unpkg.com/maplibre-gl@5.2.0/dist/maplibre-gl.js" integrity="sha384-VMpBpZqwtNOdABxFshfUPnyuqncfZtE0j1evmoWakxTDQ+me3EE4G48K/+dSDxSX" crossorigin="anonymous"></script>
<style>
  html, body, #map {{ margin: 0; padding: 0; width: 100%; height: 100%; font-family: system-ui, sans-serif; }}
  .panel {{
    position: absolute; top: 12px; left: 12px; z-index: 2; max-width: 320px;
    background: rgba(15, 23, 42, 0.9); color: #f8fafc; padding: 12px 14px;
    border-radius: 10px; box-shadow: 0 6px 20px rgba(0,0,0,0.3); font-size: 13px;
  }}
  .panel h1 {{ font-size: 15px; margin: 0 0 6px 0; }}
  .panel .meta {{ color: #94a3b8; font-size: 11px; margin-bottom: 8px; }}
  .layer-row {{ display: flex; align-items: center; gap: 6px; margin: 5px 0; }}
  .opacity {{ width: 100%; margin-top: 8px; }}
</style>
</head>
<body>
<div id="map"></div>
<div class="panel">
  <h1>{safe_title}</h1>
  <div class="meta">NASA OPERA via opera-rs &amp; titiler-cmr ({style_note})</div>
  <div id="layers"></div>
  <label style="display:block;margin-top:8px;font-size:12px;">Opacity <input id="opacity" class="opacity" type="range" min="0" max="1" step="0.05" value="0.85" /></label>
</div>
<script>
const OPERA_LAYERS = {layers_json};
const OVERLAY = {overlay_json};
const BOUNDS = {bounds_json};
const map = new maplibregl.Map({{
  container: 'map',
  style: {{
    version: 8,
    sources: {{
      carto: {{
        type: 'raster',
        tiles: ['https://a.basemaps.cartocdn.com/rastertiles/voyager/{{z}}/{{x}}/{{y}}.png'],
        tileSize: 256,
        attribution: '&copy; OpenStreetMap &copy; CARTO | NASA OPERA'
      }}
    }},
    layers: [{{ id: 'carto', type: 'raster', source: 'carto' }}]
  }},
  bounds: BOUNDS,
  fitBoundsOptions: {{ padding: 36 }}
}});
map.addControl(new maplibregl.NavigationControl(), 'top-right');
map.addControl(new maplibregl.ScaleControl({{ unit: 'metric' }}), 'bottom-left');
map.on('load', () => {{
  const container = document.getElementById('layers');
  for (const layer of OPERA_LAYERS) {{
    map.addSource(layer.id, {{ type: 'raster', tiles: [layer.tile_url], tileSize: 256 }});
    map.addLayer({{ id: layer.id, type: 'raster', source: layer.id, paint: {{ 'raster-opacity': layer.opacity }} }});
    const row = document.createElement('label');
    row.className = 'layer-row';
    const input = document.createElement('input');
    input.type = 'checkbox';
    input.checked = true;
    const span = document.createElement('span');
    span.textContent = layer.label;
    row.append(input, ' ', span);
    input.addEventListener('change', (e) => {{
      map.setLayoutProperty(layer.id, 'visibility', e.target.checked ? 'visible' : 'none');
    }});
    container.appendChild(row);
  }}
  if (OVERLAY) {{
    map.addSource('overlay', {{ type: 'geojson', data: OVERLAY }});
    map.addLayer({{ id: 'overlay-fill', type: 'fill', source: 'overlay', paint: {{ 'fill-color': '#f97316', 'fill-opacity': 0.15 }} }});
    map.addLayer({{ id: 'overlay-line', type: 'line', source: 'overlay', paint: {{ 'line-color': '#f97316', 'line-width': 2 }} }});
  }}
  document.getElementById('opacity').addEventListener('input', (e) => {{
    const val = parseFloat(e.target.value);
    for (const layer of OPERA_LAYERS) map.setPaintProperty(layer.id, 'raster-opacity', val);
  }});
}});
</script>
</body>
</html>
"#
    ))
}

/// Writes an interactive MapLibre GL JS HTML file to `out_path`.
pub fn export_interactive_html(
    out_path: impl AsRef<Path>,
    title: &str,
    bbox: BBox,
    layers: &[HtmlTileLayer],
    overlay_geojson: Option<&Value>,
    style_hint: Option<&LayerStyle>,
) -> Result<()> {
    let html = build_interactive_html(title, bbox, layers, overlay_geojson, style_hint)?;
    let path = out_path.as_ref();
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, html)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flood_extent::decode_png_rgba;

    #[test]
    fn renders_decorated_png_and_gif_offline() {
        let mut img = RgbaImage {
            width: 320,
            height: 240,
            data: [200, 220, 240, 255].repeat(320 * 240),
        };
        let opts = PreviewOptions {
            title: Some("DSWX-HLS 2023-04-30".into()),
            ..PreviewOptions::new(
                BBox::new(-119.9, 35.8, -119.4, 36.3).unwrap(),
                "OPERA_L3_DSWX-HLS_V1",
                Some("B01_WTR".into()),
            )
        };
        decorate_preview(&mut img, &opts);
        let png_bytes = encode_png_rgba(&img).unwrap();
        assert_eq!(&png_bytes[..8], b"\x89PNG\r\n\x1a\n");

        let decoded = decode_png_rgba(&png_bytes).unwrap();
        assert_eq!((decoded.width, decoded.height), (320, 240));

        let side = combine_side_by_side(&img, &img);
        assert_eq!(side.width, 320 * 2 + 6);

        let gif_bytes = encode_animated_gif(&[img.clone(), img], 50).unwrap();
        assert!(gif_bytes.starts_with(b"GIF89a") || gif_bytes.starts_with(b"GIF87a"));
    }

    #[test]
    fn builds_interactive_html_document_and_escapes_untrusted_strings() {
        let style = LayerStyle {
            colormap: None,
            colormap_name: Some("<b>bad</b>".into()),
            rescale: None,
        };
        let html = build_interactive_html(
            "Tulare Lake <Flood>",
            BBox::new(-119.9, 35.8, -119.4, 36.3).unwrap(),
            &[HtmlTileLayer {
                id: "dswx".into(),
                label: "</script><img src=x onerror=alert(1)>".into(),
                tile_url: "https://example.org/{z}/{x}/{y}.png".into(),
                opacity: 0.85,
            }],
            None,
            Some(&style),
        )
        .unwrap();
        assert!(html.contains("maplibregl.Map"));
        assert!(html.contains("Tulare Lake &lt;Flood&gt;"));
        assert!(!html.contains("<b>bad</b>"));
        assert!(!html.contains("</script><img"));
        assert!(html.contains("span.textContent = layer.label"));
    }

    #[test]
    fn rejects_mismatched_gif_frame_dimensions() {
        let a = RgbaImage {
            width: 4,
            height: 4,
            data: vec![0; 4 * 4 * 4],
        };
        let b = RgbaImage {
            width: 8,
            height: 4,
            data: vec![0; 8 * 4 * 4],
        };
        assert!(encode_animated_gif(&[a, b], 50).is_err());
    }
}
