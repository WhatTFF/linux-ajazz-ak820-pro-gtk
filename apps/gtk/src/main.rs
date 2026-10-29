//! GTK3 control app for the AJAZZ / Epomaker AK820 Pro screen.
//!
//! Visual language (dark cards, orange accent, chips, segmented controls)
//! mirrors the upstream web UI; the theme lives in `style.css`.

mod autostart;
mod decode;
mod fx;
mod gen;
mod net;
mod update;

use std::cell::{Cell, RefCell};
use std::io::Read;
use std::io::Write;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ak820_device::{Device, DeviceLock};
use ak820_proto::{build_payload, parse_payload, CivilTime, FRAME_BYTES, MAX_FRAMES};
use ak820_store::{history, settings};
use chrono::{Datelike, Timelike};
use gdk_pixbuf::{Colorspace, InterpType, Pixbuf};
use glib::signal::Inhibit;
use glib::Continue;
use gtk::prelude::*;
use gtk::{
    Box, Button, ComboBoxText, Dialog, Entry, EventBox, FileChooserAction, FileChooserDialog,
    FlowBox, Grid, HeaderBar, Image, Label, MessageDialog, Orientation, PolicyType, ProgressBar,
    ResponseType, ScrolledWindow, Stack, TextView, ToggleButton, Window, WindowType,
};
use image::RgbaImage;

use crate::decode::{load as decode_load, Fit as DecodeFit};
use crate::fx::{
    composite_over, draw_frame, draw_lines_to_surface, fit_text, text_width, CapPos, FrameOpts, Fx,
    TextOpts,
};
use crate::gen::{default_params, generators, hsv as gen_hsv, GenParams, SourceFrame};

const W: i32 = 128;
const H: i32 = 128;
const PREVIEW_PX: i32 = 244;
const CANVAS_PX: i32 = 384;
const THUMB_PX: i32 = 114;
const HIST_PX: i32 = 92;

/// App-wide editable state.
#[derive(Clone)]
struct App {
    gen_id: RefCell<String>,
    gen_params: RefCell<GenParams>,
    smooth: Cell<u16>,
    fx: RefCell<Fx>,
    opts: RefCell<FrameOpts>,
    source: RefCell<Vec<SourceFrame>>,
    output: RefCell<Vec<SourceFrame>>,
    speed: Cell<i32>,
    trim_start: Cell<usize>,
    trim_end: Cell<usize>,
    skip: Cell<usize>,
    syncing_trim: Cell<bool>,
    name: RefCell<String>,
    max_frames: RefCell<usize>,
    cancel: RefCell<Option<Arc<AtomicBool>>>,
    connected: Cell<bool>,
    has_file: Cell<bool>,
    file_source: RefCell<Vec<SourceFrame>>,
    file_name: RefCell<String>,
    // ---- Draw ----
    draw_buffer: RefCell<RgbaImage>,
    draw_undo: RefCell<Vec<RgbaImage>>,
    draw_tool: RefCell<String>,
    draw_size: RefCell<f64>,
    draw_color: RefCell<[u8; 3]>,
    draw_rainbow: RefCell<bool>,
    draw_bg: RefCell<[u8; 3]>,
    // ---- Text ----
    text_value: RefCell<String>,
    text_mode: RefCell<String>,
    text_colour: RefCell<String>,
    text_size: RefCell<f64>,
    text_speed: RefCell<f64>,
    text_fg: RefCell<[u8; 3]>,
    text_bg: RefCell<[u8; 3]>,
    text_outline: RefCell<bool>,
    text_font: RefCell<String>,
    // ---- background work ----
    sync_busy: Cell<bool>,
    last_update_check: Cell<Option<Instant>>,
}

impl App {
    fn new() -> Self {
        let db = RgbaImage::from_pixel(W as u32, H as u32, image::Rgba([0, 0, 0, 255]));
        Self {
            gen_id: RefCell::new("rainbow".into()),
            gen_params: RefCell::new(default_params("rainbow")),
            smooth: Cell::new(36),
            fx: RefCell::new(Fx::default()),
            opts: RefCell::new(FrameOpts {
                fit: fx::Fit::Cover,
                rot: 0,
                mirror: false,
                bg: [0, 0, 0],
                zoom: 100.0,
                panx: 0.0,
                pany: 0.0,
                bri: 100.0,
                con: 100.0,
                sat: 100.0,
            }),
            source: RefCell::new(Vec::new()),
            output: RefCell::new(Vec::new()),
            speed: Cell::new(0),
            trim_start: Cell::new(0),
            trim_end: Cell::new(usize::MAX),
            skip: Cell::new(1),
            syncing_trim: Cell::new(false),
            name: RefCell::new("Animation".into()),
            max_frames: RefCell::new(MAX_FRAMES),
            cancel: RefCell::new(None),
            connected: Cell::new(false),
            has_file: Cell::new(false),
            file_source: RefCell::new(Vec::new()),
            file_name: RefCell::new("File".into()),
            draw_buffer: RefCell::new(db),
            draw_undo: RefCell::new(Vec::new()),
            draw_tool: RefCell::new("brush".into()),
            draw_size: RefCell::new(4.0),
            draw_color: RefCell::new([255, 255, 255]),
            draw_rainbow: RefCell::new(false),
            draw_bg: RefCell::new([0, 0, 0]),
            text_value: RefCell::new("HELLO".into()),
            text_mode: RefCell::new("static".into()),
            text_colour: RefCell::new("solid".into()),
            text_size: RefCell::new(40.0),
            text_speed: RefCell::new(3.0),
            text_fg: RefCell::new([255, 255, 255]),
            text_bg: RefCell::new([232, 89, 12]),
            text_outline: RefCell::new(false),
            text_font: RefCell::new("sans".into()),
            sync_busy: Cell::new(false),
            last_update_check: Cell::new(None),
        }
    }

    fn rebuild_output(&self) {
        let fx = self.fx.borrow().clone();
        let opts = self.opts.borrow().clone();
        let source = self.source.borrow().clone();
        let max_frames = *self.max_frames.borrow();
        if source.is_empty() {
            self.output.replace(Vec::new());
            return;
        }
        let len = source.len();
        let mut a = self.trim_start.get().min(len - 1);
        let mut b = self.trim_end.get().min(len - 1);
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        let skip = self.skip.get().max(1);
        let speed = 2f64.powi(self.speed.get());

        let mut plan: Vec<(usize, f64)> = Vec::new();
        let mut i = a;
        while i <= b {
            let end = (i + skip).min(b + 1);
            let delay: f64 = source[i..end].iter().map(|f| f.delay_ms as f64).sum();
            plan.push((i, delay / speed));
            i += skip;
        }
        if plan.len() == 1 && fx::is_animated(&fx) {
            let n = fx.loop_len.max(1) as usize;
            plan = (0..n).map(|_| (plan[0].0, 70.0 / speed)).collect();
        }

        let loop_len = plan.len().min(max_frames).max(1);
        let n = loop_len;
        let mut out = Vec::with_capacity(n);
        for (oi, (si, delay)) in plan.iter().take(n).enumerate() {
            let t = (oi % loop_len) as f64 / loop_len as f64;
            let sf = &source[*si];
            let mut rendered =
                RgbaImage::from_pixel(W as u32, H as u32, image::Rgba([0, 0, 0, 255]));
            draw_frame(&sf.image, &opts, &fx, t, oi, None, &mut rendered);
            out.push(SourceFrame {
                image: rendered,
                delay_ms: (delay.round() as i64).clamp(2, 510) as u16,
            });
        }
        self.output.replace(out);
    }

    /// Render the current text state into source frames.
    fn build_text_frames(&self) -> Vec<SourceFrame> {
        let raw = self.text_value.borrow().clone();
        let text = raw.trim().to_string();
        let text = if text.is_empty() {
            " ".to_string()
        } else {
            text
        };
        let mode = self.text_mode.borrow().clone();
        let rainbow = *self.text_colour.borrow() == "rainbow";
        let max_size = *self.text_size.borrow();
        let speed = *self.text_speed.borrow();
        let fg = *self.text_fg.borrow();
        let bg = *self.text_bg.borrow();
        let outline = *self.text_outline.borrow();
        let family = self.text_font.borrow().clone();
        let (size, lines) = fit_text(
            &(),
            &text,
            &family,
            max_size,
            W as f64 - 8.0,
            H as f64 - 8.0,
        );
        let total = lines
            .iter()
            .map(|l| l.chars().count())
            .sum::<usize>()
            .max(1);

        let render = |size: f64,
                      lines: &[String],
                      phase: f64,
                      limit: Option<usize>,
                      wave: Option<f64>,
                      cx: Option<f64>|
         -> SourceFrame {
            let mut img =
                RgbaImage::from_pixel(W as u32, H as u32, image::Rgba([bg[0], bg[1], bg[2], 255]));
            let opts = TextOpts {
                size,
                family: family.clone(),
                cx,
                cy: H as f64 / 2.0 + 1.0,
                fill: fg,
                rainbow,
                phase,
                wave,
                outline: if outline { Some([0, 0, 0]) } else { None },
                limit,
            };
            let surf = draw_lines_to_surface(lines, &opts);
            composite_over(&mut img, &surf);
            SourceFrame {
                image: img,
                delay_ms: 100,
            }
        };
        let blank = || -> SourceFrame {
            SourceFrame {
                image: RgbaImage::from_pixel(
                    W as u32,
                    H as u32,
                    image::Rgba([bg[0], bg[1], bg[2], 255]),
                ),
                delay_ms: 500,
            }
        };

        let mut out = Vec::new();
        match mode.as_str() {
            "scroll" => {
                let line = text.replace('\n', " ");
                let scroll_size = max_size.min((H as f64 - 8.0) / 7.0).max(2.0);
                let tw = text_width(&line, scroll_size, &family);
                let dist = W as f64 + tw;
                let px = speed.max((dist / MAX_FRAMES as f64).ceil()).max(1.0);
                let n = (dist / px).ceil().max(1.0) as usize;
                let line = vec![line];
                for k in 0..n {
                    let cx = W as f64 - k as f64 * px + tw / 2.0;
                    let mut f = render(
                        scroll_size,
                        &line,
                        k as f64 / n as f64,
                        None,
                        None,
                        Some(cx),
                    );
                    f.delay_ms = 70;
                    out.push(f);
                }
            }
            "blink" => {
                let delay = (560.0 - speed * 50.0).max(80.0) as u16;
                let mut on = render(size, &lines, 0.0, None, None, None);
                on.delay_ms = delay;
                out.push(on);
                let mut off = blank();
                off.delay_ms = delay;
                out.push(off);
            }
            "wave" => {
                let delay = (90.0 - speed * 8.0).max(30.0) as u16;
                let n = 24usize;
                for k in 0..n {
                    let mut f = render(
                        size,
                        &lines,
                        k as f64 / n as f64,
                        None,
                        Some(size * 0.18),
                        None,
                    );
                    f.delay_ms = delay;
                    out.push(f);
                }
            }
            "type" => {
                let step = ((total as f64) / (MAX_FRAMES as f64 - 5.0)).ceil().max(1.0) as usize;
                let delay = (200.0 - speed * 20.0).max(40.0) as u16;
                let mut k = step;
                while k < total + step {
                    let limit = k.min(total);
                    let mut f = render(size, &lines, 0.0, Some(limit), None, None);
                    f.delay_ms = delay;
                    out.push(f);
                    k += step;
                }
                for _ in 0..3 {
                    let mut f = render(size, &lines, 0.0, None, None, None);
                    f.delay_ms = 500;
                    out.push(f);
                }
                let mut off = blank();
                off.delay_ms = 300;
                out.push(off);
            }
            _ => {
                let n = if rainbow { 24usize } else { 1 };
                let delay = if rainbow {
                    (110.0 - speed * 10.0).max(30.0) as u16
                } else {
                    500
                };
                for k in 0..n {
                    let mut f = render(size, &lines, k as f64 / n as f64, None, None, None);
                    f.delay_ms = delay;
                    out.push(f);
                }
            }
        }
        out
    }

    fn set_source(&self, frames: Vec<SourceFrame>) {
        let limit = *self.max_frames.borrow();
        let frames = frames.into_iter().take(limit).collect::<Vec<_>>();
        let len = frames.len();
        self.source.replace(frames);
        self.trim_start.set(0);
        self.trim_end.set(len.saturating_sub(1));
        self.rebuild_output();
    }

    fn set_text_source(&self) {
        self.set_source(self.build_text_frames());
    }

    fn draw_to_source(&self) {
        let frame = SourceFrame {
            image: self.draw_buffer.borrow().clone(),
            delay_ms: 100,
        };
        self.set_source(vec![frame]);
    }

    fn output_payload(&self) -> Vec<u8> {
        let out = self.output.borrow();
        let limit = *self.max_frames.borrow();
        let n = out.len().min(limit).max(1);
        let mut all_px: Vec<Vec<u8>> = Vec::with_capacity(n);
        for sf in out.iter().take(n) {
            let img = &sf.image;
            let mut px = Vec::with_capacity(W as usize * H as usize * 2);
            for y in 0..H as u32 {
                for x in 0..W as u32 {
                    let p = img.get_pixel(x, y).0;
                    let v = ((u16::from(p[0]) & 0xf8) << 8)
                        | ((u16::from(p[1]) & 0xfc) << 3)
                        | (u16::from(p[2]) >> 3);
                    px.push(v as u8);
                    px.push((v >> 8) as u8);
                }
            }
            all_px.push(px);
        }
        let frames: Vec<ak820_proto::Frame<'_>> = all_px
            .iter()
            .zip(out.iter().take(n).map(|sf| sf.delay_ms))
            .map(|(px, delay)| ak820_proto::Frame {
                pixels: px,
                delay_ms: delay,
            })
            .collect();
        build_payload(&frames).unwrap_or_default()
    }
}

#[derive(Clone)]
struct Widgets {
    window: Window,
    preview: Image,
    draw_canvas: Image,
    draw_area: EventBox,
    msg: Label,
    frames_info: Label,
    size_info: Label,
    pill: Label,
    pill_dot: Label,
    connect_btn: Button,
    settings_btn: Button,

    dev_panel: Box,
    dev_vid_pid: Label,
    dev_firmware: Label,
    dev_mode: Label,
    dev_battery: Label,
    dev_charging: Label,
    dev_profile: Label,
    dev_frames: Label,
    dev_direction: Label,
    sync_btn: Button,
    sync_status: Label,
    builtin_combo: ComboBoxText,
    builtin_btn: Button,
    builtin_status: Label,

    tab_buttons: RefCell<Vec<(String, ToggleButton)>>,
    file_btn: Button,
    upload_btn: Button,
    save_hist: Button,
    upload_progress: ProgressBar,
    cancel_btn: Button,

    giphy_entry: Entry,
    giphy_rating: ComboBoxText,
    giphy_search_btn: Button,
    giphy_more_btn: Button,
    giphy_box: FlowBox,
    giphy_scroll: ScrolledWindow,
    giphy_status: Label,
    giphy_query: RefCell<String>,
    giphy_offset: RefCell<usize>,

    gen_grid: FlowBox,
    gen_param_box: Box,
    gen_buttons: RefCell<Vec<(String, String, ToggleButton)>>,
    gframes_scale: gtk::Scale,
    gframes_out: Label,
    shuffle_btn: Button,
    surprise_btn: Button,

    txt_view: TextView,
    tmode_buttons: RefCell<Vec<(String, ToggleButton)>>,
    tcol_buttons: RefCell<Vec<(String, ToggleButton)>>,
    tsize_scale: gtk::Scale,
    tsize_out: Label,
    tfg_btn: gtk::ColorButton,
    tbg_btn: gtk::ColorButton,
    toutline_btn: ToggleButton,
    tfont_combo: ComboBoxText,
    tspeed_scale: gtk::Scale,
    tspeed_out: Label,

    tool_buttons: RefCell<Vec<(String, ToggleButton)>>,
    bsize_scale: gtk::Scale,
    bsize_out: Label,
    bcolor_btn: gtk::ColorButton,
    brainbow_btn: ToggleButton,
    dbg_btn: gtk::ColorButton,
    dclear_btn: Button,
    dundo_btn: Button,
    dtrace_btn: Button,

    fx_chips: RefCell<Vec<(String, ToggleButton)>>,
    fx_random_btn: Button,
    fx_clear_btn: Button,
    fx_hue_scale: gtk::Scale,
    fx_hue_out: Label,
    fx_pixel_scale: gtk::Scale,
    fx_pixel_out: Label,
    fx_cap_entry: Entry,
    fx_cappos_buttons: RefCell<Vec<(String, ToggleButton)>>,
    fx_capcolor_btn: gtk::ColorButton,
    fx_loop_scale: gtk::Scale,
    fx_loop_out: Label,

    fit_buttons: RefCell<Vec<(String, ToggleButton)>>,
    rot_buttons: RefCell<Vec<(String, ToggleButton)>>,
    zoom_scale: gtk::Scale,
    zoom_out: Label,
    bg_btn: gtk::ColorButton,
    flip_btn: ToggleButton,
    panx_scale: gtk::Scale,
    panx_out: Label,
    pany_scale: gtk::Scale,
    pany_out: Label,
    bri_scale: gtk::Scale,
    bri_out: Label,
    con_scale: gtk::Scale,
    con_out: Label,
    sat_scale: gtk::Scale,
    sat_out: Label,
    speed_scale: gtk::Scale,
    speed_out: Label,
    trim_start: gtk::Scale,
    trim_end: gtk::Scale,
    skip_scale: gtk::Scale,
    tstart_out: Label,
    tend_out: Label,
    skip_out: Label,
    adjust_reset_btn: Button,

    history_box: FlowBox,
    hist_clear_btn: Button,
    app: Rc<App>,
}

enum Instance {
    Running,
    Listener(UnixListener),
    Unavailable,
}

fn socket_path() -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    let user = std::env::var("USER").unwrap_or_else(|_| "default".into());
    PathBuf::from(dir).join(format!("ak820-gtk-{user}.sock"))
}

/// A second launch asks the already-running instance to show its window.
fn claim_instance() -> Instance {
    let path = socket_path();
    if let Ok(mut stream) = UnixStream::connect(&path) {
        let _ = stream.write_all(b"show\n");
        return Instance::Running;
    }
    let _ = std::fs::remove_file(&path);
    match UnixListener::bind(&path) {
        Ok(listener) => Instance::Listener(listener),
        Err(_) => Instance::Unavailable,
    }
}

fn main() {
    let listener = match claim_instance() {
        Instance::Running => return,
        Instance::Listener(listener) => Some(listener),
        Instance::Unavailable => None,
    };
    let tray_flag = std::env::args().any(|a| a == "--tray");
    gtk::init().expect("failed to initialize GTK");
    install_theme();
    if let Some(s) = gtk::Settings::get_default() {
        s.set_property_gtk_application_prefer_dark_theme(true);
    }

    let window = Window::new(WindowType::Toplevel);
    window.set_title("AK820 Screen Studio");
    window.set_default_size(1100, 820);
    window.set_size_request(920, 620);
    window.get_style_context().add_class("ak820");

    let app = Rc::new(App::new());
    let w = build_ui(&window, &app);
    connect_signals(&w, &app);
    start_preview_timer(&w);
    start_watchers(&w);

    let initial = settings::Settings::load();
    let _ = autostart::set(initial.start_at_login);

    // The tray indicator must exist before the window is hidden, otherwise a
    // start-minimized launch would be unreachable on desktop environments
    // without another way to restore it.
    let _tray = build_tray(&w);
    let has_tray = _tray.is_some();

    // Closing hides the window when "keep running" is enabled and a tray icon
    // exists to reopen it. Without one, quit so the app is never lost.
    window.connect_delete_event(move |win, _| {
        if has_tray && settings::Settings::load().minimize_to_tray {
            win.hide();
            eprintln!("AK820 Screen Studio is still running. Use the tray icon to reopen it.");
            Inhibit(true)
        } else {
            gtk::main_quit();
            Inhibit(false)
        }
    });
    window.connect_destroy(|_| gtk::main_quit());

    // A second launch reopens the hidden window of this instance.
    if let Some(listener) = listener {
        let (tx, rx) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                drop(stream);
                let _ = tx.send(());
            }
        });
        let window = window.clone();
        glib::timeout_add_local(200, move || {
            while rx.try_recv().is_ok() {
                show_window(&window);
            }
            Continue(true)
        });
    }

    window.show_all();
    w.upload_progress.hide();
    w.cancel_btn.hide();
    w.sync_btn.set_sensitive(false);
    if (initial.start_minimized || tray_flag) && has_tray {
        window.hide();
    }

    // Populate the keyboard panel automatically when a device is already there.
    {
        let w2 = w.clone();
        glib::timeout_add_local(700, move || {
            if !ak820_device::presence().is_empty() {
                connect_device(&w2);
            }
            Continue(false)
        });
    }

    // Sync once at start, then let the 12-hour watcher take over.
    if initial.sync_on_start {
        let w2 = w.clone();
        let quit_after = initial.quit_after_start_sync;
        glib::timeout_add_local(1500, move || {
            spawn_clock_sync(
                &w2,
                true,
                Some(std::boxed::Box::new(move |_| {
                    if quit_after {
                        gtk::main_quit();
                    }
                })),
            );
            Continue(false)
        });
    }

    schedule_update_check(&w);
    gtk::main();
}

fn install_theme() {
    let provider = gtk::CssProvider::new();
    provider
        .load_from_data(include_bytes!("../style.css"))
        .expect("style.css failed to parse");
    if let Some(screen) = gdk::Screen::get_default() {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn start_preview_timer(w: &Widgets) {
    let app = Rc::clone(&w.app);
    let preview = w.preview.clone();
    let index = Cell::new(0usize);
    let elapsed = Cell::new(0u64);
    glib::timeout_add_local(16, move || {
        let out = app.output.borrow();
        if out.is_empty() {
            index.set(0);
            elapsed.set(0);
            return Continue(true);
        }
        if index.get() >= out.len() {
            index.set(0);
        }
        let delay = out[index.get()].delay_ms.max(16) as u64;
        elapsed.set(elapsed.get() + 16);
        if elapsed.get() >= delay {
            elapsed.set(0);
            let i = index.get();
            let pb = pixbuf_from_rgba(&out[i].image, PREVIEW_PX);
            preview.set_from_pixbuf(Some(&pb));
            index.set((i + 1) % out.len());
        }
        Continue(true)
    });
}

// ---------- small UI helpers ----------

fn rgba(r: f64, g: f64, b: f64) -> gdk::RGBA {
    gdk::RGBA {
        red: r,
        green: g,
        blue: b,
        alpha: 1.0,
    }
}

fn add_class<W: IsA<gtk::Widget>>(widget: &W, class: &str) {
    widget.get_style_context().add_class(class);
}

fn mk_label(text: &str, class: &str) -> Label {
    let l = Label::new(Some(text));
    l.set_xalign(0.0);
    if !class.is_empty() {
        add_class(&l, class);
    }
    l
}

fn heading(text: &str) -> Label {
    let l = Label::new(Some(&text.to_uppercase()));
    l.set_xalign(0.0);
    add_class(&l, "section-title");
    l
}

/// Outer card (background + radius) with an inner padded content box.
fn card(pad: i32, spacing: i32) -> (Box, Box) {
    let outer = Box::new(Orientation::Vertical, 0);
    add_class(&outer, "card");
    let inner = Box::new(Orientation::Vertical, spacing);
    inner.set_margin_start(pad);
    inner.set_margin_end(pad);
    inner.set_margin_top(pad);
    inner.set_margin_bottom(pad);
    outer.pack_start(&inner, true, true, 0);
    (outer, inner)
}

fn pixbuf_from_rgba(img: &RgbaImage, size: i32) -> Pixbuf {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for p in img.pixels() {
        data.extend_from_slice(&p.0);
    }
    let pb = Pixbuf::from_mut_slice(data, Colorspace::Rgb, true, 8, w, h, w * 4);
    if size == w {
        pb
    } else {
        match pb.scale_simple(size, size, InterpType::Nearest) {
            Some(s) => s,
            None => pb,
        }
    }
}

fn rgb565_image(px: &[u8]) -> RgbaImage {
    let mut rgba = Vec::with_capacity(W as usize * H as usize * 4);
    for p in px.chunks_exact(2) {
        let v = u16::from_le_bytes([p[0], p[1]]);
        rgba.push((((v >> 11) & 31) * 255 / 31) as u8);
        rgba.push((((v >> 5) & 63) * 255 / 63) as u8);
        rgba.push(((v & 31) * 255 / 31) as u8);
        rgba.push(255);
    }
    RgbaImage::from_raw(W as u32, H as u32, rgba)
        .unwrap_or_else(|| RgbaImage::from_pixel(W as u32, H as u32, image::Rgba([0, 0, 0, 255])))
}

fn frames_from_payload(payload: &[u8]) -> Result<Vec<SourceFrame>, String> {
    let parsed = parse_payload(payload, usize::MAX).map_err(|e| e.to_string())?;
    Ok(parsed
        .into_iter()
        .map(|f| SourceFrame {
            image: rgb565_image(&f.pixels),
            delay_ms: f.delay_ms,
        })
        .collect())
}

fn update_preview(w: &Widgets, app: &App) {
    let n;
    {
        let out = app.output.borrow();
        n = out.len();
        if let Some(sf) = out.first() {
            let pb = pixbuf_from_rgba(&sf.image, PREVIEW_PX);
            w.preview.set_from_pixbuf(Some(&pb));
        }
    }
    w.frames_info
        .set_text(&format!("{} frame{}", n, if n == 1 { "" } else { "s" }));
    let secs = (n * W as usize * H as usize * 2).div_ceil(30000);
    w.size_info.set_text(&format!("~{secs} s to send"));
    w.upload_btn.set_sensitive(n > 0 && app.connected.get());
    w.save_hist.set_sensitive(n > 0);

    // Keep the trim sliders in sync without re-entering the render pipeline.
    let len = app.source.borrow().len();
    let max = len.saturating_sub(1);
    let a = app.trim_start.get().min(max);
    let b = app.trim_end.get().min(max);
    app.syncing_trim.set(true);
    w.trim_start.set_range(0.0, max.max(1) as f64);
    w.trim_end.set_range(0.0, max.max(1) as f64);
    if (w.trim_start.get_value() - a as f64).abs() > 0.5 {
        w.trim_start.set_value(a as f64);
    }
    if (w.trim_end.get_value() - b as f64).abs() > 0.5 {
        w.trim_end.set_value(b as f64);
    }
    app.syncing_trim.set(false);
    w.tstart_out.set_text(&format!("{}", a + 1));
    w.tend_out.set_text(&format!("{}", b + 1));
}

fn update_canvas(w: &Widgets, app: &App) {
    let db = app.draw_buffer.borrow();
    let pb = pixbuf_from_rgba(&db, CANVAS_PX);
    w.draw_canvas.set_from_pixbuf(Some(&pb));
}

fn regenerate(w: &Widgets, app: &Rc<App>) {
    let id = app.gen_id.borrow().clone();
    let params = app.gen_params.borrow().clone();
    let smooth = app.smooth.get();
    if let Some(g) = generators().iter().find(|g| g.id == id) {
        let frames = (g.build_fn)(&params, smooth);
        app.set_source(frames);
    }
    update_preview(w, app);
}

fn set_param(app: &Rc<App>, w: &Widgets, field: &str, value: &str) {
    if field == "text" {
        app.gen_params.borrow_mut().text = value.to_string();
    } else {
        let mut p = app.gen_params.borrow_mut();
        match field {
            "pattern" => {
                p.pattern = match value {
                    "v" => gen::Pattern::V,
                    "d" => gen::Pattern::D,
                    "r" => gen::Pattern::R,
                    "s" => gen::Pattern::S,
                    "c" => gen::Pattern::C,
                    _ => gen::Pattern::H,
                }
            }
            "soft" => {
                p.soft = match value {
                    "pastel" => gen::Soft::Pastel,
                    "dark" => gen::Soft::Dark,
                    _ => gen::Soft::Full,
                }
            }
            "palette" => {
                p.palette = match value {
                    "neon" => gen::Palette::Neon,
                    "fire" => gen::Palette::Fire,
                    "ocean" => gen::Palette::Ocean,
                    "toxic" => gen::Palette::Toxic,
                    "mono" => gen::Palette::Mono,
                    _ => gen::Palette::Rainbow,
                }
            }
            "color" => {
                p.color = match value {
                    "blue" => gen::Color::Blue,
                    "red" => gen::Color::Red,
                    "classic" => gen::Color::Classic,
                    "rainbow" => gen::Color::Rainbow,
                    "white" => gen::Color::White,
                    "gold" => gen::Color::Gold,
                    "mono" => gen::Color::Mono,
                    _ => gen::Color::Green,
                }
            }
            "trail" => {
                p.trail = if value == "on" {
                    gen::Trail::On
                } else {
                    gen::Trail::Off
                }
            }
            "flame" => {
                p.flame = match value {
                    "ice" => gen::Flame::Ice,
                    "toxic" => gen::Flame::Toxic,
                    "rainbow" => gen::Flame::Rainbow,
                    _ => gen::Flame::Classic,
                }
            }
            "sky" => {
                p.sky = match value {
                    "dusk" => gen::Sky::Dusk,
                    "black" => gen::Sky::Black,
                    _ => gen::Sky::Night,
                }
            }
            "mood" => {
                p.mood = match value {
                    "rainbow" => gen::Mood::Rainbow,
                    "pastel" => gen::Mood::Pastel,
                    _ => gen::Mood::Love,
                }
            }
            "size" => {
                p.size = match value {
                    "small" => gen::Size::Small,
                    "large" => gen::Size::Large,
                    _ => gen::Size::Medium,
                }
            }
            "bands" => p.bands = value.parse().unwrap_or(1),
            _ => {}
        }
    }
    regenerate(w, app);
}

// ---------- reusable controls ----------

fn scale_ctl(title: &str, min: f64, max: f64, step: f64, value: f64) -> (Box, gtk::Scale, Label) {
    let vbox = Box::new(Orientation::Vertical, 4);
    add_class(&vbox, "ctl");
    vbox.set_hexpand(true);
    let row = Box::new(Orientation::Horizontal, 8);
    let l = mk_label(title, "ctl-label");
    l.set_halign(gtk::Align::Start);
    let out = mk_label("", "ctl-value");
    out.set_halign(gtk::Align::End);
    row.pack_start(&l, true, true, 0);
    row.pack_start(&out, false, false, 0);
    let scale = gtk::Scale::with_range(Orientation::Horizontal, min, max, step);
    scale.set_value(value);
    scale.set_hexpand(true);
    scale.set_draw_value(false);
    let adj = scale.get_adjustment();
    adj.set_step_increment(step.max(1.0));
    adj.set_page_increment((step * 10.0).max(10.0));
    no_scroll(&scale);
    vbox.pack_start(&row, false, false, 0);
    vbox.pack_start(&scale, false, false, 0);
    (vbox, scale, out)
}

/// Keep vertical wheel/touchpad scrolling for the page, but keep the slider
/// itself usable with horizontal scrolling.
fn no_scroll(scale: &gtk::Scale) {
    scale.connect_scroll_event(move |s, ev| {
        let (dx, dy) = ev.get_delta();
        let dir = ev.get_direction();
        let horizontal = matches!(
            dir,
            gdk::ScrollDirection::Left | gdk::ScrollDirection::Right
        ) || dx.abs() > dy.abs();

        if horizontal {
            let adj = s.get_adjustment();
            let step = adj.get_step_increment().max(1.0);
            let delta = if dx != 0.0 {
                dx * step
            } else {
                match dir {
                    gdk::ScrollDirection::Left => -step,
                    gdk::ScrollDirection::Right => step,
                    _ => 0.0,
                }
            };
            if delta != 0.0 {
                s.set_value((s.get_value() + delta).clamp(adj.get_lower(), adj.get_upper()));
            }
            return Inhibit(true);
        }

        // Vertical scrolling belongs to the page, not the slider.
        if let Some(scroll) = s
            .get_ancestor(gtk::ScrolledWindow::static_type())
            .and_then(|w| w.downcast::<gtk::ScrolledWindow>().ok())
        {
            if let Some(adj) = scroll.get_vadjustment() {
                let delta = if dy != 0.0 {
                    dy * 20.0
                } else {
                    match dir {
                        gdk::ScrollDirection::Up => -40.0,
                        gdk::ScrollDirection::Down => 40.0,
                        _ => 0.0,
                    }
                };
                if delta != 0.0 {
                    let max = (adj.get_upper() - adj.get_page_size()).max(adj.get_lower());
                    adj.set_value((adj.get_value() + delta).clamp(adj.get_lower(), max));
                }
            }
        }
        Inhibit(true)
    });
}

fn seg_box(
    title: &str,
    items: &[(&str, &str)],
    active: &str,
) -> (Box, Vec<(String, ToggleButton)>) {
    let vbox = Box::new(Orientation::Vertical, 4);
    add_class(&vbox, "ctl");
    vbox.set_hexpand(true);
    if !title.is_empty() {
        let l = mk_label(title, "ctl-label");
        vbox.pack_start(&l, false, false, 0);
    }
    let row = Box::new(Orientation::Horizontal, 4);
    row.set_homogeneous(false);
    let mut buttons: Vec<(String, ToggleButton)> = Vec::new();
    for (value, label) in items {
        let b = ToggleButton::with_label(label);
        add_class(&b, "seg-btn");
        b.set_hexpand(true);
        b.set_active(*value == active);
        row.pack_start(&b, true, true, 0);
        buttons.push(((*value).to_string(), b));
    }
    vbox.pack_start(&row, false, false, 0);
    (vbox, buttons)
}

fn chip_row(items: &[(&str, &str)]) -> (Box, Vec<(String, ToggleButton)>) {
    let row = Box::new(Orientation::Horizontal, 6);
    let mut buttons = Vec::new();
    for (key, label) in items {
        let b = ToggleButton::with_label(label);
        add_class(&b, "chip");
        row.pack_start(&b, false, false, 0);
        buttons.push(((*key).to_string(), b));
    }
    (row, buttons)
}

fn wire_seg(buttons: &[(String, ToggleButton)], app: &Rc<App>, w: &Widgets, field: &str) {
    make_exclusive(buttons);
    for (value, button) in buttons {
        let value = value.clone();
        let field = field.to_string();
        let app = Rc::clone(app);
        let w = w.clone();
        button.connect_toggled(move |b| {
            if b.get_active() {
                set_param(&app, &w, &field, &value);
            }
        });
    }
}

fn wire_scale_ctl<F>(
    scale: &gtk::Scale,
    out: &Label,
    app: &Rc<App>,
    w: &Widgets,
    field: &str,
    fmt: F,
) where
    F: Fn(f64) -> String + 'static,
{
    out.set_text(&fmt(scale.get_value()));
    let out = out.clone();
    let app = Rc::clone(app);
    let w = w.clone();
    let field = field.to_string();
    scale.connect_value_changed(move |s| {
        let v = s.get_value();
        out.set_text(&fmt(v));
        set_param(&app, &w, &field, &format!("{v}"));
    });
}

fn build_ui(window: &Window, app: &Rc<App>) -> Widgets {
    let header = HeaderBar::new();
    header.set_show_close_button(true);
    add_class(&header, "app-titlebar");
    let bar_title = mk_label("AK820 Screen Studio", "titlebar-title");
    bar_title.set_halign(gtk::Align::Center);
    header.set_custom_title(Some(&bar_title));
    window.set_titlebar(Some(&header));

    let page = Box::new(Orientation::Vertical, 16);
    page.set_margin_start(16);
    page.set_margin_end(16);
    page.set_margin_top(20);
    page.set_margin_bottom(24);
    window.add(&page);

    // ---------- header ----------
    let brand = Box::new(Orientation::Vertical, 2);
    brand.set_valign(gtk::Align::Center);
    let title = mk_label("AK820 Screen Studio", "brand-title");
    add_class(&title, "brand-title");
    let sub = Label::new(None);
    sub.set_xalign(0.0);
    sub.set_markup(
        "Put GIFs, video, text, \
         <span foreground='#ff3b3b'>r</span><span foreground='#ff9f1c'>a</span>\
         <span foreground='#ffe11c'>i</span><span foreground='#3ddc4a'>n</span>\
         <span foreground='#1cb0ff'>b</span><span foreground='#8a4dff'>o</span>\
         <span foreground='#ff3bd4'>w</span><span foreground='#ff3b3b'>s</span> \
         and drawings on the AK820 Pro's screen. No driver, no browser, no account.",
    );
    add_class(&sub, "brand-sub");
    brand.pack_start(&title, false, false, 0);
    brand.pack_start(&sub, false, false, 0);

    let pill_box = Box::new(Orientation::Horizontal, 8);
    add_class(&pill_box, "pill");
    pill_box.set_valign(gtk::Align::Center);
    let pill_dot = mk_label("●", "pill-dot");
    let pill = mk_label("Not connected", "pill-text");
    pill_box.pack_start(&pill_dot, false, false, 0);
    pill_box.pack_start(&pill, false, false, 0);

    let connect_btn = Button::with_label("Connect keyboard");
    add_class(&connect_btn, "primary");
    add_class(&connect_btn, "connect");
    let settings_btn = Button::with_label("⚙");
    add_class(&settings_btn, "icon");
    settings_btn.set_tooltip_text(Some("Settings"));

    let conn = Box::new(Orientation::Horizontal, 12);
    conn.set_valign(gtk::Align::Center);
    conn.pack_start(&pill_box, false, false, 0);
    conn.pack_start(&connect_btn, false, false, 0);
    conn.pack_start(&settings_btn, false, false, 0);

    let header_row = Box::new(Orientation::Horizontal, 16);
    header_row.set_halign(gtk::Align::Center);
    header_row.pack_start(&brand, true, true, 0);
    header_row.pack_end(&conn, false, false, 0);
    page.pack_start(&header_row, false, false, 0);

    // ---------- sidebar ----------
    let sidebar = Box::new(Orientation::Vertical, 16);
    sidebar.set_size_request(340, -1);
    sidebar.set_valign(gtk::Align::Start);

    let (preview_card, preview_in) = card(18, 0);
    preview_card.set_hexpand(false);

    let kb = Box::new(Orientation::Vertical, 0);
    add_class(&kb, "kb");
    let kb_in = Box::new(Orientation::Vertical, 0);
    kb_in.set_margin_start(18);
    kb_in.set_margin_end(18);
    kb_in.set_margin_top(18);
    kb_in.set_margin_bottom(18);
    kb.pack_start(&kb_in, true, true, 0);
    let bezel = Box::new(Orientation::Vertical, 0);
    add_class(&bezel, "bezel");
    bezel.set_halign(gtk::Align::Center);
    let preview = Image::new();
    preview.set_size_request(PREVIEW_PX, PREVIEW_PX);
    preview.set_margin_start(12);
    preview.set_margin_end(12);
    preview.set_margin_top(12);
    preview.set_margin_bottom(12);
    bezel.pack_start(&preview, false, false, 0);
    kb_in.pack_start(&bezel, false, false, 0);

    let meta = Box::new(Orientation::Horizontal, 8);
    meta.set_margin_top(12);
    let frames_info = mk_label("No image yet", "muted");
    let size_info = Label::new(Some(""));
    size_info.set_xalign(1.0);
    add_class(&size_info, "muted");
    meta.pack_start(&frames_info, true, true, 0);
    meta.pack_end(&size_info, false, false, 0);

    let upload_btn = Button::with_label("Send to keyboard");
    add_class(&upload_btn, "primary");
    add_class(&upload_btn, "big");
    upload_btn.set_sensitive(false);
    let save_hist = Button::with_label("Save to history");
    save_hist.set_sensitive(false);
    let upload_progress = ProgressBar::new();
    upload_progress.set_show_text(false);
    upload_progress.set_no_show_all(true);
    let cancel_btn = Button::with_label("Cancel");
    cancel_btn.set_no_show_all(true);
    let msg = mk_label("", "msg");
    msg.set_line_wrap(true);

    let actions = Box::new(Orientation::Vertical, 10);
    actions.set_margin_top(14);
    actions.pack_start(&upload_btn, false, false, 0);
    actions.pack_start(&save_hist, false, false, 0);
    actions.pack_start(&upload_progress, false, false, 0);
    actions.pack_start(&cancel_btn, false, false, 0);
    actions.pack_start(&msg, false, false, 0);

    preview_in.pack_start(&kb, false, false, 0);
    preview_in.pack_start(&meta, false, false, 0);
    preview_in.pack_start(&actions, false, false, 0);

    // device card (hidden until connected)
    let (dev_panel, dev_in) = card(18, 10);
    dev_panel.set_no_show_all(true);
    dev_in.pack_start(&heading("Keyboard"), false, false, 0);
    let dev_grid = Grid::new();
    dev_grid.set_column_spacing(14);
    dev_grid.set_row_spacing(3);
    let dev_vid_pid = mk_label("vid/pid: —", "muted");
    let dev_firmware = mk_label("firmware: —", "muted");
    let dev_mode = mk_label("mode: —", "muted");
    let dev_battery = mk_label("battery: —", "muted");
    let dev_charging = mk_label("charging: —", "muted");
    let dev_profile = mk_label("profile: —", "muted");
    let dev_frames = mk_label("max frames: —", "muted");
    let dev_direction = mk_label("screen: —", "muted");
    dev_grid.attach(&dev_vid_pid, 0, 0, 1, 1);
    dev_grid.attach(&dev_firmware, 0, 1, 1, 1);
    dev_grid.attach(&dev_mode, 0, 2, 1, 1);
    dev_grid.attach(&dev_battery, 0, 3, 1, 1);
    dev_grid.attach(&dev_charging, 1, 0, 1, 1);
    dev_grid.attach(&dev_profile, 1, 1, 1, 1);
    dev_grid.attach(&dev_frames, 1, 2, 1, 1);
    dev_grid.attach(&dev_direction, 1, 3, 1, 1);
    dev_in.pack_start(&dev_grid, false, false, 0);

    let sync_btn = Button::with_label("Sync clock now");
    let builtin_combo = ComboBoxText::new();
    for n in 1..=3 {
        builtin_combo.append_text(&format!("Built-in {n}"));
    }
    builtin_combo.set_active(Some(0));
    let builtin_btn = Button::with_label("Show");
    let dev_row = Box::new(Orientation::Horizontal, 8);
    dev_row.set_margin_top(6);
    dev_row.pack_start(&sync_btn, false, false, 0);
    dev_row.pack_start(&builtin_combo, false, false, 0);
    dev_row.pack_start(&builtin_btn, false, false, 0);
    dev_in.pack_start(&dev_row, false, false, 0);
    let sync_status = mk_label("", "faint");
    let builtin_status = mk_label("", "faint");
    dev_in.pack_start(&sync_status, false, false, 0);
    dev_in.pack_start(&builtin_status, false, false, 0);

    sidebar.pack_start(&preview_card, false, false, 0);
    sidebar.pack_start(&dev_panel, false, false, 0);

    // ---------- main column ----------
    let main_col = Box::new(Orientation::Vertical, 16);
    main_col.set_hexpand(true);

    // tabs
    let (tabs_card, tabs_in) = card(18, 16);
    let tabs_outer = Box::new(Orientation::Horizontal, 0);
    add_class(&tabs_outer, "tabs");
    let tabs_row = Box::new(Orientation::Horizontal, 4);
    tabs_row.set_homogeneous(true);
    tabs_row.set_margin_start(4);
    tabs_row.set_margin_end(4);
    tabs_row.set_margin_top(4);
    tabs_row.set_margin_bottom(4);
    tabs_outer.pack_start(&tabs_row, true, true, 0);
    let stack = Stack::new();
    stack.set_hexpand(true);
    stack.set_hhomogeneous(false);
    stack.set_vhomogeneous(false);
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(120);
    let mut tab_buttons: Vec<(String, ToggleButton)> = Vec::new();
    for (i, (name, label)) in [
        ("anim", "Animations"),
        ("file", "Image / Video"),
        ("text", "Text"),
        ("draw", "Draw"),
    ]
    .iter()
    .enumerate()
    {
        let b = ToggleButton::with_label(label);
        add_class(&b, "tab");
        b.set_active(i == 0);
        tabs_row.pack_start(&b, true, true, 0);
        tab_buttons.push(((*name).to_string(), b));
    }
    for (i, (name, button)) in tab_buttons.iter().enumerate() {
        let stack = stack.clone();
        let name = name.clone();
        let others: Vec<ToggleButton> = tab_buttons
            .iter()
            .filter(|(_, b)| b != button)
            .map(|(_, b)| b.clone())
            .collect();
        button.connect_toggled(move |b| {
            if b.get_active() {
                stack.set_visible_child_name(&name);
                for o in &others {
                    if o.get_active() {
                        o.set_active(false);
                    }
                }
            }
            let _ = i;
        });
    }
    tabs_in.pack_start(&tabs_outer, false, false, 0);
    tabs_in.pack_start(&stack, true, true, 0);

    // ---------- animations page ----------
    let anim_page = Box::new(Orientation::Vertical, 16);
    let gen_grid = FlowBox::new();
    gen_grid.set_max_children_per_line(5);
    gen_grid.set_min_children_per_line(5);
    gen_grid.set_selection_mode(gtk::SelectionMode::None);
    gen_grid.set_column_spacing(10);
    gen_grid.set_row_spacing(10);
    gen_grid.set_homogeneous(true);
    gen_grid.set_halign(gtk::Align::Fill);
    gen_grid.set_hexpand(true);
    let gen_param_box = Box::new(Orientation::Vertical, 0);
    let (smooth_ctl, gframes_scale, gframes_out) = scale_ctl("Smoothness", 12.0, 96.0, 4.0, 36.0);
    gframes_out.set_text("36 frames");
    let shuffle_btn = Button::with_label("Shuffle");
    let surprise_btn = Button::with_label("🎲 Surprise me");
    let anim_buttons = Box::new(Orientation::Horizontal, 8);
    anim_buttons.pack_start(&shuffle_btn, false, false, 0);
    anim_buttons.pack_start(&surprise_btn, false, false, 0);
    let anim_buttons_ctl = Box::new(Orientation::Vertical, 4);
    anim_buttons_ctl.set_valign(gtk::Align::End);
    anim_buttons_ctl.pack_start(&mk_label(" ", "ctl-label"), false, false, 0);
    anim_buttons_ctl.pack_start(&anim_buttons, false, false, 0);
    let anim_bottom = Grid::new();
    anim_bottom.set_column_spacing(20);
    anim_bottom.set_column_homogeneous(true);
    anim_bottom.attach(&smooth_ctl, 0, 0, 1, 1);
    anim_bottom.attach(&anim_buttons_ctl, 1, 0, 1, 1);
    anim_page.pack_start(&gen_grid, false, false, 0);
    anim_page.pack_start(&gen_param_box, false, false, 0);
    anim_page.pack_start(&anim_bottom, false, false, 0);
    stack.add_named(&anim_page, "anim");

    // ---------- image / video page ----------
    let file_page = Box::new(Orientation::Vertical, 14);
    let file_btn = Button::new();
    add_class(&file_btn, "drop");
    file_btn.set_size_request(-1, 92);
    let drop_in = Box::new(Orientation::Vertical, 4);
    drop_in.set_halign(gtk::Align::Center);
    let drop_title = Label::new(None);
    drop_title.set_markup("<b>Drop a file here, paste, or click to choose</b>");
    let drop_sub = mk_label("GIF, PNG, JPG, WebP, APNG, MP4, WebM", "muted");
    drop_sub.set_halign(gtk::Align::Center);
    drop_in.pack_start(&drop_title, false, false, 0);
    drop_in.pack_start(&drop_sub, false, false, 0);
    file_btn.add(&drop_in);
    file_page.pack_start(&file_btn, false, false, 0);

    let giphy_head = Box::new(Orientation::Horizontal, 8);
    let giphy_entry = Entry::new();
    giphy_entry.set_placeholder_text(Some("Search GIPHY…"));
    giphy_entry.set_hexpand(true);
    let giphy_rating = ComboBoxText::new();
    for (id, label) in [("g", "G"), ("pg", "PG"), ("pg-13", "PG-13"), ("r", "R")] {
        giphy_rating.append(Some(id), label);
    }
    giphy_rating.set_active_id(Some("r"));
    let giphy_search_btn = Button::with_label("Search");
    giphy_head.pack_start(&giphy_entry, true, true, 0);
    giphy_head.pack_start(&giphy_rating, false, false, 0);
    giphy_head.pack_start(&giphy_search_btn, false, false, 0);
    file_page.pack_start(&giphy_head, false, false, 0);

    let giphy_status = mk_label("", "faint");
    giphy_status.set_line_wrap(true);
    file_page.pack_start(&giphy_status, false, false, 0);

    let giphy_box = FlowBox::new();
    giphy_box.set_max_children_per_line(8);
    giphy_box.set_min_children_per_line(4);
    giphy_box.set_selection_mode(gtk::SelectionMode::None);
    giphy_box.set_column_spacing(6);
    giphy_box.set_row_spacing(6);
    giphy_box.set_homogeneous(true);
    giphy_box.set_halign(gtk::Align::Fill);
    giphy_box.set_hexpand(true);
    let giphy_scroll = ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    giphy_scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    giphy_scroll.set_min_content_height(220);
    giphy_scroll.set_no_show_all(true);
    giphy_scroll.add(&giphy_box);
    file_page.pack_start(&giphy_scroll, false, false, 0);

    let giphy_more_btn = Button::with_label("More");
    giphy_more_btn.set_halign(gtk::Align::Start);
    giphy_more_btn.set_no_show_all(true);
    file_page.pack_start(&giphy_more_btn, false, false, 0);
    stack.add_named(&file_page, "file");

    // ---------- text page ----------
    let text_page = Box::new(Orientation::Vertical, 0);
    let text_grid = Grid::new();
    text_grid.set_column_spacing(20);
    text_grid.set_row_spacing(14);
    text_grid.set_column_homogeneous(true);
    let txt_label = mk_label("Text", "ctl-label");
    text_grid.attach(&txt_label, 0, 0, 2, 1);
    let txt_view = TextView::new();
    txt_view.set_wrap_mode(gtk::WrapMode::WordChar);
    txt_view.set_size_request(-1, 72);
    txt_view
        .get_buffer()
        .expect("text view buffer")
        .set_text("HELLO");
    text_grid.attach(&txt_view, 0, 1, 2, 1);

    let (tmode_box, tmode_buttons) = seg_box(
        "Style",
        &[
            ("static", "Static"),
            ("scroll", "Scroll"),
            ("wave", "Wave"),
            ("type", "Typewriter"),
            ("blink", "Blink"),
        ],
        "static",
    );
    text_grid.attach(&tmode_box, 0, 2, 2, 1);
    let (tcol_box, tcol_buttons) = seg_box(
        "Colour",
        &[("solid", "Solid"), ("rainbow", "Rainbow")],
        "solid",
    );
    text_grid.attach(&tcol_box, 0, 3, 1, 1);
    let (tsize_box, tsize_scale, tsize_out) = scale_ctl("Max size", 10.0, 96.0, 1.0, 40.0);
    text_grid.attach(&tsize_box, 1, 3, 1, 1);

    let colors_box = Box::new(Orientation::Vertical, 4);
    add_class(&colors_box, "ctl");
    colors_box.pack_start(&mk_label("Text / background", "ctl-label"), false, false, 0);
    let colors_row = Box::new(Orientation::Horizontal, 8);
    let tfg_btn = gtk::ColorButton::new();
    let tbg_btn = gtk::ColorButton::new();
    let toutline_btn = ToggleButton::with_label("Outline");
    add_class(&toutline_btn, "chip");
    colors_row.pack_start(&tfg_btn, false, false, 0);
    colors_row.pack_start(&tbg_btn, false, false, 0);
    colors_row.pack_start(&toutline_btn, false, false, 0);
    colors_box.pack_start(&colors_row, false, false, 0);
    text_grid.attach(&colors_box, 0, 4, 1, 1);
    let font_box = Box::new(Orientation::Vertical, 4);
    add_class(&font_box, "ctl");
    font_box.pack_start(&mk_label("Font", "ctl-label"), false, false, 0);
    let tfont_combo = ComboBoxText::new();
    for (id, label) in [
        ("sans", "Sans"),
        ("Comfortaa", "Rounded"),
        ("serif", "Serif"),
        ("monospace", "Mono"),
        ("Liberation Sans Narrow", "Impact"),
        ("cursive", "Comic"),
    ] {
        tfont_combo.append(Some(id), label);
    }
    tfont_combo.set_active_id(Some("sans"));
    font_box.pack_start(&tfont_combo, false, false, 0);
    text_grid.attach(&font_box, 1, 4, 1, 1);
    let (tspeed_box, tspeed_scale, tspeed_out) = scale_ctl("Speed", 1.0, 8.0, 1.0, 3.0);
    text_grid.attach(&tspeed_box, 0, 5, 1, 1);
    text_page.pack_start(&text_grid, false, false, 0);
    stack.add_named(&text_page, "text");

    // ---------- draw page ----------
    let draw_page = Box::new(Orientation::Vertical, 0);
    let draw_layout = Box::new(Orientation::Horizontal, 18);
    let draw_area = EventBox::new();
    add_class(&draw_area, "canvas-wrap");
    draw_area.set_size_request(CANVAS_PX + 2, CANVAS_PX + 2);
    draw_area.set_visible_window(true);
    draw_area.set_events(
        gdk::EventMask::BUTTON1_MOTION_MASK
            | gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::BUTTON_RELEASE_MASK,
    );
    let draw_canvas = Image::new();
    draw_canvas.set_size_request(CANVAS_PX, CANVAS_PX);
    draw_area.add(&draw_canvas);
    draw_layout.pack_start(&draw_area, false, false, 0);

    let tools = Box::new(Orientation::Vertical, 12);
    tools.set_hexpand(true);
    let (tool_box, tool_buttons) =
        seg_box("Tool", &[("brush", "Brush"), ("erase", "Eraser")], "brush");
    tools.pack_start(&tool_box, false, false, 0);
    let (bsize_box, bsize_scale, bsize_out) = scale_ctl("Brush size", 1.0, 24.0, 1.0, 4.0);
    tools.pack_start(&bsize_box, false, false, 0);
    let bcol_box = Box::new(Orientation::Vertical, 4);
    add_class(&bcol_box, "ctl");
    bcol_box.pack_start(&mk_label("Colour", "ctl-label"), false, false, 0);
    let bcol_row = Box::new(Orientation::Horizontal, 8);
    let bcolor_btn = gtk::ColorButton::new();
    let brainbow_btn = ToggleButton::with_label("Rainbow brush");
    add_class(&brainbow_btn, "chip");
    bcol_row.pack_start(&bcolor_btn, false, false, 0);
    bcol_row.pack_start(&brainbow_btn, false, false, 0);
    bcol_box.pack_start(&bcol_row, false, false, 0);
    tools.pack_start(&bcol_box, false, false, 0);
    let dbg_box = Box::new(Orientation::Vertical, 4);
    add_class(&dbg_box, "ctl");
    dbg_box.pack_start(&mk_label("Background", "ctl-label"), false, false, 0);
    let dbg_row = Box::new(Orientation::Horizontal, 8);
    let dbg_btn = gtk::ColorButton::new();
    let dclear_btn = Button::with_label("Clear");
    dbg_row.pack_start(&dbg_btn, false, false, 0);
    dbg_row.pack_start(&dclear_btn, false, false, 0);
    dbg_box.pack_start(&dbg_row, false, false, 0);
    tools.pack_start(&dbg_box, false, false, 0);
    let drow = Box::new(Orientation::Horizontal, 8);
    let dundo_btn = Button::with_label("Undo");
    let dtrace_btn = Button::with_label("Start from last image");
    drow.pack_start(&dundo_btn, false, false, 0);
    drow.pack_start(&dtrace_btn, false, false, 0);
    tools.pack_start(&drow, false, false, 0);
    draw_layout.pack_start(&tools, true, true, 0);
    draw_page.pack_start(&draw_layout, false, false, 0);
    stack.add_named(&draw_page, "draw");

    main_col.pack_start(&tabs_card, false, false, 0);

    // ---------- effects card ----------
    let (fx_card, fx_in) = card(18, 12);
    let fx_head = Box::new(Orientation::Horizontal, 8);
    fx_head.pack_start(&heading("Effects"), true, true, 0);
    let fx_random_btn = Button::with_label("🎲 Random");
    add_class(&fx_random_btn, "tiny");
    let fx_clear_btn = Button::with_label("Clear");
    add_class(&fx_clear_btn, "tiny");
    fx_head.pack_end(&fx_clear_btn, false, false, 0);
    fx_head.pack_end(&fx_random_btn, false, false, 0);
    fx_in.pack_start(&fx_head, false, false, 0);

    let fx_state = app.fx.borrow().clone();
    let mut fx_chips: Vec<(String, ToggleButton)> = Vec::new();
    let fx_groups: [(&str, &[(&str, &str)]); 3] = [
        (
            "Rainbow",
            &[
                ("cycle", "Colour cycle"),
                ("wash", "Rainbow wash"),
                ("border", "Rainbow border"),
                ("capRainbow", "Rainbow caption"),
            ],
        ),
        (
            "Motion",
            &[
                ("spin", "Spin"),
                ("shake", "Shake"),
                ("breathe", "Zoom pulse"),
                ("pulse", "Glow pulse"),
                ("glitch", "Glitch"),
                ("sparkle", "Sparkles"),
            ],
        ),
        (
            "Look",
            &[
                ("scan", "CRT lines"),
                ("vignette", "Vignette"),
                ("posterize", "Poster"),
                ("gray", "Black & white"),
                ("sepia", "Sepia"),
                ("invert", "Invert"),
            ],
        ),
    ];
    for (group, items) in fx_groups {
        let row = Box::new(Orientation::Horizontal, 10);
        let gl = mk_label(group, "glabel");
        gl.set_size_request(82, -1);
        gl.set_yalign(0.6);
        let (chips, buttons) = chip_row(items);
        for (key, button) in buttons {
            let active = match key.as_str() {
                "cycle" => fx_state.cycle,
                "wash" => fx_state.wash,
                "border" => fx_state.border,
                "capRainbow" => fx_state.cap_rainbow,
                "spin" => fx_state.spin > 0,
                "shake" => fx_state.shake,
                "breathe" => fx_state.breathe,
                "pulse" => fx_state.pulse,
                "glitch" => fx_state.glitch,
                "sparkle" => fx_state.sparkle,
                "scan" => fx_state.scan,
                "vignette" => fx_state.vignette,
                "posterize" => fx_state.posterize,
                "gray" => fx_state.gray,
                "sepia" => fx_state.sepia,
                "invert" => fx_state.invert,
                _ => false,
            };
            button.set_active(active);
            fx_chips.push((key, button));
        }
        row.pack_start(&gl, false, false, 0);
        row.pack_start(&chips, false, false, 0);
        fx_in.pack_start(&row, false, false, 0);
    }

    let fx_controls = Grid::new();
    fx_controls.set_column_spacing(20);
    fx_controls.set_row_spacing(12);
    fx_controls.set_column_homogeneous(true);
    fx_controls.set_margin_top(6);
    let (hue_box, fx_hue_scale, fx_hue_out) = scale_ctl("Hue shift", 0.0, 355.0, 5.0, 0.0);
    fx_hue_out.set_text("0°");
    let (pixel_box, fx_pixel_scale, fx_pixel_out) = scale_ctl("Pixelate", 1.0, 16.0, 1.0, 1.0);
    fx_pixel_out.set_text("off");
    fx_controls.attach(&hue_box, 0, 0, 1, 1);
    fx_controls.attach(&pixel_box, 1, 0, 1, 1);
    let cap_box = Box::new(Orientation::Vertical, 4);
    add_class(&cap_box, "ctl");
    cap_box.pack_start(&mk_label("Caption", "ctl-label"), false, false, 0);
    let fx_cap_entry = Entry::new();
    fx_cap_entry.set_placeholder_text(Some("Optional text on top of anything"));
    cap_box.pack_start(&fx_cap_entry, false, false, 0);
    fx_controls.attach(&cap_box, 0, 1, 2, 1);
    let (cappos_box, fx_cappos_buttons) = seg_box(
        "Caption position",
        &[("top", "Top"), ("middle", "Middle"), ("bottom", "Bottom")],
        "bottom",
    );
    fx_controls.attach(&cappos_box, 0, 2, 1, 1);
    let capcol_box = Box::new(Orientation::Vertical, 4);
    add_class(&capcol_box, "ctl");
    capcol_box.pack_start(&mk_label("Caption colour", "ctl-label"), false, false, 0);
    let fx_capcolor_btn = gtk::ColorButton::new();
    fx_capcolor_btn.set_halign(gtk::Align::Start);
    capcol_box.pack_start(&fx_capcolor_btn, false, false, 0);
    fx_controls.attach(&capcol_box, 1, 2, 1, 1);
    let (loop_box, fx_loop_scale, fx_loop_out) =
        scale_ctl("Loop length for still images", 8.0, 72.0, 4.0, 24.0);
    fx_loop_out.set_text("24 frames");
    fx_controls.attach(&loop_box, 0, 3, 2, 1);
    fx_in.pack_start(&fx_controls, false, false, 0);
    main_col.pack_start(&fx_card, false, false, 0);

    // ---------- adjust card ----------
    let (adj_card, adj_in) = card(18, 12);
    let adj_head = Box::new(Orientation::Horizontal, 8);
    adj_head.pack_start(&heading("Adjust"), true, true, 0);
    let adjust_reset_btn = Button::with_label("Reset");
    add_class(&adjust_reset_btn, "tiny");
    adj_head.pack_end(&adjust_reset_btn, false, false, 0);
    adj_in.pack_start(&adj_head, false, false, 0);
    let adj_grid = Grid::new();
    adj_grid.set_column_spacing(20);
    adj_grid.set_row_spacing(12);
    adj_grid.set_column_homogeneous(true);
    adj_grid.set_margin_top(6);
    let (fit_box, fit_buttons) = seg_box(
        "Fit",
        &[
            ("cover", "Fill"),
            ("contain", "Fit"),
            ("stretch", "Stretch"),
        ],
        "cover",
    );
    let (rot_box, rot_buttons) = seg_box(
        "Rotate",
        &[("0", "0°"), ("90", "90°"), ("180", "180°"), ("270", "270°")],
        "0",
    );
    adj_grid.attach(&fit_box, 0, 0, 1, 1);
    adj_grid.attach(&rot_box, 1, 0, 1, 1);
    let (zoom_box, zoom_scale, zoom_out) = scale_ctl("Zoom", 100.0, 400.0, 1.0, 100.0);
    zoom_out.set_text("100%");
    adj_grid.attach(&zoom_box, 0, 1, 1, 1);
    let bg_box = Box::new(Orientation::Vertical, 4);
    add_class(&bg_box, "ctl");
    bg_box.pack_start(&mk_label("Background", "ctl-label"), false, false, 0);
    let bg_row = Box::new(Orientation::Horizontal, 8);
    let bg_btn = gtk::ColorButton::new();
    let flip_btn = ToggleButton::with_label("Mirror");
    add_class(&flip_btn, "chip");
    bg_row.pack_start(&bg_btn, false, false, 0);
    bg_row.pack_start(&flip_btn, false, false, 0);
    bg_box.pack_start(&bg_row, false, false, 0);
    adj_grid.attach(&bg_box, 1, 1, 1, 1);
    let (panx_box, panx_scale, panx_out) = scale_ctl("Pan left / right", -100.0, 100.0, 1.0, 0.0);
    panx_out.set_text("0");
    adj_grid.attach(&panx_box, 0, 2, 1, 1);
    let (pany_box, pany_scale, pany_out) = scale_ctl("Pan up / down", -100.0, 100.0, 1.0, 0.0);
    pany_out.set_text("0");
    adj_grid.attach(&pany_box, 1, 2, 1, 1);
    let (bri_box, bri_scale, bri_out) = scale_ctl("Brightness", 20.0, 200.0, 1.0, 100.0);
    bri_out.set_text("100%");
    adj_grid.attach(&bri_box, 0, 3, 1, 1);
    let (con_box, con_scale, con_out) = scale_ctl("Contrast", 20.0, 200.0, 1.0, 100.0);
    con_out.set_text("100%");
    adj_grid.attach(&con_box, 1, 3, 1, 1);
    let (sat_box, sat_scale, sat_out) = scale_ctl("Saturation", 0.0, 250.0, 1.0, 100.0);
    sat_out.set_text("100%");
    adj_grid.attach(&sat_box, 0, 4, 1, 1);
    let (speed_box, speed_scale, speed_out) = scale_ctl("Speed", -3.0, 3.0, 1.0, 0.0);
    speed_out.set_text("1×");
    adj_grid.attach(&speed_box, 1, 4, 1, 1);
    let (tstart_box, trim_start, tstart_out) = scale_ctl("First frame", 0.0, 255.0, 1.0, 0.0);
    tstart_out.set_text("1");
    adj_grid.attach(&tstart_box, 0, 5, 1, 1);
    let (tend_box, trim_end, tend_out) = scale_ctl("Last frame", 0.0, 255.0, 1.0, 0.0);
    tend_out.set_text("1");
    adj_grid.attach(&tend_box, 1, 5, 1, 1);
    let (skip_box, skip_scale, skip_out) = scale_ctl("Keep", 1.0, 6.0, 1.0, 1.0);
    skip_out.set_text("all frames");
    adj_grid.attach(&skip_box, 0, 6, 1, 1);
    adj_in.pack_start(&adj_grid, false, false, 0);
    main_col.pack_start(&adj_card, false, false, 0);

    // ---------- history card ----------
    let (hist_card, hist_in) = card(18, 10);
    let hist_head = Box::new(Orientation::Horizontal, 8);
    hist_head.pack_start(&heading("History"), true, true, 0);
    let hist_clear_btn = Button::with_label("Clear all");
    add_class(&hist_clear_btn, "tiny");
    hist_head.pack_end(&hist_clear_btn, false, false, 0);
    hist_in.pack_start(&hist_head, false, false, 0);
    let hint = mk_label(
        "Everything you send is saved here, so you can see what's on the keyboard, what was there before, and put any of it back.",
        "faint",
    );
    hint.set_line_wrap(true);
    hist_in.pack_start(&hint, false, false, 0);
    let history_box = FlowBox::new();
    history_box.set_max_children_per_line(6);
    history_box.set_min_children_per_line(1);
    history_box.set_selection_mode(gtk::SelectionMode::None);
    history_box.set_column_spacing(12);
    history_box.set_row_spacing(12);
    history_box.set_homogeneous(false);
    history_box.set_margin_top(6);
    history_box.set_halign(gtk::Align::Fill);
    history_box.set_hexpand(true);
    hist_in.pack_start(&history_box, false, false, 0);
    main_col.pack_start(&hist_card, false, false, 0);

    let footer_text = format!(
        "Unofficial app, not affiliated with AJAZZ or Epomaker. Works with the AK820 Pro (USB 0C45:800A / 8009). · v{}",
        env!("CARGO_PKG_VERSION")
    );
    let footer = mk_label(&footer_text, "faint");
    footer.set_halign(gtk::Align::Center);
    footer.set_xalign(0.0);
    footer.set_line_wrap(true);

    let main_scroll = ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    main_scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    main_scroll.set_hexpand(true);
    main_scroll.set_vexpand(true);
    main_scroll.add(&main_col);

    let content = Box::new(Orientation::Horizontal, 16);
    content.set_halign(gtk::Align::Center);
    content.set_vexpand(true);
    content.pack_start(&sidebar, false, false, 0);
    content.pack_start(&main_scroll, true, true, 0);
    page.pack_start(&content, true, true, 0);
    page.pack_start(&footer, false, false, 0);

    // Keep the whole layout readable on wide screens: centre a bounded column
    // instead of stretching every card across the monitor.
    {
        let header_row = header_row.clone();
        let content = content.clone();
        let footer = footer.clone();
        window.connect_size_allocate(move |_, alloc| {
            let target = (alloc.width - 32).clamp(720, 1360);
            header_row.set_size_request(target, -1);
            content.set_size_request(target, -1);
            footer.set_size_request(target, -1);
        });
    }

    Widgets {
        window: window.clone(),
        preview,
        draw_canvas,
        draw_area,
        msg,
        frames_info,
        size_info,
        pill,
        pill_dot,
        connect_btn,
        settings_btn,
        dev_panel,
        dev_vid_pid,
        dev_firmware,
        dev_mode,
        dev_battery,
        dev_charging,
        dev_profile,
        dev_frames,
        dev_direction,
        sync_btn,
        sync_status,
        builtin_combo,
        builtin_btn,
        builtin_status,
        tab_buttons: RefCell::new(tab_buttons),
        file_btn,
        upload_btn,
        save_hist,
        upload_progress,
        cancel_btn,
        giphy_entry,
        giphy_rating,
        giphy_search_btn,
        giphy_more_btn,
        giphy_box,
        giphy_scroll,
        giphy_status,
        giphy_query: RefCell::new(String::new()),
        giphy_offset: RefCell::new(0),
        gen_grid,
        gen_param_box,
        gen_buttons: RefCell::new(Vec::new()),
        gframes_scale,
        gframes_out,
        shuffle_btn,
        txt_view,
        tmode_buttons: RefCell::new(tmode_buttons),
        tcol_buttons: RefCell::new(tcol_buttons),
        tsize_scale,
        tsize_out,
        tfg_btn,
        tbg_btn,
        toutline_btn,
        tfont_combo,
        tspeed_scale,
        tspeed_out,
        tool_buttons: RefCell::new(tool_buttons),
        bsize_scale,
        bsize_out,
        bcolor_btn,
        brainbow_btn,
        dbg_btn,
        dclear_btn,
        dundo_btn,
        dtrace_btn,
        fx_chips: RefCell::new(fx_chips),
        fx_hue_scale,
        fx_hue_out,
        fx_pixel_scale,
        fx_pixel_out,
        fx_cap_entry,
        fx_cappos_buttons: RefCell::new(fx_cappos_buttons),
        fx_capcolor_btn,
        fx_loop_scale,
        fx_loop_out,
        fit_buttons: RefCell::new(fit_buttons),
        rot_buttons: RefCell::new(rot_buttons),
        zoom_scale,
        zoom_out,
        bg_btn,
        flip_btn,
        panx_scale,
        panx_out,
        pany_scale,
        pany_out,
        bri_scale,
        bri_out,
        con_scale,
        con_out,
        sat_scale,
        sat_out,
        speed_scale,
        speed_out,
        trim_start,
        trim_end,
        skip_scale,
        history_box,
        hist_clear_btn,
        app: Rc::clone(app),
        surprise_btn,
        fx_random_btn,
        fx_clear_btn,
        adjust_reset_btn,
        tstart_out,
        tend_out,
        skip_out,
    }
}

fn set_msg(w: &Widgets, text: &str, class: &str) {
    w.msg.set_text(text);
    let ctx = w.msg.get_style_context();
    ctx.remove_class("ok");
    ctx.remove_class("err");
    if !class.is_empty() {
        ctx.add_class(class);
    }
}

fn set_pill(w: &Widgets, state: &str, text: &str) {
    let ctx = w.pill_dot.get_style_context();
    ctx.remove_class("ok");
    ctx.remove_class("err");
    ctx.remove_class("busy");
    match state {
        "ok" => ctx.add_class("ok"),
        "err" => ctx.add_class("err"),
        "busy" => ctx.add_class("busy"),
        _ => {}
    }
    w.pill.set_text(text);
}

// ---------- background clock sync, hotplug and tray ----------

type SyncDone = std::boxed::Box<dyn FnOnce(Result<String, String>)>;

/// Push the local clock, retrying while another AK820 process holds the lock.
fn sync_time() -> Result<String, String> {
    let now = chrono::Local::now();
    let time = CivilTime {
        year: now.year() as u16,
        month: now.month() as u8,
        day: now.day() as u8,
        hour: now.hour() as u8,
        minute: now.minute() as u8,
        second: now.second() as u8,
        weekday: now.weekday().num_days_from_sunday() as u8,
    };
    let mut last = "the keyboard stayed busy".to_string();
    for attempt in 0..6 {
        match DeviceLock::acquire() {
            Ok(_lock) => {
                let mut dev = Device::open().map_err(|e| e.to_string())?;
                if !dev.has_config() {
                    return Err(
                        "no clock interface: replug the dongle, or use the USB cable.".into(),
                    );
                }
                dev.sync_clock(time).map_err(|e| e.to_string())?;
                return Ok(format!(
                    "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                    time.year, time.month, time.day, time.hour, time.minute, time.second
                ));
            }
            Err(ak820_device::Error::Locked(_)) if attempt < 5 => {
                std::thread::sleep(Duration::from_millis(300));
                last = "the keyboard is busy: another AK820 tool is talking to it".into();
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Err(last)
}

/// Run a sync off the UI thread and report the result on the main thread.
fn spawn_clock_sync(w: &Widgets, background: bool, done: Option<SyncDone>) {
    if w.app.sync_busy.get() {
        if let Some(cb) = done {
            cb(Err(
                "the keyboard is busy: another AK820 operation is running".into(),
            ));
        }
        return;
    }
    w.app.sync_busy.set(true);
    let status = w.sync_status.clone();
    let msg = w.msg.clone();
    let visible = w.window.get_visible();
    if !background {
        status.set_text("Syncing clock…");
        if visible {
            set_msg(w, "Syncing clock…", "");
        }
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(sync_time());
    });
    let done = Rc::new(RefCell::new(done));
    let app = Rc::clone(&w.app);
    glib::timeout_add_local(100, move || match rx.try_recv() {
        Ok(Ok(stamp)) => {
            app.sync_busy.set(false);
            status.set_text(&format!("Synced {stamp}"));
            let ctx = status.get_style_context();
            ctx.remove_class("err");
            ctx.add_class("ok");
            if visible {
                msg.set_text("Clock synced to this computer.");
                let ctx = msg.get_style_context();
                ctx.remove_class("err");
                ctx.add_class("ok");
            }
            if let Some(cb) = done.borrow_mut().take() {
                cb(Ok(stamp));
            }
            Continue(false)
        }
        Ok(Err(e)) => {
            app.sync_busy.set(false);
            status.set_text(&format!("Sync failed: {e}"));
            let ctx = status.get_style_context();
            ctx.remove_class("ok");
            ctx.add_class("err");
            if visible {
                msg.set_text(&format!("Clock sync failed: {e}"));
                let ctx = msg.get_style_context();
                ctx.remove_class("ok");
                ctx.add_class("err");
            }
            if let Some(cb) = done.borrow_mut().take() {
                cb(Err(e));
            }
            Continue(false)
        }
        Err(TryRecvError::Empty) => Continue(true),
        Err(_) => {
            app.sync_busy.set(false);
            Continue(false)
        }
    });
}

fn show_window(window: &Window) {
    window.show();
    window.deiconify();
    window.present();
}

#[derive(Clone, Copy)]
enum TrayCmd {
    Open,
    Sync,
    Quit,
}

struct Tray {
    tx: mpsc::Sender<TrayCmd>,
}

/// The bundled PNG as an ARGB32 pixmap for StatusNotifierItem.
fn tray_icon_pixmap() -> Vec<ksni::Icon> {
    let Ok(image) =
        image::load_from_memory(include_bytes!("../../../packaging/linux/ak820-gtk.png"))
    else {
        return Vec::new();
    };
    let image = image
        .resize(32, 32, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let mut data = Vec::with_capacity(32 * 32 * 4);
    for pixel in image.pixels() {
        let [r, g, b, a] = pixel.0;
        // ARGB32, network byte order.
        data.extend_from_slice(&[a, r, g, b]);
    }
    vec![ksni::Icon {
        width: 32,
        height: 32,
        data,
    }]
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "ak820-gtk".into()
    }

    fn title(&self) -> String {
        "AK820 Screen Studio".into()
    }

    fn icon_name(&self) -> String {
        "ak820-gtk".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        tray_icon_pixmap()
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        let item = |label: &str, cmd: TrayCmd| -> ksni::MenuItem<Self> {
            let tx = self.tx.clone();
            StandardItem {
                label: label.into(),
                activate: std::boxed::Box::new(move |_| {
                    let _ = tx.send(cmd);
                }),
                ..Default::default()
            }
            .into()
        };
        vec![
            item("Open AK820 Screen Studio", TrayCmd::Open),
            ksni::MenuItem::Separator,
            item("Sync clock now", TrayCmd::Sync),
            ksni::MenuItem::Separator,
            item("Quit", TrayCmd::Quit),
        ]
    }
}

/// Build the StatusNotifierItem (the AppIndicator protocol) and pump its menu
/// actions on the GTK main thread. The returned handle must stay alive.
fn build_tray(w: &Widgets) -> Option<ksni::blocking::Handle<Tray>> {
    use ksni::blocking::TrayMethods;
    let (tx, rx) = mpsc::channel();
    let handle = match (Tray { tx }).spawn() {
        Ok(handle) => handle,
        Err(e) => {
            eprintln!("tray icon unavailable: {e}");
            return None;
        }
    };
    let w2 = w.clone();
    glib::timeout_add_local(200, move || {
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                TrayCmd::Open => show_window(&w2.window),
                TrayCmd::Sync => spawn_clock_sync(&w2, false, None),
                TrayCmd::Quit => gtk::main_quit(),
            }
        }
        Continue(true)
    });
    Some(handle)
}

/// Put the newest history entry back on the screen, best effort.
fn restore_latest(w: &Widgets) {
    if ak820_device::find().bulk.is_none() {
        return;
    }
    let Ok(payload) = history::load(1) else {
        return;
    };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send((|| -> Result<(), String> {
            let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
            let mut dev = Device::open().map_err(|e| e.to_string())?;
            dev.upload(&payload, &mut |_, _| {})
                .map_err(|e| e.to_string())
        })());
    });
    let visible = w.window.get_visible();
    let msg = w.msg.clone();
    glib::timeout_add_local(100, move || match rx.try_recv() {
        Ok(Ok(())) => {
            if visible {
                msg.set_text("Restored the last design.");
                let ctx = msg.get_style_context();
                ctx.remove_class("err");
                ctx.add_class("ok");
            }
            Continue(false)
        }
        Ok(Err(e)) => {
            if visible {
                msg.set_text(&format!("Could not restore the last design: {e}"));
                let ctx = msg.get_style_context();
                ctx.remove_class("ok");
                ctx.add_class("err");
            }
            Continue(false)
        }
        Err(TryRecvError::Empty) => Continue(true),
        Err(_) => Continue(false),
    });
}

/// Watch USB hotplug and the two clock timers (interval + 12 h).
fn start_watchers(w: &Widgets) {
    let w2 = w.clone();
    let last = Cell::new(Some(ak820_device::presence()));
    glib::timeout_add_seconds_local(2, move || {
        let now = ak820_device::presence();
        if last.get() == Some(now) {
            return Continue(true);
        }
        let appeared = last.get().is_some_and(|p| p.is_empty()) && !now.is_empty();
        let disappeared = last.get().is_some_and(|p| !p.is_empty()) && now.is_empty();
        last.set(Some(now));

        if disappeared {
            w2.app.connected.set(false);
            w2.sync_btn.set_sensitive(false);
            w2.dev_panel.hide();
            set_pill(&w2, "err", "Not connected");
            update_preview(&w2, &w2.app);
            if w2.window.get_visible() {
                set_msg(&w2, "Keyboard disconnected.", "");
            }
            return Continue(true);
        }
        if now.is_empty() {
            return Continue(true);
        }

        if appeared {
            let s = settings::Settings::load();
            if s.sync_on_connect {
                let w3 = w2.clone();
                glib::timeout_add_local(900, move || {
                    spawn_clock_sync(&w3, true, None);
                    Continue(false)
                });
            }
            if s.restore_on_connect {
                restore_latest(&w2);
            }
        }
        connect_device(&w2);
        Continue(true)
    });

    let w3 = w.clone();
    let interval_elapsed = Cell::new(0u64);
    let clock_elapsed = Cell::new(0u64);
    glib::timeout_add_seconds_local(5, move || {
        let s = settings::Settings::load();
        interval_elapsed.set(interval_elapsed.get() + 5);
        clock_elapsed.set(clock_elapsed.get() + 5);

        if s.sync_interval_secs < 10 {
            interval_elapsed.set(0);
        } else if interval_elapsed.get() >= s.sync_interval_secs {
            interval_elapsed.set(0);
            if !ak820_device::presence().is_empty() && !w3.app.sync_busy.get() {
                spawn_clock_sync(&w3, true, None);
            }
        }

        if !s.sync_on_start {
            clock_elapsed.set(0);
        } else if clock_elapsed.get() >= 12 * 3600 {
            clock_elapsed.set(0);
            if !ak820_device::presence().is_empty() && !w3.app.sync_busy.get() {
                spawn_clock_sync(&w3, true, None);
            }
        }
        Continue(true)
    });
}

// ---------- updates ----------

fn schedule_update_check(w: &Widgets) {
    let w2 = w.clone();
    glib::timeout_add_local(4 * 1000, move || {
        auto_check_updates(&w2);
        Continue(false)
    });
    let w3 = w.clone();
    glib::timeout_add_seconds_local(12 * 3600, move || {
        auto_check_updates(&w3);
        Continue(true)
    });
}

fn auto_check_updates(w: &Widgets) {
    if settings::Settings::load().check_updates {
        check_updates(w, false, None);
    }
}

fn check_updates(w: &Widgets, manual: bool, status: Option<Label>) {
    let s = settings::Settings::load();
    if !manual && !s.check_updates {
        return;
    }
    if !manual {
        if let Some(last) = w.app.last_update_check.get() {
            if last.elapsed() < Duration::from_secs(12 * 3600) {
                return;
            }
        }
    }
    w.app.last_update_check.set(Some(Instant::now()));
    if let Some(label) = &status {
        label.set_text("Checking GitHub…");
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(net::releases());
    });
    let w2 = w.clone();
    let status = status.clone();
    glib::timeout_add_local(150, move || match rx.try_recv() {
        Ok(Ok(releases)) => {
            let current = env!("CARGO_PKG_VERSION");
            let found = releases
                .into_iter()
                .find(|r| update::newer_version(&r.tag_name, current).is_gt());
            match found {
                Some(release) => {
                    if let Some(label) = &status {
                        label.set_text("");
                    }
                    let skipped = settings::Settings::load().skipped_update;
                    if manual || skipped != release.tag_name {
                        show_update_dialog(&w2, release);
                    }
                }
                None => {
                    if let Some(label) = &status {
                        label.set_text(&format!("You are up to date (v{current})."));
                    }
                }
            }
            Continue(false)
        }
        Ok(Err(e)) => {
            if let Some(label) = &status {
                label.set_text(&format!("Could not check for updates: {e}"));
            }
            Continue(false)
        }
        Err(TryRecvError::Empty) => Continue(true),
        Err(_) => Continue(false),
    });
}

struct UpdateState {
    release: net::Release,
    asset: Option<net::Asset>,
    path: Option<PathBuf>,
    installed: bool,
    busy: bool,
}

fn copy_text(text: &str) {
    gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).set_text(text);
}

fn show_update_dialog(w: &Widgets, release: net::Release) {
    let kind = update::package_kind();
    let ext = update::asset_extension();
    let asset = release
        .assets
        .iter()
        .find(|a| a.name.ends_with(ext))
        .cloned();
    let action_text = if kind == "appimage" {
        "Update"
    } else {
        "Download and install"
    };
    let dialog = Dialog::new();
    dialog.set_title("Update available");
    dialog.set_transient_for(Some(&w.window));
    dialog.set_modal(true);
    dialog.set_default_size(580, -1);
    dialog.add_button("Release page", ResponseType::Help);
    dialog.add_button("Later", ResponseType::Cancel);
    let action = Button::with_label(action_text);
    add_class(&action, "primary");
    dialog.add_action_widget(&action, ResponseType::Apply);
    action.set_sensitive(asset.is_some());

    let area = dialog.get_content_area();
    area.set_spacing(10);
    area.set_margin_start(22);
    area.set_margin_end(22);
    area.set_margin_top(20);
    area.set_margin_bottom(16);
    area.pack_start(
        &mk_label("UPDATE AVAILABLE", "section-title"),
        false,
        false,
        0,
    );
    let version = format!(
        "{}{} — you have v{} ({}).",
        if release.name.is_empty() {
            release.tag_name.clone()
        } else {
            release.name.clone()
        },
        if release.prerelease {
            " (pre-release)"
        } else {
            ""
        },
        env!("CARGO_PKG_VERSION"),
        kind
    );
    let version_label = mk_label(&version, "muted");
    version_label.set_line_wrap(true);
    area.pack_start(&version_label, false, false, 0);
    if !release.body.trim().is_empty() {
        let notes = release.body.lines().take(10).collect::<Vec<_>>().join("\n");
        let notes_label = mk_label(&notes, "faint");
        notes_label.set_line_wrap(true);
        notes_label.set_selectable(true);
        area.pack_start(&notes_label, false, false, 0);
    }
    let state_label = mk_label("", "faint");
    state_label.set_line_wrap(true);
    if asset.is_none() {
        state_label.set_text(&format!(
            "This release has no {ext} file. Use the release page."
        ));
    }
    area.pack_start(&state_label, false, false, 0);

    let command_row = Box::new(Orientation::Horizontal, 8);
    command_row.set_no_show_all(true);
    let command_entry = Entry::new();
    command_entry.set_editable(false);
    command_entry.set_hexpand(true);
    let copy_btn = Button::with_label("Copy command");
    command_row.pack_start(&command_entry, true, true, 0);
    command_row.pack_start(&copy_btn, false, false, 0);
    area.pack_start(&command_row, false, false, 0);
    {
        let entry = command_entry.clone();
        copy_btn.connect_clicked(move |_| copy_text(entry.get_text().as_ref()));
    }

    let state = Rc::new(RefCell::new(UpdateState {
        release: release.clone(),
        asset,
        path: None,
        installed: false,
        busy: false,
    }));
    let command_row_cb = command_row.clone();

    {
        let state = Rc::clone(&state);
        dialog.connect_response(move |dlg, response| match response {
            ResponseType::Cancel => {
                let mut s = settings::Settings::load();
                s.skipped_update = state.borrow().release.tag_name.clone();
                let _ = s.save();
                dlg.close();
            }
            ResponseType::Help => {
                let url = format!(
                    "https://github.com/WhatTFF/linux-ajazz-ak820-pro-gtk/releases/tag/{}",
                    state.borrow().release.tag_name
                );
                let _ = update::open_url(&url);
            }
            ResponseType::Apply => {
                handle_update_action(
                    &state,
                    &action,
                    &state_label,
                    &command_row_cb,
                    &command_entry,
                );
            }
            _ => {}
        });
    }

    dialog.show_all();
    command_row.hide();
}

fn handle_update_action(
    state: &Rc<RefCell<UpdateState>>,
    action: &Button,
    status: &Label,
    command_row: &Box,
    command_entry: &Entry,
) {
    {
        let mut s = state.borrow_mut();
        if s.busy {
            return;
        }
        if s.installed {
            let _ = update::restart();
            gtk::main_quit();
            return;
        }
        s.busy = true;
    }
    action.set_sensitive(false);

    let (path, asset, appimage) = {
        let s = state.borrow();
        (
            s.path.clone(),
            s.asset.clone(),
            update::package_kind() == "appimage",
        )
    };
    if let Some(path) = path {
        status.set_text("Installing…");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(update::install(&path));
        });
        let state = Rc::clone(state);
        let action = action.clone();
        let status = status.clone();
        glib::timeout_add_local(200, move || match rx.try_recv() {
            Ok(Ok(text)) => {
                {
                    let mut s = state.borrow_mut();
                    s.busy = false;
                    s.installed = true;
                }
                status.set_text(&text);
                action.set_label("Restart now");
                action.set_sensitive(true);
                Continue(false)
            }
            Ok(Err(e)) => {
                state.borrow_mut().busy = false;
                status.set_text(&format!("Update failed: {e}"));
                action.set_sensitive(true);
                Continue(false)
            }
            Err(TryRecvError::Empty) => Continue(true),
            Err(_) => Continue(false),
        });
        return;
    }

    let Some(asset) = asset else {
        state.borrow_mut().busy = false;
        action.set_sensitive(true);
        return;
    };
    if appimage {
        status.set_text("Replacing this AppImage…");
        let url = asset.browser_download_url.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(update::replace_appimage(&url));
        });
        let state = Rc::clone(state);
        let action = action.clone();
        let status = status.clone();
        glib::timeout_add_local(200, move || match rx.try_recv() {
            Ok(Ok(text)) => {
                {
                    let mut s = state.borrow_mut();
                    s.busy = false;
                    s.installed = true;
                }
                status.set_text(&text);
                action.set_label("Restart now");
                action.set_sensitive(true);
                Continue(false)
            }
            Ok(Err(e)) => {
                state.borrow_mut().busy = false;
                status.set_text(&format!("Update failed: {e}"));
                action.set_sensitive(true);
                Continue(false)
            }
            Err(TryRecvError::Empty) => Continue(true),
            Err(_) => Continue(false),
        });
        return;
    }

    status.set_text("Downloading…");
    let url = asset.browser_download_url.clone();
    let name = asset.name.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(update::download(&url, &name));
    });
    let state = Rc::clone(state);
    let action = action.clone();
    let status = status.clone();
    let command_row = command_row.clone();
    let command_entry = command_entry.clone();
    glib::timeout_add_local(200, move || match rx.try_recv() {
        Ok(Ok(path)) => {
            {
                let mut s = state.borrow_mut();
                s.busy = false;
                s.path = Some(path.clone());
            }
            let command = update::install_command(update::package_kind(), &path);
            command_entry.set_text(&command);
            command_row.show_all();
            status.set_text(&format!(
                "Downloaded to {}. Install it now, or copy the command and run it yourself.",
                path.display()
            ));
            action.set_label("Install now");
            action.set_sensitive(true);
            Continue(false)
        }
        Ok(Err(e)) => {
            state.borrow_mut().busy = false;
            status.set_text(&format!("Update failed: {e}"));
            action.set_sensitive(true);
            Continue(false)
        }
        Err(TryRecvError::Empty) => Continue(true),
        Err(_) => Continue(false),
    });
}

fn connect_signals(w: &Widgets, app: &Rc<App>) {
    // ---------- tab actions ----------
    for (name, button) in w.tab_buttons.borrow().iter() {
        let name = name.clone();
        let app = Rc::clone(app);
        let w2 = w.clone();
        button.connect_toggled(move |b| {
            if !b.get_active() {
                return;
            }
            match name.as_str() {
                "anim" => regenerate(&w2, &app),
                "text" => {
                    app.set_text_source();
                    update_preview(&w2, &app);
                }
                "draw" => {
                    app.draw_to_source();
                    update_preview(&w2, &app);
                }
                "file" => {
                    if app.has_file.get() {
                        app.name.replace(app.file_name.borrow().clone());
                        app.set_source(app.file_source.borrow().clone());
                    } else {
                        app.set_source(Vec::new());
                    }
                    update_preview(&w2, &app);
                }
                _ => {}
            }
        });
    }

    // ---------- generator cards ----------
    for g in &generators() {
        let b = ToggleButton::new();
        add_class(&b, "gen-card");
        let col = Box::new(Orientation::Vertical, 4);
        let frames = (g.build_fn)(&default_params(g.id), 24);
        if let Some(first) = frames.get(12).or_else(|| frames.first()) {
            let pb = pixbuf_from_rgba(&first.image, THUMB_PX);
            let img = Image::from_pixbuf(Some(&pb));
            col.pack_start(&img, false, false, 0);
        }
        let name = Label::new(Some(g.name));
        name.set_halign(gtk::Align::Center);
        add_class(&name, "gen-name");
        col.pack_start(&name, false, false, 0);
        b.add(&col);
        w.gen_grid.add(&b);
        w.gen_buttons
            .borrow_mut()
            .push((g.id.to_string(), g.name.to_string(), b));
    }
    {
        let entries = w.gen_buttons.borrow().clone();
        for (_, _, b) in &entries {
            let others: Vec<ToggleButton> = entries
                .iter()
                .filter(|(_, _, o)| o != b)
                .map(|(_, _, o)| o.clone())
                .collect();
            b.connect_toggled(move |t| {
                if t.get_active() {
                    for o in &others {
                        if o.get_active() {
                            o.set_active(false);
                        }
                    }
                }
            });
        }
    }
    {
        let entries = w.gen_buttons.borrow().clone();
        for (id, name, b) in &entries {
            let id = id.clone();
            let name = name.clone();
            let app = Rc::clone(app);
            let w2 = w.clone();
            b.connect_toggled(move |t| {
                if !t.get_active() {
                    return;
                }
                app.gen_id.replace(id.clone());
                app.gen_params.replace(default_params(&id));
                app.name.replace(name.clone());
                show_gen_params(&w2, &app);
                regenerate(&w2, &app);
                update_canvas(&w2, &app);
            });
        }
    }
    if let Some((_, _, b)) = w.gen_buttons.borrow().first().cloned() {
        b.set_active(true);
    }

    // ---------- smoothness ----------
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.gframes_scale.connect_value_changed(move |s| {
            let n = s.get_value() as u16;
            app.smooth.set(n);
            w2.gframes_out.set_text(&format!("{n} frames"));
            regenerate(&w2, &app);
        });
    }

    // ---------- shuffle / surprise ----------
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.shuffle_btn.connect_clicked(move |_| {
            let entries = w2.gen_buttons.borrow().clone();
            if entries.is_empty() {
                return;
            }
            let current = app.gen_id.borrow().clone();
            let idx = entries
                .iter()
                .position(|(id, _, _)| *id == current)
                .unwrap_or(0);
            entries[(idx + 1) % entries.len()].2.set_active(true);
        });
    }
    {
        let w2 = w.clone();
        w.surprise_btn.connect_clicked(move |_| {
            let entries = w2.gen_buttons.borrow().clone();
            if entries.is_empty() {
                return;
            }
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as usize)
                .unwrap_or(0);
            entries[nanos % entries.len()].2.set_active(true);
        });
    }

    // ---------- file picker ----------
    {
        let w2 = w.clone();
        w.file_btn.connect_clicked(move |_| choose_file(&w2));
    }

    // ---------- drop files anywhere / paste an image ----------
    {
        let targets = [gtk::TargetEntry::new(
            "text/uri-list",
            gtk::TargetFlags::OTHER_APP,
            0,
        )];
        w.window
            .drag_dest_set(gtk::DestDefaults::ALL, &targets, gdk::DragAction::COPY);
        let w2 = w.clone();
        w.window
            .connect_drag_data_received(move |_, _, _, _, data, _, _| {
                for uri in data.get_uris() {
                    if let Ok((path, _)) = glib::filename_from_uri(&uri) {
                        load_path_into_app(&w2, &path);
                    }
                }
            });
        let w2 = w.clone();
        w.window.connect_key_press_event(move |_, ev| {
            let ctrl = ev.get_state().contains(gdk::ModifierType::CONTROL_MASK);
            let is_v = ev
                .get_keyval()
                .to_unicode()
                .is_some_and(|c| c.eq_ignore_ascii_case(&'v'));
            if ctrl && is_v {
                paste_clipboard(&w2);
                return Inhibit(true);
            }
            Inhibit(false)
        });
    }

    // ---------- GIPHY search ----------
    {
        let w2 = w.clone();
        w.giphy_search_btn
            .connect_clicked(move |_| giphy_search(&w2, false));
    }
    {
        let w2 = w.clone();
        w.giphy_more_btn
            .connect_clicked(move |_| giphy_search(&w2, true));
    }
    {
        let w2 = w.clone();
        w.giphy_entry
            .connect_activate(move |_| giphy_search(&w2, false));
    }

    // ---------- upload ----------
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.upload_btn.connect_clicked(move |_| {
            let payload = app.output_payload();
            if payload.is_empty() {
                set_msg(&w2, "Nothing to send yet.", "err");
                return;
            }
            let flag = Arc::new(AtomicBool::new(false));
            app.cancel.replace(Some(Arc::clone(&flag)));
            let done = Arc::new(std::sync::atomic::AtomicU32::new(0));
            let progress = w2.upload_progress.clone();
            let cancel = w2.cancel_btn.clone();
            let btn = w2.upload_btn.clone();
            let msg = w2.msg.clone();
            progress.set_fraction(0.0);
            progress.show();
            cancel.show();
            btn.set_sensitive(false);
            set_msg(&w2, "Uploading…", "");
            let (tx, rx) = mpsc::channel();
            let done_t = Arc::clone(&done);
            std::thread::spawn(move || {
                let _ = tx.send((|| -> Result<(), String> {
                    let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
                    let mut dev = Device::open().map_err(|e| e.to_string())?;
                    dev.upload_with(&payload, &mut |d, total| {
                        done_t.store(
                            ((d * 100).checked_div(total).unwrap_or(0) as u32).clamp(1, 100),
                            Ordering::Relaxed,
                        );
                        !flag.load(Ordering::Relaxed)
                    })
                    .map_err(|e| e.to_string())
                })());
            });
            let done_ui = Arc::clone(&done);
            glib::timeout_add_local(60, move || match rx.try_recv() {
                Ok(Ok(())) => {
                    progress.set_fraction(1.0);
                    progress.hide();
                    cancel.hide();
                    btn.set_sensitive(true);
                    msg.set_text("Upload complete.");
                    let ctx = msg.get_style_context();
                    ctx.remove_class("err");
                    ctx.add_class("ok");
                    Continue(false)
                }
                Ok(Err(e)) => {
                    progress.hide();
                    cancel.hide();
                    btn.set_sensitive(true);
                    msg.set_text(&format!("Upload failed: {e}"));
                    let ctx = msg.get_style_context();
                    ctx.remove_class("ok");
                    ctx.add_class("err");
                    Continue(false)
                }
                Err(TryRecvError::Empty) => {
                    let pct = done_ui.load(Ordering::Relaxed);
                    progress.set_fraction(pct as f64 / 100.0);
                    Continue(true)
                }
                Err(_) => Continue(false),
            });
        });
    }
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.cancel_btn.connect_clicked(move |_| {
            if let Some(flag) = app.cancel.borrow().as_ref() {
                flag.store(true, Ordering::Relaxed);
            }
            set_msg(&w2, "Cancelling…", "");
        });
    }

    // ---------- save to history ----------
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.save_hist.connect_clicked(move |_| {
            let payload = app.output_payload();
            let name = app.name.borrow().clone();
            match history::save(&payload, &name) {
                Ok(_) => {
                    set_msg(&w2, "Saved to history.", "ok");
                    refresh_history(&w2);
                }
                Err(e) => set_msg(&w2, &format!("Save failed: {e}"), "err"),
            }
        });
    }

    // ---------- connect ----------
    {
        let w2 = w.clone();
        w.connect_btn.connect_clicked(move |_| connect_device(&w2));
    }

    // ---------- sync clock ----------
    {
        let w2 = w.clone();
        w.sync_btn
            .connect_clicked(move |_| spawn_clock_sync(&w2, false, None));
    }

    // ---------- built-in animation ----------
    {
        let w2 = w.clone();
        w.builtin_btn.connect_clicked(move |_| {
            let index = w2.builtin_combo.get_active().unwrap_or(0) as u8;
            let status = w2.builtin_status.clone();
            status.set_text("Selecting built-in…");
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send((|| -> Result<(), String> {
                    let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
                    let mut dev = Device::open().map_err(|e| e.to_string())?;
                    dev.show_builtin(index).map_err(|e| e.to_string())
                })());
            });
            let status = status.clone();
            glib::timeout_add_local(100, move || match rx.try_recv() {
                Ok(Ok(())) => {
                    status.set_text(&format!("Built-in {} selected", index + 1));
                    Continue(false)
                }
                Ok(Err(e)) => {
                    status.set_text(&format!("Failed: {e}"));
                    Continue(false)
                }
                Err(TryRecvError::Empty) => Continue(true),
                Err(_) => Continue(false),
            });
        });
    }

    // ---------- settings ----------
    {
        let w2 = w.clone();
        w.settings_btn.connect_clicked(move |_| open_settings(&w2));
    }

    // ---------- history ----------
    {
        let w2 = w.clone();
        w.hist_clear_btn.connect_clicked(move |_| {
            let confirm = MessageDialog::new(
                Some(&w2.window),
                gtk::DialogFlags::MODAL,
                gtk::MessageType::Question,
                gtk::ButtonsType::YesNo,
                "Delete all saved history from this computer?",
            );
            let w3 = w2.clone();
            confirm.connect_response(move |dlg, response| {
                if response == ResponseType::Yes {
                    match history::clear() {
                        Ok(n) => {
                            set_msg(
                                &w3,
                                &format!(
                                    "Deleted {n} history entr{}.",
                                    if n == 1 { "y" } else { "ies" }
                                ),
                                "ok",
                            );
                            refresh_history(&w3);
                        }
                        Err(e) => set_msg(&w3, &format!("Could not clear history: {e}"), "err"),
                    }
                }
                dlg.close();
            });
            confirm.show_all();
        });
    }

    refresh_history(w);
    connect_editor(w, app);
}

fn choose_file(w: &Widgets) {
    let dialog = FileChooserDialog::new(
        Some("Choose an image or animation"),
        Some(&w.window),
        FileChooserAction::Open,
    );
    dialog.set_modal(true);
    dialog.add_button("Cancel", ResponseType::Cancel);
    dialog.add_button("Open", ResponseType::Accept);
    dialog.set_default_response(ResponseType::Accept);

    let w2 = w.clone();
    dialog.connect_response(move |dlg, response| {
        if response == ResponseType::Accept {
            if let Some(path) = dlg.get_filename() {
                load_path_into_app(&w2, &path);
            }
        }
        dlg.close();
    });
    dialog.show_all();
}

/// Decode a file and make it the current source.
fn load_path_into_app(w: &Widgets, path: &Path) {
    let app = Rc::clone(&w.app);
    match decode_load(path, DecodeFit::Cover, [0, 0, 0, 255], None) {
        Ok(frames) => {
            let frames: Vec<SourceFrame> = frames
                .into_iter()
                .map(|f| SourceFrame {
                    image: f.image,
                    delay_ms: f.delay_ms,
                })
                .collect();
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("File")
                .to_string();
            app.name.replace(name.clone());
            app.file_name.replace(name);
            app.file_source.replace(frames.clone());
            app.set_source(frames);
            app.has_file.set(true);
            update_preview(w, &app);
            set_msg(w, "File loaded.", "ok");
        }
        Err(e) => set_msg(w, &format!("Decode failed: {e}"), "err"),
    }
}

/// Turn a clipboard image into the current source.
fn paste_clipboard(w: &Widgets) {
    let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
    if let Some(pb) = clipboard.wait_for_image() {
        let (width, height) = (pb.get_width(), pb.get_height());
        let rowstride = pb.get_rowstride() as usize;
        let channels = pb.get_n_channels() as usize;
        let alpha = pb.get_has_alpha();
        let Some(bytes) = pb.read_pixel_bytes() else {
            set_msg(w, "Clipboard image could not be read.", "err");
            return;
        };
        let bytes = bytes.as_ref();
        let mut rgba =
            RgbaImage::from_pixel(width as u32, height as u32, image::Rgba([0, 0, 0, 255]));
        for y in 0..height as usize {
            for x in 0..width as usize {
                let i = y * rowstride + x * channels;
                if i + 2 >= bytes.len() {
                    continue;
                }
                let a = if alpha && i + 3 < bytes.len() {
                    bytes[i + 3]
                } else {
                    255
                };
                rgba.put_pixel(
                    x as u32,
                    y as u32,
                    image::Rgba([bytes[i], bytes[i + 1], bytes[i + 2], a]),
                );
            }
        }
        let frames = vec![SourceFrame {
            image: crate::decode::fit_to_screen(&rgba, DecodeFit::Cover, [0, 0, 0, 255]),
            delay_ms: 100,
        }];
        w.app.name.replace("Clipboard".into());
        w.app.file_name.replace("Clipboard".into());
        w.app.file_source.replace(frames.clone());
        w.app.set_source(frames);
        w.app.has_file.set(true);
        update_preview(w, &w.app);
        set_msg(w, "Image pasted from the clipboard.", "ok");
        return;
    }
    if let Some(text) = clipboard.wait_for_text() {
        for line in text.lines() {
            let uri = line.trim();
            if let Ok((path, _)) = glib::filename_from_uri(uri) {
                load_path_into_app(w, &path);
                return;
            }
        }
    }
    set_msg(w, "Clipboard has no image or file to paste.", "err");
}

fn giphy_result_thumb(bytes: &[u8]) -> Option<RgbaImage> {
    image::load_from_memory(bytes).ok().map(|i| i.to_rgba8())
}

/// Run a GIPHY search and fill the result grid. `more` appends the next page.
fn giphy_search(w: &Widgets, more: bool) {
    let key = settings::Settings::load().giphy_key;
    if key.trim().is_empty() {
        set_msg(
            w,
            "Add a GIPHY API key in Settings, then search again.",
            "err",
        );
        return;
    }
    let query = w.giphy_entry.get_text().to_string();
    if more {
        let previous = w.giphy_query.borrow().clone();
        if previous != query {
            w.giphy_offset.replace(0);
        }
    } else {
        w.giphy_offset.replace(0);
        for child in w.giphy_box.get_children() {
            w.giphy_box.remove(&child);
        }
        w.giphy_more_btn.hide();
        w.giphy_scroll.hide();
    }
    w.giphy_query.replace(query.clone());
    let rating = w
        .giphy_rating
        .get_active_id()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "r".into());
    let offset = *w.giphy_offset.borrow();
    w.giphy_status.set_text("Searching GIPHY…");
    w.giphy_search_btn.set_sensitive(false);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = net::giphy_search(&key, &query, &rating, offset, 24).map(|gifs| {
            gifs.into_iter()
                .map(|gif| {
                    let thumb = net::image_bytes(&gif.preview_url)
                        .ok()
                        .and_then(|bytes| giphy_result_thumb(&bytes));
                    (gif, thumb)
                })
                .collect::<Vec<_>>()
        });
        let _ = tx.send(result);
    });
    let w2 = w.clone();
    glib::timeout_add_local(100, move || match rx.try_recv() {
        Ok(Ok(results)) => {
            w2.giphy_search_btn.set_sensitive(true);
            if results.is_empty() {
                w2.giphy_status.set_text("No results.");
                Continue(false)
            } else {
                w2.giphy_status.set_text(&format!(
                    "{} result(s). Click one to load it.",
                    results.len()
                ));
                *w2.giphy_offset.borrow_mut() += results.len();
                for (gif, thumb) in results {
                    giphy_add_result(&w2, gif, thumb);
                }
                w2.giphy_scroll.show_all();
                w2.giphy_more_btn.show();
                Continue(false)
            }
        }
        Ok(Err(e)) => {
            w2.giphy_search_btn.set_sensitive(true);
            w2.giphy_status.set_text(&format!("GIPHY: {e}"));
            Continue(false)
        }
        Err(TryRecvError::Empty) => Continue(true),
        Err(_) => {
            w2.giphy_search_btn.set_sensitive(true);
            Continue(false)
        }
    });
}

fn giphy_add_result(w: &Widgets, gif: net::Gif, thumb: Option<RgbaImage>) {
    let button = Button::new();
    add_class(&button, "gen-card");
    let col = Box::new(Orientation::Vertical, 4);
    if let Some(img) = thumb {
        let pb = pixbuf_from_rgba(&img, 84);
        let image = Image::from_pixbuf(Some(&pb));
        col.pack_start(&image, false, false, 0);
    }
    let title = gif.title.clone();
    let name = if title.is_empty() {
        gif.id.clone()
    } else {
        title
    };
    let label = Label::new(Some(&name));
    label.set_max_width_chars(14);
    label.set_halign(gtk::Align::Center);
    add_class(&label, "gen-name");
    col.pack_start(&label, false, false, 0);
    button.add(&col);
    let w2 = w.clone();
    let url = gif.url.clone();
    let id = gif.id.clone();
    button.connect_clicked(move |_| giphy_load(&w2, &url, &id));
    w.giphy_box.add(&button);
    w.giphy_box.show_all();
}

fn giphy_load(w: &Widgets, url: &str, id: &str) {
    w.giphy_status.set_text("Downloading GIF…");
    let url = url.to_string();
    let id = id.to_string();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ =
            tx.send(net::giphy_download(&url).and_then(|bytes| {
                decode::load_bytes(&bytes, DecodeFit::Cover, [0, 0, 0, 255], None)
            }));
    });
    let w2 = w.clone();
    glib::timeout_add_local(100, move || match rx.try_recv() {
        Ok(Ok(frames)) => {
            let frames: Vec<SourceFrame> = frames
                .into_iter()
                .map(|f| SourceFrame {
                    image: f.image,
                    delay_ms: f.delay_ms,
                })
                .collect();
            w2.app.name.replace(format!("GIPHY {id}"));
            w2.app.file_name.replace(format!("GIPHY {id}"));
            w2.app.file_source.replace(frames.clone());
            w2.app.set_source(frames);
            w2.app.has_file.set(true);
            update_preview(&w2, &w2.app);
            w2.giphy_status
                .set_text("GIF loaded. Press Send to keyboard.");
            Continue(false)
        }
        Ok(Err(e)) => {
            w2.giphy_status
                .set_text(&format!("GIPHY download failed: {e}"));
            Continue(false)
        }
        Err(TryRecvError::Empty) => Continue(true),
        Err(_) => Continue(false),
    });
}

fn connect_device(w: &Widgets) {
    set_pill(w, "busy", "Connecting…");
    w.connect_btn.set_sensitive(false);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send((|| -> Result<ak820_proto::DeviceInfo, String> {
            let mut last = "the keyboard stayed busy".to_string();
            for attempt in 0..5 {
                match DeviceLock::acquire() {
                    Ok(_lock) => {
                        let mut dev = Device::open().map_err(|e| e.to_string())?;
                        return dev.info().map_err(|e| e.to_string());
                    }
                    Err(ak820_device::Error::Locked(_)) if attempt < 4 => {
                        std::thread::sleep(Duration::from_millis(300));
                        last = "the keyboard is busy: another AK820 tool is talking to it".into();
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
            Err(last)
        })());
    });
    let w2 = w.clone();
    glib::timeout_add_local(80, move || match rx.try_recv() {
        Ok(Ok(info)) => {
            w2.dev_vid_pid
                .set_text(&format!("vid/pid: {:04x}:{:04x}", info.vid, info.pid));
            w2.dev_firmware.set_text(&format!(
                "firmware: {}.{:02}",
                info.firmware_hundredths / 100,
                info.firmware_hundredths % 100
            ));
            w2.dev_mode.set_text(&format!("mode: {}", info.work_mode));
            w2.dev_battery
                .set_text(&format!("battery: {}%", info.battery));
            w2.dev_charging
                .set_text(&format!("charging: {}", info.charging));
            w2.dev_profile
                .set_text(&format!("profile: {}", info.profile));
            w2.dev_frames
                .set_text(&format!("max frames: {}", info.max_frames));
            w2.dev_direction
                .set_text(&format!("screen: {}", info.tft_direction));
            *w2.app.max_frames.borrow_mut() = info.max_frames as usize;
            w2.dev_panel.show_all();
            w2.connect_btn.set_label("Reconnect");
            w2.connect_btn.set_sensitive(true);
            w2.sync_btn.set_sensitive(true);
            set_pill(
                &w2,
                "ok",
                &format!("Connected · {:04x}:{:04x}", info.vid, info.pid),
            );
            set_msg(&w2, "Keyboard connected.", "ok");
            w2.app.connected.set(true);
            update_preview(&w2, &w2.app);
            Continue(false)
        }
        Ok(Err(e)) => {
            w2.connect_btn.set_sensitive(true);
            w2.sync_btn.set_sensitive(false);
            set_pill(&w2, "err", "Not connected");
            w2.app.connected.set(false);
            update_preview(&w2, &w2.app);
            set_msg(&w2, &format!("Connect failed: {e}"), "err");
            Continue(false)
        }
        Err(TryRecvError::Empty) => Continue(true),
        Err(_) => Continue(false),
    });
}

fn show_gen_params(w: &Widgets, app: &Rc<App>) {
    for child in w.gen_param_box.get_children() {
        w.gen_param_box.remove(&child);
    }
    let id = app.gen_id.borrow().clone();
    let p = app.gen_params.borrow().clone();
    let grid = Grid::new();
    grid.set_column_spacing(20);
    grid.set_row_spacing(14);
    grid.set_column_homogeneous(true);

    let pat_active = match p.pattern {
        gen::Pattern::V => "v",
        gen::Pattern::D => "d",
        gen::Pattern::R => "r",
        gen::Pattern::S => "s",
        gen::Pattern::C => "c",
        _ => "h",
    };
    let soft_active = match p.soft {
        gen::Soft::Pastel => "pastel",
        gen::Soft::Dark => "dark",
        _ => "full",
    };
    let pal_active = match p.palette {
        gen::Palette::Neon => "neon",
        gen::Palette::Fire => "fire",
        gen::Palette::Ocean => "ocean",
        gen::Palette::Toxic => "toxic",
        gen::Palette::Mono => "mono",
        _ => "rainbow",
    };
    let col_active = match p.color {
        gen::Color::Blue => "blue",
        gen::Color::Red => "red",
        gen::Color::Classic => "classic",
        gen::Color::Rainbow => "rainbow",
        gen::Color::White => "white",
        gen::Color::Gold => "gold",
        gen::Color::Mono => "mono",
        _ => "green",
    };

    match id.as_str() {
        "rainbow" => {
            let (pat, pats) = seg_box(
                "Pattern",
                &[
                    ("h", "Across"),
                    ("v", "Down"),
                    ("d", "Diagonal"),
                    ("r", "Rings"),
                    ("s", "Spiral"),
                    ("c", "Swirl"),
                ],
                pat_active,
            );
            wire_seg(&pats, app, w, "pattern");
            grid.attach(&pat, 0, 0, 2, 1);
            let (bands, bands_s, bands_o) = scale_ctl("Bands", 1.0, 6.0, 1.0, p.bands as f64);
            wire_scale_ctl(&bands_s, &bands_o, app, w, "bands", |v| {
                format!("{}", v as i32)
            });
            grid.attach(&bands, 0, 1, 1, 1);
            let (look, looks) = seg_box(
                "Look",
                &[("full", "Bright"), ("pastel", "Pastel"), ("dark", "Deep")],
                soft_active,
            );
            wire_seg(&looks, app, w, "soft");
            grid.attach(&look, 1, 1, 1, 1);
        }
        "plasma" => {
            let (pal, pals) = seg_box(
                "Colours",
                &[
                    ("rainbow", "Rainbow"),
                    ("neon", "Neon"),
                    ("fire", "Lava"),
                    ("ocean", "Ocean"),
                    ("toxic", "Toxic"),
                ],
                pal_active,
            );
            wire_seg(&pals, app, w, "palette");
            grid.attach(&pal, 0, 0, 2, 1);
        }
        "ripples" => {
            let (pal, pals) = seg_box(
                "Colours",
                &[
                    ("rainbow", "Rainbow"),
                    ("ocean", "Ocean"),
                    ("neon", "Neon"),
                    ("mono", "Mono"),
                ],
                pal_active,
            );
            wire_seg(&pals, app, w, "palette");
            grid.attach(&pal, 0, 0, 2, 1);
        }
        "matrix" => {
            let (col, cols) = seg_box(
                "Colour",
                &[
                    ("green", "Green"),
                    ("rainbow", "Rainbow"),
                    ("blue", "Blue"),
                    ("red", "Red"),
                ],
                col_active,
            );
            wire_seg(&cols, app, w, "color");
            grid.attach(&col, 0, 0, 2, 1);
        }
        "stars" => {
            let (col, cols) = seg_box(
                "Stars",
                &[("white", "White"), ("rainbow", "Rainbow"), ("gold", "Gold")],
                col_active,
            );
            wire_seg(&cols, app, w, "color");
            grid.attach(&col, 0, 0, 1, 1);
            let (trail, trails) = seg_box(
                "Trails",
                &[("off", "Off"), ("on", "On")],
                if p.trail == gen::Trail::On {
                    "on"
                } else {
                    "off"
                },
            );
            wire_seg(&trails, app, w, "trail");
            grid.attach(&trail, 1, 0, 1, 1);
        }
        "fire" => {
            let (flame, flames) = seg_box(
                "Flame",
                &[
                    ("classic", "Classic"),
                    ("ice", "Ice"),
                    ("toxic", "Toxic"),
                    ("rainbow", "Rainbow"),
                ],
                match p.flame {
                    gen::Flame::Ice => "ice",
                    gen::Flame::Toxic => "toxic",
                    gen::Flame::Rainbow => "rainbow",
                    _ => "classic",
                },
            );
            wire_seg(&flames, app, w, "flame");
            grid.attach(&flame, 0, 0, 2, 1);
        }
        "snow" => {
            let (sky, skies) = seg_box(
                "Sky",
                &[("night", "Night"), ("dusk", "Dusk"), ("black", "Black")],
                match p.sky {
                    gen::Sky::Dusk => "dusk",
                    gen::Sky::Black => "black",
                    _ => "night",
                },
            );
            wire_seg(&skies, app, w, "sky");
            grid.attach(&sky, 0, 0, 2, 1);
        }
        "hearts" => {
            let (mood, moods) = seg_box(
                "Colours",
                &[
                    ("love", "Love"),
                    ("rainbow", "Rainbow"),
                    ("pastel", "Pastel"),
                ],
                match p.mood {
                    gen::Mood::Rainbow => "rainbow",
                    gen::Mood::Pastel => "pastel",
                    _ => "love",
                },
            );
            wire_seg(&moods, app, w, "mood");
            grid.attach(&mood, 0, 0, 2, 1);
        }
        "bounce" => {
            let text_box = Box::new(Orientation::Vertical, 4);
            add_class(&text_box, "ctl");
            text_box.pack_start(&mk_label("Text", "ctl-label"), false, false, 0);
            let entry = Entry::new();
            entry.set_text(&p.text);
            entry.set_max_length(20);
            text_box.pack_start(&entry, false, false, 0);
            let app_c = Rc::clone(app);
            let w_c = w.clone();
            entry.connect_changed(move |e| {
                set_param(&app_c, &w_c, "text", &e.get_text());
            });
            grid.attach(&text_box, 0, 0, 1, 1);
            let (size, sizes) = seg_box(
                "Size",
                &[("small", "Small"), ("medium", "Medium"), ("large", "Large")],
                match p.size {
                    gen::Size::Small => "small",
                    gen::Size::Large => "large",
                    _ => "medium",
                },
            );
            wire_seg(&sizes, app, w, "size");
            grid.attach(&size, 1, 0, 1, 1);
        }
        "eq" => {
            let (col, cols) = seg_box(
                "Colours",
                &[
                    ("classic", "Classic"),
                    ("rainbow", "Rainbow"),
                    ("mono", "Mono"),
                ],
                col_active,
            );
            wire_seg(&cols, app, w, "color");
            grid.attach(&col, 0, 0, 2, 1);
        }
        _ => {}
    }
    w.gen_param_box.pack_start(&grid, false, false, 0);
    w.gen_param_box.show_all();
}

fn stamp(app: &App, x0: i32, y0: i32, x1: i32, y1: i32) {
    let mut buf = app.draw_buffer.borrow_mut();
    let radius = (*app.draw_size.borrow() / 2.0).max(0.5);
    let tool = app.draw_tool.borrow().clone();
    let rainbow = *app.draw_rainbow.borrow();
    let bg = *app.draw_bg.borrow();
    let fg = *app.draw_color.borrow();
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
    for s in 0..=steps {
        let t = s as f64 / steps as f64;
        let cx = x0 as f64 + (x1 - x0) as f64 * t;
        let cy = y0 as f64 + (y1 - y0) as f64 * t;
        let col = if tool == "erase" {
            bg
        } else if rainbow {
            gen_hsv(cx * 2.2 + cy * 1.1, 1.0, 1.0)
        } else {
            fg
        };
        for py in (cy - radius).floor() as i32..=(cy + radius).ceil() as i32 {
            for px in (cx - radius).floor() as i32..=(cx + radius).ceil() as i32 {
                if px < 0 || py < 0 || px >= W || py >= H {
                    continue;
                }
                let dx = px as f64 + 0.5 - cx;
                let dy = py as f64 + 0.5 - cy;
                if dx * dx + dy * dy <= radius * radius {
                    buf.put_pixel(
                        px as u32,
                        py as u32,
                        image::Rgba([col[0], col[1], col[2], 255]),
                    );
                }
            }
        }
    }
    let _ = tool;
}

fn make_exclusive(buttons: &[(String, ToggleButton)]) {
    for (_, button) in buttons {
        let others: Vec<ToggleButton> = buttons
            .iter()
            .filter(|(_, o)| o != button)
            .map(|(_, o)| o.clone())
            .collect();
        button.connect_toggled(move |t| {
            if t.get_active() {
                for o in &others {
                    if o.get_active() {
                        o.set_active(false);
                    }
                }
            }
        });
    }
}

fn connect_editor(w: &Widgets, app: &Rc<App>) {
    // ---------- draw canvas ----------
    let drawing = Rc::new(Cell::new(false));
    let last = Rc::new(Cell::new((0i32, 0i32)));
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        let drawing = Rc::clone(&drawing);
        let last = Rc::clone(&last);
        w.draw_area.connect_button_press_event(move |area, ev| {
            if ev.get_button() != 1 {
                return Inhibit(false);
            }
            let (ax, ay) = ev.get_position();
            let aw = area.get_allocated_width().max(1) as f64;
            let ah = area.get_allocated_height().max(1) as f64;
            let x = ((ax * W as f64 / aw) as i32).clamp(0, W - 1);
            let y = ((ay * H as f64 / ah) as i32).clamp(0, H - 1);
            app.draw_undo
                .borrow_mut()
                .push(app.draw_buffer.borrow().clone());
            drawing.set(true);
            last.set((x, y));
            stamp(&app, x, y, x, y);
            app.draw_to_source();
            update_canvas(&w2, &app);
            update_preview(&w2, &app);
            Inhibit(true)
        });
    }
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        let drawing = Rc::clone(&drawing);
        let last = Rc::clone(&last);
        w.draw_area.connect_motion_notify_event(move |area, ev| {
            if !drawing.get() {
                return Inhibit(false);
            }
            let (ax, ay) = ev.get_position();
            let aw = area.get_allocated_width().max(1) as f64;
            let ah = area.get_allocated_height().max(1) as f64;
            let x = ((ax * W as f64 / aw) as i32).clamp(0, W - 1);
            let y = ((ay * H as f64 / ah) as i32).clamp(0, H - 1);
            let (lx, ly) = last.get();
            stamp(&app, lx, ly, x, y);
            last.set((x, y));
            app.draw_to_source();
            update_canvas(&w2, &app);
            update_preview(&w2, &app);
            Inhibit(true)
        });
    }
    {
        let drawing = Rc::clone(&drawing);
        w.draw_area.connect_button_release_event(move |_, _| {
            drawing.set(false);
            Inhibit(true)
        });
    }

    make_exclusive(&w.tool_buttons.borrow());
    for (value, button) in w.tool_buttons.borrow().iter() {
        let value = value.clone();
        let app = Rc::clone(app);
        button.connect_toggled(move |t| {
            if t.get_active() {
                app.draw_tool.replace(value.clone());
            }
        });
    }
    w.bsize_out.set_text("4px");
    {
        let app = Rc::clone(app);
        let out = w.bsize_out.clone();
        w.bsize_scale.connect_value_changed(move |s| {
            let v = s.get_value();
            app.draw_size.replace(v);
            out.set_text(&format!("{}px", v as i32));
        });
    }
    w.bcolor_btn.set_rgba(&rgba(1.0, 1.0, 1.0));
    {
        let app = Rc::clone(app);
        w.bcolor_btn.connect_color_set(move |btn| {
            let c = btn.get_rgba();
            app.draw_color.replace([
                (c.red * 255.0) as u8,
                (c.green * 255.0) as u8,
                (c.blue * 255.0) as u8,
            ]);
        });
    }
    {
        let app = Rc::clone(app);
        w.brainbow_btn.connect_toggled(move |t| {
            app.draw_rainbow.replace(t.get_active());
        });
    }
    w.dbg_btn.set_rgba(&rgba(0.0, 0.0, 0.0));
    {
        let app = Rc::clone(app);
        w.dbg_btn.connect_color_set(move |btn| {
            let c = btn.get_rgba();
            app.draw_bg.replace([
                (c.red * 255.0) as u8,
                (c.green * 255.0) as u8,
                (c.blue * 255.0) as u8,
            ]);
        });
    }
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.dclear_btn.connect_clicked(move |_| {
            let bg = *app.draw_bg.borrow();
            {
                let mut buf = app.draw_buffer.borrow_mut();
                for px in buf.pixels_mut() {
                    *px = image::Rgba([bg[0], bg[1], bg[2], 255]);
                }
            }
            app.draw_undo.borrow_mut().clear();
            app.draw_to_source();
            update_canvas(&w2, &app);
            update_preview(&w2, &app);
        });
    }
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.dundo_btn.connect_clicked(move |_| {
            if let Some(prev) = app.draw_undo.borrow_mut().pop() {
                *app.draw_buffer.borrow_mut() = prev;
            }
            app.draw_to_source();
            update_canvas(&w2, &app);
            update_preview(&w2, &app);
        });
    }
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.dtrace_btn.connect_clicked(move |_| {
            if let Some(frame) = app.output.borrow().first() {
                *app.draw_buffer.borrow_mut() = frame.image.clone();
            }
            app.draw_to_source();
            update_canvas(&w2, &app);
            update_preview(&w2, &app);
        });
    }

    // ---------- text ----------
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        let buffer = w.txt_view.get_buffer().expect("text buffer");
        buffer.connect_changed(move |b| {
            let text = b
                .get_text(&b.get_start_iter(), &b.get_end_iter(), false)
                .map(|s| s.to_string())
                .unwrap_or_default();
            app.text_value.replace(text);
            app.set_text_source();
            update_preview(&w2, &app);
        });
    }
    make_exclusive(&w.tmode_buttons.borrow());
    for (value, button) in w.tmode_buttons.borrow().iter() {
        let value = value.clone();
        let app = Rc::clone(app);
        let w2 = w.clone();
        button.connect_toggled(move |t| {
            if t.get_active() {
                app.text_mode.replace(value.clone());
                app.set_text_source();
                update_preview(&w2, &app);
            }
        });
    }
    make_exclusive(&w.tcol_buttons.borrow());
    for (value, button) in w.tcol_buttons.borrow().iter() {
        let value = value.clone();
        let app = Rc::clone(app);
        let w2 = w.clone();
        button.connect_toggled(move |t| {
            if t.get_active() {
                app.text_colour.replace(value.clone());
                app.set_text_source();
                update_preview(&w2, &app);
            }
        });
    }
    w.tsize_out.set_text("40px");
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.tsize_scale.connect_value_changed(move |s| {
            let v = s.get_value();
            w2.tsize_out.set_text(&format!("{}px", v as i32));
            app.text_size.replace(v);
            app.set_text_source();
            update_preview(&w2, &app);
        });
    }
    w.tfg_btn.set_rgba(&rgba(1.0, 1.0, 1.0));
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.tfg_btn.connect_color_set(move |btn| {
            let c = btn.get_rgba();
            app.text_fg.replace([
                (c.red * 255.0) as u8,
                (c.green * 255.0) as u8,
                (c.blue * 255.0) as u8,
            ]);
            app.set_text_source();
            update_preview(&w2, &app);
        });
    }
    w.tbg_btn.set_rgba(&rgba(0.909, 0.349, 0.047));
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.tbg_btn.connect_color_set(move |btn| {
            let c = btn.get_rgba();
            app.text_bg.replace([
                (c.red * 255.0) as u8,
                (c.green * 255.0) as u8,
                (c.blue * 255.0) as u8,
            ]);
            app.set_text_source();
            update_preview(&w2, &app);
        });
    }
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.toutline_btn.connect_toggled(move |t| {
            app.text_outline.replace(t.get_active());
            app.set_text_source();
            update_preview(&w2, &app);
        });
    }
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.tfont_combo.connect_changed(move |cb| {
            if let Some(f) = cb.get_active_id() {
                app.text_font.replace(f.to_string());
                app.set_text_source();
                update_preview(&w2, &app);
            }
        });
    }
    w.tspeed_out.set_text("3×");
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        let out = w.tspeed_out.clone();
        w.tspeed_scale.connect_value_changed(move |s| {
            let v = s.get_value();
            out.set_text(&format!("{}×", v as i32));
            app.text_speed.replace(v);
            app.set_text_source();
            update_preview(&w2, &app);
        });
    }

    connect_fx_and_adjust(w, app);
}

fn pct(v: f64) -> String {
    format!("{}%", v as i32)
}
fn num(v: f64) -> String {
    format!("{}", v as i32)
}
fn mult(v: f64) -> String {
    let f = 2f64.powi(v as i32);
    if f >= 1.0 {
        format!("{}×", f as i32)
    } else {
        format!("{f}×")
    }
}

fn connect_fx_and_adjust(w: &Widgets, app: &Rc<App>) {
    // ---------- effect chips ----------
    for (key, button) in w.fx_chips.borrow().iter() {
        let key = key.clone();
        let app = Rc::clone(app);
        let w2 = w.clone();
        button.connect_toggled(move |b| {
            let on = b.get_active();
            {
                let mut fx = app.fx.borrow_mut();
                match key.as_str() {
                    "cycle" => fx.cycle = on,
                    "wash" => fx.wash = on,
                    "border" => fx.border = on,
                    "capRainbow" => fx.cap_rainbow = on,
                    "spin" => fx.spin = if on { 1 } else { 0 },
                    "shake" => fx.shake = on,
                    "breathe" => fx.breathe = on,
                    "pulse" => fx.pulse = on,
                    "glitch" => fx.glitch = on,
                    "sparkle" => fx.sparkle = on,
                    "scan" => fx.scan = on,
                    "vignette" => fx.vignette = on,
                    "posterize" => fx.posterize = on,
                    "gray" => fx.gray = on,
                    "sepia" => fx.sepia = on,
                    "invert" => fx.invert = on,
                    _ => {}
                }
            }
            app.rebuild_output();
            update_preview(&w2, &app);
        });
    }
    w.fx_hue_out.set_text("0°");
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        let out = w.fx_hue_out.clone();
        w.fx_hue_scale.connect_value_changed(move |s| {
            let v = s.get_value();
            out.set_text(&format!("{}°", v as i32));
            app.fx.borrow_mut().hue = v;
            app.rebuild_output();
            update_preview(&w2, &app);
        });
    }
    w.fx_pixel_out.set_text("off");
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        let out = w.fx_pixel_out.clone();
        w.fx_pixel_scale.connect_value_changed(move |s| {
            let v = s.get_value() as u8;
            out.set_text(
                if v <= 1 {
                    "off".to_string()
                } else {
                    format!("{v}px")
                }
                .as_str(),
            );
            app.fx.borrow_mut().pixel = v;
            app.rebuild_output();
            update_preview(&w2, &app);
        });
    }
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.fx_cap_entry.connect_changed(move |e| {
            let text = e.get_text().to_string();
            app.fx.borrow_mut().caption = if text.is_empty() { None } else { Some(text) };
            app.rebuild_output();
            update_preview(&w2, &app);
        });
    }
    make_exclusive(&w.fx_cappos_buttons.borrow());
    for (value, button) in w.fx_cappos_buttons.borrow().iter() {
        let value = value.clone();
        let app = Rc::clone(app);
        let w2 = w.clone();
        button.connect_toggled(move |t| {
            if t.get_active() {
                app.fx.borrow_mut().cap_pos = match value.as_str() {
                    "top" => CapPos::Top,
                    "middle" => CapPos::Middle,
                    _ => CapPos::Bottom,
                };
                app.rebuild_output();
                update_preview(&w2, &app);
            }
        });
    }
    w.fx_capcolor_btn.set_rgba(&rgba(1.0, 1.0, 1.0));
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.fx_capcolor_btn.connect_color_set(move |btn| {
            let c = btn.get_rgba();
            app.fx.borrow_mut().cap_color = [
                (c.red * 255.0) as u8,
                (c.green * 255.0) as u8,
                (c.blue * 255.0) as u8,
            ];
            app.rebuild_output();
            update_preview(&w2, &app);
        });
    }
    w.fx_loop_out.set_text("24 frames");
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        let out = w.fx_loop_out.clone();
        w.fx_loop_scale.connect_value_changed(move |s| {
            let v = s.get_value() as u16;
            out.set_text(&format!("{v} frames"));
            app.fx.borrow_mut().loop_len = v;
            app.rebuild_output();
            update_preview(&w2, &app);
        });
    }
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.fx_clear_btn.connect_clicked(move |_| {
            let chips = w2.fx_chips.borrow().clone();
            for (_, b) in &chips {
                b.set_active(false);
            }
            let _ = &app;
        });
    }
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.fx_random_btn.connect_clicked(move |_| {
            let chips = w2.fx_chips.borrow().clone();
            if chips.is_empty() {
                return;
            }
            for (_, b) in &chips {
                b.set_active(false);
            }
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as usize)
                .unwrap_or(0);
            for i in 0..3 {
                chips[(nanos / (i + 1)) % chips.len()].1.set_active(true);
            }
            app.rebuild_output();
            update_preview(&w2, &app);
        });
    }

    // ---------- adjust ----------
    make_exclusive(&w.fit_buttons.borrow());
    for (value, button) in w.fit_buttons.borrow().iter() {
        let value = value.clone();
        let app = Rc::clone(app);
        let w2 = w.clone();
        button.connect_toggled(move |t| {
            if t.get_active() {
                app.opts.borrow_mut().fit = match value.as_str() {
                    "contain" => fx::Fit::Contain,
                    "stretch" => fx::Fit::Stretch,
                    _ => fx::Fit::Cover,
                };
                app.rebuild_output();
                update_preview(&w2, &app);
            }
        });
    }
    make_exclusive(&w.rot_buttons.borrow());
    for (value, button) in w.rot_buttons.borrow().iter() {
        let value = value.clone();
        let app = Rc::clone(app);
        let w2 = w.clone();
        button.connect_toggled(move |t| {
            if t.get_active() {
                app.opts.borrow_mut().rot = value.parse().unwrap_or(0);
                app.rebuild_output();
                update_preview(&w2, &app);
            }
        });
    }

    macro_rules! opt_scale {
        ($scale:expr, $out:expr, $field:ident, $fmt:ident) => {{
            let out = $out.clone();
            $out.set_text(&$fmt($scale.get_value()));
            let app = Rc::clone(app);
            let w2 = w.clone();
            $scale.connect_value_changed(move |s| {
                let v = s.get_value();
                app.opts.borrow_mut().$field = v;
                out.set_text(&$fmt(v));
                app.rebuild_output();
                update_preview(&w2, &app);
            });
        }};
    }
    opt_scale!(w.zoom_scale, w.zoom_out, zoom, pct);
    opt_scale!(w.panx_scale, w.panx_out, panx, num);
    opt_scale!(w.pany_scale, w.pany_out, pany, num);
    opt_scale!(w.bri_scale, w.bri_out, bri, pct);
    opt_scale!(w.con_scale, w.con_out, con, pct);
    opt_scale!(w.sat_scale, w.sat_out, sat, pct);

    w.speed_out.set_text("1×");
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        let out = w.speed_out.clone();
        w.speed_scale.connect_value_changed(move |s| {
            let v = s.get_value() as i32;
            out.set_text(&mult(v as f64));
            app.speed.set(v);
            app.rebuild_output();
            update_preview(&w2, &app);
        });
    }
    w.tstart_out.set_text("1");
    w.tend_out.set_text("1");
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.trim_start.connect_value_changed(move |s| {
            if app.syncing_trim.get() {
                return;
            }
            app.trim_start.set(s.get_value() as usize);
            app.rebuild_output();
            update_preview(&w2, &app);
        });
    }
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.trim_end.connect_value_changed(move |s| {
            if app.syncing_trim.get() {
                return;
            }
            app.trim_end.set(s.get_value() as usize);
            app.rebuild_output();
            update_preview(&w2, &app);
        });
    }
    w.skip_out.set_text("all frames");
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        let out = w.skip_out.clone();
        w.skip_scale.connect_value_changed(move |s| {
            let v = s.get_value() as i32;
            out.set_text(
                if v <= 1 {
                    "all frames".to_string()
                } else {
                    format!("every {v}th")
                }
                .as_str(),
            );
            app.skip.set(v.max(1) as usize);
            app.rebuild_output();
            update_preview(&w2, &app);
        });
    }
    w.bg_btn.set_rgba(&rgba(0.0, 0.0, 0.0));
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.bg_btn.connect_color_set(move |btn| {
            let c = btn.get_rgba();
            app.opts.borrow_mut().bg = [
                (c.red * 255.0) as u8,
                (c.green * 255.0) as u8,
                (c.blue * 255.0) as u8,
            ];
            app.rebuild_output();
            update_preview(&w2, &app);
        });
    }
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.flip_btn.connect_toggled(move |t| {
            app.opts.borrow_mut().mirror = t.get_active();
            app.rebuild_output();
            update_preview(&w2, &app);
        });
    }
    {
        let app = Rc::clone(app);
        let w2 = w.clone();
        w.adjust_reset_btn.connect_clicked(move |_| {
            *app.opts.borrow_mut() = FrameOpts {
                fit: fx::Fit::Cover,
                rot: 0,
                mirror: false,
                bg: [0, 0, 0],
                zoom: 100.0,
                panx: 0.0,
                pany: 0.0,
                bri: 100.0,
                con: 100.0,
                sat: 100.0,
            };
            w2.zoom_scale.set_value(100.0);
            w2.panx_scale.set_value(0.0);
            w2.pany_scale.set_value(0.0);
            w2.bri_scale.set_value(100.0);
            w2.con_scale.set_value(100.0);
            w2.sat_scale.set_value(100.0);
            w2.speed_scale.set_value(0.0);
            app.speed.set(0);
            app.skip.set(1);
            let len = app.source.borrow().len();
            app.trim_start.set(0);
            app.trim_end.set(len.saturating_sub(1));
            app.syncing_trim.set(true);
            w2.skip_scale.set_value(1.0);
            w2.trim_start.set_value(0.0);
            w2.trim_end.set_value(len.saturating_sub(1) as f64);
            app.syncing_trim.set(false);
            if let Some((_, b)) = w2.fit_buttons.borrow().first() {
                b.set_active(true);
            }
            if let Some((_, b)) = w2.rot_buttons.borrow().first() {
                b.set_active(true);
            }
            w2.flip_btn.set_active(false);
            w2.bg_btn.set_rgba(&rgba(0.0, 0.0, 0.0));
            app.rebuild_output();
            update_preview(&w2, &app);
        });
    }

    update_canvas(w, app);
}

fn history_thumb(path: &Path) -> Option<Pixbuf> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut header = [0u8; 256];
    file.read_exact(&mut header).ok()?;
    let mut px = vec![0u8; FRAME_BYTES];
    file.read_exact(&mut px).ok()?;
    Some(pixbuf_from_rgba(&rgb565_image(&px), HIST_PX))
}

fn refresh_history(w: &Widgets) {
    let list = &w.history_box;
    for child in list.get_children() {
        list.remove(&child);
    }
    for entry in history::list() {
        let button = Button::new();
        add_class(&button, "hitem");
        let col = Box::new(Orientation::Vertical, 5);
        if let Some(pb) = history_thumb(&entry.path) {
            let img = Image::from_pixbuf(Some(&pb));
            col.pack_start(&img, false, false, 0);
        }
        let name = mk_label(&entry.name, "hitem-name");
        name.set_max_width_chars(14);
        col.pack_start(&name, false, false, 0);
        let stem = entry
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let stamp = stem.split_once('_').map(|(s, _)| s).unwrap_or(stem);
        let when = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%d-%H%M%S")
            .ok()
            .map(|dt| {
                let today = chrono::Local::now().date_naive() == dt.date();
                if today {
                    dt.format("%-I:%M %p").to_string()
                } else {
                    dt.format("%b %-d").to_string()
                }
            });
        let frames = entry.bytes.saturating_sub(256) as usize / FRAME_BYTES;
        let mut cap = format!("{frames} fr");
        if let Some(when) = when {
            cap = format!("{when} · {cap}");
        }
        let cap = mk_label(&cap, "hitem-cap");
        cap.set_max_width_chars(18);
        col.pack_start(&cap, false, false, 0);
        button.add(&col);
        let app = Rc::clone(&w.app);
        let w2 = w.clone();
        let index = entry.index;
        let entry_name = entry.name.clone();
        button.connect_clicked(move |_| match history::load(index) {
            Ok(payload) => match frames_from_payload(&payload) {
                Ok(frames) => {
                    app.name.replace(entry_name.clone());
                    app.set_source(frames);
                    app.has_file.set(false);
                    app.file_source.replace(Vec::new());
                    update_preview(&w2, &app);
                    set_msg(&w2, &format!("Restored “{entry_name}”."), "ok");
                }
                Err(e) => set_msg(&w2, &format!("History load failed: {e}"), "err"),
            },
            Err(e) => set_msg(&w2, &format!("History load failed: {e}"), "err"),
        });
        let wrapper = Box::new(Orientation::Vertical, 2);
        wrapper.pack_start(&button, false, false, 0);
        let remove = Button::with_label("✕ Remove");
        add_class(&remove, "tiny");
        let w3 = w.clone();
        let index = entry.index;
        remove.connect_clicked(move |_| match history::delete(index) {
            Ok(()) => refresh_history(&w3),
            Err(e) => set_msg(&w3, &format!("Could not delete history entry: {e}"), "err"),
        });
        wrapper.pack_start(&remove, false, false, 0);
        list.add(&wrapper);
    }
    list.show_all();
}

fn open_settings(w: &Widgets) {
    let s = settings::Settings::load();
    let dialog = Dialog::new();
    dialog.set_title("Settings");
    dialog.set_transient_for(Some(&w.window));
    dialog.set_modal(true);
    dialog.set_default_size(620, 780);
    dialog.add_button("Cancel", ResponseType::Cancel);
    dialog.add_button("Save", ResponseType::Ok);
    dialog.set_default_response(ResponseType::Ok);
    if let Some(save) = dialog.get_widget_for_response(ResponseType::Ok) {
        add_class(&save, "primary");
    }

    let page = Box::new(Orientation::Vertical, 14);
    page.set_margin_start(22);
    page.set_margin_end(22);
    page.set_margin_top(20);
    page.set_margin_bottom(18);
    let title = mk_label("SETTINGS", "section-title");
    page.pack_start(&title, false, false, 0);
    let hint = mk_label(
        "Stored in ~/.local/share/ak820/settings.json, shared with ak820ctl and the systemd units.",
        "faint",
    );
    hint.set_line_wrap(true);
    page.pack_start(&hint, false, false, 0);

    let group = |name: &str| -> (Box, Box) {
        let outer = Box::new(Orientation::Vertical, 0);
        add_class(&outer, "card");
        let inner = Box::new(Orientation::Vertical, 8);
        inner.set_margin_start(16);
        inner.set_margin_end(16);
        inner.set_margin_top(14);
        inner.set_margin_bottom(14);
        inner.pack_start(&heading(name), false, false, 0);
        outer.pack_start(&inner, true, true, 0);
        (outer, inner)
    };

    // ---- Startup ----
    let (startup_card, startup_in) = group("Startup");
    let autostart = gtk::CheckButton::with_label("Start AK820 Screen Studio when I log in");
    autostart.set_active(s.start_at_login);
    startup_in.pack_start(&autostart, false, false, 0);
    let startmin =
        gtk::CheckButton::with_label("Start hidden in the tray instead of showing the window");
    startmin.set_active(s.start_minimized);
    startup_in.pack_start(&startmin, false, false, 0);
    let tray = gtk::CheckButton::with_label("Keep running in the tray when I close the window");
    tray.set_active(s.minimize_to_tray);
    startup_in.pack_start(&tray, false, false, 0);
    let bg_hint = mk_label(
        "The tray icon has Open, Sync clock now and Quit. Without a tray (e.g. plain GNOME \
         without the AppIndicator extension) closing the window quits the app.",
        "faint",
    );
    bg_hint.set_line_wrap(true);
    startup_in.pack_start(&bg_hint, false, false, 0);
    let quit_btn = Button::with_label("Quit AK820 Screen Studio");
    quit_btn.set_halign(gtk::Align::Start);
    quit_btn.connect_clicked(|_| gtk::main_quit());
    startup_in.pack_start(&quit_btn, false, false, 0);
    page.pack_start(&startup_card, false, false, 0);

    // ---- Clock ----
    let (clock_card, clock_in) = group("Clock");
    let sync_start =
        gtk::CheckButton::with_label("Sync the clock when the app starts, then every 12 hours");
    sync_start.set_active(s.sync_on_start);
    let sync_conn = gtk::CheckButton::with_label("Push the clock when the keyboard connects");
    sync_conn.set_active(s.sync_on_connect);
    let restore = gtk::CheckButton::with_label("Restore the last image when the keyboard connects");
    restore.set_active(s.restore_on_connect);
    let quit_sync = gtk::CheckButton::with_label("Quit right after the start-up sync");
    quit_sync.set_active(s.quit_after_start_sync);
    let autosync =
        gtk::CheckButton::with_label("Let the ak820-clock.timer sync while the app is closed");
    autosync.set_active(s.autosync_clock);
    clock_in.pack_start(&sync_start, false, false, 0);
    clock_in.pack_start(&sync_conn, false, false, 0);
    clock_in.pack_start(&restore, false, false, 0);
    clock_in.pack_start(&quit_sync, false, false, 0);
    clock_in.pack_start(&autosync, false, false, 0);
    let interval_row = Box::new(Orientation::Horizontal, 8);
    let interval_lbl = mk_label(
        "Extra re-sync interval (seconds, 0 = off, minimum 10)",
        "ctl-label",
    );
    interval_lbl.set_halign(gtk::Align::Start);
    interval_lbl.set_line_wrap(true);
    let interval_entry = Entry::new();
    interval_entry.set_text(&format!("{}", s.sync_interval_secs));
    interval_entry.set_placeholder_text(Some("0 = off"));
    interval_entry.set_width_chars(8);
    interval_row.pack_start(&interval_lbl, true, true, 0);
    interval_row.pack_start(&interval_entry, false, false, 0);
    clock_in.pack_start(&interval_row, false, false, 0);
    let sync_now = Button::with_label("Sync clock now");
    sync_now.set_halign(gtk::Align::Start);
    let clock_status = mk_label("", "faint");
    clock_status.set_line_wrap(true);
    {
        let w2 = w.clone();
        let status = clock_status.clone();
        sync_now.connect_clicked(move |_| {
            let status = status.clone();
            spawn_clock_sync(
                &w2,
                false,
                Some(std::boxed::Box::new(move |result| match result {
                    Ok(stamp) => status.set_text(&format!("Synced {stamp}")),
                    Err(e) => status.set_text(&format!("Sync failed: {e}")),
                })),
            );
        });
    }
    clock_in.pack_start(&sync_now, false, false, 0);
    clock_in.pack_start(&clock_status, false, false, 0);
    let clock_hint = mk_label(
        "The background timer keeps the keyboard clock accurate while the app is closed. \
         Enable it once with systemctl --user enable --now ak820-clock.timer, or leave it off \
         and let the app sync while it runs.",
        "faint",
    );
    clock_hint.set_line_wrap(true);
    clock_in.pack_start(&clock_hint, false, false, 0);
    page.pack_start(&clock_card, false, false, 0);

    // ---- Updates ----
    let (update_card, update_in) = group("Updates");
    let installed = format!(
        "Installed: v{} ({} build)",
        env!("CARGO_PKG_VERSION"),
        update::package_kind()
    );
    update_in.pack_start(&mk_label(&installed, "faint"), false, false, 0);
    let updates = gtk::CheckButton::with_label("Check for updates automatically (every 12 hours)");
    updates.set_active(s.check_updates);
    update_in.pack_start(&updates, false, false, 0);
    let update_row = Box::new(Orientation::Horizontal, 8);
    let check_now = Button::with_label("Check now");
    let update_status = mk_label("", "faint");
    update_status.set_line_wrap(true);
    update_row.pack_start(&check_now, false, false, 0);
    update_row.pack_start(&update_status, true, true, 0);
    update_in.pack_start(&update_row, false, false, 0);
    {
        let w2 = w.clone();
        let status = update_status.clone();
        check_now.connect_clicked(move |_| {
            check_updates(&w2, true, Some(status.clone()));
        });
    }
    page.pack_start(&update_card, false, false, 0);

    // ---- Uninstall ----
    let (remove_card, remove_in) = group("Uninstall");
    let remove_hint = mk_label(
        "Remove AK820 Screen Studio from this computer. Your history and settings in \
         ~/.local/share/ak820 are kept.",
        "faint",
    );
    remove_hint.set_line_wrap(true);
    remove_in.pack_start(&remove_hint, false, false, 0);
    let command_row = Box::new(Orientation::Horizontal, 8);
    let command_entry = Entry::new();
    command_entry.set_text(&update::uninstall_command());
    command_entry.set_editable(false);
    command_entry.set_hexpand(true);
    let copy_cmd = Button::with_label("Copy command");
    command_row.pack_start(&command_entry, true, true, 0);
    command_row.pack_start(&copy_cmd, false, false, 0);
    remove_in.pack_start(&command_row, false, false, 0);
    {
        let entry = command_entry.clone();
        copy_cmd.connect_clicked(move |_| copy_text(entry.get_text().as_ref()));
    }
    let uninstall_btn = Button::with_label("Uninstall AK820 Screen Studio");
    uninstall_btn.set_halign(gtk::Align::Start);
    add_class(&uninstall_btn, "primary");
    let uninstall_status = mk_label("", "faint");
    uninstall_status.set_line_wrap(true);
    remove_in.pack_start(&uninstall_btn, false, false, 0);
    remove_in.pack_start(&uninstall_status, false, false, 0);
    {
        let parent = w.window.clone();
        let status = uninstall_status.clone();
        let button = uninstall_btn.clone();
        uninstall_btn.connect_clicked(move |_| {
            let confirm = MessageDialog::new(
                Some(&parent),
                gtk::DialogFlags::MODAL,
                gtk::MessageType::Question,
                gtk::ButtonsType::YesNo,
                "Uninstall AK820 Screen Studio? History and settings are kept.",
            );
            let status = status.clone();
            let button = button.clone();
            confirm.connect_response(move |dlg, response| {
                if response == ResponseType::Yes {
                    button.set_sensitive(false);
                    status.set_text("Removing…");
                    let (tx, rx) = mpsc::channel();
                    std::thread::spawn(move || {
                        let _ = tx.send(update::uninstall());
                    });
                    let status = status.clone();
                    let button = button.clone();
                    glib::timeout_add_local(200, move || match rx.try_recv() {
                        Ok(Ok(text)) => {
                            status.set_text(&text);
                            glib::timeout_add_local(2000, move || {
                                gtk::main_quit();
                                Continue(false)
                            });
                            Continue(false)
                        }
                        Ok(Err(e)) => {
                            status.set_text(&format!("Uninstall failed: {e}"));
                            button.set_sensitive(true);
                            Continue(false)
                        }
                        Err(TryRecvError::Empty) => Continue(true),
                        Err(_) => Continue(false),
                    });
                    dlg.close();
                } else {
                    dlg.close();
                }
            });
            confirm.show_all();
        });
    }
    page.pack_start(&remove_card, false, false, 0);

    // ---- Web ----
    let (net_card, net_in) = group("Web");
    net_in.set_hexpand(true);
    let giphy_lbl = mk_label(
        "GIPHY API key (stored in settings.json, used only for GIF search)",
        "ctl-label",
    );
    giphy_lbl.set_halign(gtk::Align::Start);
    giphy_lbl.set_line_wrap(true);
    let giphy_row = Box::new(Orientation::Horizontal, 8);
    let giphy_entry = Entry::new();
    giphy_entry.set_text(&s.giphy_key);
    giphy_entry.set_placeholder_text(Some("paste your GIPHY API key"));
    giphy_entry.set_hexpand(true);
    let get_key = Button::with_label("Get a free key");
    giphy_row.pack_start(&giphy_entry, true, true, 0);
    giphy_row.pack_start(&get_key, false, false, 0);
    get_key.connect_clicked(|_| {
        let _ = update::open_url("https://developers.giphy.com/dashboard/?create=true");
    });
    net_in.pack_start(&giphy_lbl, false, false, 0);
    net_in.pack_start(&giphy_row, false, false, 0);
    page.pack_start(&net_card, false, false, 0);

    let scroll = ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    scroll.add(&page);
    dialog.get_content_area().pack_start(&scroll, true, true, 0);
    dialog.show_all();

    let w_ui = w.clone();
    dialog.connect_response(move |dlg, response| {
        if response == ResponseType::Ok {
            let mut s = settings::Settings::load();
            s.start_at_login = autostart.get_active();
            s.start_minimized = startmin.get_active();
            s.minimize_to_tray = tray.get_active();
            s.sync_on_start = sync_start.get_active();
            s.sync_on_connect = sync_conn.get_active();
            s.restore_on_connect = restore.get_active();
            s.quit_after_start_sync = quit_sync.get_active();
            s.autosync_clock = autosync.get_active();
            s.check_updates = updates.get_active();
            s.giphy_key = giphy_entry.get_text().trim().to_string();
            let interval = interval_entry.get_text().trim().parse::<u64>().unwrap_or(0);
            s.sync_interval_secs = if interval == 0 { 0 } else { interval.max(10) };
            match s.save() {
                Ok(()) => {
                    if let Err(e) = autostart::set(s.start_at_login) {
                        set_msg(
                            &w_ui,
                            &format!("Settings saved, but autostart failed: {e}"),
                            "err",
                        );
                    } else {
                        set_msg(&w_ui, "Settings saved.", "ok");
                    }
                }
                Err(e) => set_msg(&w_ui, &format!("Could not save settings: {e}"), "err"),
            }
        }
        dlg.close();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_frame() -> RgbaImage {
        RgbaImage::from_fn(128, 128, |x, y| {
            image::Rgba([(x * 2) as u8, (y * 2) as u8, 128, 255])
        })
    }

    fn frame_with_i(fx: Fx, t: f64, i: usize) -> RgbaImage {
        let mut out = RgbaImage::from_pixel(128, 128, image::Rgba([0, 0, 0, 255]));
        let opts = FrameOpts {
            fit: crate::fx::Fit::Cover,
            rot: 0,
            mirror: false,
            bg: [0, 0, 0],
            zoom: 100.0,
            panx: 0.0,
            pany: 0.0,
            bri: 100.0,
            con: 100.0,
            sat: 100.0,
        };
        draw_frame(&base_frame(), &opts, &fx, t, i, None, &mut out);
        out
    }

    fn frame_with(fx: Fx, t: f64) -> RgbaImage {
        frame_with_i(fx, t, 0)
    }

    fn differs(a: &RgbaImage, b: &RgbaImage) -> bool {
        a.pixels().zip(b.pixels()).any(|(p, q)| p != q)
    }

    #[test]
    fn text_renders_on_its_background() {
        let app = App::new();
        app.set_text_source();
        let out = app.output.borrow();
        let img = &out.first().expect("text frame").image;
        assert_eq!(
            img.get_pixel(1, 1).0,
            [232, 89, 12, 255],
            "text background should be the configured background colour"
        );
        assert!(
            img.pixels().any(|p| p.0 == [255, 255, 255, 255]),
            "text pixels should be drawn in the configured foreground colour"
        );
    }

    #[test]
    fn generators_are_varied() {
        for g in generators() {
            let frames = (g.build_fn)(&default_params(g.id), 36);
            let f = frames.get(12).expect("frame 12 exists");
            let mut colors = std::collections::HashSet::new();
            for px in f.image.pixels() {
                colors.insert(px.0);
            }
            assert!(colors.len() > 1, "{} collapsed to a single colour", g.id);
        }
    }

    #[test]
    fn effects_change_frames() {
        let plain = frame_with(Fx::default(), 0.25);
        for key in [
            "cycle", "wash", "border", "spin", "shake", "breathe", "pulse", "glitch", "sparkle",
        ] {
            let mut fx = Fx::default();
            match key {
                "cycle" => fx.cycle = true,
                "wash" => fx.wash = true,
                "border" => fx.border = true,
                "spin" => fx.spin = 1,
                "shake" => fx.shake = true,
                "breathe" => fx.breathe = true,
                "pulse" => fx.pulse = true,
                "glitch" => fx.glitch = true,
                "sparkle" => fx.sparkle = true,
                _ => unreachable!(),
            }
            let changed = if key == "glitch" {
                (0..24).any(|i| differs(&plain, &frame_with_i(fx.clone(), 0.25, i)))
            } else {
                differs(&plain, &frame_with(fx, 0.25))
            };
            assert!(changed, "effect {key} produced no change");
        }
        for key in ["scan", "vignette", "posterize", "gray", "sepia", "invert"] {
            let mut fx = Fx::default();
            match key {
                "scan" => fx.scan = true,
                "vignette" => fx.vignette = true,
                "posterize" => fx.posterize = true,
                "gray" => fx.gray = true,
                "sepia" => fx.sepia = true,
                "invert" => fx.invert = true,
                _ => unreachable!(),
            }
            assert!(
                differs(&plain, &frame_with(fx, 0.25)),
                "effect {key} produced no change"
            );
        }
        let fx = Fx {
            pixel: 8,
            ..Default::default()
        };
        assert!(
            differs(&plain, &frame_with(fx, 0.25)),
            "pixelate produced no change"
        );
    }

    fn test_frames(n: usize) -> Vec<SourceFrame> {
        (0..n)
            .map(|i| SourceFrame {
                image: RgbaImage::from_pixel(128, 128, image::Rgba([i as u8, 0, 0, 255])),
                delay_ms: 100,
            })
            .collect()
    }

    #[test]
    fn static_text_is_one_frame() {
        let app = App::new();
        app.set_text_source();
        assert_eq!(app.output.borrow().len(), 1);
    }

    #[test]
    fn trim_skip_and_speed_apply() {
        let app = App::new();
        app.set_source(test_frames(10));
        app.trim_start.set(2);
        app.trim_end.set(5);
        app.skip.set(2);
        app.speed.set(1);
        app.rebuild_output();
        let out = app.output.borrow();
        assert_eq!(out.len(), 2, "trim 2..5 step 2 should keep two frames");
        assert_eq!(out[0].delay_ms, 100, "two 100ms frames at 2x = 100ms");
    }

    #[test]
    fn caption_position_moves() {
        let top = Fx {
            caption: Some("TEST".into()),
            cap_pos: CapPos::Top,
            ..Default::default()
        };
        let mut bottom = top.clone();
        bottom.cap_pos = CapPos::Bottom;
        assert!(differs(&frame_with(top, 0.0), &frame_with(bottom, 0.0)));
    }
}
