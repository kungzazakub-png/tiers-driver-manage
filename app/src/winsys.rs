//! Reading this PC: installed drivers, devices without a driver, host info, and the live driver version/INF of one device.
use crate::catalog::{Date, HostInfo, InstalledDriver};
use serde::Deserialize;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{CM_Get_DevNode_PropertyW, CM_Locate_DevNodeW, CM_LOCATE_DEVNODE_NORMAL};
use windows::Win32::Devices::Properties::{DEVPKEY_Device_DriverInfPath, DEVPKEY_Device_DriverVersion};
use windows::Win32::Foundation::DEVPROPKEY;
use windows::Win32::Devices::Properties::DEVPROPTYPE;


/// WMI returns a one-element array as a plain string for some devices.
fn str_or_seq<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum V { One(String), Many(Vec<String>) }
    Ok(Option::<V>::deserialize(d)?.map(|v| match v { V::One(s) => vec![s], V::Many(m) => m }))
}
#[derive(Deserialize, Debug)]
#[serde(rename = "Win32_PnPSignedDriver", rename_all = "PascalCase")]
struct SignedDriver {
    #[serde(rename = "DeviceID")] device_id: Option<String>,
    device_name: Option<String>,
    device_class: Option<String>,
    manufacturer: Option<String>,
    driver_version: Option<String>,
    driver_date: Option<String>,
    #[serde(rename = "HardwareID", default, deserialize_with = "str_or_seq")] hardware_id: Option<Vec<String>>,
    #[serde(rename = "CompatID", default, deserialize_with = "str_or_seq")] compat_id: Option<Vec<String>>,
}

#[derive(Deserialize, Debug)]
#[serde(rename = "Win32_PnPEntity", rename_all = "PascalCase")]
struct PnpEntity {
    #[serde(rename = "PNPDeviceID")] pnp_device_id: Option<String>,
    name: Option<String>,
    #[serde(rename = "PNPClass")] pnp_class: Option<String>,
    manufacturer: Option<String>,
    config_manager_error_code: Option<u32>,
    #[serde(rename = "HardwareID", default, deserialize_with = "str_or_seq")] hardware_id: Option<Vec<String>>,
    #[serde(rename = "CompatibleID", default, deserialize_with = "str_or_seq")] compatible_id: Option<Vec<String>>,
}

/// CIM datetime "20240208000000.000000+000" -> date.
fn cim_date(s: &str) -> Option<Date> {
    if s.len() < 8 { return None; }
    Some(Date { y: s[0..4].parse().ok()?, m: s[4..6].parse().ok()?, d: s[6..8].parse().ok()? })
}

fn joined(a: Option<Vec<String>>, b: Option<Vec<String>>) -> Vec<String> {
    a.unwrap_or_default().into_iter().chain(b.unwrap_or_default()).filter(|s| !s.is_empty()).collect()
}

/// Every device that has a driver (Windows only returns the most specific hardware ID here, so catalog patterns look like PCI\VEN_10DE&DEV_*).
pub fn installed_drivers() -> Result<Vec<InstalledDriver>, String> {
    let con = wmi::WMIConnection::new().map_err(|e| e.to_string())?;
    let rows: Vec<SignedDriver> = con.raw_query("SELECT DeviceID, DeviceName, DeviceClass, Manufacturer, DriverVersion, DriverDate, HardwareID, CompatID FROM Win32_PnPSignedDriver").map_err(|e| e.to_string())?;
    Ok(rows.into_iter().filter_map(|r| {
        let (name, version) = (r.device_name.filter(|s| !s.is_empty())?, r.driver_version.filter(|s| !s.is_empty())?);
        Some(InstalledDriver {
            instance_id: r.device_id.unwrap_or_default(), name, class: r.device_class.unwrap_or_default(), maker: r.manufacturer.unwrap_or_default(), version,
            date: r.driver_date.as_deref().and_then(cim_date), hardware_ids: joined(r.hardware_id, r.compat_id),
        })
    }).collect())
}

/// Devices Windows reports as having no working driver (problem code 1 = not configured, 28 = no driver; 22 = disabled by the user is ignored).
/// Returns the devices shaped like installed drivers (no version) plus a readable line each for the log.
pub fn devices_without_driver() -> Result<(Vec<InstalledDriver>, Vec<String>), String> {
    let con = wmi::WMIConnection::new().map_err(|e| e.to_string())?;
    let rows: Vec<PnpEntity> = con.raw_query("SELECT PNPDeviceID, Name, PNPClass, Manufacturer, ConfigManagerErrorCode, HardwareID, CompatibleID FROM Win32_PnPEntity WHERE ConfigManagerErrorCode = 1 OR ConfigManagerErrorCode = 28").map_err(|e| e.to_string())?;
    let mut devs = Vec::new();
    let mut lines = Vec::new();
    for r in rows {
        let name = r.name.filter(|s| !s.is_empty()).unwrap_or_else(|| "Unknown device".into());
        lines.push(format!("{name} [problem code {}]", r.config_manager_error_code.unwrap_or(0)));
        devs.push(InstalledDriver {
            instance_id: r.pnp_device_id.unwrap_or_default(), name, class: r.pnp_class.unwrap_or_default(), maker: r.manufacturer.unwrap_or_default(), version: String::new(),
            date: None, hardware_ids: joined(r.hardware_id, r.compatible_id),
        });
    }
    Ok((devs, lines))
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct OsRow { build_number: Option<String> }
#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct CsRow { manufacturer: Option<String>, model: Option<String> }
#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct CpuRow { name: Option<String> }

pub fn host_info() -> HostInfo {
    let arch = match std::env::consts::ARCH { "x86_64" => "x64", "aarch64" => "arm64", "x86" => "x86", other => other }.to_string();
    let build = build_number();
    HostInfo { arch, os: if build >= 22000 { "11".into() } else { "10".into() } }
}

fn build_number() -> u32 {
    let Ok(con) = wmi::WMIConnection::new() else { return 0 };
    con.raw_query::<OsRow>("SELECT BuildNumber FROM Win32_OperatingSystem").ok().and_then(|v| v.into_iter().next()).and_then(|r| r.build_number).and_then(|b| b.parse().ok()).unwrap_or(0)
}

/// "Micro-Star International Co., Ltd. MS-7D99 · Intel(R) Core(TM) i7-14700KF · Windows 11 (build 26300)"
pub fn machine_summary() -> String {
    let Ok(con) = wmi::WMIConnection::new() else { return "Windows".into() };
    let cs = con.raw_query::<CsRow>("SELECT Manufacturer, Model FROM Win32_ComputerSystem").ok().and_then(|v| v.into_iter().next());
    let cpu = con.raw_query::<CpuRow>("SELECT Name FROM Win32_Processor").ok().and_then(|v| v.into_iter().next()).and_then(|c| c.name);
    let b = build_number();
    let squash = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{} {}   ·   {}   ·   Windows {} (build {b})", cs.as_ref().and_then(|c| c.manufacturer.clone()).unwrap_or_default().trim(), cs.as_ref().and_then(|c| c.model.clone()).unwrap_or_default().trim(), squash(&cpu.unwrap_or_default()), if b >= 22000 { "11" } else { "10" })
}

fn wide(s: &str) -> Vec<u16> { OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect() }

fn dev_property_string(instance_id: &str, key: &DEVPROPKEY) -> Option<String> {
    unsafe {
        let mut devinst = 0u32;
        let id = wide(instance_id);
        if CM_Locate_DevNodeW(&mut devinst, PCWSTR(id.as_ptr()), CM_LOCATE_DEVNODE_NORMAL).0 != 0 { return None; }
        let mut ty = DEVPROPTYPE(0);
        let mut size = 0u32;
        let _ = CM_Get_DevNode_PropertyW(devinst, key, &mut ty, None, &mut size, 0);
        if size == 0 { return None; }
        let mut buf = vec![0u8; size as usize];
        if CM_Get_DevNode_PropertyW(devinst, key, &mut ty, Some(buf.as_mut_ptr()), &mut size, 0).0 != 0 { return None; }
        let units: Vec<u16> = buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
        Some(String::from_utf16_lossy(&units)).filter(|s| !s.is_empty())
    }
}

/// Live driver version from Plug and Play (Win32_PnPSignedDriver can keep showing the OLD version after an install).
pub fn live_driver_version(instance_id: &str) -> Option<String> {
    if instance_id.is_empty() { return None; }
    dev_property_string(instance_id, &DEVPKEY_Device_DriverVersion)
}

/// "oem34.inf" for the driver package a device uses now.
pub fn driver_inf(instance_id: &str) -> Option<String> {
    if instance_id.is_empty() { return None; }
    dev_property_string(instance_id, &DEVPKEY_Device_DriverInfPath)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cim_dates_parse() {
        assert_eq!(cim_date("20240208000000.000000+000"), Some(Date { y: 2024, m: 2, d: 8 }));
        assert_eq!(cim_date("1968"), None);
    }
}


