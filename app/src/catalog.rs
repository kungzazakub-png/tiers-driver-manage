//! Driver catalog entries and the rules that decide whether an entry applies to an installed device.
use crate::policy;
use crate::version::Ver;
use serde::{Deserialize, Deserializer, Serialize};

fn one_or_many<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum V { One(String), Many(Vec<String>), None }
    Ok(match V::deserialize(d)? { V::One(s) => vec![s], V::Many(v) => v, V::None => vec![] })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub name: String,
    #[serde(default)] pub publisher: String,
    #[serde(rename = "match", default, deserialize_with = "one_or_many")] pub match_: Vec<String>,
    #[serde(default)] pub version: String,
    #[serde(default)] pub date: Option<String>,
    #[serde(rename = "sizeMB", default)] pub size_mb: Option<f64>,
    #[serde(rename = "type", default)] pub kind: String,
    #[serde(default)] pub url: String,
    #[serde(default)] pub sha256: String,
    #[serde(default, deserialize_with = "one_or_many")] pub os: Vec<String>,
    #[serde(default, deserialize_with = "one_or_many")] pub arch: Vec<String>,
    #[serde(default)] pub class: Option<String>,
    #[serde(rename = "nameLike", default, deserialize_with = "one_or_many")] pub name_like: Vec<String>,
    #[serde(default)] pub signer: Option<String>,
    #[serde(rename = "silentArgs", default)] pub silent_args: Option<String>,
}

/// driverdb.json (shipped / local) and the online cache share this shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CatalogFile {
    #[serde(default)] pub revision: i64,
    #[serde(default)] pub drivers: Vec<CatalogEntry>,
}

pub fn parse_catalog(text: &str) -> Result<Vec<CatalogEntry>, String> {
    let f: CatalogFile = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    Ok(f.drivers.into_iter().filter(|c| !c.name.is_empty()).collect())
}

/// Calendar date (driver dates carry no time of interest).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date { pub y: i32, pub m: u32, pub d: u32 }
impl Date {
    pub fn iso(&self) -> String { format!("{:04}-{:02}-{:02}", self.y, self.m, self.d) }
    /// "2026-01-12" or "2026-01-12T00:00:00"
    pub fn parse_iso(s: &str) -> Option<Date> {
        let mut it = s.get(..10)?.split('-');
        Some(Date { y: it.next()?.parse().ok()?, m: it.next()?.parse().ok()?, d: it.next()?.parse().ok()? })
    }
}

#[derive(Debug, Clone)]
pub struct InstalledDriver {
    pub instance_id: String,
    pub name: String,
    pub class: String,
    pub maker: String,
    pub version: String,
    pub date: Option<Date>,
    pub hardware_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct HostInfo { pub arch: String, pub os: String }

/// Where an offer comes from and what is needed to install it.
#[derive(Debug, Clone)]
pub enum OfferKind {
    Catalog(CatalogEntry),
    MsCatalog { update_id: String, key: String, name: String },
    Nvidia { url: String, version: String, signer: String, silent_args: String },
    WindowsUpdate { update_id: String },
}

#[derive(Debug, Clone)]
pub struct Offer {
    pub id: String,
    pub title: String,
    pub class: String,
    pub maker: String,
    pub date: String,
    pub size_mb: Option<f64>,
    pub source: String,
    pub instance_id: String,
    pub device_name: String,
    pub kind: OfferKind,
}

/// PowerShell `-like` as the catalog uses it: `*` and `?`, case-insensitive.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) { pi += 1; ti += 1; }
        else if pi < p.len() && p[pi] == '*' { star = Some(pi); mark = ti; pi += 1; }
        else if let Some(s) = star { pi = s + 1; mark += 1; ti = mark; }
        else { return false; }
    }
    while pi < p.len() && p[pi] == '*' { pi += 1; }
    pi == p.len()
}

/// An entry applies when: its os/arch (if given) fit this PC, one "match" wildcard fits an installed device's hardware/compatible ID,
/// the device class equals "class" (if given), AND the catalog version is newer than installed.
/// `live_version` re-reads the driver version from Plug and Play (WMI can keep showing the old version after an install).
pub fn find_catalog_updates(installed: &[InstalledDriver], catalog: &[CatalogEntry], host: &HostInfo, live_version: &dyn Fn(&str) -> Option<String>) -> Vec<Offer> {
    let mut found = Vec::new();
    for c in catalog {
        let Some(new_v) = Ver::parse(&c.version) else { continue };
        if !c.os.is_empty() && !c.os.iter().any(|o| o == &host.os) { continue; }
        if !c.arch.is_empty() && !c.arch.iter().any(|a| a == &host.arch) { continue; }
        for d in installed {
            if let Some(cl) = &c.class { if !cl.eq_ignore_ascii_case(&d.class) { continue; } }
            if !c.name_like.is_empty() && !c.name_like.iter().any(|n| wildcard_match(n, &d.name)) { continue; }
            let hit = c.match_.iter().any(|pat| d.hardware_ids.iter().any(|h| !pat.is_empty() && wildcard_match(pat, h)));
            if !hit { continue; }
            let mut cur = Ver::parse(&d.version);
            if let Some(live) = live_version(&d.instance_id).and_then(|v| Ver::parse(&v)) { cur = Some(live); }
            if let Some(cur) = cur { if new_v <= cur { continue; } }
            found.push(Offer {
                id: format!("cat:{} {}", c.name, c.version),
                title: format!("{} {}", c.name, c.version),
                class: d.class.clone(), maker: c.publisher.clone(), date: c.date.clone().unwrap_or_default(), size_mb: c.size_mb,
                source: "Catalog".into(), instance_id: d.instance_id.clone(), device_name: d.name.clone(), kind: OfferKind::Catalog(c.clone()),
            });
            break;
        }
    }
    found
}

/// The checks an online catalog entry must pass before the program will use it. Returns the reason when it fails.
pub fn check_entry(c: &CatalogEntry) -> Option<String> {
    if c.name.is_empty() { return Some("no name".into()); }
    if Ver::parse(&c.version).is_none() { return Some(format!("invalid version ({})", c.version)); }
    if c.match_.iter().all(|m| m.is_empty()) { return Some("no match".into()); }
    if c.kind != "installer" && c.kind != "inf-zip" { return Some(format!("unknown type ({})", c.kind)); }
    if c.sha256.len() != 64 || !c.sha256.chars().all(|ch| ch.is_ascii_hexdigit()) { return Some("invalid sha256".into()); }
    if let Err(e) = policy::assert_allowed_url(&c.url) { return Some(e); }
    if c.kind == "installer" && (c.signer.as_deref().unwrap_or("").is_empty() || c.silent_args.as_deref().unwrap_or("").is_empty()) {
        return Some("installer needs signer and silentArgs".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> HostInfo { HostInfo { arch: "x64".into(), os: "11".into() } }
    fn no_live(_: &str) -> Option<String> { None }
    fn dev(name: &str, class: &str, ver: &str, hw: &[&str]) -> InstalledDriver {
        InstalledDriver { instance_id: "i".into(), name: name.into(), class: class.into(), maker: "x".into(), version: ver.into(), date: None, hardware_ids: hw.iter().map(|s| s.to_string()).collect() }
    }
    fn entry(json: &str) -> CatalogEntry { serde_json::from_str(json).unwrap() }
    fn nv() -> CatalogEntry {
        entry(r#"{"name":"NV","match":["PCI\\VEN_10DE&DEV_*"],"class":"DISPLAY","version":"2.0.0.0","type":"installer","url":"https://us.download.nvidia.com/x/setup.exe","sha256":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","signer":"NVIDIA","silentArgs":"-s","nameLike":["NVIDIA GeForce RTX*","NVIDIA GeForce GTX 16*"]}"#)
    }

    #[test]
    fn wildcards_behave_like_powershell_like() {
        assert!(wildcard_match("PCI\\VEN_10DE&DEV_*", "pci\\ven_10de&dev_2504&subsys_1"));
        assert!(wildcard_match("a?c", "abc"));
        assert!(!wildcard_match("a?c", "ac"));
        assert!(wildcard_match("*", ""));
        assert!(!wildcard_match("abc*", "ab"));
        assert!(wildcard_match("*b*", "abc"));
    }

    #[test]
    fn newer_version_is_matched() {
        let c = entry(r#"{"name":"T","match":["PCI\\VEN_1234&DEV_*"],"version":"2.0.0.0","type":"inf-zip","url":"https://downloadmirror.intel.com/x.zip","sha256":"A"}"#);
        let o = find_catalog_updates(&[dev("D", "NET", "1.0.0.0", &["PCI\\VEN_1234&DEV_0001&SUBSYS_1"])], &[c], &host(), &no_live);
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].id, "cat:T 2.0.0.0");
    }
    #[test]
    fn same_or_older_version_is_ignored() {
        let c = nv();
        let hw = ["PCI\\VEN_10DE&DEV_2504"];
        assert!(find_catalog_updates(&[dev("NVIDIA GeForce RTX 3060", "DISPLAY", "2.0.0.0", &hw)], &[c.clone()], &host(), &no_live).is_empty());
        assert!(find_catalog_updates(&[dev("NVIDIA GeForce RTX 3060", "DISPLAY", "3.0.0.0", &hw)], &[c], &host(), &no_live).is_empty());
    }
    #[test]
    fn unrelated_hardware_is_ignored() {
        assert!(find_catalog_updates(&[dev("NVIDIA GeForce RTX 3060", "DISPLAY", "1.0.0.0", &["PCI\\VEN_8086&DEV_1"])], &[nv()], &host(), &no_live).is_empty());
    }
    #[test]
    fn class_filter_skips_same_vendor_audio_device() {
        let hw = ["PCI\\VEN_10DE&DEV_228E"];
        assert!(find_catalog_updates(&[dev("NVIDIA High Definition Audio", "MEDIA", "1.0.0.0", &hw)], &[nv()], &host(), &no_live).is_empty());
    }
    #[test]
    fn os_and_arch_filters() {
        let mut c = nv(); c.os = vec!["10".into()];
        let hw = ["PCI\\VEN_10DE&DEV_2504"];
        assert!(find_catalog_updates(&[dev("NVIDIA GeForce RTX 3060", "DISPLAY", "1.0.0.0", &hw)], &[c], &host(), &no_live).is_empty());
        let mut c2 = nv(); c2.arch = vec!["arm64".into()];
        assert!(find_catalog_updates(&[dev("NVIDIA GeForce RTX 3060", "DISPLAY", "1.0.0.0", &hw)], &[c2], &host(), &no_live).is_empty());
    }
    #[test]
    fn name_like_picks_the_right_cards() {
        let hw = ["PCI\\VEN_10DE&DEV_1"];
        let m = |n: &str| !find_catalog_updates(&[dev(n, "DISPLAY", "1.0.0.0", &hw)], &[nv()], &host(), &no_live).is_empty();
        assert!(m("NVIDIA GeForce RTX 3060"));
        assert!(!m("NVIDIA GeForce GTX 970"));
        assert!(!m("NVIDIA Quadro P2000"));
    }
    #[test]
    fn live_version_suppresses_an_offer_after_install() {
        let hw = ["PCI\\VEN_10DE&DEV_2504"];
        let live = |_: &str| Some("2.0.0.0".to_string());
        assert!(find_catalog_updates(&[dev("NVIDIA GeForce RTX 3060", "DISPLAY", "1.0.0.0", &hw)], &[nv()], &host(), &live).is_empty());
    }

    #[test]
    fn single_strings_and_arrays_both_load() {
        let c = entry(r#"{"name":"T","match":"PCI\\X*","os":"11","version":"1.0"}"#);
        assert_eq!(c.match_, vec!["PCI\\X*"]);
        assert_eq!(c.os, vec!["11"]);
    }
    #[test]
    fn the_shipped_catalog_loads_and_passes_every_check() {
        let list = parse_catalog(include_str!("../../DriverScan/driverdb.json")).unwrap();
        assert!(list.len() >= 5);
        for c in &list { assert_eq!(check_entry(c), None, "entry {}", c.name); }
    }
    #[test]
    fn entry_checks_reject_bad_entries() {
        let mut c = nv();
        c.url = "https://evil.example.com/x.exe".into();
        assert!(check_entry(&c).unwrap().contains("allowed list"));
        let mut c = nv(); c.sha256 = String::new();
        assert_eq!(check_entry(&c).as_deref(), Some("invalid sha256"));
        let mut c = nv(); c.silent_args = None;
        assert!(check_entry(&c).unwrap().contains("silentArgs"));
        let mut c = nv(); c.kind = "exe".into();
        assert!(check_entry(&c).unwrap().contains("unknown type"));
    }
    #[test]
    fn intel_graphics_entries_match_the_right_gpus_only() {
        let list = parse_catalog(include_str!("../../DriverScan/driverdb.json")).unwrap();
        let names = |d: InstalledDriver| find_catalog_updates(&[d], &list, &host(), &no_live).into_iter().map(|o| o.title).collect::<Vec<_>>().join(",");
        assert!(names(dev("Intel(R) Arc(TM) A770 Graphics", "DISPLAY", "31.0.101.4000", &["PCI\\VEN_8086&DEV_56A0&SUBSYS_0&REV_08"])).starts_with("Intel Arc Graphics Driver"));
        assert!(names(dev("Intel(R) UHD Graphics 770", "DISPLAY", "31.0.101.4000", &["PCI\\VEN_8086&DEV_4680&SUBSYS_0&REV_0C"])).starts_with("Intel Graphics Driver (11th-14th Gen)"));
        assert_eq!(names(dev("Intel(R) UHD Graphics 630", "DISPLAY", "27.20.100.8000", &["PCI\\VEN_8086&DEV_9BC8&SUBSYS_0&REV_05"])), "");
        assert_eq!(names(dev("Intel(R) Arc(TM) A770 Graphics", "DISPLAY", "32.0.101.9175", &["PCI\\VEN_8086&DEV_56A0&SUBSYS_0&REV_08"])), "");
    }
}
