//! tdm-fetcher: reads a report file, resolves new hardware IDs through the Microsoft Update Catalog and updates the catalog JSON.
//!   fetcher --reports reports.json --catalog DriverScan/driverdb.json --out dist/pkgs --repo owner/name [--max 10]
//! Prints one line per decision and writes the list of zip files to upload into <out>/uploaded.txt.
use tdm::catalog::{CatalogEntry, CatalogFile, HostInfo};
use tdm::fetch::{resolve, Env, Outcome, Reported};

fn arg(a: &[String], name: &str) -> Option<String> { a.iter().position(|x| x == name).and_then(|i| a.get(i + 1)).cloned() }

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (Some(reports), Some(cat), Some(out), Some(repo)) = (arg(&a, "--reports"), arg(&a, "--catalog"), arg(&a, "--out"), arg(&a, "--repo")) else {
        eprintln!("usage: fetcher --reports FILE --catalog FILE --out DIR --repo owner/name [--max N]"); std::process::exit(2);
    };
    let max: usize = arg(&a, "--max").and_then(|m| m.parse().ok()).unwrap_or(10);
    let text = std::fs::read_to_string(&reports).expect("reports file");
    // accepts {"devices":[...]} (the report body) or a plain list
    let items: Vec<Reported> = serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')).ok()
        .and_then(|v| serde_json::from_value(v.get("devices").cloned().unwrap_or(v)).ok()).unwrap_or_default();
    // the catalog is edited as plain JSON so every other field in the file stays exactly as it was
    let mut doc: serde_json::Value = serde_json::from_str(std::fs::read_to_string(&cat).expect("catalog file").trim_start_matches('\u{feff}')).expect("catalog JSON");
    let mut file: CatalogFile = serde_json::from_value(doc.clone()).expect("catalog entries");
    std::fs::create_dir_all(&out).expect("out dir");
    let (http, sys) = (tdm::http::Real::new(), tdm::sys::RealSys);
    let env = Env { http: &http, sys: &sys, host: HostInfo { arch: "x64".into(), os: "11".into() }, repo: &repo };
    let (mut added, mut zips): (usize, Vec<String>) = (0, vec![]);
    for item in items.iter().take(max) {
        // one lookup per device; entries added in this run count as "already covered" for the next report
        let existing: Vec<CatalogEntry> = file.drivers.clone();
        match resolve(&env, &existing, item, std::path::Path::new(&out)) {
            Outcome::Added(e, zip) => { println!("ADDED   {} -> {} ({})", e.match_.join(","), e.name, e.version); zips.push(zip.to_string_lossy().into_owned()); file.drivers.push(*e); added += 1; }
            Outcome::Skipped(why) => println!("SKIPPED {why}"),
        }
    }
    if added > 0 {
        let arr = doc["drivers"].as_array_mut().expect("drivers list");
        for e in &file.drivers[arr.len()..] {
            let mut v = serde_json::to_value(e).unwrap();
            if let serde_json::Value::Object(m) = &mut v { m.retain(|_, x| !x.is_null() && x.as_array().map_or(true, |a| !a.is_empty())); }
            arr.push(v);
        }
        std::fs::write(&cat, serde_json::to_string_pretty(&doc).unwrap()).expect("write catalog");
    }
    std::fs::write(std::path::Path::new(&out).join("uploaded.txt"), zips.join("\n")).expect("list");
    println!("done: {added} added");
}
