//! Opt-in report of devices that no source has a driver for. Only hardware IDs, the device class and the device name leave
//! the PC: no instance IDs (they can contain serial numbers), no machine name, no user name, no IP stored by the program.
use crate::catalog::InstalledDriver;
use crate::http::Http;
use regex::Regex;
use serde::Serialize;

#[derive(Serialize, Debug, PartialEq)]
pub struct Item { pub ids: Vec<String>, pub class: String, pub name: String }

pub const MAX_ITEMS: usize = 50;
const MAX_IDS: usize = 4;

/// The hardware-ID alphabet the backend also enforces; anything else is dropped rather than sent.
fn id_ok(id: &str) -> bool { id.len() <= 200 && Regex::new(r"^[A-Za-z0-9_&\\\-\.#\{\}]+$").unwrap().is_match(id) }

pub fn build(unresolved: &[InstalledDriver]) -> Vec<Item> {
    unresolved.iter().filter_map(|d| {
        let ids: Vec<String> = d.hardware_ids.iter().filter(|i| id_ok(i)).take(MAX_IDS).cloned().collect();
        if ids.is_empty() { return None; }
        Some(Item { ids, class: d.class.chars().filter(|c| c.is_ascii_alphanumeric()).take(32).collect(), name: d.name.chars().filter(|c| !c.is_control()).take(80).collect() })
    }).take(MAX_ITEMS).collect()
}

/// Sends the report; returns how many devices were sent.
pub fn send(http: &dyn Http, url: &str, items: &[Item]) -> Result<usize, String> {
    if items.is_empty() { return Err("There is nothing to report".into()); }
    if !url.starts_with("https://") { return Err("The report address must be https".into()); }
    let body = serde_json::json!({ "v": 1, "devices": items }).to_string();
    http.post_json(url, &body)?;
    Ok(items.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Fake;
    fn dev(name: &str, class: &str, ids: &[&str]) -> InstalledDriver {
        InstalledDriver { instance_id: "USB\\VID_04A9&PID_1830\\SERIAL-12345".into(), name: name.into(), class: class.into(), maker: String::new(), version: String::new(), date: None, hardware_ids: ids.iter().map(|s| s.to_string()).collect() }
    }
    #[test]
    fn only_ids_class_and_name_are_sent_never_the_instance_id() {
        let items = build(&[dev("Canon MG3600 series", "USB", &["USB\\VID_04A9&PID_1830&REV_0100", "USB\\VID_04A9&PID_1830"])]);
        assert_eq!(items.len(), 1);
        let json = serde_json::to_string(&items).unwrap();
        assert!(!json.contains("SERIAL") && json.contains("VID_04A9"));
    }
    #[test]
    fn odd_ids_are_dropped_and_devices_without_ids_are_skipped() {
        assert!(build(&[dev("x", "USB", &["USB\\VID_1&PID_2; rm -rf", "<script>"])]).is_empty());
        assert_eq!(build(&[dev("x", "USB", &["USB\\VID_1&PID_2; bad", "PCI\\VEN_8086&DEV_1234"])])[0].ids, vec!["PCI\\VEN_8086&DEV_1234"]);
        let many: Vec<_> = (0..80).map(|i| dev("d", "NET", &[&format!("PCI\\VEN_8086&DEV_{i:04X}")])).collect();
        assert_eq!(build(&many).len(), MAX_ITEMS);
    }
    #[test]
    fn send_posts_json_to_an_https_address_only() {
        let http = Fake::new(vec![("report.example", "ok")]);
        let items = build(&[dev("Canon", "USB", &["USB\\VID_04A9&PID_1830"])]);
        assert_eq!(send(&http, "https://report.example/report", &items), Ok(1));
        assert!(http.calls.borrow().iter().any(|c| c.starts_with("BODY {") && c.contains("VID_04A9")));
        assert!(send(&http, "http://report.example/report", &items).is_err());
        assert!(send(&http, "https://report.example/report", &[]).is_err());
    }
}
