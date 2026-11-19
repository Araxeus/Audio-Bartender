fn main() {
    // GPUI loads its window icon from resource 1, and the tray reuses it.
    embed_resource::compile("app.rc", embed_resource::NONE).manifest_optional().unwrap();
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
}
