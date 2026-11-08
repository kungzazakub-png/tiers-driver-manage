//! The four profiles and the setting list. Only documented powercfg settings are used; nothing touches BIOS, voltage,
//! clocks, C-states, core parking, Defender, Windows Update or thermal protection.
use super::hw::Tier;

pub struct Setting { pub label: &'static str, pub sub: &'static str, pub guid: &'static str, pub unit: &'static str }

pub const SUB_PROCESSOR: &str = "54533251-82be-4824-96c1-47b60b740d00";
pub const MIN_STATE: Setting = Setting { label: "Minimum processor state", sub: SUB_PROCESSOR, guid: "893dee8e-2bef-41e0-89c6-b55d0929964c", unit: "%" };
pub const MAX_STATE: Setting = Setting { label: "Maximum processor state", sub: SUB_PROCESSOR, guid: "bc5038f7-23e0-4960-96da-33abaf5935ec", unit: "%" };
pub const COOLING: Setting = Setting { label: "System cooling policy", sub: SUB_PROCESSOR, guid: "94d3a615-a899-4ac5-ae2b-e4d8f634367f", unit: "cool" };
pub const USB_SUSPEND: Setting = Setting { label: "USB selective suspend", sub: "2a737441-1930-4402-8d77-b2bebba308a3", guid: "48e6b7a6-50f5-4782-a5d4-53bb8f07e226", unit: "onoff" };
pub const PCIE_LINK: Setting = Setting { label: "PCI Express link state power management", sub: "501a4d13-42af-4429-9fd1-a8218c268e20", guid: "ee12f906-d277-404b-b6da-e5fa1a576df5", unit: "pcie" };
// read-only, used by the Ultimate vs Balanced comparison
pub const BOOST_MODE: Setting = Setting { label: "Processor performance boost mode", sub: SUB_PROCESSOR, guid: "be337238-0d82-4146-a960-4f3749d470c7", unit: "" };
pub const PARKING_MIN: Setting = Setting { label: "Minimum cores kept unparked", sub: SUB_PROCESSOR, guid: "0cc5b647-c1df-4637-891a-dec35c318583", unit: "%" };
pub const IDLE_DISABLE: Setting = Setting { label: "Processor idle disable", sub: SUB_PROCESSOR, guid: "5d76a2ca-e8c0-402f-a133-2158492d58ad", unit: "" };

pub const OVERLAY_BEST: &str = "ded574b5-45a0-4f42-8737-46345c09c238";

pub struct Profile {
    pub tier: Tier,
    pub title: &'static str,
    pub suited: &'static str,
    pub goal: &'static str,
    /// sets Windows Power Mode to Best Performance
    pub power_mode_best: bool,
    pub usb_optional: bool,
    pub pcie_optional: bool,
    pub ultimate_optional: bool,
}

pub const PROFILES: [Profile; 4] = [
    Profile { tier: Tier::Low, title: "Low spec", suited: "เครื่องเก่า ระดับ Core i3 / Ryzen 3 แรม 4–8 GB", goal: "เสถียรและเย็น ใช้แผน Balanced ไม่ปรับอะไรแรง",
              power_mode_best: false, usb_optional: false, pcie_optional: false, ultimate_optional: false },
    Profile { tier: Tier::Medium, title: "Medium", suited: "ระดับ Core i5 / Ryzen 5 แรม 8–16 GB", goal: "สมดุลระหว่างความแรงกับการกินไฟ",
              power_mode_best: true, usb_optional: false, pcie_optional: false, ultimate_optional: false },
    Profile { tier: Tier::High, title: "High performance", suited: "Core i5–i7 / Ryzen 5–7 รุ่นใหม่ แรม 16–32 GB", goal: "เน้นเล่นเกมและความไวในการตอบสนอง",
              power_mode_best: true, usb_optional: true, pcie_optional: true, ultimate_optional: false },
    Profile { tier: Tier::Extreme, title: "Extreme performance", suited: "Core i7–i9 / Ryzen 7–9 แรม 32 GB ขึ้นไป เครื่องตั้งโต๊ะที่ระบายความร้อนดี", goal: "ตัดการประหยัดไฟที่ไม่จำเป็นออก ระบบป้องกัน CPU ยังทำงานอยู่",
              power_mode_best: false, usb_optional: true, pcie_optional: true, ultimate_optional: true },
];

pub fn profile(t: Tier) -> &'static Profile { PROFILES.iter().find(|p| p.tier == t).unwrap() }

#[derive(Clone, Copy, Default, Debug, serde::Serialize, serde::Deserialize)]
pub struct Opts { pub usb_off: bool, pub pcie_off: bool, pub ultimate: bool, pub restore_point: bool }

/// One change the profile makes. `ac`/`dc` None = leave that side untouched.
pub struct Change { pub setting: &'static Setting, pub ac: Option<u32>, pub dc: Option<u32> }

/// The settings a profile writes into the NEW plan (Keep Default settings are simply not listed).
pub fn changes(p: &Profile, o: Opts) -> Vec<Change> {
    let mut v = vec![
        Change { setting: &MIN_STATE, ac: Some(5), dc: Some(5) },
        Change { setting: &MAX_STATE, ac: Some(100), dc: Some(100) },
        Change { setting: &COOLING, ac: Some(1), dc: Some(1) },
    ];
    if p.usb_optional && o.usb_off { v.push(Change { setting: &USB_SUSPEND, ac: Some(0), dc: None }); }
    if p.pcie_optional && o.pcie_off { v.push(Change { setting: &PCIE_LINK, ac: Some(0), dc: None }); }
    v
}

pub fn describe(s: &Setting, v: u32) -> String {
    match s.unit {
        "%" => format!("{v}%"),
        "cool" => if v == 1 { "Active".into() } else { "Passive".into() },
        "onoff" => if v == 1 { "Enabled".into() } else { "Disabled".into() },
        "pcie" => match v { 0 => "Off".into(), 1 => "Moderate power savings".into(), 2 => "Maximum power savings".into(), n => n.to_string() },
        _ => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_settings_are_only_written_when_chosen_and_allowed() {
        let none = Opts::default();
        assert_eq!(changes(profile(Tier::High), none).len(), 3);
        assert_eq!(changes(profile(Tier::High), Opts { usb_off: true, pcie_off: true, ..none }).len(), 5);
        // Low and Medium have no optional settings: choosing them changes nothing
        assert_eq!(changes(profile(Tier::Low), Opts { usb_off: true, pcie_off: true, ..none }).len(), 3);
        assert!(changes(profile(Tier::Medium), Opts { usb_off: true, ..none }).iter().all(|c| c.setting.guid != USB_SUSPEND.guid));
    }
    #[test]
    fn minimum_is_never_forced_to_100() {
        for p in PROFILES.iter() { assert!(changes(p, Opts { usb_off: true, pcie_off: true, ultimate: true, restore_point: true }).iter().all(|c| c.setting.guid != MIN_STATE.guid || c.ac == Some(5))); }
    }
}
