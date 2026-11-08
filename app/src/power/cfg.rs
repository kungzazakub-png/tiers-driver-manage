//! powercfg.exe wrapper. Every call checks the exit code; nothing is assumed to have worked.
//! Output text is localized, so values are read by position/GUID, never by label.
use crate::sys::Sys;
use regex::Regex;

pub trait Cfg {
    /// Runs `powercfg.exe <args>`; returns (exit code, combined output).
    fn powercfg(&self, args: &[&str]) -> (i32, String);
}
impl<T: Sys + ?Sized> Cfg for T {
    fn powercfg(&self, args: &[&str]) -> (i32, String) { self.run("powercfg.exe", args) }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scheme { pub guid: String, pub name: String, pub active: bool }

pub const BALANCED: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";
pub const ULTIMATE: &str = "e9a42b02-d5df-448d-aa00-03f14749eb61";

fn guid_re() -> Regex { Regex::new(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}").unwrap() }

fn friendly(code: i32, out: &str) -> String {
    let l = out.to_lowercase();
    if l.contains("denied") || l.contains("privilege") || code == 5 { "Access denied: run the program as Administrator".into() }
    else if l.contains("does not exist") || l.contains("not exist") || l.contains("invalid") { format!("Windows does not support this ({})", out.trim()) }
    else { format!("powercfg failed with code {code}: {}", out.trim()) }
}

fn ok(c: &dyn Cfg, args: &[&str]) -> Result<String, String> {
    let (code, out) = c.powercfg(args);
    if code == 0 { Ok(out) } else { Err(friendly(code, &out)) }
}

pub fn parse_schemes(text: &str) -> Vec<Scheme> {
    let g = guid_re();
    text.lines().filter_map(|line| {
        let m = g.find(line)?;
        let rest = &line[m.end()..];
        let name = match (rest.find('('), rest.rfind(')')) { (Some(a), Some(b)) if b > a => rest[a + 1..b].trim().to_string(), _ => String::new() };
        Some(Scheme { guid: m.as_str().to_lowercase(), name, active: line.trim_end().ends_with('*') })
    }).collect()
}

pub fn list(c: &dyn Cfg) -> Result<Vec<Scheme>, String> { Ok(parse_schemes(&ok(c, &["/list"])?)) }

pub fn active(c: &dyn Cfg) -> Result<Scheme, String> {
    let out = ok(c, &["/getactivescheme"])?;
    parse_schemes(&out).into_iter().next().map(|mut s| { s.active = true; s }).ok_or_else(|| "Could not read the active power plan".to_string())
}

pub fn duplicate(c: &dyn Cfg, src: &str) -> Result<String, String> {
    let out = ok(c, &["/duplicatescheme", src])?;
    guid_re().find(&out).map(|m| m.as_str().to_lowercase()).ok_or_else(|| "powercfg did not return the new plan GUID".to_string())
}
pub fn rename(c: &dyn Cfg, guid: &str, name: &str, desc: &str) -> Result<(), String> { ok(c, &["/changename", guid, name, desc]).map(|_| ()) }
pub fn set_active(c: &dyn Cfg, guid: &str) -> Result<(), String> { ok(c, &["/setactive", guid]).map(|_| ()) }
pub fn delete(c: &dyn Cfg, guid: &str) -> Result<(), String> { ok(c, &["/delete", guid]).map(|_| ()) }
pub fn export(c: &dyn Cfg, guid: &str, path: &str) -> Result<(), String> { ok(c, &["/export", path, guid]).map(|_| ()) }
pub fn import(c: &dyn Cfg, path: &str) -> Result<String, String> {
    let out = ok(c, &["/import", path])?;
    guid_re().find(&out).map(|m| m.as_str().to_lowercase()).ok_or_else(|| "powercfg did not return the imported plan GUID".to_string())
}
pub fn restore_defaults(c: &dyn Cfg) -> Result<(), String> { ok(c, &["/restoredefaultschemes"]).map(|_| ()) }

/// (AC, DC) index of one setting; None when this Windows / CPU does not have the setting.
pub fn query(c: &dyn Cfg, scheme: &str, sub: &str, setting: &str) -> Option<(u32, u32)> {
    // /qh also answers for hidden settings (system cooling policy, boost mode ...) that /query leaves out
    let (mut code, mut out) = c.powercfg(&["/qh", scheme, sub, setting]);
    if code != 0 { (code, out) = c.powercfg(&["/query", scheme, sub, setting]); }
    if code != 0 { return None; }
    let hex = Regex::new(r"0x([0-9a-fA-F]{8})").unwrap();
    let v: Vec<u32> = hex.captures_iter(&out).filter_map(|m| u32::from_str_radix(&m[1], 16).ok()).collect();
    if v.len() < 2 { return None; }
    Some((v[v.len() - 2], v[v.len() - 1]))
}

pub fn set_ac(c: &dyn Cfg, scheme: &str, sub: &str, setting: &str, v: u32) -> Result<(), String> { ok(c, &["/setacvalueindex", scheme, sub, setting, &v.to_string()]).map(|_| ()) }
pub fn set_dc(c: &dyn Cfg, scheme: &str, sub: &str, setting: &str, v: u32) -> Result<(), String> { ok(c, &["/setdcvalueindex", scheme, sub, setting, &v.to_string()]).map(|_| ()) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_the_list_in_any_language() {
        let t = "Existing Power Schemes (* Active)\n-----------------------------------\nPower Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced) *\nPower Scheme GUID: 8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c  (High performance)\nGUID แผนการใช้พลังงาน: 11111111-2222-3333-4444-555555555555  (แผนของฉัน)\n";
        let s = parse_schemes(t);
        assert_eq!(s.len(), 3);
        assert!(s[0].active && s[0].name == "Balanced" && !s[1].active);
        assert_eq!(s[2].name, "แผนของฉัน");
    }
}
