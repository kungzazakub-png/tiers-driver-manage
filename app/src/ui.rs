//! Window state. Presentation is the web page in assets/index.html (WebView2, so text is drawn by Windows with ClearType);
//! this module holds the state, runs the background work and turns the state into the JSON the page draws.
use crate::backup::{self, Backup};
use crate::catalog::{Date, HostInfo, InstalledDriver, Offer};
use crate::engine::{self, Code, Config, InstallDeps, InvRow, ScanDeps, Shared};
use crate::http::Real;
use crate::sys::RealSys;
use crate::update::{self, Manifest};
use crate::power::{self, backup as pbackup, cfg as pcfg, hw::{self as phw, Hardware, Tier}, overlay::RealOverlay, profiles::{self, Opts}, Comparison, PreviewRow, Report};
use crate::memory::{self, Monitor, RealMem};
use crate::boost::{self, RegExe};
use crate::settings::{self, Settings};
use crate::{selfupdate, winsys, wu};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO: &str = "kungzazakub-png/tiers-driver-manage";
pub const PUBLIC_KEY: &str = include_str!("../assets/UpdatePublicKey.xml");
pub const SHIPPED_CATALOG: &str = include_str!("../assets/driverdb.json");
/// Where "Report" sends hardware IDs (the Cloudflare Worker in backend/worker). Empty = the button is hidden.
pub const REPORT_URL: &str = "https://tiers-hwid-report.tiers-driver-manage.workers.dev/report";
pub const PAGE_HTML: &str = include_str!("../assets/index.html");
const MAX_PASSES: u32 = 3;

/// Lets background threads wake the window loop and lets the app close the window.
#[derive(Clone)]
pub struct Ctx { pub wake: Arc<dyn Fn() + Send + Sync>, pub close: Arc<dyn Fn() + Send + Sync> }
impl Ctx { fn repaint(&self) { (self.wake)() } }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab { Updates, Installed, Backups, Power, Boost, Memory, Settings, Log }

/// Boost tab state. `on` is read from the registry when the tab opens and after each change (not on every redraw).
struct BoostUi { on: Vec<bool>, gpus: Vec<String>, loaded: bool, err: String, store: boost::Store, path: std::path::PathBuf }
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode { Review, Run, Results }
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind { Info, Good, Warn, Bad }

pub struct Row { pub offer: Offer, pub ticked: bool, pub system: bool }
pub struct ResultRow { pub offer: Offer, pub code: Code, pub reason: String }
pub struct BannerBtn { pub id: &'static str, pub text: String, pub primary: bool, pub link: bool, pub enabled: bool }
pub struct Banner { pub kind: Kind, pub text: String, pub buttons: Vec<BannerBtn> }

enum Running { None, Scan(Shared), Install(Shared) }

enum PowerMsg { Loaded { hw: Hardware, tier: Tier, why: Vec<String>, preview: Vec<PreviewRow> }, Preview(Vec<PreviewRow>), Done { report: Report, status: &'static str, lines: Vec<String> }, Compared(Comparison, Vec<String>) }

/// State of the Power Plan tab.
pub struct PowerUi {
    hw: Option<Hardware>, recommended: Tier, why: Vec<String>, selected: Tier, opts: Opts, preview: Vec<PreviewRow>,
    report: Option<Report>, cmp: Option<Comparison>, status: String, busy: bool, confirm_reset: bool, loaded: bool, rx: Option<Receiver<PowerMsg>>,
}

pub struct App {
    http: Arc<Real>,
    cfg: Arc<Config>,
    host: HostInfo,
    machine: String,
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
    power: PowerUi,
    mem: Monitor,
    settings: Settings,
    settings_path: std::path::PathBuf,
    /// None = not read yet
    autostart: Option<bool>,
    settings_err: String,
    boost: BoostUi,
    report_rx: Option<Receiver<Result<usize, String>>>,
    reported: bool,
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

pub fn iso_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{}T{:02}:{:02}:{:02}", Date::today().iso(), (secs / 3600) % 24, (secs / 60) % 60, secs % 60)
}

fn code_name(c: Code) -> &'static str {
    match c { Code::Ok => "ok", Code::Fail => "fail", Code::Skip => "skip", Code::Wait => "wait", Code::Dl => "dl", Code::Inst => "inst" }
}

impl App {
    pub fn new(cfg: Config) -> App {
        let settings_path = cfg.catalog_path.parent().map(|p| p.join("settings.json")).unwrap_or_else(|| "settings.json".into());
        let settings = Settings::load(&settings_path);
        let mut mem = Monitor::new();
        mem.on = settings.mem_auto; mem.poll_ms = settings.mem_poll_ms; mem.rule = memory::Rule { standby_mb: settings.mem_standby_mb, free_mb: settings.mem_free_mb };
        let boost_path = settings_path.with_file_name("boost.json");
        App {
            boost: BoostUi { on: vec![], gpus: vec![], loaded: false, err: String::new(), store: boost::load(&boost_path), path: boost_path },
            mem, settings, settings_path, autostart: None, settings_err: String::new(),
            http: Arc::new(Real::new()), cfg: Arc::new(cfg), host: winsys::host_info(), machine: winsys::machine_summary(),
            tab: Tab::Updates, mode: Mode::Review, rows: vec![], info_rows: vec![], results: vec![], inventory: vec![], missing_count: 0,
            offers: HashMap::new(), declined: HashSet::new(), running: Running::None, scanned: false, status: "Ready to scan".into(), banner: None,
            restore_point: true, restore_done: false, log: vec![], need_reboot: false, pass: 0, auto: false, backups: vec![], selected_backup: None,
            confirm_rollback: None, update: None, update_rx: None, apply_rx: None, updating: false,
            report_rx: None, reported: false,
            power: PowerUi { hw: None, recommended: Tier::Low, why: vec![], selected: Tier::Low, opts: Opts { restore_point: true, ..Default::default() }, preview: vec![], report: None, cmp: None, status: "Ready".into(), busy: false, confirm_reset: false, loaded: false, rx: None },
        }
    }

    fn boost_refresh(&mut self) {
        self.boost.on = boost::TWEAKS.iter().map(|t| boost::is_on(&RegExe, t)).collect();
        if !self.boost.loaded { self.boost.gpus = boost::gpu_names(); self.boost.loaded = true; }
    }

    fn boost_apply(&mut self, id: &str, on: bool) {
        let Some(t) = boost::tweak(id) else { return };
        match boost::set(&RegExe, &mut self.boost.store, t, on) {
            Ok(()) => {
                if id == "mouse" { boost::refresh_mouse(&RegExe); }
                self.boost.err = boost::save(&self.boost.path, &self.boost.store).err().map(|e| format!("Could not save the undo information: {e}")).unwrap_or_default();
                self.say(format!("Boost: {} {}", t.title, if on { "on" } else { "off" }));
            }
            Err(e) => { self.say(format!("[FAILED] Boost: {e}")); self.boost.err = e; }
        }
    }

    fn boost_view(&self) -> Value {
        let b = &self.boost;
        let (tip_name, tip) = boost::latency_tip(&b.gpus);
        let items: Vec<Value> = boost::TWEAKS.iter().enumerate().map(|(i, t)| json!({ "id": t.id, "title": t.title, "why": t.why, "effect": t.effect, "restart": t.restart, "rec": t.recommended,
            "on": b.on.get(i).copied().unwrap_or(false) })).collect();
        let rec_done = boost::TWEAKS.iter().enumerate().filter(|(_, t)| t.recommended).all(|(i, _)| b.on.get(i).copied().unwrap_or(false));
        json!({ "items": items, "err": b.err, "gpus": b.gpus.join(", "), "tipName": tip_name, "tip": tip, "recDone": rec_done, "canUndo": !b.store.is_empty() })
    }

    /// Closing the window hides it to the tray instead of quitting.
    pub fn tray_on_close(&self) -> bool { self.settings.tray_on_close }
    pub fn mem_auto(&self) -> bool { self.mem.on }

    fn save_settings(&mut self) {
        if let Err(e) = self.settings.save(&self.settings_path) { self.say(format!("[FAILED] Could not save the settings: {e}")); }
    }
    /// Memory choices are remembered between runs.
    fn save_mem_settings(&mut self) {
        let (m, s) = (&self.mem, &mut self.settings);
        (s.mem_auto, s.mem_poll_ms, s.mem_standby_mb, s.mem_free_mb) = (m.on, m.poll_ms, m.rule.standby_mb, m.rule.free_mb);
        self.save_settings();
    }

    fn set_autostart(&mut self, on: bool) {
        let exe = std::env::current_exe().unwrap_or_default();
        let user = format!("{}\\{}", std::env::var("USERDOMAIN").unwrap_or_default(), std::env::var("USERNAME").unwrap_or_default());
        let work = self.settings_path.parent().map(|p| p.to_path_buf()).unwrap_or_else(std::env::temp_dir);
        match settings::set_autostart(&RealSys, on, &exe, &user, &work) {
            Ok(()) => {
                self.settings_err.clear();
                self.autostart = Some(on);
                // a program that starts in the tray must stay in the tray when its window is closed
                if on { self.settings.tray_on_close = true; self.save_settings(); }
                self.say(if on { "Settings: the program will start with Windows, in the tray" } else { "Settings: start with Windows turned off" });
            }
            Err(e) => { self.settings_err = e.clone(); self.say(format!("[FAILED] Settings: {e}")); self.autostart = Some(settings::autostart_enabled(&RealSys)); }
        }
    }

    fn busy(&self) -> bool { !matches!(self.running, Running::None) || self.updating || self.power.busy }
    pub fn is_busy(&self) -> bool { self.busy() }

    fn say(&mut self, line: impl Into<String>) { self.log.push(line.into()); }

    // ---------- background work ----------
    pub fn check_for_update(&mut self, ctx: &Ctx) {
        if self.cfg.repo.is_empty() { return; }
        let (tx, rx) = channel();
        let (http, repo, ctx2) = (self.http.clone(), self.cfg.repo.clone(), ctx.clone());
        std::thread::spawn(move || { let _ = tx.send(update::get_app_update(&repo, VERSION, PUBLIC_KEY, &*http)); ctx2.repaint(); });
        self.update_rx = Some(rx);
    }

    fn start_update(&mut self, ctx: &Ctx) {
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
            ctx2.repaint();
        });
        self.apply_rx = Some(rx);
    }

    fn start_scan(&mut self, ctx: &Ctx) {
        if self.busy() { return; }
        self.rows.clear(); self.info_rows.clear(); self.results.clear(); self.offers.clear(); self.declined.clear();
        self.log.clear(); self.need_reboot = false; self.pass = 0; self.auto = false; self.restore_done = false; self.banner = None; self.mode = Mode::Review;
        self.begin_scan(ctx);
    }

    fn begin_scan(&mut self, ctx: &Ctx) {
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
            ctx2.repaint();
        });
    }

    fn start_install(&mut self, ctx: &Ctx) {
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
        self.banner = Some(Banner { kind: Kind::Info, text: "กำลังเตรียมติดตั้ง...".into(), buttons: vec![BannerBtn { id: "stop", text: "Stop after this item".into(), primary: false, link: false, enabled: true }] });
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
            ctx2.repaint();
        });
    }

    // ---------- polling ----------
    pub fn poll(&mut self, ctx: &Ctx) {
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
                        (ctx.close)();
                    }
                    Err(e) => { self.say(format!("[FAILED] Program update: {e}")); self.status = "Stopped: the program update failed (your existing files were not changed). See the log for details.".into(); }
                }
            }
        }
        if let Some(rx) = &self.report_rx {
            if let Ok(r) = rx.try_recv() {
                self.report_rx = None;
                match r { Ok(n) => { self.reported = true; self.status = format!("Done: reported {n} device(s). Thank you, a driver may be added later."); self.say(format!("Report sent: {n} device(s)")); } Err(e) => { self.status = "Stopped: the report could not be sent. See the log.".into(); self.say(format!("[FAILED] Report: {e}")); } }
            }
        }
        let mut reload_power = false;
        if let Some(rx) = &self.power.rx {
            if let Ok(m) = rx.try_recv() {
                self.power.rx = None; self.power.busy = false;
                match m {
                    PowerMsg::Loaded { hw, tier, why, preview } => { let first = !self.power.loaded; self.power.loaded = true; self.power.hw = Some(hw); self.power.recommended = tier; self.power.why = why; if first { self.power.selected = tier; } self.power.preview = preview; }
                    PowerMsg::Preview(p) => self.power.preview = p,
                    PowerMsg::Done { report, status, lines } => { self.power.status = status.into(); self.power.report = Some(report); self.log.extend(lines); reload_power = true; }
                    PowerMsg::Compared(c, lines) => { self.power.cmp = Some(c); self.log.extend(lines); }
                }
            }
        }
        if reload_power { self.power_load(ctx); }
        let lines = self.mem.tick(&RealMem, std::time::Instant::now(), self.tab == Tab::Memory);
        self.log.extend(lines);
        let finished = match &self.running {
            Running::Scan(j) | Running::Install(j) => { let g = j.lock().unwrap(); if !g.status.is_empty() && !g.done { self.status = g.status.clone(); } g.done }
            Running::None => false,
        };
        if matches!(self.running, Running::Install(_)) && !finished { self.refresh_run_banner(); }
        if finished {
            match std::mem::replace(&mut self.running, Running::None) {
                Running::Scan(j) => self.finish_scan(j),
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
        let text = if stopping { format!("จะหยุดหลังรายการนี้ · เสร็จแล้ว {done} จาก {total}") } else { format!("กำลังติดตั้ง {done} จาก {total} · ห้ามปิดโปรแกรมหรือปิดเครื่อง") };
        self.banner = Some(Banner { kind: Kind::Info, text, buttons: vec![BannerBtn { id: "stop", text: if stopping { "Stopping...".into() } else { "Stop after this item".into() }, primary: false, link: false, enabled: !stopping }] });
    }

    fn finish_scan(&mut self, j: Shared) {
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
            self.banner = if ok > 0 { Some(Banner { kind: Kind::Info, text: format!("ติดตั้งแล้ว {ok} · พบเพิ่มอีก {} รายการจากฮาร์ดแวร์ที่เพิ่งตรวจพบ ดูรายการแล้วกด Install อีกครั้ง", self.rows.len()), buttons: vec![] }) }
                else if self.rows.iter().any(|r| r.system) { Some(Banner { kind: Kind::Warn, text: "ไดรเวอร์ Chipset และไดรเวอร์ระบบไม่ได้ติ๊กไว้ให้ เพราะกระทบทั้งเครื่องและส่วนใหญ่ไม่จำเป็นต้องอัปเดต ถ้าต้องการให้ติ๊กเองได้".into(), buttons: vec![] }) } else { None };
            self.tab = Tab::Updates;
        }
    }

    fn finish_install(&mut self, j: Shared, ctx: &Ctx) {
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
            self.banner = Some(Banner { kind: Kind::Info, text: "ติดตั้งรอบนี้เสร็จแล้ว · กำลังสแกนอีกครั้ง เผื่อมีฮาร์ดแวร์ใหม่ที่ต้องการไดรเวอร์".into(), buttons: vec![] });
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
        if self.need_reboot { text.push_str(" · ต้องรีสตาร์ทเครื่องเพื่อให้ไดรเวอร์ทำงานเต็มที่"); } else if fail == 0 && skip == 0 { text.push_str(" · เสร็จแล้ว ไม่ต้องรีสตาร์ท"); }
        let kind = if fail > 0 && ok == 0 { Kind::Bad } else if fail > 0 || skip > 0 || self.need_reboot { Kind::Warn } else { Kind::Good };
        let b = |id: &'static str, t: &str, primary: bool, link: bool| BannerBtn { id, text: t.into(), primary, link, enabled: true };
        let mut buttons = vec![];
        if self.need_reboot { buttons.push(b("restart", "Restart now", true, false)); buttons.push(b("later", "Later", false, true)); }
        if fail > 0 { buttons.push(b("retry", "Retry failed ones", !self.need_reboot, false)); }
        if !self.need_reboot && !self.results.is_empty() { buttons.push(b("restart", "Restart computer", false, true)); }
        if !self.need_reboot && fail == 0 { buttons.push(b("rescan", "Scan again", false, true)); }
        self.banner = Some(if self.results.is_empty() { Banner { kind: Kind::Good, text: "ไม่มีอะไรต้องติดตั้ง".into(), buttons: vec![] } } else { Banner { kind, text, buttons } });
        self.status = "Done".into();
    }

    fn banner_click(&mut self, id: &str, ctx: &Ctx) {
        match id {
            "stop" => { if let Running::Install(j) = &self.running { j.lock().unwrap().cancel = true; } }
            "restart" => {
                let _ = std::process::Command::new("shutdown.exe").args(["/r", "/t", "30", "/c", "TierS Driver Manage: restart to finish installing drivers"]).spawn();
                self.banner = Some(Banner { kind: Kind::Warn, text: "เครื่องจะรีสตาร์ทใน 30 วินาที · บันทึกงานที่เปิดอยู่ให้เรียบร้อย".into(), buttons: vec![BannerBtn { id: "abort", text: "Cancel restart".into(), primary: true, link: false, enabled: true }] });
            }
            "abort" => {
                let _ = std::process::Command::new("shutdown.exe").arg("/a").spawn();
                self.banner = Some(Banner { kind: Kind::Warn, text: "ยกเลิกการรีสตาร์ทแล้ว · ไดรเวอร์จะทำงานเต็มที่หลังรีสตาร์ท".into(), buttons: vec![BannerBtn { id: "restart", text: "Restart now".into(), primary: true, link: false, enabled: true }] });
            }
            "later" => self.banner = Some(Banner { kind: Kind::Warn, text: "ยังไม่ได้รีสตาร์ท · ไดรเวอร์ที่ติดตั้งจะทำงานเต็มที่หลังรีสตาร์ท".into(), buttons: vec![BannerBtn { id: "restart", text: "Restart now".into(), primary: true, link: false, enabled: true }] }),
            "retry" => self.retry_failed(ctx),
            "rescan" => self.start_scan(ctx),
            _ => {}
        }
    }

    /// Puts the failed drivers back in the list, ticked, and installs them again.
    fn retry_failed(&mut self, ctx: &Ctx) {
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

    /// Opt-in: runs only when the user clicks the button; sends hardware IDs of the devices that have no driver anywhere.
    fn send_report(&mut self, ctx: &Ctx) {
        if REPORT_URL.is_empty() || self.reported || self.report_rx.is_some() { return; }
        let items = crate::report::build(&self.info_rows);
        let (tx, rx) = channel();
        let (http, ctx2) = (self.http.clone(), ctx.clone());
        self.status = "Sending the report...".into();
        std::thread::spawn(move || { let _ = tx.send(crate::report::send(&*http, REPORT_URL, &items)); ctx2.repaint(); });
        self.report_rx = Some(rx);
    }

    // ---------- power plan ----------
    fn power_job(&mut self, ctx: &Ctx, f: impl FnOnce(&power::Env) -> PowerMsg + Send + 'static) {
        if self.busy() { return; }
        self.power.busy = true;
        let (tx, rx) = channel();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            wu::init_com();
            let (sys, ov) = (RealSys, RealOverlay);
            let root = pbackup::root();
            let log = power::log::Log::new(Some(root.join("power.log")));
            let rp = || crate::restorepoint::create("TierS Power Plan");
            let env = power::Env { cfg: &sys, overlay: &ov, root: root.clone(), log: &log, restore_point: &rp };
            let _ = tx.send(f(&env));
            ctx2.repaint();
        });
        self.power.rx = Some(rx);
    }

    fn power_load(&mut self, ctx: &Ctx) {
        let (first, sel, opts) = (!self.power.loaded, self.power.selected, self.power.opts);
        self.power_job(ctx, move |env| {
            let name = pcfg::active(env.cfg).map(|a| a.name).unwrap_or_else(|_| "unknown".into());
            let hw = phw::detect(name);
            let (tier, why) = phw::score(&hw);
            let preview = power::preview(env, if first { tier } else { sel }, opts);
            PowerMsg::Loaded { hw, tier, why, preview }
        });
    }

    fn power_refresh_preview(&mut self, ctx: &Ctx) {
        let (sel, opts) = (self.power.selected, self.power.opts);
        self.power_job(ctx, move |env| PowerMsg::Preview(power::preview(env, sel, opts)));
    }

    fn power_apply(&mut self, ctx: &Ctx) {
        let Some(hw) = self.power.hw.clone() else { return };
        if self.busy() { return; }
        let (t, o) = (self.power.selected, self.power.opts);
        self.power.status = "Applying".into();
        self.power_job(ctx, move |env| { let report = power::apply(env, t, o, &hw); let status = if report.ok { "Success" } else { "Failed" }; PowerMsg::Done { report, status, lines: env.log.lines() } });
    }

    fn power_run(&mut self, ctx: &Ctx, what: &str, f: impl FnOnce(&power::Env) -> (Report, &'static str) + Send + 'static) {
        if self.busy() { return; }
        self.power.status = "Applying".into();
        self.log.push(format!("Power plan: {what}"));
        self.power_job(ctx, move |env| { let (report, status) = f(env); PowerMsg::Done { report, status, lines: env.log.lines() } });
    }

    // ---------- commands from the page ----------
    pub fn handle(&mut self, msg: &str, ctx: &Ctx) {
        let v: Value = serde_json::from_str(msg).unwrap_or(Value::Null);
        let cmd = v["cmd"].as_str().unwrap_or("");
        let i = v["i"].as_u64().map(|x| x as usize);
        match cmd {
            "scan" => self.start_scan(ctx),
            "install" => self.start_install(ctx),
            "update" => self.start_update(ctx),
            "report" => self.send_report(ctx),
            "tab" => {
                self.tab = match v["tab"].as_str() { Some("installed") => Tab::Installed, Some("backups") => Tab::Backups, Some("power") => Tab::Power, Some("boost") => Tab::Boost, Some("memory") => Tab::Memory, Some("settings") => Tab::Settings, Some("log") => Tab::Log, _ => Tab::Updates };
                if self.tab == Tab::Boost { self.boost_refresh(); }
                if self.tab == Tab::Settings { self.autostart = Some(settings::autostart_enabled(&RealSys)); }
                if self.tab == Tab::Backups { self.refresh_backups(); }
                if self.tab == Tab::Power && !self.power.loaded { self.power_load(ctx); }
            }
            "tick" => if let (Some(i), Some(on)) = (i, v["on"].as_bool()) { if self.mode == Mode::Review && !self.busy() { if let Some(r) = self.rows.get_mut(i) { r.ticked = on; } } },
            "tickAll" => if let Some(on) = v["on"].as_bool() { if self.mode == Mode::Review && !self.busy() { for r in self.rows.iter_mut() { r.ticked = on; } } },
            "banner" => if let Some(id) = v["id"].as_str() { self.banner_click(id, ctx) },
            "restorePoint" => if let Some(on) = v["on"].as_bool() { if !self.busy() { self.restore_point = on; } },
            "pick" => if let Some(i) = i { if i < self.backups.len() { self.selected_backup = Some(i); } },
            "rollback" => { if self.selected_backup.is_some() && !self.busy() { self.confirm_rollback = self.selected_backup; } }
            "rollbackYes" => { if let Some(i) = self.confirm_rollback.take() { self.do_rollback(i); } }
            "rollbackNo" => self.confirm_rollback = None,
            "mem_purge" => match self.mem.purge_now(&RealMem) { Ok(line) => self.say(line), Err(e) => { self.mem.err = e.clone(); self.say(format!("[FAILED] Memory: {e}")); } },
            "mem_auto" => if let Some(on) = v["v"].as_u64().map(|x| x == 1) { self.mem.on = on; if on { self.mem.err.clear(); self.say("Memory: automatic cleaning started"); } else { self.say("Memory: automatic cleaning stopped"); } self.save_mem_settings(); },
            "mem_poll" => if let Some(x) = v["v"].as_u64() { self.mem.poll_ms = memory::snap(x, memory::POLL_MS); self.save_mem_settings(); },
            "mem_standby" => if let Some(x) = v["v"].as_u64() { self.mem.rule.standby_mb = memory::snap(x, memory::STANDBY_MB); self.save_mem_settings(); },
            "mem_free" => if let Some(x) = v["v"].as_u64() { self.mem.rule.free_mb = memory::snap(x, memory::FREE_MB); self.save_mem_settings(); },
            "boost_set" => if let (Some(id), Some(on)) = (v["id"].as_str(), v["v"].as_u64().map(|x| x == 1)) { self.boost_apply(id, on); self.boost_refresh(); },
            "boost_rec" => { for t in boost::TWEAKS.iter().filter(|t| t.recommended) { self.boost_apply(t.id, true); } self.boost_refresh(); }
            "boost_undo" => { let ids: Vec<String> = self.boost.store.keys().cloned().collect(); for id in ids { self.boost_apply(&id, false); } self.boost_refresh(); }
            "set_tray" => if let Some(on) = v["v"].as_u64().map(|x| x == 1) { self.settings.tray_on_close = on; self.save_settings(); },
            "set_autostart" => if let Some(on) = v["v"].as_u64().map(|x| x == 1) { self.set_autostart(on); },
            "power_tier" => if !self.power.busy { if let Some(t) = v["tier"].as_str().and_then(Tier::from_key) { self.power.selected = t; self.power_refresh_preview(ctx); } },
            "power_opt" => if !self.power.busy { if let Some(on) = v["on"].as_bool() {
                match v["name"].as_str() { Some("usb") => self.power.opts.usb_off = on, Some("pcie") => self.power.opts.pcie_off = on, Some("ultimate") => self.power.opts.ultimate = on, Some("restore") => self.power.opts.restore_point = on, _ => {} }
                if v["name"].as_str() != Some("restore") { self.power_refresh_preview(ctx); }
            } },
            "power_apply" => self.power_apply(ctx),
            "power_backup" => self.power_run(ctx, "Backup settings", |env| { let r = power::backup_only(env); let s = if r.ok { "Success" } else { "Failed" }; (r, s) }),
            "power_restore" => self.power_run(ctx, "Restore original settings", |env| { let r = power::restore(env, power::Which::Original); let s = if r.ok { "Restore Completed" } else { "Failed" }; (r, s) }),
            "power_undo" => self.power_run(ctx, "Undo last optimization", |env| { let r = power::restore(env, power::Which::Last); let s = if r.ok { "Restore Completed" } else { "Failed" }; (r, s) }),
            "power_reset" => if !self.busy() { self.power.confirm_reset = true; },
            "power_reset_no" => self.power.confirm_reset = false,
            "power_reset_yes" => { self.power.confirm_reset = false; self.power_run(ctx, "Reset to Windows default", |env| { let r = power::reset_defaults(env); let s = if r.ok { "Success" } else { "Failed" }; (r, s) }) }
            "power_compare" => if let Some(hw) = self.power.hw.clone() { self.power_job(ctx, move |env| { let c = power::compare_ultimate(env, &hw); PowerMsg::Compared(c, env.log.lines()) }) },
            _ => {}
        }
    }

    // ---------- view model ----------
    fn selected(&self) -> (usize, f64) { self.rows.iter().filter(|r| r.ticked).fold((0, 0.0), |(n, s), r| (n + 1, s + r.offer.size_mb.unwrap_or(0.0))) }

    fn row_json(&self, i: usize, o: &Offer, code: Option<Code>, reason: &str, ticked: bool, system: bool) -> Value {
        let (note, tone) = match code {
            Some(Code::Wait) => ("Queued".to_string(), "muted"), Some(Code::Dl) => (format!("Downloading ({})", size_text(o.size_mb)), "accent"), Some(Code::Inst) => ("Installing".into(), "accent"),
            Some(Code::Ok) => ("Installed".into(), "good"),
            Some(Code::Fail) => (if reason.is_empty() { "Failed".into() } else { format!("Failed: {}", plain_reason(reason)) }, "bad"),
            Some(Code::Skip) => (if reason.is_empty() { "Skipped".into() } else { format!("Skipped: {}", plain_reason(reason)) }, "warn"),
            None => if system { ("Review first: affects the whole PC".into(), "warn") } else { (String::new(), "muted") },
        };
        let tip = if reason.is_empty() { note.clone() } else { format!("{note}\n({reason})") };
        json!({ "i": i, "title": o.title, "cls": class_label(&o.class), "source": source_label(&o.source), "date": date_text(&o.date), "size": size_text(o.size_mb),
                "ticked": ticked, "system": system, "state": code.map(code_name).unwrap_or("none"), "note": note, "tone": tone, "tip": tip })
    }

    pub fn view(&self) -> Value {
        let busy = self.busy();
        let items: HashMap<String, engine::ItemState> = if let Running::Install(j) = &self.running { j.lock().unwrap().items.clone() } else { HashMap::new() };
        let mut rows = vec![];
        let mut progress = Value::Null;
        if self.mode == Mode::Results {
            for (i, r) in self.results.iter().enumerate() { rows.push(self.row_json(i, &r.offer, Some(r.code), &r.reason, false, false)); }
        } else {
            let mut done = 0usize;
            for (i, r) in self.rows.iter().enumerate() {
                let st = items.get(&r.offer.id);
                let code = if self.mode == Mode::Run { Some(st.map(|s| s.code).unwrap_or(Code::Wait)) } else { None };
                if matches!(code, Some(Code::Ok | Code::Fail | Code::Skip)) { done += 1; }
                rows.push(self.row_json(i, &r.offer, code, st.map(|s| s.reason.as_str()).unwrap_or(""), r.ticked, r.system));
            }
            if self.mode == Mode::Run { progress = json!(done as f64 / self.rows.len().max(1) as f64); }
        }
        let infos: Vec<Value> = if self.mode == Mode::Review { self.info_rows.iter().map(|d| json!({ "name": d.name, "cls": class_label(&d.class), "hwid": d.hardware_ids.first().cloned().unwrap_or_default() })).collect() } else { vec![] };
        let years = self.cfg.old_years;
        let inventory: Vec<Value> = self.inventory.iter().map(|d| {
            let old = d.age.map(|a| a > years).unwrap_or(false) && !d.maker.starts_with("Microsoft");
            json!({ "name": d.name, "cls": class_label(&d.class), "maker": d.maker, "version": d.version,
                    "date": d.date.map(|x| if x.y < 1990 { "—".to_string() } else { x.iso() }).unwrap_or_default(), "old": old, "known": d.age.is_some(), "age": format!("{:.1}", d.age.unwrap_or(0.0)) })
        }).collect();
        let backups: Vec<Value> = self.backups.iter().map(|b| json!({ "time": b.entry.time.replace('T', " ").chars().take(16).collect::<String>(), "device": b.entry.device, "version": b.entry.old_version, "kind": if b.entry.folder.is_empty() { "Built-in driver" } else { "Saved copy" } })).collect();
        let (sel_n, sel_mb) = self.selected();
        // the summary pills
        let ok = self.results.iter().filter(|r| r.code == Code::Ok).count();
        let (up_icon, up_tone, up_text) = match self.mode {
            Mode::Review if !self.rows.is_empty() => ("up", "accent", format!("{} to install", self.rows.len())),
            Mode::Review => ("check", "good", "Up to date".to_string()),
            Mode::Run => ("dot", "accent", format!("Installing {}", self.rows.len())),
            Mode::Results => { let bad = self.results.len() - ok; if bad > 0 { ("info", "warn", format!("{ok} installed · {bad} not done")) } else { ("check", "good", format!("{ok} installed")) } }
        };
        let (st_tone, st_icon) = if self.status.starts_with("Stopped") { ("bad", "cross") } else if self.status.starts_with("Done") || self.status.starts_with("Scan complete") { ("good", "check") } else if busy { ("accent", "dot") } else { ("muted", "") };
        json!({
            "version": VERSION, "machine": self.machine, "busy": busy, "status": self.status, "statusTone": st_tone, "statusIcon": st_icon, "progress": progress,
            "update": self.update.as_ref().map(|m| json!({ "version": m.version, "notes": m.notes })),
            "tab": match self.tab { Tab::Updates => "updates", Tab::Installed => "installed", Tab::Backups => "backups", Tab::Power => "power", Tab::Boost => "boost", Tab::Memory => "memory", Tab::Settings => "settings", Tab::Log => "log" },
            "mode": match self.mode { Mode::Review => "review", Mode::Run => "run", Mode::Results => "results" },
            "scanned": self.scanned,
            "banner": self.banner.as_ref().map(|b| json!({ "kind": match b.kind { Kind::Info => "info", Kind::Good => "good", Kind::Warn => "warn", Kind::Bad => "bad" }, "text": b.text,
                "buttons": b.buttons.iter().map(|x| json!({ "id": x.id, "text": x.text, "primary": x.primary, "link": x.link, "enabled": x.enabled })).collect::<Vec<_>>() })),
            "rows": rows, "infos": infos, "inventory": inventory, "backups": backups, "selectedBackup": self.selected_backup,
            "pills": { "drivers": self.inventory.len(), "upIcon": up_icon, "upTone": up_tone, "upText": up_text, "missing": self.missing_count },
            "selN": sel_n, "selSize": size_text(Some(sel_mb)), "allTicked": !self.rows.is_empty() && self.rows.iter().all(|r| r.ticked),
            "report": { "available": !REPORT_URL.is_empty() && self.mode == Mode::Review, "count": crate::report::build(&self.info_rows).len(), "done": self.reported, "sending": self.report_rx.is_some() },
            "restorePoint": self.restore_point, "log": self.log.join("\n"), "power": self.power_view(), "boost": self.boost_view(), "memory": self.memory_view(),
            "settings": json!({ "tray": self.settings.tray_on_close, "autostart": self.autostart, "err": self.settings_err }),
            "confirm": self.confirm_rollback.and_then(|i| self.backups.get(i)).map(|b| json!({ "device": b.entry.device, "version": b.entry.old_version })),
        })
    }

    fn memory_view(&self) -> Value {
        let m = &self.mem;
        let sz = |b: u64| size_text(Some((b / memory::MB) as f64));
        let st = m.stats.map(|s| {
            let t = s.total.max(1) as f64;
            let in_use = s.total.saturating_sub(s.available);
            json!({ "total": sz(s.total), "inUse": sz(in_use), "available": sz(s.available), "standby": sz(s.standby), "modified": sz(s.modified), "free": sz(s.free),
                "pctUse": (in_use as f64 / t * 100.0).round(), "pctStandby": (s.standby as f64 / t * 100.0).round(), "pctFree": (s.free as f64 / t * 100.0).round() })
        });
        let mb = |b: u64| b / memory::MB;
        json!({ "stats": st, "err": m.err, "note": m.note, "on": m.on, "purges": m.purges, "poll": m.poll_ms, "standbyMb": m.rule.standby_mb, "freeMb": m.rule.free_mb,
            "standbyNowMb": m.stats.map(|s| mb(s.standby)), "freeNowMb": m.stats.map(|s| mb(s.free)), "wouldClean": m.stats.map(|s| m.rule.hit(&s)),
            "pollR": memory::POLL_MS, "standbyR": memory::STANDBY_MB, "freeR": memory::FREE_MB })
    }

    fn power_view(&self) -> Value {
        let p = &self.power;
        let hw = p.hw.as_ref().map(|h| json!({
            "vendor": match h.vendor { phw::Vendor::Intel => "Intel", phw::Vendor::Amd => "AMD", phw::Vendor::Unknown => "Unknown" }, "cpu": if h.cpu_name.is_empty() { "Unknown".to_string() } else { h.cpu_name.clone() },
            "cores": if h.threads > 0 { format!("{} cores / {} threads", h.cores, h.threads) } else { "Unknown".into() }, "ram": if h.ram_gb > 0.0 { format!("{:.1} GB", h.ram_gb) } else { "Unknown".into() },
            "windows": h.windows, "type": if h.laptop { "Laptop" } else { "Desktop" }, "source": match h.on_ac { Some(true) => "Plugged in (AC)", Some(false) => "On battery", None => "Unknown" }, "plan": h.active_plan }));
        let battery_block = p.selected == Tier::Extreme && p.hw.as_ref().map(|h| h.on_ac == Some(false)).unwrap_or(false);
        json!({
            "loaded": p.loaded, "hw": hw, "recommended": p.recommended.key(), "recommendedName": p.recommended.name(), "why": p.why, "selected": p.selected.key(), "opts": p.opts,
            "preview": p.preview, "report": p.report, "cmp": p.cmp, "status": p.status, "busy": p.busy, "confirmReset": p.confirm_reset, "batteryBlock": battery_block,
            "profiles": profiles::PROFILES.iter().map(|x| json!({ "key": x.tier.key(), "title": x.title, "suited": x.suited, "goal": x.goal, "powerMode": x.power_mode_best, "usb": x.usb_optional, "pcie": x.pcie_optional, "ultimate": x.ultimate_optional })).collect::<Vec<_>>(),
        })
    }

    /// Fills the window with demo data for screenshots (developer only).
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
        if state == "installed" { self.tab = Tab::Installed; }
        if state == "log" { self.log = vec!["Installed drivers: 203; older than 3 years (Microsoft drivers not counted): 12".into(), "Scan complete: 6 newer drivers found".into()]; self.tab = Tab::Log; }
        if state == "results" {
            self.results = offers.iter().skip(2).take(4).enumerate().map(|(i, o)| ResultRow { offer: o.clone(), code: [Code::Ok, Code::Ok, Code::Fail, Code::Skip][i], reason: if i == 2 { "SHA-256 mismatch (got X)".into() } else if i == 3 { "Stopped by request".into() } else { String::new() } }).collect();
            self.rows.clear(); self.need_reboot = true; self.show_results();
        }
    }
}
