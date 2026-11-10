#[path = "../../deps/libmpv.rs"]
mod pin;

use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, bail, ensure};
use sha2::{Digest, Sha256};

const REPO: &str = "ispikers/plaivo";

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    match args.first().map(String::as_str) {
        Some("libmpv") => libmpv(),
        Some("bundle") => bundle(flag("--release"), flag("--open")),
        _ => bail!("usage: cargo run -p xtask -- libmpv | bundle [--release] [--open]"),
    }
}

fn bundle(release: bool, open: bool) -> anyhow::Result<()> {
    ensure!(
        cfg!(target_os = "macos"),
        "bundle builds the macOS Plaivo.app"
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .args(["build", "-p", "plaivo"])
        .args(release.then_some("--release"))
        .current_dir(root)
        .status()?;
    ensure!(status.success(), "cargo build failed");

    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"))
        .join(if release { "release" } else { "debug" });
    let app = target.join("Plaivo.app");
    let contents = app.join("Contents");
    let exe_dir = contents.join("MacOS");
    if app.exists() {
        std::fs::remove_dir_all(&app)?;
    }
    std::fs::create_dir_all(exe_dir.join("shaders"))?;
    std::fs::copy(target.join("plaivo"), exe_dir.join("plaivo"))?;
    copy_files(&root.join("deps/macos"), &exe_dir)?;
    copy_files(&root.join("shaders"), &exe_dir.join("shaders"))?;

    let manifest = std::fs::read_to_string(root.join("Cargo.toml"))?;
    let version = manifest
        .lines()
        .find_map(|l| l.strip_prefix("version = \""))
        .and_then(|v| v.strip_suffix('"'))
        .context("no version in Cargo.toml")?;
    std::fs::write(contents.join("Info.plist"), info_plist(version))?;

    let status = Command::new("codesign")
        .args(["--force", "--deep", "--sign", "-"])
        .arg(&app)
        .status()?;
    ensure!(status.success(), "codesign failed");
    println!("{}", app.display());
    if open {
        Command::new("open").arg(&app).status()?;
    }
    Ok(())
}

fn copy_files(src: &Path, dst: &Path) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let path = entry?.path();
        if path.is_file() && !path.file_name().unwrap().to_string_lossy().starts_with('.') {
            std::fs::copy(&path, dst.join(path.file_name().unwrap()))?;
        }
    }
    Ok(())
}

fn info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Plaivo</string>
    <key>CFBundleDisplayName</key>
    <string>Plaivo</string>
    <key>CFBundleIdentifier</key>
    <string>xyz.spikers.plaivo</string>
    <key>CFBundleExecutable</key>
    <string>plaivo</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>LSMinimumSystemVersion</key>
    <string>15.0</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.entertainment</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
"#
    )
}

fn libmpv() -> anyhow::Result<()> {
    let (os, asset) = if cfg!(windows) {
        ("windows", "libmpv-windows-x64.tar.gz")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        ("macos", "libmpv-macos-arm64.tar.gz")
    } else {
        bail!("libmpv is only built for Windows x64 and macOS arm64");
    };
    let deps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../deps");
    let pin = pin::read(&deps, os).context("cannot read deps/libmpv.env")?;
    ensure!(
        !pin.sha256.is_empty(),
        "deps/libmpv.env has no {}_SHA256: run the libmpv workflow, then paste the hashes from its summary",
        os.to_uppercase()
    );
    let dir = deps.join(os);
    if pin::installed(&dir, &pin) {
        println!("deps/{os} already has {}", pin.tag);
        return Ok(());
    }

    let archive = download(&pin.tag, asset)?;
    let sha256 = format!("{:x}", Sha256::digest(&archive));
    ensure!(
        sha256 == pin.sha256,
        "{asset} in {} has sha256 {sha256}, but deps/libmpv.env pins {}",
        pin.tag,
        pin.sha256
    );

    std::fs::create_dir_all(&dir)?;
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.is_file() && path.file_name() != Some(OsStr::new(".gitkeep")) {
            std::fs::remove_file(path)?;
        }
    }
    std::fs::write(dir.join(asset), &archive)?;
    let status = Command::new("tar")
        .args(["-xzf", asset])
        .current_dir(&dir)
        .status()
        .context("running tar")?;
    std::fs::remove_file(dir.join(asset))?;
    ensure!(status.success(), "tar could not extract {asset}");
    std::fs::write(dir.join(pin::STAMP), &pin.sha256)?;
    println!(
        "installed {} (mpv {}, FFmpeg {}) into deps/{os}",
        pin.tag, pin.mpv, pin.ffmpeg
    );
    Ok(())
}

fn download(tag: &str, asset: &str) -> anyhow::Result<Vec<u8>> {
    let token = github_token();
    let get = |url: &str, accept: &str| {
        let mut req = ureq::get(url).header("Accept", accept);
        if let Some(t) = &token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        req.call()
    };
    let release: serde_json::Value = serde_json::from_str(
        &get(
            &format!("https://api.github.com/repos/{REPO}/releases/tags/{tag}"),
            "application/vnd.github+json",
        )
        .with_context(|| format!("release {tag} not found on {REPO}; run the libmpv workflow"))?
        .into_body()
        .read_to_string()?,
    )?;
    let url = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"] == asset)
        .and_then(|a| a["url"].as_str())
        .with_context(|| format!("release {tag} has no {asset}"))?;
    println!("downloading {asset} from {tag}");
    let mut body = get(url, "application/octet-stream")?.into_body();
    Ok(body.with_config().limit(1 << 30).read_to_vec()?)
}

fn github_token() -> Option<String> {
    if let Some(t) = ["GITHUB_TOKEN", "GH_TOKEN"]
        .into_iter()
        .find_map(|k| std::env::var(k).ok())
    {
        return Some(t);
    }
    let mut git = Command::new("git")
        .args(["credential", "fill"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    git.stdin
        .take()?
        .write_all(b"protocol=https\nhost=github.com\n\n")
        .ok()?;
    let out = git.wait_with_output().ok()?;
    String::from_utf8(out.stdout)
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("password=").map(str::to_owned))
}
