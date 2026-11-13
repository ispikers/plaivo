#[path = "deps/libmpv.rs"]
mod libmpv;
#[path = "licenses/generate.rs"]
mod licenses;

use std::path::{Path, PathBuf};

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    licenses::write_licenses(Some("desktop.json"));
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let deps = root.join("deps");
    println!(
        "cargo:rerun-if-changed={}",
        deps.join("libmpv.env").display()
    );
    let pin = libmpv::read(&deps, &os);
    let (mpv, ffmpeg) = pin.as_ref().map_or(("", ""), |p| (&p.mpv, &p.ffmpeg));
    println!("cargo:rustc-env=PLAIVO_MPV_VERSION={mpv}");
    println!("cargo:rustc-env=PLAIVO_FFMPEG_VERSION={ffmpeg}");
    if matches!(os.as_str(), "windows" | "macos") {
        copy_runtime_libs(&root, &out, &deps.join(&os), pin);
    }
    if os == "windows" {
        // GPUI loads icon resource 1 from the exe for the window and taskbar.
        println!("cargo:rerun-if-changed=resources/app.ico");
        embed_resource::compile(root.join("resources/app.rc"), embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}

// Copies the runtime libraries in deps/<os>/ (libmpv) and the mpv shaders next to the built executable, so every profile runs as-is.
fn copy_runtime_libs(root: &Path, out: &Path, src: &Path, pin: Option<libmpv::Pin>) {
    let Some(profile_dir) = out.ancestors().nth(3) else {
        return;
    };
    copy_files(src, profile_dir);
    copy_files(&root.join("shaders"), &profile_dir.join("shaders"));
    match pin {
        Some(pin) if !libmpv::installed(src, &pin) => println!(
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
