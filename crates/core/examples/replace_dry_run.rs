//! Prints what "Replace the Windows volume icon" would change for the exe at the given
//! path, without touching Explorer or the registry.

fn main() {
    for replace in [true, false] {
        println!("replace={replace}:");
        for line in mediakeys_core::replace_native_volume_dry_run(replace) {
            println!("  {line}");
        }
    }
}
