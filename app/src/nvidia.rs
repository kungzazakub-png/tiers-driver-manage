//! NVIDIA direct: asks NVIDIA's public lookup service for the newest driver for the exact GPU model.
//! Trust: the lookup host must be on the API list; the download itself is checked later (allowed host, signed installer).
use crate::catalog::{Date, HostInfo, InstalledDriver, Offer, OfferKind};
use crate::http::Http;
use crate::policy::{assert_allowed_url_with, API_HOSTS};
use crate::version::nvidia_version;
use regex::Regex;

pub const SIGNER: &str = "NVIDIA Corporation";
pub const SILENT_ARGS: &str = "-s -noreboot -noeula";
const SERIES_URL: &str = "https://www.nvidia.com/Download/API/lookupValueSearch.aspx?TypeID=2&ParentID=1";
const PRODUCTS_URL: &str = "https://www.nvidia.com/Download/API/lookupValueSearch.aspx?TypeID=3&ParentID=";

#[derive(Debug, PartialEq)]
pub struct Product { pub psid: String, pub pfid: String }

fn get_text(http: &dyn Http, url: &str) -> Result<String, String> {
    assert_allowed_url_with(url, API_HOSTS, false)?;
    Ok(String::from_utf8_lossy(&http.get(url)?).into_owned())
}

/// `<LookupValue ...><Name>x</Name><Value>1</Value>` pairs.
fn lookup_values(xml: &str) -> Vec<(String, String)> {
    Regex::new(r"(?s)<Name>(.*?)</Name>\s*<Value>(\d+)</Value>").unwrap().captures_iter(xml).map(|c| (c[1].to_string(), c[2].to_string())).collect()
}

/// "NVIDIA GeForce RTX 3060" -> series "GeForce RTX 30 Series" -> product id. Laptop GPUs only match notebook series.
pub fn find_product(device_name: &str, http: &dyn Http) -> Result<Option<Product>, String> {
    let name = device_name.trim_start_matches("NVIDIA ").trim().to_string();
    let Some(m) = Regex::new(r"(?:RTX|GTX|GT)\s*(\d{3,4})").unwrap().captures(&name) else { return Ok(None) };
    let num = &m[1];
    let gen = if num.len() == 4 { num[..2].to_string() } else { format!("{}00", &num[..1]) };
    let laptop = Regex::new(r"Laptop|Max-Q|Mobile").unwrap().is_match(&name);
    let series_re = Regex::new(&format!(r"\b{gen}\s+Series")).unwrap();
    for (sname, sid) in lookup_values(&get_text(http, SERIES_URL)?) {
        if !series_re.is_match(&sname) || sname.contains("Notebook") != laptop { continue; }
        for (pname, pid) in lookup_values(&get_text(http, &format!("{PRODUCTS_URL}{sid}"))?) {
            if pname.eq_ignore_ascii_case(&name) { return Ok(Some(Product { psid: sid, pfid: pid })); }
        }
    }
    Ok(None)
}

fn parse_release_date(s: &str) -> Option<Date> {
    // "Wed Nov 04, 2026"
    let c = Regex::new(r"([A-Za-z]{3}) (\d{1,2}), (\d{4})").unwrap().captures(s)?;
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let m = months.iter().position(|x| x.eq_ignore_ascii_case(&c[1]))? as u32 + 1;
    Some(Date { y: c[3].parse().ok()?, m, d: c[2].parse().ok()? })
}

fn parse_size_mb(s: &str) -> Option<f64> {
    let c = Regex::new(r"([\d\.]+)\s*(MB|GB)").unwrap().captures(s)?;
    let n: f64 = c[1].parse().ok()?;
    Some((n * if &c[2] == "GB" { 1024.0 } else { 1.0 } * 10.0).round() / 10.0)
}

pub fn find_updates(installed: &[InstalledDriver], host: &HostInfo, http: &dyn Http, live_version: &dyn Fn(&str) -> Option<String>) -> Result<Vec<Offer>, String> {
    let mut found = Vec::new();
    if host.arch != "x64" { return Ok(found); }
    for d in installed.iter().filter(|d| d.class.eq_ignore_ascii_case("DISPLAY") && d.maker.to_ascii_lowercase().starts_with("nvidia") && d.name.contains("GeForce")) {
        let mut cur = nvidia_version(&d.version);
        if let Some(l) = live_version(&d.instance_id).and_then(|v| nvidia_version(&v)) { cur = Some(l); }
        let Some(cur) = cur else { continue };
        let Some(prod) = find_product(&d.name, http)? else { continue };
        let os = if host.os == "11" { 135 } else { 57 };
        let url = format!("https://gfwsl.geforce.com/services_toolkit/services/com/nvidia/services/AjaxDriverService.php?func=DriverManualLookup&psid={}&pfid={}&osID={os}&languageCode=1033&beta=0&isWHQL=0&dltype=-1&dch=1&upCRD=0&qnf=0&sort1=0&numberOfResults=1", prod.psid, prod.pfid);
        let json: serde_json::Value = serde_json::from_str(&get_text(http, &url)?).map_err(|e| e.to_string())?;
        let info = &json["IDS"][0]["downloadInfo"];
        let (Some(dl), Some(ver)) = (info["DownloadURL"].as_str(), info["Version"].as_str()) else { continue };
        let Ok(new) = ver.trim().parse::<f64>() else { continue };
        if new <= cur { continue; }
        found.push(Offer {
            id: format!("ven:nvidia:{ver}"), title: format!("NVIDIA GeForce Game Ready Driver {ver} [now {cur:.2}]"), class: "DISPLAY".into(), maker: "NVIDIA".into(),
            date: info["ReleaseDateTime"].as_str().and_then(parse_release_date).map(|x| x.iso()).unwrap_or_default(),
            size_mb: info["DownloadURLFileSize"].as_str().and_then(parse_size_mb), source: "NVIDIA".into(), instance_id: d.instance_id.clone(), device_name: d.name.clone(),
            kind: OfferKind::Nvidia { url: dl.to_string(), version: ver.to_string(), signer: SIGNER.into(), silent_args: SILENT_ARGS.into() },
        });
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Fake;

    fn host() -> HostInfo { HostInfo { arch: "x64".into(), os: "11".into() } }
    fn gpu(name: &str, ver: &str) -> InstalledDriver {
        InstalledDriver { instance_id: "x".into(), name: name.into(), class: "DISPLAY".into(), maker: "NVIDIA".into(), version: ver.into(), date: None, hardware_ids: vec![] }
    }
    fn no_live(_: &str) -> Option<String> { None }
    fn server() -> Fake {
        Fake::new(vec![
            ("TypeID=2", "<LookupValueSearch><LookupValues><LookupValue><Name>GeForce RTX 30 Series</Name><Value>120</Value></LookupValue><LookupValue><Name>GeForce RTX 30 Series (Notebooks)</Name><Value>123</Value></LookupValue></LookupValues></LookupValueSearch>"),
            ("ParentID=120", "<LookupValueSearch><LookupValues><LookupValue><Name>GeForce RTX 3060</Name><Value>933</Value></LookupValue><LookupValue><Name>GeForce RTX 3060 Ti</Name><Value>934</Value></LookupValue></LookupValues></LookupValueSearch>"),
            ("psid=120&pfid=933", r#"{ "Success":"1", "IDS":[{ "downloadInfo": { "Version":"700.01", "DownloadURL":"https://us.download.nvidia.com/Windows/700.01/x.exe", "DownloadURLFileSize":"1.04 GB", "ReleaseDateTime":"Wed Nov 04, 2026" } }] }"#),
        ])
    }

    #[test]
    fn newer_driver_is_offered_for_the_exact_model_not_the_ti() {
        let o = find_updates(&[gpu("NVIDIA GeForce RTX 3060", "32.0.16.2114")], &host(), &server(), &no_live).unwrap();
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].id, "ven:nvidia:700.01");
        assert_eq!(o[0].size_mb, Some(1065.0));
        assert_eq!(o[0].date, "2026-11-04");
        assert!(o[0].title.contains("[now 621.14]"));
    }
    #[test]
    fn same_or_newer_installed_driver_is_not_offered() {
        assert!(find_updates(&[gpu("NVIDIA GeForce RTX 3060", "32.0.17.0001")], &host(), &server(), &no_live).unwrap().is_empty());
    }
    #[test]
    fn laptop_gpu_is_matched_against_the_notebook_series_only() {
        // only the notebook series would be queried for products; the fake has no route for ParentID=123, so a wrong query would error
        let h = Fake::new(vec![("TypeID=2", "<LookupValueSearch><LookupValues><LookupValue><Name>GeForce RTX 30 Series</Name><Value>120</Value></LookupValue></LookupValues></LookupValueSearch>")]);
        assert!(find_updates(&[gpu("NVIDIA GeForce RTX 3060 Laptop GPU", "32.0.15.1000")], &host(), &h, &no_live).unwrap().is_empty());
    }
    #[test]
    fn non_x64_pc_gets_no_offer() {
        let arm = HostInfo { arch: "arm64".into(), os: "11".into() };
        assert!(find_updates(&[gpu("NVIDIA GeForce RTX 3060", "32.0.16.2114")], &arm, &server(), &no_live).unwrap().is_empty());
    }
    #[test]
    fn live_version_suppresses_an_offer() {
        let live = |_: &str| Some("32.0.17.0001".to_string());
        assert!(find_updates(&[gpu("NVIDIA GeForce RTX 3060", "32.0.16.2114")], &host(), &server(), &live).unwrap().is_empty());
    }
    #[test]
    fn non_geforce_and_non_nvidia_devices_are_ignored() {
        let mut quadro = gpu("NVIDIA Quadro P2000", "32.0.16.2114");
        assert!(find_updates(&[quadro.clone()], &host(), &server(), &no_live).unwrap().is_empty());
        quadro.name = "NVIDIA GeForce RTX 3060".into(); quadro.maker = "Intel".into();
        assert!(find_updates(&[quadro], &host(), &server(), &no_live).unwrap().is_empty());
    }
}
