//! Side effects the installer needs, behind a trait so the install logic can be tested without touching the PC.
use crate::trust::{self, SignatureInfo};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

pub trait Sys {
    /// Runs a program with the given arguments (pnputil, expand.exe, icacls); returns (exit code, combined output).
    fn run(&self, program: &str, args: &[&str]) -> (i32, String);
    /// Runs an installer with its silent switches exactly as the catalog wrote them; waits; returns the exit code.
    fn run_installer(&self, exe: &Path, silent_args: &str) -> Result<i32, String>;
    fn signature(&self, file: &Path) -> SignatureInfo;
    fn live_driver_version(&self, instance_id: &str) -> Option<String>;
    fn driver_inf(&self, instance_id: &str) -> Option<String>;
    /// Work folder readable by Administrators and SYSTEM only, so no other program can swap a file after it was checked.
    fn lock_down(&self, dir: &Path);
}

pub struct RealSys;

fn system32(exe: &str) -> String {
    format!("{}\\System32\\{exe}", std::env::var("windir").unwrap_or_else(|_| "C:\\Windows".into()))
}

impl Sys for RealSys {
    fn run(&self, program: &str, args: &[&str]) -> (i32, String) {
        let prog = if program.contains('\\') { program.to_string() } else { system32(program) };
        match Command::new(prog).args(args).creation_flags(0x0800_0000).output() {
            Ok(o) => {
                let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&o.stderr));
                (o.status.code().unwrap_or(-1), text)
            }
            Err(e) => (-1, e.to_string()),
        }
    }
    fn run_installer(&self, exe: &Path, silent_args: &str) -> Result<i32, String> {
        let status = Command::new(exe).raw_arg(silent_args).creation_flags(0x0800_0000).status().map_err(|e| e.to_string())?;
        Ok(status.code().unwrap_or(-1))
    }
    fn signature(&self, file: &Path) -> SignatureInfo { trust::verify_file(file) }
    fn live_driver_version(&self, instance_id: &str) -> Option<String> { crate::winsys::live_driver_version(instance_id) }
    fn driver_inf(&self, instance_id: &str) -> Option<String> { crate::winsys::driver_inf(instance_id) }
    fn lock_down(&self, dir: &Path) {
        let _ = self.run("icacls.exe", &[&dir.to_string_lossy(), "/inheritance:r", "/grant:r", "*S-1-5-32-544:(OI)(CI)F", "*S-1-5-18:(OI)(CI)F"]);
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::cell::RefCell;

    /// Records every call; answers from the fields.
    pub struct FakeSys {
        pub calls: RefCell<Vec<String>>,
        pub exit_code: i32,
        pub installer_exit: i32,
        pub signature: SignatureInfo,
        pub live: Option<String>,
        pub inf: Option<String>,
        /// files `pnputil /export-driver` should create
        pub export_files: bool,
    }
    impl FakeSys {
        pub fn ok() -> FakeSys {
            FakeSys { calls: Default::default(), exit_code: 0, installer_exit: 0, signature: SignatureInfo { status: "Valid".into(), subject: "CN=Test Vendor, O=Test".into() }, live: None, inf: None, export_files: true }
        }
        pub fn calls(&self) -> Vec<String> { self.calls.borrow().clone() }
    }
    impl Sys for FakeSys {
        fn run(&self, program: &str, args: &[&str]) -> (i32, String) {
            self.calls.borrow_mut().push(format!("{program} {}", args.join(" ")));
            if program.starts_with("pnputil") && args.first() == Some(&"/export-driver") && self.export_files {
                let dir = Path::new(args[2]);
                std::fs::create_dir_all(dir).unwrap();
                std::fs::write(dir.join("old.inf"), ";old").unwrap();
            }
            if program.starts_with("expand") {
                // expand.exe <cab> -F:* <dest>: pretend the cab held one INF that names the hardware
                let dest = Path::new(args[2]);
                std::fs::create_dir_all(dest).unwrap();
                std::fs::write(dest.join("driver.inf"), "[Strings]\r\n%dev%=Install, PCI\\VEN_8086&DEV_1234\r\n").unwrap();
            }
            (self.exit_code, String::new())
        }
        fn run_installer(&self, exe: &Path, silent_args: &str) -> Result<i32, String> {
            self.calls.borrow_mut().push(format!("installer {} {silent_args}", exe.file_name().unwrap().to_string_lossy()));
            Ok(self.installer_exit)
        }
        fn signature(&self, _f: &Path) -> SignatureInfo { self.signature.clone() }
        fn live_driver_version(&self, _i: &str) -> Option<String> { self.live.clone() }
        fn driver_inf(&self, _i: &str) -> Option<String> { self.inf.clone() }
        fn lock_down(&self, _d: &Path) {}
    }
}
