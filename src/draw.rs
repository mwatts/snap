use crate::schema::{Annotation, Point};
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, Transform,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Circle,
    Rect,
    Arrow,
    Freehand,
    Text,
    Marker,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Crop {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CropDims {
    pub preview_width: f64,
    pub preview_height: f64,
    pub native_width: f64,
    pub native_height: f64,
}

/// Live session shapes. Stroke width is kept for composite (Task 10); sidecar omits it.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionItem {
    Circle {
        center: [f64; 2],
        radius: [f64; 2],
        color: String,
        stroke_width: f32,
    },
    Rect {
        position: [f64; 2],
        size: [f64; 2],
        color: String,
        stroke_width: f32,
    },
    Arrow {
        from: [f64; 2],
        to: [f64; 2],
        color: String,
        stroke_width: f32,
    },
    Freehand {
        points: Vec<Point>,
        color: String,
        stroke_width: f32,
    },
    Text {
        position: [f64; 2],
        content: String,
        color: String,
    },
    Marker {
        position: [f64; 2],
        number: u32,
        color: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub tool: Tool,
    pub color: String,
    pub width: f32,
    pub dim: bool,
    pub marker_counter: u32,
    pub items: Vec<SessionItem>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            tool: Tool::Circle,
            color: "#FF3B30".into(),
            width: 4.0,
            dim: false,
            marker_counter: 1,
            items: Vec::new(),
        }
    }

    pub fn undo(&mut self) {
        if let Some(item) = self.items.pop() {
            if matches!(item, SessionItem::Marker { .. }) {
                self.marker_counter = self.marker_counter.saturating_sub(1).max(1);
            }
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.marker_counter = 1;
    }

    pub fn add_marker(&mut self, x: f64, y: f64) {
        self.items.push(SessionItem::Marker {
            position: [x, y],
            number: self.marker_counter,
            color: self.color.clone(),
        });
        self.marker_counter += 1;
    }

    pub fn to_annotations(&self) -> Vec<Annotation> {
        self.items
            .iter()
            .map(|item| match item {
                SessionItem::Circle {
                    center,
                    radius,
                    color,
                    ..
                } => Annotation::Circle {
                    center: *center,
                    radius: *radius,
                    color: color.clone(),
                    label: None,
                },
                SessionItem::Rect {
                    position,
                    size,
                    color,
                    ..
                } => Annotation::Rect {
                    position: *position,
                    size: *size,
                    color: color.clone(),
                    label: None,
                },
                SessionItem::Arrow {
                    from, to, color, ..
                } => Annotation::Arrow {
                    from: *from,
                    to: *to,
                    color: color.clone(),
                    label: None,
                },
                SessionItem::Freehand { points, color, .. } => Annotation::Freehand {
                    points: points.clone(),
                    color: color.clone(),
                    label: None,
                },
                SessionItem::Text {
                    position,
                    content,
                    color,
                } => Annotation::Text {
                    position: *position,
                    content: content.clone(),
                    color: color.clone(),
                },
                SessionItem::Marker {
                    position,
                    number,
                    color,
                } => Annotation::Marker {
                    position: *position,
                    number: *number,
                    color: color.clone(),
                },
            })
            .collect()
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

fn clamp(value: f64, min: f64, max: f64) -> f64 {
    value.max(min).min(max)
}

fn round_i(value: f64) -> f64 {
    value.round() as i64 as f64
}

/// Map a preview-space crop to native capture pixels (matches `export-scale.mjs`).
pub fn map_crop_to_native(crop: &Crop, dims: &CropDims) -> Option<Crop> {
    if dims.preview_width == 0.0
        || dims.preview_height == 0.0
        || dims.native_width == 0.0
        || dims.native_height == 0.0
    {
        return None;
    }

    let sx = dims.native_width / dims.preview_width;
    let sy = dims.native_height / dims.preview_height;

    let x = clamp(round_i(crop.x * sx), 0.0, dims.native_width);
    let y = clamp(round_i(crop.y * sy), 0.0, dims.native_height);
    let w = clamp(round_i(crop.w * sx), 0.0, dims.native_width - x);
    let h = clamp(round_i(crop.h * sy), 0.0, dims.native_height - y);

    Some(Crop { x, y, w, h })
}

const MARKER_RADIUS: f64 = 16.0;
const TEXT_SIZE: f32 = 16.0;
const TEXT_STROKE_PX: f32 = 3.0;
const RECT_FILL_ALPHA: u8 = 0x1A;

#[derive(Debug, thiserror::Error)]
pub enum DrawError {
    #[error("decode: {0}")]
    Decode(String),
    #[error("encode: {0}")]
    Encode(String),
    #[error("{0}")]
    Invalid(String),
}

/// Composite capture PNG + session items at export resolution.
///
/// Stroke width comes from each `SessionItem` (sidecar JSON has no width field).
/// `crop` is already native pixels (caller mapped). `logical_w` / `logical_h`
/// are overlay coordinates; scale is `export / logical`.
pub fn composite_png(
    capture_png: &[u8],
    crop: Option<&Crop>,
    items: &[SessionItem],
    logical_w: f64,
    logical_h: f64,
    font: &[u8],
) -> Result<Vec<u8>, DrawError> {
    let decoded = image::load_from_memory(capture_png)
        .map_err(|e| DrawError::Decode(e.to_string()))?
        .to_rgba8();
    let img = match crop {
        Some(crop) => apply_native_crop(&decoded, crop)?,
        None => decoded,
    };
    let export_w = img.width();
    let export_h = img.height();
    if export_w == 0 || export_h == 0 {
        return Err(DrawError::Invalid("empty image".into()));
    }

    let sx = if logical_w == 0.0 {
        1.0
    } else {
        export_w as f64 / logical_w
    };
    let sy = if logical_h == 0.0 {
        1.0
    } else {
        export_h as f64 / logical_h
    };
    let scale = ((sx + sy) / 2.0) as f32;

    let mut pixmap = Pixmap::new(export_w, export_h)
        .ok_or_else(|| DrawError::Invalid("could not allocate pixmap".into()))?;
    copy_image_to_pixmap(&img, &mut pixmap);

    let face = if needs_font(items) {
        Some(
            fontdue::Font::from_bytes(font, fontdue::FontSettings::default())
                .map_err(|e| DrawError::Invalid(e.to_string()))?,
        )
    } else {
        None
    };

    for item in items {
        draw_item(&mut pixmap, item, sx, sy, scale, face.as_ref());
    }

    encode_png(&pixmap)
}

fn needs_font(items: &[SessionItem]) -> bool {
    items
        .iter()
        .any(|a| matches!(a, SessionItem::Text { .. } | SessionItem::Marker { .. }))
}

fn apply_native_crop(img: &image::RgbaImage, crop: &Crop) -> Result<image::RgbaImage, DrawError> {
    let img_w = img.width() as f64;
    let img_h = img.height() as f64;
    let x = clamp(crop.x, 0.0, img_w);
    let y = clamp(crop.y, 0.0, img_h);
    let w = clamp(crop.w, 0.0, img_w - x);
    let h = clamp(crop.h, 0.0, img_h - y);
    let x = round_i(x) as u32;
    let y = round_i(y) as u32;
    let w = round_i(w) as u32;
    let h = round_i(h) as u32;
    if w == 0 || h == 0 {
        return Err(DrawError::Invalid("empty crop".into()));
    }
    Ok(image::imageops::crop_imm(img, x, y, w, h).to_image())
}

fn copy_image_to_pixmap(img: &image::RgbaImage, pixmap: &mut Pixmap) {
    let src = img.as_raw();
    let dst = pixmap.data_mut();
    for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        let a = s[3] as u16;
        d[0] = ((s[0] as u16 * a) / 255) as u8;
        d[1] = ((s[1] as u16 * a) / 255) as u8;
        d[2] = ((s[2] as u16 * a) / 255) as u8;
        d[3] = s[3];
    }
}

fn encode_png(pixmap: &Pixmap) -> Result<Vec<u8>, DrawError> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, pixmap.width(), pixmap.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| DrawError::Encode(e.to_string()))?;
        writer
            .write_image_data(pixmap.data())
            .map_err(|e| DrawError::Encode(e.to_string()))?;
    }
    Ok(out)
}

fn parse_color(hex: &str) -> Color {
    parse_color_alpha(hex, 255)
}

fn parse_color_alpha(hex: &str, alpha: u8) -> Color {
    let t = hex.trim().trim_start_matches('#');
    if t.len() >= 6 {
        let r = u8::from_str_radix(&t[0..2], 16).unwrap_or(0);
        let g = u8::from_str_radix(&t[2..4], 16).unwrap_or(0);
        let b = u8::from_str_radix(&t[4..6], 16).unwrap_or(0);
        Color::from_rgba8(r, g, b, alpha)
    } else {
        Color::from_rgba8(255, 59, 48, alpha)
    }
}

fn solid_paint(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    paint
}

fn stroke_style(width: f32) -> Stroke {
    Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    }
}

fn finish_path(pb: PathBuilder) -> Option<Path> {
    pb.finish()
}

fn stroke_path(pixmap: &mut Pixmap, path: &Path, color: Color, width: f32) {
    pixmap.stroke_path(
        path,
        &solid_paint(color),
        &stroke_style(width),
        Transform::identity(),
        None,
    );
}

fn fill_path(pixmap: &mut Pixmap, path: &Path, color: Color) {
    pixmap.fill_path(
        path,
        &solid_paint(color),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn norm_rect(x: f32, y: f32, w: f32, h: f32) -> Option<Rect> {
    let (x, w) = if w < 0.0 { (x + w, -w) } else { (x, w) };
    let (y, h) = if h < 0.0 { (y + h, -h) } else { (y, h) };
    Rect::from_xywh(x, y, w, h)
}

fn draw_item(
    pixmap: &mut Pixmap,
    item: &SessionItem,
    sx: f64,
    sy: f64,
    scale: f32,
    font: Option<&fontdue::Font>,
) {
    match item {
        SessionItem::Circle {
            center,
            radius,
            color,
            stroke_width,
        } => {
            let stroke_w = *stroke_width * scale;
            let rx = radius[0].abs() * sx;
            let ry = radius[1].abs() * sy;
            let Some(rect) = Rect::from_xywh(
                (center[0] * sx - rx) as f32,
                (center[1] * sy - ry) as f32,
                (rx * 2.0) as f32,
                (ry * 2.0) as f32,
            ) else {
                return;
            };
            let mut pb = PathBuilder::new();
            pb.push_oval(rect);
            if let Some(path) = finish_path(pb) {
                stroke_path(pixmap, &path, parse_color(color), stroke_w);
            }
        }
        SessionItem::Rect {
            position,
            size,
            color,
            stroke_width,
        } => {
            let stroke_w = *stroke_width * scale;
            let Some(rect) = norm_rect(
                (position[0] * sx) as f32,
                (position[1] * sy) as f32,
                (size[0] * sx) as f32,
                (size[1] * sy) as f32,
            ) else {
                return;
            };
            let mut fill_pb = PathBuilder::new();
            fill_pb.push_rect(rect);
            if let Some(path) = finish_path(fill_pb) {
                fill_path(pixmap, &path, parse_color_alpha(color, RECT_FILL_ALPHA));
            }
            let mut stroke_pb = PathBuilder::new();
            stroke_pb.push_rect(rect);
            if let Some(path) = finish_path(stroke_pb) {
                stroke_path(pixmap, &path, parse_color(color), stroke_w);
            }
        }
        SessionItem::Arrow {
            from,
            to,
            color,
            stroke_width,
        } => {
            draw_arrow(
                pixmap,
                *from,
                *to,
                parse_color(color),
                *stroke_width * scale,
                sx,
                sy,
                scale,
            );
        }
        SessionItem::Freehand {
            points,
            color,
            stroke_width,
        } => {
            if points.len() < 2 {
                return;
            }
            let stroke_w = *stroke_width * scale;
            let mut pb = PathBuilder::new();
            pb.move_to((points[0].x * sx) as f32, (points[0].y * sy) as f32);
            if points.len() == 2 {
                pb.line_to((points[1].x * sx) as f32, (points[1].y * sy) as f32);
            } else {
                for i in 1..points.len() - 1 {
                    let mid_x = ((points[i].x + points[i + 1].x) / 2.0 * sx) as f32;
                    let mid_y = ((points[i].y + points[i + 1].y) / 2.0 * sy) as f32;
                    pb.quad_to(
                        (points[i].x * sx) as f32,
                        (points[i].y * sy) as f32,
                        mid_x,
                        mid_y,
                    );
                }
                let last = points.last().unwrap();
                pb.line_to((last.x * sx) as f32, (last.y * sy) as f32);
            }
            if let Some(path) = finish_path(pb) {
                stroke_path(pixmap, &path, parse_color(color), stroke_w);
            }
        }
        SessionItem::Text {
            position,
            content,
            color,
        } => {
            let Some(font) = font else { return };
            let x = (position[0] * sx) as f32;
            let y = (position[1] * sy) as f32;
            let px = TEXT_SIZE * scale;
            let offset = TEXT_STROKE_PX * scale;
            let dark = Color::from_rgba8(0, 0, 0, 204);
            for (dx, dy) in [
                (-offset, 0.0),
                (offset, 0.0),
                (0.0, -offset),
                (0.0, offset),
                (-offset, -offset),
                (offset, -offset),
                (-offset, offset),
                (offset, offset),
            ] {
                draw_text(pixmap, font, content, x + dx, y + dy, px, dark, false);
            }
            draw_text(pixmap, font, content, x, y, px, parse_color(color), false);
        }
        SessionItem::Marker {
            position,
            number,
            color,
        } => {
            let cx = (position[0] * sx) as f32;
            let cy = (position[1] * sy) as f32;
            let r = (MARKER_RADIUS * scale as f64) as f32;
            let mut pb = PathBuilder::new();
            pb.push_circle(cx, cy, r);
            if let Some(path) = finish_path(pb) {
                fill_path(pixmap, &path, parse_color(color));
            }
            if let Some(font) = font {
                draw_text(
                    pixmap,
                    font,
                    &number.to_string(),
                    cx,
                    cy,
                    r,
                    Color::from_rgba8(255, 255, 255, 255),
                    true,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_arrow(
    pixmap: &mut Pixmap,
    from: [f64; 2],
    to: [f64; 2],
    color: Color,
    stroke_w: f32,
    sx: f64,
    sy: f64,
    scale: f32,
) {
    let x1 = (from[0] * sx) as f32;
    let y1 = (from[1] * sy) as f32;
    let x2 = (to[0] * sx) as f32;
    let y2 = (to[1] * sy) as f32;
    let mut shaft = PathBuilder::new();
    shaft.move_to(x1, y1);
    shaft.line_to(x2, y2);
    if let Some(path) = finish_path(shaft) {
        stroke_path(pixmap, &path, color, stroke_w);
    }

    let head_len = 12.0 * scale + stroke_w;
    let angle = (y2 - y1).atan2(x2 - x1);
    let mut head = PathBuilder::new();
    head.move_to(x2, y2);
    head.line_to(
        x2 - head_len * (angle - std::f32::consts::PI / 6.0).cos(),
        y2 - head_len * (angle - std::f32::consts::PI / 6.0).sin(),
    );
    head.line_to(
        x2 - head_len * (angle + std::f32::consts::PI / 6.0).cos(),
        y2 - head_len * (angle + std::f32::consts::PI / 6.0).sin(),
    );
    head.close();
    if let Some(path) = finish_path(head) {
        fill_path(pixmap, &path, color);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    pixmap: &mut Pixmap,
    font: &fontdue::Font,
    text: &str,
    x: f32,
    y: f32,
    px: f32,
    color: Color,
    center: bool,
) {
    if px <= 0.0 || text.is_empty() {
        return;
    }
    let mut glyphs = Vec::new();
    let mut width = 0.0f32;
    for ch in text.chars() {
        let (metrics, bitmap) = font.rasterize(ch, px);
        width += metrics.advance_width;
        glyphs.push((metrics, bitmap));
    }
    let mut pen_x = if center { x - width / 2.0 } else { x };
    let baseline = if center {
        match font.horizontal_line_metrics(px) {
            Some(m) => y + (m.ascent + m.descent) / 2.0 - m.descent,
            None => y,
        }
    } else {
        y
    };
    for (metrics, bitmap) in glyphs {
        let gx = (pen_x + metrics.xmin as f32).round() as i32;
        let gy = (baseline - metrics.ymin as f32 - metrics.height as f32).round() as i32;
        blit_glyph(
            pixmap,
            gx,
            gy,
            metrics.width,
            metrics.height,
            &bitmap,
            color,
        );
        pen_x += metrics.advance_width;
    }
}

fn blit_glyph(
    pixmap: &mut Pixmap,
    origin_x: i32,
    origin_y: i32,
    width: usize,
    height: usize,
    bitmap: &[u8],
    color: Color,
) {
    let cr = color.red();
    let cg = color.green();
    let cb = color.blue();
    let ca = color.alpha();
    let dst_w = pixmap.width() as i32;
    let dst_h = pixmap.height() as i32;
    let data = pixmap.data_mut();
    for row in 0..height {
        let py = origin_y + row as i32;
        if py < 0 || py >= dst_h {
            continue;
        }
        for col in 0..width {
            let px = origin_x + col as i32;
            if px < 0 || px >= dst_w {
                continue;
            }
            let cov = bitmap[row * width + col] as f32 / 255.0;
            if cov <= 0.0 {
                continue;
            }
            let src_a = ca * cov;
            if src_a <= 0.0 {
                continue;
            }
            let i = ((py * dst_w + px) * 4) as usize;
            let dst_a = data[i + 3] as f32 / 255.0;
            let inv = 1.0 - src_a;
            let out_a = src_a + dst_a * inv;
            let sr = cr * src_a;
            let sg = cg * src_a;
            let sb = cb * src_a;
            let dr = data[i] as f32 / 255.0;
            let dg = data[i + 1] as f32 / 255.0;
            let db = data[i + 2] as f32 / 255.0;
            if out_a > 0.0 {
                data[i] = ((sr + dr * inv) * 255.0).round().clamp(0.0, 255.0) as u8;
                data[i + 1] = ((sg + dg * inv) * 255.0).round().clamp(0.0, 255.0) as u8;
                data[i + 2] = ((sb + db * inv) * 255.0).round().clamp(0.0, 255.0) as u8;
                data[i + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Keep first and last; keep a mid point when hypot to last kept ≥ tolerance.
pub fn simplify_points(points: &[Point], tolerance: f64) -> Vec<Point> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut result = vec![points[0].clone()];
    for p in &points[1..points.len() - 1] {
        let prev = result.last().unwrap();
        let dist = (p.x - prev.x).hypot(p.y - prev.y);
        if dist >= tolerance {
            result.push(p.clone());
        }
    }
    result.push(points[points.len() - 1].clone());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Point;

    #[test]
    fn map_crop_scales_preview_to_native() {
        let mapped = map_crop_to_native(
            &Crop {
                x: 100.0,
                y: 50.0,
                w: 400.0,
                h: 200.0,
            },
            &CropDims {
                preview_width: 1920.0,
                preview_height: 1080.0,
                native_width: 3840.0,
                native_height: 2160.0,
            },
        )
        .unwrap();
        assert_eq!(
            mapped,
            Crop {
                x: 200.0,
                y: 100.0,
                w: 800.0,
                h: 400.0
            }
        );
    }

    #[test]
    fn map_crop_clamps_to_native_bounds() {
        let mapped = map_crop_to_native(
            &Crop {
                x: -10.0,
                y: 1000.0,
                w: 2000.0,
                h: 300.0,
            },
            &CropDims {
                preview_width: 1000.0,
                preview_height: 1000.0,
                native_width: 2000.0,
                native_height: 2000.0,
            },
        )
        .unwrap();
        assert_eq!(
            mapped,
            Crop {
                x: 0.0,
                y: 2000.0,
                w: 2000.0,
                h: 0.0
            }
        );
    }

    #[test]
    fn simplify_keeps_ends_and_drops_dense_mids() {
        let pts = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 0.5, y: 0.0 },
            Point { x: 3.0, y: 0.0 },
        ];
        let out = simplify_points(&pts, 2.0);
        assert_eq!(out.first().unwrap().x, 0.0);
        assert_eq!(out.last().unwrap().x, 3.0);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn undo_marker_decrements_counter() {
        let mut s = Session::new();
        s.add_marker(1.0, 2.0);
        s.add_marker(3.0, 4.0);
        assert_eq!(s.marker_counter, 3);
        s.undo();
        assert_eq!(s.marker_counter, 2);
        assert_eq!(s.to_annotations().len(), 1);
        s.clear();
        assert!(s.to_annotations().is_empty());
        assert_eq!(s.marker_counter, 1);
    }

    #[test]
    fn composite_writes_png_signature() {
        // 4x4 red PNG generated in the test
        let mut img = image::RgbaImage::new(4, 4);
        for p in img.pixels_mut() {
            *p = image::Rgba([255, 0, 0, 255]);
        }
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        let font = include_bytes!("../assets/LiberationSans-Bold.ttf");
        let out = composite_png(
            &buf,
            None,
            &[SessionItem::Circle {
                center: [2.0, 2.0],
                radius: [1.0, 1.0],
                color: "#000000".into(),
                stroke_width: 4.0,
            }],
            4.0,
            4.0,
            font,
        )
        .unwrap();
        assert_eq!(&out[..8], b"\x89PNG\r\n\x1a\n");
        assert!(out.len() > buf.len() / 2);
    }

    fn solid_png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut img = image::RgbaImage::new(w, h);
        for p in img.pixels_mut() {
            *p = image::Rgba(rgba);
        }
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        buf
    }

    #[test]
    fn composite_honors_stroke_width() {
        let buf = solid_png(48, 48, [255, 255, 255, 255]);
        let font = include_bytes!("../assets/LiberationSans-Bold.ttf");
        let item = |width: f32| SessionItem::Rect {
            position: [8.0, 8.0],
            size: [32.0, 32.0],
            color: "#000000".into(),
            stroke_width: width,
        };
        let thin = composite_png(&buf, None, &[item(2.0)], 48.0, 48.0, font).unwrap();
        let thick = composite_png(&buf, None, &[item(6.0)], 48.0, 48.0, font).unwrap();
        let a = image::load_from_memory(&thin).unwrap().to_rgba8();
        let b = image::load_from_memory(&thick).unwrap().to_rgba8();
        let different = a.pixels().zip(b.pixels()).filter(|(p, q)| p != q).count();
        assert!(
            different > 0,
            "2px and 6px strokes must produce different pixels"
        );
    }
}
