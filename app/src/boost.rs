//! "Boost" tab: a few documented Windows gaming settings, each one a switch.
//! Every change remembers the value it replaced (boost.json), so switching off puts the exact old value back.
//! Nothing here touches security features, services, drivers or the network stack.
use crate::sys::{RealSys, Sys};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub trait Reg {
    /// (type, data) of a value; DWORD data as decimal text. None = missing.
    fn get(&self, key: &str, name: &str) -> Option<(String, String)>;
    fn set(&self, key: &str, name: &str, kind: &str, data: &str) -> Result<(), String>;
    fn delete(&self, key: &str, name: &str) -> Result<(), String>;
}

pub struct RegExe;

impl Reg for RegExe {
    fn get(&self, key: &str, name: &str) -> Option<(String, String)> {
        let (code, out) = RealSys.run("reg.exe", &["query", key, "/v", name]);
        if code != 0 { return None; }
        out.lines().find_map(|l| {
            let rest = l.trim().strip_prefix(name)?;
            let rest = rest.trim_start();
            let (kind, data) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            if !kind.starts_with("REG_") { return None; }
            let data = data.trim();
            let data = if kind == "REG_DWORD" { u32::from_str_radix(data.trim_start_matches("0x"), 16).ok()?.to_string() } else { data.to_string() };
            Some((kind.to_string(), data))
        })
    }
    fn set(&self, key: &str, name: &str, kind: &str, data: &str) -> Result<(), String> {
        let (code, out) = RealSys.run("reg.exe", &["add", key, "/v", name, "/t", kind, "/d", data, "/f"]);
        if code == 0 { Ok(()) } else { Err(out.trim().to_string()) }
    }
    fn delete(&self, key: &str, name: &str) -> Result<(), String> {
        if self.get(key, name).is_none() { return Ok(()); }
        let (code, out) = RealSys.run("reg.exe", &["delete", key, "/v", name, "/f"]);
        if code == 0 { Ok(()) } else { Err(out.trim().to_string()) }
    }
}

/// What a value should become.
#[derive(Clone, Copy)]
pub enum Val { D(u32), S(&'static str), /// add one `Name=1;` entry to a `;` list
    Add(&'static str), /// remove it again
    Strip(&'static str), Del }

pub struct Target { pub key: &'static str, pub name: &'static str, pub on: Val, /// value used when we have no backup
    pub off: Val }

pub struct Tweak {
    pub id: &'static str, pub title: &'static str, pub why: &'static str, pub effect: &'static str,
    pub restart: bool, pub recommended: bool, pub targets: &'static [Target],
}

macro_rules! k { ($s:literal) => { concat!("HKCU\\", $s) }; }

pub const TWEAKS: &[Tweak] = &[
    Tweak { id: "mouse", title: "Turn off mouse acceleration", effect: "Input", restart: false, recommended: true,
        why: "Turns off \"Enhance pointer precision\". The cursor then moves the same distance for the same hand movement, so aim is consistent.",
        targets: &[
            Target { key: k!("Control Panel\\Mouse"), name: "MouseSpeed", on: Val::S("0"), off: Val::S("1") },
            Target { key: k!("Control Panel\\Mouse"), name: "MouseThreshold1", on: Val::S("0"), off: Val::S("6") },
            Target { key: k!("Control Panel\\Mouse"), name: "MouseThreshold2", on: Val::S("0"), off: Val::S("10") },
        ] },
    Tweak { id: "gamemode", title: "Game Mode on", effect: "Small", restart: false, recommended: true,
        why: "Windows lowers the priority of background work and postpones driver and update installs while a game runs.",
        targets: &[Target { key: k!("Software\\Microsoft\\GameBar"), name: "AutoGameModeEnabled", on: Val::D(1), off: Val::Del }] },
    Tweak { id: "dvr", title: "Turn off background game recording", effect: "Small to medium", restart: false, recommended: true,
        why: "Stops Xbox Game Bar from recording in the background. The recording takes GPU and disk time even when you never use it. Manual capture with Win+G still works.",
        targets: &[
            Target { key: k!("System\\GameConfigStore"), name: "GameDVR_Enabled", on: Val::D(0), off: Val::D(1) },
            Target { key: k!("Software\\Microsoft\\Windows\\CurrentVersion\\GameDVR"), name: "AppCaptureEnabled", on: Val::D(0), off: Val::D(1) },
        ] },
    Tweak { id: "windowed", title: "Optimizations for windowed games", effect: "Medium (windowed and borderless games)", restart: false, recommended: true,
        why: "DirectX 10 and 11 games in a window or borderless mode use the same low-delay presentation as full screen. Takes effect the next time a game starts.",
        targets: &[Target { key: k!("Software\\Microsoft\\DirectX\\UserGpuPreferences"), name: "DirectXUserGlobalSettings", on: Val::Add("SwapEffectUpgradeEnable=1;"), off: Val::Strip("SwapEffectUpgradeEnable=1;") }] },
    Tweak { id: "effects", title: "Turn off transparency and taskbar animation", effect: "Low-end PCs", restart: false, recommended: false,
        why: "Fewer visual effects, so older PCs and weak graphics feel lighter. The look of Windows changes a little.",
        targets: &[
            Target { key: k!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"), name: "EnableTransparency", on: Val::D(0), off: Val::D(1) },
            Target { key: k!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced"), name: "TaskbarAnimations", on: Val::D(0), off: Val::D(1) },
        ] },
    Tweak { id: "hags", title: "Hardware-accelerated GPU scheduling", effect: "Depends on the graphics card", restart: true, recommended: false,
        why: "The graphics card schedules its own work. Helps some newer cards (RTX 40 and newer, RX 7000 and newer) and can make older ones stutter. Try it and keep it only if games feel better.",
        targets: &[Target { key: "HKLM\\SYSTEM\\CurrentControlSet\\Control\\GraphicsDrivers", name: "HwSchMode", on: Val::D(2), off: Val::Del }] },
    Tweak { id: "timer", title: "System-wide fine timer", effect: "Only some games", restart: true, recommended: false,
        why: "Gives every program the fine timer a game asked for (older games that run capped near 64 FPS). Uses a little more power. Games that set their own timer already ignore this.",
        targets: &[Target { key: "HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\kernel", name: "GlobalTimerResolutionRequests", on: Val::D(1), off: Val::Del }] },
];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Prior { pub key: String, pub name: String, pub old: Option<(String, String)> }

/// tweak id -> the values it replaced
pub type Store = BTreeMap<String, Vec<Prior>>;

pub fn load(path: &Path) -> Store { std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default() }

pub fn save(path: &Path, s: &Store) -> Result<(), String> {
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn entry_key(tok: &str) -> &str { tok.split('=').next().unwrap_or(tok) }
fn entries(s: &str) -> Vec<&str> { s.split(';').map(str::trim).filter(|x| !x.is_empty()).collect() }

/// The new (type, data) for a value, or None to delete it.
fn compute(existing: Option<&str>, v: Val) -> Option<(&'static str, String)> {
    match v {
        Val::D(n) => Some(("REG_DWORD", n.to_string())),
        Val::S(s) => Some(("REG_SZ", s.to_string())),
        Val::Del => None,
        Val::Add(tok) | Val::Strip(tok) => {
            let mut list: Vec<&str> = entries(existing.unwrap_or("")).into_iter().filter(|e| entry_key(e) != entry_key(tok)).collect();
            if matches!(v, Val::Add(_)) { list.push(tok.trim_end_matches(';')); }
            if list.is_empty() { None } else { Some(("REG_SZ", list.iter().map(|e| format!("{e};")).collect())) }
        }
    }
}

fn target_on(reg: &dyn Reg, t: &Target) -> bool {
    let cur = reg.get(t.key, t.name);
    match t.on {
        Val::D(n) => cur.map(|(k, d)| k == "REG_DWORD" && d == n.to_string()).unwrap_or(false),
        Val::S(s) => cur.map(|(_, d)| d == s).unwrap_or(false),
        Val::Add(tok) => cur.map(|(_, d)| entries(&d).contains(&tok.trim_end_matches(';'))).unwrap_or(false),
        Val::Strip(_) | Val::Del => cur.is_none(),
    }
}

pub fn is_on(reg: &dyn Reg, t: &Tweak) -> bool { t.targets.iter().all(|x| target_on(reg, x)) }

fn write(reg: &dyn Reg, key: &str, name: &str, v: Val) -> Result<(), String> {
    let cur = reg.get(key, name);
    match compute(cur.as_ref().map(|c| c.1.as_str()), v) {
        Some((kind, data)) => reg.set(key, name, kind, &data),
        None => reg.delete(key, name),
    }
}

fn put_back(reg: &dyn Reg, p: &Prior) -> Result<(), String> {
    match &p.old { Some((kind, data)) => reg.set(&p.key, &p.name, kind, data), None => reg.delete(&p.key, &p.name) }
}

/// Switch one tweak on or off. On a failure part-way, what was already changed is put back.
pub fn set(reg: &dyn Reg, store: &mut Store, t: &Tweak, on: bool) -> Result<(), String> {
    if on {
        if store.contains_key(t.id) { return Ok(()); }
        let priors: Vec<Prior> = t.targets.iter().map(|x| Prior { key: x.key.into(), name: x.name.into(), old: reg.get(x.key, x.name) }).collect();
        for (i, x) in t.targets.iter().enumerate() {
            if let Err(e) = write(reg, x.key, x.name, x.on) {
                for p in &priors[..i] { let _ = put_back(reg, p); }
                return Err(format!("{}: {e}", t.title));
            }
        }
        store.insert(t.id.into(), priors);
        Ok(())
    } else if let Some(priors) = store.get(t.id).cloned() {
        for p in &priors { put_back(reg, p).map_err(|e| format!("{}: {e}", t.title))?; }
        store.remove(t.id);
        Ok(())
    } else {
        // switched on outside this program (no backup): use the Windows default
        for x in t.targets { write(reg, x.key, x.name, x.off).map_err(|e| format!("{}: {e}", t.title))?; }
        Ok(())
    }
}

pub fn tweak(id: &str) -> Option<&'static Tweak> { TWEAKS.iter().find(|t| t.id == id) }

/// Applies the mouse values to the running session (the registry alone only takes effect at the next sign-in).
pub fn refresh_mouse(reg: &dyn Reg) {
    use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SPI_SETMOUSE};
    let g = |n: &str| reg.get("HKCU\\Control Panel\\Mouse", n).and_then(|(_, d)| d.trim().parse::<i32>().ok()).unwrap_or(0);
    let mut v = [g("MouseThreshold1"), g("MouseThreshold2"), g("MouseSpeed")];
    unsafe { let _ = SystemParametersInfoW(SPI_SETMOUSE, 0, Some(v.as_mut_ptr() as *mut _), SPIF_UPDATEINIFILE | SPIF_SENDCHANGE); }
}

/// Names of the graphics cards, for the in-game latency advice.
pub fn gpu_names() -> Vec<String> {
    #[derive(Deserialize)] #[serde(rename_all = "PascalCase")] struct V { name: Option<String> }
    let Ok(con) = wmi::WMIConnection::new() else { return vec![] };
    con.raw_query::<V>("SELECT Name FROM Win32_VideoController").map(|v| v.into_iter().filter_map(|x| x.name).collect()).unwrap_or_default()
}

/// The in-game setting that lowers input lag the most, for this card.
pub fn latency_tip(gpus: &[String]) -> (&'static str, &'static str) {
    let all = gpus.join(" ").to_lowercase();
    if all.contains("nvidia") || all.contains("geforce") { ("NVIDIA Reflex", "In games that have it, set NVIDIA Reflex to On (or On + Boost). It is the biggest input-lag reduction you can get, and it is a setting inside the game.") }
    else if all.contains("radeon") || all.contains("amd") { ("AMD Anti-Lag", "Turn on Anti-Lag in AMD Software (Gaming, Graphics) and, in games that offer it, Anti-Lag 2 in the game's own settings.") }
    else if all.contains("intel") { ("Low-latency mode", "Keep the Intel graphics driver up to date and, in games that offer a low-latency or Reflex-style option, turn it on.") }
    else { ("Low-latency mode", "In games that offer a low-latency option, turn it on.") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Fake { m: RefCell<HashMap<(String, String), (String, String)>>, fail_on: Option<&'static str> }
    impl Reg for Fake {
        fn get(&self, k: &str, n: &str) -> Option<(String, String)> { self.m.borrow().get(&(k.into(), n.into())).cloned() }
        fn set(&self, k: &str, n: &str, kind: &str, d: &str) -> Result<(), String> {
            if self.fail_on == Some(n) { return Err("denied".into()); }
            self.m.borrow_mut().insert((k.into(), n.into()), (kind.into(), d.into())); Ok(())
        }
        fn delete(&self, k: &str, n: &str) -> Result<(), String> { self.m.borrow_mut().remove(&(k.into(), n.into())); Ok(()) }
    }

    #[test]
    fn switch_on_then_off_puts_back_the_exact_old_values() {
        let reg = Fake::default();
        reg.set("HKCU\\Control Panel\\Mouse", "MouseSpeed", "REG_SZ", "2").unwrap();
        let (mut st, t) = (Store::new(), tweak("mouse").unwrap());
        assert!(!is_on(&reg, t));
        set(&reg, &mut st, t, true).unwrap();
        assert!(is_on(&reg, t));
        set(&reg, &mut st, t, false).unwrap();
        assert_eq!(reg.get("HKCU\\Control Panel\\Mouse", "MouseSpeed").unwrap().1, "2");
        assert!(reg.get("HKCU\\Control Panel\\Mouse", "MouseThreshold1").is_none());
        assert!(st.is_empty());
    }

    #[test]
    fn windowed_setting_keeps_the_other_entries() {
        let (reg, t) = (Fake::default(), tweak("windowed").unwrap());
        let (k, n) = (t.targets[0].key, t.targets[0].name);
        reg.set(k, n, "REG_SZ", "VRROptimizeEnable=0;").unwrap();
        let mut st = Store::new();
        set(&reg, &mut st, t, true).unwrap();
        assert_eq!(reg.get(k, n).unwrap().1, "VRROptimizeEnable=0;SwapEffectUpgradeEnable=1;");
        assert!(is_on(&reg, t));
        set(&reg, &mut st, t, false).unwrap();
        assert_eq!(reg.get(k, n).unwrap().1, "VRROptimizeEnable=0;");
    }

    #[test]
    fn a_failure_part_way_leaves_nothing_changed() {
        let reg = Fake { fail_on: Some("AppCaptureEnabled"), ..Default::default() };
        let (mut st, t) = (Store::new(), tweak("dvr").unwrap());
        assert!(set(&reg, &mut st, t, true).is_err());
        assert!(reg.get(t.targets[0].key, t.targets[0].name).is_none());
        assert!(st.is_empty());
    }

    #[test]
    fn off_without_a_backup_writes_the_windows_default() {
        let (reg, t) = (Fake::default(), tweak("dvr").unwrap());
        reg.set(t.targets[0].key, t.targets[0].name, "REG_DWORD", "0").unwrap();
        set(&reg, &mut Store::new(), t, false).unwrap();
        assert_eq!(reg.get(t.targets[0].key, t.targets[0].name).unwrap().1, "1");
    }

    #[test]
    fn real_reg_exe_round_trip_on_a_scratch_key() {
        let key = "HKCU\\Software\\TierS-Test-Boost";
        let r = RegExe;
        r.set(key, "D", "REG_DWORD", "7").unwrap();
        r.set(key, "S", "REG_SZ", "a b;c=1;").unwrap();
        assert_eq!(r.get(key, "D"), Some(("REG_DWORD".into(), "7".into())));
        assert_eq!(r.get(key, "S"), Some(("REG_SZ".into(), "a b;c=1;".into())));
        r.delete(key, "D").unwrap();
        assert!(r.get(key, "D").is_none());
        r.delete(key, "missing").unwrap();
        let _ = RealSys.run("reg.exe", &["delete", key, "/f"]);
    }

    #[test]
    fn advice_follows_the_card() {
        assert_eq!(latency_tip(&["NVIDIA GeForce RTX 4070".into()]).0, "NVIDIA Reflex");
        assert_eq!(latency_tip(&["AMD Radeon RX 7800 XT".into()]).0, "AMD Anti-Lag");
    }
}
