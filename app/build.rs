fn main() {
    // manifest (administrator, per-monitor DPI), icon and version info are compiled into the .exe
    let _ = embed_resource::compile("assets/app.rc", embed_resource::NONE);
}
