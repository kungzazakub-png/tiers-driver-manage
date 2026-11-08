use super::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

struct State { plans: Vec<(String, String)>, active: String, vals: HashMap<(String, String), (u32, u32)>, n: u32, calls: Vec<String> }
struct FakeCfg { s: RefCell<State>, fail_set_active: bool, fail_export: bool, ultimate_missing: bool, unsupported: HashSet<String> }

const USER_PLAN: &str = "11111111-2222-3333-4444-555555555555";

impl FakeCfg {
    fn new() -> FakeCfg {
        let mut vals = HashMap::new();
        for g in [cfg::BALANCED, USER_PLAN] {
            for s in [&profiles::MIN_STATE, &profiles::MAX_STATE, &profiles::COOLING, &profiles::USB_SUSPEND, &profiles::PCIE_LINK, &profiles::BOOST_MODE, &profiles::PARKING_MIN, &profiles::IDLE_DISABLE] {
                let v = if s.guid == profiles::MIN_STATE.guid { (10, 10) } else if s.guid == profiles::MAX_STATE.guid { (100, 100) } else if s.guid == profiles::COOLING.guid { (0, 0) } else { (1, 1) };
                vals.insert((g.to_string(), s.guid.to_string()), v);
            }
        }
        let plans = vec![(cfg::BALANCED.into(), "Balanced".into()), ("8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c".into(), "High performance".into()), (USER_PLAN.into(), "My plan".into())];
        FakeCfg { s: RefCell::new(State { plans, active: USER_PLAN.into(), vals, n: 0, calls: vec![] }), fail_set_active: false, fail_export: false, ultimate_missing: false, unsupported: HashSet::new() }
    }
    fn calls(&self) -> Vec<String> { self.s.borrow().calls.clone() }
    fn active_guid(&self) -> String { self.s.borrow().active.clone() }
    fn has(&self, g: &str) -> bool { self.s.borrow().plans.iter().any(|p| p.0 == g) }
    fn val(&self, scheme: &str, setting: &str) -> (u32, u32) { self.s.borrow().vals[&(scheme.to_string(), setting.to_string())] }
}

impl Cfg for FakeCfg {
    fn powercfg(&self, a: &[&str]) -> (i32, String) {
        let mut s = self.s.borrow_mut();
        s.calls.push(a.join(" "));
        match a[0] {
            "/list" => (0, s.plans.iter().map(|(g, n)| format!("Power Scheme GUID: {g}  ({n}){}\n", if *g == s.active { " *" } else { "" })).collect()),
            "/getactivescheme" => { let n = s.plans.iter().find(|p| p.0 == s.active).map(|p| p.1.clone()).unwrap_or_default(); (0, format!("Power Scheme GUID: {}  ({n})", s.active)) }
            "/duplicatescheme" => {
                if a[1] == cfg::ULTIMATE && self.ultimate_missing { return (1, "The power scheme does not exist".into()); }
                s.n += 1; let g = format!("00000000-0000-0000-0000-{:012}", s.n);
                let src = a[1].to_string();
                let copy: Vec<_> = s.vals.iter().filter(|((sc, _), _)| *sc == src || (src == cfg::ULTIMATE && sc == cfg::BALANCED)).map(|((_, st), v)| (st.clone(), *v)).collect();
                for (st, v) in copy { s.vals.insert((g.clone(), st), v); }
                s.plans.push((g.clone(), "copy".into()));
                (0, format!("Power Scheme GUID: {g}  (copy)"))
            }
            "/changename" => { let g = a[1].to_string(); if let Some(p) = s.plans.iter_mut().find(|p| p.0 == g) { p.1 = a[2].to_string(); } (0, String::new()) }
            "/setactive" => { if self.fail_set_active { return (5, "Access is denied.".into()); } s.active = a[1].into(); (0, String::new()) }
            "/delete" => { let g = a[1].to_string(); s.plans.retain(|p| p.0 != g); (0, String::new()) }
            "/export" => { if self.fail_export { return (1, "cannot export".into()); } std::fs::write(a[1], "pow").unwrap(); (0, String::new()) }
            "/import" => { s.n += 1; let g = format!("99999999-0000-0000-0000-{:012}", s.n); s.plans.push((g.clone(), "imported".into())); (0, format!("Imported Power Scheme GUID: {g}")) }
            "/query" | "/qh" => {
                if self.unsupported.contains(a[3]) { return (1, "The power scheme, subgroup or setting specified does not exist.".into()); }
                match s.vals.get(&(a[1].to_string(), a[3].to_string())) { Some(v) => (0, format!("Power Setting GUID: {}\n  Maximum Possible Setting: 0x00000064\n  Current AC Power Setting Index: 0x{:08x}\n  Current DC Power Setting Index: 0x{:08x}\n", a[3], v.0, v.1)), None => (1, "does not exist".into()) }
            }
            "/setacvalueindex" => { let k = (a[1].to_string(), a[3].to_string()); let e = s.vals.entry(k).or_insert((0, 0)); e.0 = a[4].parse().unwrap(); (0, String::new()) }
            "/setdcvalueindex" => { let k = (a[1].to_string(), a[3].to_string()); let e = s.vals.entry(k).or_insert((0, 0)); e.1 = a[4].parse().unwrap(); (0, String::new()) }
            "/restoredefaultschemes" => { s.plans.retain(|p| p.0 == cfg::BALANCED || p.0.starts_with("8c5e7fda")); s.active = cfg::BALANCED.into(); (0, String::new()) }
            _ => (1, "unknown".into()),
        }
    }
}

struct FakeOverlay { avail: bool, cur: RefCell<String>, fail: bool }
impl Overlay for FakeOverlay {
    fn available(&self) -> bool { self.avail }
    fn get(&self) -> Option<String> { Some(self.cur.borrow().clone()) }
    fn set(&self, g: &str) -> Result<(), String> { if self.fail { return Err("refused".into()); } *self.cur.borrow_mut() = g.into(); Ok(()) }
}
fn overlay() -> FakeOverlay { FakeOverlay { avail: true, cur: RefCell::new("00000000-0000-0000-0000-000000000000".into()), fail: false } }

fn hw(ac: Option<bool>, laptop: bool) -> Hardware {
    Hardware { vendor: hw::Vendor::Intel, cpu_name: "Intel(R) Core(TM) i7-14700KF".into(), cores: 20, threads: 28, ram_gb: 64.0, windows: "Windows 11".into(), build: 26100, laptop, on_ac: ac, active_plan: "My plan".into() }
}
fn root(name: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("tdm_pw_{}_{}", std::process::id(), name)); let _ = std::fs::remove_dir_all(&d); d }
fn opts() -> Opts { Opts { usb_off: false, pcie_off: false, ultimate: false, restore_point: false } }

macro_rules! mkenv { ($c:expr, $o:expr, $name:expr, $log:ident) => {{ $log = Log::new(None); Env { cfg: &$c, overlay: &$o, root: root($name), log: &$log, restore_point: &|| Ok(()) } }}; }

#[test]
fn apply_high_makes_a_new_plan_and_leaves_the_users_plan_alone() {
    let c = FakeCfg::new(); let o = overlay(); let log;
    let e = mkenv!(c, o, "high", log);
    let r = apply(&e, Tier::High, opts(), &hw(Some(true), false));
    assert!(r.ok, "{}", r.text());
    let new = c.active_guid();
    assert_ne!(new, USER_PLAN);
    assert_eq!(c.val(&new, profiles::MIN_STATE.guid), (5, 5));
    assert_eq!(c.val(&new, profiles::MAX_STATE.guid), (100, 100));
    assert_eq!(c.val(&new, profiles::COOLING.guid), (1, 1));
    assert_eq!(c.val(USER_PLAN, profiles::MIN_STATE.guid), (10, 10), "the user's plan must not change");
    assert_eq!(o.cur.borrow().as_str(), OVERLAY_BEST);
    let recs = backup::list(&e.root);
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].original_guid, USER_PLAN);
    assert_eq!(recs[0].created_guid.as_deref(), Some(new.as_str()));
    assert!(e.root.join(&recs[0].id).join("plan.pow").exists());
    assert!(!c.calls().iter().any(|x| x.starts_with("/restoredefaultschemes")), "reset must never run automatically");
}

#[test]
fn extreme_is_refused_on_battery_before_anything_is_touched() {
    let c = FakeCfg::new(); let o = overlay(); let log;
    let e = mkenv!(c, o, "bat", log);
    let r = apply(&e, Tier::Extreme, opts(), &hw(Some(false), true));
    assert!(!r.ok && r.failed[0].contains("battery"));
    assert!(c.calls().is_empty());
}

#[test]
fn no_backup_means_no_change() {
    let mut c = FakeCfg::new(); c.fail_export = true; let o = overlay(); let log;
    let e = mkenv!(c, o, "nobk", log);
    let r = apply(&e, Tier::High, opts(), &hw(Some(true), false));
    assert!(!r.ok);
    assert!(!c.calls().iter().any(|x| x.starts_with("/duplicatescheme") || x.starts_with("/setactive")));
    assert_eq!(c.active_guid(), USER_PLAN);
}

#[test]
fn unsupported_settings_are_reported_not_forced() {
    let mut c = FakeCfg::new(); c.unsupported.insert(profiles::PCIE_LINK.guid.into()); let o = overlay(); let log;
    let e = mkenv!(c, o, "unsup", log);
    let r = apply(&e, Tier::High, Opts { pcie_off: true, usb_off: true, ..opts() }, &hw(Some(true), false));
    assert!(r.ok, "{}", r.text());
    assert!(r.unsupported.iter().any(|x| x.contains("PCI Express")));
    assert!(!c.calls().iter().any(|x| x.starts_with("/setacvalueindex") && x.contains(profiles::PCIE_LINK.guid)));
    assert!(r.applied.iter().any(|x| x.contains("USB selective suspend")));
}

#[test]
fn failed_activation_removes_the_new_plan_and_keeps_the_users_plan() {
    let mut c = FakeCfg::new(); c.fail_set_active = true; let o = overlay(); let log;
    let e = mkenv!(c, o, "act", log);
    let r = apply(&e, Tier::Medium, opts(), &hw(Some(true), false));
    assert!(!r.ok);
    assert_eq!(c.active_guid(), USER_PLAN);
    assert_eq!(c.s.borrow().plans.len(), 3, "the plan that was created must be deleted again");
    assert!(r.failed.iter().any(|x| x.contains("Access denied")));
}

#[test]
fn undo_last_goes_back_and_removes_only_the_program_plan() {
    let c = FakeCfg::new(); let o = overlay(); let log;
    let e = mkenv!(c, o, "undo", log);
    assert!(apply(&e, Tier::High, opts(), &hw(Some(true), false)).ok);
    let created = backup::list(&e.root)[0].created_guid.clone().unwrap();
    let r = restore(&e, Which::Last);
    assert!(r.ok, "{}", r.text());
    assert_eq!(c.active_guid(), USER_PLAN);
    assert!(!c.has(&created) && c.has(USER_PLAN));
    assert_eq!(o.cur.borrow().as_str(), "00000000-0000-0000-0000-000000000000");
    assert!(backup::list(&e.root)[0].restored);
    assert!(!restore(&e, Which::Last).ok, "a second undo has nothing left to restore");
}

#[test]
fn restore_reimports_a_plan_that_was_deleted() {
    let c = FakeCfg::new(); let o = overlay(); let log;
    let e = mkenv!(c, o, "reimp", log);
    assert!(apply(&e, Tier::Low, opts(), &hw(Some(true), false)).ok);
    c.s.borrow_mut().plans.retain(|p| p.0 != USER_PLAN);
    let r = restore(&e, Which::Original);
    assert!(r.ok, "{}", r.text());
    assert!(r.applied.iter().any(|x| x.contains("re-imported")));
}

#[test]
fn restore_without_backup_says_so() {
    let c = FakeCfg::new(); let o = overlay(); let log;
    let e = mkenv!(c, o, "none", log);
    let r = restore(&e, Which::Original);
    assert!(!r.ok && r.failed[0].contains("no backup"));
}

#[test]
fn reset_exports_custom_plans_first_and_aborts_if_that_fails() {
    let c = FakeCfg::new(); let o = overlay(); let log;
    let e = mkenv!(c, o, "reset", log);
    let r = reset_defaults(&e);
    assert!(r.ok, "{}", r.text());
    let calls = c.calls();
    let exp = calls.iter().position(|x| x.starts_with("/export")).unwrap();
    let rst = calls.iter().position(|x| x.starts_with("/restoredefaultschemes")).unwrap();
    assert!(exp < rst);
    let mut c2 = FakeCfg::new(); c2.fail_export = true; let log2;
    let e2 = mkenv!(c2, o, "reset2", log2);
    let r2 = reset_defaults(&e2);
    assert!(!r2.ok);
    assert!(!c2.calls().iter().any(|x| x.starts_with("/restoredefaultschemes")), "never delete the user's plans when they could not be saved");
}

#[test]
fn ultimate_comparison_cleans_up_its_temporary_plan() {
    let c = FakeCfg::new(); let o = overlay(); let log;
    let e = mkenv!(c, o, "cmp", log);
    let before = c.s.borrow().plans.len();
    let cmp = compare_ultimate(&e, &hw(Some(true), false));
    assert!(cmp.available && !cmp.rows.is_empty());
    assert_eq!(c.s.borrow().plans.len(), before);
    assert!(cmp.advice.starts_with("Optional"));
    let l = compare_ultimate(&e, &hw(Some(true), true));
    assert!(l.advice.starts_with("Not recommended"));
    let mut c3 = FakeCfg::new(); c3.ultimate_missing = true; let log3;
    let e3 = mkenv!(c3, o, "cmp3", log3);
    assert!(!compare_ultimate(&e3, &hw(Some(true), false)).available);
}

#[test]
fn preview_lists_the_changes_without_touching_anything() {
    let c = FakeCfg::new(); let o = overlay(); let log;
    let e = mkenv!(c, o, "prev", log);
    let rows = preview(&e, Tier::High, Opts { usb_off: true, ..opts() });
    assert!(rows.iter().any(|r| r.label == "Minimum processor state" && r.new.contains("5%")));
    assert!(rows.iter().any(|r| r.label == "USB selective suspend" && r.new.contains("Disabled")));
    assert!(!c.calls().iter().any(|x| x.starts_with("/set") || x.starts_with("/dup") || x.starts_with("/delete")));
}
