pub mod capture;
pub mod cli;
pub mod daemon;
pub mod doctor;
pub mod draw;
pub mod inbox;
pub mod ipc;
pub mod logutil;
pub mod overlay;
pub mod paths;
pub mod project;
pub mod schema;
pub mod state;
pub mod window;

#[cfg(test)]
mod tests {
    #[test]
    fn macos_info_plist_keeps_usage_strings() {
        let plist = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Info.plist"));
        assert!(plist.contains("<key>NSAccessibilityUsageDescription</key>"));
        assert!(plist.contains("Snap uses Accessibility to register a global keyboard shortcut."));
        assert!(plist.contains("<key>NSAppleEventsUsageDescription</key>"));
        assert!(plist.contains("Snap uses Apple Events to read the active window title and URL."));
    }
}
