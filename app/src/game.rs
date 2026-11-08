//! Per-game settings (full-screen optimizations off, higher CPU/IO priority) and "faster power plan while a game runs".
//! Every registry change goes through boost::set_raw, so switching it off puts the exact old value back.
use crate::boost::{self, Reg, Store};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Game {
    /// file name, e.g. "cs2.exe"
    pub exe: String,
    /// full path (needed for the full-screen setting; Windows keys it by path)
    pub path: String,
    pub fso: bool,
    pub prio: bool,
}

const LAYERS: &str = "HKCU\\Software\\Microsoft\\Windows NT\\CurrentVersion\\AppCompatFlags\\Layers";
const IFEO: &str = "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Image File Execution Options";
const FSO_FLAG: &str = "DISABLEDXMAXIMIZEDWINDOWEDMODE";

/// A plain program file name: the name becomes a registry key, so no path characters are allowed.
pub fn valid_exe(s: &str) -> bool {
    let l = s.to_lowercase();
    l.ends_with(".exe") && l.len() <= 64 && !s.chars().any(|c| matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'))
}

pub fn exe_of_path(p: &str) -> String { p.rsplit(['\\', '/']).next().unwrap_or("").to_string() }

fn fso_items(reg: &dyn Reg, g: &Game) -> Vec<(String, String, Option<(String, String)>)> {
    let cur = reg.get(LAYERS, &g.path).map(|c| c.1).unwrap_or_default();
    let mut flags: Vec<&str> = cur.split_whitespace().filter(|t| *t != "~").collect();
    if !flags.contains(&FSO_FLAG) { flags.push(FSO_FLAG); }
    vec![(LAYERS.into(), g.path.clone(), Some(("REG_SZ".into(), format!("~ {}", flags.join(" ")))))]
}

fn prio_items(g: &Game) -> Vec<(String, String, Option<(String, String)>)> {
    let key = format!("{IFEO}\\{}\\PerfOptions", g.exe);
    // 3 = High (never Realtime: it can freeze the PC)
    vec![(key.clone(), "CpuPriorityClass".into(), Some(("REG_DWORD".into(), "3".into()))), (key, "IoPriority".into(), Some(("REG_DWORD".into(), "3".into())))]
}

fn fso_id(g: &Game) -> String { format!("game-fso:{}", g.path.to_lowercase()) }
fn prio_id(g: &Game) -> String { format!("game-prio:{}", g.exe.to_lowercase()) }

/// Makes the registry match the two switches of this game.
pub fn apply(reg: &dyn Reg, store: &mut Store, g: &Game) -> Result<(), String> {
    if !valid_exe(&g.exe) { return Err(format!("{} is not a program file name", g.exe)); }
    if !g.path.is_empty() { boost::set_raw(reg, store, &fso_id(g), &fso_items(reg, g), g.fso)?; }
    boost::set_raw(reg, store, &prio_id(g), &prio_items(g), g.prio)
}

/// Puts back everything this program changed for the game.
pub fn remove(reg: &dyn Reg, store: &mut Store, g: &Game) -> Result<(), String> {
    boost::set_raw(reg, store, &fso_id(g), &[], false)?;
    boost::set_raw(reg, store, &prio_id(g), &[], false)
}

// ---------- faster power plan while a game runs ----------

pub trait Power {
    fn active(&self) -> Option<String>;
    fn set(&self, guid: &str) -> Result<(), String>;
    /// (process id, lower-case exe name) of the window in front
    fn foreground(&self) -> Option<(u32, String)>;
    fn alive(&self, pid: u32) -> bool;
}

#[derive(Default)]
pub struct Watch {
    pub on: bool,
    pub plan: String,
    /// plan to go back to; None = nothing was changed by us
    saved: Option<String>,
    pid: Option<u32>,
    last: Option<Instant>,
    pub note: String,
}

impl Watch {
    pub fn new(on: bool, plan: String) -> Watch { Watch { on, plan, ..Default::default() } }
    pub fn running(&self) -> bool { self.pid.is_some() }

    fn revert(&mut self, p: &dyn Power) -> Option<String> {
        self.pid = None;
        let back = self.saved.take()?;
        Some(match p.set(&back) { Ok(()) => { self.note.clear(); "Game plan: game closed, power plan put back".to_string() } Err(e) => format!("[FAILED] Game plan: could not put the power plan back: {e}") })
    }

    /// Called often; looks at most every 2 seconds. Returns log lines.
    pub fn tick(&mut self, p: &dyn Power, now: Instant, games: &[String]) -> Vec<String> {
        if self.last.map(|l| now.duration_since(l) < Duration::from_secs(2)).unwrap_or(false) { return vec![]; }
        self.last = Some(now);
        if !self.on || self.plan.is_empty() { return self.revert(p).into_iter().collect(); }
        if let Some(pid) = self.pid {
            return if p.alive(pid) { vec![] } else { self.revert(p).into_iter().collect() };
        }
        let Some((pid, exe)) = p.foreground() else { return vec![] };
        if !games.iter().any(|g| g.eq_ignore_ascii_case(&exe)) { return vec![]; }
        let before = p.active();
        self.pid = Some(pid);
        if before.as_deref() == Some(self.plan.as_str()) { return vec![]; }
        match p.set(&self.plan) {
            Ok(()) => { self.saved = before; self.note = format!("{exe} is running: faster power plan on"); vec![format!("Game plan: {exe} started, faster power plan on")] }
            Err(e) => { self.pid = None; self.on = false; vec![format!("[FAILED] Game plan: {e}. Turned off.")] }
        }
    }

    /// Program closing: do not leave the PC on the game plan.
    pub fn finish(&mut self, p: &dyn Power) { let _ = self.revert(p); }
}

pub struct RealPower;

impl Power for RealPower {
    fn active(&self) -> Option<String> { crate::power::cfg::active(&crate::sys::RealSys).ok().map(|s| s.guid) }
    fn set(&self, guid: &str) -> Result<(), String> { crate::power::cfg::set_active(&crate::sys::RealSys, guid) }
    fn foreground(&self) -> Option<(u32, String)> {
        use windows::core::PWSTR;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
        unsafe {
            let h = GetForegroundWindow();
            if h.0.is_null() { return None; }
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, Some(&mut pid));
            if pid == 0 { return None; }
            let ph = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 520];
            let mut n = buf.len() as u32;
            let r = QueryFullProcessImageNameW(ph, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n);
            let _ = CloseHandle(ph);
            r.ok()?;
            Some((pid, exe_of_path(&String::from_utf16_lossy(&buf[..n as usize])).to_lowercase()))
        }
    }
    fn alive(&self, pid: u32) -> bool {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code).is_ok() && code == 259; // STILL_ACTIVE
            let _ = CloseHandle(h);
            ok
        }
    }
}

/// "Open file" dialog for a game's .exe; None when cancelled.
pub fn pick_exe() -> Option<String> {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW};
    let filter: Vec<u16> = "Programs (*.exe)\0*.exe\0\0".encode_utf16().collect();
    let title: Vec<u16> = "Choose the game's program file\0".encode_utf16().collect();
    let mut buf = [0u16; 1024];
    let mut ofn = OPENFILENAMEW { lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32, lpstrFilter: PCWSTR(filter.as_ptr()), lpstrFile: PWSTR(buf.as_mut_ptr()), nMaxFile: buf.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()), Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR, ..Default::default() };
    if !unsafe { GetOpenFileNameW(&mut ofn) }.as_bool() { return None; }
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Fake(RefCell<HashMap<(String, String), (String, String)>>);
    impl Reg for Fake {
        fn get(&self, k: &str, n: &str) -> Option<(String, String)> { self.0.borrow().get(&(k.into(), n.into())).cloned() }
        fn set(&self, k: &str, n: &str, kind: &str, d: &str) -> Result<(), String> { self.0.borrow_mut().insert((k.into(), n.into()), (kind.into(), d.into())); Ok(()) }
        fn delete(&self, k: &str, n: &str) -> Result<(), String> { self.0.borrow_mut().remove(&(k.into(), n.into())); Ok(()) }
    }
    fn game() -> Game { Game { exe: "cs2.exe".into(), path: "C:\\Games\\cs2.exe".into(), fso: true, prio: true } }

    #[test]
    fn game_switches_apply_and_undo_exactly() {
        let (reg, mut st, mut g) = (Fake::default(), Store::new(), game());
        reg.set(LAYERS, &g.path, "REG_SZ", "~ RUNASADMIN").unwrap();
        apply(&reg, &mut st, &g).unwrap();
        assert_eq!(reg.get(LAYERS, &g.path).unwrap().1, "~ RUNASADMIN DISABLEDXMAXIMIZEDWINDOWEDMODE");
        assert_eq!(reg.get(&format!("{IFEO}\\cs2.exe\\PerfOptions"), "CpuPriorityClass").unwrap().1, "3");
        g.fso = false; g.prio = false;
        apply(&reg, &mut st, &g).unwrap();
        assert_eq!(reg.get(LAYERS, &g.path).unwrap().1, "~ RUNASADMIN");
        assert!(reg.get(&format!("{IFEO}\\cs2.exe\\PerfOptions"), "CpuPriorityClass").is_none());
        assert!(st.is_empty());
    }

    #[test]
    fn remove_puts_everything_back() {
        let (reg, mut st, g) = (Fake::default(), Store::new(), game());
        apply(&reg, &mut st, &g).unwrap();
        remove(&reg, &mut st, &g).unwrap();
        assert!(reg.0.borrow().is_empty() && st.is_empty());
    }

    #[test]
    fn exe_names_cannot_climb_the_registry() {
        assert!(valid_exe("cs2.exe") && valid_exe("VALORANT-Win64-Shipping.exe"));
        assert!(!valid_exe("..\\evil.exe") && !valid_exe("a\\b.exe") && !valid_exe("notes.txt") && !valid_exe(""));
        let (reg, mut st) = (Fake::default(), Store::new());
        assert!(apply(&reg, &mut st, &Game { exe: "x\\y.exe".into(), path: String::new(), fso: false, prio: true }).is_err());
    }

    struct FakePower { active: RefCell<String>, fg: RefCell<Option<(u32, String)>>, alive: RefCell<bool>, sets: RefCell<Vec<String>> }
    impl Power for FakePower {
        fn active(&self) -> Option<String> { Some(self.active.borrow().clone()) }
        fn set(&self, g: &str) -> Result<(), String> { *self.active.borrow_mut() = g.into(); self.sets.borrow_mut().push(g.into()); Ok(()) }
        fn foreground(&self) -> Option<(u32, String)> { self.fg.borrow().clone() }
        fn alive(&self, _: u32) -> bool { *self.alive.borrow() }
    }

    #[test]
    fn plan_goes_up_when_the_game_starts_and_back_when_it_closes() {
        let p = FakePower { active: RefCell::new("balanced".into()), fg: RefCell::new(Some((7, "cs2.exe".into()))), alive: RefCell::new(true), sets: RefCell::default() };
        let mut w = Watch { on: true, plan: "ultimate".into(), ..Default::default() };
        let games = vec!["cs2.exe".to_string()];
        let t0 = Instant::now();
        w.tick(&p, t0, &games);
        assert_eq!(*p.active.borrow(), "ultimate");
        *p.fg.borrow_mut() = Some((9, "chrome.exe".into())); // alt-tab: still running, plan stays
        w.tick(&p, t0 + Duration::from_secs(3), &games);
        assert_eq!(*p.active.borrow(), "ultimate");
        *p.alive.borrow_mut() = false;
        w.tick(&p, t0 + Duration::from_secs(6), &games);
        assert_eq!(*p.active.borrow(), "balanced");
        assert!(!w.running());
    }

    #[test]
    fn real_calls_read_the_plan_and_tell_live_from_dead_processes() {
        let p = RealPower;
        assert!(p.active().is_some());
        assert!(p.alive(std::process::id()));
        assert!(!p.alive(0x00FF_FFF0));
        let _ = p.foreground(); // None is fine when no window is in front (CI)
    }

    #[test]
    fn nothing_changes_when_the_plan_is_already_the_game_plan_or_the_switch_is_off() {
        let p = FakePower { active: RefCell::new("ultimate".into()), fg: RefCell::new(Some((7, "cs2.exe".into()))), alive: RefCell::new(true), sets: RefCell::default() };
        let mut w = Watch { on: true, plan: "ultimate".into(), ..Default::default() };
        w.tick(&p, Instant::now(), &["cs2.exe".to_string()]);
        assert!(p.sets.borrow().is_empty());
        let mut off = Watch::default();
        off.tick(&p, Instant::now(), &["cs2.exe".to_string()]);
        assert!(p.sets.borrow().is_empty());
    }
}
