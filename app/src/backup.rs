//! Driver backup before an install and roll back. Same folder and manifest format as the PowerShell version, so old backups still work.
use crate::sys::Sys;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub fn backup_root() -> PathBuf {
    PathBuf::from(std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".into())).join("TierS Driver Manage").join("Backups")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct BackupEntry {
    pub device: String,
    pub instance_id: String,
    pub old_inf: String,
    pub old_version: String,
    #[serde(default)] pub folder: String,
    #[serde(default)] pub time: String,
}

#[derive(Debug, Clone)]
pub struct Backup { pub set_dir: PathBuf, pub entry: BackupEntry }

fn is_oem_inf(inf: &str) -> bool {
    let l = inf.to_ascii_lowercase();
    l.strip_prefix("oem").and_then(|r| r.strip_suffix(".inf")).map(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit())).unwrap_or(false)
}

fn has_inf(dir: &Path) -> bool {
    fs::read_dir(dir).map(|rd| rd.flatten().any(|e| { let p = e.path(); if p.is_dir() { has_inf(&p) } else { p.extension().map(|x| x.eq_ignore_ascii_case("inf")).unwrap_or(false) } })).unwrap_or(false)
}

/// Exports the driver now bound to the device. Returns the manifest entry, or `None` when there is nothing to back up
/// (no driver yet, or a driver built into Windows: rolling back then simply removes the new package).
pub fn backup_device_driver(instance_id: &str, device_name: &str, set_dir: &Path, index: usize, now: &str, sys: &dyn Sys) -> Option<BackupEntry> {
    let inf = sys.driver_inf(instance_id).filter(|s| !s.is_empty())?;
    let mut e = BackupEntry { device: device_name.into(), instance_id: instance_id.into(), old_inf: inf.clone(), old_version: sys.live_driver_version(instance_id).unwrap_or_default(), folder: String::new(), time: now.into() };
    if is_oem_inf(&inf) {
        let dir = set_dir.join(index.to_string());
        fs::create_dir_all(&dir).ok()?;
        let _ = sys.run("pnputil.exe", &["/export-driver", &inf, &dir.to_string_lossy()]);
        if !has_inf(&dir) { return None; }
        e.folder = index.to_string();
    }
    Some(e)
}

pub fn save_manifest(set_dir: &Path, entries: &[BackupEntry]) -> Result<(), String> {
    fs::create_dir_all(set_dir).map_err(|e| e.to_string())?;
    fs::write(set_dir.join("manifest.json"), serde_json::to_vec_pretty(entries).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn read_manifest(path: &Path) -> Vec<BackupEntry> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum M { Many(Vec<BackupEntry>), One(BackupEntry) }
    let Ok(text) = fs::read_to_string(path) else { return vec![] };
    match serde_json::from_str::<M>(text.trim_start_matches('\u{feff}')) { Ok(M::Many(v)) => v, Ok(M::One(e)) => vec![e], Err(_) => vec![] }
}

/// Every backed-up device, newest first.
pub fn list_backups(root: &Path) -> Vec<Backup> {
    let mut all = Vec::new();
    if let Ok(rd) = fs::read_dir(root) {
        for set in rd.flatten().filter(|e| e.path().is_dir()) {
            for entry in read_manifest(&set.path().join("manifest.json")) { all.push(Backup { set_dir: set.path(), entry }); }
        }
    }
    all.sort_by(|a, b| b.entry.time.cmp(&a.entry.time));
    all
}

/// Puts the old driver back: install the saved package (when one was saved), then remove the package the device uses now so Windows
/// falls back to the old one (or to the built-in driver).
pub fn restore_device_driver(b: &Backup, sys: &dyn Sys) -> Result<String, String> {
    let now = sys.driver_inf(&b.entry.instance_id).filter(|s| !s.is_empty()).ok_or("The device is not present or has no driver now.")?;
    if now.eq_ignore_ascii_case(&b.entry.old_inf) { return Ok("The device already uses the backed-up driver.".into()); }
    if !b.entry.folder.is_empty() {
        let dir = b.set_dir.join(&b.entry.folder);
        if !has_inf(&dir) { return Err("The backup files are missing.".into()); }
        let pattern = dir.join("*.inf");
        let (code, out) = sys.run("pnputil.exe", &["/add-driver", &pattern.to_string_lossy(), "/subdirs", "/install"]);
        if code != 0 && code != 3010 { return Err(format!("Could not add the old driver: {}", out.trim())); }
    }
    if !is_oem_inf(&now) { return Err("The current driver is built into Windows and cannot be removed.".into()); }
    let (code, out) = sys.run("pnputil.exe", &["/delete-driver", &now, "/uninstall", "/force"]);
    if code != 0 && code != 3010 { return Err(format!("Could not remove the new driver: {}", out.trim())); }
    Ok(format!("Rolled back {} to version {}. A restart may be needed.", b.entry.device, b.entry.old_version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::fake::FakeSys;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tdm_bk_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&d);
        d
    }
    fn sys_with(inf: &str, ver: &str) -> FakeSys { let mut s = FakeSys::ok(); s.inf = Some(inf.into()); s.live = Some(ver.into()); s }

    #[test]
    fn third_party_driver_is_exported_and_recorded() {
        let set = tmp("a"); let sys = sys_with("oem34.inf", "1.0.0.0");
        let e = backup_device_driver("USB\\X", "Test Bluetooth", &set, 1, "2026-11-08T01:00:00", &sys).unwrap();
        assert_eq!((e.folder.as_str(), e.old_inf.as_str(), e.old_version.as_str()), ("1", "oem34.inf", "1.0.0.0"));
        assert!(sys.calls().iter().any(|c| c.contains("/export-driver oem34.inf")));
        assert!(set.join("1/old.inf").exists());
    }
    #[test]
    fn built_in_driver_is_recorded_without_files_and_no_driver_gets_no_entry() {
        let set = tmp("b");
        let e = backup_device_driver("PCI\\X", "Built-in", &set, 2, "t", &sys_with("netrtl64.inf", "10.0")).unwrap();
        assert_eq!(e.folder, "");
        assert!(backup_device_driver("PCI\\Y", "None", &set, 3, "t", &FakeSys::ok()).is_none());
    }
    #[test]
    fn export_that_produces_no_inf_gives_no_entry() {
        let set = tmp("c"); let mut sys = sys_with("oem34.inf", "1"); sys.export_files = false;
        assert!(backup_device_driver("X", "D", &set, 1, "t", &sys).is_none());
    }
    #[test]
    fn manifests_round_trip_newest_first_and_a_single_object_manifest_loads() {
        let root = tmp("d");
        let mk = |dev: &str, time: &str| BackupEntry { device: dev.into(), instance_id: "i".into(), old_inf: "oem1.inf".into(), old_version: "1".into(), folder: "1".into(), time: time.into() };
        save_manifest(&root.join("set1"), &[mk("A", "2026-01-01T00:00:00"), mk("B", "2026-01-01T00:00:00")]).unwrap();
        save_manifest(&root.join("set2"), &[mk("C", "2026-02-01T00:00:00")]).unwrap();
        // PowerShell wrote a lone entry as an object, not an array
        fs::create_dir_all(root.join("set3")).unwrap();
        fs::write(root.join("set3/manifest.json"), r#"{"Device":"D","InstanceId":"i","OldInf":"oem2.inf","OldVersion":"2","Folder":"","Time":"2026-03-01T00:00:00"}"#).unwrap();
        let l = list_backups(&root);
        assert_eq!(l.iter().map(|b| b.entry.device.as_str()).collect::<Vec<_>>(), vec!["D", "C", "A", "B"]);
    }
    fn saved(set: &Path, folder: &str) -> Backup {
        fs::create_dir_all(set.join("1")).unwrap(); fs::write(set.join("1/old.inf"), ";").unwrap();
        Backup { set_dir: set.to_path_buf(), entry: BackupEntry { device: "Test Bluetooth".into(), instance_id: "i".into(), old_inf: "oem34.inf".into(), old_version: "1.0.0.0".into(), folder: folder.into(), time: "t".into() } }
    }
    #[test]
    fn rollback_adds_the_old_package_then_removes_the_new_one() {
        let set = tmp("e"); let b = saved(&set, "1"); let sys = sys_with("oem99.inf", "2.0.0.0");
        let msg = restore_device_driver(&b, &sys).unwrap();
        assert!(msg.contains("Rolled back Test Bluetooth to version 1.0.0.0"));
        let c = sys.calls();
        assert_eq!(c.len(), 2);
        assert!(c[0].contains("/add-driver") && c[0].ends_with("/subdirs /install"));
        assert_eq!(c[1], "pnputil.exe /delete-driver oem99.inf /uninstall /force");
    }
    #[test]
    fn rollback_touches_nothing_when_the_device_already_uses_the_saved_driver() {
        let set = tmp("f"); let b = saved(&set, "1"); let sys = sys_with("oem34.inf", "1");
        assert!(restore_device_driver(&b, &sys).unwrap().contains("already uses"));
        assert!(sys.calls().is_empty());
    }
    #[test]
    fn a_built_in_current_driver_is_never_deleted() {
        let set = tmp("g"); let b = saved(&set, "1");
        assert!(restore_device_driver(&b, &sys_with("netrtl64.inf", "9")).unwrap_err().contains("built into Windows"));
    }
    #[test]
    fn pnputil_failures_are_reported() {
        let set = tmp("h"); let b = saved(&set, "1"); let mut sys = sys_with("oem99.inf", "2"); sys.exit_code = 5;
        assert!(restore_device_driver(&b, &sys).unwrap_err().contains("Could not add the old driver"));
    }
    #[test]
    fn missing_backup_files_and_missing_device_are_reported() {
        let set = tmp("i"); let mut b = saved(&set, "1"); b.entry.folder = "9".into();
        assert!(restore_device_driver(&b, &sys_with("oem99.inf", "2")).unwrap_err().contains("backup files are missing"));
        assert!(restore_device_driver(&b, &FakeSys::ok()).unwrap_err().contains("not present"));
    }
}
