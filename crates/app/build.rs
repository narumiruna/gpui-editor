fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // GPUI imports TaskDialogIndirect from version 6 of the Windows common controls.
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("windows.manifest");
    println!("cargo::rerun-if-changed={}", manifest.display());
    println!("cargo::rustc-link-arg-bin=gpui-editor=/MANIFEST:EMBED");
    println!(
        "cargo::rustc-link-arg-bin=gpui-editor=/MANIFESTINPUT:{}",
        manifest.display()
    );
}
