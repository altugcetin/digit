fn main() {
    // Layer shell must load before libwayland, or every overlay silently becomes a window.
    // A library named by the binary comes first on the link line, whatever the toolchain.
    println!("cargo:rustc-link-lib=dylib=gtk4-layer-shell");
}
