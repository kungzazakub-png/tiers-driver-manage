//! System restore point before an install (Windows allows one every 24 hours; a refusal is reported, not fatal).
use windows::Win32::System::Restore::{SRSetRestorePointW, RESTOREPOINTINFOW, RESTOREPOINTINFO_EVENT_TYPE, RESTOREPOINTINFO_TYPE, STATEMGRSTATUS};

pub fn create(description: &str) -> Result<(), String> {
    unsafe {
        let mut info = RESTOREPOINTINFOW { dwEventType: RESTOREPOINTINFO_EVENT_TYPE(100), dwRestorePtType: RESTOREPOINTINFO_TYPE(12), llSequenceNumber: 0, szDescription: [0; 256] }; // BEGIN_SYSTEM_CHANGE, MODIFY_SETTINGS
        for (i, c) in description.encode_utf16().take(255).enumerate() { info.szDescription[i] = c; }
        let mut status = STATEMGRSTATUS::default();
        if !SRSetRestorePointW(&info, &mut status).as_bool() { let code = { status.nStatus.0 }; return Err(format!("Windows refused (code {code})")); }
        let end = RESTOREPOINTINFOW { dwEventType: RESTOREPOINTINFO_EVENT_TYPE(101), dwRestorePtType: RESTOREPOINTINFO_TYPE(12), llSequenceNumber: { status.llSequenceNumber }, szDescription: [0; 256] }; // END_SYSTEM_CHANGE
        let mut s2 = STATEMGRSTATUS::default();
        let _ = SRSetRestorePointW(&end, &mut s2);
        Ok(())
    }
}


