fn main() {
    // The real version lives in the repo's top-level `VERSION` file (kept out
    // of Cargo.toml/Cargo.lock so a version bump never invalidates the cached
    // build dependencies). Inject it as a compile-time env var; `gta_mo_core`
    // re-exports it as `VERSION`.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let version_path = std::path::Path::new(&manifest_dir).join("../../VERSION");
    println!("cargo:rerun-if-changed={}", version_path.display());

    let version = std::fs::read_to_string(&version_path)
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("CARGO_PKG_VERSION").ok())
        .unwrap_or_else(|| "0.0.0".to_string());

    println!("cargo:rustc-env=GTA_MO_VERSION={version}");
}
