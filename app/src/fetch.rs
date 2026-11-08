//! Backend auto-fetcher (runs on the GitHub runner, not on user PCs): turns a reported hardware ID into a catalog entry.
//! Only Microsoft Update Catalog packages are used: the cab must match Microsoft's own SHA-1, mention the hardware ID in its INF
//! files and carry Microsoft-signed catalog (.cat) files. The reporter chooses nothing except the hardware ID, so a malicious
//! report can at worst make the fetcher look up a driver that Microsoft itself publishes.
use crate::catalog::{wildcard_match, CatalogEntry, HostInfo};
use crate::http::Http;
use crate::policy::assert_allowed_url;
use crate::sys::Sys;
use crate::{crypto, mscatalog};
use serde::Deserialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Debug, Clone)]
pub struct Reported { pub ids: Vec<String>, #[serde(default)] pub class: String, #[serde(default)] pub name: String }

pub struct Env<'a> { pub http: &'a dyn Http, pub sys: &'a dyn Sys, pub host: HostInfo, pub repo: &'a str }

/// "PCI\VEN_8086&DEV_7A60&SUBSYS_1" -> "PCI\VEN_8086&DEV_7A60*"
pub fn pattern_of(hid: &str) -> Option<String> { mscatalog::hardware_key(hid).map(|k| format!("{k}*")) }

pub fn covered(entries: &[CatalogEntry], ids: &[String]) -> bool {
    entries.iter().any(|e| e.match_.iter().any(|p| ids.iter().any(|i| wildcard_match(p, i))))
}

fn slug(s: &str) -> String { s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().trim_matches('-').to_string() }

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = fs::read_dir(dir) { for e in rd.flatten() { let p = e.path(); if p.is_dir() { files_under(&p, out); } else { out.push(p); } } }
}

fn read_any(p: &Path) -> String {
    let b = fs::read(p).unwrap_or_default();
    if b.starts_with(&[0xFF, 0xFE]) { String::from_utf16_lossy(&b[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()) } else { String::from_utf8_lossy(&b).into_owned() }
}

fn zip_dir(dir: &Path, zip_path: &Path) -> Result<(), String> {
    let mut files = vec![];
    files_under(dir, &mut files);
    files.sort();
    let f = fs::File::create(zip_path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipWriter::new(f);
    let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for p in files {
        let rel = p.strip_prefix(dir).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
        z.start_file(rel, opt).map_err(|e| e.to_string())?;
        z.write_all(&fs::read(&p).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    z.finish().map_err(|e| e.to_string())?;
    Ok(())
}

pub enum Outcome { Added(Box<CatalogEntry>, PathBuf), Skipped(String) }

/// Resolves one reported device. Never panics on bad input; every refusal is a `Skipped` with the reason.
pub fn resolve(env: &Env, entries: &[CatalogEntry], item: &Reported, out_dir: &Path) -> Outcome {
    let skip = |why: String| Outcome::Skipped(format!("{}: {why}", item.ids.first().cloned().unwrap_or_default()));
    let mut keys: Vec<String> = vec![];
    for i in &item.ids { if let Some(k) = mscatalog::hardware_key(i) { if !keys.contains(&k) { keys.push(k); } } }
    if keys.is_empty() { return skip("not a PCI, USB or HD-Audio hardware ID".into()); }
    if covered(entries, &item.ids) { return skip("already in the catalog".into()); }
    let mut found = None;
    for k in &keys {
        match mscatalog::search(env.http, k) {
            Ok(rows) => if let Some(b) = mscatalog::select_best(&rows, &env.host) { found = Some((k.clone(), b)); break; },
            Err(e) => return skip(format!("Microsoft Catalog search failed: {e}")),
        }
    }
    let Some((key, best)) = found else { return skip("no driver in the Microsoft Update Catalog".into()) };
    let run = || -> Result<(CatalogEntry, PathBuf), String> {
        let downloads = mscatalog::get_downloads(env.http, &best.update_id)?;
        if downloads.is_empty() { return Err("no download link".into()); }
        let work = out_dir.join(format!("work-{}", slug(&key)));
        let _ = fs::remove_dir_all(&work);
        let pkg = work.join("pkg");
        fs::create_dir_all(&pkg).map_err(|e| e.to_string())?;
        for (n, d) in downloads.iter().enumerate() {
            assert_allowed_url(&d.url)?;
            let cab = work.join(format!("part{n}.cab"));
            env.http.download(&d.url, &cab)?;
            if crypto::sha1_base64_file(&cab).map_err(|e| e.to_string())? != d.digest { return Err("cab SHA-1 differs from the value Microsoft reports".into()); }
            let (code, _) = env.sys.run("expand.exe", &[&cab.to_string_lossy(), "-F:*", &pkg.to_string_lossy()]);
            if code != 0 { return Err("cab could not be extracted".into()); }
        }
        let mut all = vec![];
        files_under(&pkg, &mut all);
        let infs: Vec<_> = all.iter().filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("inf")).unwrap_or(false)).collect();
        if infs.is_empty() { return Err("the package has no INF file".into()); }
        let needle = key.to_lowercase();
        if !infs.iter().any(|p| read_any(p).to_lowercase().contains(&needle)) { return Err(format!("no INF in the package mentions {key}")); }
        let cats: Vec<_> = all.iter().filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("cat")).unwrap_or(false)).collect();
        if cats.is_empty() { return Err("the package has no signed catalog (.cat) file".into()); }
        for c in cats {
            let s = env.sys.signature(c);
            if s.status != "Valid" || !s.subject.to_lowercase().contains("microsoft") { return Err(format!("catalog {} is not Microsoft-signed ({})", c.file_name().unwrap_or_default().to_string_lossy(), s.status)); }
        }
        let file = format!("{}-{}.zip", slug(&key), slug(&best.version));
        let zip_path = out_dir.join(&file);
        zip_dir(&pkg, &zip_path)?;
        let _ = fs::remove_dir_all(&work);
        let class = if item.class.is_empty() { None } else { Some(item.class.clone()) };
        Ok((CatalogEntry {
            name: best.title.clone(), publisher: "Microsoft Update Catalog".into(), match_: vec![format!("{key}*")], version: best.version.clone(),
            date: best.date.map(|d| d.iso()), size_mb: Some((fs::metadata(&zip_path).map(|m| m.len()).unwrap_or(0) as f64 / 1_048_576.0 * 100.0).round() / 100.0),
            kind: "inf-zip".into(), url: format!("https://github.com/{}/releases/download/drivers/{file}", env.repo), sha256: crypto::sha256_file(&zip_path).map_err(|e| e.to_string())?,
            os: vec!["10".into(), "11".into()], arch: vec!["x64".into()], class, ..Default::default()
        }, zip_path))
    };
    match run() { Ok((e, z)) => Outcome::Added(Box::new(e), z), Err(e) => skip(e) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Fake;
    use crate::sys::fake::FakeSys;

    fn entry(m: &str) -> CatalogEntry { CatalogEntry { name: "x".into(), match_: vec![m.into()], ..Default::default() } }
    #[test]
    fn patterns_and_coverage() {
        assert_eq!(pattern_of("PCI\\VEN_10EC&DEV_8168&SUBSYS_1&REV_06").as_deref(), Some("PCI\\VEN_10EC&DEV_8168*"));
        assert_eq!(pattern_of("ACPI\\PNP0A08"), None);
        assert!(covered(&[entry("PCI\\VEN_10EC&DEV_8168*")], &["PCI\\VEN_10EC&DEV_8168&SUBSYS_1".into()]));
        assert!(!covered(&[entry("PCI\\VEN_10EC&DEV_8168*")], &["PCI\\VEN_8086&DEV_1234".into()]));
    }
    #[test]
    fn unusable_reports_are_skipped_without_network_calls() {
        let http = Fake::new(vec![]);
        let sys = FakeSys::ok();
        let env = Env { http: &http, sys: &sys, host: HostInfo { arch: "x64".into(), os: "11".into() }, repo: "o/r" };
        let out = std::env::temp_dir().join(format!("tdm_fetch_{}", std::process::id()));
        for ids in [vec!["ACPI\\PNP0A08".to_string()], vec!["PCI\\VEN_8086&DEV_1234&SUBSYS_1".to_string()]] {
            let r = resolve(&env, &[entry("PCI\\VEN_8086&DEV_1234*")], &Reported { ids, class: String::new(), name: String::new() }, &out);
            assert!(matches!(r, Outcome::Skipped(_)));
        }
        assert!(http.calls.borrow().is_empty(), "nothing is looked up for a device that is unusable or already covered");
    }
    #[test]
    fn zip_round_trip_keeps_the_package_files() {
        let d = std::env::temp_dir().join(format!("tdm_zip_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("pkg").join("sub")).unwrap();
        fs::write(d.join("pkg").join("a.inf"), "x").unwrap();
        fs::write(d.join("pkg").join("sub").join("b.cat"), "y").unwrap();
        zip_dir(&d.join("pkg"), &d.join("p.zip")).unwrap();
        crate::install::extract_zip_safe(&d.join("p.zip"), &d.join("out")).unwrap();
        assert!(d.join("out").join("a.inf").exists() && d.join("out").join("sub").join("b.cat").exists());
    }
}
