#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use tdm::engine::Config;
use tdm::{catalog, ui, winsys};

fn diag() {
    let host = winsys::host_info();
    println!("host: arch={} os={}", host.arch, host.os);
    println!("machine: {}", winsys::machine_summary());
    match winsys::installed_drivers() {
        Ok(list) => {
            println!("installed drivers: {}", list.len());
            if let Some(d) = list.iter().find(|d: &&catalog::InstalledDriver| d.name.contains("Bluetooth") && d.maker.starts_with("Intel")) {
                println!("sample: {} v{} live={:?}", d.name, d.version, winsys::live_driver_version(&d.instance_id));
            }
        }
        Err(e) => println!("installed drivers FAILED: {e}"),
    }
    match winsys::devices_without_driver() { Ok((d, l)) => println!("devices without driver: {} {:?}", d.len(), l), Err(e) => println!("without-driver FAILED: {e}") }
}

/// Single-instance guard. With --wait-for-previous (restart after an update) wait for the old copy to exit first.
fn single_instance(wait: bool) -> Option<windows::Win32::Foundation::HANDLE> {
    use windows::core::w;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    for _ in 0..if wait { 40 } else { 1 } {
        let h = unsafe { CreateMutexW(None, true, w!("Global\\TierS.DriverManage.Instance")) }.ok()?;
        if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS { return Some(h); }
        // release our handle, or it would keep the other copy's mutex alive and we would never get it
        unsafe { let _ = windows::Win32::Foundation::CloseHandle(h); }
        if !wait { return None; }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    None
}

fn note(msg: &str) {
    use windows::core::{w, HSTRING};
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
    unsafe { MessageBoxW(None, &HSTRING::from(msg), w!("TierS Driver Manage"), MB_OK | MB_ICONINFORMATION); }
}

fn log_error(msg: &str) {
    let dir = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join("TierS Driver Manage");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("error.log"), msg);
    use windows::core::{w, HSTRING};
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    unsafe { MessageBoxW(None, &HSTRING::from(format!("{msg}\n\n(saved to {})", dir.join("error.log").display())), w!("TierS Driver Manage"), MB_OK | MB_ICONERROR); }
}

fn main() {
    std::panic::set_hook(Box::new(|i| log_error(&format!("Crash: {i}"))));
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--diag") { diag(); return; }
    let wait = args.iter().any(|a| a == "--wait-for-previous");
    let shot = args.iter().position(|a| a == "--screenshot").and_then(|i| args.get(i + 1)).map(std::path::PathBuf::from);
    let demo = args.iter().position(|a| a == "--demo").and_then(|i| args.get(i + 1)).cloned();
    let _guard = if shot.is_some() { None } else { match single_instance(wait) { Some(h) => Some(h), None => { note("TierS Driver Manage is already running (look in the taskbar or Task Manager). Close the other copy first."); return } } };

    let dir = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join("TierS Driver Manage");
    let _ = std::fs::create_dir_all(&dir);
    let catalog_path = dir.join("driverdb.json");
    let _ = std::fs::write(&catalog_path, ui::SHIPPED_CATALOG);
    if let Ok(exe) = std::env::current_exe() { let _ = std::fs::remove_file(exe.with_extension("exe.old")); }
    let cfg = Config { catalog_path, online_catalog_path: dir.join("driverdb.online.json"), repo: ui::REPO.into(), public_key: ui::PUBLIC_KEY.into(), block_catalog: false, old_years: 3.0 };

    let icon = image::load_from_memory(include_bytes!("../assets/tiers-logo.png")).ok().map(|i| { let r = i.to_rgba8(); egui_icon(r) });
    let mut vp = eframe::egui::ViewportBuilder::default().with_title("TierS Driver Manage").with_inner_size([1100.0, 720.0]).with_min_inner_size([900.0, 560.0]);
    if let Some(i) = icon { vp = vp.with_icon(i); }
    let opts = eframe::NativeOptions { viewport: vp, ..Default::default() };
    if let Err(e) = eframe::run_native("TierS Driver Manage", opts, Box::new(move |cc| {
        let mut app = ui::App::new(cc, cfg, shot);
        if let Some(d) = demo { app.demo(&d); }
        Ok(Box::new(app))
    })) { log_error(&format!("Could not open the window: {e}")); }
}

fn egui_icon(r: image::RgbaImage) -> eframe::egui::IconData { let (w, h) = (r.width(), r.height()); eframe::egui::IconData { rgba: r.into_raw(), width: w, height: h } }


