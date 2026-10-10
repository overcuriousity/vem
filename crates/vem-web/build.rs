//! A release build must embed the frontend: refuse to build one without `frontend/dist/index.html`.

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let dist = std::path::Path::new(&manifest).join("../../frontend/dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    println!("cargo:rerun-if-env-changed=VEM_ALLOW_NO_FRONTEND");
    let release = std::env::var("PROFILE")
        .map(|p| p == "release")
        .unwrap_or(false);
    if release
        && !dist.join("index.html").is_file()
        && std::env::var_os("VEM_ALLOW_NO_FRONTEND").is_none()
    {
        panic!(
            "frontend/dist/index.html is missing. Run scripts/build-release.sh (or `cd frontend && npm ci && npm run build`) \
             before a release build, or set VEM_ALLOW_NO_FRONTEND=1 to build a binary without the UI."
        );
    }
}
