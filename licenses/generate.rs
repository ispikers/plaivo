use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

const RUNTIME: &str = r#"pub struct License {
    pub name: &'static str,
    pub version: &'static str,
    pub license: &'static str,
    parts: &'static [(u32, u32)],
}

impl License {
    pub fn parts(&self) -> impl Iterator<Item = &'static str> {
        let texts = texts();
        self.parts.iter().map(move |&(a, b)| &texts[a as usize..b as usize])
    }
}

fn texts() -> &'static str {
    static TEXTS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXTS.get_or_init(|| {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut flate2::read::DeflateDecoder::new(&include_bytes!(concat!(env!("OUT_DIR"), "/licenses.deflate"))[..]), &mut text).expect("embedded licenses inflate");
        text
    })
}

#[cfg(test)]
pub fn assert_matches_cargo_tree(package: &str) {
    let out = std::process::Command::new(env!("CARGO")).args(["tree", "--offline", "-p", package, "-e", "normal,no-proc-macro", "--prefix", "none", "-f", "{p}"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let tree: std::collections::BTreeSet<(String, String)> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter_map(|l| {
            let mut words = l.split(' ');
            Some((words.next()?.to_owned(), words.next()?.trim_start_matches('v').to_owned()))
        })
        .filter(|(name, _)| !MEMBERS.contains(&name.as_str()))
        .collect();
    let listed: std::collections::BTreeSet<(String, String)> = LICENSES[MANIFEST..].iter().map(|l| (l.name.to_owned(), l.version.to_owned())).collect();
    assert!(tree.len() > 50, "cargo tree listed only {} crates", tree.len());
    assert_eq!(listed, tree);
    for l in &LICENSES {
        assert!(!l.name.is_empty() && !l.license.is_empty() && !l.parts.is_empty(), "{} {} has no license", l.name, l.version);
        assert!(l.parts().all(|t| !t.trim().is_empty()), "{} {} has an empty license text", l.name, l.version);
    }
}
"#;

pub fn write_licenses(manifest: Option<&str>) {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let package = std::env::var("CARGO_PKG_NAME").unwrap();
    let target = std::env::var("TARGET").unwrap();
    let meta: Value = serde_json::from_slice(&cargo(&dir, &["metadata", "--format-version", "1", "--offline", "--filter-platform", &target])).unwrap();
    let workspace = PathBuf::from(s(&meta, "workspace_root"));
    let licenses = workspace.join("licenses");
    println!("cargo:rerun-if-changed={}", workspace.join("Cargo.lock").display());
    println!("cargo:rerun-if-changed={}", licenses.display());

    let member_ids: HashSet<&str> = meta["workspace_members"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    let packages = meta["packages"].as_array().unwrap();
    let members: BTreeSet<&str> = packages.iter().filter(|p| member_ids.contains(s(p, "id"))).map(|p| s(p, "name")).collect();
    let by_version: HashMap<(&str, &str), &Value> = packages.iter().map(|p| ((s(p, "name"), s(p, "version")), p)).collect();

    let mut entries: Vec<(String, String, String, Vec<String>)> = manifest.map(|m| manifest_entries(&licenses, m)).unwrap_or_default();
    let manifest_len = entries.len();
    for (name, version) in runtime_crates(&dir, &package, &target).iter().filter(|(n, _)| !members.contains(n.as_str())) {
        let p = by_version.get(&(name.as_str(), version.as_str())).unwrap_or_else(|| panic!("cargo metadata has no package {name} {version}"));
        let license = p["license"].as_str().unwrap_or("see license text").replace('/', " OR ");
        entries.push((name.clone(), version.clone(), license, license_texts(p, &licenses)));
    }

    let mut blob = String::new();
    let mut spans: HashMap<String, (usize, usize)> = HashMap::new();
    let mut rows = String::new();
    for (name, version, license, texts) in &entries {
        let parts: Vec<String> = texts
            .iter()
            .map(|t| {
                let (a, b) = *spans.entry(t.clone()).or_insert_with(|| {
                    blob.push_str(t);
                    (blob.len() - t.len(), blob.len())
                });
                format!("({a}, {b})")
            })
            .collect();
        writeln!(rows, "    License {{ name: {name:?}, version: {version:?}, license: {license:?}, parts: &[{}] }},", parts.join(", ")).unwrap();
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let mut deflate = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    deflate.write_all(blob.as_bytes()).unwrap();
    std::fs::write(out.join("licenses.deflate"), deflate.finish().unwrap()).unwrap();
    let mut src = String::from(RUNTIME);
    writeln!(src, "\n#[cfg(test)]\nconst MEMBERS: [&str; {}] = {:?};\n#[cfg(test)]\nconst MANIFEST: usize = {manifest_len};\n", members.len(), members.iter().collect::<Vec<_>>()).unwrap();
    writeln!(src, "pub static LICENSES: [License; {}] = [\n{rows}];", entries.len()).unwrap();
    std::fs::write(out.join("licenses.rs"), src).unwrap();
}

fn cargo(dir: &Path, args: &[&str]) -> Vec<u8> {
    let output = std::process::Command::new(std::env::var("CARGO").unwrap()).current_dir(dir).args(args).output().expect("run cargo");
    assert!(
        output.status.success(),
        "`cargo {}` failed; the license list needs every workspace dependency on disk, so run `cargo fetch` once:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn runtime_crates(dir: &Path, package: &str, target: &str) -> BTreeSet<(String, String)> {
    let tree = cargo(dir, &["tree", "--offline", "-p", package, "--target", target, "-e", "normal,no-proc-macro", "--prefix", "none", "-f", "{p}"]);
    String::from_utf8(tree)
        .unwrap()
        .lines()
        .filter_map(|l| {
            let mut words = l.split(' ');
            Some((words.next()?.to_owned(), words.next()?.trim_start_matches('v').to_owned()))
        })
        .collect()
}

fn manifest_entries(licenses: &Path, manifest: &str) -> Vec<(String, String, String, Vec<String>)> {
    let path = licenses.join(manifest);
    let list: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    list.as_array()
        .unwrap_or_else(|| panic!("{} must be a JSON array", path.display()))
        .iter()
        .map(|e| {
            let files = e["files"].as_array().filter(|f| !f.is_empty()).unwrap_or_else(|| panic!("{manifest}: {} lists no license files", s(e, "name")));
            let texts = files
                .iter()
                .map(|f| {
                    let f = f.as_str().unwrap_or_default();
                    let text = normalize(&std::fs::read_to_string(licenses.join(f)).unwrap_or_else(|err| panic!("licenses/{f}: {err}")));
                    assert!(!text.is_empty(), "licenses/{f} is empty");
                    text
                })
                .collect();
            assert!(!s(e, "name").is_empty() && !s(e, "license").is_empty(), "{manifest}: every entry needs a name and a license");
            (s(e, "name").to_owned(), s(e, "version").to_owned(), s(e, "license").to_owned(), texts)
        })
        .collect()
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
    if let Some(id) = ["Apache-2.0", "MPL-2.0"].into_iter().find(|id| expr.contains(id)) {
        return vec![normalize(&std::fs::read_to_string(licenses.join(format!("{id}.txt"))).unwrap())];
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
