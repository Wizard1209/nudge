fn main() {
    // cfg! and #[cfg] in a build script check the host; CARGO_CFG_TARGET_OS names the target.
    // proof: https://doc.rust-lang.org/cargo/reference/environment-variables.html
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set("ProductName", "Nudge")
        .set("FileDescription", "Nudge")
        .set("OriginalFilename", "nudge.exe")
        .set("InternalName", "nudge")
        .set(
            "LegalCopyright",
            "Copyright (c) 2026 Wizard1209. MIT License.",
        );
    res.compile().expect("compiling the Windows resource file");
}
