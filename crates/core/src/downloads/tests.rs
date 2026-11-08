use super::*;
use crate::plex::Backend;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

#[derive(Clone, Debug)]
struct Req {
    method: String,
    path: String,
    range: Option<u64>,
    body: String,
}

struct Resp {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    cut: Option<usize>,
    paced: bool,
}

impl Resp {
    fn ok(body: impl Into<Vec<u8>>) -> Self {
        Resp { status: 200, headers: Vec::new(), body: body.into(), cut: None, paced: false }
    }

    fn status(status: u16) -> Self {
        Resp { status, ..Resp::ok("") }
    }
}

type Log = Arc<Mutex<Vec<Req>>>;
type Handler = dyn Fn(&Req) -> Resp + Send + Sync;

fn serve(handler: impl Fn(&Req) -> Resp + Send + Sync + 'static) -> (String, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::default();
    let (handler, sink): (Arc<Handler>, Log) = (Arc::new(handler), log.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (handler, sink) = (handler.clone(), sink.clone());
            std::thread::spawn(move || handle(stream, &*handler, &sink));
        }
    });
    (base, log)
}

fn handle(stream: TcpStream, handler: &Handler, log: &Log) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut words = line.split_whitespace().map(str::to_owned);
    let (method, path) = (words.next().unwrap_or_default(), words.next().unwrap_or_default());
    let (mut range, mut len) = (None, 0);
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let Some((k, v)) = header.trim_end().split_once(':') else { break };
        match k.to_ascii_lowercase().as_str() {
            "range" => range = v.trim().strip_prefix("bytes=").and_then(|r| r.trim_end_matches('-').parse().ok()),
            "content-length" => len = v.trim().parse().unwrap_or(0),
            _ => {}
        }
    }
    let mut body = vec![0; len];
    reader.read_exact(&mut body)?;
    let req = Req { method, path, range, body: String::from_utf8_lossy(&body).into_owned() };
    let resp = handler(&req);
    log.lock().unwrap().push(req);
    let mut head = format!("HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n", resp.status, resp.body.len());
    for (k, v) in &resp.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let mut out = stream;
    out.write_all(head.as_bytes())?;
    let body = &resp.body[..resp.cut.unwrap_or(usize::MAX).min(resp.body.len())];
    if resp.paced {
        for chunk in body.chunks(8 << 10) {
            out.write_all(chunk)?;
            std::thread::sleep(Duration::from_millis(5));
        }
    } else {
        out.write_all(body)?;
    }
    out.flush()
}

fn media(data: &[u8], req: &Req, honour: bool, cut: Option<usize>) -> Resp {
    match req.range.filter(|_| honour) {
        Some(from) if from as usize >= data.len() => Resp { headers: vec![("Content-Range", format!("bytes */{}", data.len()))], ..Resp::status(416) },
        Some(from) => Resp {
            status: 206,
            headers: vec![("Content-Range", format!("bytes {from}-{}/{}", data.len() - 1, data.len()))],
            body: data[from as usize..].to_vec(),
            cut,
            paced: false,
        },
        None => Resp { body: data.to_vec(), cut, ..Resp::ok("") },
    }
}

fn bytes(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 31 % 251) as u8).collect()
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plaivo-dl-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn plenty(_: &Path) -> Option<u64> {
    Some(u64::MAX / 2)
}

fn no_space(_: &Path) -> Option<u64> {
    Some(0)
}

fn engine(root: &Path, free_space: fn(&Path) -> Option<u64>) -> Downloads {
    Downloads::start(root.to_owned(), Client::new("test".into()), free_space, Box::new(|| {}))
}

fn until(what: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !what() {
        assert!(Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait(d: &Downloads, what: impl Fn(&[Entry]) -> bool) -> Vec<Entry> {
    until(|| what(&d.snapshot()));
    d.snapshot()
}

fn episode(key: u32, full: bool) -> Value {
    let mut v = json!({
        "ratingKey": key.to_string(), "type": "episode", "title": format!("Ep {key}"), "grandparentTitle": "Show",
        "parentRatingKey": "110", "grandparentRatingKey": "100", "parentIndex": 1, "index": key - 100, "duration": 1000,
        "thumb": format!("/library/metadata/{key}/thumb/1"), "parentThumb": "/library/metadata/110/thumb/1", "grandparentThumb": "/library/metadata/100/thumb/1",
    });
    if full {
        v["Media"] = json!([{"container": "mkv", "Part": [{"key": format!("/library/parts/{key}/file.mkv"), "file": format!("/tv/Show/S01E0{}.mkv", key - 100), "Stream": [
            {"streamType": 1, "codec": "h264"},
            {"streamType": 3, "key": "/library/streams/7", "codec": "srt", "languageCode": "eng", "displayTitle": "English (SRT External)"},
            {"streamType": 3, "codec": "ass", "displayTitle": "Embedded"},
        ]}]}]);
    }
    v
}

fn fake_plex(data: Arc<Vec<u8>>, cut_first: bool, paced: bool) -> (Server, Log) {
    let cut_done = AtomicBool::new(!cut_first);
    let (base, log) = serve(move |req| {
        let wrap = |m: Value| Resp::ok(json!({"MediaContainer": {"Metadata": m}}).to_string());
        match req.path.split('?').next().unwrap() {
            "/library/metadata/100/allLeaves" => wrap(json!([episode(101, false), episode(102, false)])),
            "/library/metadata/101" => wrap(json!([episode(101, true)])),
            "/library/metadata/102" => wrap(json!([episode(102, true)])),
            "/library/metadata/110" => wrap(json!([{"ratingKey": "110", "type": "season", "title": "Season 1", "parentRatingKey": "100", "thumb": "/library/metadata/110/thumb/1"}])),
            "/library/metadata/100" => wrap(json!([{"ratingKey": "100", "type": "show", "title": "Show", "thumb": "/library/metadata/100/thumb/1", "art": "/library/metadata/100/art/1"}])),
            "/library/parts/101/file.mkv" if !cut_done.swap(true, Ordering::SeqCst) => media(&data, req, true, Some(data.len() / 3)),
            p if p.starts_with("/library/parts/") => Resp { paced, ..media(&data, req, true, None) },
            "/library/streams/7" => Resp::ok("1\n00:00:01,000 --> 00:00:02,000\nHi\n"),
            "/photo/:/transcode" => Resp::ok("jpeg"),
            _ => Resp::status(404),
        }
    });
    (Server { name: "fake".into(), id: "srv1".into(), uri: base, token: "t".into(), ..Default::default() }, log)
}

fn show() -> Item {
    Item { rating_key: "100".into(), kind: "show".into(), title: "Show".into(), ..Default::default() }
}

fn ep(key: &str) -> Item {
    Item { rating_key: key.into(), kind: "episode".into(), ..Default::default() }
}

fn part(root: &Path, key: &str) -> PathBuf {
    root.join("srv1").join(key).join("media.mkv.part")
}

#[test]
fn range_resume_after_interruption() {
    let data = Arc::new(bytes(1 << 20));
    let first = AtomicBool::new(true);
    let served = data.clone();
    let (base, log) = serve(move |req| media(&served, req, true, first.swap(false, Ordering::SeqCst).then_some(300_000)));
    let dir = temp("resume");
    let part = dir.join("media.mkv.part");
    let agent = Client::new("t".into()).agent;
    let url = format!("{base}/file.mkv");
    assert!(matches!(fetch::fetch(&agent, &url, &part, 0, plenty, |_, _| true), Err(Fail::Net(_))));
    let have = std::fs::metadata(&part).unwrap().len();
    assert!(have > 0 && have < data.len() as u64, "{have}");
    let mut ticks = 0;
    let len = data.len() as u64;
    assert_eq!(fetch::fetch(&agent, &url, &part, len, plenty, |_, total| { ticks += 1; total == len }), Ok(len));
    assert!(ticks > 0);
    assert_eq!(std::fs::read(&part).unwrap(), *data);
    assert_eq!(log.lock().unwrap().iter().map(|r| r.range).collect::<Vec<_>>(), [Some(0), Some(have)]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn range_edge_cases() {
    let data = Arc::new(bytes(200_000));
    let len = data.len() as u64;
    let agent = Client::new("t".into()).agent;
    let dir = temp("ranges");
    let part = dir.join("media.mkv.part");

    let served = data.clone();
    let (ignoring, _) = serve(move |req| media(&served, req, false, None));
    std::fs::write(&part, vec![7u8; 5000]).unwrap();
    assert_eq!(fetch::fetch(&agent, &format!("{ignoring}/f"), &part, 0, plenty, |_, _| true), Ok(len), "a server ignoring Range restarts from zero");
    assert_eq!(std::fs::read(&part).unwrap(), *data);

    let served = data.clone();
    let (honouring, _) = serve(move |req| media(&served, req, true, None));
    let url = format!("{honouring}/f");
    assert_eq!(fetch::fetch(&agent, &url, &part, len, plenty, |_, _| true), Ok(len), "416 on a complete .part keeps it");
    assert_eq!(std::fs::read(&part).unwrap(), *data);

    std::fs::write(&part, vec![1u8; data.len() + 10]).unwrap();
    assert!(matches!(fetch::fetch(&agent, &url, &part, 0, plenty, |_, _| true), Err(Fail::Net(_))));
    assert!(!part.exists(), "a rejected resume range starts over");

    std::fs::write(&part, &data[..1000]).unwrap();
    assert_eq!(fetch::fetch(&agent, &url, &part, len + 1, plenty, |_, _| true), Err(Fail::Net("The file changed on the server".into())));
    assert!(!part.exists());

    let served = data.clone();
    let (skewed, _) = serve(move |req| {
        let from = req.range.unwrap_or(0) as usize + 10;
        Resp { status: 206, headers: vec![("Content-Range", format!("bytes {from}-{}/{}", served.len() - 1, served.len()))], body: served[from..].to_vec(), ..Resp::ok("") }
    });
    std::fs::write(&part, &data[..1000]).unwrap();
    assert_eq!(fetch::fetch(&agent, &format!("{skewed}/f"), &part, 0, plenty, |_, _| true), Err(Fail::Net("The server resumed at the wrong position".into())));
    assert!(!part.exists());

    assert_eq!(fetch::fetch(&agent, &url, &part, 0, no_space, |_, _| true), Err(Fail::Fatal("Not enough free space for this download".into())));
    let (gone, _) = serve(|_| Resp::status(404));
    assert_eq!(fetch::fetch(&agent, &format!("{gone}/f"), &part, 0, plenty, |_, _| true), Err(Fail::Fatal("The server refused the download (404)".into())));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bulk_show_downloads_for_offline_use_then_removes() {
    let data = Arc::new(bytes(600_000));
    let (srv, log) = fake_plex(data.clone(), true, false);
    let root = temp("bulk");
    let d = engine(&root, plenty);
    d.enqueue(&srv, &show());
    let s = wait(&d, |s| s.len() == 2 && s.iter().all(|e| e.state == State::Done));
    assert_eq!(s.iter().map(|e| (e.key.as_str(), e.title.as_str(), e.subtitle.as_str())).collect::<Vec<_>>(), [("101", "Show", "S1 · E1 · Ep 101"), ("102", "Show", "S1 · E2 · Ep 102")]);
    assert_eq!((s[0].total, s[0].bytes, s[0].file.as_str(), s[0].parents.clone()), (data.len() as u64, data.len() as u64, "media.mkv", vec!["110".to_owned(), "100".to_owned()]));
    let resumed = log.lock().unwrap().iter().filter(|r| r.path.starts_with("/library/parts/101")).map(|r| r.range).collect::<Vec<_>>();
    assert_eq!(resumed.len(), 2);
    assert!(resumed[1].unwrap() > 0, "the retry resumed instead of restarting: {resumed:?}");
    assert!(!log.lock().unwrap().iter().any(|r| r.path.contains("timeline")));

    let local = d.local("srv1", "101").unwrap();
    assert_eq!(std::fs::read(&local.file).unwrap(), *data);
    assert_eq!(local.subtitles.len(), 1);
    assert_eq!((local.subtitles[0].0.file_name().unwrap().to_str(), local.subtitles[0].1.language_code.as_str()), (Some("sub-1.srt"), "eng"));
    assert_eq!(d.item("srv1", "100").unwrap().title, "Show");
    assert_eq!(d.item("srv1", "110").unwrap().kind, "season");
    assert_eq!(d.library().iter().map(|(srv, it)| (srv.as_str(), it.media[0].part[0].key.as_str())).collect::<Vec<_>>(), [("srv1", "/library/parts/101/file.mkv"), ("srv1", "/library/parts/102/file.mkv")]);
    let poster = d.image("srv1", "/library/metadata/100/thumb/1").unwrap();
    assert_eq!(std::fs::read(&poster).unwrap(), b"jpeg");
    assert!(d.image("srv1", "/library/metadata/101/thumb/1").is_some() && d.image("srv1", "/nope").is_none());

    d.enqueue(&srv, &show());
    let s = wait(&d, |s| s.len() == 2 && s.iter().all(|e| e.state == State::Done));
    assert_eq!(s.len(), 2, "re-adding a show skips downloaded episodes");

    d.remove("srv1", "101");
    assert!(d.local("srv1", "101").is_none() && d.snapshot().len() == 1);
    until(|| !root.join("srv1").join("101").exists() && d.image("srv1", "/library/metadata/101/thumb/1").is_none());
    assert!(d.item("srv1", "100").is_some() && d.image("srv1", "/library/metadata/100/thumb/1").is_some(), "parents stay while an episode needs them");
    d.remove("srv1", "102");
    until(|| d.item("srv1", "100").is_none() && d.inner.lock().entries.is_empty());
    assert!(!root.join("srv1").join("100").exists() && !root.join("srv1").join("110").exists());
    until(|| std::fs::read_dir(root.join("srv1").join(IMAGES)).unwrap().count() == 0);
    drop(d);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn pause_resume_and_remove_the_active_download() {
    let data = Arc::new(bytes(1 << 20));
    let (srv, log) = fake_plex(data.clone(), false, true);
    let root = temp("active");
    let d = engine(&root, plenty);
    d.enqueue(&srv, &ep("101"));
    wait(&d, |s| s[0].state == State::Downloading && s[0].bytes > 0);
    d.pause("srv1", "101");
    let s = wait(&d, |s| s[0].state == State::Paused);
    until(|| d.inner.lock().active.is_none());
    let have = std::fs::metadata(part(&root, "101")).unwrap().len();
    assert!(have > 0 && have < data.len() as u64 && s[0].bytes < data.len() as u64);
    d.resume("srv1", "101");
    wait(&d, |s| s[0].state == State::Done);
    assert_eq!(std::fs::read(d.local("srv1", "101").unwrap().file).unwrap(), *data);
    let ranges: Vec<u64> = log.lock().unwrap().iter().filter(|r| r.path.starts_with("/library/parts/101")).filter_map(|r| r.range).collect();
    assert!(ranges.len() == 2 && ranges[1] >= have, "{ranges:?} {have}");

    d.enqueue(&srv, &ep("102"));
    wait(&d, |s| s.len() == 2 && s[1].state == State::Downloading && s[1].bytes > 0);
    d.remove("srv1", "102");
    assert_eq!(d.snapshot().len(), 1);
    until(|| d.inner.lock().entries.len() == 1 && !root.join("srv1").join("102").exists());
    assert!(root.join("srv1").join("100").join(ITEM).exists(), "101 still needs the show");
    drop(d);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn changed_server_file_restarts_the_partial_download() {
    let data = Arc::new(bytes(300_000));
    let (srv, _) = fake_plex(data.clone(), false, false);
    let root = temp("changed");
    let mut e = Entry::new("srv1", &ep("101"));
    (e.file, e.source, e.total) = ("media.mkv".into(), "/library/parts/101/old/file.mkv".into(), data.len() as u64);
    assert!(store::write_json(&root.join(QUEUE), &vec![e]));
    std::fs::create_dir_all(root.join("srv1").join("101")).unwrap();
    std::fs::write(part(&root, "101"), vec![9u8; 1000]).unwrap();
    let d = engine(&root, plenty);
    assert_eq!(d.snapshot()[0].bytes, 1000);
    d.enqueue(&srv, &ep("101"));
    let s = wait(&d, |s| s[0].state == State::Done);
    assert_eq!(s[0].source, "/library/parts/101/file.mkv");
    assert_eq!(std::fs::read(d.local("srv1", "101").unwrap().file).unwrap(), *data);
    drop(d);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn jellyfin_downloads_the_static_original() {
    let data = Arc::new(bytes(50_000));
    let served = data.clone();
    let (base, log) = serve(move |req| match req.path.split('?').next().unwrap() {
        "/Items/j1" => Resp::ok(
            json!({"Id": "j1", "Name": "Film", "Type": "Movie", "ProductionYear": 2010, "ImageTags": {"Primary": "p1"}, "MediaSources": [{"Id": "j1", "Path": "/m/Film.mkv", "Container": "mkv", "MediaStreams": [
                {"Type": "Video", "Index": 0, "Codec": "h264"},
                {"Type": "Subtitle", "Index": 2, "Codec": "subrip", "Language": "eng", "DisplayTitle": "English", "IsExternal": true},
            ]}]})
            .to_string(),
        ),
        "/Videos/j1/stream" => media(&served, req, true, None),
        "/Videos/j1/j1/Subtitles/2/Stream.srt" => Resp::ok("1\n00:00:01,000 --> 00:00:02,000\nHi\n"),
        "/Items/j1/Images/Primary" => Resp::ok("jpeg"),
        _ => Resp::status(404),
    });
    let srv = Server { name: "jf".into(), id: "jf1".into(), uri: base, token: "tok".into(), backend: Backend::Jellyfin, user_id: "u1".into(), ..Default::default() };
    let root = temp("jellyfin");
    let d = engine(&root, plenty);
    d.enqueue(&srv, &Item { rating_key: "j1".into(), kind: "movie".into(), ..Default::default() });
    let s = wait(&d, |s| s[0].state == State::Done);
    assert_eq!((s[0].title.as_str(), s[0].subtitle.as_str()), ("", ""));
    let local = d.local("jf1", "j1").unwrap();
    assert_eq!(std::fs::read(&local.file).unwrap(), *data);
    assert_eq!(local.subtitles.iter().map(|(p, _)| p.file_name().unwrap().to_str().unwrap()).collect::<Vec<_>>(), ["sub-1.srt"]);
    assert!(d.image("jf1", "/Items/j1/Images/Primary?tag=p1").is_some());
    let log = log.lock().unwrap();
    let download = log.iter().find(|r| r.path.starts_with("/Videos/j1/stream")).unwrap();
    assert_eq!((download.path.as_str(), download.range), ("/Videos/j1/stream?Static=true&MediaSourceId=j1&ApiKey=tok", Some(0)));
    drop(d);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn free_space_check_fails_gracefully() {
    let (srv, _) = fake_plex(Arc::new(bytes(1000)), false, false);
    let root = temp("space");
    let d = engine(&root, no_space);
    d.enqueue(&srv, &ep("101"));
    let s = wait(&d, |s| s[0].state == State::Failed);
    assert_eq!(s[0].error, "Not enough free space for this download");
    assert!(!part(&root, "101").exists());
    assert!(d.local("srv1", "101").is_none());
    d.enqueue(&Server { id: String::new(), ..srv.clone() }, &ep("5"));
    d.enqueue(&srv, &ep(""));
    assert_eq!(d.snapshot().len(), 1, "empty ids are ignored");
    drop(d);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn queue_and_offline_progress_persist() {
    let root = temp("persist");
    let unreachable = Server { name: "gone".into(), id: "srv1".into(), uri: "http://127.0.0.1:9".into(), ..Default::default() };
    let movie = Item { rating_key: "5".into(), kind: "movie".into(), title: "M".into(), year: Some(2001), ..Default::default() };
    {
        let d = engine(&root, plenty);
        d.enqueue(&unreachable, &movie);
        d.pause("srv1", "5");
        wait(&d, |s| s[0].state == State::Paused);
        d.record_progress("srv1", "5", "playing", 10, 100, false);
        d.record_progress("srv1", "5", "paused", 20, 100, false);
        d.record_progress("srv1", "6", "stopped", 30, 100, false);
    }
    let d = engine(&root, plenty);
    let s = d.snapshot();
    assert_eq!(s.len(), 1);
    assert_eq!((s[0].key.as_str(), s[0].kind.as_str(), s[0].title.as_str(), s[0].subtitle.as_str(), s[0].state), ("5", "movie", "M", "2001", State::Paused));
    let pending = d.inner.lock().pending.clone();
    assert_eq!(pending.iter().map(|p| (p.key.as_str(), p.time_ms)).collect::<Vec<_>>(), [("5", 20), ("6", 30)]);
    drop(d);

    let crashed = temp("crashed");
    let mut e = Entry::new("srv1", &movie);
    e.state = State::Downloading;
    e.file = "media.mkv".into();
    assert!(store::write_json(&crashed.join(QUEUE), &vec![e]));
    std::fs::create_dir_all(crashed.join("srv1").join("5")).unwrap();
    std::fs::write(part(&crashed, "5"), [0u8; 42]).unwrap();
    let d = engine(&crashed, plenty);
    assert_eq!(d.snapshot().iter().map(|e| (e.state, e.bytes)).collect::<Vec<_>>(), [(State::Queued, 42)]);
    drop(d);
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(crashed);
}

fn seed_show(root: &Path) {
    let snapshot = |key: &str, it: Value| {
        let dir = root.join("srv1").join(key);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(ITEM), it.to_string()).unwrap();
    };
    snapshot("101", episode(101, true));
    snapshot("110", json!({"ratingKey": "110", "type": "season", "title": "Season 1"}));
    snapshot("100", json!({"ratingKey": "100", "type": "show", "title": "Show"}));
    std::fs::write(root.join("srv1").join("101").join("media.mkv"), b"video").unwrap();
}

#[test]
fn removing_a_queued_show_keeps_its_downloaded_episodes_metadata() {
    let root = temp("showremove");
    seed_show(&root);
    let mut episode = Entry::new("srv1", &ep("101"));
    (episode.state, episode.file, episode.parents) = (State::Done, "media.mkv".into(), vec!["110".into(), "100".into()]);
    let pending_show = Entry { state: State::Paused, ..Entry::new("srv1", &show()) };
    assert!(store::write_json(&root.join(QUEUE), &vec![episode, pending_show]));
    let d = engine(&root, plenty);
    until(|| d.item("srv1", "100").is_some());
    d.remove("srv1", "100");
    until(|| d.inner.lock().entries.len() == 1);
    assert!(root.join("srv1").join("100").join(ITEM).exists() && d.item("srv1", "100").is_some());
    d.remove("srv1", "101");
    until(|| d.inner.lock().entries.is_empty());
    assert!(["100", "101", "110"].iter().all(|k| !root.join("srv1").join(k).exists()));
    drop(d);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unreadable_queue_is_kept_and_rebuilt() {
    let root = temp("corrupt");
    seed_show(&root);
    std::fs::write(root.join(QUEUE), b"{ not json").unwrap();
    let d = engine(&root, plenty);
    let s = d.snapshot();
    assert_eq!(s.iter().map(|e| (e.key.as_str(), e.state, e.bytes, e.parents.len())).collect::<Vec<_>>(), [("101", State::Done, 5, 2)]);
    assert_eq!(std::fs::read(root.join("downloads.json.bad")).unwrap(), b"{ not json");
    until(|| d.local("srv1", "101").is_some_and(|l| l.subtitles.is_empty()) && d.library().len() == 1);
    drop(d);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn offline_progress_coalesces_into_replay_requests() {
    let at = |key: &str, time_ms| Pending { server: "s".into(), key: key.into(), time_ms, duration_ms: 100_000, updated: 1_700_000_000, watched: false };
    let mut list = Vec::new();
    progress::coalesce(&mut list, at("101", 1000));
    progress::coalesce(&mut list, at("102", 5000));
    progress::coalesce(&mut list, at("101", 61_000));
    assert_eq!(list, [at("102", 5000), at("101", 61_000)]);
    progress::coalesce(&mut list, at("102", 95_000));
    progress::coalesce(&mut list, at("102", 4000));
    assert_eq!(list[1], Pending { watched: true, ..at("102", 4000) }, "a watched mark survives a later short reopen");

    let plex = Server { name: "p".into(), id: "s".into(), uri: "http://x".into(), token: "t".into(), ..Default::default() };
    let jf = Server { backend: Backend::Jellyfin, user_id: "u1".into(), ..plex.clone() };
    let paths = |p: &Pending| progress::requests(&plex, p).into_iter().map(|r| (r.path, r.body)).collect::<Vec<_>>();
    assert_eq!(paths(&list[0]), [(plex_timeline("101", 61_000), None)]);
    let scrobble = "/:/scrobble?identifier=com.plexapp.plugins.library&key=102".to_owned();
    assert_eq!(paths(&list[1]), [(scrobble.clone(), None), (plex_timeline("102", 4000), None)]);
    assert_eq!(paths(&Pending { watched: true, ..at("102", 95_000) }), [(scrobble, None)]);

    let body = |p: &Pending| progress::requests(&jf, p).pop().unwrap();
    let r = body(&list[0]);
    assert_eq!(r.path, "/UserItems/101/UserData?userId=u1");
    assert_eq!(r.body, Some(json!({"PlaybackPositionTicks": 610_000_000u64, "LastPlayedDate": "2023-11-14T22:13:20.000Z"})), "never sends Played:false");
    assert_eq!(body(&list[1]).body, Some(json!({"PlaybackPositionTicks": 40_000_000u64, "Played": true, "LastPlayedDate": "2023-11-14T22:13:20.000Z"})));
    assert_eq!(body(&Pending { watched: true, ..at("1", 99_000) }).body.unwrap()["PlaybackPositionTicks"], 0);
    assert_eq!(progress::iso(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(progress::iso(951_782_400), "2000-02-29T00:00:00.000Z");
}

fn plex_timeline(key: &str, time: u64) -> String {
    format!("/:/timeline?ratingKey={key}&key=%2Flibrary%2Fmetadata%2F{key}&state=stopped&time={time}&duration=100000&offline=1&updated=1700000000")
}

#[test]
fn jellyfin_replay_respects_newer_server_state() {
    let last_played = Arc::new(Mutex::new("2030-01-01T00:00:00.0000000Z"));
    let date = last_played.clone();
    let (base, log) = serve(move |req| match req.method.as_str() {
        "GET" => Resp::ok(json!({"LastPlayedDate": *date.lock().unwrap(), "Played": true}).to_string()),
        _ => Resp::ok("{}"),
    });
    let srv = Server { name: "jf".into(), id: "jf1".into(), uri: base, token: "tok".into(), backend: Backend::Jellyfin, user_id: "u1".into(), ..Default::default() };
    let c = Client::new("c".into());
    let send = |watched| {
        let p = Pending { server: "jf1".into(), key: "j1".into(), time_ms: 1000, duration_ms: 100_000, updated: 1_700_000_000, watched };
        progress::requests(&srv, &p).iter().try_for_each(|r| progress::send(&c, &srv, r)).unwrap();
    };
    send(false);
    send(true);
    *last_played.lock().unwrap() = "2020-01-01T00:00:00.0000000Z";
    send(false);
    let seen: Vec<(String, Option<Value>)> = log.lock().unwrap().iter().map(|r| (r.method.clone(), serde_json::from_str(&r.body).ok())).collect();
    let get = ("GET".to_owned(), None);
    assert_eq!(
        seen,
        [
            get.clone(),
            get.clone(),
            ("POST".into(), Some(json!({"Played": true}))),
            get,
            ("POST".into(), Some(json!({"PlaybackPositionTicks": 10_000_000u64, "LastPlayedDate": "2023-11-14T22:13:20.000Z"}))),
        ]
    );
}

#[test]
fn auto_remove_watched_and_replay_when_reachable() {
    let (srv, log) = fake_plex(Arc::new(bytes(1000)), false, false);
    let root = temp("autoremove");
    let d = engine(&root, plenty);
    d.enqueue(&srv, &ep("101"));
    d.enqueue(&srv, &ep("102"));
    wait(&d, |s| s.len() == 2 && s.iter().all(|e| e.state == State::Done));
    d.set_auto_remove_watched(true);
    d.record_progress("srv1", "102", "stopped", 400, 1000, false);
    assert_eq!(d.item("srv1", "102").unwrap().view_offset, Some(400));
    assert_eq!(store::read_json::<Item>(&root.join("srv1").join("102").join(ITEM)).unwrap().view_offset, Some(400));
    d.record_progress("srv1", "101", "playing", 900, 1000, false);
    std::thread::sleep(PRUNE_DELAY + Duration::from_millis(300));
    assert_eq!(d.snapshot().len(), 2, "playing past the threshold is not a stop");
    d.record_progress("srv1", "101", "stopped", 950, 1000, false);
    assert_eq!(d.item("srv1", "101").unwrap().view_count, Some(1));
    let s = wait(&d, |s| s.len() == 1);
    assert_eq!(s[0].key, "102");
    until(|| !root.join("srv1").join("101").exists());

    d.set_servers(&[srv]);
    until(|| d.inner.lock().pending.is_empty());
    let sent: Vec<String> = log.lock().unwrap().iter().map(|r| r.path.clone()).filter(|p| p.starts_with("/:/")).collect();
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(sent.iter().any(|p| p == "/:/scrobble?identifier=com.plexapp.plugins.library&key=101"));
    assert!(sent.iter().any(|p| p.starts_with("/:/timeline?ratingKey=102&") && p.contains("time=400&") && p.contains("offline=1&updated=")));
    assert!(store::read_json::<Vec<Pending>>(&root.join(PROGRESS)).unwrap().is_empty());
    drop(d);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
#[ignore = "reads the owner's Plex config and a real server (GET only)"]
fn live_plex_range_read() {
    let config = PathBuf::from(std::env::var_os("APPDATA").expect("APPDATA")).join("Plaivo").join("config.json");
    let account: crate::Account = store::read_json(&config).expect("Plaivo config.json");
    let client = Client::new(account.client_id.clone());
    let cx = Ctx::new(client.clone(), Source::Live);
    let (srv, item) = account
        .servers
        .iter()
        .filter(|s| !s.is_jellyfin())
        .find_map(|srv| {
            let sections = srv.sections(&cx).ok()?;
            sections.iter().find_map(|sec| {
                let (items, _) = srv.section_items(&cx, &sec.key, 0, 1, "addedAt:desc").ok()?;
                let key = &items.first()?.rating_key;
                let leaf = if sec.kind == "show" { srv.all_episodes(&cx, key).ok()?.into_iter().next()? } else { items.into_iter().next()? };
                let it = srv.metadata(&cx, &leaf.rating_key).ok()?;
                it.media.first()?.part.first()?;
                Some((srv, it))
            })
        })
        .expect("no reachable Plex server with a playable item");
    let part = &item.media[0].part[0];
    let url = srv.media_url(&part.key);
    const MB: u64 = 1 << 20;
    let dir = std::env::temp_dir().join(format!("plaivo-live-range-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("head.{}", extension(Path::new(&part.file).extension().and_then(|e| e.to_str()).unwrap_or(""), "bin")));

    let resp = fetch::request(&client.agent, &url, 0).expect("range request");
    let status = resp.status().as_u16();
    let (start, total) = fetch::range_of(&resp).expect("content-range");
    let mut head = Vec::new();
    resp.into_body().into_reader().take(2 * MB).read_to_end(&mut head).unwrap();
    std::fs::write(&file, &head).unwrap();
    println!("{} {} first request: status {status}, start {start}, total {total:?}, wrote {} bytes", item.kind, item.rating_key, std::fs::metadata(&file).unwrap().len());
    assert_eq!((status, start), (206, 0));
    assert!(total.unwrap() > 2 * MB);
    assert_eq!(head.len() as u64, 2 * MB);

    let resp = fetch::request(&client.agent, &url, MB).expect("resume request");
    let status = resp.status().as_u16();
    let (start, _) = fetch::range_of(&resp).expect("content-range");
    let mut tail = Vec::new();
    resp.into_body().into_reader().take(MB).read_to_end(&mut tail).unwrap();
    let matches = tail[..] == head[MB as usize..];
    println!("resume request: status {status}, start {start}, bytes match the first request: {matches}");
    assert_eq!((status, start), (206, MB));
    assert!(matches);

    std::fs::remove_dir_all(&dir).unwrap();
    println!("scratch dir deleted: {}", !dir.exists());
}

#[test]
fn synced_progress_updates_the_snapshot_without_a_replay() {
    let (srv, _log) = fake_plex(Arc::new(bytes(1000)), false, false);
    let root = temp("synced");
    let d = engine(&root, plenty);
    d.enqueue(&srv, &ep("101"));
    wait(&d, |s| s.len() == 1 && s[0].state == State::Done);
    d.record_progress("srv1", "101", "stopped", 400, 1000, true);
    assert_eq!(d.item("srv1", "101").unwrap().view_offset, Some(400));
    assert!(d.inner.lock().pending.is_empty());
}
