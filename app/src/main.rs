use tdm::{catalog, winsys};

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    if arg == "--diag" {
        // developer check of the Windows layer on this PC
        let host = winsys::host_info();
        println!("host: arch={} os={}", host.arch, host.os);
        println!("machine: {}", winsys::machine_summary());
        match winsys::installed_drivers() {
            Ok(list) => {
                println!("installed drivers: {}", list.len());
                if let Some(d) = list.iter().find(|d: &&catalog::InstalledDriver| d.name.contains("Bluetooth") && d.maker.starts_with("Intel")) {
                    println!("sample: {} v{} date={:?} ids={}", d.name, d.version, d.date, d.hardware_ids.len());
                    println!("live version: {:?}  inf: {:?}", winsys::live_driver_version(&d.instance_id), winsys::driver_inf(&d.instance_id));
                }
            }
            Err(e) => println!("installed drivers FAILED: {e}"),
        }
        match winsys::devices_without_driver() { Ok((d, l)) => println!("devices without driver: {} {:?}", d.len(), l), Err(e) => println!("without-driver FAILED: {e}") }
        return;
    }
    println!("TierS Driver Manage (Rust port, work in progress)");
}
