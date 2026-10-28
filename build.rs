// The Windows exe's VERSIONINFO: what Explorer's Details tab and Task Manager
// show, and what the code signing policy requires every signed binary to carry
// (product name and version set and identical across builds).
fn main() {
    // The target, not the host: the exe is cross-compiled from Linux.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set("ProductName", "Nudge")
        .set("FileDescription", "Nudge")
        .set("OriginalFilename", "nudge.exe")
        .set("InternalName", "nudge")
        .set("LegalCopyright", "Copyright (c) 2026 Wizard1209. MIT License.");
    // TODO: the daisy as the exe icon — needs an .ico rasterised from
    // assets/daisy.svg (16/32/48/256 px).
    res.compile().expect("compiling the Windows resource file");
}
