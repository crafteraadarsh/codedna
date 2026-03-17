fn main() {
    // On Windows, libgit2-sys requires these system libraries.
    // Some environments don't link them automatically, so we
    // explicitly request them here as a safety net.
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        println!("cargo:rustc-link-lib=advapi32");
        println!("cargo:rustc-link-lib=crypt32");
    }
}
