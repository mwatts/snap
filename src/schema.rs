use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowSource {
    pub window_title: Option<String>,
    pub url: Option<String>,
    pub window_class: Option<String>,
    pub pid: Option<u32>,
    pub display: String,
    pub resolution: [u32; 2],
}

impl Default for WindowSource {
    fn default() -> Self {
        Self {
            window_title: None,
            url: None,
            window_class: None,
            pid: None,
            display: "primary".into(),
            resolution: [0, 0],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Annotation {
    #[serde(rename = "circle")]
    Circle {
        center: [f64; 2],
        radius: [f64; 2],
        color: String,
        label: Option<String>,
    },
    #[serde(rename = "rect")]
    Rect {
        position: [f64; 2],
        size: [f64; 2],
        color: String,
        label: Option<String>,
    },
    #[serde(rename = "arrow")]
    Arrow {
        from: [f64; 2],
        to: [f64; 2],
        color: String,
        label: Option<String>,
    },
    #[serde(rename = "freehand")]
    Freehand {
        points: Vec<Point>,
        color: String,
        label: Option<String>,
    },
    #[serde(rename = "text")]
    Text {
        position: [f64; 2],
        content: String,
        color: String,
    },
    #[serde(rename = "marker")]
    Marker {
        position: [f64; 2],
        number: u32,
        color: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sidecar {
    pub timestamp: String,
    pub source: WindowSource,
    pub annotations: Vec<Annotation>,
    pub image_filename: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnotationRecord {
    pub filename: String,
    pub image_path: Option<String>,
    pub timestamp: String,
    pub source: WindowSource,
    pub annotations: Vec<Annotation>,
    pub image_filename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

impl AnnotationRecord {
    pub fn from_sidecar(sidecar: Sidecar, image_path: Option<String>) -> Self {
        Self {
            filename: sidecar.image_filename.trim_end_matches(".png").to_string(),
            image_path,
            timestamp: sidecar.timestamp,
            source: sidecar.source,
            annotations: sidecar.annotations,
            image_filename: sidecar.image_filename,
            warning: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub has_new: bool,
    pub new_count: usize,
    pub new_annotations: Vec<String>,
    pub total_in_inbox: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_round_trip_matches_fork_shape() {
        // r## so hex colors like #FF3B30 do not terminate the raw string
        let json = r##"{
            "timestamp": "2026-08-18T14:20:15.897Z",
            "source": {
                "window_title": "localhost:3000",
                "url": null,
                "window_class": "Brave-browser",
                "pid": 12345,
                "display": "primary",
                "resolution": [1920, 1080]
            },
            "annotations": [
                {
                    "type": "circle",
                    "center": [340.0, 220.0],
                    "radius": [45.0, 45.0],
                    "color": "#FF3B30",
                    "label": null
                },
                {
                    "type": "freehand",
                    "points": [{"x": 1.0, "y": 2.0}, {"x": 3.0, "y": 4.0}],
                    "color": "#007AFF",
                    "label": null
                },
                {
                    "type": "text",
                    "position": [10.0, 20.0],
                    "content": "fix this",
                    "color": "#FF3B30"
                },
                {
                    "type": "marker",
                    "position": [5.0, 6.0],
                    "number": 1,
                    "color": "#34C759"
                }
            ],
            "image_filename": "snap-20260818-142015-897.png"
        }"##;
        let side: Sidecar = serde_json::from_str(json).unwrap();
        assert_eq!(side.annotations.len(), 4);
        let out = serde_json::to_value(&side).unwrap();
        assert_eq!(out["annotations"][0]["type"], "circle");
        assert_eq!(out["annotations"][1]["points"][0]["x"], 1.0);
    }
}
