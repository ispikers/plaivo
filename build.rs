use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    write_notices(&root, &out);
    copy_runtime_libs(&root, &out);
}

// Copies the runtime libraries in deps/<os>/ (libmpv) next to the built executable, so every profile runs as-is.
fn copy_runtime_libs(root: &Path, out: &Path) {
    let os = match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => "windows",
        Ok("macos") => "macos",
        _ => return,
    };
    let src = root.join("deps").join(os);
    println!("cargo:rerun-if-changed={}", src.display());
    let Some(profile_dir) = out.ancestors().nth(3) else { return };
    let Ok(entries) = std::fs::read_dir(&src) else {
        println!("cargo:warning={} is missing; put libmpv there or playback will not work", src.display());
        return;
    };
    for entry in entries.flatten() {
        let from = entry.path();
        if !from.is_file() || entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let to = profile_dir.join(entry.file_name());
        let stale = match (from.metadata(), to.metadata()) {
            (Ok(a), Ok(b)) => a.len() != b.len() || a.modified().ok() > b.modified().ok(),
            _ => true,
        };
        if stale && let Err(e) = std::fs::copy(&from, &to) {
            println!("cargo:warning=could not copy {} to {}: {e}", from.display(), to.display());
        }
    }
}

fn write_notices(root: &Path, out: &Path) {
    let licenses = root.join("licenses");
    println!("cargo:rerun-if-changed=Cargo.lock");
    println!("cargo:rerun-if-changed={}", licenses.display());
    let read = |name: &str| normalize(&std::fs::read_to_string(licenses.join(name)).unwrap_or_else(|e| panic!("licenses/{name}: {e}")));

    let crates = runtime_crates(root);
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in &crates {
        let label = format!("{} {} ({})", s(p, "name"), s(p, "version"), p["license"].as_str().unwrap_or("see license text"));
        for text in license_texts(p, &licenses) {
            groups.entry(text).or_default().push(label.clone());
        }
    }
    let mut groups: Vec<(String, Vec<String>)> = groups.into_iter().collect();
    groups.sort_by(|a, b| a.1[0].cmp(&b.1[0]));

    let rule = "=".repeat(80);
    let mut doc = format!(
        "Plaivo third-party notices\n{rule}\n\nPlaivo is built on the open-source software listed below. Each component is\nused under the license reproduced with it.\n\n{rule}\n{}\n\n{rule}\n{}\n\n{rule}\n{}\n\n{rule}\n{}\n\n{rule}\nRust crates\n-----------\n\nThe Plaivo executable contains the following {} Rust crates.\n",
        read("libmpv.txt"),
        read("LGPL-2.1.txt"),
        read("LGPL-3.0.txt"),
        read("GPL-3.0.txt"),
        crates.len(),
    );
    for (text, labels) in &groups {
        doc.push_str(&format!("\n{rule}\n{}\n{}\n\n{text}\n", labels.join("\n"), "-".repeat(80)));
    }
    std::fs::write(out.join("THIRD-PARTY-NOTICES.txt"), doc).unwrap();
    println!("cargo:rustc-env=PLAIVO_CRATE_COUNT={}", crates.len());
}

fn runtime_crates(root: &Path) -> Vec<Value> {
    let target = std::env::var("TARGET").unwrap();
    let output = std::process::Command::new(std::env::var("CARGO").unwrap())
        .current_dir(root)
        .args(["metadata", "--format-version", "1", "--offline", "--filter-platform", &target])
        .output()
        .expect("run cargo metadata");
    assert!(output.status.success(), "cargo metadata failed: {}", String::from_utf8_lossy(&output.stderr));
    let meta: Value = serde_json::from_slice(&output.stdout).unwrap();
    let packages: HashMap<&str, &Value> = meta["packages"].as_array().unwrap().iter().map(|p| (s(p, "id"), p)).collect();
    let members: HashSet<&str> = meta["workspace_members"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    let normal_deps: HashMap<&str, Vec<&str>> = meta["resolve"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            let deps = n["deps"].as_array().unwrap().iter().filter(|d| d["dep_kinds"].as_array().unwrap().iter().any(|k| k["kind"].is_null())).map(|d| s(d, "pkg")).collect();
            (s(n, "id"), deps)
        })
        .collect();
    let me = packages.values().find(|p| s(p, "name") == std::env::var("CARGO_PKG_NAME").unwrap()).map(|p| s(p, "id")).unwrap();
    let mut seen = HashSet::new();
    let mut stack = vec![me];
    while let Some(id) = stack.pop() {
        if seen.insert(id) {
            stack.extend(&normal_deps[id]);
        }
    }
    let mut crates: Vec<Value> = seen.into_iter().filter(|id| !members.contains(id)).map(|id| packages[id].clone()).collect();
    crates.sort_by(|a, b| (s(a, "name"), s(a, "version")).cmp(&(s(b, "name"), s(b, "version"))));
    crates
}

fn license_texts(p: &Value, licenses: &Path) -> Vec<String> {
    let dir = Path::new(s(p, "manifest_path")).parent().unwrap();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|f| f.is_file() && is_license_file(f))
        .collect();
    if let Some(f) = p["license_file"].as_str() {
        files.push(dir.join(f));
    }
    files.sort();
    files.dedup();
    let texts: Vec<String> = files.iter().filter_map(|f| read_license(f)).collect();
    if !texts.is_empty() {
        return texts;
    }
    let expr = p["license"].as_str().unwrap_or_default();
    if expr.contains("Apache-2.0") {
        return vec![normalize(&std::fs::read_to_string(licenses.join("Apache-2.0.txt")).unwrap())];
    }
    if expr.contains("MIT") {
        let authors: Vec<&str> = p["authors"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|a| a.split(" <").next().unwrap_or(a)).collect();
        let holder = if authors.is_empty() { format!("the {} authors", s(p, "name")) } else { authors.join(", ") };
        return vec![normalize(&std::fs::read_to_string(licenses.join("MIT.txt")).unwrap().replace("{copyright}", &holder))];
    }
    panic!("{} ships no license file and its license ({expr}) has no fallback text in licenses/", s(p, "name"));
}

fn is_license_file(f: &Path) -> bool {
    let name = f.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    ["license", "licence", "copying", "copyright", "notice", "unlicense"].iter().any(|p| name.starts_with(p))
}

fn read_license(f: &Path) -> Option<String> {
    let text = normalize(&std::fs::read_to_string(f).ok()?);
    // Git checkouts on Windows store symlinks as small files holding the target path.
    if !text.contains('\n') && text.starts_with("..") {
        return std::fs::read_to_string(f.parent()?.join(&text)).ok().map(|t| normalize(&t));
    }
    (!text.is_empty()).then_some(text)
}

fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").trim().to_owned()
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}
