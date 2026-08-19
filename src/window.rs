use crate::schema::WindowSource;
use std::process::Command;

pub trait ActiveWindow {
    fn read(&self) -> WindowSource;
}

pub struct CliWindow;

impl ActiveWindow for CliWindow {
    fn read(&self) -> WindowSource {
        #[cfg(target_os = "macos")]
        {
            macos_window()
        }
        #[cfg(target_os = "linux")]
        {
            linux_window()
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            WindowSource::default()
        }
    }
}

#[cfg(target_os = "macos")]
fn osascript(script: &str) -> Option<String> {
    Command::new("osascript")
        .args(["-e", script])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.is_empty() { None } else { Some(s) }
            } else {
                None
            }
        })
}

#[cfg(target_os = "macos")]
fn macos_window() -> WindowSource {
    let app_name = osascript(
        "tell application \"System Events\" to get name of first process where frontmost is true",
    );

    let window_title = app_name.as_deref().and_then(|name| {
        osascript(&format!(
            "tell application \"{}\" to get name of front window",
            name
        ))
    });

    let is_browser = app_name
        .as_deref()
        .map(|n| {
            let lower = n.to_lowercase();
            lower.contains("safari")
                || lower.contains("firefox")
                || lower.contains("chrome")
                || lower.contains("brave")
                || lower.contains("webkit")
        })
        .unwrap_or(false);

    let url = if is_browser {
        app_name.as_deref().and_then(|name| {
            let lower = name.to_lowercase();
            let script = if lower.contains("safari") {
                "tell application \"Safari\" to get URL of current tab of front window".to_string()
            } else {
                format!(
                    "tell application \"{}\" to get URL of active tab of front window",
                    name
                )
            };
            osascript(&script)
        })
    } else {
        None
    };

    WindowSource {
        window_title: window_title.or_else(|| app_name.clone()),
        url,
        window_class: app_name,
        pid: None,
        display: "primary".into(),
        resolution: [0, 0],
    }
}

#[cfg(target_os = "linux")]
fn linux_window() -> WindowSource {
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        return WindowSource::default();
    }

    let title = run_xdotool(&["getactivewindow", "getwindowname"]);
    let class = run_xdotool(&["getactivewindow", "getwindowclassname"]);
    let pid_str = run_xdotool(&["getactivewindow", "getwindowpid"]);
    let pid = pid_str.as_ref().and_then(|s| s.trim().parse::<u32>().ok());

    let url = title.as_ref().and_then(|t| {
        let is_browser = class
            .as_ref()
            .map(|c| {
                let lower = c.to_lowercase();
                lower.contains("brave")
                    || lower.contains("firefox")
                    || lower.contains("chrom")
                    || lower.contains("webkit")
            })
            .unwrap_or(false);
        if is_browser { Some(t.clone()) } else { None }
    });

    WindowSource {
        window_title: title,
        url,
        window_class: class,
        pid,
        display: "primary".into(),
        resolution: [0, 0],
    }
}

#[cfg(target_os = "linux")]
fn run_xdotool(args: &[&str]) -> Option<String> {
    Command::new("xdotool")
        .args(args)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_trait_fake_reads_source() {
        struct Fake;
        impl ActiveWindow for Fake {
            fn read(&self) -> WindowSource {
                WindowSource::default()
            }
        }
        let src = Fake.read();
        assert_eq!(src.display, "primary");
        assert_eq!(src.resolution, [0, 0]);
    }
}
