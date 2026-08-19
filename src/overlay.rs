use crate::capture::{CaptureError, CliCapture, ScreenCapture, capture_temp_path};
use crate::daemon::project_name;
use crate::draw::{
    Crop, CropDims, Session, SessionItem, Tool, composite_png, map_crop_to_native, simplify_points,
};
use crate::inbox::Inbox;
use crate::logutil::log_event;
use crate::paths::Paths;
use crate::project::set_active;
use crate::schema::{Point as InkPoint, Sidecar, WindowSource};
use crate::state::{GlobalState, ProjectRegistry};
use crate::window::{ActiveWindow, CliWindow};
use chrono::{DateTime, Utc};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gpui::{
    App, Bounds, Context, Entity, FocusHandle, Focusable, KeyBinding, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, QuitMode, Render,
    SharedString, WeakEntity, Window, WindowBounds, WindowHandle, WindowKind, WindowOptions,
    actions, canvas, div, img, point, prelude::*, px, rgb, size,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::Duration;

const COLORS: [&str; 6] = [
    "#FF3B30", "#007AFF", "#34C759", "#FFCC00", "#FFFFFF", "#000000",
];
const WIDTHS: [f32; 3] = [2.0, 4.0, 6.0];

pub enum IpcEvent {
    Capture,
    Use(PathBuf),
    Shutdown,
}

/// Letterboxed contain-fit of a capture (`iw`×`ih`) inside a viewport (`vw`×`vh`).
/// Matches `ObjectFit::Contain` and the fork's `computeBgLayout` (without the dpr cap).
#[derive(Debug, Clone, PartialEq)]
pub struct ContainFit {
    pub offset_x: f64,
    pub offset_y: f64,
    pub draw_w: f64,
    pub draw_h: f64,
    pub image_w: f64,
    pub image_h: f64,
}

impl ContainFit {
    pub fn new(vw: f64, vh: f64, iw: f64, ih: f64) -> Self {
        if vw <= 0.0 || vh <= 0.0 || iw <= 0.0 || ih <= 0.0 {
            return Self {
                offset_x: 0.0,
                offset_y: 0.0,
                draw_w: 0.0,
                draw_h: 0.0,
                image_w: iw,
                image_h: ih,
            };
        }
        let scale = (vw / iw).min(vh / ih);
        let draw_w = iw * scale;
        let draw_h = ih * scale;
        Self {
            offset_x: ((vw - draw_w) / 2.0).round(),
            offset_y: ((vh - draw_h) / 2.0).round(),
            draw_w,
            draw_h,
            image_w: iw,
            image_h: ih,
        }
    }

    pub fn viewport_to_preview(&self, x: f64, y: f64) -> [f64; 2] {
        [x - self.offset_x, y - self.offset_y]
    }

    pub fn viewport_crop_to_preview(&self, crop: &Crop) -> Crop {
        let [x, y] = self.viewport_to_preview(crop.x, crop.y);
        Crop {
            x,
            y,
            w: crop.w,
            h: crop.h,
        }
    }

    pub fn preview_to_native(&self, x: f64, y: f64) -> [f64; 2] {
        if self.draw_w == 0.0 || self.draw_h == 0.0 {
            return [0.0, 0.0];
        }
        [
            x * self.image_w / self.draw_w,
            y * self.image_h / self.draw_h,
        ]
    }

    pub fn viewport_to_native(&self, x: f64, y: f64) -> [f64; 2] {
        let p = self.viewport_to_preview(x, y);
        self.preview_to_native(p[0], p[1])
    }

    pub fn crop_dims(&self) -> CropDims {
        CropDims {
            preview_width: self.draw_w,
            preview_height: self.draw_h,
            native_width: self.image_w,
            native_height: self.image_h,
        }
    }
}

fn map_session_coords(session: &Session, f: impl Fn(f64, f64) -> [f64; 2]) -> Session {
    let mut session = session.clone();
    for item in &mut session.items {
        match item {
            SessionItem::Circle { center, .. } => *center = f(center[0], center[1]),
            SessionItem::Rect { position, .. } => *position = f(position[0], position[1]),
            SessionItem::Arrow { from, to, .. } => {
                *from = f(from[0], from[1]);
                *to = f(to[0], to[1]);
            }
            SessionItem::Freehand { points, .. } => {
                for p in points {
                    let n = f(p.x, p.y);
                    p.x = n[0];
                    p.y = n[1];
                }
            }
            SessionItem::Text { position, .. } | SessionItem::Marker { position, .. } => {
                *position = f(position[0], position[1]);
            }
        }
    }
    session
}

fn crop_png_bytes(png: &[u8], crop: &Crop) -> Result<Vec<u8>, anyhow::Error> {
    let img = image::load_from_memory(png)?.to_rgba8();
    let img_w = img.width() as f64;
    let img_h = img.height() as f64;
    let x = crop.x.max(0.0).min(img_w);
    let y = crop.y.max(0.0).min(img_h);
    let w = crop.w.max(0.0).min(img_w - x);
    let h = crop.h.max(0.0).min(img_h - y);
    let x = x.round() as u32;
    let y = y.round() as u32;
    let w = w.round() as u32;
    let h = h.round() as u32;
    if w == 0 || h == 0 {
        anyhow::bail!("empty crop");
    }
    let cropped = image::imageops::crop_imm(&img, x, y, w, h).to_image();
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgba8(cropped)
        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)?;
    Ok(buf)
}

pub fn save_session(
    repo: &Path,
    capture_png: &[u8],
    crop: Option<&Crop>,
    mut source: WindowSource,
    session: &Session,
    logical_w: f64,
    logical_h: f64,
    now: DateTime<Utc>,
) -> Result<PathBuf, anyhow::Error> {
    let font = include_bytes!("../assets/LiberationSans-Bold.ttf");
    let dynimg = image::load_from_memory(capture_png)?;
    if source.resolution == [0, 0] {
        source.resolution = [dynimg.width(), dynimg.height()];
    }
    let native_crop = crop.and_then(|c| {
        map_crop_to_native(
            c,
            &CropDims {
                preview_width: logical_w,
                preview_height: logical_h,
                native_width: dynimg.width() as f64,
                native_height: dynimg.height() as f64,
            },
        )
    });
    let png = composite_png(
        capture_png,
        native_crop.as_ref(),
        &session.to_annotations(),
        logical_w,
        logical_h,
        font,
    )?;
    let mut sidecar = Sidecar {
        timestamp: now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        source,
        annotations: session.to_annotations(),
        image_filename: String::new(),
    };
    Ok(Inbox::new(repo.join(".snap")).save(&mut sidecar, &png, now)?)
}

actions!(
    overlay,
    [
        SaveOverlay,
        CancelOverlay,
        UndoStroke,
        ToggleDim,
        ToolCircle,
        ToolRect,
        ToolArrow,
        ToolFreehand,
        ToolText,
        ToolMarker,
    ]
);

pub fn run_app(paths: Paths, ipc_rx: Receiver<IpcEvent>) {
    gpui_platform::application()
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx: &mut App| {
            set_accessory_policy();
            cx.set_app_identity("dev.snap.snap", "snap");
            cx.bind_keys([
                KeyBinding::new("enter", SaveOverlay, Some("Overlay")),
                KeyBinding::new("escape", CancelOverlay, Some("Overlay")),
                KeyBinding::new("c", ToolCircle, Some("Overlay")),
                KeyBinding::new("r", ToolRect, Some("Overlay")),
                KeyBinding::new("a", ToolArrow, Some("Overlay")),
                KeyBinding::new("f", ToolFreehand, Some("Overlay")),
                KeyBinding::new("t", ToolText, Some("Overlay")),
                KeyBinding::new("n", ToolMarker, Some("Overlay")),
                KeyBinding::new("d", ToggleDim, Some("Overlay")),
                KeyBinding::new("cmd-z", UndoStroke, Some("Overlay")),
                KeyBinding::new("ctrl-z", UndoStroke, Some("Overlay")),
            ]);

            let runtime = cx.new(|_| Runtime::new(paths, ipc_rx));
            register_hotkey(&runtime, cx);
            runtime.update(cx, |rt, cx| rt.start_polling(cx));
        });
}

fn set_accessory_policy() {
    #[cfg(target_os = "macos")]
    {
        use objc::{class, msg_send, sel, sel_impl};
        unsafe {
            let app: *mut objc::runtime::Object =
                msg_send![class!(NSApplication), sharedApplication];
            let _: isize = msg_send![app, setActivationPolicy: 1isize];
        }
    }
}

fn register_hotkey(runtime: &Entity<Runtime>, cx: &mut App) {
    let manager = match GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(e) => {
            runtime.update(cx, |rt, _| {
                log_event(&rt.paths, &format!("hotkey manager: {e}"));
            });
            return;
        }
    };
    let hotkey = HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyS);
    if let Err(e) = manager.register(hotkey) {
        runtime.update(cx, |rt, _| {
            log_event(&rt.paths, &format!("hotkey register: {e}"));
        });
    }
    runtime.update(cx, |rt, _| {
        rt.hotkey = Some(manager);
    });
}

struct Runtime {
    paths: Paths,
    active: Option<PathBuf>,
    overlay: Option<WindowHandle<OverlayView>>,
    error: Option<WindowHandle<ErrorBanner>>,
    hotkey: Option<GlobalHotKeyManager>,
    ipc_rx: Receiver<IpcEvent>,
    capturing: bool,
}

impl Runtime {
    fn new(paths: Paths, ipc_rx: Receiver<IpcEvent>) -> Self {
        let active = GlobalState::load(&paths.state)
            .unwrap_or_default()
            .active_project;
        Self {
            paths,
            active,
            overlay: None,
            error: None,
            hotkey: None,
            ipc_rx,
            capturing: false,
        }
    }

    fn start_polling(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(20))
                    .await;
                let evs = this.update(cx, |rt, _| {
                    let mut out = Vec::new();
                    while let Ok(ev) = rt.ipc_rx.try_recv() {
                        out.push(ev);
                    }
                    while let Ok(ev) = GlobalHotKeyEvent::receiver().try_recv() {
                        if ev.state == HotKeyState::Pressed {
                            out.push(IpcEvent::Capture);
                        }
                    }
                    out
                });
                let Ok(evs) = evs else { break };
                for ev in evs {
                    match ev {
                        IpcEvent::Shutdown => {
                            let _ = this.update(cx, |_, cx| cx.quit());
                            return;
                        }
                        other => {
                            let _ = this.update(cx, |rt, cx| rt.handle_ipc(other, cx));
                        }
                    }
                }
            }
        })
        .detach();
    }

    fn handle_ipc(&mut self, ev: IpcEvent, cx: &mut Context<Self>) {
        match ev {
            IpcEvent::Capture => self.begin_capture(cx),
            IpcEvent::Use(path) => self.active = Some(path),
            IpcEvent::Shutdown => cx.quit(),
        }
    }

    fn begin_capture(&mut self, cx: &mut Context<Self>) {
        if self.overlay.is_some() || self.capturing {
            return;
        }
        let Some(repo) = self.active.clone() else {
            log_event(&self.paths, "capture: no last-active project");
            return;
        };
        if !repo.join(".snap").join("config.toml").is_file() {
            log_event(
                &self.paths,
                &format!(
                    "capture: last-active has no .snap/config.toml ({})",
                    repo.display()
                ),
            );
            return;
        }

        let source = CliWindow.read();
        let dest = capture_temp_path();
        self.capturing = true;
        let result = CliCapture.capture_to(&dest);
        self.capturing = false;
        match result {
            Ok(()) => match std::fs::read(&dest) {
                Ok(bytes) => self.open_overlay(repo, bytes, dest, source, cx),
                Err(e) => self.show_error(format!("could not read capture: {e}"), cx),
            },
            Err(CaptureError::Cancelled) => {
                log_event(&self.paths, "capture cancelled");
            }
            Err(CaptureError::Failed(msg)) => {
                log_event(&self.paths, &format!("capture failed: {msg}"));
                self.show_error(msg, cx);
            }
        }
    }

    fn open_overlay(
        &mut self,
        repo: PathBuf,
        capture_png: Vec<u8>,
        capture_path: PathBuf,
        source: WindowSource,
        cx: &mut Context<Self>,
    ) {
        cx.activate(true);
        let paths = self.paths.clone();
        let project = project_name(&repo);
        let runtime = cx.entity().downgrade();
        let selecting = cfg!(not(target_os = "macos"));
        let opened = cx.open_window(overlay_window_options(), move |window, cx| {
            let view = cx.new(|cx| {
                OverlayView::new(
                    paths,
                    repo,
                    project,
                    capture_png,
                    capture_path,
                    source,
                    selecting,
                    runtime,
                    window,
                    cx,
                )
            });
            let focus = view.read(cx).focus.clone();
            focus.focus(window, cx);
            view
        });
        match opened {
            Ok(handle) => self.overlay = Some(handle),
            Err(e) => {
                log_event(&self.paths, &format!("open overlay: {e}"));
                self.show_error(format!("could not open overlay: {e}"), cx);
            }
        }
    }

    fn show_error(&mut self, msg: String, cx: &mut Context<Self>) {
        cx.activate(true);
        let opts = error_window_options(cx);
        let opened = cx.open_window(opts, |_, cx| {
            cx.new(|_| ErrorBanner {
                message: msg.into(),
            })
        });
        match opened {
            Ok(handle) => {
                self.error = Some(handle);
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_secs(3)).await;
                    let _ = this.update(cx, |rt, cx| {
                        if let Some(handle) = rt.error.take() {
                            let _ = handle.update(cx, |_, window, _| window.remove_window());
                        }
                        if rt.overlay.is_none() {
                            cx.hide();
                        }
                    });
                })
                .detach();
            }
            Err(e) => log_event(&self.paths, &format!("error overlay: {e}")),
        }
    }

    fn overlay_closed(&mut self, cx: &mut Context<Self>) {
        self.overlay = None;
        cx.hide();
    }
}

fn overlay_window_options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Fullscreen(Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(1280.), px(800.)),
        })),
        titlebar: None,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        focus: true,
        show: true,
        ..Default::default()
    }
}

fn error_window_options(cx: &mut App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(440.), px(120.)),
            cx,
        ))),
        titlebar: None,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        focus: true,
        show: true,
        ..Default::default()
    }
}

struct ErrorBanner {
    message: SharedString,
}

impl Render for ErrorBanner {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .bg(rgb(0x111111))
            .text_color(rgb(0xff3b30))
            .child(self.message.clone())
            .child(
                div()
                    .mt_2()
                    .text_color(rgb(0x888888))
                    .child("Closing in 3 seconds..."),
            )
    }
}

#[derive(Clone, PartialEq)]
enum Gesture {
    None,
    Select {
        start: [f64; 2],
        end: [f64; 2],
    },
    Shape {
        start: [f64; 2],
        end: [f64; 2],
        shift: bool,
    },
    Freehand {
        points: Vec<InkPoint>,
    },
}

struct OverlayView {
    paths: Paths,
    repo: PathBuf,
    project_name: String,
    capture_png: Vec<u8>,
    capture_path: PathBuf,
    source: WindowSource,
    session: Session,
    gesture: Gesture,
    selecting: bool,
    crop: Option<Crop>,
    text_pos: Option<[f64; 2]>,
    text_buf: String,
    banner: Option<String>,
    toolbar_pos: Option<Point<Pixels>>,
    toolbar_drag: Option<Point<Pixels>>,
    logical: [f64; 2],
    image_w: f64,
    image_h: f64,
    original_layout: Option<ContainFit>,
    focus: FocusHandle,
    runtime: WeakEntity<Runtime>,
}

impl OverlayView {
    fn new(
        paths: Paths,
        repo: PathBuf,
        project_name: String,
        capture_png: Vec<u8>,
        capture_path: PathBuf,
        source: WindowSource,
        selecting: bool,
        runtime: WeakEntity<Runtime>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let size = window.viewport_size();
        let (image_w, image_h) = image::load_from_memory(&capture_png)
            .map(|im| (im.width() as f64, im.height() as f64))
            .unwrap_or((1.0, 1.0));
        Self {
            paths,
            repo,
            project_name,
            capture_png,
            capture_path,
            source,
            session: Session::new(),
            gesture: Gesture::None,
            selecting,
            crop: None,
            text_pos: None,
            text_buf: String::new(),
            banner: None,
            toolbar_pos: None,
            toolbar_drag: None,
            logical: [f32::from(size.width) as f64, f32::from(size.height) as f64],
            image_w,
            image_h,
            original_layout: None,
            focus: cx.focus_handle(),
            runtime,
        }
    }

    fn contain_layout(&self) -> ContainFit {
        ContainFit::new(self.logical[0], self.logical[1], self.image_w, self.image_h)
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(rt) = self.runtime.upgrade() {
            rt.update(cx, |rt, cx| rt.overlay_closed(cx));
        }
        window.remove_window();
    }

    fn prepare_save(&self) -> Result<(Vec<u8>, Session, f64, f64, WindowSource), anyhow::Error> {
        let mut source = self.source.clone();
        source.resolution = [self.image_w as u32, self.image_h as u32];
        if let Some(preview_crop) = &self.crop {
            let layout = self
                .original_layout
                .clone()
                .unwrap_or_else(|| self.contain_layout());
            let native = map_crop_to_native(preview_crop, &layout.crop_dims())
                .ok_or_else(|| anyhow::anyhow!("empty crop"))?;
            let png = crop_png_bytes(&self.capture_png, &native)?;
            Ok((
                png,
                self.session.clone(),
                self.logical[0],
                self.logical[1],
                source,
            ))
        } else {
            let layout = self.contain_layout();
            let session =
                map_session_coords(&self.session, |x, y| layout.viewport_to_preview(x, y));
            let logical_w = if layout.draw_w > 0.0 {
                layout.draw_w
            } else {
                self.logical[0]
            };
            let logical_h = if layout.draw_h > 0.0 {
                layout.draw_h
            } else {
                self.logical[1]
            };
            Ok((
                self.capture_png.clone(),
                session,
                logical_w,
                logical_h,
                source,
            ))
        }
    }

    fn save_and_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.text_pos.is_some() {
            self.commit_text(cx);
        }
        let prepared = self.prepare_save();
        match prepared.and_then(|(png, session, lw, lh, source)| {
            save_session(&self.repo, &png, None, source, &session, lw, lh, Utc::now())
        }) {
            Ok(path) => {
                log_event(&self.paths, &format!("saved {}", path.display()));
                self.close(window, cx);
            }
            Err(e) => {
                log_event(&self.paths, &format!("save failed: {e}"));
                self.banner = Some(e.to_string());
                cx.notify();
            }
        }
    }

    fn on_save(&mut self, _: &SaveOverlay, window: &mut Window, cx: &mut Context<Self>) {
        if self.text_pos.is_some() {
            self.commit_text(cx);
            return;
        }
        if self.selecting {
            return;
        }
        self.save_and_close(window, cx);
    }

    fn on_cancel(&mut self, _: &CancelOverlay, window: &mut Window, cx: &mut Context<Self>) {
        if self.gesture != Gesture::None {
            self.gesture = Gesture::None;
            cx.notify();
            return;
        }
        if self.text_pos.is_some() {
            self.text_pos = None;
            self.text_buf.clear();
            cx.notify();
            return;
        }
        self.close(window, cx);
    }

    fn on_undo(&mut self, _: &UndoStroke, _: &mut Window, cx: &mut Context<Self>) {
        if self.text_pos.is_some() {
            return;
        }
        self.session.undo();
        cx.notify();
    }

    fn on_dim(&mut self, _: &ToggleDim, _: &mut Window, cx: &mut Context<Self>) {
        if self.text_pos.is_some() {
            return;
        }
        self.session.dim = !self.session.dim;
        cx.notify();
    }

    fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        if self.text_pos.is_some() || self.selecting {
            return;
        }
        self.session.tool = tool;
        cx.notify();
    }

    fn commit_text(&mut self, cx: &mut Context<Self>) {
        if let Some(pos) = self.text_pos.take() {
            let content = self.text_buf.trim().to_string();
            self.text_buf.clear();
            if !content.is_empty() {
                self.session.items.push(SessionItem::Text {
                    position: [pos[0], pos[1] + 16.0],
                    content,
                    color: self.session.color.clone(),
                });
            }
        }
        cx.notify();
    }

    fn cycle_project(&mut self, cx: &mut Context<Self>) {
        let reg = ProjectRegistry::load(&self.paths.projects).unwrap_or_default();
        let live: Vec<_> = reg
            .projects
            .into_iter()
            .filter(|p| p.path.exists())
            .collect();
        if live.is_empty() {
            return;
        }
        let idx = live.iter().position(|p| p.path == self.repo).unwrap_or(0);
        let next = &live[(idx + 1) % live.len()];
        if let Err(e) = set_active(&self.paths, &next.path) {
            self.banner = Some(e.to_string());
        } else {
            self.repo = next.path.clone();
            self.project_name = next.name.clone();
        }
        cx.notify();
    }

    fn pointer(ev_pos: Point<Pixels>) -> [f64; 2] {
        [f32::from(ev_pos.x) as f64, f32::from(ev_pos.y) as f64]
    }

    fn on_down(&mut self, ev: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if ev.button != MouseButton::Left {
            return;
        }
        let p = Self::pointer(ev.position);
        if self.selecting {
            self.gesture = Gesture::Select { start: p, end: p };
            cx.notify();
            return;
        }
        if self.session.tool == Tool::Text {
            self.commit_text(cx);
            self.text_pos = Some(p);
            self.text_buf.clear();
            cx.notify();
            return;
        }
        if self.session.tool == Tool::Marker {
            self.session.add_marker(p[0], p[1]);
            cx.notify();
            return;
        }
        if self.session.tool == Tool::Freehand {
            self.gesture = Gesture::Freehand {
                points: vec![InkPoint { x: p[0], y: p[1] }],
            };
        } else {
            self.gesture = Gesture::Shape {
                start: p,
                end: p,
                shift: ev.modifiers.shift,
            };
        }
        cx.notify();
    }

    fn on_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(origin) = self.toolbar_drag {
            let next = point(ev.position.x - origin.x, ev.position.y - origin.y);
            self.toolbar_pos = Some(next);
            cx.notify();
            return;
        }
        let p = Self::pointer(ev.position);
        match &mut self.gesture {
            Gesture::None => {}
            Gesture::Select { end, .. } => {
                *end = p;
                cx.notify();
            }
            Gesture::Shape { end, shift, .. } => {
                *end = p;
                *shift = ev.modifiers.shift;
                cx.notify();
            }
            Gesture::Freehand { points } => {
                points.push(InkPoint { x: p[0], y: p[1] });
                cx.notify();
            }
        }
    }

    fn on_up(&mut self, ev: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.toolbar_drag = None;
        let p = Self::pointer(ev.position);
        match std::mem::replace(&mut self.gesture, Gesture::None) {
            Gesture::Select { start, .. } => {
                let x = start[0].min(p[0]);
                let y = start[1].min(p[1]);
                let w = (p[0] - start[0]).abs();
                let h = (p[1] - start[1]).abs();
                if w >= 10.0 && h >= 10.0 {
                    self.commit_linux_crop(Crop { x, y, w, h });
                }
            }
            Gesture::Shape { start, shift, .. } => self.commit_shape(start, p, shift),
            Gesture::Freehand { points } => {
                if points.len() > 2 {
                    let simplified = simplify_points(&points, 2.0);
                    self.session.items.push(SessionItem::Freehand {
                        points: simplified,
                        color: self.session.color.clone(),
                        stroke_width: self.session.width,
                    });
                }
            }
            Gesture::None => {}
        }
        cx.notify();
    }

    fn commit_shape(&mut self, start: [f64; 2], end: [f64; 2], shift: bool) {
        match self.session.tool {
            Tool::Circle => {
                let rx = (end[0] - start[0]).abs() / 2.0;
                let ry = if shift {
                    rx
                } else {
                    (end[1] - start[1]).abs() / 2.0
                };
                if rx > 2.0 || ry > 2.0 {
                    self.session.items.push(SessionItem::Circle {
                        center: [
                            start[0] + (end[0] - start[0]) / 2.0,
                            start[1] + (end[1] - start[1]) / 2.0,
                        ],
                        radius: [rx, ry],
                        color: self.session.color.clone(),
                        stroke_width: self.session.width,
                    });
                }
            }
            Tool::Rect => {
                let w = end[0] - start[0];
                let h = if shift { w } else { end[1] - start[1] };
                if w.abs() > 2.0 || h.abs() > 2.0 {
                    self.session.items.push(SessionItem::Rect {
                        position: start,
                        size: [w, h],
                        color: self.session.color.clone(),
                        stroke_width: self.session.width,
                    });
                }
            }
            Tool::Arrow => {
                if (end[0] - start[0]).hypot(end[1] - start[1]) > 5.0 {
                    self.session.items.push(SessionItem::Arrow {
                        from: start,
                        to: end,
                        color: self.session.color.clone(),
                        stroke_width: self.session.width,
                    });
                }
            }
            _ => {}
        }
    }

    fn commit_linux_crop(&mut self, viewport_crop: Crop) {
        let layout = self.contain_layout();
        let preview = layout.viewport_crop_to_preview(&viewport_crop);
        let Some(native) = map_crop_to_native(&preview, &layout.crop_dims()) else {
            return;
        };
        if native.w < 1.0 || native.h < 1.0 {
            return;
        }
        let Ok(bytes) = crop_png_bytes(&self.capture_png, &native) else {
            return;
        };
        let preview_path = std::env::temp_dir().join("snap-capture-preview.png");
        if std::fs::write(&preview_path, &bytes).is_err() {
            return;
        }
        self.capture_path = preview_path;
        self.crop = Some(preview);
        self.original_layout = Some(layout);
        self.selecting = false;
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.text_pos.is_none() {
            return;
        }
        match ev.keystroke.key.as_str() {
            "enter" | "escape" => return,
            "backspace" => {
                self.text_buf.pop();
                cx.notify();
                cx.stop_propagation();
                return;
            }
            _ => {}
        }
        if ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform {
            return;
        }
        if let Some(ch) = ev.keystroke.key_char.as_deref() {
            if !ch.is_empty() {
                self.text_buf.push_str(ch);
                cx.notify();
                cx.stop_propagation();
            }
        }
    }
}

impl Session {
    fn commit_preview(&mut self, start: [f64; 2], end: [f64; 2], shift: bool) {
        let color = self.color.clone();
        let width = self.width;
        match self.tool {
            Tool::Circle => {
                let rx = (end[0] - start[0]).abs() / 2.0;
                let ry = if shift {
                    rx
                } else {
                    (end[1] - start[1]).abs() / 2.0
                };
                self.items.push(SessionItem::Circle {
                    center: [
                        start[0] + (end[0] - start[0]) / 2.0,
                        start[1] + (end[1] - start[1]) / 2.0,
                    ],
                    radius: [rx, ry],
                    color,
                    stroke_width: width,
                });
            }
            Tool::Rect => {
                let w = end[0] - start[0];
                let h = if shift { w } else { end[1] - start[1] };
                self.items.push(SessionItem::Rect {
                    position: start,
                    size: [w, h],
                    color,
                    stroke_width: width,
                });
            }
            Tool::Arrow => {
                self.items.push(SessionItem::Arrow {
                    from: start,
                    to: end,
                    color,
                    stroke_width: width,
                });
            }
            _ => {}
        }
    }
}

fn hex_color(hex: &str) -> gpui::Rgba {
    let t = hex.trim().trim_start_matches('#');
    let n = u32::from_str_radix(t, 16).unwrap_or(0xff3b30);
    rgb(n)
}

fn paint_select(window: &mut Window, start: [f64; 2], end: [f64; 2]) {
    let x = start[0].min(end[0]) as f32;
    let y = start[1].min(end[1]) as f32;
    let w = (end[0] - start[0]).abs() as f32;
    let h = (end[1] - start[1]).abs() as f32;
    let mut b = PathBuilder::stroke(px(1.5));
    b.move_to(point(px(x), px(y)));
    b.line_to(point(px(x + w), px(y)));
    b.line_to(point(px(x + w), px(y + h)));
    b.line_to(point(px(x), px(y + h)));
    b.close();
    if let Ok(path) = b.build() {
        window.paint_path(path, rgb(0xffffff));
    }
}

fn paint_item(window: &mut Window, item: &SessionItem) {
    match item {
        SessionItem::Circle {
            center,
            radius,
            color,
            stroke_width,
        } => {
            if let Some(path) = ellipse_path(
                center[0] as f32,
                center[1] as f32,
                radius[0].abs() as f32,
                radius[1].abs() as f32,
                *stroke_width,
            ) {
                window.paint_path(path, hex_color(color));
            }
        }
        SessionItem::Rect {
            position,
            size,
            color,
            stroke_width,
        } => {
            let x = position[0] as f32;
            let y = position[1] as f32;
            let w = size[0] as f32;
            let h = size[1] as f32;
            let mut fill = PathBuilder::fill();
            fill.move_to(point(px(x), px(y)));
            fill.line_to(point(px(x + w), px(y)));
            fill.line_to(point(px(x + w), px(y + h)));
            fill.line_to(point(px(x), px(y + h)));
            fill.close();
            if let Ok(path) = fill.build() {
                window.paint_path(path, hex_color(color).opacity(0.1));
            }
            let mut stroke = PathBuilder::stroke(px(*stroke_width));
            stroke.move_to(point(px(x), px(y)));
            stroke.line_to(point(px(x + w), px(y)));
            stroke.line_to(point(px(x + w), px(y + h)));
            stroke.line_to(point(px(x), px(y + h)));
            stroke.close();
            if let Ok(path) = stroke.build() {
                window.paint_path(path, hex_color(color));
            }
        }
        SessionItem::Arrow {
            from,
            to,
            color,
            stroke_width,
        } => paint_arrow(window, *from, *to, color, *stroke_width),
        SessionItem::Freehand {
            points,
            color,
            stroke_width,
        } => {
            if points.len() < 2 {
                return;
            }
            let mut b = PathBuilder::stroke(px(*stroke_width));
            b.move_to(point(px(points[0].x as f32), px(points[0].y as f32)));
            for p in &points[1..] {
                b.line_to(point(px(p.x as f32), px(p.y as f32)));
            }
            if let Ok(path) = b.build() {
                window.paint_path(path, hex_color(color));
            }
        }
        SessionItem::Marker {
            position, color, ..
        } => {
            if let Some(path) = filled_circle(position[0] as f32, position[1] as f32, 16.0) {
                window.paint_path(path, hex_color(color));
            }
        }
        SessionItem::Text { .. } => {}
    }
}

fn ellipse_path(cx: f32, cy: f32, rx: f32, ry: f32, width: f32) -> Option<gpui::Path<Pixels>> {
    if rx <= 0.0 && ry <= 0.0 {
        return None;
    }
    let mut b = PathBuilder::stroke(px(width));
    const N: i32 = 48;
    for i in 0..=N {
        let t = i as f32 / N as f32 * std::f32::consts::TAU;
        let p = point(px(cx + rx * t.cos()), px(cy + ry * t.sin()));
        if i == 0 {
            b.move_to(p);
        } else {
            b.line_to(p);
        }
    }
    b.close();
    b.build().ok()
}

fn filled_circle(cx: f32, cy: f32, r: f32) -> Option<gpui::Path<Pixels>> {
    let mut b = PathBuilder::fill();
    const N: i32 = 32;
    for i in 0..=N {
        let t = i as f32 / N as f32 * std::f32::consts::TAU;
        let p = point(px(cx + r * t.cos()), px(cy + r * t.sin()));
        if i == 0 {
            b.move_to(p);
        } else {
            b.line_to(p);
        }
    }
    b.close();
    b.build().ok()
}

fn paint_arrow(window: &mut Window, from: [f64; 2], to: [f64; 2], color: &str, width: f32) {
    let x1 = from[0] as f32;
    let y1 = from[1] as f32;
    let x2 = to[0] as f32;
    let y2 = to[1] as f32;
    let mut shaft = PathBuilder::stroke(px(width));
    shaft.move_to(point(px(x1), px(y1)));
    shaft.line_to(point(px(x2), px(y2)));
    if let Ok(path) = shaft.build() {
        window.paint_path(path, hex_color(color));
    }
    let head_len = 12.0 + width;
    let angle = (y2 - y1).atan2(x2 - x1);
    let mut head = PathBuilder::fill();
    head.move_to(point(px(x2), px(y2)));
    head.line_to(point(
        px(x2 - head_len * (angle - std::f32::consts::PI / 6.0).cos()),
        px(y2 - head_len * (angle - std::f32::consts::PI / 6.0).sin()),
    ));
    head.line_to(point(
        px(x2 - head_len * (angle + std::f32::consts::PI / 6.0).cos()),
        px(y2 - head_len * (angle + std::f32::consts::PI / 6.0).sin()),
    ));
    head.close();
    if let Ok(path) = head.build() {
        window.paint_path(path, hex_color(color));
    }
}

impl Focusable for OverlayView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for OverlayView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        self.logical = [f32::from(size.width) as f64, f32::from(size.height) as f64];
        let items = self.session.items.clone();
        let gesture = self.gesture.clone();
        let color = self.session.color.clone();
        let width = self.session.width;
        let tool = self.session.tool;
        let dim = self.session.dim;
        let capture_path = self.capture_path.clone();
        let selecting = self.selecting;
        let project = self.project_name.clone();
        let banner = self.banner.clone();
        let text_pos = self.text_pos;
        let text_buf = self.text_buf.clone();

        let paint_session = self.session.clone();
        let paint_gesture = gesture.clone();

        let canvas_el = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let mut items = paint_session.items.clone();
                match &paint_gesture {
                    Gesture::Shape { start, end, shift } => {
                        let mut tmp = paint_session.clone();
                        tmp.commit_preview(*start, *end, *shift);
                        items = tmp.items;
                    }
                    Gesture::Freehand { points } if points.len() >= 2 => {
                        items.push(SessionItem::Freehand {
                            points: points.clone(),
                            color: paint_session.color.clone(),
                            stroke_width: paint_session.width,
                        });
                    }
                    Gesture::Select { start, end } => paint_select(window, *start, *end),
                    _ => {}
                }
                for item in &items {
                    paint_item(window, item);
                }
            },
        )
        .size_full();

        let labels = items.into_iter().filter_map(|item| match item {
            SessionItem::Text {
                position,
                content,
                color,
            } => Some(
                div()
                    .absolute()
                    .left(px(position[0] as f32))
                    .top(px(position[1] as f32 - 16.0))
                    .text_color(hex_color(&color))
                    .text_sm()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(content)
                    .into_any_element(),
            ),
            SessionItem::Marker {
                position, number, ..
            } => Some(
                div()
                    .absolute()
                    .left(px(position[0] as f32 - 8.0))
                    .top(px(position[1] as f32 - 8.0))
                    .w(px(16.))
                    .h(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(0xffffff))
                    .text_xs()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(number.to_string())
                    .into_any_element(),
            ),
            _ => None,
        });

        let on_save = cx.listener(Self::on_save);
        let on_cancel = cx.listener(Self::on_cancel);
        let on_undo = cx.listener(Self::on_undo);
        let on_dim = cx.listener(Self::on_dim);
        let on_circle = cx.listener(|this, _: &ToolCircle, _, cx| this.set_tool(Tool::Circle, cx));
        let on_rect = cx.listener(|this, _: &ToolRect, _, cx| this.set_tool(Tool::Rect, cx));
        let on_arrow = cx.listener(|this, _: &ToolArrow, _, cx| this.set_tool(Tool::Arrow, cx));
        let on_free =
            cx.listener(|this, _: &ToolFreehand, _, cx| this.set_tool(Tool::Freehand, cx));
        let on_text = cx.listener(|this, _: &ToolText, _, cx| this.set_tool(Tool::Text, cx));
        let on_marker = cx.listener(|this, _: &ToolMarker, _, cx| this.set_tool(Tool::Marker, cx));
        let on_key = cx.listener(Self::on_key);
        let on_down = cx.listener(Self::on_down);
        let on_move = cx.listener(Self::on_move);
        let on_up = cx.listener(Self::on_up);
        let toolbar = if selecting {
            None
        } else {
            Some(self.toolbar(cx, tool, &color, width, &project))
        };

        div()
            .id("overlay")
            .key_context("Overlay")
            .track_focus(&self.focus)
            .size_full()
            .bg(rgb(0x1a1a1a))
            .text_color(rgb(0xffffff))
            .on_action(on_save)
            .on_action(on_cancel)
            .on_action(on_undo)
            .on_action(on_dim)
            .on_action(on_circle)
            .on_action(on_rect)
            .on_action(on_arrow)
            .on_action(on_free)
            .on_action(on_text)
            .on_action(on_marker)
            .on_key_down(on_key)
            .on_mouse_down(MouseButton::Left, on_down)
            .on_mouse_move(on_move)
            .on_mouse_up(MouseButton::Left, on_up)
            .child(
                img(capture_path)
                    .size_full()
                    .object_fit(if self.crop.is_some() {
                        gpui::ObjectFit::Fill
                    } else {
                        gpui::ObjectFit::Contain
                    }),
            )
            .when(dim, |el| {
                el.child(div().absolute().inset_0().bg(rgb(0x000000).opacity(0.3)))
            })
            .child(div().absolute().inset_0().child(canvas_el))
            .children(labels)
            .when_some(text_pos, |el, pos| {
                el.child(
                    div()
                        .absolute()
                        .left(px(pos[0] as f32))
                        .top(px(pos[1] as f32))
                        .px_1()
                        .bg(rgb(0x000000).opacity(0.6))
                        .text_color(hex_color(&color))
                        .child(if text_buf.is_empty() {
                            "|".to_string()
                        } else {
                            format!("{text_buf}|")
                        }),
                )
            })
            .when_some(banner.clone(), |el, msg| {
                el.child(
                    div()
                        .absolute()
                        .bottom(px(24.))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .bg(rgb(0x111111))
                                .text_color(rgb(0xff3b30))
                                .child(msg),
                        ),
                )
            })
            .when_some(toolbar, |el, toolbar| el.child(toolbar))
            .when(selecting, |el| {
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .bg(rgb(0x000000).opacity(0.55))
                                .child("Drag to select region"),
                        ),
                )
            })
    }
}

impl OverlayView {
    fn toolbar(
        &self,
        cx: &mut Context<Self>,
        tool: Tool,
        color: &str,
        width: f32,
        project: &str,
    ) -> gpui::AnyElement {
        let pos = self.toolbar_pos;
        let colors: Vec<_> = COLORS
            .iter()
            .map(|c| self.color_btn(c, color, cx).into_any_element())
            .collect();
        let widths: Vec<_> = WIDTHS
            .iter()
            .map(|w| self.width_btn(*w, width, cx).into_any_element())
            .collect();
        let bar = div()
            .id("toolbar")
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .rounded_lg()
            .bg(rgb(0x121214).opacity(0.94))
            .border_1()
            .border_color(rgb(0x333333))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.drag_handle(cx))
            .child(self.tool_btn("C", Tool::Circle, tool, cx))
            .child(self.tool_btn("R", Tool::Rect, tool, cx))
            .child(self.tool_btn("A", Tool::Arrow, tool, cx))
            .child(self.tool_btn("F", Tool::Freehand, tool, cx))
            .child(self.tool_btn("T", Tool::Text, tool, cx))
            .child(self.tool_btn("N", Tool::Marker, tool, cx))
            .child(divider())
            .children(colors)
            .child(divider())
            .children(widths)
            .child(divider())
            .child(self.action_btn("Dim", self.session.dim, cx, |this, _, cx| {
                this.session.dim = !this.session.dim;
                cx.notify();
            }))
            .child(self.action_btn("Undo", false, cx, |this, _, cx| {
                this.session.undo();
                cx.notify();
            }))
            .child(self.action_btn("Clear", false, cx, |this, _, cx| {
                this.session.clear();
                cx.notify();
            }))
            .child(self.action_btn("Save", false, cx, |this, window, cx| {
                this.save_and_close(window, cx);
            }))
            .child(divider())
            .child(
                div()
                    .id("project-chip")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(0x2a2a2e))
                    .cursor_pointer()
                    .child(project.to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.cycle_project(cx))),
            );

        match pos {
            Some(p) => div()
                .absolute()
                .left(p.x)
                .top(p.y)
                .child(bar)
                .into_any_element(),
            None => div()
                .absolute()
                .top(px(16.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(bar)
                .into_any_element(),
        }
    }

    fn drag_handle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("toolbar-drag")
            .px_1()
            .text_color(rgb(0x666666))
            .cursor_move()
            .child("=")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    let origin = this.toolbar_pos.unwrap_or(point(px(0.), px(16.)));
                    this.toolbar_drag =
                        Some(point(ev.position.x - origin.x, ev.position.y - origin.y));
                    cx.stop_propagation();
                }),
            )
    }

    fn tool_btn(
        &self,
        label: &'static str,
        tool: Tool,
        current: Tool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = tool == current;
        div()
            .id(label)
            .w(px(28.))
            .h(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .bg(if active { rgb(0x3a3a40) } else { rgb(0x121214) })
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.set_tool(tool, cx)))
    }

    fn color_btn(
        &self,
        hex: &'static str,
        current: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = hex.eq_ignore_ascii_case(current);
        div()
            .id(hex)
            .w(px(16.))
            .h(px(16.))
            .rounded_full()
            .bg(hex_color(hex))
            .border_2()
            .border_color(if active { rgb(0xffffff) } else { rgb(0x121214) })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.session.color = hex.to_string();
                cx.notify();
            }))
    }

    fn width_btn(&self, w: f32, current: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let active = (w - current).abs() < f32::EPSILON;
        div()
            .id(SharedString::from(format!("w{w}")))
            .px_1()
            .rounded_md()
            .cursor_pointer()
            .bg(if active { rgb(0x3a3a40) } else { rgb(0x121214) })
            .child(format!("{w:.0}"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.session.width = w;
                cx.notify();
            }))
    }

    fn action_btn(
        &self,
        label: &'static str,
        active: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut OverlayView, &mut Window, &mut Context<OverlayView>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(label)
            .px_2()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .bg(if active { rgb(0x3a3a40) } else { rgb(0x121214) })
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
    }
}

fn divider() -> impl IntoElement {
    div().w(px(1.)).h(px(18.)).bg(rgb(0x333333)).mx_1()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Annotation;

    #[test]
    fn save_session_writes_pair_to_active_inbox() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".snap/inbox")).unwrap();
        let mut img = image::RgbaImage::new(4, 4);
        for p in img.pixels_mut() {
            *p = image::Rgba([255, 0, 0, 255]);
        }
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        let mut session = Session::new();
        session.add_marker(2.0, 2.0);
        let path = save_session(
            &repo,
            &buf,
            None,
            WindowSource::default(),
            &session,
            4.0,
            4.0,
            Utc::now(),
        )
        .unwrap();
        assert!(path.exists());
        let rec = Inbox::new(repo.join(".snap")).latest(false).unwrap();
        assert!(matches!(
            rec.annotations[0],
            Annotation::Marker { number: 1, .. }
        ));
    }

    #[test]
    fn contain_fit_letterboxes_mismatched_aspect() {
        let fit = ContainFit::new(200.0, 100.0, 100.0, 100.0);
        assert_eq!(fit.offset_x, 50.0);
        assert_eq!(fit.offset_y, 0.0);
        assert_eq!(fit.draw_w, 100.0);
        assert_eq!(fit.draw_h, 100.0);
        assert_eq!(fit.viewport_to_preview(50.0, 0.0), [0.0, 0.0]);
        assert_eq!(fit.viewport_to_preview(150.0, 100.0), [100.0, 100.0]);
        assert_eq!(fit.viewport_to_native(100.0, 50.0), [50.0, 50.0]);
    }

    #[test]
    fn contain_fit_maps_viewport_crop_to_native() {
        let fit = ContainFit::new(400.0, 400.0, 400.0, 200.0);
        assert_eq!(fit.offset_x, 0.0);
        assert_eq!(fit.offset_y, 100.0);
        assert_eq!(fit.draw_w, 400.0);
        assert_eq!(fit.draw_h, 200.0);
        let viewport_crop = Crop {
            x: 50.0,
            y: 150.0,
            w: 100.0,
            h: 50.0,
        };
        let preview = fit.viewport_crop_to_preview(&viewport_crop);
        assert_eq!(
            preview,
            Crop {
                x: 50.0,
                y: 50.0,
                w: 100.0,
                h: 50.0
            }
        );
        let native = map_crop_to_native(&preview, &fit.crop_dims()).unwrap();
        assert_eq!(
            native,
            Crop {
                x: 50.0,
                y: 50.0,
                w: 100.0,
                h: 50.0
            }
        );
    }

    #[test]
    fn contain_fit_scales_preview_crop_when_image_is_larger() {
        let fit = ContainFit::new(200.0, 100.0, 400.0, 200.0);
        assert_eq!(fit.offset_x, 0.0);
        assert_eq!(fit.offset_y, 0.0);
        assert_eq!(fit.draw_w, 200.0);
        let preview = fit.viewport_crop_to_preview(&Crop {
            x: 20.0,
            y: 10.0,
            w: 40.0,
            h: 20.0,
        });
        let native = map_crop_to_native(&preview, &fit.crop_dims()).unwrap();
        assert_eq!(
            native,
            Crop {
                x: 40.0,
                y: 20.0,
                w: 80.0,
                h: 40.0
            }
        );
    }

    #[test]
    fn save_session_uses_contain_fit_preview_coords_not_viewport() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".snap/inbox")).unwrap();
        let mut img = image::RgbaImage::new(8, 4);
        for p in img.pixels_mut() {
            *p = image::Rgba([0, 0, 255, 255]);
        }
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();

        let fit = ContainFit::new(8.0, 8.0, 8.0, 4.0);
        assert_eq!(fit.offset_y, 2.0);
        let [px, py] = fit.viewport_to_preview(4.0, 4.0);
        assert_eq!([px, py], [4.0, 2.0]);

        let mut session = Session::new();
        session.add_marker(4.0, 4.0);
        let mapped = map_session_coords(&session, |x, y| fit.viewport_to_preview(x, y));
        save_session(
            &repo,
            &buf,
            None,
            WindowSource::default(),
            &mapped,
            fit.draw_w,
            fit.draw_h,
            Utc::now(),
        )
        .unwrap();
        let rec = Inbox::new(repo.join(".snap")).latest(false).unwrap();
        match &rec.annotations[0] {
            Annotation::Marker {
                position, number, ..
            } => {
                assert_eq!(*number, 1);
                assert_eq!(*position, [4.0, 2.0]);
            }
            other => panic!("expected marker, got {other:?}"),
        }
    }
}
