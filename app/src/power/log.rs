//! Every change is written to power.log (and kept in memory so the window can show it).
use crate::catalog::Date;
use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;

pub fn stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{}T{:02}:{:02}:{:02}", Date::today().iso(), (secs / 3600) % 24, (secs / 60) % 60, secs % 60)
}

pub struct Log { path: Option<PathBuf>, lines: RefCell<Vec<String>> }

impl Log {
    pub fn new(path: Option<PathBuf>) -> Log { Log { path, lines: RefCell::new(vec![]) } }
    pub fn line(&self, msg: &str) {
        let l = format!("{} {msg}", stamp());
        if let Some(p) = &self.path {
            if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) { let _ = writeln!(f, "{l}"); }
        }
        self.lines.borrow_mut().push(l);
    }
    pub fn lines(&self) -> Vec<String> { self.lines.borrow().clone() }
}
