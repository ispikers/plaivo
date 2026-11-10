use std::path::Path;

pub const STAMP: &str = ".libmpv";

pub struct Pin {
    pub tag: String,
    pub sha256: String,
}

pub fn read(deps: &Path, os: &str) -> Option<Pin> {
    let text = std::fs::read_to_string(deps.join("libmpv.env")).ok()?;
    let get = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim().to_owned())
            .unwrap_or_default()
    };
    Some(Pin {
        tag: get("TAG"),
        sha256: get(&format!("{}_SHA256", os.to_uppercase())),
    })
}

pub fn installed(dir: &Path, pin: &Pin) -> bool {
    !pin.sha256.is_empty()
        && std::fs::read_to_string(dir.join(STAMP)).is_ok_and(|s| s.trim() == pin.sha256)
}
