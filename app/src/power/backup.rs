//! Backup records: one folder per backup with the exported plan (.pow) and a JSON description of everything the
//! program is about to change, so it can be put back.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ValueRec { pub label: String, pub sub: String, pub guid: String, pub ac: Option<u32>, pub dc: Option<u32> }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Record {
    pub id: String,
    pub time: String,
    /// "manual" (Backup Settings) or "apply"
    pub kind: String,
    pub profile: String,
    pub original_guid: String,
    pub original_name: String,
    /// exported plan file name inside the backup folder
    pub export: String,
    pub overlay_before: Option<String>,
    /// plan created by the optimization (deleted again on restore)
    pub created_guid: Option<String>,
    pub values: Vec<ValueRec>,
    pub restore_point: String,
    pub restored: bool,
}

pub fn root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("TierS Driver Manage").join("power")
}

pub fn new_id(root: &Path, now: &str) -> String {
    let base: String = now.chars().filter(|c| c.is_ascii_digit() || *c == 'T').collect::<String>().replace('T', "-");
    let mut id = base.clone();
    let mut n = 1;
    while root.join(&id).exists() { n += 1; id = format!("{base}-{n}"); }
    id
}

pub fn folder(root: &Path, id: &str) -> PathBuf { root.join(id) }

pub fn save(root: &Path, r: &Record) -> Result<(), String> {
    let d = folder(root, &r.id);
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(r).map_err(|e| e.to_string())?;
    std::fs::write(d.join("backup.json"), text).map_err(|e| e.to_string())
}

/// All records, oldest first.
pub fn list(root: &Path) -> Vec<Record> {
    let Ok(rd) = std::fs::read_dir(root) else { return vec![] };
    let mut v: Vec<Record> = rd.flatten().filter_map(|e| std::fs::read_to_string(e.path().join("backup.json")).ok()).filter_map(|t| serde_json::from_str(&t).ok()).collect();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
}
