#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use tdm::engine::Config;
use tdm::{catalog, ui, winsys};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Icon, WindowBuilder};
use wry::{WebContext, WebViewBuilder};

enum UserEvent { Wake, Ipc(String), Close }

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

/// Developer check of the power code on this PC: exports the active plan and compares Ultimate with Balanced. Nothing is activated or changed.
fn power_test() {
    use tdm::power::{self, backup, hw, log::Log, overlay::{Overlay, RealOverlay}};
    let (sys, ov) = (tdm::sys::RealSys, RealOverlay);
    let log = Log::new(None);
    let rp = || Ok(());
    let env = power::Env { cfg: &sys, overlay: &ov, root: std::env::temp_dir().join("tdm_power_test"), log: &log, restore_point: &rp };
    let name = power::cfg::active(&sys).map(|a| a.name).unwrap_or_default();
    let h = hw::detect(name);
    println!("{h:?}\nscore: {:?}\noverlay available: {} current: {:?}", hw::score(&h), ov.available(), ov.get());
    println!("{}", power::backup_only(&env).text());
    println!("{:?}", power::compare_ultimate(&env, &h));
    println!("backups: {}", backup::list(&env.root).len());
}

fn run_window(cfg: Config, demo: Option<String>, dir: std::path::PathBuf) -> Result<(), String> {
    use base64::Engine;
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = Arc::new(Mutex::new(event_loop.create_proxy()));
    let icon = image::load_from_memory(include_bytes!("../assets/tiers-logo.png")).ok().and_then(|i| { let r = i.to_rgba8(); let (w, h) = (r.width(), r.height()); Icon::from_rgba(r.into_raw(), w, h).ok() });
    let window = WindowBuilder::new().with_title("TierS Driver Manage").with_inner_size(tao::dpi::LogicalSize::new(1100.0, 720.0)).with_min_inner_size(tao::dpi::LogicalSize::new(900.0, 560.0)).with_window_icon(icon).build(&event_loop).map_err(|e| e.to_string())?;
    // WebView2 keeps its profile here: the program folder is not writable for normal users
    let mut web_ctx = WebContext::new(Some(dir.join("webview")));
    let logo = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(include_bytes!("../assets/tiers-logo.png")));
    let html = ui::PAGE_HTML.replace("__LOGO__", &logo);
    let p2 = proxy.clone();
    let webview = WebViewBuilder::new_with_web_context(&mut web_ctx).with_html(html).with_background_color((244, 246, 249, 255)).with_devtools(false)
        .with_ipc_handler(move |req| { let _ = p2.lock().unwrap().send_event(UserEvent::Ipc(req.body().to_string())); })
        .build(&window).map_err(|e| e.to_string())?;

    let (pw, pc) = (proxy.clone(), proxy.clone());
    let ctx = ui::Ctx { wake: Arc::new(move || { let _ = pw.lock().unwrap().send_event(UserEvent::Wake); }), close: Arc::new(move || { let _ = pc.lock().unwrap().send_event(UserEvent::Close); }) };
    let mut app = ui::App::new(cfg);
    match &demo { Some(d) => { app.demo(d); if d == "power" { app.handle(r#"{"cmd":"tab","tab":"power"}"#, &ctx); } } None => app.check_for_update(&ctx) }
    let mut last: Option<String> = None;
    event_loop.run(move |event, _, flow| {
        let _keep = &web_ctx;
        *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(250));
        let mut tick = false;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => { *flow = ControlFlow::Exit; return; }
            Event::UserEvent(UserEvent::Close) => { *flow = ControlFlow::Exit; return; }
            Event::UserEvent(UserEvent::Ipc(m)) => { if m.contains("\"ready\"") { last = None; } else { app.handle(&m, &ctx); } tick = true; }
            Event::UserEvent(UserEvent::Wake) | Event::NewEvents(StartCause::ResumeTimeReached { .. }) => tick = true,
            _ => {}
        }
        if tick {
            app.poll(&ctx);
            let v = app.view().to_string();
            if last.as_deref() != Some(v.as_str()) { let _ = webview.evaluate_script(&format!("window.setState({v})")); last = Some(v); }
        }
    });
}

fn main() {
    std::panic::set_hook(Box::new(|i| log_error(&format!("Crash: {i}"))));
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--diag") { diag(); return; }
    if args.iter().any(|a| a == "--power-test") { power_test(); return; }
    if args.iter().any(|a| a == "--memory-bench") {
        // developer check: cost of one memory read, and the CPU time the whole run used
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let (mut ok, mut err, n) = (0u32, 0u32, 2000u32);
        let t = Instant::now();
        for _ in 0..n { if tdm::memory::Mem::stats(&tdm::memory::RealMem).is_ok() { ok += 1 } else { err += 1 } }
        let wall = t.elapsed();
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let _ = unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
        let ticks = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 10_000.0; // ms
        let text = format!("{n} reads: ok {ok}, failed {err}\nper read: {:.1} microseconds\nCPU time used: kernel {:.0} ms + user {:.0} ms", wall.as_secs_f64() * 1e6 / n as f64, ticks(k), ticks(u));
        let _ = std::fs::write(std::env::temp_dir().join("tdm_bench.txt"), &text);
        note(&text);
        return;
    }
    if args.iter().any(|a| a == "--memory-test") { let r = format!("{:?}", tdm::memory::Mem::stats(&tdm::memory::RealMem)); println!("{r}"); note(&r); return; }
    let wait = args.iter().any(|a| a == "--wait-for-previous");
    let demo = args.iter().position(|a| a == "--demo").and_then(|i| args.get(i + 1)).cloned();
    let _guard = if demo.is_some() { None } else { match single_instance(wait) { Some(h) => Some(h), None => { note("TierS Driver Manage is already running (look in the taskbar or Task Manager). Close the other copy first."); return } } };

    let dir = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join("TierS Driver Manage");
    let _ = std::fs::create_dir_all(&dir);
    let catalog_path = dir.join("driverdb.json");
    let _ = std::fs::write(&catalog_path, ui::SHIPPED_CATALOG);
    if let Ok(exe) = std::env::current_exe() { tdm::selfupdate::cleanup_old(&exe); }
    let cfg = Config { catalog_path, online_catalog_path: dir.join("driverdb.online.json"), repo: ui::REPO.into(), public_key: ui::PUBLIC_KEY.into(), block_catalog: false, old_years: 3.0 };
    if let Err(e) = run_window(cfg, demo, dir) { log_error(&format!("Could not open the window: {e}\n\nThe program needs the Microsoft Edge WebView2 Runtime (included with Windows 11 and current Windows 10)." )); }
}