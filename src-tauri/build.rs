fn main() {
    tauri_build::build();
    // The shared library's test harness also links the native dialog APIs.
    // Unlike app binaries it does not receive Tauri's resource manifest.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
        // App binaries already embed the full Tauri manifest through resource.lib.
        println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
    }
}
