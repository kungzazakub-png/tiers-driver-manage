#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use tdm::engine::Config;
use tdm::{catalog, ui, winsys};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Icon, Window, WindowBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use wry::{WebContext, WebView, WebViewBuilder};

enum UserEvent { Wake, Ipc(String), Close, Show, Tray(String) }
type Proxy = Arc<Mutex<EventLoopProxy<UserEvent>>>;

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

/// A second launch asks the running copy (maybe sitting in the tray) to show its window. false = nobody answered.
fn ask_running_copy_to_show() -> bool {
    use windows::core::w;
    use windows::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};
    unsafe {
        let Ok(h) = OpenEventW(EVENT_MODIFY_STATE, false, w!("Global\\TierS.DriverManage.Show")) else { return false };
        let ok = SetEvent(h).is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(h);
        ok
    }
}

/// The running copy waits for that event and shows its window when it fires.
fn listen_for_show_requests(proxy: Proxy) {
    use windows::core::w;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};
    let Ok(h) = (unsafe { CreateEventW(None, false, false, w!("Global\\TierS.DriverManage.Show")) }) else { return };
    let raw = h.0 as isize; // HANDLE is not Send; the number is
    std::thread::spawn(move || loop {
        if unsafe { WaitForSingleObject(HANDLE(raw as *mut _), INFINITE) }.0 != 0 { break; }
        let _ = proxy.lock().unwrap().send_event(UserEvent::Show);
    });
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

/// (Re)creates the WebView2 page. While the window sits in the tray the page is destroyed so it holds no memory.
fn make_webview(window: &Window, web_ctx: &mut WebContext, proxy: &Proxy, html: &str) -> Result<WebView, String> {
    let p = proxy.clone();
    WebViewBuilder::new_with_web_context(web_ctx).with_html(html).with_background_color((244, 246, 249, 255)).with_devtools(false)
        .with_ipc_handler(move |req| { let _ = p.lock().unwrap().send_event(UserEvent::Ipc(req.body().to_string())); })
        .build(window).map_err(|e| e.to_string())
}

fn run_window(cfg: Config, demo: Option<String>, dir: std::path::PathBuf, start_in_tray: bool) -> Result<(), String> {
    use base64::Engine;
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy: Proxy = Arc::new(Mutex::new(event_loop.create_proxy()));
    let logo_png = include_bytes!("../assets/tiers-logo.png");
    let rgba = image::load_from_memory(logo_png).ok().map(|i| i.to_rgba8());
    let icon = rgba.clone().and_then(|r| { let (w, h) = (r.width(), r.height()); Icon::from_rgba(r.into_raw(), w, h).ok() });

    // the tray icon; if Windows refuses it the program simply behaves as before (closing the window quits)
    let open_item = MenuItem::with_id("open", "Open TierS Driver Manage", true, None);
    let auto_item = CheckMenuItem::with_id("auto", "Automatic memory cleaning", true, false, None);
    let clean_item = MenuItem::with_id("clean", "Clean standby list now", true, None);
    let quit_item = MenuItem::with_id("quit", "Quit", true, None);
    let menu = Menu::new();
    let _ = menu.append_items(&[&open_item, &PredefinedMenuItem::separator(), &auto_item, &clean_item, &PredefinedMenuItem::separator(), &quit_item]);
    let tray_icon = rgba.and_then(|r| { let (w, h) = (r.width(), r.height()); tray_icon::Icon::from_rgba(r.into_raw(), w, h).ok() });
    let mut builder = TrayIconBuilder::new().with_menu(Box::new(menu)).with_menu_on_left_click(false).with_tooltip("TierS Driver Manage");
    if let Some(i) = tray_icon { builder = builder.with_icon(i); }
    let tray = if demo.is_some() { None } else { builder.build().ok() };
    let (pm, pt) = (proxy.clone(), proxy.clone());
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| { let _ = pm.lock().unwrap().send_event(UserEvent::Tray(e.id.0.clone())); }));
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } = e { let _ = pt.lock().unwrap().send_event(UserEvent::Show); }
    }));
    let hidden_start = start_in_tray && tray.is_some() && demo.is_none();

    let window = WindowBuilder::new().with_title("TierS Driver Manage").with_inner_size(tao::dpi::LogicalSize::new(1100.0, 720.0)).with_min_inner_size(tao::dpi::LogicalSize::new(900.0, 560.0)).with_window_icon(icon).with_visible(!hidden_start).build(&event_loop).map_err(|e| e.to_string())?;
    // WebView2 keeps its profile here: the program folder is not writable for normal users
    let mut web_ctx = WebContext::new(Some(dir.join("webview")));
    let logo = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(logo_png));
    let html = ui::PAGE_HTML.replace("__LOGO__", &logo);
    let mut webview: Option<WebView> = if hidden_start { None } else { Some(make_webview(&window, &mut web_ctx, &proxy, &html)?) };
    if demo.is_none() { listen_for_show_requests(proxy.clone()); }

    let (pw, pc) = (proxy.clone(), proxy.clone());
    let ctx = ui::Ctx { wake: Arc::new(move || { let _ = pw.lock().unwrap().send_event(UserEvent::Wake); }), close: Arc::new(move || { let _ = pc.lock().unwrap().send_event(UserEvent::Close); }) };
    let mut app = ui::App::new(cfg);
    match &demo { Some(d) => { app.demo(d); if d == "power" { app.handle(r#"{"cmd":"tab","tab":"power"}"#, &ctx); } } None => app.check_for_update(&ctx) }
    let mut last: Option<String> = None;
    let mut shown_auto = false;
    event_loop.run(move |event, _, flow| {
        let _keep = (&tray, &open_item);
        *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(250));
        let mut tick = false;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                if tray.is_some() && app.tray_on_close() { window.set_visible(false); webview = None; last = None; }
                else { *flow = ControlFlow::Exit; return; }
            }
            Event::LoopDestroyed => { app.shutdown(); return; }
            Event::UserEvent(UserEvent::Close) => { *flow = ControlFlow::Exit; return; }
            Event::UserEvent(UserEvent::Show) => {
                window.set_visible(true); window.set_minimized(false); window.set_focus();
                if webview.is_none() { match make_webview(&window, &mut web_ctx, &proxy, &html) { Ok(w) => webview = Some(w), Err(e) => log_error(&format!("Could not open the window: {e}")) } }
                last = None; tick = true;
            }
            Event::UserEvent(UserEvent::Tray(id)) => {
                match id.as_str() {
                    "open" => { let _ = proxy.lock().unwrap().send_event(UserEvent::Show); }
                    "auto" => { let v = if app.mem_auto() { 0 } else { 1 }; app.handle(&format!(r#"{{"cmd":"mem_auto","v":{v}}}"#), &ctx); }
                    "clean" => app.handle(r#"{"cmd":"mem_purge"}"#, &ctx),
                    "quit" => { *flow = ControlFlow::Exit; return; }
                    _ => {}
                }
                tick = true;
            }
            Event::UserEvent(UserEvent::Ipc(m)) => { if m.contains("\"ready\"") { last = None; } else { app.handle(&m, &ctx); } tick = true; }
            Event::UserEvent(UserEvent::Wake) | Event::NewEvents(StartCause::ResumeTimeReached { .. }) => tick = true,
            _ => {}
        }
        if tick {
            app.poll(&ctx);
            if let Some(t) = &tray {
                let on = app.mem_auto();
                if on != shown_auto { shown_auto = on; auto_item.set_checked(on); let _ = t.set_tooltip(Some(if on { "TierS Driver Manage - memory cleaning on" } else { "TierS Driver Manage" })); }
            }
            if let Some(w) = &webview {
                let v = app.view().to_string();
                if last.as_deref() != Some(v.as_str()) { let _ = w.evaluate_script(&format!("window.setState({v})")); last = Some(v); }
            }
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
    let tray_start = args.iter().any(|a| a == "--tray");
    let demo = args.iter().position(|a| a == "--demo").and_then(|i| args.get(i + 1)).cloned();
    let _guard = if demo.is_some() { None } else {
        match single_instance(wait) {
            Some(h) => Some(h),
            None => {
                // already running (maybe in the tray): bring its window up. Started by Windows at sign-in there is nothing to show.
                if !tray_start && !wait && !ask_running_copy_to_show() { note("TierS Driver Manage is already running (look in the taskbar, the tray or Task Manager). Close the other copy first."); }
                return
            }
        }
    };

    let dir = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join("TierS Driver Manage");
    let _ = std::fs::create_dir_all(&dir);
    let catalog_path = dir.join("driverdb.json");
    let _ = std::fs::write(&catalog_path, ui::SHIPPED_CATALOG);
    if let Ok(exe) = std::env::current_exe() { tdm::selfupdate::cleanup_old(&exe); }
    let cfg = Config { catalog_path, online_catalog_path: dir.join("driverdb.online.json"), repo: ui::REPO.into(), public_key: ui::PUBLIC_KEY.into(), block_catalog: false, old_years: 3.0 };
    if let Err(e) = run_window(cfg, demo, dir, tray_start) { log_error(&format!("Could not open the window: {e}\n\nThe program needs the Microsoft Edge WebView2 Runtime (included with Windows 11 and current Windows 10)." )); }
}
