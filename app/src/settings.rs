//! Program settings (saved as settings.json next to the catalog) and the "start with Windows" scheduled task.
//! Start-up uses a scheduled task, not the Run registry key: the program needs administrator rights, and Windows skips
//! such programs in the Run key (or asks for permission every time). A task with the highest run level starts silently.
use crate::memory;
use crate::sys::Sys;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const TASK: &str = "TierS Driver Manage";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// closing the window hides it to the tray instead of quitting
    pub tray_on_close: bool,
    pub mem_auto: bool,
    pub mem_poll_ms: u64,
    pub mem_standby_mb: u64,
    pub mem_free_mb: u64,
    /// games with their own settings, and the "faster power plan while a game runs" switch
    pub games: Vec<crate::game::Game>,
    pub game_auto_plan: bool,
    pub game_plan: String,
}

impl Default for Settings {
    fn default() -> Settings { Settings { tray_on_close: false, mem_auto: false, mem_poll_ms: 1000, mem_standby_mb: 1024, mem_free_mb: 1024, games: vec![], game_auto_plan: false, game_plan: String::new() } }
}

impl Settings {
    /// A missing or damaged file gives the defaults; values are snapped into their allowed ranges.
    pub fn load(path: &Path) -> Settings {
        let mut s: Settings = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        s.mem_poll_ms = memory::snap(s.mem_poll_ms, memory::POLL_MS);
        s.mem_standby_mb = memory::snap(s.mem_standby_mb, memory::STANDBY_MB);
        s.mem_free_mb = memory::snap(s.mem_free_mb, memory::FREE_MB);
        s.games.retain(|g| crate::game::valid_exe(&g.exe)); // a hand-edited file must not create odd registry keys
        s
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        // write a temp file first so a crash can never leave half a settings file
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }
}

fn xml_escape(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;") }

/// Task definition: at sign-in, as this user, highest run level, no battery or run-time limits, opens in the tray.
pub fn task_xml(exe: &str, user: &str) -> String {
    let (exe, user) = (xml_escape(exe), xml_escape(user));
    format!(r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>Starts TierS Driver Manage in the system tray</Description></RegistrationInfo>
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId><Delay>PT15S</Delay></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals>
  <Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><AllowStartOnDemand>true</AllowStartOnDemand><Enabled>true</Enabled></Settings>
  <Actions Context="Author"><Exec><Command>"{exe}"</Command><Arguments>--tray</Arguments></Exec></Actions>
</Task>
"#)
}

pub fn autostart_enabled(sys: &dyn Sys) -> bool { sys.run("schtasks.exe", &["/Query", "/TN", TASK]).0 == 0 }

fn friendly(code: i32, out: &str) -> String {
    if out.to_lowercase().contains("denied") { "Access denied: run the program as Administrator".into() } else { format!("Windows could not change the start-up task (code {code}): {}", out.trim()) }
}

/// Creates or removes the start-up task. `work` is a folder to write the temporary task file in.
pub fn set_autostart(sys: &dyn Sys, on: bool, exe: &Path, user: &str, work: &Path) -> Result<(), String> {
    if !on {
        if !autostart_enabled(sys) { return Ok(()); }
        let (code, out) = sys.run("schtasks.exe", &["/Delete", "/TN", TASK, "/F"]);
        return if code == 0 { Ok(()) } else { Err(friendly(code, &out)) };
    }
    let xml = work.join("autostart-task.xml");
    // schtasks wants UTF-16 with a byte-order mark
    let mut bytes = vec![0xFF, 0xFE];
    for u in task_xml(&exe.to_string_lossy(), user).encode_utf16() { bytes.extend_from_slice(&u.to_le_bytes()); }
    std::fs::write(&xml, bytes).map_err(|e| e.to_string())?;
    let (code, out) = sys.run("schtasks.exe", &["/Create", "/TN", TASK, "/XML", &xml.to_string_lossy(), "/F"]);
    let _ = std::fs::remove_file(&xml);
    if code == 0 { Ok(()) } else { Err(friendly(code, &out)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::fake::FakeSys;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("tdm_settings_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn missing_or_broken_file_gives_defaults() {
        let d = tmp("broken");
        assert_eq!(Settings::load(&d.join("none.json")), Settings::default());
        std::fs::write(d.join("bad.json"), "{ not json").unwrap();
        assert_eq!(Settings::load(&d.join("bad.json")), Settings::default());
    }

    #[test]
    fn saved_settings_come_back_and_values_are_snapped() {
        let d = tmp("round");
        let p = d.join("settings.json");
        let s = Settings { tray_on_close: true, mem_auto: true, mem_poll_ms: 2000, mem_standby_mb: 2048, mem_free_mb: 512, games: vec![crate::game::Game { exe: "cs2.exe".into(), path: "C:\\g\\cs2.exe".into(), fso: true, prio: false }], game_auto_plan: true, game_plan: "abc".into() };
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p), s);
        assert!(!d.join("settings.json.tmp").exists());
        std::fs::write(&p, r#"{"mem_standby_mb": 1, "mem_poll_ms": 99999999}"#).unwrap();
        let l = Settings::load(&p);
        assert_eq!((l.mem_standby_mb, l.mem_poll_ms), (256, 10_000));
        assert!(!l.tray_on_close, "fields missing from the file use their defaults");
    }

    #[test]
    fn task_xml_is_escaped_and_runs_in_the_tray() {
        let x = task_xml(r"C:\Program Files\Tiers & Co\app.exe", r"PC\Ann <x>");
        assert!(x.contains("Tiers &amp; Co") && x.contains("PC\\Ann &lt;x&gt;"));
        assert!(x.contains("<Arguments>--tray</Arguments>") && x.contains("HighestAvailable") && x.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(x.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
    }

    #[test]
    fn enabling_creates_the_task_from_a_utf16_file_and_cleans_up() {
        let d = tmp("on");
        let sys = FakeSys::ok();
        set_autostart(&sys, true, Path::new(r"C:\app\x.exe"), "PC\\Ann", &d).unwrap();
        let calls = sys.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("schtasks.exe /Create /TN TierS Driver Manage /XML ") && calls[0].ends_with(" /F"));
        assert!(!d.join("autostart-task.xml").exists(), "the temporary task file is removed");
    }

    #[test]
    fn disabling_deletes_only_when_the_task_exists() {
        let d = tmp("off");
        let sys = FakeSys::ok(); // every call "succeeds", so the task counts as present
        set_autostart(&sys, false, Path::new("x.exe"), "u", &d).unwrap();
        assert_eq!(sys.calls(), vec!["schtasks.exe /Query /TN TierS Driver Manage", "schtasks.exe /Delete /TN TierS Driver Manage /F"]);
        let mut none = FakeSys::ok();
        none.exit_code = 1; // the query fails: no task
        assert!(set_autostart(&none, false, Path::new("x.exe"), "u", &d).is_ok());
        assert_eq!(none.calls().len(), 1, "nothing to delete");
    }

    #[test]
    fn failures_are_reported_in_plain_words() {
        let d = tmp("fail");
        let mut sys = FakeSys::ok();
        sys.exit_code = 1;
        let e = set_autostart(&sys, true, Path::new("x.exe"), "u", &d).unwrap_err();
        assert!(e.contains("code 1"));
        assert!(friendly(5, "ERROR: Access is denied.").contains("Administrator"));
    }
}
