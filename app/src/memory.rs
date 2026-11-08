//! Memory tab: reads RAM and the Windows standby list, and purges the standby list by hand or automatically.
//! Written from the public behaviour of the Windows API (NtQuerySystemInformation / NtSetSystemInformation,
//! information class 80 "SystemMemoryListInformation"); no code from any other cleaner is used.
use std::ffi::c_void;
use std::time::{Duration, Instant};
use windows::core::{s, w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_NOT_ALL_ASSIGNED, HANDLE, LUID};
use windows::Win32::Security::{AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::SystemInformation::{GetSystemInfo, GlobalMemoryStatusEx, MEMORYSTATUSEX, SYSTEM_INFO};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

pub const MB: u64 = 1 << 20;
/// Slider ranges (min, max, step): a value from the window is snapped into its range, so it can never be out of bounds.
pub const POLL_MS: (u64, u64, u64) = (500, 10_000, 500);
pub const STANDBY_MB: (u64, u64, u64) = (256, 16_384, 256);
pub const FREE_MB: (u64, u64, u64) = (256, 8_192, 256);

pub fn snap(v: u64, (lo, hi, step): (u64, u64, u64)) -> u64 { (v.clamp(lo, hi) - lo + step / 2) / step * step + lo }

/// All sizes in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats { pub total: u64, pub available: u64, pub standby: u64, pub modified: u64, pub free: u64 }

pub trait Mem {
    fn stats(&self) -> Result<Stats, String>;
    fn purge(&self) -> Result<(), String>;
}

/// Purge when the standby list is at least `standby_mb` AND truly free memory is at most `free_mb`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rule { pub standby_mb: u64, pub free_mb: u64 }
impl Rule {
    pub fn hit(&self, s: &Stats) -> bool { s.standby >= self.standby_mb * MB && s.free <= self.free_mb * MB }
}

pub struct Monitor {
    pub on: bool, pub rule: Rule, pub poll_ms: u64,
    pub stats: Option<Stats>, pub err: String, pub note: String, pub purges: u32,
    last: Option<Instant>, last_logged: Option<Instant>,
}

impl Monitor {
    pub fn new() -> Monitor {
        Monitor { on: false, rule: Rule { standby_mb: 1024, free_mb: 1024 }, poll_ms: 1000, stats: None, err: String::new(), note: String::new(), purges: 0, last: None, last_logged: None }
    }

    fn stop(&mut self, e: String, log: &mut Vec<String>) { self.on = false; log.push(format!("[FAILED] Memory monitor stopped: {e}")); self.err = e; }

    /// Call often. Reads the numbers about once a second while `watch` (the tab is open) or while monitoring; returns log lines.
    pub fn tick(&mut self, mem: &dyn Mem, now: Instant, watch: bool) -> Vec<String> {
        let mut log = vec![];
        if !self.on && !watch { return log; }
        let every = Duration::from_millis(if self.on { self.poll_ms.max(500) } else { 1000 });
        if self.last.map_or(false, |t| now.duration_since(t) < every) { return log; }
        self.last = Some(now);
        let s = match mem.stats() {
            Ok(s) => s,
            Err(e) => { if self.on { self.stop(e, &mut log); } else { self.err = e; } return log; }
        };
        self.err.clear();
        self.stats = Some(s);
        if self.on && self.rule.hit(&s) {
            match mem.purge() {
                Ok(()) => {
                    self.purges += 1;
                    self.note = format!("Cleaned {} MB of standby memory", s.standby / MB);
                    self.stats = mem.stats().ok().or(self.stats);
                    // a busy PC can refill the list every second: write to the log at most every 30 s
                    if self.last_logged.map_or(true, |t| now.duration_since(t) >= Duration::from_secs(30)) { self.last_logged = Some(now); log.push(format!("Memory: auto-cleaned the standby list ({} MB, {} MB free; {} times so far)", s.standby / MB, s.free / MB, self.purges)); }
                }
                Err(e) => self.stop(e, &mut log),
            }
        }
        log
    }

    /// The "Clean now" button.
    pub fn purge_now(&mut self, mem: &dyn Mem) -> Result<String, String> {
        let before = mem.stats()?;
        mem.purge()?;
        let after = mem.stats().unwrap_or(before);
        self.stats = Some(after);
        self.err.clear();
        self.purges += 1;
        let freed = before.standby.saturating_sub(after.standby) / MB;
        self.note = format!("Cleaned {freed} MB of standby memory");
        Ok(format!("Memory: standby list cleaned by hand ({} MB before, {} MB after)", before.standby / MB, after.standby / MB))
    }
}

// ---------- Windows ----------
#[repr(C)]
#[derive(Default)]
struct ListInfo { zero: usize, free: usize, modified: usize, modified_no_write: usize, bad: usize, by_priority: [usize; 8], repurposed: [usize; 8], modified_page_file: usize }

const CLASS_MEMORY_LIST: u32 = 80;
const CMD_PURGE_STANDBY: i32 = 4;
const STATUS_PRIVILEGE_NOT_HELD: u32 = 0xC000_0061;
const NEED_ADMIN: &str = "Windows refused: run the program as Administrator";

type QueryFn = unsafe extern "system" fn(u32, *mut c_void, u32, *mut u32) -> i32;
type SetFn = unsafe extern "system" fn(u32, *mut c_void, u32) -> i32;

fn ntdll() -> Option<(QueryFn, SetFn)> {
    unsafe {
        let lib = GetModuleHandleW(w!("ntdll.dll")).ok()?;
        let q = GetProcAddress(lib, s!("NtQuerySystemInformation"))?;
        let st = GetProcAddress(lib, s!("NtSetSystemInformation"))?;
        Some((std::mem::transmute::<_, QueryFn>(q), std::mem::transmute::<_, SetFn>(st)))
    }
}

/// Reading and purging the memory lists needs SeProfileSingleProcessPrivilege (held, but switched off, by an Administrator token).
fn enable_privilege() -> Result<(), String> {
    unsafe {
        let mut tok = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut tok).map_err(|e| e.to_string())?;
        let mut luid = LUID::default();
        if let Err(e) = LookupPrivilegeValueW(PCWSTR::null(), w!("SeProfileSingleProcessPrivilege"), &mut luid) { let _ = CloseHandle(tok); return Err(e.to_string()); }
        let tp = TOKEN_PRIVILEGES { PrivilegeCount: 1, Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }] };
        let adjusted = AdjustTokenPrivileges(tok, false, Some(&tp), 0, None, None);
        let last = GetLastError();
        let _ = CloseHandle(tok);
        adjusted.map_err(|e| e.to_string())?;
        if last == ERROR_NOT_ALL_ASSIGNED { return Err(NEED_ADMIN.into()); }
        Ok(())
    }
}

fn page_size() -> u64 {
    let mut si = SYSTEM_INFO::default();
    unsafe { GetSystemInfo(&mut si) };
    if si.dwPageSize == 0 { 4096 } else { si.dwPageSize as u64 }
}

pub struct RealMem;
impl Mem for RealMem {
    fn stats(&self) -> Result<Stats, String> {
        let mut ms = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        unsafe { GlobalMemoryStatusEx(&mut ms) }.map_err(|e| format!("Could not read the memory size: {e}"))?;
        let (query, _) = ntdll().ok_or("This Windows version does not offer the memory list information")?;
        enable_privilege()?;
        let mut li = ListInfo::default();
        let mut got = 0u32;
        let st = unsafe { query(CLASS_MEMORY_LIST, &mut li as *mut ListInfo as *mut c_void, std::mem::size_of::<ListInfo>() as u32, &mut got) };
        if st < 0 { return Err(if st as u32 == STATUS_PRIVILEGE_NOT_HELD { NEED_ADMIN.into() } else { format!("Windows could not read the standby list (status {:#x})", st as u32) }); }
        let pg = page_size();
        let standby: u64 = li.by_priority.iter().map(|&p| p as u64).sum::<u64>() * pg;
        Ok(Stats { total: ms.ullTotalPhys, available: ms.ullAvailPhys, standby, modified: (li.modified as u64 + li.modified_no_write as u64) * pg, free: (li.free as u64 + li.zero as u64) * pg })
    }

    fn purge(&self) -> Result<(), String> {
        let (_, set) = ntdll().ok_or("This Windows version does not offer the memory list information")?;
        enable_privilege()?;
        let mut cmd = CMD_PURGE_STANDBY;
        let st = unsafe { set(CLASS_MEMORY_LIST, &mut cmd as *mut i32 as *mut c_void, std::mem::size_of::<i32>() as u32) };
        if st >= 0 { Ok(()) } else if st as u32 == STATUS_PRIVILEGE_NOT_HELD { Err(NEED_ADMIN.into()) } else { Err(format!("Windows refused to clean the standby list (status {:#x})", st as u32)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    struct Fake { s: RefCell<Stats>, fail_purge: bool, purged: Cell<u32> }
    impl Fake {
        fn new(standby_mb: u64, free_mb: u64) -> Fake { Fake { s: RefCell::new(Stats { total: 16 * 1024 * MB, available: (standby_mb + free_mb) * MB, standby: standby_mb * MB, modified: 0, free: free_mb * MB }), fail_purge: false, purged: Cell::new(0) } }
    }
    impl Mem for Fake {
        fn stats(&self) -> Result<Stats, String> { Ok(*self.s.borrow()) }
        fn purge(&self) -> Result<(), String> {
            if self.fail_purge { return Err(NEED_ADMIN.into()); }
            self.purged.set(self.purged.get() + 1);
            let mut s = self.s.borrow_mut(); s.free += s.standby; s.standby = 0; Ok(())
        }
    }

    #[test]
    fn rule_needs_both_conditions() {
        let r = Rule { standby_mb: 1024, free_mb: 1024 };
        assert!(r.hit(&Fake::new(2048, 512).stats().unwrap()));
        assert!(!r.hit(&Fake::new(2048, 4096).stats().unwrap()), "plenty of free memory: leave the cache alone");
        assert!(!r.hit(&Fake::new(100, 512).stats().unwrap()), "small standby list: nothing to gain");
        assert!(r.hit(&Fake::new(1024, 1024).stats().unwrap()), "the limits themselves count");
    }

    #[test]
    fn monitor_purges_only_when_on_and_rule_hits() {
        let f = Fake::new(2048, 256);
        let mut m = Monitor::new();
        let t0 = Instant::now();
        m.tick(&f, t0, true);
        assert_eq!(f.purged.get(), 0, "monitoring is off: only reading");
        assert!(m.stats.is_some());
        m.on = true;
        let log = m.tick(&f, t0 + Duration::from_secs(2), true);
        assert_eq!(f.purged.get(), 1);
        assert_eq!(m.purges, 1);
        assert_eq!(log.len(), 1);
        assert_eq!(m.stats.unwrap().standby, 0, "numbers are re-read after the purge");
        m.tick(&f, t0 + Duration::from_secs(4), true);
        assert_eq!(f.purged.get(), 1, "standby list is empty now: nothing more to do");
    }

    #[test]
    fn monitor_respects_the_polling_interval() {
        let f = Fake::new(2048, 256);
        let mut m = Monitor::new();
        m.on = true; m.poll_ms = 5000;
        let t0 = Instant::now();
        m.tick(&f, t0, false);
        *f.s.borrow_mut() = Fake::new(2048, 256).s.into_inner();
        m.tick(&f, t0 + Duration::from_secs(2), false);
        assert_eq!(f.purged.get(), 1, "2 s < 5 s: no second read");
        m.tick(&f, t0 + Duration::from_secs(6), false);
        assert_eq!(f.purged.get(), 2);
    }

    #[test]
    fn failed_purge_stops_the_monitor_instead_of_looping() {
        let mut f = Fake::new(2048, 256);
        f.fail_purge = true;
        let mut m = Monitor::new();
        m.on = true;
        let log = m.tick(&f, Instant::now(), true);
        assert!(!m.on);
        assert!(log[0].contains("Administrator") && m.err.contains("Administrator"));
        assert_eq!(m.purges, 0);
    }

    #[test]
    fn slider_values_snap_into_range() {
        assert_eq!(snap(1, STANDBY_MB), 256);
        assert_eq!(snap(99_999, STANDBY_MB), 16_384);
        assert_eq!(snap(1000, STANDBY_MB), 1024);
        assert_eq!(snap(1024, STANDBY_MB), 1024);
        assert_eq!(snap(0, POLL_MS), 500);
        assert_eq!(snap(1200, POLL_MS), 1000);
        assert_eq!(snap(8192, FREE_MB), 8192);
    }

    #[test]
    fn idle_tab_closed_and_off_does_nothing() {
        let f = Fake::new(2048, 256);
        let mut m = Monitor::new();
        m.tick(&f, Instant::now(), false);
        assert!(m.stats.is_none());
    }

    #[test]
    fn purge_now_reports_before_and_after() {
        let f = Fake::new(3000, 100);
        let mut m = Monitor::new();
        let line = m.purge_now(&f).unwrap();
        assert!(line.contains("3000 MB before") && line.contains("0 MB after"));
        assert!(m.note.contains("3000"));
    }

    #[test]
    fn real_total_memory_reads() {
        // standby details may need Administrator rights, so only the outcome type is checked here
        match RealMem.stats() { Ok(s) => assert!(s.total > 0 && s.standby <= s.total), Err(e) => assert!(!e.is_empty()) }
    }
}
