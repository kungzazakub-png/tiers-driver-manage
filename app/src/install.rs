//! Download, verify and install one driver. Every rule here is a safety check: https + allowed host, pinned hash, publisher signature,
//! the package must mention the device's hardware ID, and zip files must not write outside their folder.
use crate::catalog::CatalogEntry;
use crate::crypto;
use crate::http::Http;
use crate::mscatalog;
use crate::policy::assert_allowed_url;
use crate::sys::Sys;
use crate::version::nvidia_version;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct InstallResult { pub success: bool, pub reboot: bool, pub message: String }

impl InstallResult {
    fn fail(msg: impl Into<String>) -> Self { InstallResult { success: false, reboot: false, message: msg.into() } }
}

fn file_name_of(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let name = path.rsplit('/').next().unwrap_or("");
    if name.is_empty() { "download.bin".into() } else { name.to_string() }
}

/// Extracts a zip; any entry that would land outside `dest` rejects the whole archive.
pub fn extract_zip_safe(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(fs::File::open(zip_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = f.enclosed_name() else { return Err(format!("Unsafe path found in zip file: {}", f.name())) };
        let target: PathBuf = dest.join(rel);
        if f.is_dir() { fs::create_dir_all(&target).map_err(|e| e.to_string())?; continue; }
        if let Some(p) = target.parent() { fs::create_dir_all(p).map_err(|e| e.to_string())?; }
        let mut out = fs::File::create(&target).map_err(|e| e.to_string())?;
        std::io::copy(&mut f, &mut out).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn has_inf(dir: &Path) -> bool {
    fn walk(d: &Path) -> bool {
        fs::read_dir(d).map(|rd| rd.flatten().any(|e| { let p = e.path(); if p.is_dir() { walk(&p) } else { p.extension().map(|x| x.eq_ignore_ascii_case("inf")).unwrap_or(false) } })).unwrap_or(false)
    }
    walk(dir)
}

fn inf_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { inf_files(&p, out); } else if p.extension().map(|x| x.eq_ignore_ascii_case("inf")).unwrap_or(false) { out.push(p); }
        }
    }
}

/// INF files are often UTF-16: decode by BOM like PowerShell's Select-String does.
fn read_text_any(p: &Path) -> String {
    let b = fs::read(p).unwrap_or_default();
    if b.starts_with(&[0xFF, 0xFE]) { String::from_utf16_lossy(&b[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()) }
    else if b.starts_with(&[0xFE, 0xFF]) { String::from_utf16_lossy(&b[2..].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>()) }
    else { String::from_utf8_lossy(&b).into_owned() }
}

fn pnputil_install(pkg: &Path, sys: &dyn Sys) -> InstallResult {
    let pattern = pkg.join("*.inf");
    let (code, out) = sys.run("pnputil.exe", &["/add-driver", &pattern.to_string_lossy(), "/subdirs", "/install"]);
    judge_pnputil(code, &out)
}

fn judge_pnputil(code: i32, out: &str) -> InstallResult {
    let low = out.to_lowercase();
    // pnputil exits non-zero when no device needed an update: the package is already in the store and the hardware already uses it
    if code != 0 && code != 3010 && low.contains("driver package is up-to-date on device") { return InstallResult { success: true, reboot: false, message: "Already up to date (this driver is already installed)".into() }; }
    InstallResult { success: code == 0 || code == 3010, reboot: code == 3010 || low.contains("reboot") || low.contains("restart"), message: out.trim().to_string() }
}

fn prepare_work(work: &Path, sys: &dyn Sys) -> Result<(), String> {
    fs::create_dir_all(work).map_err(|e| e.to_string())?;
    sys.lock_down(work);
    Ok(())
}

/// A driver from the catalog: `inf-zip` (a zip of INF packages) or `installer` (a signed vendor setup run silently).
/// `local` is for tests: use an already-downloaded file instead of fetching the URL (the URL is still checked).
pub fn install_catalog_item(c: &CatalogEntry, work: &Path, http: &dyn Http, sys: &dyn Sys, local: Option<&Path>) -> InstallResult {
    let run = || -> Result<InstallResult, String> {
        assert_allowed_url(&c.url)?;
        prepare_work(work, sys)?;
        let file = work.join(file_name_of(&c.url));
        match local { Some(l) => { fs::copy(l, &file).map_err(|e| e.to_string())?; } None => http.download(&c.url, &file)? }
        crypto::check_sha256(&file, &c.sha256)?;
        match c.kind.as_str() {
            "inf-zip" => {
                let pkg = work.join("pkg");
                extract_zip_safe(&file, &pkg)?;
                Ok(pnputil_install(&pkg, sys))
            }
            "installer" => {
                let signer = c.signer.as_deref().filter(|s| !s.is_empty()).ok_or("An installer entry must specify a signer (publisher name) to be installed")?;
                let args = c.silent_args.as_deref().filter(|s| !s.is_empty()).ok_or("An installer entry must specify silentArgs")?;
                let sig = sys.signature(&file);
                if sig.status != "Valid" { return Err(format!("The installer's digital signature is not valid ({})", sig.status)); }
                if !sig.subject.to_lowercase().contains(&signer.to_lowercase()) { return Err(format!("Signer does not match the expected publisher: {}", sig.subject)); }
                let code = sys.run_installer(&file, args)?;
                Ok(InstallResult { success: code == 0 || code == 3010, reboot: code == 3010, message: format!("exit code {code}") })
            }
            other => Err(format!("Unknown type: {other} (use inf-zip or installer)")),
        }
    };
    run().unwrap_or_else(InstallResult::fail)
}

/// A driver found through the Microsoft Update Catalog: cabs verified against Microsoft's SHA-1, extracted, checked for the hardware ID.
pub fn install_ms_item(update_id: &str, key: &str, work: &Path, http: &dyn Http, sys: &dyn Sys, local_cab: Option<&Path>) -> InstallResult {
    let run = || -> Result<InstallResult, String> {
        let files = mscatalog::get_downloads(http, update_id)?;
        if files.is_empty() { return Err("No download link found in the Microsoft catalog".into()); }
        prepare_work(work, sys)?;
        let pkg = work.join("pkg");
        fs::create_dir_all(&pkg).map_err(|e| e.to_string())?;
        for (i, f) in files.iter().enumerate() {
            assert_allowed_url(&f.url)?;
            let cab = work.join(format!("part{i}.cab"));
            match local_cab { Some(l) => { fs::copy(l, &cab).map_err(|e| e.to_string())?; } None => http.download(&f.url, &cab)? }
            if crypto::sha1_base64_file(&cab).map_err(|e| e.to_string())? != f.digest { return Err("File SHA-1 does not match the value reported by the Microsoft catalog".into()); }
            let (code, _) = sys.run("expand.exe", &[&cab.to_string_lossy(), "-F:*", &pkg.to_string_lossy()]);
            if code != 0 { return Err("Failed to extract the cab file".into()); }
        }
        // never install a package that does not mention this device's hardware ID
        let mut infs = Vec::new();
        inf_files(&pkg, &mut infs);
        let needle = key.to_lowercase();
        if !infs.iter().any(|p| read_text_any(p).to_lowercase().contains(&needle)) { return Err(format!("This package does not contain hardware {key}, so it will not be installed")); }
        if !has_inf(&pkg) { return Err("The package has no INF files".into()); }
        Ok(pnputil_install(&pkg, sys))
    };
    run().unwrap_or_else(InstallResult::fail)
}

/// A vendor-direct driver (NVIDIA): no hash is published, so the vendor's signature is the check.
pub fn install_vendor_item(url: &str, version: &str, signer: &str, silent_args: &str, instance_id: &str, work: &Path, http: &dyn Http, sys: &dyn Sys, local: Option<&Path>) -> InstallResult {
    let run = || -> Result<InstallResult, String> {
        assert_allowed_url(url)?;
        prepare_work(work, sys)?;
        let file = work.join(file_name_of(url));
        match local { Some(l) => { fs::copy(l, &file).map_err(|e| e.to_string())?; } None => http.download(url, &file)? }
        let sig = sys.signature(&file);
        if sig.status != "Valid" { return Err(format!("The installer's digital signature is not valid ({}), so it will not be installed", sig.status)); }
        if !sig.subject.to_lowercase().contains(&signer.to_lowercase()) { return Err(format!("Signer does not match {signer}: {}", sig.subject)); }
        let code = sys.run_installer(&file, silent_args)?;
        let mut res = InstallResult { success: code == 0 || code == 3010, reboot: code == 3010, message: format!("exit code {code}") };
        if !res.success && !instance_id.is_empty() {
            // the installer may report "reboot needed" with another code: trust the driver Windows now reports
            let now = sys.live_driver_version(instance_id).and_then(|v| nvidia_version(&v));
            let want: Option<f64> = version.trim().parse().ok();
            if let (Some(now), Some(want)) = (now, want) { if now >= want { res.success = true; res.reboot = true; } }
        }
        Ok(res)
    };
    run().unwrap_or_else(InstallResult::fail)
}

#[allow(dead_code)]
fn read_to_string_lossy(p: &Path) -> String { let mut s = String::new(); let _ = fs::File::open(p).and_then(|mut f| f.read_to_string(&mut s)); s }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Fake;
    use crate::sys::fake::FakeSys;
    use crate::trust::SignatureInfo;
    use std::io::Write;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tdm_inst_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }
    fn zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            for (n, b) in files { z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap(); z.write_all(b).unwrap(); }
            z.finish().unwrap();
        }
        buf.into_inner()
    }
    fn entry(kind: &str, url: &str, sha: &str) -> CatalogEntry {
        CatalogEntry { name: "T".into(), kind: kind.into(), url: url.into(), sha256: sha.into(), signer: Some("Test Vendor".into()), silent_args: Some("/s".into()), version: "1.0".into(), match_: vec!["x".into()], ..Default::default() }
    }
    const URL: &str = "https://us.download.nvidia.com/x/setup.exe";

    #[test]
    fn wrong_sha256_stops_before_anything_runs() {
        let w = tmp("a"); let sys = FakeSys::ok();
        let http = Fake::with_bytes(vec![("setup.exe", b"data".to_vec())]);
        let r = install_catalog_item(&entry("installer", URL, &"0".repeat(64)), &w, &http, &sys, None);
        assert!(!r.success && r.message.contains("SHA-256 mismatch"));
        assert!(!sys.calls().iter().any(|c| c.starts_with("installer")));
    }
    #[test]
    fn right_hash_and_signature_runs_the_installer_silently() {
        let w = tmp("b"); let sys = FakeSys::ok();
        let http = Fake::with_bytes(vec![("setup.exe", b"data".to_vec())]);
        let r = install_catalog_item(&entry("installer", URL, &crypto::sha256_hex(b"data")), &w, &http, &sys, None);
        assert!(r.success, "{}", r.message);
        assert_eq!(sys.calls().last().unwrap(), "installer setup.exe /s");
    }
    #[test]
    fn unsigned_file_is_refused_even_with_the_right_hash() {
        let w = tmp("c"); let mut sys = FakeSys::ok();
        sys.signature = SignatureInfo { status: "NotSigned".into(), subject: String::new() };
        let http = Fake::with_bytes(vec![("setup.exe", b"data".to_vec())]);
        let r = install_catalog_item(&entry("installer", URL, &crypto::sha256_hex(b"data")), &w, &http, &sys, None);
        assert!(!r.success && r.message.contains("signature is not valid"));
    }
    #[test]
    fn installer_entry_needs_signer_and_silent_args() {
        let w = tmp("d"); let sys = FakeSys::ok();
        let http = Fake::with_bytes(vec![("setup.exe", b"data".to_vec())]);
        let mut e = entry("installer", URL, &crypto::sha256_hex(b"data")); e.signer = None;
        assert!(install_catalog_item(&e, &w, &http, &sys, None).message.contains("must specify a signer"));
        let mut e2 = entry("installer", URL, &crypto::sha256_hex(b"data")); e2.silent_args = None;
        assert!(install_catalog_item(&e2, &w, &http, &sys, None).message.contains("silentArgs"));
    }
    #[test]
    fn signer_of_another_publisher_is_refused() {
        let w = tmp("e"); let mut sys = FakeSys::ok();
        sys.signature.subject = "CN=Somebody Else".into();
        let http = Fake::with_bytes(vec![("setup.exe", b"data".to_vec())]);
        assert!(install_catalog_item(&entry("installer", URL, &crypto::sha256_hex(b"data")), &w, &http, &sys, None).message.contains("Signer does not match"));
    }
    #[test]
    fn host_outside_the_list_is_refused() {
        let w = tmp("f"); let sys = FakeSys::ok(); let http = Fake::with_bytes(vec![]);
        let r = install_catalog_item(&entry("installer", "https://evil.example.com/setup.exe", "A"), &w, &http, &sys, None);
        assert!(r.message.contains("allowed list"));
    }
    #[test]
    fn already_up_to_date_is_not_a_failure() {
        let out = "Adding driver package: rt640x64.inf
Driver package added successfully. (Already exists in the system)
Published Name: oem24.inf
Driver package is up-to-date on device: PCI\VEN_10EC
Total driver packages: 1
Added driver packages: 0";
        let r = judge_pnputil(259, out);
        assert!(r.success && !r.reboot && r.message.contains("Already up to date"));
        assert!(!judge_pnputil(1, "Failed to add driver package").success);
    }
    #[test]
    fn safe_zip_is_installed_with_pnputil() {
        let w = tmp("g"); let sys = FakeSys::ok();
        let z = zip_bytes(&[("drv/a.inf", b";inf"), ("drv/a.sys", b"x")]);
        let http = Fake::with_bytes(vec![("pkg.zip", z.clone())]);
        let r = install_catalog_item(&entry("inf-zip", "https://downloadmirror.intel.com/1/pkg.zip", &crypto::sha256_hex(&z)), &w, &http, &sys, None);
        assert!(r.success, "{}", r.message);
        assert!(sys.calls().iter().any(|c| c.contains("/add-driver") && c.contains("/subdirs /install")));
        assert!(w.join("pkg/drv/a.inf").exists());
    }
    #[test]
    fn zip_with_path_traversal_is_rejected() {
        let w = tmp("h"); let sys = FakeSys::ok();
        let z = zip_bytes(&[("../evil.txt", b"x")]);
        let http = Fake::with_bytes(vec![("pkg.zip", z.clone())]);
        let r = install_catalog_item(&entry("inf-zip", "https://downloadmirror.intel.com/1/pkg.zip", &crypto::sha256_hex(&z)), &w, &http, &sys, None);
        assert!(!r.success && r.message.contains("Unsafe path"), "{}", r.message);
        assert!(!w.parent().unwrap().join("evil.txt").exists());
    }
    #[test]
    fn reboot_exit_codes_are_reported() {
        let w = tmp("i"); let mut sys = FakeSys::ok(); sys.installer_exit = 3010;
        let http = Fake::with_bytes(vec![("setup.exe", b"data".to_vec())]);
        let r = install_catalog_item(&entry("installer", URL, &crypto::sha256_hex(b"data")), &w, &http, &sys, None);
        assert!(r.success && r.reboot);
    }

    // ---- Microsoft Catalog ----
    fn ms_http(cab: &[u8], digest: &str, host: &str) -> Fake {
        let page = format!("<script>x.url = 'https://{host}/d/x.cab'; x.digest = '{digest}';</script>");
        Fake::with_bytes(vec![("DownloadDialog.aspx", page.into_bytes()), ("x.cab", cab.to_vec())])
    }
    fn sha1_b64(data: &[u8]) -> String {
        use base64::{engine::general_purpose::STANDARD as B64, Engine};
        use sha1::{Digest, Sha1};
        B64.encode(Sha1::digest(data))
    }
    #[test]
    fn ms_package_with_the_right_digest_and_hardware_is_installed() {
        let w = tmp("j"); let sys = FakeSys::ok();
        let r = install_ms_item("id", "PCI\\VEN_8086&DEV_1234", &w, &ms_http(b"cab", &sha1_b64(b"cab"), "catalog.s.download.windowsupdate.com"), &sys, None);
        assert!(r.success, "{}", r.message);
        assert!(sys.calls().iter().any(|c| c.starts_with("expand.exe")) && sys.calls().iter().any(|c| c.contains("/add-driver")));
    }
    #[test]
    fn ms_wrong_digest_refused() {
        let w = tmp("k"); let sys = FakeSys::ok();
        let r = install_ms_item("id", "PCI\\VEN_8086&DEV_1234", &w, &ms_http(b"cab", "AAAA", "catalog.s.download.windowsupdate.com"), &sys, None);
        assert!(r.message.contains("SHA-1"));
    }
    #[test]
    fn ms_host_outside_the_list_refused() {
        let w = tmp("l"); let sys = FakeSys::ok();
        let r = install_ms_item("id", "PCI\\VEN_8086&DEV_1234", &w, &ms_http(b"cab", &sha1_b64(b"cab"), "evil.example.com"), &sys, None);
        assert!(r.message.contains("allowed list"));
    }
    #[test]
    fn ms_package_without_this_hardware_id_is_refused() {
        let w = tmp("m"); let sys = FakeSys::ok();
        let r = install_ms_item("id", "PCI\\VEN_FFFF&DEV_FFFF", &w, &ms_http(b"cab", &sha1_b64(b"cab"), "catalog.s.download.windowsupdate.com"), &sys, None);
        assert!(r.message.contains("does not contain hardware"));
        assert!(!sys.calls().iter().any(|c| c.contains("/add-driver")));
    }

    // ---- vendor direct ----
    #[test]
    fn vendor_unsigned_wrong_maker_and_wrong_host_are_refused() {
        let w = tmp("n"); let http = Fake::with_bytes(vec![("setup.exe", b"d".to_vec())]);
        let mut sys = FakeSys::ok(); sys.signature = SignatureInfo { status: "NotSigned".into(), subject: String::new() };
        assert!(install_vendor_item(URL, "700.01", "NVIDIA Corporation", "-s", "", &w, &http, &sys, None).message.contains("signature is not valid"));
        let mut sys2 = FakeSys::ok(); sys2.signature.subject = "CN=Microsoft".into();
        assert!(install_vendor_item(URL, "700.01", "NVIDIA Corporation", "-s", "", &w, &http, &sys2, None).message.contains("Signer does not match"));
        assert!(install_vendor_item("https://evil.example.com/setup.exe", "1", "NVIDIA", "-s", "", &w, &http, &FakeSys::ok(), None).message.contains("allowed list"));
    }
    #[test]
    fn vendor_unusual_exit_code_still_counts_when_windows_reports_the_new_driver() {
        let w = tmp("o"); let http = Fake::with_bytes(vec![("setup.exe", b"d".to_vec())]);
        let mut sys = FakeSys::ok(); sys.signature.subject = "CN=NVIDIA Corporation".into(); sys.installer_exit = 1; sys.live = Some("32.0.17.0001".into());
        let r = install_vendor_item(URL, "700.01", "NVIDIA Corporation", "-s", "dev", &w, &http, &sys, None);
        assert!(r.success && r.reboot);
        sys.live = Some("32.0.16.2114".into());
        assert!(!install_vendor_item(URL, "700.01", "NVIDIA Corporation", "-s", "dev", &w, &http, &sys, None).success);
    }
}
