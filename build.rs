// Keep the overlay process lean: the DLLs below are only used by the editor
// window (OpenGL, IME, COM file dialog, theming). Delay-loading them means the
// overlay never maps them; they load on first use, i.e. only in `--menu`.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if windows && msvc {
        for dll in [
            "opengl32", "dwmapi", "imm32", "ole32", "combase", "shell32", "uxtheme",
        ] {
            println!("cargo:rustc-link-arg-bins=/DELAYLOAD:{dll}.dll");
        }
        println!("cargo:rustc-link-arg-bins=delayimp.lib");
    }
}
