//! The preview (the Java `PreviewWindow`): the selected file of the active tab, shown docked
//! next to the sides or in a window of its own (F3, the toolbar, the F-key bar). Images (all
//! formats of `fileem_core::images`) and text are loaded in the background; selection changes
//! within [`SETTLE`] load only the last one, and a newer load stops the older one. Session:
//! `preview.enabled`, `preview.docked`, `preview.width/height/x/y`, `split.preview`.

use std::io;
use std::time::Duration;

use fileem_core::fs::preview::{self as kinds, Kind};
use fileem_core::fs::FileEntry;
use fileem_core::images::{self, Format};
use fileem_core::util::formats;
use slint::{ComponentHandle, Rgba8Pixel, SharedPixelBuffer};

use super::{keys, Shared};
use crate::bg::{self, Job};
use crate::preview::{Zoom, TEXT_SIZE, ZOOM_STEP};
use crate::{PreviewView, PreviewWindow};

/// Selection changes within this time load only the last one (arrow keys through a folder).
pub const SETTLE: Duration = Duration::from_millis(120);

const NOTHING: &str = "Select a file to preview it.";

enum Content {
    Message(String),
    Image(slint::Image, (f64, f64)),
    Text(String),
}

pub struct Preview {
    enabled: bool,
    docked: bool,
    /// What the selection asks for, and what is shown (or loading).
    wanted: Option<FileEntry>,
    shown: Option<FileEntry>,
    loaded: bool,
    content: Content,
    zoom: Zoom,
    /// The space the content is shown in.
    view: (f64, f64),
    generation: u64,
    job: Option<Job>,
    settle: slint::Timer,
    pub(super) window: Option<PreviewWindow>,
    top_seq: i32,
}

impl Default for Preview {
    fn default() -> Self {
        Preview {
            enabled: false,
            docked: false,
            wanted: None,
            shown: None,
            loaded: false,
            content: Content::Message(NOTHING.into()),
            zoom: Zoom::default(),
            view: (400.0, 300.0),
            generation: 0,
            job: None,
            settle: slint::Timer::default(),
            window: None,
            top_seq: 0,
        }
    }
}

fn same(a: Option<&FileEntry>, b: Option<&FileEntry>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.path() == b.path() && a.size() == b.size() && a.modified() == b.modified(),
        (None, None) => true,
        _ => false,
    }
}

/// Restores the session's preview: on or off, docked, the docked pane's width.
pub fn restore(app: &Shared) {
    let (on, docked) = {
        let a = app.borrow();
        if let Some(ui) = a.ui.upgrade() {
            ui.set_preview_split(a.session.get_f64("split.preview", 0.7).clamp(0.3, 0.9) as f32);
        }
        (a.session.get_bool("preview.enabled", false), a.session.get_bool("preview.docked", false))
    };
    app.borrow_mut().preview.docked = docked;
    set_enabled(app, on);
}

pub fn save(app: &Shared) {
    let a = app.borrow();
    let (s, p) = (&a.session, &a.preview);
    s.put("preview.enabled", p.enabled);
    s.put("preview.docked", p.docked);
    if let (Some(ui), true) = (a.ui.upgrade(), p.enabled && p.docked) {
        s.put("split.preview", ui.get_preview_split());
    }
    if let Some(w) = p.window.as_ref().filter(|w| w.window().is_visible()) {
        let scale = w.window().scale_factor();
        let size = w.window().size().to_logical(scale);
        s.put("preview.width", size.width.round() as i64);
        s.put("preview.height", size.height.round() as i64);
        let at = w.window().position().to_logical(scale);
        s.put("preview.x", at.x.round() as i64);
        s.put("preview.y", at.y.round() as i64);
    }
}

pub fn toggle(app: &Shared) {
    let on = !app.borrow().preview.enabled;
    set_enabled(app, on);
}

pub fn toggle_dock(app: &Shared) {
    if let (Some(ui), true) = (app.borrow().ui.upgrade(), app.borrow().preview.docked) {
        app.borrow().session.put("split.preview", ui.get_preview_split());
    }
    let docked = !app.borrow().preview.docked;
    app.borrow_mut().preview.docked = docked;
    place(app);
}

pub fn set_enabled(app: &Shared, on: bool) {
    {
        let mut a = app.borrow_mut();
        a.preview.enabled = on;
        if !on {
            stop_loading(&mut a.preview);
            a.preview.settle.stop();
            // shown again later: load what is selected then
            a.preview.loaded = false;
        }
    }
    place(app);
    if on {
        selection_changed(app);
        load(app);
    }
}

/// Shows the content where enabled and docked say it belongs.
fn place(app: &Shared) {
    let (on, docked) = (app.borrow().preview.enabled, app.borrow().preview.docked);
    let Some(ui) = app.borrow().ui.upgrade() else { return };
    ui.set_preview_on(on);
    ui.set_preview_docked(on && docked);
    if !on || docked {
        hide_window(app);
        push(app);
        return;
    }
    if app.borrow().preview.window.is_none() {
        let w = create_window(app);
        app.borrow_mut().preview.window = Some(w);
    }
    push(app);
    // showing and hiding run Slint's change handlers, which may call back in here: no borrow held
    if let Some(w) = window(app) {
        let _ = w.show();
    }
}

fn create_window(app: &Shared) -> PreviewWindow {
    let w = PreviewWindow::new().expect("the preview window");
    super::theme::style(app, &w);
    {
        let a = app.borrow();
        let s = &a.session;
        let (width, height) = (s.get_f64("preview.width", 560.0).max(260.0), s.get_f64("preview.height", 480.0).max(200.0));
        w.window().set_size(slint::LogicalSize::new(width as f32, height as f32));
        match (s.get("preview.x").and_then(|v| v.parse::<f32>().ok()), s.get("preview.y").and_then(|v| v.parse::<f32>().ok())) {
            (Some(x), Some(y)) => w.window().set_position(slint::LogicalPosition::new(x, y)),
            // first time: next to the main window (where the platform lets windows place themselves)
            _ => {
                if let Some(ui) = a.ui.upgrade() {
                    let main = ui.window();
                    let scale = main.scale_factor();
                    let at = main.position().to_logical(scale);
                    let size = main.size().to_logical(scale);
                    w.window().set_position(slint::LogicalPosition::new(at.x + size.width + 8.0, at.y + 60.0));
                }
            }
        }
    }
    let weak = std::rc::Rc::downgrade(app);
    let with = move |f: fn(&Shared)| {
        let weak = weak.clone();
        move || {
            if let Some(app) = weak.upgrade() {
                f(&app);
            }
        }
    };
    w.on_fit(with(fit));
    w.on_actual(with(actual));
    w.on_dock(with(toggle_dock));
    w.on_switch_off(with(|app| set_enabled(app, false)));
    let weak = std::rc::Rc::downgrade(app);
    w.on_zoom(move |d| {
        if let Some(app) = weak.upgrade() {
            zoom(&app, d);
        }
    });
    let weak = std::rc::Rc::downgrade(app);
    w.on_resized(move |width, height| {
        if let Some(app) = weak.upgrade() {
            resized(&app, width, height);
        }
    });
    let weak = std::rc::Rc::downgrade(app);
    w.on_key(move |text, ctrl, alt, shift| weak.upgrade().is_some_and(|app| keys::key(&app, &text, ctrl, alt, shift)));
    let weak = std::rc::Rc::downgrade(app);
    // closing the window switches the preview off
    w.window().on_close_requested(move || {
        if let Some(app) = weak.upgrade() {
            set_enabled(&app, false);
        }
        slint::CloseRequestResponse::HideWindow
    });
    w
}

/// The preview's window, while it floats (tests, closing the app).
pub fn window(app: &Shared) -> Option<PreviewWindow> {
    app.borrow().preview.window.as_ref().map(|w| w.clone_strong())
}

pub fn hide_window(app: &Shared) {
    if let Some(w) = window(app) {
        let _ = w.hide();
    }
}

/// The active tab's selection may have changed: preview its entry (the focused row if it is
/// selected, else the first selected one) once the selection settles.
pub fn selection_changed(app: &Shared) {
    let entry = {
        let a = app.borrow();
        let tab = a.tab();
        let current = tab.current_row().filter(|&r| tab.is_selected(r));
        current.or_else(|| (0..tab.len()).find(|&r| tab.is_selected(r))).map(|r| tab.row(r).clone())
    };
    let mut a = app.borrow_mut();
    if same(entry.as_ref(), a.preview.wanted.as_ref()) {
        return;
    }
    a.preview.wanted = entry;
    if a.preview.enabled {
        let weak = std::rc::Rc::downgrade(app);
        a.preview.settle.start(slint::TimerMode::SingleShot, SETTLE, move || {
            if let Some(app) = weak.upgrade() {
                load(&app);
            }
        });
    }
}

fn stop_loading(p: &mut Preview) {
    if let Some(job) = p.job.take() {
        job.cancel();
    }
}

type Decoded = io::Result<Option<SharedPixelBuffer<Rgba8Pixel>>>;

fn load(app: &Shared) {
    let (entry, generation) = {
        let mut a = app.borrow_mut();
        let p = &mut a.preview;
        p.settle.stop();
        if p.loaded && same(p.wanted.as_ref(), p.shown.as_ref()) {
            return;
        }
        stop_loading(p);
        p.generation += 1;
        p.loaded = true;
        p.shown = p.wanted.clone();
        (p.shown.clone(), p.generation)
    };
    let Some(e) = entry else {
        show(app, Content::Message(NOTHING.into()));
        return;
    };
    let weak = std::rc::Rc::downgrade(app);
    let path = e.path().to_path_buf();
    let job = match kinds::kind_of(e.extension(), e.is_dir()) {
        Kind::Folder => {
            show(app, Content::Message(format!("{}\nFolder", e.name())));
            return;
        }
        Kind::Image(format) => {
            show(app, Content::Message("Loading…".into()));
            bg::run(
                move |stop| -> Decoded {
                    let p = images::decode_until(&path, kinds::image_side(format), stop)?;
                    Ok(p.map(|p| SharedPixelBuffer::clone_from_slice(&p.rgba, p.width, p.height)))
                },
                move |result: Decoded| {
                    let Some(app) = weak.upgrade() else { return };
                    if app.borrow().preview.generation != generation {
                        return;
                    }
                    match result {
                        Ok(Some(buf)) => {
                            let size = (f64::from(buf.width()), f64::from(buf.height()));
                            show(&app, Content::Image(slint::Image::from_rgba8(buf), size));
                        }
                        Ok(None) => info(&app, &e, no_image(format)),
                        Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                        Err(err) => info(&app, &e, &format!("Cannot read this file: {err}")),
                    }
                },
            )
        }
        // text, and unknown files that may be text after all
        Kind::Text | Kind::Other => {
            show(app, Content::Message("Loading…".into()));
            bg::run(
                move |_| kinds::read_text(&path, kinds::MAX_TEXT_BYTES),
                move |result| {
                    let Some(app) = weak.upgrade() else { return };
                    if app.borrow().preview.generation != generation {
                        return;
                    }
                    match result {
                        Ok(Some(t)) => {
                            let more = formats::size(Some(kinds::MAX_TEXT_BYTES as u64));
                            let text = if t.truncated { format!("{}\n\n… (only the first {more} is shown)", t.content) } else { t.content };
                            show(&app, Content::Text(text));
                        }
                        Ok(None) => info(&app, &e, "No preview available."),
                        Err(err) => info(&app, &e, &format!("Cannot read this file: {err}")),
                    }
                },
            )
        }
    };
    app.borrow_mut().preview.job = Some(job);
}

fn no_image(format: Format) -> &'static str {
    match format {
        Format::Raw => "This raw file has no embedded preview to show.",
        Format::Psd => "No preview available for this Photoshop file.",
        _ => "Cannot show this image.",
    }
}

/// The file's name, type and size, and why there is no preview.
fn info(app: &Shared, e: &FileEntry, reason: &str) {
    let size = if e.is_dir() { String::new() } else { format!("\n{}", formats::size(e.size())) };
    show(app, Content::Message(format!("{}\n{}{size}\n\n{reason}", e.name(), e.type_name())));
}

fn show(app: &Shared, content: Content) {
    {
        let mut a = app.borrow_mut();
        let p = &mut a.preview;
        p.zoom = Zoom::default();
        match &content {
            Content::Image(_, size) => p.zoom.fit(Some(*size), p.view),
            Content::Text(_) => p.zoom.set(1.0),
            Content::Message(_) => {}
        }
        p.content = content;
        p.top_seq += 1;
    }
    push(app);
}

fn image_size(p: &Preview) -> Option<(f64, f64)> {
    match p.content {
        Content::Image(_, size) => Some(size),
        _ => None,
    }
}

pub fn zoom(app: &Shared, direction: i32) {
    let mut a = app.borrow_mut();
    if matches!(a.preview.content, Content::Message(_)) {
        return;
    }
    a.preview.zoom.by(if direction > 0 { ZOOM_STEP } else { 1.0 / ZOOM_STEP });
    drop(a);
    push(app);
}

/// Images: the whole image in the view, never enlarged. Text: the normal size.
pub fn fit(app: &Shared) {
    let mut a = app.borrow_mut();
    let p = &mut a.preview;
    if matches!(p.content, Content::Message(_)) {
        return;
    }
    let (image, view) = (image_size(p), p.view);
    p.zoom.fit(image, view);
    drop(a);
    push(app);
}

pub fn actual(app: &Shared) {
    let mut a = app.borrow_mut();
    if matches!(a.preview.content, Content::Message(_)) {
        return;
    }
    a.preview.zoom.set(1.0);
    drop(a);
    push(app);
}

/// The space the content is shown in changed: a fitted image follows it.
pub fn resized(app: &Shared, width: f32, height: f32) {
    let mut a = app.borrow_mut();
    let p = &mut a.preview;
    p.view = (f64::from(width), f64::from(height));
    let (image, view) = (image_size(p), p.view);
    p.zoom.follow(image, view);
    drop(a);
    push(app);
}

/// Hands the content to the docked pane and the window.
fn push(app: &Shared) {
    let a = app.borrow();
    let p = &a.preview;
    let z = p.zoom.zoom;
    let title = p.shown.as_ref().map(|e| e.name().to_string()).unwrap_or_default();
    let mut view = PreviewView {
        title: title.into(),
        docked: p.docked,
        top_seq: p.top_seq,
        font_size: (TEXT_SIZE * z) as f32,
        ..PreviewView::default()
    };
    match &p.content {
        Content::Message(m) => view.message = m.into(),
        Content::Image(img, (w, h)) => {
            view.mode = 1;
            view.image = img.clone();
            view.image_w = (w * z) as f32;
            view.image_h = (h * z) as f32;
            view.tools = true;
        }
        Content::Text(t) => {
            view.mode = 2;
            view.text = t.as_str().into();
            view.tools = true;
        }
    }
    let (w, ui) = (p.window.as_ref().map(|w| w.clone_strong()), a.ui.upgrade());
    drop(a);
    if let Some(w) = w {
        w.set_view(view.clone());
    }
    if let Some(ui) = ui {
        ui.set_preview(view);
    }
}

/// For tests: whether it is on, docked, what is shown, the zoom and whether it fits.
pub fn state(app: &Shared) -> (bool, bool, Option<String>, f64, bool) {
    let a = app.borrow();
    let p = &a.preview;
    (p.enabled, p.docked, p.shown.as_ref().map(|e| e.name().to_string()), p.zoom.zoom, p.zoom.fitting)
}
