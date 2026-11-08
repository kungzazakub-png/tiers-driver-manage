//! Power Plan Manager. The user's own plans are never edited: a profile is written into a NEW copy of Balanced
//! (or Ultimate Performance when chosen), activated, and every step is verified by reading the value back.
//! Before anything changes the active plan is exported and its values are recorded, so Restore can put it back.
pub mod backup;
pub mod cfg;
pub mod hw;
pub mod log;
pub mod overlay;
pub mod profiles;

use backup::{Record, ValueRec};
use cfg::Cfg;
use hw::{Hardware, Tier};
use log::Log;
use overlay::Overlay;
use profiles::{describe, Opts, Setting, OVERLAY_BEST};
use serde::Serialize;
use std::path::PathBuf;

pub struct Env<'a> {
    pub cfg: &'a dyn Cfg,
    pub overlay: &'a dyn Overlay,
    pub root: PathBuf,
    pub log: &'a Log,
    pub restore_point: &'a dyn Fn() -> Result<(), String>,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct Line { pub label: String, pub value: String }

#[derive(Serialize, Clone, Default, Debug)]
pub struct Report {
    pub title: String,
    pub ok: bool,
    pub previous: Vec<Line>,
    pub new: Vec<Line>,
    pub applied: Vec<String>,
    pub unsupported: Vec<String>,
    pub failed: Vec<String>,
    pub restore: String,
}

impl Report {
    fn fail(title: &str, why: String) -> Report { Report { title: title.into(), ok: false, failed: vec![why], ..Default::default() } }
    pub fn text(&self) -> String {
        let sect = |name: &str, v: &[String]| if v.is_empty() { format!("{name}: none\n") } else { format!("{name}:\n{}\n", v.iter().map(|x| format!("  - {x}")).collect::<Vec<_>>().join("\n")) };
        let lines = |name: &str, v: &[Line]| format!("{name}:\n{}\n", v.iter().map(|l| format!("  {}: {}", l.label, l.value)).collect::<Vec<_>>().join("\n"));
        format!("{} - {}\n\n{}\n{}\n{}{}{}Restore status: {}\n", self.title, if self.ok { "SUCCESS" } else { "FAILED" }, lines("Previous configuration", &self.previous), lines("New configuration", &self.new),
                sect("Successfully applied", &self.applied), sect("Unsupported settings", &self.unsupported), sect("Failed settings", &self.failed), self.restore)
    }
}

fn is_builtin(guid: &str) -> bool { matches!(guid, "381b4222-f694-41f0-9685-ff5bb260df2e" | "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c" | "a1841308-3541-4fab-bc81-f71556f20b4a" | "e9a42b02-d5df-448d-aa00-03f14749eb61") }

const MAIN: [&Setting; 5] = [&profiles::MIN_STATE, &profiles::MAX_STATE, &profiles::COOLING, &profiles::USB_SUSPEND, &profiles::PCIE_LINK];

pub fn read_values(c: &dyn Cfg, scheme: &str) -> Vec<ValueRec> {
    MAIN.iter().map(|s| { let q = cfg::query(c, scheme, s.sub, s.guid); ValueRec { label: s.label.into(), sub: s.sub.into(), guid: s.guid.into(), ac: q.map(|x| x.0), dc: q.map(|x| x.1) } }).collect()
}

fn setting_of(guid: &str) -> Option<&'static Setting> { MAIN.iter().copied().find(|s| s.guid == guid) }

/// "5%" when plugged in and battery agree, otherwise "10% AC · 5% DC"; one-sided changes say which side.
pub fn pair_text(s: &Setting, ac: Option<u32>, dc: Option<u32>) -> String {
    match (ac, dc) {
        (Some(a), Some(d)) if a == d => describe(s, a),
        (Some(a), Some(d)) => format!("{} AC · {} DC", describe(s, a), describe(s, d)),
        (Some(a), None) => format!("{} (AC only)", describe(s, a)),
        (None, Some(d)) => format!("{} (DC only)", describe(s, d)),
        _ => "not supported on this PC".into(),
    }
}

pub fn show_value(v: &ValueRec) -> String { match setting_of(&v.guid) { Some(s) => pair_text(s, v.ac, v.dc), None => "unknown".into() } }

fn overlay_line(o: &dyn Overlay) -> String {
    if !o.available() { return "not available on this Windows version".into(); }
    o.get().map(|g| overlay::overlay_name(&g).to_string()).unwrap_or_else(|| "unknown".into())
}

/// Exports the active plan and records its values. Nothing is changed. Fails (and the caller must stop) if the backup cannot be made.
pub fn backup_now(env: &Env, kind: &str, profile: &str) -> Result<Record, String> {
    let act = cfg::active(env.cfg)?;
    let now = log::stamp();
    let id = backup::new_id(&env.root, &now);
    let dir = backup::folder(&env.root, &id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create the backup folder: {e}"))?;
    let file = dir.join("plan.pow");
    cfg::export(env.cfg, &act.guid, &file.to_string_lossy()).map_err(|e| format!("Could not export the current power plan: {e}"))?;
    if !file.exists() { return Err("The exported power plan file was not created".into()); }
    let rec = Record {
        id, time: now, kind: kind.into(), profile: profile.into(), original_guid: act.guid.clone(), original_name: act.name.clone(), export: "plan.pow".into(),
        overlay_before: if env.overlay.available() { env.overlay.get() } else { None }, created_guid: None, values: read_values(env.cfg, &act.guid), restore_point: "not requested".into(), restored: false,
    };
    backup::save(&env.root, &rec)?;
    env.log.line(&format!("Backup {} saved: plan '{}' ({}), {} values recorded", rec.id, rec.original_name, rec.original_guid, rec.values.len()));
    Ok(rec)
}

pub fn apply(env: &Env, tier: Tier, opts: Opts, hw: &Hardware) -> Report {
    let title = "Apply profile";
    let prof = profiles::profile(tier);
    env.log.line(&format!("Apply profile {} requested (usb_off={}, pcie_off={}, ultimate={})", tier.name(), opts.usb_off, opts.pcie_off, opts.ultimate));
    if tier == Tier::Extreme && hw.on_ac == Some(false) { let r = Report::fail(title, "The Extreme profile is not applied while the PC runs on battery. Plug in the charger first.".into()); env.log.line(&r.failed[0]); return r; }
    let mut rep = Report { title: format!("{title}: {}", prof.title), ..Default::default() };
    // 1. backup first: no backup, no change
    let mut rec = match backup_now(env, "apply", tier.name()) { Ok(r) => r, Err(e) => { env.log.line(&format!("FAILED: {e}")); return Report::fail(title, format!("{e}. Nothing was changed.")); } };
    rep.previous.push(Line { label: "Active power plan".into(), value: format!("{} ({})", rec.original_name, rec.original_guid) });
    for v in &rec.values { rep.previous.push(Line { label: v.label.clone(), value: show_value(v) }); }
    rep.previous.push(Line { label: "Windows power mode".into(), value: rec.overlay_before.as_deref().map(|g| overlay::overlay_name(g).to_string()).unwrap_or_else(|| "not available".into()) });
    // 2. restore point (optional)
    if opts.restore_point {
        match (env.restore_point)() { Ok(()) => rec.restore_point = "created".into(), Err(e) => rec.restore_point = format!("not created: {e}") }
        env.log.line(&format!("Restore point: {}", rec.restore_point));
    }
    // 3. new plan: a copy, the user's plan stays untouched
    let want_ultimate = opts.ultimate && prof.ultimate_optional;
    let mut base = if want_ultimate { cfg::ULTIMATE } else { cfg::BALANCED }.to_string();
    let new = match cfg::duplicate(env.cfg, &base) {
        Ok(g) => g,
        Err(e) => {
            env.log.line(&format!("Base plan {base} unavailable: {e}"));
            if want_ultimate { rep.unsupported.push(format!("Ultimate Performance plan is not available on this PC ({e}); Balanced is used instead")); } else { rep.unsupported.push(format!("The built-in Balanced plan is missing ({e}); a copy of your current plan is used instead")); }
            base = if want_ultimate { cfg::BALANCED.to_string() } else { rec.original_guid.clone() };
            match cfg::duplicate(env.cfg, &base).or_else(|_| cfg::duplicate(env.cfg, &rec.original_guid)) {
                Ok(g) => g,
                Err(e2) => { rep.failed.push(format!("Could not create a new power plan: {e2}")); env.log.line(&format!("FAILED: {e2}")); rep.restore = "Nothing was changed".into(); return rep; }
            }
        }
    };
    rec.created_guid = Some(new.clone());
    let _ = backup::save(&env.root, &rec);
    let name = format!("TierS Gaming - {}", prof.title);
    if let Err(e) = cfg::rename(env.cfg, &new, &name, &format!("Created by TierS Driver Manage ({})", prof.goal)) { env.log.line(&format!("Rename failed (harmless): {e}")); }
    env.log.line(&format!("Created plan {new} from {base}"));
    // 4. settings
    let mut expect: Vec<(&Setting, Option<u32>, Option<u32>)> = vec![];
    for ch in profiles::changes(prof, opts) {
        let s = ch.setting;
        if cfg::query(env.cfg, &new, s.sub, s.guid).is_none() { rep.unsupported.push(format!("{}: not supported on this PC, left unchanged", s.label)); env.log.line(&format!("Unsupported: {}", s.label)); continue; }
        let mut ok = true;
        if let Some(v) = ch.ac { if let Err(e) = cfg::set_ac(env.cfg, &new, s.sub, s.guid, v) { rep.failed.push(format!("{} (plugged in): {e}", s.label)); env.log.line(&format!("FAILED {} AC: {e}", s.label)); ok = false; } }
        if let Some(v) = ch.dc { if let Err(e) = cfg::set_dc(env.cfg, &new, s.sub, s.guid, v) { rep.failed.push(format!("{} (battery): {e}", s.label)); env.log.line(&format!("FAILED {} DC: {e}", s.label)); ok = false; } }
        if ok { expect.push((s, ch.ac, ch.dc)); }
    }
    // 5. activate; if that fails remove the plan we made and leave the user's plan active
    if let Err(e) = cfg::set_active(env.cfg, &new) {
        let _ = cfg::delete(env.cfg, &new);
        rep.failed.push(format!("Could not activate the new plan: {e}"));
        rep.restore = format!("Your plan '{}' is still active; nothing was changed", rec.original_name);
        env.log.line(&format!("FAILED activate: {e}"));
        rec.created_guid = None; rec.restored = true; let _ = backup::save(&env.root, &rec);
        return rep;
    }
    env.log.line(&format!("Activated {new}"));
    // 6. Windows power mode
    if prof.power_mode_best {
        if !env.overlay.available() { rep.unsupported.push("Windows power mode (Best performance): not available on this Windows version".into()); }
        else {
            match env.overlay.set(OVERLAY_BEST) {
                Ok(()) if env.overlay.get().as_deref() == Some(OVERLAY_BEST) => { rep.applied.push("Windows power mode: Best performance".into()); env.log.line("Power mode set to Best performance"); }
                Ok(()) => rep.failed.push("Windows power mode did not change after the request".into()),
                Err(e) => { rep.failed.push(format!("Windows power mode: {e}")); env.log.line(&format!("FAILED power mode: {e}")); }
            }
        }
    }
    // 7. verify by reading everything back
    match cfg::active(env.cfg) { Ok(a) if a.guid == new => rep.applied.push(format!("Active power plan: {} ({})", a.name, a.guid)), Ok(a) => rep.failed.push(format!("Active plan is {} instead of the new plan", a.guid)), Err(e) => rep.failed.push(format!("Could not verify the active plan: {e}")) }
    for (s, ac, dc) in expect {
        let got = cfg::query(env.cfg, &new, s.sub, s.guid);
        let good = got.map(|(a, d)| ac.map_or(true, |x| x == a) && dc.map_or(true, |x| x == d)).unwrap_or(false);
        let want = pair_text(s, ac, dc);
        if good { rep.applied.push(format!("{}: {want}", s.label)); } else { rep.failed.push(format!("{}: expected {want}, Windows reports {}", s.label, got.map(|(a, d)| format!("{a}/{d}")).unwrap_or_else(|| "nothing".into()))); }
    }
    for v in read_values(env.cfg, &new) { rep.new.push(Line { label: v.label.clone(), value: show_value(&v) }); }
    rep.new.insert(0, Line { label: "Active power plan".into(), value: format!("{name} ({new})") });
    rep.new.push(Line { label: "Windows power mode".into(), value: overlay_line(env.overlay) });
    rep.ok = rep.failed.is_empty();
    rep.restore = format!("Backup {} is ready: use Undo Last Optimization to go back to '{}'", rec.id, rec.original_name);
    let _ = backup::save(&env.root, &rec);
    let _ = std::fs::write(backup::folder(&env.root, &rec.id).join("report.txt"), rep.text());
    env.log.line(&format!("Apply finished: {} applied, {} unsupported, {} failed", rep.applied.len(), rep.unsupported.len(), rep.failed.len()));
    rep
}

/// "Backup Settings" button: export and record only.
pub fn backup_only(env: &Env) -> Report {
    match backup_now(env, "manual", "-") {
        Ok(r) => Report { title: "Backup settings".into(), ok: true, previous: std::iter::once(Line { label: "Active power plan".into(), value: format!("{} ({})", r.original_name, r.original_guid) }).chain(r.values.iter().map(|v| Line { label: v.label.clone(), value: show_value(v) })).collect(),
                          applied: vec![format!("Saved to {}", backup::folder(&env.root, &r.id).display())], restore: "Use Restore Original Settings to return to this state".into(), ..Report { ok: true, ..Default::default() } },
        Err(e) => { env.log.line(&format!("Backup failed: {e}")); Report::fail("Backup settings", e) }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Which { Original, Last }

fn restore_one(env: &Env, rec: &Record, rep: &mut Report) -> bool {
    let plans = match cfg::list(env.cfg) { Ok(p) => p, Err(e) => { rep.failed.push(e); return false } };
    let target = if plans.iter().any(|p| p.guid == rec.original_guid) { Some(rec.original_guid.clone()) } else {
        let file = backup::folder(&env.root, &rec.id).join(&rec.export);
        match cfg::import(env.cfg, &file.to_string_lossy()) {
            Ok(g) => { rep.applied.push(format!("Plan '{}' was missing and was re-imported from the backup", rec.original_name)); Some(g) }
            Err(e) => { rep.failed.push(format!("Plan '{}' no longer exists and could not be imported: {e}", rec.original_name)); None }
        }
    };
    let Some(t) = target else { return false };
    if let Err(e) = cfg::set_active(env.cfg, &t) { rep.failed.push(format!("Could not re-activate '{}': {e}", rec.original_name)); return false; }
    match cfg::active(env.cfg) { Ok(a) if a.guid == t => rep.applied.push(format!("Active power plan restored: {} ({})", a.name, a.guid)), _ => { rep.failed.push("The restored plan is not active".into()); return false } }
    if let Some(o) = &rec.overlay_before {
        if env.overlay.available() {
            match env.overlay.set(o) { Ok(()) => rep.applied.push(format!("Windows power mode restored: {}", overlay::overlay_name(o))), Err(e) => rep.failed.push(format!("Windows power mode: {e}")) }
        }
    }
    true
}

fn delete_created(env: &Env, rec: &Record, rep: &mut Report) {
    let Some(g) = &rec.created_guid else { return };
    let exists = cfg::list(env.cfg).map(|l| l.iter().any(|p| &p.guid == g)).unwrap_or(false);
    if !exists { return; }
    match cfg::delete(env.cfg, g) { Ok(()) => rep.applied.push(format!("Removed the plan this program created ({g}); your own plans were not touched")), Err(e) => rep.unsupported.push(format!("Could not remove the program's plan {g}: {e}")) }
}

pub fn restore(env: &Env, which: Which) -> Report {
    let title = if which == Which::Original { "Restore original settings" } else { "Undo last optimization" };
    let all: Vec<Record> = backup::list(&env.root).into_iter().filter(|r| !r.restored).collect();
    let pick = if which == Which::Original { all.first().cloned() } else { all.iter().rev().find(|r| r.kind == "apply").cloned() };
    let Some(rec) = pick else { let r = Report::fail(title, "There is nothing to restore: no backup was found.".into()); env.log.line(&r.failed[0]); return r };
    env.log.line(&format!("{title}: using backup {}", rec.id));
    let mut rep = Report { title: title.into(), ..Default::default() };
    rep.previous.push(Line { label: "Restoring backup".into(), value: format!("{} ({})", rec.id, rec.time) });
    if !restore_one(env, &rec, &mut rep) { rep.restore = "Restore failed: your current settings were left as they are".into(); env.log.line(&rep.restore); return rep; }
    // plans created by the program from this point on go away; every record from here on is now history
    let from: Vec<Record> = all.iter().filter(|r| r.id >= rec.id && (which == Which::Original || r.id == rec.id)).cloned().collect();
    for r in &from { delete_created(env, r, &mut rep); let mut r2 = r.clone(); r2.restored = true; let _ = backup::save(&env.root, &r2); }
    for v in read_values(env.cfg, &rec.original_guid) { rep.new.push(Line { label: v.label.clone(), value: show_value(&v) }); }
    rep.new.insert(0, Line { label: "Active power plan".into(), value: format!("{} ({})", rec.original_name, rec.original_guid) });
    rep.ok = rep.failed.is_empty();
    rep.restore = if rep.ok { "Restore completed".into() } else { "Restore completed with problems".into() };
    env.log.line(&rep.restore);
    rep
}

/// "Reset to Windows Default": only on an explicit click. Custom plans are exported first, and nothing is deleted if an export fails.
pub fn reset_defaults(env: &Env) -> Report {
    let title = "Reset to Windows default";
    let plans = match cfg::list(env.cfg) { Ok(p) => p, Err(e) => return Report::fail(title, e) };
    let custom: Vec<_> = plans.iter().filter(|p| !is_builtin(&p.guid)).collect();
    let dir = env.root.join(format!("reset-{}", backup::new_id(&env.root, &log::stamp())));
    let mut rep = Report { title: title.into(), ..Default::default() };
    if !custom.is_empty() {
        if let Err(e) = std::fs::create_dir_all(&dir) { return Report::fail(title, format!("Cannot create the backup folder: {e}")); }
        for p in &custom {
            let file = dir.join(format!("{}.pow", p.guid));
            if let Err(e) = cfg::export(env.cfg, &p.guid, &file.to_string_lossy()) { env.log.line(&format!("Reset aborted: export of '{}' failed: {e}", p.name)); return Report::fail(title, format!("Could not back up your plan '{}' ({e}). Nothing was reset.", p.name)); }
            rep.previous.push(Line { label: format!("Custom plan saved: {}", p.name), value: file.display().to_string() });
        }
    }
    if let Some(a) = plans.iter().find(|p| p.active) { rep.previous.push(Line { label: "Active power plan".into(), value: format!("{} ({})", a.name, a.guid) }); }
    if let Err(e) = cfg::restore_defaults(env.cfg) { env.log.line(&format!("FAILED reset: {e}")); rep.failed.push(e); rep.restore = "Reset failed".into(); return rep; }
    match cfg::active(env.cfg) { Ok(a) => { rep.applied.push(format!("Windows default plans restored; active plan: {} ({})", a.name, a.guid)); rep.new.push(Line { label: "Active power plan".into(), value: format!("{} ({})", a.name, a.guid) }); } Err(e) => rep.failed.push(format!("Could not verify: {e}")) }
    // old records no longer describe anything that exists
    for r in backup::list(&env.root) { if !r.restored { let mut r2 = r; r2.restored = true; let _ = backup::save(&env.root, &r2); } }
    rep.restore = if custom.is_empty() { "Reset completed".into() } else { format!("Reset completed; your {} custom plan(s) were exported to {} and can be re-imported with powercfg /import", custom.len(), dir.display()) };
    rep.ok = rep.failed.is_empty();
    env.log.line(&rep.restore);
    rep
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct CmpRow { pub label: String, pub balanced: String, pub ultimate: String, pub differs: bool }
#[derive(Serialize, Clone, Default, Debug)]
pub struct Comparison { pub available: bool, pub rows: Vec<CmpRow>, pub advice: String }

/// Balanced vs Ultimate Performance, read from Windows itself (a temporary copy of Ultimate is made and removed again).
pub fn compare_ultimate(env: &Env, hw: &Hardware) -> Comparison {
    let list = [&profiles::MIN_STATE, &profiles::MAX_STATE, &profiles::COOLING, &profiles::USB_SUSPEND, &profiles::PCIE_LINK, &profiles::BOOST_MODE, &profiles::PARKING_MIN, &profiles::IDLE_DISABLE];
    let tmp = match cfg::duplicate(env.cfg, cfg::ULTIMATE) { Ok(g) => g, Err(e) => { env.log.line(&format!("Ultimate comparison: {e}")); return Comparison { available: false, rows: vec![], advice: format!("The Ultimate Performance plan is not available on this PC ({e}). Balanced is the right choice.") } } };
    let mut rows = vec![];
    for s in list {
        let b = cfg::query(env.cfg, cfg::BALANCED, s.sub, s.guid).map(|x| x.0);
        let u = cfg::query(env.cfg, &tmp, s.sub, s.guid).map(|x| x.0);
        let f = |v: Option<u32>| v.map(|x| describe(s, x)).unwrap_or_else(|| "n/a".into());
        rows.push(CmpRow { label: s.label.into(), balanced: f(b), ultimate: f(u), differs: b != u });
    }
    let _ = cfg::delete(env.cfg, &tmp);
    let (tier, _) = hw::score(hw);
    let advice = if tier == Tier::Extreme && !hw.laptop && hw.on_ac != Some(false) {
        "Optional on this PC. Ultimate removes most power-saving steps, so the CPU reacts a little faster but runs warmer and uses more power at idle. Try Balanced + Best performance first and choose Ultimate only if your cooling is good and you still see stutter.".to_string()
    } else {
        "Not recommended on this PC (laptop, battery or lower performance tier). Balanced with Best performance gives nearly the same gaming performance with less heat and power.".to_string()
    };
    env.log.line("Ultimate vs Balanced comparison read");
    Comparison { available: true, rows, advice }
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct PreviewRow { pub label: String, pub current: String, pub new: String, pub note: String }

/// What Apply would change, shown before the user clicks it.
pub fn preview(env: &Env, tier: Tier, opts: Opts) -> Vec<PreviewRow> {
    let prof = profiles::profile(tier);
    let act = cfg::active(env.cfg).ok();
    let mut v = vec![PreviewRow { label: "Power plan".into(), current: act.as_ref().map(|a| a.name.clone()).unwrap_or_else(|| "unknown".into()),
        new: format!("New plan \"TierS Gaming - {}\" copied from {}", prof.title, if opts.ultimate && prof.ultimate_optional { "Ultimate Performance" } else { "Balanced" }), note: "Your current plan is kept, not edited".into() }];
    let cur = act.map(|a| read_values(env.cfg, &a.guid)).unwrap_or_default();
    for ch in profiles::changes(prof, opts) {
        let s = ch.setting;
        let c = cur.iter().find(|x| x.guid == s.guid);
        let supported = c.map(|x| x.ac.is_some()).unwrap_or(false);
        let new = pair_text(s, ch.ac, ch.dc);
        v.push(PreviewRow { label: s.label.into(), current: c.map(show_value).unwrap_or_else(|| "unknown".into()), new, note: if supported { "Will be changed".into() } else { "Not supported on this PC: skipped".into() } });
    }
    for (name, on, opt) in [("USB selective suspend", opts.usb_off, prof.usb_optional), ("PCI Express link state", opts.pcie_off, prof.pcie_optional)] {
        if !(on && opt) { v.push(PreviewRow { label: name.into(), current: "-".into(), new: "Keep default".into(), note: if opt { "Optional: not selected".into() } else { "Not part of this profile".into() } }); }
    }
    v.push(PreviewRow { label: "Windows power mode".into(), current: overlay_line(env.overlay), new: if prof.power_mode_best { "Best performance".into() } else { "Keep default".into() },
        note: if prof.power_mode_best && !env.overlay.available() { "Not available on this Windows version: skipped".into() } else if prof.power_mode_best { "Will be changed".into() } else { "Not part of this profile".into() } });
    v
}

pub fn recommended_note(h: &Hardware) -> (Tier, Vec<String>) { hw::score(h) }

#[cfg(test)]
mod tests;
