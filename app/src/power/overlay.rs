//! Windows "Power mode" (Best performance / Balanced ...) is a power-plan overlay set through powrprof.dll.
//! The functions only exist on newer Windows builds, so they are looked up at run time; if they are missing the setting is reported as unsupported.
use windows::core::{s, w, GUID};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

pub trait Overlay {
    fn available(&self) -> bool;
    /// current overlay GUID as text (all zeros = Balanced / "Recommended")
    fn get(&self) -> Option<String>;
    fn set(&self, guid: &str) -> Result<(), String>;
}

pub fn guid_text(g: &GUID) -> String {
    format!("{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}", g.data1, g.data2, g.data3, g.data4[0], g.data4[1], g.data4[2], g.data4[3], g.data4[4], g.data4[5], g.data4[6], g.data4[7])
}

pub fn parse_guid(t: &str) -> Option<GUID> {
    let h: String = t.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if h.len() != 32 || t.len() != 36 { return None; }
    let n = u128::from_str_radix(&h, 16).ok()?;
    Some(GUID::from_u128(n))
}

pub fn overlay_name(guid: &str) -> &'static str {
    match guid {
        "ded574b5-45a0-4f42-8737-46345c09c238" => "Best performance",
        "3af9b8d9-7c97-431d-ad78-34a8bfea439f" => "Better performance",
        "961cc777-2547-4f9d-8174-7d86181b8a7a" => "Best power efficiency",
        "00000000-0000-0000-0000-000000000000" => "Balanced",
        _ => "Custom",
    }
}

pub struct RealOverlay;
type GetFn = unsafe extern "system" fn(*mut GUID) -> u32;
type SetFn = unsafe extern "system" fn(GUID) -> u32;

fn procs() -> Option<(GetFn, SetFn)> {
    unsafe {
        let lib = LoadLibraryW(w!("powrprof.dll")).ok()?;
        let g = GetProcAddress(lib, s!("PowerGetActualOverlayScheme"))?;
        let st = GetProcAddress(lib, s!("PowerSetActiveOverlayScheme"))?;
        Some((std::mem::transmute::<_, GetFn>(g), std::mem::transmute::<_, SetFn>(st)))
    }
}

impl Overlay for RealOverlay {
    fn available(&self) -> bool { procs().is_some() && self.get().is_some() }
    fn get(&self) -> Option<String> {
        let (get, _) = procs()?;
        let mut g = GUID::zeroed();
        if unsafe { get(&mut g) } != 0 { return None; }
        Some(guid_text(&g))
    }
    fn set(&self, guid: &str) -> Result<(), String> {
        let (_, set) = procs().ok_or("This Windows version has no Power mode setting")?;
        let g = parse_guid(guid).ok_or("Invalid overlay GUID")?;
        let r = unsafe { set(g) };
        if r == 0 { Ok(()) } else { Err(format!("Windows refused the Power mode change (code {r})")) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guid_round_trip() {
        let g = parse_guid("ded574b5-45a0-4f42-8737-46345c09c238").unwrap();
        assert_eq!(guid_text(&g), "ded574b5-45a0-4f42-8737-46345c09c238");
        assert_eq!(overlay_name(&guid_text(&g)), "Best performance");
    }
    #[test]
    fn real_overlay_reads_on_this_pc() {
        // only checks it does not crash; the value depends on the Windows build
        let _ = RealOverlay.get();
    }
}
