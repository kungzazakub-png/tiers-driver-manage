//! The two jobs the window starts: a scan (find newer drivers) and an install (back up, download, verify, install each chosen driver).
//! They run on a worker thread and publish progress into a shared `Job` that the window polls.
use crate::backup;
use crate::catalog::{find_catalog_updates, CatalogEntry, Date, HostInfo, InstalledDriver, Offer, OfferKind};
use crate::http::Http;
use crate::install::{self, InstallResult};
use crate::sys::Sys;
use crate::{mscatalog, nvidia, update, wu};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Code { Wait, Dl, Inst, Ok, Fail, Skip }

#[derive(Clone, Debug)]
pub struct ItemState { pub code: Code, pub reason: String }

#[derive(Clone, Debug)]
pub struct InvRow { pub name: String, pub class: String, pub maker: String, pub version: String, pub date: Option<Date>, pub age: Option<f64> }

#[derive(Default)]
pub struct Job {
    pub status: String,
    pub log: Vec<String>,
    pub done: bool,
    pub error: Option<String>,
    pub offers: Vec<Offer>,
    pub inventory: Vec<InvRow>,
    pub missing: Vec<String>,
    pub unresolved: Vec<InstalledDriver>,
    pub items: HashMap<String, ItemState>,
    pub reboot: bool,
    pub installed: usize,
    pub cancel: bool,
}
pub type Shared = Arc<Mutex<Job>>;
pub fn new_job() -> Shared { Arc::new(Mutex::new(Job::default())) }

fn status(j: &Shared, s: &str) { j.lock().unwrap().status = s.to_string(); }
fn log(j: &Shared, s: impl Into<String>) { j.lock().unwrap().log.push(s.into()); }
fn set_item(j: &Shared, id: &str, code: Code, reason: &str) { j.lock().unwrap().items.insert(id.to_string(), ItemState { code, reason: reason.to_string() }); }
fn cancelled(j: &Shared) -> bool { j.lock().unwrap().cancel }

pub struct Config {
    pub catalog_path: PathBuf,
    pub online_catalog_path: PathBuf,
    pub repo: String,
    pub public_key: String,
    /// The program folder is writable by normal users and the safe setting is on: catalog installs are refused.
    pub block_catalog: bool,
    pub old_years: f64,
}

pub struct ScanDeps<'a> {
    pub http: &'a dyn Http,
    pub host: HostInfo,
    pub installed: &'a dyn Fn() -> Result<Vec<InstalledDriver>, String>,
    pub without_driver: &'a dyn Fn() -> Result<(Vec<InstalledDriver>, Vec<String>), String>,
    pub live_version: &'a dyn Fn(&str) -> Option<String>,
    pub windows_update: &'a dyn Fn() -> Result<Vec<Offer>, String>,
    pub today: Date,
}

pub const NO_INTERNET: &str = "No internet connection found. This program needs the internet to scan and install drivers. Please connect and try again.";

pub fn scan(j: &Shared, cfg: &Config, d: &ScanDeps) {
    let run = || -> Result<(), String> {
        status(j, "Checking internet connection...");
        if d.http.get("http://www.msftconnecttest.com/connecttest.txt").is_err() { return Err(NO_INTERNET.into()); }

        status(j, "Scanning hardware and installed drivers...");
        let installed = (d.installed)()?;
        let inv: Vec<InvRow> = installed.iter().map(|x| InvRow { name: x.name.clone(), class: x.class.clone(), maker: x.maker.clone(), version: x.version.clone(), date: x.date, age: x.date.map(|dt| (dt.years_until(d.today) * 10.0).round() / 10.0) }).collect();
        j.lock().unwrap().inventory = inv;
        let (missing_devs, missing_lines) = (d.without_driver)().unwrap_or_default();
        j.lock().unwrap().missing = missing_lines;

        let mut items: Vec<Offer> = Vec::new();
        status(j, "Checking the online driver catalog and comparing with this PC...");
        match update::get_online_catalog(&cfg.repo, &cfg.public_key, &cfg.online_catalog_path, d.http) {
            Ok(Some(r)) if r.updated => log(j, format!("Online catalog: updated to revision {} ({} entries)", r.revision, r.count)),
            Ok(_) => {}
            Err(e) => log(j, format!("Online catalog: {e} (using the stored copy)")),
        }
        let cat: Vec<CatalogEntry> = update::read_all_catalogs(&cfg.catalog_path, &cfg.online_catalog_path);
        items.extend(find_catalog_updates(&installed, &cat, &d.host, d.live_version));
        log(j, format!("Catalog: {} entries, {} newer entries match this PC", cat.len(), items.len()));

        status(j, "Checking the latest version from the manufacturer (NVIDIA)...");
        let mut vendor_devices: Vec<String> = Vec::new();
        match nvidia::find_updates(&installed, &d.host, d.http, d.live_version) {
            Ok(nv) => {
                if !nv.is_empty() { items.retain(|o| !(o.source == "Catalog" && o.class.eq_ignore_ascii_case("DISPLAY") && o.maker.to_ascii_lowercase().starts_with("nvidia"))); }
                vendor_devices.extend(nv.iter().map(|o| o.device_name.clone()));
                log(j, format!("NVIDIA (direct from manufacturer): {} newer entries found", nv.len()));
                items.extend(nv);
            }
            Err(e) => log(j, format!("NVIDIA (manufacturer): {e}")),
        }

        status(j, "Searching Microsoft Update Catalog by hardware ID (may take about 1 minute)...");
        let mut probe: Vec<InstalledDriver> = missing_devs.clone();
        probe.extend(installed.iter().cloned());
        match mscatalog::find_updates(&probe, &d.host, 40, &|k| mscatalog::search(d.http, k), d.live_version) {
            Ok(r) => {
                log(j, format!("Microsoft Update Catalog: {} newer entries found", r.offers.len()));
                if r.failed_queries > 0 { log(j, format!("Warning: {} Microsoft Catalog queries failed (results may be incomplete)", r.failed_queries)); }
                items.extend(r.offers.into_iter().filter(|o| !vendor_devices.contains(&o.device_name))); // the maker's own offer wins
            }
            Err(e) => log(j, format!("Microsoft Update Catalog: {e}")),
        }

        status(j, "Searching Windows Update for drivers (may take 1-2 minutes)...");
        match (d.windows_update)() { Ok(w) => items.extend(w), Err(e) => log(j, format!("Windows Update: {e}")) }

        let unresolved: Vec<InstalledDriver> = missing_devs.into_iter().filter(|m| !items.iter().any(|o| matches!(&o.kind, OfferKind::MsCatalog { name, .. } if *name == m.name))).collect();
        let mut g = j.lock().unwrap();
        g.status = format!("Scan complete: {} driver updates found", items.len());
        g.offers = items;
        g.unresolved = unresolved;
        Ok(())
    };
    if let Err(e) = run() { j.lock().unwrap().error = Some(e); }
    j.lock().unwrap().done = true;
}

pub struct InstallDeps<'a> {
    pub http: &'a dyn Http,
    pub sys: &'a dyn Sys,
    pub wu_install: &'a dyn Fn(&[String], &dyn Fn(&str)) -> Result<wu::WuInstall, String>,
    pub create_restore_point: &'a dyn Fn() -> Result<(), String>,
    pub work_root: PathBuf,
    pub backup_root: PathBuf,
    pub now: String,
    pub stamp: String,
}

/// Installs the chosen offers. `restore_point`: create one first (the caller passes false for later rounds).
pub fn install(j: &Shared, cfg: &Config, d: &InstallDeps, ids: &[String], offers: &HashMap<String, Offer>, restore_point: bool) {
    let run = || -> Result<(), String> {
        if restore_point {
            status(j, "Creating a system restore point...");
            match (d.create_restore_point)() { Ok(()) => log(j, "Restore point created"), Err(e) => log(j, format!("Restore point skipped: {e}")) }
        }
        for id in ids { set_item(j, id, Code::Wait, ""); }

        // keep a copy of the drivers about to be replaced, so they can be rolled back from the Backups tab
        status(j, "Backing up the current drivers...");
        let set_dir = d.backup_root.join(&d.stamp);
        let mut saved = Vec::new();
        for (n, id) in ids.iter().enumerate() {
            let Some(o) = offers.get(id) else { continue };
            if o.instance_id.is_empty() { continue; }
            match backup::backup_device_driver(&o.instance_id, &o.device_name, &set_dir, n + 1, &d.now, d.sys) {
                Some(e) => { log(j, format!("Backed up the current driver of {} ({})", o.device_name, e.old_version)); saved.push(e); }
                None => {}
            }
        }
        if !saved.is_empty() { backup::save_manifest(&set_dir, &saved)?; }

        let mut ok = 0usize;
        let (wu_ids, other_ids): (Vec<String>, Vec<String>) = ids.iter().cloned().partition(|id| matches!(offers.get(id).map(|o| &o.kind), Some(OfferKind::WindowsUpdate { .. })));

        if !wu_ids.is_empty() {
            if cancelled(j) {
                for id in &wu_ids { set_item(j, id, Code::Skip, "Stopped by request"); }
            } else {
                for id in &wu_ids { set_item(j, id, Code::Dl, ""); }
                match (d.wu_install)(&wu_ids, &|s| status(j, s)) {
                    Ok(r) => {
                        for line in &r.log { log(j, line.clone()); }
                        if r.reboot { j.lock().unwrap().reboot = true; }
                        for o in r.outcomes {
                            if o.success { ok += 1; set_item(j, &o.id, Code::Ok, ""); }
                            else if o.reason.starts_with("Not found") { set_item(j, &o.id, Code::Skip, &o.reason); }
                            else { set_item(j, &o.id, Code::Fail, &o.reason); }
                        }
                    }
                    Err(e) => { for id in &wu_ids { set_item(j, id, Code::Fail, &e); } log(j, format!("Windows Update: {e}")); }
                }
            }
        }

        for id in &other_ids {
            let Some(o) = offers.get(id) else { continue };
            if cancelled(j) { set_item(j, id, Code::Skip, "Stopped by request"); log(j, format!("[SKIPPED] {}: Stopped by request", o.title)); continue; }
            if cfg.block_catalog && o.source == "Catalog" {
                set_item(j, id, Code::Skip, "Program folder is writable by normal users");
                log(j, format!("[SKIPPED] {}: the program folder is writable by normal users and RequireProtectedFolder is set. Install with TierS-Driver-Manage-Setup.exe or turn this option off", o.title));
                continue;
            }
            status(j, &format!("Downloading and verifying: {}", o.title));
            set_item(j, id, Code::Dl, "");
            let work = d.work_root.join(format!("TierS_{}", crate::crypto::sha256_hex(id.as_bytes()).chars().take(12).collect::<String>()));
            let r: InstallResult = match &o.kind {
                OfferKind::MsCatalog { update_id, key, .. } => install::install_ms_item(update_id, key, &work, d.http, d.sys, None),
                OfferKind::Nvidia { url, version, signer, silent_args } => install::install_vendor_item(url, version, signer, silent_args, &o.instance_id, &work, d.http, d.sys, None),
                OfferKind::Catalog(c) => install::install_catalog_item(c, &work, d.http, d.sys, None),
                OfferKind::WindowsUpdate { .. } => continue,
            };
            let _ = std::fs::remove_dir_all(&work);
            if r.success { ok += 1; set_item(j, id, Code::Ok, ""); log(j, format!("[OK] {}", o.title)); }
            else { set_item(j, id, Code::Fail, &r.message); log(j, format!("[FAILED] {}: {}", o.title, r.message)); }
            if r.reboot { j.lock().unwrap().reboot = true; }
        }
        for id in ids {
            let waiting = j.lock().unwrap().items.get(id).map(|s| s.code == Code::Wait).unwrap_or(false);
            if waiting { set_item(j, id, Code::Skip, "Stopped by request"); }
        }
        let mut g = j.lock().unwrap();
        g.installed = ok;
        g.status = "This install round is finished".into();
        Ok(())
    };
    if let Err(e) = run() { j.lock().unwrap().error = Some(e); }
    j.lock().unwrap().done = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::OfferKind;
    use crate::crypto::sha256_hex;
    use crate::http::Fake;
    use crate::sys::fake::FakeSys;

    fn cfg(block: bool) -> Config { Config { catalog_path: "none".into(), online_catalog_path: "none".into(), repo: String::new(), public_key: String::new(), block_catalog: block, old_years: 3.0 } }
    fn host() -> HostInfo { HostInfo { arch: "x64".into(), os: "11".into() } }
    fn dev(name: &str, class: &str, maker: &str, ver: &str, hw: &str) -> InstalledDriver { InstalledDriver { instance_id: format!("I_{name}"), name: name.into(), class: class.into(), maker: maker.into(), version: ver.into(), date: None, hardware_ids: vec![hw.into()] } }

    fn offer_cat(name: &str, url: &str, data: &[u8]) -> Offer {
        let c = CatalogEntry { name: name.into(), kind: "installer".into(), url: url.into(), sha256: sha256_hex(data), signer: Some("Test Vendor".into()), silent_args: Some("/s".into()), version: "2.0".into(), match_: vec!["x".into()], ..Default::default() };
        Offer { id: format!("cat:{name}"), title: name.into(), class: "NET".into(), maker: "x".into(), date: String::new(), size_mb: None, source: "Catalog".into(), instance_id: format!("I_{name}"), device_name: name.into(), kind: OfferKind::Catalog(c) }
    }
    fn install_deps<'a>(http: &'a dyn Http, sys: &'a dyn Sys, wu: &'a dyn Fn(&[String], &dyn Fn(&str)) -> Result<wu::WuInstall, String>, rp: &'a dyn Fn() -> Result<(), String>, tag: &str) -> InstallDeps<'a> {
        let root = std::env::temp_dir().join(format!("tdm_eng_{}_{tag}", std::process::id()));
        InstallDeps { http, sys, wu_install: wu, create_restore_point: rp, work_root: root.join("work"), backup_root: root.join("bk"), now: "2026-11-08T01:00:00".into(), stamp: "set".into() }
    }
    fn no_wu(_: &[String], _: &dyn Fn(&str)) -> Result<wu::WuInstall, String> { Err("not used".into()) }
    fn rp_ok() -> Result<(), String> { Ok(()) }

    #[test]
    fn install_marks_ok_and_failed_items_and_logs_them() {
        let good = offer_cat("Good", "https://us.download.nvidia.com/x/good.exe", b"good");
        let bad = offer_cat("Bad", "https://us.download.nvidia.com/x/bad.exe", b"expected");
        let http = Fake::with_bytes(vec![("good.exe", b"good".to_vec()), ("bad.exe", b"tampered".to_vec())]);
        let sys = FakeSys::ok();
        let d = install_deps(&http, &sys, &no_wu, &rp_ok, "a");
        let offers: HashMap<_, _> = [good.clone(), bad.clone()].into_iter().map(|o| (o.id.clone(), o)).collect();
        let j = new_job();
        install(&j, &cfg(false), &d, &[good.id.clone(), bad.id.clone()], &offers, true);
        let g = j.lock().unwrap();
        assert!(g.done && g.error.is_none());
        assert_eq!(g.items[&good.id].code, Code::Ok);
        assert_eq!(g.items[&bad.id].code, Code::Fail);
        assert!(g.items[&bad.id].reason.contains("SHA-256 mismatch"));
        assert_eq!(g.installed, 1);
        assert!(g.log.iter().any(|l| l == "[OK] Good") && g.log.iter().any(|l| l.starts_with("[FAILED] Bad:")));
        assert!(g.log.iter().any(|l| l == "Restore point created"));
    }
    #[test]
    fn stop_request_skips_what_has_not_started() {
        let a = offer_cat("A", "https://us.download.nvidia.com/x/a.exe", b"a");
        let http = Fake::with_bytes(vec![("a.exe", b"a".to_vec())]);
        let sys = FakeSys::ok();
        let d = install_deps(&http, &sys, &no_wu, &rp_ok, "b");
        let offers: HashMap<_, _> = [a.clone()].into_iter().map(|o| (o.id.clone(), o)).collect();
        let j = new_job();
        j.lock().unwrap().cancel = true;
        install(&j, &cfg(false), &d, &[a.id.clone()], &offers, false);
        assert_eq!(j.lock().unwrap().items[&a.id].code, Code::Skip);
        assert_eq!(j.lock().unwrap().installed, 0);
        assert!(!sys.calls().iter().any(|c| c.starts_with("installer")));
    }
    #[test]
    fn catalog_installs_are_refused_from_a_writable_program_folder() {
        let a = offer_cat("A", "https://us.download.nvidia.com/x/a.exe", b"a");
        let http = Fake::with_bytes(vec![("a.exe", b"a".to_vec())]);
        let sys = FakeSys::ok();
        let d = install_deps(&http, &sys, &no_wu, &rp_ok, "c");
        let offers: HashMap<_, _> = [a.clone()].into_iter().map(|o| (o.id.clone(), o)).collect();
        let j = new_job();
        install(&j, &cfg(true), &d, &[a.id.clone()], &offers, false);
        assert_eq!(j.lock().unwrap().items[&a.id].code, Code::Skip);
        assert!(j.lock().unwrap().log.iter().any(|l| l.contains("writable by normal users")));
    }
    #[test]
    fn a_refused_restore_point_does_not_stop_the_install() {
        let a = offer_cat("A", "https://us.download.nvidia.com/x/a.exe", b"a");
        let http = Fake::with_bytes(vec![("a.exe", b"a".to_vec())]);
        let sys = FakeSys::ok();
        let rp = || -> Result<(), String> { Err("Windows allows one per 24 hours".into()) };
        let d = install_deps(&http, &sys, &no_wu, &rp, "d");
        let offers: HashMap<_, _> = [a.clone()].into_iter().map(|o| (o.id.clone(), o)).collect();
        let j = new_job();
        install(&j, &cfg(false), &d, &[a.id.clone()], &offers, true);
        assert_eq!(j.lock().unwrap().items[&a.id].code, Code::Ok);
        assert!(j.lock().unwrap().log.iter().any(|l| l.starts_with("Restore point skipped")));
    }
    #[test]
    fn windows_update_items_follow_the_wu_outcome_and_reboot_flag() {
        let w = Offer { id: "wu1".into(), title: "WU driver".into(), class: "NET".into(), maker: "x".into(), date: String::new(), size_mb: None, source: "Windows Update".into(), instance_id: String::new(), device_name: String::new(), kind: OfferKind::WindowsUpdate { update_id: "wu1".into() } };
        let http = Fake::new(vec![]); let sys = FakeSys::ok();
        let wu_fn = |ids: &[String], _s: &dyn Fn(&str)| -> Result<wu::WuInstall, String> {
            Ok(wu::WuInstall { outcomes: ids.iter().map(|i| wu::WuOutcome { id: i.clone(), success: true, reason: String::new() }).collect(), reboot: true, log: vec!["[OK] wu1 (succeeded)".into()] })
        };
        let d = install_deps(&http, &sys, &wu_fn, &rp_ok, "e");
        let offers: HashMap<_, _> = [w.clone()].into_iter().map(|o| (o.id.clone(), o)).collect();
        let j = new_job();
        install(&j, &cfg(false), &d, &[w.id.clone()], &offers, false);
        let g = j.lock().unwrap();
        assert_eq!(g.items["wu1"].code, Code::Ok);
        assert!(g.reboot && g.installed == 1);
    }

    // ---- scan ----
    fn scan_with<'a>(http: &'a dyn Http, installed: &'a dyn Fn() -> Result<Vec<InstalledDriver>, String>, wu_offers: &'a dyn Fn() -> Result<Vec<Offer>, String>, missing: &'a dyn Fn() -> Result<(Vec<InstalledDriver>, Vec<String>), String>) -> ScanDeps<'a> {
        static NO_LIVE: fn(&str) -> Option<String> = |_| None;
        ScanDeps { http, host: host(), installed, without_driver: missing, live_version: &NO_LIVE, windows_update: wu_offers, today: Date { y: 2026, m: 11, d: 8 } }
    }
    #[test]
    fn scan_without_internet_reports_the_plain_message() {
        let http = Fake::new(vec![]); // nothing answers
        let inst = || Ok(vec![]); let wu_o = || Ok(vec![]); let miss = || Ok((vec![], vec![]));
        let j = new_job();
        scan(&j, &cfg(false), &scan_with(&http, &inst, &wu_o, &miss));
        let g = j.lock().unwrap();
        assert!(g.done);
        assert_eq!(g.error.as_deref(), Some(NO_INTERNET));
    }
    #[test]
    fn scan_collects_inventory_missing_devices_and_unresolved_ones() {
        let http = Fake::new(vec![("connecttest", "ok"), ("Search.aspx", "<html>We did not find any results</html>"), ("TypeID=2", "<LookupValueSearch></LookupValueSearch>")]);
        let inst = || Ok(vec![dev("Wi-Fi", "NET", "Intel", "1.0.0.0", "PCI\\VEN_8086&DEV_1111&SUBSYS_1")]);
        let miss = || Ok((vec![dev("Mystery", "", "", "", "ACPI\\XYZ0001")], vec!["Mystery [problem code 28]".to_string()]));
        let wu_o = || Ok(vec![]);
        let j = new_job();
        scan(&j, &cfg(false), &scan_with(&http, &inst, &wu_o, &miss));
        let g = j.lock().unwrap();
        assert!(g.error.is_none(), "{:?}", g.error);
        assert_eq!(g.inventory.len(), 1);
        assert_eq!(g.missing, vec!["Mystery [problem code 28]"]);
        assert_eq!(g.unresolved.len(), 1);
        assert!(g.status.starts_with("Scan complete: 0 driver updates found"));
    }
    #[test]
    fn nvidia_offer_replaces_the_static_catalog_and_microsoft_offers_for_the_same_card() {
        // the NVIDIA lookup finds a newer driver for the RTX card; any Microsoft Catalog offer for that card is dropped
        let http = Fake::new(vec![
            ("connecttest", "ok"),
            ("TypeID=2", "<LookupValueSearch><LookupValues><LookupValue><Name>GeForce RTX 30 Series</Name><Value>120</Value></LookupValue></LookupValues></LookupValueSearch>"),
            ("ParentID=120", "<LookupValueSearch><LookupValues><LookupValue><Name>GeForce RTX 3060</Name><Value>933</Value></LookupValue></LookupValues></LookupValueSearch>"),
            ("psid=120&pfid=933", r#"{ "IDS":[{ "downloadInfo": { "Version":"700.01", "DownloadURL":"https://us.download.nvidia.com/Windows/700.01/x.exe", "DownloadURLFileSize":"1 GB", "ReleaseDateTime":"Wed Nov 04, 2026" } }] }"#),
            ("Search.aspx", "<html>We did not find any results</html>"),
        ]);
        let inst = || Ok(vec![dev("NVIDIA GeForce RTX 3060", "DISPLAY", "NVIDIA", "32.0.16.2114", "PCI\\VEN_10DE&DEV_2504&SUBSYS_1")]);
        let wu_o = || Ok(vec![]); let miss = || Ok((vec![], vec![]));
        let j = new_job();
        scan(&j, &cfg(false), &scan_with(&http, &inst, &wu_o, &miss));
        let g = j.lock().unwrap();
        assert_eq!(g.offers.len(), 1);
        assert_eq!(g.offers[0].source, "NVIDIA");
    }
}
