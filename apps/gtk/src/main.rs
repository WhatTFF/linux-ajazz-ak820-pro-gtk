//! GTK3 control app for the AJAZZ / Epomaker AK820 Pro TFT screen.
//!
//! Reuses the shared protocol (`ak820-proto`), hidraw transport (`ak820-device`)
//! and on-disk history/settings (`ak820-store`) crates from the upstream
//! Rust/Tauri project. All heavy I/O runs on background threads; GTK widget
//! handles are never moved across threads (they are `!Send`). Results are sent
//! back to the main loop over an `mpsc` channel and applied via
//! `glib::timeout_add_local`.

mod decode;

use std::cell::RefCell;
use std::fs::File;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::sync::mpsc::TryRecvError;

use ak820_device::{Device, DeviceLock};
use ak820_proto::{build_payload, CivilTime, Frame, MAX_FRAMES, parse_payload};
use ak820_store::{history, settings};
use chrono::{Datelike, Timelike};
use gtk::{
    Box, Button, CheckButton, ComboBoxText, Entry, FileChooserButton, Grid,
    HeaderBar, Image, Label, ListBox, Orientation, ProgressBar, ScrolledWindow, Stack, Switch,
    Window, WindowType,
};
use gtk::prelude::*;
use image::codecs::png::PngEncoder;
use image::ImageEncoder;

use crate::decode::{load, parse_colour, to_rgb565, Fit};

/// Path used for the live preview thumbnail (PNG).
const PREVIEW_PATH: &str = "/tmp/ak820-gtk-preview.png";

fn main() {
    gtk::init().expect("failed to initialize GTK");

    let window = Window::new(WindowType::Toplevel);
    window.set_title("AK820 Pro Studio");
    window.set_default_size(900, 600);

    let views = build_ui(&window);
    connect_signals(&views);

    window.show_all();
    gtk::main();
}

/// All widget handles the signal closures need. Every field is `Clone` so each
/// closure can grab its own handle. Fields are only ever touched on the main
/// thread, so `Rc<RefCell<_>>` is fine here.
#[allow(dead_code)]
#[derive(Clone)]
struct Views {
    /// Main stack holding the pages.
    stack: gtk::Stack,
    /// Bottom status line.
    status: Label,

    // ---- Device page ----
    dev_vid_pid: Label,
    dev_firmware: Label,
    dev_mode: Label,
    dev_battery: Label,
    dev_charging: Label,
    dev_profile: Label,
    dev_frames: Label,
    dev_direction: Label,
    info_btn: Button,
    sync_btn: Button,
    sync_status: Label,
    builtin_index: Entry,
    builtin_btn: Button,
    builtin_status: Label,

    // ---- Upload page ----
    file_btn: FileChooserButton,
    fit_combo: ComboBoxText,
    bg_entry: Entry,
    delay_entry: Entry,
    maxframes_entry: Entry,
    preview: Image,
    preview_path: Rc<RefCell<Option<PathBuf>>>,
    build_btn: Button,
    upload_btn: Button,
    upload_progress: ProgressBar,
    upload_status: Label,
    save_history: CheckButton,
    upload_last_payload: Rc<RefCell<Option<Vec<u8>>>>,

    // ---- History page ----
    history_list: ListBox,
    refresh_history_btn: Button,

    // ---- Settings page ----
    sw_autosync: Switch,
    sw_restore: Switch,
    sw_sync_connect: Switch,
    sw_sync_start: Switch,
    sw_check_updates: Switch,
    sw_start_login: Switch,
    sw_minimize_tray: Switch,
    interval_entry: Entry,
    settings_status: Label,
    save_settings_btn: Button,
}

fn build_ui(window: &gtk::Window) -> Views {
    let header = HeaderBar::new();
    header.set_title(Some("AK820 Pro Studio"));
    window.set_titlebar(Some(&header));

    // --- Main stack (no sidebar; page-switch buttons live above the stack) ---
    let stack = Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(200);

    // page switch buttons
    let nav = Box::new(Orientation::Horizontal, 4);
    nav.set_margin_top(6);
    nav.set_margin_start(6);
    nav.set_margin_end(6);
    let nav_btn = |label: &str, name: &str| -> Button {
        let b = Button::with_label(label);
        let stack = stack.clone();
        let name = name.to_string();
        b.connect_clicked(move |_| {
            stack.set_visible_child_name(&name);
        });
        b
    };
    nav.pack_start(&nav_btn("Device", "device"), false, false, 0);
    nav.pack_start(&nav_btn("Upload", "upload"), false, false, 0);
    nav.pack_start(&nav_btn("History", "history"), false, false, 0);
    nav.pack_start(&nav_btn("Settings", "settings"), false, false, 0);

    let root = Box::new(Orientation::Vertical, 0);
    root.pack_start(&nav, false, false, 0);
    root.pack_start(&stack, true, true, 0);

    // status bar
    let status = Label::new(Some("ready"));
    status.set_halign(gtk::Align::Start);
    status.set_hexpand(true);
    let status_bar = Box::new(Orientation::Horizontal, 6);
    status_bar.set_margin_top(6);
    status_bar.set_margin_bottom(6);
    status_bar.set_margin_start(6);
    status_bar.set_margin_end(6);
    status_bar.pack_start(&status, true, true, 0);

    let outer = Box::new(Orientation::Vertical, 0);
    outer.pack_start(&root, true, true, 0);
    outer.pack_end(&status_bar, false, false, 0);
    window.add(&outer);

    // ===== Device page =====
    let dev_page = Box::new(Orientation::Vertical, 8);
    dev_page.set_margin_top(12);
    dev_page.set_margin_start(12);
    dev_page.set_margin_end(12);

    let info_box = Box::new(Orientation::Vertical, 4);
    let dev_vid_pid = Label::new(Some("vid/pid: —"));
    dev_vid_pid.set_halign(gtk::Align::Start);
    let dev_firmware = Label::new(Some("firmware: —"));
    dev_firmware.set_halign(gtk::Align::Start);
    let dev_mode = Label::new(Some("mode: —"));
    dev_mode.set_halign(gtk::Align::Start);
    let dev_battery = Label::new(Some("battery: —"));
    dev_battery.set_halign(gtk::Align::Start);
    let dev_charging = Label::new(Some("charging: —"));
    dev_charging.set_halign(gtk::Align::Start);
    let dev_profile = Label::new(Some("profile: —"));
    dev_profile.set_halign(gtk::Align::Start);
    let dev_frames = Label::new(Some("max frames: —"));
    dev_frames.set_halign(gtk::Align::Start);
    let dev_direction = Label::new(Some("screen: —"));
    dev_direction.set_halign(gtk::Align::Start);

    info_box.pack_start(&dev_vid_pid, false, false, 0);
    info_box.pack_start(&dev_firmware, false, false, 0);
    info_box.pack_start(&dev_mode, false, false, 0);
    info_box.pack_start(&dev_battery, false, false, 0);
    info_box.pack_start(&dev_charging, false, false, 0);
    info_box.pack_start(&dev_profile, false, false, 0);
    info_box.pack_start(&dev_frames, false, false, 0);
    info_box.pack_start(&dev_direction, false, false, 0);

    let info_btn = Button::with_label("read device info");
    let sync_btn = Button::with_label("sync clock to keyboard");
    sync_btn.set_halign(gtk::Align::Start);
    let sync_status = Label::new(Some(""));
    sync_status.set_halign(gtk::Align::Start);

    let builtin_box = Box::new(Orientation::Horizontal, 8);
    builtin_box.set_halign(gtk::Align::Start);
    let builtin_index = Entry::new();
    builtin_index.set_width_chars(4);
    builtin_index.set_text("0");
    let builtin_btn = Button::with_label("select built-in animation");
    let builtin_status = Label::new(Some(""));
    builtin_status.set_halign(gtk::Align::Start);
    builtin_box.pack_start(&builtin_index, false, false, 0);
    builtin_box.pack_start(&builtin_btn, false, false, 0);
    builtin_box.pack_start(&builtin_status, false, false, 0);

    dev_page.pack_start(&info_box, false, false, 0);
    dev_page.pack_start(&info_btn, false, false, 0);
    dev_page.pack_start(&sync_btn, false, false, 0);
    dev_page.pack_start(&sync_status, false, false, 0);
    dev_page.pack_start(&builtin_box, false, false, 0);
    dev_page.pack_start(&builtin_status, false, false, 0);

    let dev_scroll = ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    dev_scroll.set_vexpand(true);
    dev_scroll.add(&dev_page);
    stack.add_titled(&dev_scroll, "device", "Device");

    // ===== Upload page =====
    let up_page = Box::new(Orientation::Vertical, 8);
    up_page.set_margin_top(12);
    up_page.set_margin_start(12);
    up_page.set_margin_end(12);

    let file_btn = FileChooserButton::new(
        "Choose image or animated GIF",
        gtk::FileChooserAction::Open,
    );
    file_btn.set_hexpand(true);

    let fit_combo = ComboBoxText::new();
    fit_combo.append_text("cover (fill, crop)");
    fit_combo.append_text("contain (letterbox)");
    fit_combo.append_text("stretch");
    fit_combo.set_active(Some(0));

    let bg_entry = Entry::new();
    bg_entry.set_text("#000000");
    bg_entry.set_width_chars(10);
    let bg_label = Label::new(Some("background (contain):"));
    bg_label.set_halign(gtk::Align::Start);

    let delay_entry = Entry::new();
    delay_entry.set_placeholder_text(Some("force ms/frame (optional)"));
    delay_entry.set_width_chars(10);
    let delay_label = Label::new(Some("force delay (ms):"));
    delay_label.set_halign(gtk::Align::Start);

    let maxframes_entry = Entry::new();
    maxframes_entry.set_placeholder_text(Some("frame cap (optional)"));
    maxframes_entry.set_width_chars(10);
    let maxframes_label = Label::new(Some("max frames:"));
    maxframes_label.set_halign(gtk::Align::Start);

    let save_history = CheckButton::with_label("save to history");
    save_history.set_active(true);

    let build_btn = Button::with_label("build preview");
    let preview = Image::new();
    preview.set_pixel_size(128);
    let preview_path: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));

    let upload_btn = Button::with_label("upload to keyboard");
    let upload_progress = ProgressBar::new();
    upload_progress.set_hexpand(true);
    upload_progress.set_show_text(true);
    let upload_status = Label::new(Some(""));
    upload_status.set_halign(gtk::Align::Start);

    let up_grid = Grid::new();
    up_grid.set_column_spacing(8);
    up_grid.set_row_spacing(6);
    up_grid.set_column_homogeneous(false);

    let mut r = 0;
    up_grid.attach(&file_btn, 0, r, 4, 1); r += 1;
    up_grid.attach(&fit_combo, 0, r, 2, 1); r += 1;
    up_grid.attach(&bg_label, 0, r, 1, 1);
    up_grid.attach(&bg_entry, 1, r, 1, 1); r += 1;
    up_grid.attach(&delay_label, 0, r, 1, 1);
    up_grid.attach(&delay_entry, 1, r, 1, 1); r += 1;
    up_grid.attach(&maxframes_label, 0, r, 1, 1);
    up_grid.attach(&maxframes_entry, 1, r, 1, 1); r += 1;
    up_grid.attach(&save_history, 0, r, 2, 1); r += 1;
    up_grid.attach(&build_btn, 0, r, 2, 1); r += 1;
    up_grid.attach(&preview, 0, r, 2, 1); r += 1;
    up_grid.attach(&upload_btn, 0, r, 2, 1); r += 1;
    up_grid.attach(&upload_progress, 0, r, 2, 1); r += 1;
    up_grid.attach(&upload_status, 0, r, 2, 1);

    up_page.pack_start(&up_grid, false, false, 0);
    let up_scroll = ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    up_scroll.set_vexpand(true);
    up_scroll.add(&up_page);
    stack.add_titled(&up_scroll, "upload", "Upload");

    // ===== History page =====
    let hist_page = Box::new(Orientation::Vertical, 8);
    hist_page.set_margin_top(12);
    hist_page.set_margin_start(12);
    hist_page.set_margin_end(12);

    let hist_label = Label::new(Some("stored uploads (newest first):"));
    hist_label.set_halign(gtk::Align::Start);

    let history_list = ListBox::new();
    history_list.set_selection_mode(gtk::SelectionMode::Single);

    let hist_scroll = ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    hist_scroll.set_vexpand(true);
    hist_scroll.add(&history_list);

    let refresh_history_btn = Button::with_label("refresh");

    hist_page.pack_start(&hist_label, false, false, 0);
    hist_page.pack_start(&refresh_history_btn, false, false, 0);
    hist_page.pack_start(&hist_scroll, true, true, 0);
    stack.add_titled(&hist_page, "history", "History");

    // ===== Settings page =====
    let settings_page = Box::new(Orientation::Vertical, 8);
    settings_page.set_margin_top(12);
    settings_page.set_margin_start(12);
    settings_page.set_margin_end(12);

    let mk_switch = |label: &str, active: bool| -> (Switch, Box) {
        let sw = Switch::new();
        sw.set_active(active);
        let row = Box::new(Orientation::Horizontal, 8);
        let l = Label::new(Some(label));
        l.set_halign(gtk::Align::Start);
        row.pack_start(&l, true, true, 0);
        row.pack_end(&sw, false, false, 0);
        (sw, row)
    };

    let (sw_autosync, row_autosync) = mk_switch("autosync clock (60 s)", true);
    let (sw_restore, row_restore) = mk_switch("restore last image on connect", false);
    let (sw_sync_connect, row_sync_connect) = mk_switch("sync clock on connect", true);
    let (sw_sync_start, row_sync_start) = mk_switch("sync clock on app start", true);
    let (sw_check_updates, row_check_updates) = mk_switch("check for updates", true);
    let (sw_start_login, row_start_login) = mk_switch("start at login", false);
    let (sw_minimize_tray, row_minimize_tray) = mk_switch("minimize to tray on close", false);

    let interval_entry = Entry::new();
    interval_entry.set_width_chars(6);
    interval_entry.set_text("60");
    let interval_label = Label::new(Some("autosync interval (s):"));
    interval_label.set_halign(gtk::Align::Start);
    let interval_row = Box::new(Orientation::Horizontal, 8);
    interval_row.pack_start(&interval_label, false, false, 0);
    interval_row.pack_start(&interval_entry, false, false, 0);

    let save_settings_btn = Button::with_label("save settings");
    let settings_status = Label::new(Some(""));
    settings_status.set_halign(gtk::Align::Start);

    settings_page.pack_start(&row_autosync, false, false, 0);
    settings_page.pack_start(&row_restore, false, false, 0);
    settings_page.pack_start(&row_sync_connect, false, false, 0);
    settings_page.pack_start(&row_sync_start, false, false, 0);
    settings_page.pack_start(&row_check_updates, false, false, 0);
    settings_page.pack_start(&row_start_login, false, false, 0);
    settings_page.pack_start(&row_minimize_tray, false, false, 0);
    settings_page.pack_start(&interval_row, false, false, 0);
    settings_page.pack_start(&save_settings_btn, false, false, 0);
    settings_page.pack_start(&settings_status, false, false, 0);
    stack.add_titled(&settings_page, "settings", "Settings");

    stack.set_visible_child_name("device");

    window.show_all();

    // load saved settings into the UI once
    let s0 = settings::Settings::load();
    sw_autosync.set_active(s0.autosync_clock);
    sw_restore.set_active(s0.restore_on_connect);
    sw_sync_connect.set_active(s0.sync_on_connect);
    sw_sync_start.set_active(s0.sync_on_start);
    sw_check_updates.set_active(s0.check_updates);
    sw_start_login.set_active(s0.start_at_login);
    sw_minimize_tray.set_active(s0.minimize_to_tray);
    interval_entry.set_text(&s0.sync_interval_secs.to_string());

    Views {
        stack,
        status,
        dev_vid_pid,
        dev_firmware,
        dev_mode,
        dev_battery,
        dev_charging,
        dev_profile,
        dev_frames,
        dev_direction,
        info_btn,
        sync_btn,
        sync_status,
        builtin_index,
        builtin_btn,
        builtin_status,
        file_btn,
        fit_combo,
        bg_entry,
        delay_entry,
        maxframes_entry,
        preview,
        preview_path,
        build_btn,
        upload_btn,
        upload_progress,
        upload_status,
        save_history,
        history_list,
        refresh_history_btn,
        sw_autosync,
        sw_restore,
        sw_sync_connect,
        sw_sync_start,
        sw_check_updates,
        sw_start_login,
        sw_minimize_tray,
        interval_entry,
        settings_status,
        save_settings_btn,
        upload_last_payload: Rc::new(RefCell::new(None)),
    }
}

/// Spawns a worker thread running `op` and returns a receiver for its result.
fn run_on_worker<F, R>(op: F) -> mpsc::Receiver<Result<R, String>>
where
    F: FnOnce() -> Result<R, String> + Send + 'static,
    R: Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(op());
    });
    rx
}

/// Polls `rx` on the main thread every 50 ms; on the first result, runs `f` and
/// stops polling. `f` runs on the main thread and may touch GTK widgets.
fn when_done<F, R>(rx: mpsc::Receiver<Result<R, String>>, f: F) -> glib::SourceId
where
    R: 'static,
    F: FnOnce(Result<R, String>) + 'static,
{
    let f = std::cell::Cell::new(Some(f));
    glib::timeout_add_local(50, move || {
        match rx.try_recv() {
            Ok(res) => {
                f.take().unwrap()(res);
                glib::Continue(false)
            }
            Err(TryRecvError::Empty) => glib::Continue(true),
            Err(_) => glib::Continue(false),
        }
    })
}

fn connect_signals(v: &Views) {
    // ---- device info ----
    {
        let status = v.status.clone();
        let dev_vid_pid = v.dev_vid_pid.clone();
        let dev_firmware = v.dev_firmware.clone();
        let dev_mode = v.dev_mode.clone();
        let dev_battery = v.dev_battery.clone();
        let dev_charging = v.dev_charging.clone();
        let dev_profile = v.dev_profile.clone();
        let dev_frames = v.dev_frames.clone();
        let dev_direction = v.dev_direction.clone();
        v.info_btn.connect_clicked(move |_| {
            status.set_text("reading device info…");
            let rx = run_on_worker(|| -> Result<ak820_proto::DeviceInfo, String> {
                let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
                let mut dev = Device::open().map_err(|e| e.to_string())?;
                dev.info().map_err(|e| e.to_string())
            });
            let dev_vid_pid = dev_vid_pid.clone();
            let dev_firmware = dev_firmware.clone();
            let dev_mode = dev_mode.clone();
            let dev_battery = dev_battery.clone();
            let dev_charging = dev_charging.clone();
            let dev_profile = dev_profile.clone();
            let dev_frames = dev_frames.clone();
            let dev_direction = dev_direction.clone();
            let status = status.clone();
            when_done(rx, move |res| {
                match res {
                    Ok(info) => {
                        status.set_text("device info read");
                        dev_vid_pid.set_text(&format!(
                            "vid/pid: {:04x}:{:04x}",
                            info.vid, info.pid
                        ));
                        dev_firmware.set_text(&format!(
                            "firmware: {}.{:02}",
                            info.firmware_hundredths / 100,
                            info.firmware_hundredths % 100
                        ));
                        dev_mode.set_text(&format!("mode: {}", info.work_mode));
                        dev_battery.set_text(&format!("battery: {}%", info.battery));
                        dev_charging.set_text(&format!("charging: {}", info.charging));
                        dev_profile.set_text(&format!("profile: {}", info.profile));
                        dev_frames.set_text(&format!("max frames: {}", info.max_frames));
                        dev_direction.set_text(&format!("screen: {}", info.tft_direction));
                    }
                    Err(e) => status.set_text(&format!("device info failed: {e}")),
                }
            });
        });
    }

    // ---- sync clock ----
    {
        let sync_status = v.sync_status.clone();
        v.sync_btn.connect_clicked(move |_| {
            sync_status.set_text("syncing clock…");
            let rx = run_on_worker(|| -> Result<String, String> {
                let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
                let mut dev = Device::open().map_err(|e| e.to_string())?;
                let now = chrono::Local::now();
                let t = CivilTime {
                    year: now.year() as u16,
                    month: now.month() as u8,
                    day: now.day() as u8,
                    hour: now.hour() as u8,
                    minute: now.minute() as u8,
                    second: now.second() as u8,
                    weekday: now.weekday().num_days_from_sunday() as u8,
                };
                dev.sync_clock(t).map_err(|e| e.to_string())?;
                Ok(format!(
                    "synced {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                    t.year, t.month, t.day, t.hour, t.minute, t.second
                ))
            });
            let sync_status = sync_status.clone();
            when_done(rx, move |res| {
                match res {
                    Ok(msg) => sync_status.set_text(&msg),
                    Err(e) => sync_status.set_text(&format!("sync failed: {e}")),
                }
            });
        });
    }

    // ---- built-in animation ----
    {
        let builtin_status = v.builtin_status.clone();
        let builtin_index = v.builtin_index.clone();
        v.builtin_btn.connect_clicked(move |_| {
            let idx_text = builtin_index.get_text();
            let idx: u8 = match idx_text.trim().parse() {
                Ok(v) => v,
                Err(_) => {
                    builtin_status.set_text("invalid index");
                    return;
                }
            };
            builtin_status.set_text("selecting built-in…");
            let rx = run_on_worker(move || -> Result<(), String> {
                let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
                let mut dev = Device::open().map_err(|e| e.to_string())?;
                dev.show_builtin(idx).map_err(|e| e.to_string())
            });
            let builtin_status = builtin_status.clone();
            when_done(rx, move |res| {
                match res {
                    Ok(()) => builtin_status.set_text(format!("built-in {idx} selected").as_str()),
                    Err(e) => builtin_status.set_text(&format!("failed: {e}")),
                }
            });
        });
    }

    // ---- preview build ----
    {
        let preview_path = v.preview_path.clone();
        let preview = v.preview.clone();
        let upload_status = v.upload_status.clone();
        let upload_last_payload = v.upload_last_payload.clone();
        let file_btn = v.file_btn.clone();
        let fit_combo = v.fit_combo.clone();
        let bg_entry = v.bg_entry.clone();
        let delay_entry = v.delay_entry.clone();
        let maxframes_entry = v.maxframes_entry.clone();
        v.build_btn.connect_clicked(move |_| {
            let path = match file_btn.get_filename() {
                Some(p) => p,
                None => {
                    upload_status.set_text("choose a file first");
                    return;
                }
            };
            let fit = match fit_combo.get_active() {
                Some(1) => Fit::Contain,
                Some(2) => Fit::Stretch,
                _ => Fit::Cover,
            };
            let bg = parse_colour(&bg_entry.get_text()).unwrap_or([0, 0, 0, 255]);
            let force_delay: Option<u16> = delay_entry
                .get_text()
                .trim()
                .parse()
                .ok()
                .filter(|&d| d > 0);
            let max_frames: Option<usize> = maxframes_entry
                .get_text()
                .trim()
                .parse()
                .ok()
                .filter(|&d| d > 0);

            upload_status.set_text("building preview…");
            let preview_path = preview_path.clone();
            let preview = preview.clone();
            let upload_status = upload_status.clone();
            let upload_last_payload = upload_last_payload.clone();
            let path_for_thread = path.clone();
            let rx = run_on_worker(move || -> Result<Vec<u8>, String> {
                let frames = load(&path_for_thread, fit, bg, force_delay)?;
                let limit = max_frames.unwrap_or(MAX_FRAMES).min(MAX_FRAMES);
                let frames = &frames[..frames.len().min(limit)];
                let packed = to_rgb565(frames);
                let payload = build_payload(
                    &frames
                        .iter()
                        .zip(packed.iter())
                        .map(|(f, p)| Frame { pixels: p, delay_ms: f.delay_ms })
                        .collect::<Vec<_>>(),
                )
                .map_err(|e| e.to_string())?;
                // render first frame as PNG preview
                let first = parse_payload(&payload, 1)
                    .map_err(|e| e.to_string())?
                    .remove(0);
                let mut rgba = Vec::with_capacity(128 * 128 * 4);
                ak820_proto::to_rgba(&first.pixels, &mut rgba);
                let img = image::RgbaImage::from_raw(128, 128, rgba)
                    .ok_or("bad preview frame")?;
                let file = File::create(PREVIEW_PATH).map_err(|e| format!("preview: {e}"))?;
                let enc = PngEncoder::new(file);
                enc.write_image(&img, 128, 128, image::ExtendedColorType::Rgba8)
                    .map_err(|e| format!("preview png: {e}"))?;
                Ok(payload)
            });
            let preview_path = preview_path.clone();
            let preview = preview.clone();
            let upload_status = upload_status.clone();
            let upload_last_payload = upload_last_payload.clone();
            when_done(rx, move |res| {
                match res {
                    Ok(payload) => {
                        *preview_path.borrow_mut() = Some(PathBuf::from(PREVIEW_PATH));
                        preview.set_from_file(PREVIEW_PATH);
                        let n = parse_payload(&payload, usize::MAX)
                            .map(|f| f.len())
                            .unwrap_or(0);
                        upload_status.set_text(&format!("preview ready ({n} frames)"));
                        *upload_last_payload.borrow_mut() = Some(payload);
                    }
                    Err(e) => upload_status.set_text(&format!("preview failed: {e}")),
                }
            });
        });
    }

    // ---- upload ----
    {
        let upload_btn = v.upload_btn.clone();
        let upload_progress = v.upload_progress.clone();
        let upload_status = v.upload_status.clone();
        let save_history = v.save_history.clone();
        let upload_last_payload = v.upload_last_payload.clone();
        let stack = v.stack.clone();
        let file_btn = v.file_btn.clone();
        upload_btn.connect_clicked(move |_| {
            let payload = match upload_last_payload.borrow().clone() {
                Some(p) => p,
                None => {
                    upload_status.set_text("build a preview first");
                    return;
                }
            };
            let save = save_history.get_active();
            let file_name = file_btn.get_filename().unwrap_or_default();
            let name = file_name
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("image")
                .to_string();
            let progress_bar = upload_progress.clone();
            let up_status = upload_status.clone();
            let up_stack = stack.clone();
            let progress = Arc::new(AtomicU32::new(0));
            let progress_w = Arc::clone(&progress);
            let rx = run_on_worker(move || -> Result<String, String> {
                let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
                let mut dev = Device::open().map_err(|e| e.to_string())?;
                dev.upload_with(&payload, &mut |done, _| {
                    let pct = (done as u32 * 100).max(1).min(100);
                    progress_w.store(pct, Ordering::Relaxed);
                    true
                })
                .map_err(|e| e.to_string())?;
                if save {
                    history::save(&payload, &name).map_err(|e| format!("history: {e}"))?;
                }
                Ok("upload complete".into())
            });
            glib::timeout_add_local(50, move || {
                let pct = progress.load(Ordering::Relaxed);
                progress_bar.set_fraction(pct as f64 / 100.0);
                progress_bar.set_text(Some(&format!("{pct}%")));
                match rx.try_recv() {
                    Ok(Ok(msg)) => {
                        progress_bar.set_fraction(1.0);
                        progress_bar.set_text(Some("100%"));
                        up_status.set_text(&msg);
                        up_stack.set_visible_child_name("device");
                    }
                    Ok(Err(e)) => {
                        up_status.set_text(&format!("upload failed: {e}"));
                    }
                    Err(TryRecvError::Empty) => return glib::Continue(true),
                    Err(_) => return glib::Continue(false),
                }
                glib::Continue(false)
            });
        });
    }

    // ---- history ----
    {
        let upload_status = v.upload_status.clone();
        let stack = v.stack.clone();
        let history_list_click = v.history_list.clone();
        v.refresh_history_btn.connect_clicked(move |_| {
            refresh_history(&history_list_click);
        });
        // populate once at startup
        refresh_history(&v.history_list);

        // double-click a history row -> restore to keyboard
        let history_list_d = v.history_list.clone();
        let upload_status_d = upload_status.clone();
        let stack_d = stack.clone();
        history_list_d.connect_row_activated(move |_list, row| {
            let idx = row.get_index() as usize + 1; // rows are newest-first; index 1-based
            let payload = match history::load(idx) {
                Ok(p) => p,
                Err(e) => {
                    upload_status_d.set_text(&format!("history load failed: {e}"));
                    return;
                }
            };
            upload_status_d.set_text("restoring to keyboard…");
            let rx = run_on_worker(move || -> Result<(), String> {
                let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
                let mut dev = Device::open().map_err(|e| e.to_string())?;
                dev.upload(&payload, &mut |_, _| {}).map_err(|e| e.to_string())
            });
            let upload_status_d = upload_status_d.clone();
            let stack_d = stack_d.clone();
            when_done(rx, move |res| {
                match res {
                    Ok(()) => {
                        upload_status_d.set_text("restored to keyboard");
                        stack_d.set_visible_child_name("device");
                    }
                    Err(e) => upload_status_d.set_text(&format!("restore failed: {e}")),
                }
            });
        });
    }

    // ---- settings ----
    {
        let sw_autosync = v.sw_autosync.clone();
        let sw_restore = v.sw_restore.clone();
        let sw_sync_connect = v.sw_sync_connect.clone();
        let sw_sync_start = v.sw_sync_start.clone();
        let sw_check_updates = v.sw_check_updates.clone();
        let sw_start_login = v.sw_start_login.clone();
        let sw_minimize_tray = v.sw_minimize_tray.clone();
        let interval_entry = v.interval_entry.clone();
        let settings_status = v.settings_status.clone();
        v.save_settings_btn.connect_clicked(move |_| {
            let mut s = settings::Settings::load();
            s.autosync_clock = sw_autosync.get_active();
            s.restore_on_connect = sw_restore.get_active();
            s.sync_on_connect = sw_sync_connect.get_active();
            s.sync_on_start = sw_sync_start.get_active();
            s.check_updates = sw_check_updates.get_active();
            s.start_at_login = sw_start_login.get_active();
            s.minimize_to_tray = sw_minimize_tray.get_active();
            if let Ok(secs) = interval_entry.get_text().trim().parse::<u64>() {
                s.sync_interval_secs = secs;
            }
            match s.save() {
                Ok(()) => settings_status.set_text("settings saved"),
                Err(e) => settings_status.set_text(&format!("save failed: {e}")),
            }
        });
    }

    // ---- autosync background timer (60 s) ----
    {
        let s0 = settings::Settings::load();
        if s0.autosync_clock {
            glib::timeout_add_seconds_local(60, move || {
                let _ = (|| -> Result<(), String> {
                    let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
                    let mut dev = Device::open().map_err(|e| e.to_string())?;
                    let now = chrono::Local::now();
                    let t = CivilTime {
                        year: now.year() as u16,
                        month: now.month() as u8,
                        day: now.day() as u8,
                        hour: now.hour() as u8,
                        minute: now.minute() as u8,
                        second: now.second() as u8,
                        weekday: now.weekday().num_days_from_sunday() as u8,
                    };
                    dev.sync_clock(t).map_err(|e| e.to_string())
                })();
                glib::Continue(true)
            });
        }
    }
}

/// (Re)populate the history list box, newest first.
fn refresh_history(list: &gtk::ListBox) {
    // remove existing rows
    while let Some(child) = list.get_children().into_iter().next() {
        list.remove(&child);
    }
    for entry in history::list() {
        let row = gtk::ListBoxRow::new();
        let hbox = gtk::Box::new(Orientation::Horizontal, 8);
        let name = Label::new(Some(&format!("{}  ({} B)", entry.name, entry.bytes)));
        name.set_halign(gtk::Align::Start);
        hbox.pack_start(&name, true, true, 0);
        row.add(&hbox);
        row.show_all();
        list.add(&row);
    }
    list.show_all();
}
