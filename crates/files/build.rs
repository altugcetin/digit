fn main() {
    // Layer shell must load before libwayland, or kit's overlays silently become windows.
    println!("cargo:rustc-link-lib=dylib=gtk4-layer-shell");
    println!("cargo:rustc-link-arg-bins=-Wl,--export-dynamic-symbol=eglTerminate");
}
