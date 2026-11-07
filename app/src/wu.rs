//! Windows Update driver search and installation through the Windows Update Agent (COM).
use crate::catalog::{Date, Offer, OfferKind};
use windows::core::{Interface, BSTR};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::System::UpdateAgent::{IUpdate, IUpdateCollection, IUpdateSession, IWindowsDriverUpdate, UpdateCollection, UpdateSession};

const CRITERIA: &str = "IsInstalled=0 and Type='Driver' and IsHidden=0";

/// COM must be initialised on every thread that uses it; calling twice on one thread is harmless.
pub fn init_com() {
    unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }
}

fn session() -> Result<IUpdateSession, String> {
    init_com();
    unsafe { CoCreateInstance(&UpdateSession, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string()) }
}

fn variant_date(d: f64) -> Option<Date> {
    // OLE automation date: days since 1899-12-30 (= 25569 days before 1970-01-01); civil-from-days by Howard Hinnant
    if d <= 0.0 { return None; }
    let z = d.floor() as i64 - 25_569 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    Some(Date { y: (yoe + era * 400 + if m <= 2 { 1 } else { 0 }) as i32, m: m as u32, d: day as u32 })
}
/// Drivers Windows Update offers for this PC.
pub fn search_drivers() -> Result<Vec<Offer>, String> {
    unsafe {
        let s = session()?;
        let searcher = s.CreateUpdateSearcher().map_err(|e| e.to_string())?;
        let result = searcher.Search(&BSTR::from(CRITERIA)).map_err(|e| e.to_string())?;
        let updates = result.Updates().map_err(|e| e.to_string())?;
        let n = updates.Count().map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for i in 0..n {
            let u: IUpdate = updates.get_Item(i).map_err(|e| e.to_string())?;
            let id = u.Identity().map_err(|e| e.to_string())?.UpdateID().map_err(|e| e.to_string())?.to_string();
            let title = u.Title().map(|t| t.to_string()).unwrap_or_default();
            let (mut class, mut maker, mut date) = (String::new(), String::new(), String::new());
            if let Ok(w) = u.cast::<IWindowsDriverUpdate>() {
                class = w.DriverClass().map(|b| b.to_string()).unwrap_or_default();
                maker = w.DriverManufacturer().map(|b| b.to_string()).unwrap_or_default();
                date = w.DriverVerDate().ok().and_then(variant_date).map(|d| d.iso()).unwrap_or_default();
            }
            let size = u.MaxDownloadSize().ok().map(|dec| {
                // DECIMAL: 96-bit integer in Lo64/Hi32 with a scale in the sign/scale word; sizes here fit in Lo64
                (unsafe { dec.Anonymous2.Lo64 }) as f64 / 1_048_576.0
            });
            out.push(Offer {
                id: id.clone(), title, class, maker, date, size_mb: size.map(|x| (x * 10.0).round() / 10.0), source: "Windows Update".into(),
                instance_id: String::new(), device_name: String::new(), kind: OfferKind::WindowsUpdate { update_id: id },
            });
        }
        Ok(out)
    }
}

#[derive(Debug, Clone)]
pub struct WuOutcome { pub id: String, pub success: bool, pub reason: String }

pub struct WuInstall { pub outcomes: Vec<WuOutcome>, pub reboot: bool, pub log: Vec<String> }

/// Downloads and installs the chosen driver updates (by update id). Items that are no longer offered are reported as such.
pub fn install(ids: &[String], status: &dyn Fn(&str)) -> Result<WuInstall, String> {
    unsafe {
        let s = session()?;
        let searcher = s.CreateUpdateSearcher().map_err(|e| e.to_string())?;
        let found = searcher.Search(&BSTR::from(CRITERIA)).map_err(|e| e.to_string())?.Updates().map_err(|e| e.to_string())?;
        let coll: IUpdateCollection = CoCreateInstance(&UpdateCollection, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        let mut res = WuInstall { outcomes: vec![], reboot: false, log: vec![] };
        let mut chosen: Vec<(String, IUpdate)> = Vec::new();
        for i in 0..found.Count().map_err(|e| e.to_string())? {
            let u: IUpdate = found.get_Item(i).map_err(|e| e.to_string())?;
            let id = u.Identity().and_then(|x| x.UpdateID()).map(|b| b.to_string()).unwrap_or_default();
            if ids.contains(&id) {
                if u.EulaAccepted().map(|b| !b.as_bool()).unwrap_or(false) { let _ = u.AcceptEula(); }
                coll.Add(&u).map_err(|e| e.to_string())?;
                chosen.push((id, u));
            }
        }
        for id in ids { if !chosen.iter().any(|(c, _)| c == id) { res.outcomes.push(WuOutcome { id: id.clone(), success: false, reason: "Not found (may already be installed)".into() }); } }
        if chosen.is_empty() { res.log.push("The selected Windows Update items were not found (they may already be installed)".into()); return Ok(res); }
        status(&format!("Downloading {} items from Windows Update...", chosen.len()));
        let dl = s.CreateUpdateDownloader().map_err(|e| e.to_string())?;
        dl.SetUpdates(&coll).map_err(|e| e.to_string())?;
        let _ = dl.Download().map_err(|e| e.to_string())?;
        let to_install: IUpdateCollection = CoCreateInstance(&UpdateCollection, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        let mut order: Vec<String> = Vec::new();
        for (id, u) in &chosen {
            if u.IsDownloaded().map(|b| b.as_bool()).unwrap_or(false) { to_install.Add(u).map_err(|e| e.to_string())?; order.push(id.clone()); }
            else { res.outcomes.push(WuOutcome { id: id.clone(), success: false, reason: "Download failed".into() }); }
        }
        if order.is_empty() { return Ok(res); }
        status(&format!("Installing {} items from Windows Update...", order.len()));
        let inst = s.CreateUpdateInstaller().map_err(|e| e.to_string())?;
        inst.SetUpdates(&to_install).map_err(|e| e.to_string())?;
        let r = inst.Install().map_err(|e| e.to_string())?;
        res.reboot = r.RebootRequired().map(|b| b.as_bool()).unwrap_or(false);
        for (i, id) in order.iter().enumerate() {
            let code = r.GetUpdateResult(i as i32).and_then(|x| x.ResultCode()).map(|c| c.0).unwrap_or(-1);
            let text = match code { 2 => "succeeded".to_string(), 3 => "partly succeeded".to_string(), 4 => "failed".to_string(), 5 => "cancelled".to_string(), c => format!("code {c}") };
            res.outcomes.push(WuOutcome { id: id.clone(), success: code == 2, reason: if code == 2 { String::new() } else { text.clone() } });
            res.log.push(format!("{} {id} ({text})", if code == 2 { "[OK]" } else { "[FAILED]" }));
        }
        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ole_dates_convert() {
        // 2026-11-04 is OLE day 46330 (44927 = 2023-01-01; +1038 days)
        assert_eq!(variant_date(46330.0), Some(Date { y: 2026, m: 11, d: 4 }));
        assert_eq!(variant_date(25569.0), Some(Date { y: 1970, m: 1, d: 1 }));
        assert_eq!(variant_date(0.0), None);
    }
}

