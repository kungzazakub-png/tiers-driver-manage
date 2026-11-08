//! Hardware detection and the performance tier. The tier is a score from several facts; the Core i3/i5/i7 or Ryzen 3/5/7
//! brand name is never used on its own.
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde::Deserialize as De;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, PartialOrd, Ord)]
pub enum Tier { Low, Medium, High, Extreme }
impl Tier {
    pub fn name(self) -> &'static str { match self { Tier::Low => "LOW", Tier::Medium => "MEDIUM", Tier::High => "HIGH", Tier::Extreme => "EXTREME" } }
    pub fn from_key(k: &str) -> Option<Tier> { match k { "low" => Some(Tier::Low), "medium" => Some(Tier::Medium), "high" => Some(Tier::High), "extreme" => Some(Tier::Extreme), _ => None } }
    pub fn key(self) -> &'static str { match self { Tier::Low => "low", Tier::Medium => "medium", Tier::High => "high", Tier::Extreme => "extreme" } }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
pub enum Vendor { Intel, Amd, Unknown }

#[derive(Clone, Debug, Serialize)]
pub struct Hardware {
    pub vendor: Vendor,
    pub cpu_name: String,
    pub cores: u32,
    pub threads: u32,
    pub ram_gb: f64,
    pub windows: String,
    pub build: u32,
    pub laptop: bool,
    /// None = unknown
    pub on_ac: Option<bool>,
    pub active_plan: String,
}

pub fn vendor_of(name: &str, manufacturer: &str) -> Vendor {
    let s = format!("{name} {manufacturer}").to_lowercase();
    if s.contains("intel") { Vendor::Intel } else if s.contains("amd") || s.contains("ryzen") { Vendor::Amd } else { Vendor::Unknown }
}

/// CPU generation: Intel 14700KF -> 14, i7-8700 -> 8, Core Ultra -> 15; Ryzen 7 7800X3D -> 7 (series thousand).
pub fn generation(name: &str) -> Option<u32> {
    let intel = Regex::new(r"i[3579]-(\d{4,5})").unwrap();
    if let Some(c) = intel.captures(name) { let d = &c[1]; return d[..d.len() - 3].parse().ok(); }
    if name.contains("Core(TM) Ultra") || name.contains("Core Ultra") { return Some(15); }
    let amd = Regex::new(r"Ryzen\s+(?:AI\s+)?\d\s+(?:PRO\s+)?(\d)\d{3}").unwrap();
    amd.captures(name).and_then(|c| c[1].parse().ok())
}

/// (tier, the reasons that produced it)
pub fn score(h: &Hardware) -> (Tier, Vec<String>) {
    let mut pts = 0i32;
    let mut why = vec![];
    let t = match h.threads { 0 => 0, 1..=4 => 0, 5..=8 => 1, 9..=12 => 2, 13..=16 => 3, _ => 4 };
    pts += t; why.push(format!("{} threads: {t}/4 points", h.threads));
    let r = if h.ram_gb < 7.5 { 0 } else if h.ram_gb <= 16.5 { 1 } else if h.ram_gb <= 32.5 { 2 } else { 3 };
    pts += r; why.push(format!("{:.0} GB RAM: {r}/3 points", h.ram_gb));
    match generation(&h.cpu_name) {
        Some(g) => {
            let (intel_pts, amd_pts) = (if g <= 7 { 0 } else if g <= 10 { 1 } else if g <= 12 { 2 } else { 3 }, if g <= 2 { 0 } else if g <= 4 { 1 } else if g <= 5 { 2 } else { 3 });
            let p = if h.vendor == Vendor::Amd { amd_pts } else { intel_pts };
            pts += p; why.push(format!("CPU generation {g}: {p}/3 points"));
        }
        None => { pts += 1; why.push("CPU generation unknown: 1/3 points".into()); }
    }
    let up = Regex::new(r"(\d[KX]F?\b|\dHX|X3D|\bHX\b)").unwrap();
    let low = Regex::new(r"(\d[UY]\b|\d(?:G|GE)\b)").unwrap();
    if up.is_match(&h.cpu_name) { pts += 1; why.push("high-power SKU (K/X/HX): +1".into()); }
    else if low.is_match(&h.cpu_name) { pts -= 1; why.push("low-power SKU (U/Y): -1".into()); }
    let mut tier = if pts <= 3 { Tier::Low } else if pts <= 6 { Tier::Medium } else if pts <= 8 { Tier::High } else { Tier::Extreme };
    if h.cpu_name.is_empty() || h.threads == 0 { why.push("CPU could not be read: using the safe default (Balanced)".into()); return (Tier::Low, why); }
    if h.laptop && tier > Tier::High { tier = Tier::High; why.push("laptop: capped at HIGH".into()); }
    if h.laptop && h.on_ac == Some(false) && tier > Tier::Medium { tier = Tier::Medium; why.push("running on battery: capped at MEDIUM".into()); }
    (tier, why)
}

#[derive(De, Debug)]
#[serde(rename_all = "PascalCase")]
struct Cpu { name: Option<String>, manufacturer: Option<String>, number_of_cores: Option<u32>, number_of_logical_processors: Option<u32> }
#[derive(De, Debug)]
#[serde(rename_all = "PascalCase")]
struct Cs { total_physical_memory: Option<u64>, caption: Option<String> }
#[derive(De, Debug)]
#[serde(rename_all = "PascalCase")]
struct Enc { chassis_types: Option<Vec<u16>> }
#[derive(De, Debug)]
#[serde(rename_all = "PascalCase")]
struct Os { caption: Option<String>, build_number: Option<String> }
#[derive(De, Debug)]
#[serde(rename_all = "PascalCase")]
struct Bat { name: Option<String> }

/// Laptop = a portable chassis type (8,9,10,11,12,14,18,21,30,31,32) or a battery present.
pub fn is_laptop(chassis: &[u16], has_battery: bool) -> bool { has_battery || chassis.iter().any(|c| matches!(c, 8 | 9 | 10 | 11 | 12 | 14 | 18 | 21 | 30 | 31 | 32)) }

pub fn on_ac() -> Option<bool> {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut s = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut s) }.is_err() { return None; }
    match s.ACLineStatus { 1 => Some(true), 0 => Some(false), _ => None }
}

pub fn detect(active_plan: String) -> Hardware {
    let mut h = Hardware { vendor: Vendor::Unknown, cpu_name: String::new(), cores: 0, threads: 0, ram_gb: 0.0, windows: String::new(), build: 0, laptop: false, on_ac: on_ac(), active_plan };
    let Ok(con) = wmi::WMIConnection::new() else { return h };
    if let Some(c) = con.raw_query::<Cpu>("SELECT Name, Manufacturer, NumberOfCores, NumberOfLogicalProcessors FROM Win32_Processor").ok().and_then(|v| v.into_iter().next()) {
        h.cpu_name = c.name.unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
        h.vendor = vendor_of(&h.cpu_name, c.manufacturer.as_deref().unwrap_or(""));
        h.cores = c.number_of_cores.unwrap_or(0); h.threads = c.number_of_logical_processors.unwrap_or(0);
    }
    if let Some(c) = con.raw_query::<Cs>("SELECT TotalPhysicalMemory, Caption FROM Win32_ComputerSystem").ok().and_then(|v| v.into_iter().next()) {
        h.ram_gb = c.total_physical_memory.unwrap_or(0) as f64 / 1_073_741_824.0;
    }
    let chassis = con.raw_query::<Enc>("SELECT ChassisTypes FROM Win32_SystemEnclosure").ok().and_then(|v| v.into_iter().next()).and_then(|e| e.chassis_types).unwrap_or_default();
    let has_battery = con.raw_query::<Bat>("SELECT Name FROM Win32_Battery").map(|v| !v.is_empty()).unwrap_or(false);
    h.laptop = is_laptop(&chassis, has_battery);
    if let Some(o) = con.raw_query::<Os>("SELECT Caption, BuildNumber FROM Win32_OperatingSystem").ok().and_then(|v| v.into_iter().next()) {
        h.build = o.build_number.and_then(|b| b.parse().ok()).unwrap_or(0);
        h.windows = format!("{} (build {})", o.caption.unwrap_or_else(|| "Windows".into()).replace("Microsoft ", "").trim(), h.build);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hw(name: &str, threads: u32, ram: f64, laptop: bool, ac: Option<bool>) -> Hardware {
        Hardware { vendor: vendor_of(name, ""), cpu_name: name.into(), cores: threads / 2, threads, ram_gb: ram, windows: "Windows 11".into(), build: 26100, laptop, on_ac: ac, active_plan: "Balanced".into() }
    }
    #[test]
    fn generations() {
        assert_eq!(generation("Intel(R) Core(TM) i7-14700KF"), Some(14));
        assert_eq!(generation("Intel(R) Core(TM) i7-8700 CPU @ 3.20GHz"), Some(8));
        assert_eq!(generation("AMD Ryzen 7 7800X3D 8-Core Processor"), Some(7));
        assert_eq!(generation("AMD Ryzen 5 3600 6-Core Processor"), Some(3));
        assert_eq!(generation("Intel(R) Core(TM) Ultra 9 285K"), Some(15));
    }
    #[test]
    fn tiers_use_several_facts() {
        assert_eq!(score(&hw("Intel(R) Core(TM) i7-14700KF", 28, 64.0, false, Some(true))).0, Tier::Extreme);
        assert_eq!(score(&hw("Intel(R) Core(TM) i3-7100", 4, 8.0, false, Some(true))).0, Tier::Low);
        // the brand name alone does not decide: an old i7 with little RAM is not high tier
        assert!(score(&hw("Intel(R) Core(TM) i7-4500U", 4, 8.0, true, Some(true))).0 <= Tier::Low);
        assert_eq!(score(&hw("AMD Ryzen 5 5600X 6-Core Processor", 12, 16.0, false, Some(true))).0, Tier::Medium);
        assert_eq!(score(&hw("Intel(R) Core(TM) i7-12700", 20, 32.0, false, Some(true))).0, Tier::High);
    }
    #[test]
    fn laptop_and_battery_caps() {
        let (t, _) = score(&hw("Intel(R) Core(TM) i9-14900HX", 32, 64.0, true, Some(true)));
        assert_eq!(t, Tier::High);
        let (t, _) = score(&hw("Intel(R) Core(TM) i9-14900HX", 32, 64.0, true, Some(false)));
        assert_eq!(t, Tier::Medium);
    }
    #[test]
    fn unreadable_cpu_falls_back_to_balanced() {
        assert_eq!(score(&hw("", 0, 0.0, false, None)).0, Tier::Low);
    }
}
