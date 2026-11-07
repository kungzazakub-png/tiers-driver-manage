//! The program's signed update manifest and online driver catalog (both published on the publisher's GitHub releases).
//! Trust: a manifest is signed with the publisher's private key; the program only holds the PUBLIC key. The signature covers
//! version + file hash + url, so a tampered release cannot be installed. An online catalog can add drivers but can never widen the host list.
use crate::catalog::{check_entry, parse_catalog, CatalogEntry, CatalogFile};
use crate::crypto::{sha256_hex, verify_manifest};
use crate::http::Http;
use crate::policy::{assert_allowed_url_with, UPDATE_HOSTS};
use crate::version::Ver;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub signature: String,
    #[serde(default)] pub notes: String,
}

fn repo_ok(repo: &str) -> bool {
    let mut it = repo.split('/');
    let (Some(o), Some(n), None) = (it.next(), it.next(), it.next()) else { return false };
    let ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
    ok(o) && ok(n)
}

fn parse_manifest(bytes: &[u8]) -> Result<Manifest, String> {
    serde_json::from_str(String::from_utf8_lossy(bytes).trim_start_matches('\u{feff}')).map_err(|e| e.to_string())
}

fn check_signature(m: &Manifest, key_xml: &str, what: &str) -> Result<(), String> {
    if !verify_manifest(&m.version, &m.sha256, &m.url, &m.signature, key_xml) { return Err(format!("The {what} signature is not valid, so it will not be used (the file may have been tampered with)")); }
    if m.sha256.len() != 64 || !m.sha256.chars().all(|c| c.is_ascii_hexdigit()) { return Err(format!("The sha256 value in the {what} is not valid")); }
    assert_allowed_url_with(&m.url, UPDATE_HOSTS, false)
}

/// The manifest when a NEWER, correctly signed version exists; `None` when up to date or updates are not configured.
/// Errors when a manifest exists but is not trustworthy.
pub fn get_app_update(repo: &str, current: &str, key_xml: &str, http: &dyn Http) -> Result<Option<Manifest>, String> {
    if key_xml.is_empty() || !repo_ok(repo) { return Ok(None); }
    let url = format!("https://github.com/{repo}/releases/latest/download/latest.json");
    assert_allowed_url_with(&url, UPDATE_HOSTS, false)?;
    let m = parse_manifest(&http.get(&url)?)?;
    let new_v = Ver::parse(&m.version).ok_or("The version format in the update file is not valid")?;
    if let Some(cur) = Ver::parse(current) { if new_v <= cur { return Ok(None); } }
    check_signature(&m, key_xml, "update file")?;
    Ok(Some(m))
}

#[derive(Debug, PartialEq)]
pub struct OnlineResult { pub updated: bool, pub revision: i64, pub count: usize }

fn cached_revision(cache: &Path) -> i64 {
    std::fs::read_to_string(cache).ok().and_then(|t| serde_json::from_str::<CatalogFile>(t.trim_start_matches('\u{feff}')).ok()).map(|f| f.revision).unwrap_or(0)
}

/// Fetches the signed online catalog into `cache` when a newer revision exists. The old cache stays untouched on any failure.
pub fn get_online_catalog(repo: &str, key_xml: &str, cache: &Path, http: &dyn Http) -> Result<Option<OnlineResult>, String> {
    if key_xml.is_empty() || !repo_ok(repo) { return Ok(None); }
    let m = parse_manifest(&http.get(&format!("https://github.com/{repo}/releases/download/catalog/catalog.json"))?)?;
    let rev: i64 = m.version.trim_start_matches("1.").parse().map_err(|_| "The online catalog revision is not valid".to_string())?;
    let cur = cached_revision(cache);
    if rev <= cur { return Ok(Some(OnlineResult { updated: false, revision: cur, count: 0 })); }
    check_signature(&m, key_xml, "online catalog")?;
    let bytes = http.get(&m.url)?;
    if sha256_hex(&bytes) != m.sha256.to_ascii_uppercase() { return Err("The online catalog SHA-256 does not match, so it will not be used".into()); }
    let entries = parse_catalog(&String::from_utf8_lossy(&bytes))?;
    for e in &entries {
        if let Some(why) = check_entry(e) { return Err(format!("Entry '{}' in the online catalog failed validation: {why}", e.name)); }
    }
    let file = CatalogFile { revision: rev, drivers: entries.clone() };
    let tmp = cache.with_extension("json.new");
    if let Some(dir) = cache.parent() { std::fs::create_dir_all(dir).map_err(|e| e.to_string())?; }
    std::fs::write(&tmp, serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, cache).map_err(|e| e.to_string())?;
    Ok(Some(OnlineResult { updated: true, revision: rev, count: entries.len() }))
}

/// Local file first, then the online cache; an online entry replaces a local one with the same name (the online one is the maintained one).
pub fn read_all_catalogs(local: &Path, online: &Path) -> Vec<CatalogEntry> {
    let mut out: Vec<CatalogEntry> = Vec::new();
    for p in [local, online] {
        let Ok(text) = std::fs::read_to_string(p) else { continue };
        let Ok(list) = parse_catalog(&text) else { continue };
        for c in list {
            match out.iter().position(|x| x.name == c.name) { Some(i) => out[i] = c, None => out.push(c) }
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod testkit {
    use super::*;
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    use rsa::pkcs1v15::SigningKey;
    use rsa::signature::{RandomizedSigner, SignatureEncoding};
    use rsa::traits::PublicKeyParts;
    use rsa::RsaPrivateKey;
    use sha2::Sha256;

    pub struct Keys { pub private: RsaPrivateKey, pub public_xml: String }
    pub fn keys() -> Keys {
        let private = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
        let p = private.to_public_key();
        let public_xml = format!("<RSAKeyValue><Modulus>{}</Modulus><Exponent>{}</Exponent></RSAKeyValue>", B64.encode(p.n().to_bytes_be()), B64.encode(p.e().to_bytes_be()));
        Keys { private, public_xml }
    }
    pub fn sign(k: &RsaPrivateKey, version: &str, sha256: &str, url: &str) -> String {
        let sk = SigningKey::<Sha256>::new(k.clone());
        B64.encode(sk.sign_with_rng(&mut rsa::rand_core::OsRng, crate::crypto::signed_payload(version, sha256, url).as_bytes()).to_bytes())
    }
    pub fn manifest_json(k: &RsaPrivateKey, version: &str, sha256: &str, url: &str) -> String {
        serde_json::json!({ "version": version, "url": url, "sha256": sha256, "signature": sign(k, version, sha256, url) }).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;
    use crate::http::Fake;

    const ZIP_URL: &str = "https://github.com/o/r/releases/download/v/app.zip";

    #[test]
    fn newer_signed_version_is_offered_same_or_older_is_not() {
        let k = keys();
        let http = Fake::new(vec![("latest.json", &manifest_json(&k.private, "2.0.0", &"A".repeat(64), ZIP_URL))]);
        assert_eq!(get_app_update("o/r", "1.0.0", &k.public_xml, &http).unwrap().unwrap().version, "2.0.0");
        assert!(get_app_update("o/r", "2.0.0", &k.public_xml, &http).unwrap().is_none());
        assert!(get_app_update("o/r", "3.0.0", &k.public_xml, &http).unwrap().is_none());
    }
    #[test]
    fn no_repo_or_no_key_means_no_check() {
        let k = keys();
        let http = Fake::new(vec![]);
        assert!(get_app_update("", "1.0.0", &k.public_xml, &http).unwrap().is_none());
        assert!(get_app_update("not a repo", "1.0.0", &k.public_xml, &http).unwrap().is_none());
        assert!(get_app_update("o/r", "1.0.0", "", &http).unwrap().is_none());
    }
    #[test]
    fn manifest_changed_after_signing_is_refused() {
        let k = keys();
        let mut m: serde_json::Value = serde_json::from_str(&manifest_json(&k.private, "2.0.0", &"A".repeat(64), ZIP_URL)).unwrap();
        m["sha256"] = serde_json::Value::String("B".repeat(64));
        let http = Fake::new(vec![("latest.json", &m.to_string())]);
        assert!(get_app_update("o/r", "1.0.0", &k.public_xml, &http).unwrap_err().contains("signature is not valid"));
    }
    #[test]
    fn manifest_signed_with_another_key_is_refused() {
        let (k, other) = (keys(), keys());
        let http = Fake::new(vec![("latest.json", &manifest_json(&other.private, "2.0.0", &"A".repeat(64), ZIP_URL))]);
        assert!(get_app_update("o/r", "1.0.0", &k.public_xml, &http).is_err());
    }
    #[test]
    fn download_host_outside_the_list_is_refused_even_when_signed() {
        let k = keys();
        let http = Fake::new(vec![("latest.json", &manifest_json(&k.private, "2.0.0", &"A".repeat(64), "https://evil.example.com/app.zip"))]);
        assert!(get_app_update("o/r", "1.0.0", &k.public_xml, &http).unwrap_err().contains("allowed list"));
    }

    // ---- online catalog ----
    fn entry(name: &str, ver: &str, url: &str, sha: &str) -> serde_json::Value {
        serde_json::json!({ "name": name, "version": ver, "match": ["PCI\\VEN_1234&DEV_*"], "type": "installer", "url": url, "sha256": sha, "signer": "Test", "silentArgs": "/s", "class": "NET" })
    }
    const MIRROR: &str = "https://github.com/kungzazakub-png/tiers-driver-manage/releases/download/drivers/x.exe";
    fn catalog_http(k: &Keys, rev: u32, entries: Vec<serde_json::Value>) -> Fake {
        let file = serde_json::json!({ "version": 1, "drivers": entries }).to_string();
        let sha = sha256_hex(file.as_bytes());
        let cat_url = "https://github.com/o/r/releases/download/catalog/driverdb.json";
        Fake::new(vec![("catalog/catalog.json", &manifest_json(&k.private, &format!("1.{rev}"), &sha, cat_url)), ("catalog/driverdb.json", &file)])
    }
    fn temp_cache(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("tdm_cat_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        d.join("online.json")
    }

    #[test]
    fn signed_revision_is_saved_and_not_downloaded_again() {
        let k = keys(); let cache = temp_cache("a");
        let http = catalog_http(&k, 1, vec![entry("Mirror Test", "2.0.0.0", MIRROR, &"A".repeat(64))]);
        let r = get_online_catalog("o/r", &k.public_xml, &cache, &http).unwrap().unwrap();
        assert_eq!(r, OnlineResult { updated: true, revision: 1, count: 1 });
        assert!(!get_online_catalog("o/r", &k.public_xml, &cache, &http).unwrap().unwrap().updated);
    }
    #[test]
    fn catalog_signed_with_another_key_is_refused_and_the_cache_stays() {
        let (k, other) = (keys(), keys()); let cache = temp_cache("b");
        get_online_catalog("o/r", &k.public_xml, &cache, &catalog_http(&k, 1, vec![entry("A", "1.0.0.0", MIRROR, &"A".repeat(64))])).unwrap();
        assert!(get_online_catalog("o/r", &k.public_xml, &cache, &catalog_http(&other, 2, vec![entry("B", "1.0.0.0", MIRROR, &"A".repeat(64))])).is_err());
        assert_eq!(cached_revision(&cache), 1);
    }
    #[test]
    fn file_changed_after_signing_is_refused() {
        let k = keys(); let cache = temp_cache("c");
        let mut http = catalog_http(&k, 1, vec![entry("A", "1.0.0.0", MIRROR, &"A".repeat(64))]);
        for (key, bytes) in http.routes.iter_mut() { if key == "catalog/driverdb.json" { bytes[5] ^= 1; } }
        assert!(get_online_catalog("o/r", &k.public_xml, &cache, &http).unwrap_err().contains("SHA-256 does not match"));
    }
    #[test]
    fn one_bad_entry_rejects_the_whole_catalog() {
        let k = keys(); let cache = temp_cache("d");
        let outside = entry("Bad", "1.0.0.0", "https://evil.example.com/x.exe", &"A".repeat(64));
        let e = get_online_catalog("o/r", &k.public_xml, &cache, &catalog_http(&k, 1, vec![entry("Good", "1.0.0.0", MIRROR, &"A".repeat(64)), outside])).unwrap_err();
        assert!(e.contains("Entry 'Bad'") && e.contains("allowed list"));
        assert!(!cache.exists());
        let nohash = entry("NoHash", "1.0.0.0", MIRROR, "");
        assert!(get_online_catalog("o/r", &k.public_xml, &cache, &catalog_http(&k, 2, vec![nohash])).is_err());
    }
    #[test]
    fn online_entry_replaces_the_local_one_with_the_same_name() {
        let k = keys(); let cache = temp_cache("e");
        get_online_catalog("o/r", &k.public_xml, &cache, &catalog_http(&k, 1, vec![entry("Mirror Test Driver", "2.0.0.0", MIRROR, &"A".repeat(64))])).unwrap();
        let local = cache.with_file_name("local.json");
        std::fs::write(&local, serde_json::json!({ "drivers": [entry("Mirror Test Driver", "1.0.0.0", MIRROR, "B"), entry("Mine Only", "1.0.0.0", MIRROR, "B")] }).to_string()).unwrap();
        let all = read_all_catalogs(&local, &cache);
        assert_eq!(all.len(), 2);
        assert_eq!(all.iter().find(|c| c.name == "Mirror Test Driver").unwrap().version, "2.0.0.0");
    }
}
