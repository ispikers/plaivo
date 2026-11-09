use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::Path;
use std::time::Duration;

const READ_TIMEOUT: Duration = Duration::from_secs(30);
const RESERVE: u64 = 64 << 20;

pub(crate) type Response = ureq::http::Response<ureq::Body>;

#[derive(Debug, PartialEq)]
pub(crate) enum Fail {
    Abort,
    Net(String),
    Fatal(String),
}

pub(crate) fn disk(e: std::io::Error) -> Fail {
    Fail::Fatal(if e.kind() == ErrorKind::StorageFull {
        "The disk is full".into()
    } else {
        e.to_string()
    })
}

pub(crate) fn request(agent: &ureq::Agent, url: &str, from: u64) -> Result<Response, ureq::Error> {
    agent
        .get(url)
        .config()
        .timeout_global(None)
        .timeout_per_read(Some(READ_TIMEOUT))
        .http_status_as_error(false)
        .build()
        .header("Accept-Encoding", "identity")
        .header("Range", format!("bytes={from}-"))
        .call()
}

fn content_range(resp: &Response) -> Option<(&str, Option<u64>)> {
    let (range, total) = resp
        .headers()
        .get("content-range")?
        .to_str()
        .ok()?
        .strip_prefix("bytes ")?
        .split_once('/')?;
    Some((range, total.parse().ok()))
}

pub(crate) fn range_of(resp: &Response) -> Option<(u64, Option<u64>)> {
    if resp.status() != 206 {
        return Some((0, resp.body().content_length()));
    }
    let (range, total) = content_range(resp)?;
    Some((range.split_once('-')?.0.parse().ok()?, total))
}

fn restart(part: &Path, why: &str) -> Fail {
    let _ = std::fs::remove_file(part);
    Fail::Net(why.into())
}

pub(crate) fn fetch(
    agent: &ureq::Agent,
    url: &str,
    part: &Path,
    expected: u64,
    free_space: fn(&Path) -> Option<u64>,
    mut tick: impl FnMut(u64, u64) -> bool,
) -> Result<u64, Fail> {
    let have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    let resp = request(agent, url, have).map_err(|e| Fail::Net(e.to_string()))?;
    match resp.status().as_u16() {
        200 | 206 => {}
        416 if have > 0
            && content_range(&resp).and_then(|r| r.1) == Some(have)
            && (expected == 0 || expected == have) =>
        {
            return Ok(have);
        }
        416 => return Err(restart(part, "The server rejected the resume position")),
        c @ (408 | 429 | 500..) => return Err(Fail::Net(format!("The server answered {c}"))),
        c => {
            return Err(Fail::Fatal(format!(
                "The server refused the download ({c})"
            )));
        }
    }
    let (start, total) = range_of(&resp)
        .ok_or_else(|| Fail::Net("The server sent an invalid Content-Range".into()))?;
    if start != 0 && start != have {
        return Err(restart(part, "The server resumed at the wrong position"));
    }
    if start != 0 && expected != 0 && total.is_some_and(|t| t != expected) {
        return Err(restart(part, "The file changed on the server"));
    }
    if let (Some(total), Some(free)) = (total, free_space(part.parent().unwrap_or(Path::new("."))))
        && free < total.saturating_sub(start) + RESERVE
    {
        return Err(Fail::Fatal(
            "Not enough free space for this download".into(),
        ));
    }
    let mut file = if start > 0 {
        OpenOptions::new().append(true).open(part)
    } else {
        File::create(part)
    }
    .map_err(disk)?;
    let mut body = resp.into_body().into_reader();
    let mut buf = vec![0; 256 << 10];
    let mut done = start;
    loop {
        let n = match body.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(Fail::Net(e.to_string())),
        };
        file.write_all(&buf[..n]).map_err(disk)?;
        done += n as u64;
        if !tick(done, total.unwrap_or(0)) {
            return Err(Fail::Abort);
        }
    }
    file.sync_all().map_err(disk)?;
    match total {
        Some(t) if t != done => Err(Fail::Net("The connection closed early".into())),
        _ => Ok(done),
    }
}

#[cfg(windows)]
pub(crate) fn free_space(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            dir: *const u16,
            available: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
    let mut available = 0u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}

#[cfg(unix)]
#[allow(clippy::unnecessary_cast)]
pub(crate) fn free_space(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(path.as_ptr(), &mut st) } == 0)
        .then(|| st.f_bavail as u64 * st.f_frsize as u64)
}

#[cfg(not(any(windows, unix)))]
pub(crate) fn free_space(_: &Path) -> Option<u64> {
    None
}
