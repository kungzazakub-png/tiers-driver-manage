//! Authenticode check of a downloaded installer: is the signature valid, and who signed it (the subject must match the catalog's signer).
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::Security::Cryptography::{CertNameToStrW, CERT_X500_NAME_STR, X509_ASN_ENCODING, PKCS_7_ASN_ENCODING};
use windows::Win32::Security::WinTrust::{
    WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0,
    WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
};

#[derive(Debug, Clone)]
pub struct SignatureInfo {
    /// "Valid", "NotSigned", "HashMismatch" or "NotTrusted (0x...)" - the text goes into error messages.
    pub status: String,
    /// Full subject of the signing certificate, e.g. "CN=Intel Corporation, O=Intel Corporation, S=California, C=US".
    pub subject: String,
}

impl SignatureInfo { pub fn is_valid(&self) -> bool { self.status == "Valid" } }

pub fn verify_file(path: &Path) -> SignatureInfo {
    let wpath: Vec<u16> = OsStr::new(path).encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        let mut file = WINTRUST_FILE_INFO { cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32, pcwszFilePath: PCWSTR(wpath.as_ptr()), ..Default::default() };
        let mut data = WINTRUST_DATA {
            cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_FILE,
            Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
            dwStateAction: WTD_STATEACTION_VERIFY,
            ..Default::default()
        };
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let hwnd = HWND(-1isize as *mut core::ffi::c_void); // INVALID_HANDLE_VALUE: no UI
        let res = WinVerifyTrust(hwnd, &mut action, &mut data as *mut _ as *mut core::ffi::c_void) as u32;
        let status = match res {
            0 => "Valid".to_string(),
            0x800B0100 => "NotSigned".to_string(),
            0x80096010 => "HashMismatch".to_string(),
            other => format!("NotTrusted (0x{other:08X})"),
        };
        let mut subject = String::new();
        if data.hWVTStateData != HANDLE::default() && !data.hWVTStateData.is_invalid() {
            let prov = WTHelperProvDataFromStateData(data.hWVTStateData);
            if !prov.is_null() {
                let sgnr = WTHelperGetProvSignerFromChain(prov, 0, false, 0);
                if !sgnr.is_null() && !(*sgnr).pasCertChain.is_null() && (*sgnr).csCertChain > 0 {
                    let ctx = (*(*sgnr).pasCertChain).pCert;
                    if !ctx.is_null() && !(*ctx).pCertInfo.is_null() {
                        let blob = &(*(*ctx).pCertInfo).Subject;
                        let mut buf = vec![0u16; 1024];
                        let n = CertNameToStrW(X509_ASN_ENCODING | PKCS_7_ASN_ENCODING, blob, CERT_X500_NAME_STR, Some(&mut buf));
                        if n > 1 { subject = String::from_utf16_lossy(&buf[..(n as usize - 1)]); }
                    }
                }
            }
        }
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        let _ = WinVerifyTrust(hwnd, &mut action, &mut data as *mut _ as *mut core::ffi::c_void);
        SignatureInfo { status, subject }
    }
}

/// The installer rule: valid signature AND the signer named in the catalog appears in the certificate subject (like PowerShell `-like "*signer*"`).
pub fn check_signer(path: &Path, signer: &str, what: &str) -> Result<SignatureInfo, String> {
    let info = verify_file(path);
    if !info.is_valid() { return Err(format!("The {what}'s digital signature is not valid ({})", info.status)); }
    if !info.subject.to_lowercase().contains(&signer.to_lowercase()) { return Err(format!("Signer does not match the expected publisher: {}", info.subject)); }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_file_is_not_signed() {
        let p = std::env::temp_dir().join(format!("tdm_unsigned_{}.exe", std::process::id()));
        std::fs::write(&p, b"MZ not a real installer").unwrap();
        let i = verify_file(&p);
        assert!(!i.is_valid(), "{i:?}");
        assert!(check_signer(&p, "Intel", "installer").unwrap_err().contains("signature is not valid"));
        std::fs::remove_file(p).ok();
    }

    /// A real vendor-signed binary that ships with this PC (the Rust toolchain installer is not required): the GitHub CLI or Edge.
    #[test]
    fn a_real_signed_binary_is_valid_and_the_signer_must_match() {
        let candidates = [r"C:\Program Files\GitHub CLI\gh.exe", r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe", r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"];
        let Some(p) = candidates.iter().map(Path::new).find(|p| p.exists()) else { eprintln!("no signed sample binary on this PC; skipped"); return };
        let i = verify_file(p);
        assert!(i.is_valid(), "{i:?}");
        assert!(!i.subject.is_empty());
        assert!(check_signer(p, "Definitely Not The Signer", "installer").unwrap_err().contains("does not match"));
        let who = i.subject.split("CN=").nth(1).unwrap().split(',').next().unwrap().trim_matches('"').to_string();
        assert!(check_signer(p, &who, "installer").is_ok());
    }
}
