//! Replacing the running program with a newer one. Windows lets a running .exe be renamed but not overwritten,
//! so: download -> verify against the signed manifest -> rename the current file to `.old` -> put the new file in its place -> restart.
use crate::crypto;
use crate::http::Http;
use crate::install::extract_zip_safe;
use crate::policy::{assert_allowed_url_with, UPDATE_HOSTS};
use crate::update::Manifest;
use std::fs;
use std::path::{Path, PathBuf};

/// The update zip carries exactly one program file.
pub const EXE_IN_ZIP: &str = "tiers-driver-manage.exe";

fn old_path(exe: &Path) -> PathBuf { let mut s = exe.as_os_str().to_owned(); s.push(".old"); PathBuf::from(s) }

/// Downloads and verifies the update, then swaps it in. The old program stays as `<exe>.old` until the next start.
/// Any failure leaves the current program exactly as it was.
pub fn apply(m: &Manifest, current_exe: &Path, work: &Path, http: &dyn Http, local_zip: Option<&Path>) -> Result<String, String> {
    assert_allowed_url_with(&m.url, UPDATE_HOSTS, false)?;
    fs::create_dir_all(work).map_err(|e| e.to_string())?;
    let zip = work.join("update.zip");
    match local_zip { Some(l) => { fs::copy(l, &zip).map_err(|e| e.to_string())?; } None => http.download(&m.url, &zip)? }
    crypto::check_sha256(&zip, &m.sha256)?;
    let new_dir = work.join("new");
    extract_zip_safe(&zip, &new_dir)?;
    let new_exe = new_dir.join(EXE_IN_ZIP);
    let bytes = fs::read(&new_exe).map_err(|_| format!("The update is missing {EXE_IN_ZIP}"))?;
    if bytes.len() < 4096 || &bytes[..2] != b"MZ" { return Err("The update program file is not a valid Windows program".into()); }
    let old = old_path(current_exe);
    let _ = fs::remove_file(&old);
    fs::rename(current_exe, &old).map_err(|e| format!("Could not move the current program aside: {e}"))?;
    if let Err(e) = fs::copy(&new_exe, current_exe) {
        let _ = fs::remove_file(current_exe);
        let _ = fs::rename(&old, current_exe); // put the old one back
        return Err(format!("Could not place the new program: {e}"));
    }
    Ok(format!("Updated to version {}", m.version))
}

/// Called at every start: removes the previous program left by an update.
pub fn cleanup_old(current_exe: &Path) { let _ = fs::remove_file(old_path(current_exe)); }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Fake;
    use std::io::Write;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tdm_su_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }
    fn exe_bytes(tag: u8) -> Vec<u8> { let mut v = vec![b'M', b'Z']; v.extend(std::iter::repeat(tag).take(8000)); v }
    fn zip_of(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        { let mut z = zip::ZipWriter::new(&mut buf); for (n, b) in files { z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap(); z.write_all(b).unwrap(); } z.finish().unwrap(); }
        buf.into_inner()
    }
    fn manifest(zip: &[u8]) -> Manifest { Manifest { version: "2.0.1".into(), url: "https://github.com/o/r/releases/download/v2.0.1/app.zip".into(), sha256: crypto::sha256_hex(zip), signature: String::new(), notes: String::new() } }

    #[test]
    fn good_update_swaps_the_program_and_keeps_the_old_one_aside() {
        let d = tmp("a"); let cur = d.join("tiers-driver-manage.exe"); fs::write(&cur, exe_bytes(1)).unwrap();
        let z = zip_of(&[(EXE_IN_ZIP, exe_bytes(2))]);
        let http = Fake::with_bytes(vec![("app.zip", z.clone())]);
        let msg = apply(&manifest(&z), &cur, &d.join("w"), &http, None).unwrap();
        assert_eq!(msg, "Updated to version 2.0.1");
        assert_eq!(fs::read(&cur).unwrap()[100], 2);
        assert_eq!(fs::read(old_path(&cur)).unwrap()[100], 1);
        cleanup_old(&cur);
        assert!(!old_path(&cur).exists());
    }
    #[test]
    fn wrong_hash_changes_nothing() {
        let d = tmp("b"); let cur = d.join("p.exe"); fs::write(&cur, exe_bytes(1)).unwrap();
        let z = zip_of(&[(EXE_IN_ZIP, exe_bytes(2))]);
        let http = Fake::with_bytes(vec![("app.zip", z.clone())]);
        let mut m = manifest(&z); m.sha256 = "0".repeat(64);
        assert!(apply(&m, &cur, &d.join("w"), &http, None).unwrap_err().contains("SHA-256 mismatch"));
        assert_eq!(fs::read(&cur).unwrap()[100], 1);
        assert!(!old_path(&cur).exists());
    }
    #[test]
    fn host_outside_the_list_is_refused() {
        let d = tmp("c"); let cur = d.join("p.exe"); fs::write(&cur, exe_bytes(1)).unwrap();
        let z = zip_of(&[(EXE_IN_ZIP, exe_bytes(2))]);
        let mut m = manifest(&z); m.url = "https://evil.example.com/app.zip".into();
        assert!(apply(&m, &cur, &d.join("w"), &Fake::with_bytes(vec![]), None).unwrap_err().contains("allowed list"));
    }
    #[test]
    fn zip_without_the_program_or_with_a_fake_one_is_refused() {
        let d = tmp("d"); let cur = d.join("p.exe"); fs::write(&cur, exe_bytes(1)).unwrap();
        let z1 = zip_of(&[("readme.txt", b"x".to_vec())]);
        assert!(apply(&manifest(&z1), &cur, &d.join("w1"), &Fake::with_bytes(vec![("app.zip", z1.clone())]), None).unwrap_err().contains("missing"));
        let z2 = zip_of(&[(EXE_IN_ZIP, b"not a program".to_vec())]);
        assert!(apply(&manifest(&z2), &cur, &d.join("w2"), &Fake::with_bytes(vec![("app.zip", z2.clone())]), None).unwrap_err().contains("not a valid Windows program"));
        assert_eq!(fs::read(&cur).unwrap()[100], 1);
    }
    #[test]
    fn zip_slip_is_refused() {
        let d = tmp("e"); let cur = d.join("p.exe"); fs::write(&cur, exe_bytes(1)).unwrap();
        let z = zip_of(&[("../evil.exe", exe_bytes(3))]);
        assert!(apply(&manifest(&z), &cur, &d.join("w"), &Fake::with_bytes(vec![("app.zip", z.clone())]), None).unwrap_err().contains("Unsafe path"));
    }
}
