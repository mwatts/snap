use crate::schema::{Annotation, Point};

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
                    from,
                    to,
                    color,
                    ..
                } => Annotation::Arrow {
                    from: *from,
                    to: *to,
                    color: color.clone(),
                    label: None,
                },
                SessionItem::Freehand {
                    points, color, ..
                } => Annotation::Freehand {
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
}
