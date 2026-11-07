//! Microsoft Update Catalog provider: finds drivers by hardware ID (WHQL-signed by Microsoft).
//! Unofficial: it reads the catalog's web pages, so a layout change on Microsoft's side can break it. It then fails loudly.
use crate::catalog::{Date, HostInfo, InstalledDriver, Offer, OfferKind};
use crate::http::Http;
use crate::version::Ver;
use regex::Regex;

const SEARCH_URL: &str = "https://www.catalog.update.microsoft.com/Search.aspx?q=";
const DOWNLOAD_URL: &str = "https://www.catalog.update.microsoft.com/DownloadDialog.aspx";

#[derive(Debug, Clone)]
pub struct Row {
    pub update_id: String,
    pub title: String,
    pub products: String,
    pub class: String,
    pub date: Option<Date>,
    pub version: String,
    pub size_mb: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Download { pub url: String, pub digest: String }

/// "PCI\VEN_8086&DEV_7A70&SUBSYS_..." -> "PCI\VEN_8086&DEV_7A70" (the vendor/device pair the catalog indexes).
pub fn hardware_key(hid: &str) -> Option<String> {
    let res = [r"(?i)^PCI\\VEN_[0-9A-F]{4}&DEV_[0-9A-F]{4}", r"(?i)^USB\\VID_[0-9A-F]{4}&PID_[0-9A-F]{4}", r"(?i)^HDAUDIO\\FUNC_\d\d&VEN_[0-9A-F]{4}&DEV_[0-9A-F]{4}"];
    res.iter().find_map(|r| Regex::new(r).unwrap().find(hid).map(|m| m.as_str().to_ascii_uppercase()))
}

fn html_text(s: &str) -> String {
    let no_tags = Regex::new(r"<[^>]+>").unwrap().replace_all(s, "");
    let dec = no_tags.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&");
    Regex::new(r"\s+").unwrap().replace_all(&dec, " ").trim().to_string()
}

fn parse_date(s: &str) -> Option<Date> {
    let mut it = s.split('/');
    let (m, d, y) = (it.next()?.trim().parse().ok()?, it.next()?.trim().parse().ok()?, it.next()?.trim().parse().ok()?);
    Some(Date { y, m, d })
}

fn parse_size_mb(s: &str) -> Option<f64> {
    let c = Regex::new(r"^([\d\.,]+)\s*(KB|MB|GB)").unwrap().captures(s)?;
    let n: f64 = c[1].replace(',', "").parse().ok()?;
    let f = match &c[2] { "KB" => 1.0 / 1024.0, "GB" => 1024.0, _ => 1.0 };
    Some((n * f * 100.0).round() / 100.0)
}

pub fn parse_rows(html: &str) -> Vec<Row> {
    let row_re = Regex::new(r#"(?s)<tr id="([0-9a-f\-]{36})_R\d+"[^>]*>(.*?)</tr>"#).unwrap();
    let cell_re = Regex::new(r"(?s)<td[^>]*>(.*?)</td>").unwrap();
    row_re.captures_iter(html).filter_map(|r| {
        let cells: Vec<String> = cell_re.captures_iter(&r[2]).map(|c| html_text(&c[1])).collect();
        if cells.len() < 7 { return None; }
        Some(Row { update_id: r[1].to_string(), title: cells[1].clone(), products: cells[2].clone(), class: cells[3].clone(), date: parse_date(&cells[4]), version: cells[5].clone(), size_mb: parse_size_mb(&cells[6]) })
    }).collect()
}

/// Rows, or an empty list for a genuine "no results" page. Any other page means Microsoft changed it (or blocked us): fail loudly.
pub fn get_rows(html: &str) -> Result<Vec<Row>, String> {
    let rows = parse_rows(html);
    if rows.is_empty() && !html.contains("did not find any results") {
        return Err("Unrecognised Microsoft Update Catalog search page format (the page may have changed or is blocked)".into());
    }
    Ok(rows)
}

/// Newest driver row that fits this Windows (11 also accepts Windows 10 drivers).
pub fn select_best(rows: &[Row], host: &HostInfo) -> Option<Row> {
    let os_re = Regex::new(if host.os == "11" { r"Windows (11|10)" } else { "Windows 10" }).unwrap();
    let mut ok: Vec<(&Row, Ver)> = rows.iter()
        .filter(|r| r.class.to_ascii_lowercase().starts_with("drivers") && os_re.is_match(&r.products))
        .filter_map(|r| Ver::parse(&r.version).map(|v| (r, v))).collect();
    ok.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.date.cmp(&a.0.date)));
    ok.first().map(|(r, _)| (*r).clone())
}

#[derive(Debug)]
pub struct FindResult { pub offers: Vec<Offer>, pub failed_queries: usize }

/// Asks the catalog about every non-Microsoft device (one query per distinct hardware ID, at most `max_queries`).
/// Devices without any driver are expected first in `installed` so they are never starved by the query limit.
pub fn find_updates(installed: &[InstalledDriver], host: &HostInfo, max_queries: usize, search: &dyn Fn(&str) -> Result<Vec<Row>, String>, live_version: &dyn Fn(&str) -> Option<String>) -> Result<FindResult, String> {
    let mut by_key: Vec<(String, &InstalledDriver)> = Vec::new();
    for d in installed {
        let maker = d.maker.to_ascii_lowercase();
        if maker.starts_with("microsoft") || maker.starts_with("(standard") || maker.starts_with("standard") { continue; }
        if let Some(key) = d.hardware_ids.iter().find_map(|h| hardware_key(h)) {
            if !by_key.iter().any(|(k, _)| *k == key) { by_key.push((key, d)); }
        }
    }
    let (mut found, mut n, mut failed, mut first_err): (Vec<Offer>, usize, usize, String) = (Vec::new(), 0, 0, String::new());
    for (key, d) in &by_key {
        if n >= max_queries { break; }
        n += 1;
        let best = match search(key).map(|rows| select_best(&rows, host)) {
            Ok(b) => b,
            Err(e) => { failed += 1; if first_err.is_empty() { first_err = e; } continue; }
        };
        let Some(best) = best else { continue };
        let new_v = Ver::parse(&best.version);
        let mut cur = Ver::parse(&d.version);
        if let Some(live) = live_version(&d.instance_id).and_then(|v| Ver::parse(&v)) { cur = Some(live); }
        if let (Some(cur), Some(new_v)) = (cur, new_v) { if new_v <= cur { continue; } }
        // Intel INF drivers use a fake 1968 date, so the date is only trusted when both sides look real.
        if let (Some(bd), Some(dd)) = (best.date, d.date) {
            if bd.y > 1990 && dd.y > 1990 && bd < dd { continue; }
        }
        let now = if d.version.is_empty() { "no driver yet".to_string() } else { format!("now {}", d.version) };
        found.push(Offer {
            id: format!("ms:{}", best.update_id), title: format!("{}: {} [{}]", d.name, best.title, now), class: d.class.clone(), maker: d.maker.clone(),
            date: best.date.map(|x| x.iso()).unwrap_or_default(), size_mb: best.size_mb, source: "Microsoft Catalog".into(),
            instance_id: d.instance_id.clone(), device_name: d.name.clone(),
            kind: OfferKind::MsCatalog { update_id: best.update_id.clone(), key: key.clone(), name: d.name.clone() },
        });
    }
    if n > 0 && failed == n { return Err(format!("Every Microsoft Catalog query failed ({failed} attempts): {first_err}")); }
    // the same package for two devices is offered once
    let mut seen = std::collections::HashSet::new();
    found.retain(|o| match &o.kind { OfferKind::MsCatalog { update_id, .. } => seen.insert(update_id.clone()), _ => true });
    Ok(FindResult { offers: found, failed_queries: failed })
}

pub fn search(http: &dyn Http, key: &str) -> Result<Vec<Row>, String> {
    let enc: String = key.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    let page = http.get(&format!("{SEARCH_URL}{enc}"))?;
    get_rows(&String::from_utf8_lossy(&page))
}

/// Download links and Microsoft's own SHA-1 digest (base64) for one catalog update.
pub fn get_downloads(http: &dyn Http, update_id: &str) -> Result<Vec<Download>, String> {
    let body = format!(r#"[{{"size":0,"languages":"","uidInfo":"{id}","updateID":"{id}"}}]"#, id = update_id);
    let page = http.post_form(DOWNLOAD_URL, &[("updateIDs", &body)])?;
    Ok(parse_downloads(&String::from_utf8_lossy(&page)))
}

pub fn parse_downloads(page: &str) -> Vec<Download> {
    Regex::new(r"(?s)\.url\s*=\s*'(https://[^']+)'.*?\.digest\s*=\s*'([^']+)'").unwrap()
        .captures_iter(page).map(|c| Download { url: c[1].to_string(), digest: c[2].to_string() }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Fake;

    fn host() -> HostInfo { HostInfo { arch: "x64".into(), os: "11".into() } }
    fn no_live(_: &str) -> Option<String> { None }
    fn row(id: &str, class: &str, products: &str, ver: &str, date: (i32, u32, u32)) -> String {
        format!(r#"<tr id="{id}_R1"><td><input/></td><td><a>Title {ver}</a></td><td>{products}</td><td>{class}</td><td>{}/{}/{}</td><td>{ver}</td><td>1.5 MB</td></tr>"#, date.1, date.2, date.0)
    }
    const ID1: &str = "11111111-1111-1111-1111-111111111111";
    const ID2: &str = "22222222-2222-2222-2222-222222222222";
    const ID3: &str = "33333333-3333-3333-3333-333333333333";
    const ID4: &str = "44444444-4444-4444-4444-444444444444";
    fn dev(name: &str, maker: &str, ver: &str, date: Option<Date>, hw: &str) -> InstalledDriver {
        InstalledDriver { instance_id: "i".into(), name: name.into(), class: "NET".into(), maker: maker.into(), version: ver.into(), date, hardware_ids: vec![hw.into()] }
    }

    #[test]
    fn keys_are_trimmed_to_the_catalog_index() {
        assert_eq!(hardware_key("PCI\\VEN_8086&DEV_7a70&SUBSYS_1&REV_01").as_deref(), Some("PCI\\VEN_8086&DEV_7A70"));
        assert_eq!(hardware_key("USB\\VID_8087&PID_0033&REV_0001").as_deref(), Some("USB\\VID_8087&PID_0033"));
        assert_eq!(hardware_key("ACPI\\INT3400"), None);
    }

    #[test]
    fn html_rows_are_parsed_with_version_date_and_size() {
        let html = format!("<table>{}{}{}</table>", row(ID1, "Drivers (Networking)", "Windows 11", "1.0.0.1", (2025, 1, 2)), row(ID2, "Updates", "Windows 11", "9.0.0.0", (2025, 1, 2)), row(ID3, "Drivers (Networking)", "Windows 7", "3.0.0.0", (2025, 1, 2)));
        let rows = parse_rows(&html);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].date, Some(Date { y: 2025, m: 1, d: 2 }));
        assert_eq!(rows[0].size_mb, Some(1.5));
        // newest DRIVER row for this Windows: not the "Updates" class, not the Windows 7 one
        assert_eq!(select_best(&rows, &host()).unwrap().update_id, ID1);
    }

    #[test]
    fn page_layout_rules() {
        assert!(get_rows("<html>We did not find any results for your search.</html>").unwrap().is_empty());
        assert!(get_rows("<html>captcha</html>").unwrap_err().contains("Unrecognised"));
    }

    #[test]
    fn newer_catalog_driver_is_offered_same_version_is_not() {
        let rows = parse_rows(&row(ID1, "Drivers (Networking)", "Windows 11", "2.0.0.0", (2025, 6, 1)));
        let search = |_: &str| Ok(rows.clone());
        let d = dev("Wi-Fi", "Intel", "1.0.0.0", None, "PCI\\VEN_8086&DEV_1234&SUBSYS_1");
        let r = find_updates(&[d], &host(), 40, &search, &no_live).unwrap();
        assert_eq!(r.offers.len(), 1);
        assert!(r.offers[0].title.contains("[now 1.0.0.0]"));
        let same = dev("Wi-Fi", "Intel", "2.0.0.0", None, "PCI\\VEN_8086&DEV_1234&SUBSYS_1");
        assert!(find_updates(&[same], &host(), 40, &search, &no_live).unwrap().offers.is_empty());
    }

    #[test]
    fn microsoft_inbox_devices_are_skipped() {
        let rows = parse_rows(&row(ID1, "Drivers (Networking)", "Windows 11", "9.0.0.0", (2025, 6, 1)));
        let search = |_: &str| Ok(rows.clone());
        let d = dev("Standard NVM Express", "Microsoft", "1.0.0.0", None, "PCI\\VEN_8086&DEV_1234&SUBSYS_1");
        assert!(find_updates(&[d], &host(), 40, &search, &no_live).unwrap().offers.is_empty());
    }

    #[test]
    fn higher_version_but_older_real_date_is_not_offered_and_fake_1968_dates_are_ignored() {
        let rows = parse_rows(&row(ID1, "Drivers (Networking)", "Windows 11", "2.0.0.0", (2020, 1, 1)));
        let search = |_: &str| Ok(rows.clone());
        let newer_installed = dev("D", "Intel", "1.0.0.0", Some(Date { y: 2024, m: 1, d: 1 }), "PCI\\VEN_8086&DEV_1234&SUBSYS_1");
        assert!(find_updates(&[newer_installed], &host(), 40, &search, &no_live).unwrap().offers.is_empty());
        let fake_date = dev("D", "Intel", "1.0.0.0", Some(Date { y: 1968, m: 7, d: 18 }), "PCI\\VEN_8086&DEV_1234&SUBSYS_1");
        assert_eq!(find_updates(&[fake_date], &host(), 40, &search, &no_live).unwrap().offers.len(), 1);
    }

    #[test]
    fn same_package_for_two_devices_is_offered_once() {
        let rows = parse_rows(&row(ID1, "Drivers (Networking)", "Windows 11", "2.0.0.0", (2025, 6, 1)));
        let search = |_: &str| Ok(rows.clone());
        let a = dev("A", "Intel", "1.0.0.0", None, "PCI\\VEN_8086&DEV_1111&SUBSYS_1");
        let b = dev("B", "Intel", "1.0.0.0", None, "PCI\\VEN_8086&DEV_2222&SUBSYS_1");
        assert_eq!(find_updates(&[a, b], &host(), 40, &search, &no_live).unwrap().offers.len(), 1);
    }

    #[test]
    fn live_version_suppresses_an_offer() {
        let rows = parse_rows(&row(ID1, "Drivers (Networking)", "Windows 11", "2.0.0.0", (2025, 6, 1)));
        let search = |_: &str| Ok(rows.clone());
        let live = |_: &str| Some("2.0.0.0".to_string());
        let d = dev("A", "Intel", "1.0.0.0", None, "PCI\\VEN_8086&DEV_1111&SUBSYS_1");
        assert!(find_updates(&[d], &host(), 40, &search, &live).unwrap().offers.is_empty());
    }

    #[test]
    fn every_query_failing_is_an_error_one_failure_is_counted() {
        let d1 = dev("A", "Intel", "1.0.0.0", None, "PCI\\VEN_8086&DEV_1111&SUBSYS_1");
        let d2 = dev("B", "Intel", "1.0.0.0", None, "PCI\\VEN_8086&DEV_2222&SUBSYS_1");
        let fail = |_: &str| Err("boom".to_string());
        assert!(find_updates(&[d1.clone(), d2.clone()], &host(), 40, &fail, &no_live).unwrap_err().contains("Every Microsoft Catalog query failed"));
        let rows = parse_rows(&row(ID4, "Drivers (Networking)", "Windows 11", "2.0.0.0", (2025, 6, 1)));
        let half = |k: &str| if k.contains("1111") { Err("boom".to_string()) } else { Ok(rows.clone()) };
        let r = find_updates(&[d1, d2], &host(), 40, &half, &no_live).unwrap();
        assert_eq!((r.offers.len(), r.failed_queries), (1, 1));
    }

    #[test]
    fn query_limit_is_respected() {
        let calls = std::cell::Cell::new(0);
        let search = |_: &str| { calls.set(calls.get() + 1); Ok(vec![]) };
        let devs: Vec<_> = (0..10).map(|i| dev("D", "Intel", "1.0", None, &format!("PCI\\VEN_8086&DEV_{i:04X}&SUBSYS_1"))).collect();
        find_updates(&devs, &host(), 3, &search, &no_live).unwrap();
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn search_and_download_pages_go_through_the_http_trait() {
        let page = row(ID1, "Drivers (Networking)", "Windows 11", "2.0.0.0", (2025, 6, 1));
        let dl = "<script>downloadInformation[0].files[0].url = 'https://catalog.s.download.windowsupdate.com/d/x.cab'; downloadInformation[0].files[0].digest = 'AbCd=';</script>";
        let http = Fake::new(vec![("Search.aspx?q=PCI%5CVEN_8086%26DEV_1111", page.as_str()), ("DownloadDialog.aspx", dl)]);
        assert_eq!(search(&http, "PCI\\VEN_8086&DEV_1111").unwrap().len(), 1);
        assert_eq!(get_downloads(&http, ID1).unwrap(), vec![Download { url: "https://catalog.s.download.windowsupdate.com/d/x.cab".into(), digest: "AbCd=".into() }]);
    }
}

