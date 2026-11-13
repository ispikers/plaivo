#[path = "../../deps/libmpv.rs"]
mod pin;

use std::ffi::OsStr;
use std::io::{BufRead, Write};
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
        Some("bundle") => bundle(flag("--release"), flag("--open")).map(drop),
        Some("installer") => installer(),
        Some("release") => release(args.get(1).context("usage: release <version>")?),
        _ => bail!(
            "usage: cargo run -p xtask -- libmpv | bundle [--release] [--open] | installer | release <version>"
        ),
    }
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

fn version() -> anyhow::Result<String> {
    let manifest = std::fs::read_to_string(root().join("Cargo.toml"))?;
    manifest
        .lines()
        .find_map(|l| l.strip_prefix("version = \"")?.strip_suffix('"'))
        .map(str::to_owned)
        .context("no version in Cargo.toml")
}

fn run(cmd: &mut Command) -> anyhow::Result<()> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let status = cmd.status().with_context(|| format!("running {name}"))?;
    ensure!(status.success(), "{name} failed");
    Ok(())
}

fn build(release: bool) -> anyhow::Result<PathBuf> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    run(Command::new(cargo)
        .args(["build", "-p", "plaivo"])
        .args(release.then_some("--release"))
        // Static CRT: the installer doesn't ship VCRUNTIME140.dll.
        .args([
            "--config",
            r#"target.'cfg(target_env = "msvc")'.rustflags = ["-C", "target-feature=+crt-static"]"#,
        ])
        .current_dir(root()))?;
    Ok(std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("target"))
        .join(if release { "release" } else { "debug" }))
}

fn parse_version(v: &str) -> anyhow::Result<[u64; 3]> {
    let parts: Vec<u64> = v
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .with_context(|| format!("{v} is not a version like 1.2.3"))?;
    parts
        .try_into()
        .map_err(|_| anyhow::anyhow!("{v} is not a version like 1.2.3"))
}

fn ensure_libmpv(os: &str) -> anyhow::Result<PathBuf> {
    let deps = root().join("deps");
    let dir = deps.join(os);
    let pin = pin::read(&deps, os).context("cannot read deps/libmpv.env")?;
    ensure!(
        pin::installed(&dir, &pin),
        "deps/{os} is not {}; run `cargo run -p xtask -- libmpv` first",
        pin.tag
    );
    Ok(dir)
}

// Delta packages need the previous release in target/velopack first (`vpk download github`, as CI does).
fn installer() -> anyhow::Result<()> {
    let version = version()?;
    let resources = root().join("xtask/resources");
    let mut vpk = Command::new("vpk");
    vpk.args([
        "pack",
        "--packId",
        "Plaivo",
        "--packTitle",
        "Plaivo",
        "--packAuthors",
        "Plaivo",
    ])
    .args(["--packVersion", &version])
    .arg("--outputDir")
    .arg(root().join("target/velopack"));
    if cfg!(windows) {
        let deps = ensure_libmpv("windows")?;
        let target = build(true)?;
        let stage = target.join("velopack-stage");
        if stage.exists() {
            std::fs::remove_dir_all(&stage)?;
        }
        std::fs::create_dir_all(stage.join("shaders"))?;
        std::fs::copy(target.join("plaivo.exe"), stage.join("plaivo.exe"))?;
        copy_files(&deps, &stage)?;
        copy_files(&root().join("shaders"), &stage.join("shaders"))?;
        vpk.args([
            "--mainExe",
            "plaivo.exe",
            "--runtime",
            "win-x64",
            "--shortcuts",
            "StartMenuRoot",
        ])
        .arg("--packDir")
        .arg(&stage)
        .arg("--icon")
        .arg(root().join("resources/app.ico"))
        .arg("--splashImage")
        .arg(resources.join("splash.png"))
        .args(["--splashProgressColor", "#E6E6E6"]);
        if let Ok(cert) = std::env::var("PLAIVO_SIGN_CERT") {
            vpk.args([
                "--signParams",
                &format!("/sha1 {cert} /fd sha256 /td sha256 /tr http://timestamp.digicert.com"),
            ]);
        }
    } else {
        ensure_libmpv("macos")?;
        vpk.args(["--mainExe", "plaivo", "--runtime", "osx-arm64"])
            .arg("--packDir")
            .arg(bundle(true, false)?);
        for (var, flag) in [
            ("PLAIVO_SIGN_IDENTITY", "--signAppIdentity"),
            ("PLAIVO_SIGN_INSTALL_IDENTITY", "--signInstallIdentity"),
            ("PLAIVO_NOTARY_PROFILE", "--notaryProfile"),
        ] {
            if let Ok(value) = std::env::var(var) {
                vpk.args([flag, &value]);
            }
        }
    }
    run(&mut vpk)?;
    println!("{}", root().join("target/velopack").display());
    Ok(())
}

fn capture(cmd: &mut Command) -> anyhow::Result<String> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let out = cmd
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("running {name}"))?;
    ensure!(out.status.success(), "{name} failed");
    Ok(String::from_utf8(out.stdout)?.trim().to_owned())
}

fn confirm(question: &str) -> anyhow::Result<bool> {
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

// Bumps the version, tags and pushes it, waits for the release workflow, then publishes the draft it made.
fn release(wanted: &str) -> anyhow::Result<()> {
    let (next, current) = (wanted.to_owned(), version()?);
    ensure!(
        parse_version(&next)? > parse_version(&current)?,
        "{next} is not newer than the current {current}"
    );
    let tag = format!("v{next}");
    let git = |args: &[&str]| capture(Command::new("git").args(args).current_dir(root()));
    ensure!(
        git(&["rev-parse", "--abbrev-ref", "HEAD"])? == "main",
        "releases are cut from main"
    );
    ensure!(
        git(&["status", "--porcelain"])?.is_empty(),
        "commit or stash your changes first"
    );
    git(&["fetch", "origin", "main", "--tags"])?;
    ensure!(
        git(&["rev-list", "--count", "HEAD..origin/main"])? == "0",
        "main is behind origin/main; pull first"
    );
    ensure!(
        git(&["tag", "--list", &tag])?.is_empty(),
        "{tag} already exists"
    );

    let manifest_path = root().join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path)?;
    let line = format!("version = \"{current}\"");
    ensure!(manifest.contains(&line), "no `{line}` in Cargo.toml");
    std::fs::write(
        &manifest_path,
        manifest.replacen(&line, &format!("version = \"{next}\""), 1),
    )?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    run(Command::new(cargo)
        .args(["update", "-p", "plaivo", "--offline"])
        .current_dir(root()))?;
    git(&["add", "Cargo.toml", "Cargo.lock"])?;
    git(&["commit", "-m", &format!("chore: release {tag}")])?;
    git(&["tag", "-a", &tag, "-m", &format!("Plaivo {tag}")])?;
    if !confirm(&format!(
        "Committed and tagged {tag} locally. Push main and {tag} to start the release build?"
    ))? {
        println!("Not pushed. Push later with: git push --atomic origin main {tag}");
        return Ok(());
    }
    git(&["push", "--atomic", "origin", "main", &tag])?;

    println!("Waiting for the release workflow to start…");
    let run_id = (0..60)
        .find_map(|_| {
            std::thread::sleep(std::time::Duration::from_secs(5));
            capture(Command::new("gh").args([
                "run",
                "list",
                "--repo",
                REPO,
                "--workflow",
                "release.yml",
                "--branch",
                &tag,
                "--json",
                "databaseId",
                "--jq",
                ".[0].databaseId",
            ]))
            .ok()
            .filter(|id| !id.is_empty())
        })
        .context("the release workflow didn't start within 5 minutes")?;
    run(Command::new("gh").args(["run", "watch", &run_id, "--repo", REPO, "--exit-status"]))?;

    run(Command::new("gh").args(["release", "view", &tag, "--repo", REPO]))?;
    if confirm(&format!("Publish {tag} to everyone?"))? {
        run(Command::new("gh").args(["release", "edit", &tag, "--repo", REPO, "--draft=false"]))?;
        println!("Published. Installed copies pick it up within an hour.");
    } else {
        println!(
            "Left as a draft. Publish with: gh release edit {tag} --repo {REPO} --draft=false"
        );
    }
    Ok(())
}

fn bundle(release: bool, open: bool) -> anyhow::Result<PathBuf> {
    ensure!(
        cfg!(target_os = "macos"),
        "bundle builds the macOS Plaivo.app"
    );
    let root = root();
    let target = build(release)?;
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

    std::fs::write(contents.join("Info.plist"), info_plist(&version()?))?;
    std::fs::create_dir_all(contents.join("Resources"))?;
    std::fs::copy(
        root.join("resources/app.icns"),
        contents.join("Resources/app.icns"),
    )?;

    match std::env::var("PLAIVO_SIGN_IDENTITY") {
        Ok(identity) => {
            let sign = |path: &Path| {
                run(Command::new("codesign")
                    .args([
                        "--force",
                        "--timestamp",
                        "--options",
                        "runtime",
                        "--sign",
                        &identity,
                    ])
                    .arg(path))
            };
            for entry in std::fs::read_dir(&exe_dir)? {
                let path = entry?.path();
                if path.extension() == Some(OsStr::new("dylib")) {
                    sign(&path)?;
                }
            }
            sign(&app)?;
        }
        Err(_) => run(Command::new("codesign")
            .args(["--force", "--deep", "--sign", "-"])
            .arg(&app))?,
    }
    println!("{}", app.display());
    if open {
        Command::new("open").arg(&app).status()?;
    }
    Ok(app)
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
    <key>CFBundleIconFile</key>
    <string>app</string>
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
