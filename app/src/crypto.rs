//! Hashes and the signature check for the program's update manifest and online catalog.
//! The signature format is the one the existing publishing scripts produce (.NET RSA, PKCS#1 v1.5 + SHA-256, key as .NET XML).
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::signature::Verifier;
use rsa::{BigUint, RsaPublicKey};
use sha2::{Digest, Sha256, Sha512};
use std::io::Read;
use std::path::Path;

fn hex_upper(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex_upper(&Sha256::digest(data))
}

fn hash_file<D: Digest>(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(path)?;
    let mut h = D::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 { break; }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().to_vec())
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> { Ok(hex_upper(&hash_file::<Sha256>(path)?)) }
pub fn sha512_file(path: &Path) -> std::io::Result<String> { Ok(hex_upper(&hash_file::<Sha512>(path)?)) }
/// Microsoft Update Catalog publishes SHA-1 digests in base64.
pub fn sha1_base64_file(path: &Path) -> std::io::Result<String> { Ok(B64.encode(hash_file::<sha1::Sha1>(path)?)) }

/// A downloaded file must match the hash the catalog pinned (hex, any case).
pub fn check_sha256(path: &Path, expected: &str) -> Result<(), String> {
    if expected.trim().is_empty() {
        return Err("The catalog entry has no sha256 value, so it will not be installed".into());
    }
    let actual = sha256_file(path).map_err(|e| e.to_string())?;
    if actual != expected.to_ascii_uppercase() {
        return Err(format!("SHA-256 mismatch (got {actual}); the file may have been modified or corrupted"));
    }
    Ok(())
}

fn xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let s = xml.find(&open)? + open.len();
    let e = xml[s..].find(&close)? + s;
    Some(xml[s..e].trim().to_string())
}

/// Reads the public half of a .NET `ToXmlString()` RSA key (`<Modulus>` and `<Exponent>`, base64).
pub fn public_key_from_xml(xml: &str) -> Result<RsaPublicKey, String> {
    let n = B64.decode(xml_tag(xml, "Modulus").ok_or("key has no Modulus")?).map_err(|e| e.to_string())?;
    let e = B64.decode(xml_tag(xml, "Exponent").ok_or("key has no Exponent")?).map_err(|e| e.to_string())?;
    RsaPublicKey::new(BigUint::from_bytes_be(&n), BigUint::from_bytes_be(&e)).map_err(|e| e.to_string())
}

/// What the publisher signs: "version|sha256|url".
pub fn signed_payload(version: &str, sha256: &str, url: &str) -> String {
    format!("{version}|{sha256}|{url}")
}

pub fn verify_manifest(version: &str, sha256: &str, url: &str, signature_b64: &str, public_key_xml: &str) -> bool {
    let Ok(key) = public_key_from_xml(public_key_xml) else { return false };
    let Ok(sig_bytes) = B64.decode(signature_b64.trim()) else { return false };
    let Ok(sig) = Signature::try_from(sig_bytes.as_slice()) else { return false };
    VerifyingKey::<Sha256>::new(key).verify(signed_payload(version, sha256, url).as_bytes(), &sig).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn sha256_of_known_text() {
        assert_eq!(sha256_hex(b"abc"), "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD");
    }

    fn temp_file(name: &str, data: &[u8]) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("tdm_{}_{}", std::process::id(), name));
        std::fs::File::create(&p).unwrap().write_all(data).unwrap();
        p
    }

    #[test]
    fn correct_hash_is_accepted_wrong_or_missing_rejected() {
        let p = temp_file("hash.bin", b"abc");
        assert!(check_sha256(&p, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad").is_ok());
        assert!(check_sha256(&p, &"0".repeat(64)).unwrap_err().contains("SHA-256 mismatch"));
        assert!(check_sha256(&p, "").unwrap_err().contains("no sha256"));
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn sha1_digest_is_base64() {
        let p = temp_file("sha1.bin", b"abc");
        assert_eq!(sha1_base64_file(&p).unwrap(), "qZk+NkcGgWq6PiVxeFDCbJzQ2J0=");
        std::fs::remove_file(p).ok();
    }

    /// The real manifest published by the PowerShell tooling must verify with the real public key.
    #[test]
    fn signature_made_by_the_powershell_publisher_verifies() {
        let key = include_str!("../tests/fixtures/UpdatePublicKey.xml");
        let m: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/latest.json").trim_start_matches('\u{feff}')).unwrap();
        let g = |k: &str| m[k].as_str().unwrap().to_string();
        assert!(verify_manifest(&g("version"), &g("sha256"), &g("url"), &g("signature"), key));
        assert!(!verify_manifest(&g("version"), &g("sha256"), "https://github.com/evil/x.zip", &g("signature"), key), "a changed url must not verify");
        assert!(!verify_manifest("9.9.9", &g("sha256"), &g("url"), &g("signature"), key), "a changed version must not verify");
        assert!(!verify_manifest(&g("version"), &"A".repeat(64), &g("url"), &g("signature"), key), "a changed hash must not verify");
    }

    #[test]
    fn garbage_signature_or_key_never_verifies() {
        let key = include_str!("../tests/fixtures/UpdatePublicKey.xml");
        assert!(!verify_manifest("1.0.0", "A", "u", "not base64!!", key));
        assert!(!verify_manifest("1.0.0", "A", "u", "AAAA", key));
        assert!(!verify_manifest("1.0.0", "A", "u", "AAAA", "<RSAKeyValue/>"));
    }
}
