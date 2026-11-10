#[path = "licenses/generate.rs"]
mod licenses;
#[path = "deps/libmpv.rs"]
mod libmpv;

use std::path::{Path, PathBuf};

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    licenses::write_licenses(Some("desktop.json"));
    copy_runtime_libs(&root, &out);
}

// Copies the runtime libraries in deps/<os>/ (libmpv) and the mpv shaders next to the built executable, so every profile runs as-is.
fn copy_runtime_libs(root: &Path, out: &Path) {
    let os = match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => "windows",
        Ok("macos") => "macos",
        _ => return,
    };
    let Some(profile_dir) = out.ancestors().nth(3) else {
        return;
    };
    let deps = root.join("deps");
    let src = deps.join(os);
    copy_files(&src, profile_dir);
    copy_files(&root.join("shaders"), &profile_dir.join("shaders"));
    println!("cargo:rerun-if-changed={}", deps.join("libmpv.env").display());
    match libmpv::read(&deps, os) {
        Some(pin) if !libmpv::installed(&src, &pin) => println!(
            "cargo:warning={} is not {}; run `cargo run -p xtask -- libmpv` or playback may not work",
            src.display(),
            pin.tag
        ),
        None => println!("cargo:warning=deps/libmpv.env is missing"),
        _ => {}
    }
}

fn copy_files(src: &Path, dst: &Path) {
    println!("cargo:rerun-if-changed={}", src.display());
    let Ok(entries) = std::fs::read_dir(src) else {
        return;
    };
    let _ = std::fs::create_dir_all(dst);
    for entry in entries.flatten() {
        let from = entry.path();
        if !from.is_file() || entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let to = dst.join(entry.file_name());
        let stale = match (from.metadata(), to.metadata()) {
            (Ok(a), Ok(b)) => a.len() != b.len() || a.modified().ok() > b.modified().ok(),
            _ => true,
        };
        if stale && let Err(e) = std::fs::copy(&from, &to) {
            println!(
                "cargo:warning=could not copy {} to {}: {e}",
                from.display(),
                to.display()
            );
        }
    }
}
