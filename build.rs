//! Embeds the application icon into the Windows executable so Explorer,
//! the taskbar and Alt-Tab show it instead of the generic .exe icon.
//! Uses `windres` under the hood (GNU toolchain) or `rc.exe` (MSVC).

fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            // Don't fail the whole build just because the icon could not be
            // embedded - the app still runs, it just keeps the default icon.
            println!("cargo:warning=failed to embed exe icon: {e}");
        }
    }
    // Rebuild if the icon changes.
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
