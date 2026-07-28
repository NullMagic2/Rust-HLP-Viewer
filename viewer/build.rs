//! Windows resource embedding for the native HLP viewer executable.
//!
//! This keeps the project-level application icon in the built `.exe` so Explorer, the taskbar,
//! and native window chrome all pick up the same HLP identity on Windows.

fn main() {
    println!("cargo:rerun-if-changed=assets/hlp.ico");

    #[cfg(target_os = "windows")]
    {
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("assets/hlp.ico");
        resource
            .compile()
            .expect("failed to embed viewer/assets/hlp.ico as the Windows application icon");
    }
}
