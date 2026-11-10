#[path = "../../deps/libmpv.rs"]
mod pin;

use std::ffi::OsStr;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, bail, ensure};
use sha2::{Digest, Sha256};

const REPO: &str = "ispikers/plaivo";

fn main() -> anyhow::Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("libmpv") => libmpv(),
        _ => bail!("usage: cargo run -p xtask -- libmpv"),
    }
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
    println!("installed {} into deps/{os}", pin.tag);
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
