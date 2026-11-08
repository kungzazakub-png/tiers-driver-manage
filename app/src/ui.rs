//! The window (egui). Presentation only: the engine, installer and checks are in the other modules.
use crate::backup::{self, Backup};
use crate::catalog::{Date, HostInfo, InstalledDriver, Offer};
use crate::engine::{self, Code, Config, InstallDeps, InvRow, ScanDeps, Shared};
use crate::http::Real;
use crate::sys::RealSys;
use crate::update::{self, Manifest};
use crate::{selfupdate, winsys, wu};
use eframe::egui::{self, Align, Color32, CornerRadius, Layout, Margin, RichText, Sense, Stroke, Vec2};
use egui_extras::{Column, TableBuilder};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO: &str = "kungzazakub-png/tiers-driver-manage";
pub const PUBLIC_KEY: &str = include_str!("../assets/UpdatePublicKey.xml");
pub const SHIPPED_CATALOG: &str = include_str!("../assets/driverdb.json");
const MAX_PASSES: u32 = 3;

// text colors keep >= 4.5:1 on white, the page and their tinted banners
pub const NAVY: Color32 = Color32::from_rgb(15, 37, 71);
pub const PAGE: Color32 = Color32::from_rgb(244, 246, 249);
pub const BORDER: Color32 = Color32::from_rgb(218, 223, 231);
pub const TEXT: Color32 = Color32::from_rgb(27, 39, 51);
pub const MUTED: Color32 = Color32::from_rgb(78, 90, 110);
pub const ACCENT: Color32 = Color32::from_rgb(37, 99, 235);
pub const ACCENT_DARK: Color32 = Color32::from_rgb(29, 78, 216);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(234, 241, 255);
pub const GOOD: Color32 = Color32::from_rgb(17, 116, 54);
pub const GOOD_SOFT: Color32 = Color32::from_rgb(232, 245, 236);
pub const WARN: Color32 = Color32::from_rgb(158, 70, 6);
pub const WARN_SOFT: Color32 = Color32::from_rgb(254, 243, 226);
pub const BAD: Color32 = Color32::from_rgb(185, 28, 28);
pub const BAD_SOFT: Color32 = Color32::from_rgb(253, 236, 236);
pub const DISABLED: Color32 = Color32::from_rgb(226, 232, 240);
pub const DISABLED_TEXT: Color32 = Color32::from_rgb(100, 116, 139);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab { Updates, Installed, Backups, Log }
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode { Review, Run, Results }
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind { Info, Good, Warn, Bad }
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon { Check, Cross, Info, Up, Dot, Ring, Minus, Shield, Clock }

pub struct Row { pub offer: Offer, pub ticked: bool, pub system: bool }
pub struct ResultRow { pub offer: Offer, pub code: Code, pub reason: String }
pub struct BannerBtn { pub id: &'static str, pub text: String, pub primary: bool, pub link: bool, pub enabled: bool }
pub struct Banner { pub kind: Kind, pub text: String, pub buttons: Vec<BannerBtn> }

enum Running { None, Scan(Shared), Install(Shared) }

pub struct App {
    http: Arc<Real>,
    cfg: Arc<Config>,
    host: HostInfo,
    machine: String,
    logo: Option<egui::TextureHandle>,
    pub tab: Tab,
    pub mode: Mode,
    pub rows: Vec<Row>,
    pub info_rows: Vec<InstalledDriver>,
    pub results: Vec<ResultRow>,
    pub inventory: Vec<InvRow>,
    pub missing_count: usize,
    offers: HashMap<String, Offer>,
    declined: HashSet<String>,
    running: Running,
    pub scanned: bool,
    pub status: String,
    pub banner: Option<Banner>,
    pub restore_point: bool,
    restore_done: bool,
    pub log: Vec<String>,
    need_reboot: bool,
    pass: u32,
    auto: bool,
    pub backups: Vec<Backup>,
    pub selected_backup: Option<usize>,
    confirm_rollback: Option<usize>,
    pub update: Option<Manifest>,
    update_rx: Option<Receiver<Result<Option<Manifest>, String>>>,
    apply_rx: Option<Receiver<Result<String, String>>>,
    updating: bool,
    screenshot: Option<(PathBuf, u32)>,
}

fn is_system_class(c: &str) -> bool { c.eq_ignore_ascii_case("SYSTEM") }

pub fn class_label(c: &str) -> String {
    match c.to_ascii_uppercase().as_str() {
        "DISPLAY" => "Graphics", "NET" => "Network", "MEDIA" => "Audio", "BLUETOOTH" => "Bluetooth", "SYSTEM" => "System / Chipset", "USB" => "USB",
        "HDC" | "SCSIADAPTER" => "Storage", "CAMERA" => "Camera", "IMAGE" => "Camera / Scanner", "MONITOR" => "Monitor", "HIDCLASS" => "Input device",
        "KEYBOARD" => "Keyboard", "MOUSE" => "Mouse", "PRINTER" => "Printer", "FIRMWARE" => "Firmware", "PROCESSOR" => "Processor",
        _ => return c.to_string(),
    }.to_string()
}

fn source_label(s: &str) -> String { if s == "Catalog" { "Your catalog".into() } else { s.to_string() } }

pub fn size_text(mb: Option<f64>) -> String {
    match mb {
        Some(v) if v > 0.0 => if v < 1.0 { format!("{:.0} KB", v * 1024.0) } else if v >= 1024.0 { format!("{:.1} GB", v / 1024.0) } else { format!("{v:.1} MB") },
        _ => String::new(),
    }
}

/// Intel INF drivers carry a fake 1968 date: show a dash instead of a misleading date.
pub fn date_text(d: &str) -> String {
    match Date::parse_iso(d) { Some(x) if x.y < 1990 => "—".into(), Some(x) => x.iso(), None => d.to_string() }
}

/// Technical failure text -> a sentence a PC owner can act on. The raw text stays in the tooltip and the log.
pub fn plain_reason(r: &str) -> String {
    let l = r.to_lowercase();
    if l.contains("sha") { "The downloaded file does not match what was expected (it may be incomplete). You can retry.".into() }
    else if l.contains("signer") || l.contains("authenticode") || l.contains("signature") { "The manufacturer signature check failed, so it was not installed for safety.".into() }
    else if l.contains("allowed list") { "The download source is not in the program's allowed list.".into() }
    else if l.contains("timed out") || l.contains("timeout") || l.contains("remote name") || l.contains("unable to connect") || l.contains("connection") { "Could not connect. Check your internet connection and try again.".into() }
    else { r.to_string() }
}

fn paint_icon(ui: &mut egui::Ui, icon: Icon, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let p = ui.painter();
    let c = rect.center();
    let s = Stroke::new((size / 8.0).max(1.5), color);
    let r = size * 0.38;
    let pt = |x: f32, y: f32| egui::pos2(rect.left() + size * x, rect.top() + size * y);
    match icon {
        Icon::Check => { p.line_segment([pt(0.18, 0.54), pt(0.42, 0.78)], s); p.line_segment([pt(0.42, 0.78), pt(0.84, 0.26)], s); }
        Icon::Cross => { p.line_segment([pt(0.24, 0.24), pt(0.76, 0.76)], s); p.line_segment([pt(0.76, 0.24), pt(0.24, 0.76)], s); }
        Icon::Info => { p.circle_stroke(c, r, s); p.line_segment([pt(0.5, 0.46), pt(0.5, 0.69)], s); p.circle_filled(pt(0.5, 0.32), size * 0.06, color); }
        Icon::Up => { p.line_segment([pt(0.5, 0.84), pt(0.5, 0.2)], s); p.line_segment([pt(0.24, 0.46), pt(0.5, 0.18)], s); p.line_segment([pt(0.76, 0.46), pt(0.5, 0.18)], s); }
        Icon::Dot => { p.circle_filled(c, size * 0.28, color); }
        Icon::Ring => { p.circle_stroke(c, size * 0.3, s); }
        Icon::Minus => { p.circle_stroke(c, r, s); p.line_segment([pt(0.32, 0.5), pt(0.68, 0.5)], s); }
        Icon::Shield => {
            let pts = vec![pt(0.5, 0.1), pt(0.84, 0.24), pt(0.84, 0.52), pt(0.5, 0.9), pt(0.16, 0.52), pt(0.16, 0.24)];
            p.add(egui::Shape::closed_line(pts, s));
            p.line_segment([pt(0.36, 0.5), pt(0.46, 0.6)], s); p.line_segment([pt(0.46, 0.6), pt(0.65, 0.38)], s);
        }
        Icon::Clock => { p.circle_stroke(c, r, s); p.line_segment([pt(0.5, 0.3), pt(0.5, 0.5)], s); p.line_segment([pt(0.5, 0.5), pt(0.66, 0.6)], s); }
    }
}

fn button(ui: &mut egui::Ui, text: &str, primary: bool, link: bool, enabled: bool, min: Vec2) -> egui::Response {
    let (fill, fg, stroke) = if !enabled { (if link { Color32::TRANSPARENT } else { DISABLED }, DISABLED_TEXT, Stroke::NONE) }
        else if primary { (ACCENT, Color32::WHITE, Stroke::NONE) }
        else if link { (Color32::TRANSPARENT, ACCENT_DARK, Stroke::NONE) }
        else { (Color32::WHITE, ACCENT_DARK, Stroke::new(1.0, Color32::from_rgb(170, 190, 235))) };
    let mut b = egui::Button::new(sbold(text).color(fg).size(sz(if primary { 14.5 } else { 13.5 }))).fill(fill).stroke(stroke).corner_radius(CornerRadius::same(6)).min_size(min);
    if link { b = b.frame(false); }
    ui.add_enabled(enabled, b).on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn iso_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{}T{:02}:{:02}:{:02}", Date::today().iso(), (secs / 3600) % 24, (secs / 60) % 60, secs % 60)
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config, screenshot: Option<PathBuf>) -> App {
        PPP.store(cc.egui_ctx.pixels_per_point().to_bits(), std::sync::atomic::Ordering::Relaxed);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::light();
        style.visuals.panel_fill = PAGE;
        style.visuals.override_text_color = Some(TEXT);
        style.visuals.selection.bg_fill = ACCENT_SOFT;
        style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        for (_, f) in style.text_styles.iter_mut() { if f.size < 13.0 { f.size = 13.0; } }
        if let Some(f) = style.text_styles.get_mut(&egui::TextStyle::Body) { f.size = 14.0; }
        for (_, f) in style.text_styles.iter_mut() { f.size = sz(f.size); }
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        style.visuals.text_cursor.stroke = Stroke::new(2.0, ACCENT);
        let hover = Color32::from_rgb(243, 246, 251);
        style.visuals.widgets.hovered.weak_bg_fill = hover; style.visuals.widgets.hovered.bg_fill = hover; style.visuals.widgets.active.weak_bg_fill = hover;
        cc.egui_ctx.set_style(style);
        // Segoe UI Variable (the Windows 11 system font) ships with Windows: the program looks native without bundling a font; Leelawadee covers Thai
        let mut fonts = egui::FontDefinitions::default();
        let mut semibold = vec![];
        for (name, file, is_semi) in [("segoe", "SegUIVar.ttf", false), ("segoe-classic", "segoeui.ttf", false), ("leelawadee", "LeelawUI.ttf", false), ("segoe-symbols", "seguisym.ttf", false), ("segoe-semibold", "seguisb.ttf", true)] {
            if let Ok(b) = std::fs::read(format!("C:\\Windows\\Fonts\\{file}")) {
                fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(b)));
                if is_semi { semibold.push(name.to_string()); } else { let f = fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap(); let at = f.iter().position(|n| !["segoe", "segoe-classic", "leelawadee", "segoe-symbols"].contains(&n.as_str())).unwrap_or(f.len()); f.insert(at, name.into()); }
            }
        }
        let mut chain = semibold; chain.extend(fonts.families[&egui::FontFamily::Proportional].clone());
        fonts.families.insert(egui::FontFamily::Name("semibold".into()), chain);
        cc.egui_ctx.set_fonts(fonts);
        let logo = image::load_from_memory(include_bytes!("../assets/tiers-logo.png")).ok().map(|img| {
            let rgba = img.to_rgba8();
            let (w, h) = (rgba.width() as usize, rgba.height() as usize);
            cc.egui_ctx.load_texture("logo", egui::ColorImage::from_rgba_unmultiplied([w, h], rgba.as_raw()), egui::TextureOptions::LINEAR)
        });
        let host = winsys::host_info();
        let mut app = App {
            http: Arc::new(Real::new()), cfg: Arc::new(cfg), host, machine: winsys::machine_summary(), logo,
            tab: Tab::Updates, mode: Mode::Review, rows: vec![], info_rows: vec![], results: vec![], inventory: vec![], missing_count: 0,
            offers: HashMap::new(), declined: HashSet::new(), running: Running::None, scanned: false, status: "Ready to scan".into(), banner: None,
            restore_point: true, restore_done: false, log: vec![], need_reboot: false, pass: 0, auto: false, backups: vec![], selected_backup: None,
            confirm_rollback: None, update: None, update_rx: None, apply_rx: None, updating: false, screenshot: screenshot.map(|p| (p, 0)),
        };
        if app.screenshot.is_none() { app.check_for_update(&cc.egui_ctx); }
        app
    }

    fn busy(&self) -> bool { !matches!(self.running, Running::None) || self.updating }

    fn say(&mut self, line: impl Into<String>) { self.log.push(line.into()); }

    // ---------- background work ----------
    fn check_for_update(&mut self, ctx: &egui::Context) {
        if self.cfg.repo.is_empty() { return; }
        let (tx, rx) = channel();
        let (http, repo, ctx2) = (self.http.clone(), self.cfg.repo.clone(), ctx.clone());
        std::thread::spawn(move || { let _ = tx.send(update::get_app_update(&repo, VERSION, PUBLIC_KEY, &*http)); ctx2.request_repaint(); });
        self.update_rx = Some(rx);
    }

    fn start_update(&mut self, ctx: &egui::Context) {
        let Some(m) = self.update.clone() else { return };
        if self.busy() { return; }
        self.updating = true;
        self.status = format!("Downloading and verifying the update to version {}...", m.version);
        let (tx, rx) = channel();
        let (http, ctx2) = (self.http.clone(), ctx.clone());
        std::thread::spawn(move || {
            let exe = std::env::current_exe().unwrap_or_default();
            let work = std::env::temp_dir().join(format!("TierS_upd_{}", std::process::id()));
            let r = selfupdate::apply(&m, &exe, &work, &*http, None);
            let _ = std::fs::remove_dir_all(&work);
            let _ = tx.send(r);
            ctx2.request_repaint();
        });
        self.apply_rx = Some(rx);
    }

    fn start_scan(&mut self, ctx: &egui::Context) {
        if self.busy() { return; }
        self.rows.clear(); self.info_rows.clear(); self.results.clear(); self.offers.clear(); self.declined.clear();
        self.log.clear(); self.need_reboot = false; self.pass = 0; self.auto = false; self.restore_done = false; self.banner = None; self.mode = Mode::Review;
        self.begin_scan(ctx);
    }

    fn begin_scan(&mut self, ctx: &egui::Context) {
        let job = engine::new_job();
        self.running = Running::Scan(job.clone());
        let (cfg, http, host, ctx2) = (self.cfg.clone(), self.http.clone(), self.host.clone(), ctx.clone());
        std::thread::spawn(move || {
            wu::init_com();
            let live = |id: &str| winsys::live_driver_version(id);
            let installed = || winsys::installed_drivers();
            let without = || winsys::devices_without_driver();
            let wu_search = || wu::search_drivers();
            let deps = ScanDeps { http: &*http, host, installed: &installed, without_driver: &without, live_version: &live, windows_update: &wu_search, today: Date::today() };
            engine::scan(&job, &cfg, &deps);
            ctx2.request_repaint();
        });
    }

    fn start_install(&mut self, ctx: &egui::Context) {
        if self.busy() || self.mode != Mode::Review { return; }
        let mut ids = Vec::new();
        let mut keep = Vec::new();
        for r in self.rows.drain(..) { if r.ticked { ids.push(r.offer.id.clone()); keep.push(r); } else { self.declined.insert(r.offer.id.clone()); } }
        self.info_rows.clear();
        self.rows = keep;
        if ids.is_empty() { return; }
        self.auto = true; self.pass = 1; self.mode = Mode::Run;
        let job = engine::new_job();
        self.running = Running::Install(job.clone());
        self.banner = Some(Banner { kind: Kind::Info, text: "Getting ready to install...".into(), buttons: vec![BannerBtn { id: "stop", text: "Stop after this item".into(), primary: false, link: false, enabled: true }] });
        let offers = self.offers.clone();
        let (cfg, http, ctx2, rp) = (self.cfg.clone(), self.http.clone(), ctx.clone(), self.restore_point && !self.restore_done);
        self.restore_done = true;
        std::thread::spawn(move || {
            wu::init_com();
            let sys = RealSys;
            let wu_fn = |ids: &[String], st: &dyn Fn(&str)| wu::install(ids, st);
            let rp_fn = || crate::restorepoint::create("TierS Driver Manage");
            let stamp = format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
            let d = InstallDeps { http: &*http, sys: &sys, wu_install: &wu_fn, create_restore_point: &rp_fn, work_root: std::env::temp_dir(), backup_root: backup::backup_root(), now: iso_now(), stamp };
            engine::install(&job, &cfg, &d, &ids, &offers, rp);
            ctx2.request_repaint();
        });
    }

    // ---------- polling ----------
    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.update_rx {
            if let Ok(r) = rx.try_recv() {
                self.update_rx = None;
                match r { Ok(Some(m)) => self.update = Some(m), Ok(None) => {}, Err(e) => self.say(format!("Program update check: {e}")) }
            }
        }
        if let Some(rx) = &self.apply_rx {
            if let Ok(r) = rx.try_recv() {
                self.apply_rx = None; self.updating = false;
                match r {
                    Ok(msg) => {
                        self.say(msg.clone());
                        self.status = format!("Done: {msg}. Restarting the program...");
                        if let Ok(exe) = std::env::current_exe() { let _ = std::process::Command::new(exe).arg("--wait-for-previous").spawn(); }
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    Err(e) => { self.say(format!("[FAILED] Program update: {e}")); self.status = "Stopped: the program update failed (your existing files were not changed). See the log for details.".into(); }
                }
            }
        }
        let finished = match &self.running {
            Running::Scan(j) | Running::Install(j) => { let g = j.lock().unwrap(); if !g.status.is_empty() && !g.done { self.status = g.status.clone(); } g.done }
            Running::None => false,
        };
        if matches!(self.running, Running::Install(_)) && !finished { self.refresh_run_banner(); }
        if finished {
            match std::mem::replace(&mut self.running, Running::None) {
                Running::Scan(j) => self.finish_scan(j, ctx),
                Running::Install(j) => self.finish_install(j, ctx),
                Running::None => {}
            }
        }
    }

    fn refresh_run_banner(&mut self) {
        let Running::Install(j) = &self.running else { return };
        let g = j.lock().unwrap();
        let total = self.rows.len();
        let done = self.rows.iter().filter(|r| matches!(g.items.get(&r.offer.id).map(|s| s.code), Some(Code::Ok | Code::Fail | Code::Skip))).count();
        let stopping = g.cancel;
        let text = if stopping { format!("Stopping after this item · {done} of {total} done") } else { format!("Installing {done} of {total} · do not close the program or turn off the PC") };
        self.banner = Some(Banner { kind: Kind::Info, text, buttons: vec![BannerBtn { id: "stop", text: if stopping { "Stopping...".into() } else { "Stop after this item".into() }, primary: false, link: false, enabled: !stopping }] });
    }

    fn finish_scan(&mut self, j: Shared, ctx: &egui::Context) {
        let g = j.lock().unwrap();
        for l in &g.log { self.log.push(l.clone()); }
        if let Some(e) = &g.error {
            self.status = format!("Stopped: {e}"); self.log.push(format!("Stopped: {e}"));
            drop(g);
            if !self.results.is_empty() { self.show_results(); } else { self.tab = Tab::Log; }
            return;
        }
        let done_ids: HashSet<String> = self.results.iter().map(|r| r.offer.id.clone()).collect();
        let items: Vec<Offer> = g.offers.iter().filter(|o| !self.declined.contains(&o.id) && !done_ids.contains(&o.id)).cloned().collect();
        self.rows = items.iter().map(|o| { let system = is_system_class(&o.class); Row { offer: o.clone(), ticked: !system, system } }).collect();
        for o in &items { self.offers.insert(o.id.clone(), o.clone()); }
        self.info_rows = g.unresolved.clone();
        self.inventory = { let mut v = g.inventory.clone(); v.sort_by(|a, b| b.age.unwrap_or(-1.0).partial_cmp(&a.age.unwrap_or(-1.0)).unwrap()); v };
        self.missing_count = g.missing.len();
        let old = self.inventory.iter().filter(|r| r.age.map(|a| a > self.cfg.old_years).unwrap_or(false) && !r.maker.starts_with("Microsoft")).count();
        self.log.push(format!("Installed drivers: {}; older than {} years (Microsoft drivers not counted): {old}", self.inventory.len(), self.cfg.old_years));
        if !g.missing.is_empty() { self.log.push(format!("Devices without a working driver ({}):\n  - {}", g.missing.len(), g.missing.join("\n  - "))); }
        self.scanned = true;
        self.mode = Mode::Review;
        let status = g.status.clone();
        drop(g);
        self.auto = false;
        if items.is_empty() {
            self.log.push("Scan complete: no newer drivers found from Windows Update or your catalog".into());
            if !self.results.is_empty() { self.show_results(); return; }
            self.status = "Scan complete: no newer drivers found".into();
            self.banner = None;
        } else {
            self.status = status;
            let ok = self.results.iter().filter(|r| r.code == Code::Ok).count();
            self.banner = if ok > 0 { Some(Banner { kind: Kind::Info, text: format!("Installed {ok} · {} more found from newly detected hardware. Review the list and click Install again.", self.rows.len()), buttons: vec![] }) }
                else if self.rows.iter().any(|r| r.system) { Some(Banner { kind: Kind::Warn, text: "Chipset and system drivers are not ticked because they affect the whole PC and most do not need updating. Tick them yourself if you want them.".into(), buttons: vec![] }) } else { None };
            self.tab = Tab::Updates;
        }
        ctx.request_repaint();
    }

    fn finish_install(&mut self, j: Shared, ctx: &egui::Context) {
        let (installed, reboot, cancel, error) = {
            let g = j.lock().unwrap();
            for l in &g.log { self.log.push(l.clone()); }
            let ids: HashSet<String> = self.rows.iter().map(|r| r.offer.id.clone()).collect();
            self.results.retain(|r| !ids.contains(&r.offer.id));
            for r in &self.rows {
                let st = g.items.get(&r.offer.id).cloned().unwrap_or(engine::ItemState { code: Code::Skip, reason: "Not installed".into() });
                let (code, reason) = if matches!(st.code, Code::Ok | Code::Fail | Code::Skip) { (st.code, st.reason) } else { (Code::Skip, "Not installed".into()) };
                self.results.push(ResultRow { offer: r.offer.clone(), code, reason });
            }
            if let Some(e) = &g.error { self.log.push(format!("Stopped: {e}")); }
            (g.installed, g.reboot, g.cancel, g.error.clone())
        };
        if reboot { self.need_reboot = true; }
        self.rows.clear();
        if error.is_none() && self.auto && installed > 0 && !cancel && self.pass < MAX_PASSES {
            self.pass += 1;
            self.log.push(format!("--- Round {}: scanning again in case newly detected hardware needs drivers ---", self.pass));
            self.show_results();
            self.banner = Some(Banner { kind: Kind::Info, text: "This install round is finished · scanning again in case newly detected hardware needs drivers".into(), buttons: vec![] });
            self.begin_scan(ctx);
            return;
        }
        self.show_results();
        if let Some(e) = error { self.status = format!("Stopped: {e}"); }
    }

    pub fn show_results(&mut self) {
        self.mode = Mode::Results; self.tab = Tab::Updates; self.auto = false;
        let (mut ok, mut fail, mut skip) = (0, 0, 0);
        for r in &self.results { match r.code { Code::Ok => ok += 1, Code::Fail => fail += 1, _ => skip += 1 } }
        let mut parts = vec![format!("Installed {ok}")];
        if skip > 0 { parts.push(format!("Skipped {skip}")); }
        if fail > 0 { parts.push(format!("Failed {fail}")); }
        let mut text = parts.join(" · ");
        if self.need_reboot { text.push_str(" · A restart is needed for the drivers to work fully"); } else if fail == 0 && skip == 0 { text.push_str(" · Finished, no restart needed"); }
        let kind = if fail > 0 && ok == 0 { Kind::Bad } else if fail > 0 || skip > 0 || self.need_reboot { Kind::Warn } else { Kind::Good };
        let b = |id: &'static str, t: &str, primary: bool, link: bool| BannerBtn { id, text: t.into(), primary, link, enabled: true };
        let mut buttons = vec![];
        if self.need_reboot { buttons.push(b("restart", "Restart now", true, false)); buttons.push(b("later", "Later", false, true)); }
        if fail > 0 { buttons.push(b("retry", "Retry failed ones", !self.need_reboot, false)); }
        if !self.need_reboot && !self.results.is_empty() { buttons.push(b("restart", "Restart computer", false, true)); }
        if !self.need_reboot && fail == 0 { buttons.push(b("rescan", "Scan again", false, true)); }
        self.banner = Some(if self.results.is_empty() { Banner { kind: Kind::Good, text: "Nothing needs to be installed".into(), buttons: vec![] } } else { Banner { kind, text, buttons } });
        self.status = "Done".into();
    }

    fn banner_click(&mut self, id: &str, ctx: &egui::Context) {
        match id {
            "stop" => { if let Running::Install(j) = &self.running { j.lock().unwrap().cancel = true; } }
            "restart" => {
                let _ = std::process::Command::new("shutdown.exe").args(["/r", "/t", "30", "/c", "TierS Driver Manage: restart to finish installing drivers"]).spawn();
                self.banner = Some(Banner { kind: Kind::Warn, text: "The computer will restart in 30 seconds · save any open work".into(), buttons: vec![BannerBtn { id: "abort", text: "Cancel restart".into(), primary: true, link: false, enabled: true }] });
            }
            "abort" => {
                let _ = std::process::Command::new("shutdown.exe").arg("/a").spawn();
                self.banner = Some(Banner { kind: Kind::Warn, text: "Restart cancelled · drivers will work fully after a restart".into(), buttons: vec![BannerBtn { id: "restart", text: "Restart now".into(), primary: true, link: false, enabled: true }] });
            }
            "later" => self.banner = Some(Banner { kind: Kind::Warn, text: "Not restarted yet · installed drivers will work fully after a restart".into(), buttons: vec![BannerBtn { id: "restart", text: "Restart now".into(), primary: true, link: false, enabled: true }] }),
            "retry" => self.retry_failed(ctx),
            "rescan" => self.start_scan(ctx),
            _ => {}
        }
    }

    /// Puts the failed drivers back in the list, ticked, and installs them again.
    fn retry_failed(&mut self, ctx: &egui::Context) {
        let failed: Vec<Offer> = self.results.iter().filter(|r| r.code == Code::Fail).map(|r| r.offer.clone()).collect();
        if failed.is_empty() { return; }
        self.rows = failed.into_iter().map(|o| Row { offer: o, ticked: true, system: false }).collect();
        self.mode = Mode::Review;
        self.start_install(ctx);
    }

    fn refresh_backups(&mut self) {
        self.backups = backup::list_backups(&backup::backup_root());
        if self.selected_backup.map(|i| i >= self.backups.len()).unwrap_or(false) { self.selected_backup = None; }
    }

    fn do_rollback(&mut self, i: usize) {
        let Some(b) = self.backups.get(i).cloned() else { return };
        let (ok, msg) = match backup::restore_device_driver(&b, &RealSys) { Ok(m) => (true, m), Err(e) => (false, e) };
        self.say(format!("{} Roll back {}: {msg}", if ok { "[OK]" } else { "[FAILED]" }, b.entry.device));
        self.status = format!("{} {msg}", if ok { "Done:" } else { "Stopped:" });
        self.refresh_backups();
    }

    // ---------- drawing ----------
    fn selected(&self) -> (usize, f64) { self.rows.iter().filter(|r| r.ticked).fold((0, 0.0), |(n, s), r| (n + 1, s + r.offer.size_mb.unwrap_or(0.0))) }

    fn header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            if let Some(t) = &self.logo { ui.add(egui::Image::new(t).fit_to_exact_size(Vec2::splat(34.0))); }
            ui.add_space(6.0);
            ui.label(sbold("TierS Driver Manage").size(sz(18.0)).color(NAVY));
            ui.add_space(14.0);
            let (r, _) = ui.allocate_exact_size(Vec2::new(1.0, 20.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::ZERO, BORDER);
            ui.add_space(10.0);
            ui.label(RichText::new(&self.machine).color(MUTED).size(sz(13.0)));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(14.0);
                ui.label(RichText::new(format!("v{VERSION}")).color(MUTED).size(sz(13.0)));
                if let Some(m) = self.update.clone() {
                    ui.add_space(8.0);
                    if button(ui, &format!("Update to {}", m.version), false, false, !self.busy(), Vec2::new(0.0, 30.0)).on_hover_text(m.notes.clone()).clicked() { self.start_update(ctx); }
                }
            });
        });
    }

    fn action_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.add_space(20.0);
            if button(ui, "Scan drivers", true, false, !self.busy(), Vec2::new(168.0, 40.0)).on_hover_text("F5").clicked() { self.start_scan(ctx); }
            ui.add_space(8.0);
            let (icon, color) = if self.status.starts_with("Stopped") { (Some(Icon::Cross), BAD) } else if self.status.starts_with("Done") || self.status.starts_with("Scan complete") { (Some(Icon::Check), GOOD) } else if self.busy() { (Some(Icon::Dot), ACCENT) } else { (None, MUTED) };
            if let Some(i) = icon { paint_icon(ui, i, color, 18.0); }
            ui.label(RichText::new(&self.status).color(if color == MUTED { MUTED } else { color }).size(sz(14.0)));
        });
        ui.add_space(10.0);
        if self.busy() { ui.add(egui::ProgressBar::new(self.run_fraction()).desired_height(3.0).corner_radius(CornerRadius::ZERO).animate(true)); } else { ui.add_space(3.0); }
    }

    fn run_fraction(&self) -> f32 {
        if let (Mode::Run, Running::Install(j)) = (self.mode, &self.running) {
            let g = j.lock().unwrap();
            let total = self.rows.len().max(1) as f32;
            let done = self.rows.iter().filter(|r| matches!(g.items.get(&r.offer.id).map(|s| s.code), Some(Code::Ok | Code::Fail | Code::Skip))).count() as f32;
            return done / total;
        }
        0.0
    }

    /// One bordered, clickable summary pill; returns true when clicked.
    fn pill(ui: &mut egui::Ui, icon: Icon, color: Color32, text: String) -> bool {
        let r = egui::Frame::new().fill(Color32::WHITE).stroke(Stroke::new(1.0, BORDER)).corner_radius(CornerRadius::same(14)).inner_margin(Margin::symmetric(10, 4)).show(ui, |ui| {
            ui.horizontal(|ui| { paint_icon(ui, icon, color, 16.0); ui.label(sbold(text).size(sz(13.0)).color(TEXT)); });
        }).response.interact(Sense::click());
        if r.hovered() { ui.painter().rect_stroke(r.rect, CornerRadius::same(14), Stroke::new(1.0, ACCENT), egui::StrokeKind::Inside); }
        r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    }

    /// Tabs on the left (sitting on the content card), status pills on the right.
    fn tabs_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_space(20.0);
            for (t, name) in [(Tab::Updates, "Updates"), (Tab::Installed, "Installed drivers"), (Tab::Backups, "Backups"), (Tab::Log, "Log")] {
                let active = self.tab == t;
                let font = egui::FontId::new(sz(14.5), egui::FontFamily::Name("semibold".into()));
                let color = if active { ACCENT } else { MUTED };
                let w = ui.painter().layout_no_wrap(name.into(), font.clone(), color).size().x + 28.0;
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 40.0), Sense::click());
                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                ui.painter().text(egui::pos2(rect.center().x, rect.center().y - 1.0), egui::Align2::CENTER_CENTER, name, font, if resp.hovered() && !active { TEXT } else { color });
                if active { ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(rect.left() + 8.0, rect.bottom() - 3.0), egui::pos2(rect.right() - 8.0, rect.bottom())), CornerRadius::same(1), ACCENT); }
                if resp.clicked() { self.tab = t; if t == Tab::Backups { self.refresh_backups(); } }
                ui.add_space(-4.0);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(20.0);
                if !self.scanned { if Self::pill(ui, Icon::Ring, MUTED, "Not scanned yet".into()) { self.tab = Tab::Installed; } return; }
                let (icon, color, text) = if self.missing_count > 0 { (Icon::Info, BAD, format!("{} without a driver", self.missing_count)) } else { (Icon::Check, GOOD, "No problem devices".to_string()) };
                if Self::pill(ui, icon, color, text) { self.tab = Tab::Log; }
                let tickable = self.rows.len();
                let (icon, color, text) = match self.mode {
                    Mode::Review if tickable > 0 => (Icon::Up, ACCENT, format!("{tickable} to install")),
                    Mode::Review => (Icon::Check, GOOD, "Up to date".to_string()),
                    Mode::Run => (Icon::Dot, ACCENT, format!("Installing {tickable}")),
                    Mode::Results => {
                        let ok = self.results.iter().filter(|r| r.code == Code::Ok).count(); let bad = self.results.len() - ok;
                        if bad > 0 { (Icon::Info, WARN, format!("{ok} installed · {bad} not done")) } else { (Icon::Check, GOOD, format!("{ok} installed")) }
                    }
                };
                if Self::pill(ui, icon, color, text) { self.tab = Tab::Updates; }
                if Self::pill(ui, Icon::Dot, MUTED, format!("{} drivers", self.inventory.len())) { self.tab = Tab::Installed; }
            });
        });
    }

    fn banner_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(b) = &self.banner else { return };
        let (bg, fg, icon) = match b.kind { Kind::Info => (ACCENT_SOFT, ACCENT, Icon::Dot), Kind::Good => (GOOD_SOFT, GOOD, Icon::Check), Kind::Warn => (WARN_SOFT, WARN, Icon::Info), Kind::Bad => (BAD_SOFT, BAD, Icon::Cross) };
        let text = b.text.clone();
        let btns: Vec<(&'static str, String, bool, bool, bool)> = b.buttons.iter().map(|x| (x.id, x.text.clone(), x.primary, x.link, x.enabled)).collect();
        let mut clicked: Option<&'static str> = None;
        egui::Frame::new().fill(bg).inner_margin(Margin::symmetric(16, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                paint_icon(ui, icon, fg, 20.0);
                ui.add(egui::Label::new(RichText::new(text).size(sz(14.0)).color(TEXT)).wrap());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    for (id, t, primary, link, enabled) in &btns { if button(ui, t, *primary, *link, *enabled, Vec2::new(0.0, 34.0)).clicked() { clicked = Some(*id); } }
                });
            });
        });
        let r = ui.min_rect();
        ui.painter().hline(r.x_range(), ui.cursor().top() - 0.5, Stroke::new(1.0, BORDER));
        if let Some(id) = clicked { self.banner_click(id, ctx); }
    }

    fn updates_table(&mut self, ui: &mut egui::Ui) {
        let mode = self.mode;
        let results_mode = mode != Mode::Review;
        let running_items: HashMap<String, engine::ItemState> = if let Running::Install(j) = &self.running { j.lock().unwrap().items.clone() } else { HashMap::new() };
        let mut ticks: Vec<(usize, bool)> = vec![];
        let avail = ui.available_height();
        let painter = ui.painter().clone();
        TableBuilder::new(ui).striped(false).resizable(false).cell_layout(Layout::left_to_right(Align::Center)).max_scroll_height(avail)
            .column(Column::exact(if results_mode { 12.0 } else { 40.0 })).column(Column::exact(30.0)).column(Column::remainder().at_least(220.0).clip(true))
            .column(Column::exact(110.0)).column(Column::exact(120.0)).column(Column::exact(90.0)).column(Column::exact(70.0)).column(Column::exact(280.0))
            .header(34.0, |mut h| {
                for t in ["", "", "Driver / device", "Type", "Source", "Date", "Size", if results_mode { "Status" } else { "Notes" }] { h.col(|ui| th(ui, t)); }
            })
            .body(|body| {
                let n_rows = if mode == Mode::Results { self.results.len() } else { self.rows.len() };
                let infos = if mode == Mode::Review { self.info_rows.len() } else { 0 };
                body.rows(46.0, n_rows + infos, |mut row| {
                    let i = row.index();
                    if i >= n_rows {
                        let d = &self.info_rows[i - n_rows];
                        row.col(|_| {}); row.col(|ui| { ui.add_space(6.0); paint_icon(ui, Icon::Info, MUTED, 18.0); });
                        row.col(|ui| { ui.label(&d.name).on_hover_text(format!("{}\nHardware ID: {}\nNo driver was found in Windows Update or the Microsoft Catalog. Try the manufacturer's website using this Hardware ID.", d.name, d.hardware_ids.first().cloned().unwrap_or_default())); });
                        row.col(|ui| { ui.label(class_label(&d.class)); }); row.col(|ui| { ui.label("—"); }); row.col(|_| {}); row.col(|_| {});
                        row.col(|ui| { ui.label(sbold("No driver found").color(MUTED)); });
                        row_divider(&painter, &row.response());
                        return;
                    }
                    let (offer, code, reason, ticked) = if mode == Mode::Results { let r = &self.results[i]; (&r.offer, Some(r.code), r.reason.clone(), false) } else {
                        let r = &self.rows[i];
                        let st = running_items.get(&r.offer.id);
                        (&r.offer, if mode == Mode::Run { Some(st.map(|s| s.code).unwrap_or(Code::Wait)) } else { None }, st.map(|s| s.reason.clone()).unwrap_or_default(), r.ticked)
                    };
                    row.col(|ui| { if !results_mode { ui.add_space(12.0); let mut t = ticked; if check(ui, &mut t).changed() { ticks.push((i, t)); } } });
                    row.col(|ui| { ui.add_space(6.0); match code {
                        Some(Code::Ok) => paint_icon(ui, Icon::Check, GOOD, 18.0), Some(Code::Fail) => paint_icon(ui, Icon::Cross, BAD, 18.0), Some(Code::Skip) => paint_icon(ui, Icon::Minus, WARN, 18.0),
                        Some(Code::Dl | Code::Inst) => paint_icon(ui, Icon::Dot, ACCENT, 18.0), Some(Code::Wait) => paint_icon(ui, Icon::Ring, MUTED, 18.0),
                        None => if is_system_class(&offer.class) { paint_icon(ui, Icon::Shield, WARN, 18.0) },
                    } });
                    row.col(|ui| { ui.label(RichText::new(&offer.title).size(sz(14.0))).on_hover_text(&offer.title); });
                    row.col(|ui| { ui.label(class_label(&offer.class)); });
                    row.col(|ui| { ui.label(source_label(&offer.source)); });
                    row.col(|ui| { ui.label(date_text(&offer.date)); });
                    row.col(|ui| { ui.label(size_text(offer.size_mb)); });
                    row.col(|ui| {
                        let (text, color) = match code {
                            Some(Code::Wait) => ("Queued".to_string(), MUTED), Some(Code::Dl) => (format!("Downloading ({})", size_text(offer.size_mb)), ACCENT), Some(Code::Inst) => ("Installing".into(), ACCENT),
                            Some(Code::Ok) => ("Installed".into(), GOOD), Some(Code::Fail) => (if reason.is_empty() { "Failed".into() } else { format!("Failed: {}", plain_reason(&reason)) }, BAD),
                            Some(Code::Skip) => (if reason.is_empty() { "Skipped".into() } else { format!("Skipped: {}", plain_reason(&reason)) }, WARN),
                            None => if is_system_class(&offer.class) { ("Review first: affects the whole PC".into(), WARN) } else { (String::new(), MUTED) },
                        };
                        let l = ui.add(egui::Label::new(sbold(&text).size(sz(12.5)).color(color)).wrap());
                        if !reason.is_empty() || is_system_class(&offer.class) { l.on_hover_text(if reason.is_empty() { text.clone() } else { format!("{text}\n({reason})") }); }
                    });
                    row_divider(&painter, &row.response());
                });
            });
        for (i, t) in ticks { if let Some(r) = self.rows.get_mut(i) { r.ticked = t; } }
    }

    fn installed_table(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_height();
        let painter = ui.painter().clone();
        TableBuilder::new(ui).striped(false).sense(Sense::click()).cell_layout(Layout::left_to_right(Align::Center)).max_scroll_height(avail)
            .column(Column::exact(40.0)).column(Column::remainder().at_least(220.0).clip(true)).column(Column::exact(130.0)).column(Column::exact(180.0)).column(Column::exact(130.0)).column(Column::exact(110.0)).column(Column::exact(130.0))
            .header(34.0, |mut h| { for t in ["", "Device", "Type", "Manufacturer", "Version", "Driver date", "Status"] { h.col(|ui| th(ui, t)); } })
            .body(|body| {
                let years = self.cfg.old_years;
                body.rows(40.0, self.inventory.len(), |mut row| {
                    let d = &self.inventory[row.index()];
                    let old = d.age.map(|a| a > years).unwrap_or(false) && !d.maker.starts_with("Microsoft");
                    row.col(|ui| { ui.add_space(12.0); if old { paint_icon(ui, Icon::Clock, WARN, 18.0) } });
                    row.col(|ui| { ui.label(&d.name).on_hover_text(&d.name); }); row.col(|ui| { ui.label(class_label(&d.class)); }); row.col(|ui| { ui.label(&d.maker); });
                    row.col(|ui| { ui.label(&d.version); }); row.col(|ui| { ui.label(d.date.map(|x| if x.y < 1990 { "—".to_string() } else { x.iso() }).unwrap_or_default()); });
                    row.col(|ui| { if old { ui.label(sbold(format!("Old ({:.1} yr)", d.age.unwrap_or(0.0))).color(WARN)); } else if d.age.is_some() { ui.label(RichText::new("OK").color(MUTED)); } });
                    row_divider(&painter, &row.response());
                });
            });
    }

    fn backups_table(&mut self, ui: &mut egui::Ui) {
        if self.backups.is_empty() {
            ui.centered_and_justified(|ui| { ui.label(RichText::new("No backups yet\nBefore a driver is replaced, the one it replaces is saved here so you can roll back.").size(sz(15.0)).color(MUTED)); });
            return;
        }
        let avail = ui.available_height();
        let painter = ui.painter().clone();
        let mut pick = None;
        TableBuilder::new(ui).striped(false).sense(Sense::click()).cell_layout(Layout::left_to_right(Align::Center)).max_scroll_height(avail)
            .column(Column::exact(180.0)).column(Column::remainder().at_least(240.0).clip(true)).column(Column::exact(170.0)).column(Column::exact(150.0))
            .header(34.0, |mut h| { for t in ["Backed up", "Device", "Saved version", "Backup"] { h.col(|ui| th(ui, t)); } })
            .body(|body| {
                body.rows(42.0, self.backups.len(), |mut row| {
                    let i = row.index(); let b = &self.backups[i].entry;
                    row.set_selected(self.selected_backup == Some(i));
                    row.col(|ui| { ui.add_space(12.0); ui.label(b.time.replace('T', " ").chars().take(16).collect::<String>()); });
                    row.col(|ui| { ui.label(&b.device); }); row.col(|ui| { ui.label(&b.old_version); });
                    row.col(|ui| { ui.label(if b.folder.is_empty() { "Built-in driver" } else { "Saved copy" }); });
                    row_divider(&painter, &row.response());
                    if row.response().clicked() { pick = Some(i); }
                });
            });
        if pick.is_some() { self.selected_backup = pick; }
    }

    fn footer(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.add_space(20.0);
            let busy = self.busy();
            let show_sel = self.tab != Tab::Backups && self.mode == Mode::Review && !self.rows.is_empty() && !busy;
            if show_sel {
                let mut a = self.rows.iter().all(|r| r.ticked);
                if check(ui, &mut a).changed() { for r in self.rows.iter_mut() { r.ticked = a; } }
                let (n, sz) = self.selected();
                ui.label(RichText::new(format!("{n} of {} selected{}", self.rows.len(), if sz > 0.0 { format!(" · {}", size_text(Some(sz))) } else { String::new() })).color(MUTED));
                ui.add_space(18.0);
                let (r, _) = ui.allocate_exact_size(Vec2::new(1.0, 20.0), Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::ZERO, BORDER);
                ui.add_space(10.0);
            }
            ui.add_enabled_ui(!busy, |ui| {
                check(ui, &mut self.restore_point).on_hover_text("A restore point lets you roll the PC back if a new driver causes problems.\nWindows allows one every 24 hours; if it cannot be created the program logs it and continues.");
                ui.label(RichText::new("Create a restore point first").color(if busy { DISABLED_TEXT } else { TEXT }));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(20.0);
                if self.tab == Tab::Backups {
                    if button(ui, "Roll back selected", true, false, self.selected_backup.is_some() && !busy, Vec2::new(200.0, 42.0)).clicked() { self.confirm_rollback = self.selected_backup; }
                } else if self.mode == Mode::Review && !self.rows.is_empty() && !busy {
                    let (n, _) = self.selected();
                    if button(ui, &if n > 0 { format!("Install {n} selected") } else { "Install".into() }, true, false, n > 0, Vec2::new(200.0, 42.0)).clicked() { self.start_install(ctx); }
                }
            });
        });
    }

    /// Fills the window with demo data for the screenshot tool (developer only).
    pub fn demo(&mut self, state: &str) {
        let mk = |id: &str, title: &str, class: &str, source: &str, date: &str, mb: f64| Offer { id: id.into(), title: title.into(), class: class.into(), maker: "x".into(), date: date.into(), size_mb: Some(mb), source: source.into(), instance_id: String::new(), device_name: String::new(), kind: crate::catalog::OfferKind::WindowsUpdate { update_id: id.into() } };
        let offers = vec![
            mk("a", "Intel(R) Management Engine Interface #1: Intel System Driver Update", "SYSTEM", "Microsoft Catalog", "2026-01-12", 0.23),
            mk("b", "Intel(R) SPI (flash) Controller - 7A24: INTEL Driver Update", "SYSTEM", "Microsoft Catalog", "1968-07-17", 0.02),
            mk("c", "Realtek Gaming 2.5GbE Family Controller 1125.31.0.1", "NET", "Catalog", "2026-09-02", 12.4),
            mk("d", "Intel Wi-Fi Driver 24.90.0.1", "NET", "Catalog", "2026-11-01", 72.3),
            mk("e", "NVIDIA GeForce Game Ready Driver 700.01", "DISPLAY", "NVIDIA", "2026-11-05", 989.2),
            mk("f", "Creative Sound Blaster G3 - Driver Update", "MEDIA", "Windows Update", "2026-06-10", 38.5),
        ];
        self.scanned = true;
        self.status = "Scan complete: 6 driver updates found".into();
        self.inventory = (0..203).map(|i| InvRow { name: format!("Device {i}"), class: "NET".into(), maker: "Intel".into(), version: "1.0".into(), date: None, age: Some((i % 7) as f64) }).collect();
        self.rows = offers.iter().map(|o| { let s = is_system_class(&o.class); Row { offer: o.clone(), ticked: !s, system: s } }).collect();
        self.info_rows = vec![InstalledDriver { instance_id: "x".into(), name: "Canon MG3600 series".into(), class: "USB".into(), maker: String::new(), version: String::new(), date: None, hardware_ids: vec!["USB\\VID_04A9&PID_1830".into()] }];
        self.banner = Some(Banner { kind: Kind::Warn, text: "Chipset and system drivers are not ticked because they affect the whole PC and most do not need updating. Tick them yourself if you want them.".into(), buttons: vec![] });
        if state == "results" {
            self.results = offers.iter().skip(2).take(4).enumerate().map(|(i, o)| ResultRow { offer: o.clone(), code: [Code::Ok, Code::Ok, Code::Fail, Code::Skip][i], reason: if i == 2 { "SHA-256 mismatch (got X)".into() } else if i == 3 { "Stopped by request".into() } else { String::new() } }).collect();
            self.rows.clear(); self.need_reboot = true; self.show_results();
        }
    }
}

static PPP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f800000);
/// Font sizes are snapped to whole screen pixels: fractional glyph sizes blur the stems.
fn sz(v: f32) -> f32 { let p = f32::from_bits(PPP.load(std::sync::atomic::Ordering::Relaxed)).max(0.5); ((v * p).round() / p).max(1.0 / p) }

fn sbold(t: impl Into<String>) -> RichText { RichText::new(t.into()).family(egui::FontFamily::Name("semibold".into())) }

/// Square check box: white with a border, accent fill and a white tick when on.
fn check(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
    if resp.clicked() { *on = !*on; resp.mark_changed(); }
    let r = egui::Rect::from_center_size(rect.center(), Vec2::splat(18.0));
    let p = ui.painter();
    if *on {
        p.rect_filled(r, CornerRadius::same(4), ACCENT);
        let s = Stroke::new(2.0, Color32::WHITE);
        p.line_segment([egui::pos2(r.left() + 4.5, r.center().y), egui::pos2(r.left() + 7.8, r.bottom() - 5.0)], s);
        p.line_segment([egui::pos2(r.left() + 7.8, r.bottom() - 5.0), egui::pos2(r.right() - 4.0, r.top() + 5.0)], s);
    } else {
        p.rect_filled(r, CornerRadius::same(4), Color32::WHITE);
        p.rect_stroke(r, CornerRadius::same(4), Stroke::new(1.5, if resp.hovered() { ACCENT } else { Color32::from_rgb(148, 163, 184) }), egui::StrokeKind::Inside);
    }
    if !ui.is_enabled() { p.rect_filled(r, CornerRadius::same(4), Color32::from_white_alpha(140)); }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Column header cell: muted semibold text with a line under it.
fn th(ui: &mut egui::Ui, t: &str) {
    ui.label(sbold(t).size(sz(12.5)).color(MUTED));
    let r = ui.max_rect();
    ui.painter().hline(r.x_range().expand(4.0), r.bottom(), Stroke::new(1.0, BORDER));
}

/// Hairline under a table row plus a soft hover tint.
fn row_divider(painter: &egui::Painter, resp: &egui::Response) {
    let r = resp.rect;
    if resp.hovered() { painter.rect_filled(r, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(37, 99, 235, 10)); }
    painter.hline(r.x_range(), r.bottom(), Stroke::new(1.0, Color32::from_rgb(232, 236, 242)));
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        if self.busy() { ctx.request_repaint_after(std::time::Duration::from_millis(250)); }
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) && !self.busy() { self.start_scan(ctx); }
        for (k, t) in [(egui::Key::Num1, Tab::Updates), (egui::Key::Num2, Tab::Installed), (egui::Key::Num3, Tab::Backups), (egui::Key::Num4, Tab::Log)] {
            if ctx.input(|i| i.modifiers.command && i.key_pressed(k)) { self.tab = t; if t == Tab::Backups { self.refresh_backups(); } }
        }

        egui::TopBottomPanel::top("header").frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(Margin::symmetric(0, 10)).stroke(Stroke::new(1.0, BORDER))).show(ctx, |ui| self.header(ui, ctx));
        egui::TopBottomPanel::top("actions").frame(egui::Frame::new().fill(PAGE).inner_margin(Margin { left: 0, right: 0, top: 14, bottom: 0 })).show(ctx, |ui| self.action_bar(ui, ctx));
        egui::TopBottomPanel::top("tabs").frame(egui::Frame::new().fill(PAGE).inner_margin(Margin::ZERO)).show(ctx, |ui| self.tabs_row(ui));
        egui::TopBottomPanel::bottom("footer").frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(Margin::symmetric(0, 12)).stroke(Stroke::new(1.0, BORDER))).show(ctx, |ui| self.footer(ui, ctx));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(PAGE).inner_margin(Margin { left: 20, right: 20, top: 0, bottom: 14 })).show(ctx, |ui| {
            egui::Frame::new().fill(Color32::WHITE).stroke(Stroke::new(1.0, BORDER)).corner_radius(CornerRadius { nw: 0, ne: 8, sw: 8, se: 8 }).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                match self.tab {
                    Tab::Updates => {
                        self.banner_ui(ui, ctx);
                        if self.rows.is_empty() && self.results.is_empty() && self.info_rows.is_empty() {
                            let (title, sub, color) = if self.busy() { ("Scanning this PC", "This can take about 1–2 minutes.", ACCENT) }
                                else if self.scanned { ("Drivers are up to date", "No newer drivers were found in the sources checked.", GOOD) }
                                else { ("Scan this PC for driver updates", "Scan finds newer drivers, you review the list, then install.\nIt takes about 1–2 minutes and needs an internet connection.", NAVY) };
                            ui.vertical_centered(|ui| {
                                ui.add_space((ui.available_height() / 2.0 - 60.0).max(20.0));
                                ui.label(sbold(title).size(sz(22.0)).color(color));
                                ui.add_space(6.0);
                                ui.label(RichText::new(sub).size(sz(14.5)).color(MUTED));
                            });
                        } else { self.updates_table(ui); }
                    }
                    Tab::Installed => self.installed_table(ui),
                    Tab::Backups => self.backups_table(ui),
                    Tab::Log => { let mut text = self.log.join("\n"); egui::ScrollArea::vertical().show(ui, |ui| { ui.add(egui::TextEdit::multiline(&mut text).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(30).frame(false).margin(Margin::same(14))); }); }
                }
            });
        });

        if let Some(i) = self.confirm_rollback {
            let b = self.backups.get(i).map(|b| (b.entry.device.clone(), b.entry.old_version.clone()));
            let (mut close, mut go) = (false, false);
            if let Some((dev, ver)) = b {
                egui::Window::new("Roll back driver").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(format!("Roll back {dev} to version {ver}?"));
                    ui.label("The driver it uses now will be removed from this PC. A restart may be needed.");
                    ui.add_space(6.0);
                    ui.horizontal(|ui| { if button(ui, "Roll back", true, false, true, Vec2::new(120.0, 34.0)).clicked() { go = true; close = true; } if button(ui, "Cancel", false, false, true, Vec2::new(100.0, 34.0)).clicked() { close = true; } });
                });
            } else { close = true; }
            if go { self.do_rollback(i); }
            if close { self.confirm_rollback = None; }
        }

        // developer screenshot mode: render a few frames, save the window image, exit
        if let Some((path, frame)) = &mut self.screenshot {
            *frame += 1;
            if *frame == 6 { ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default())); }
            let shot = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
            if let Some(img) = shot {
                let (w, h) = (img.width() as u32, img.height() as u32);
                let raw: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_array()).collect();
                if let Some(buf) = image::RgbaImage::from_raw(w, h, raw) { let _ = buf.save(&*path); }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }
    }
}
