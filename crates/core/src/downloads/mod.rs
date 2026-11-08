mod fetch;
mod progress;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::plex::{Client, Ctx, Item, Media, Part, Server, Source, Stream};
use crate::store;
use fetch::{Fail, disk};
use progress::Pending;

const QUEUE: &str = "downloads.json";
const PROGRESS: &str = "progress.json";
const ITEM: &str = "item.json";
const IMAGES: &str = ".images";
const IMAGE_SIZE: u32 = 960;
const NOTIFY_EVERY: Duration = Duration::from_millis(250);
const FIRST_RETRY: Duration = Duration::from_secs(1);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(16);
const REPLAY_DELAY: Duration = Duration::from_secs(30);
const MAX_REPLAY_DELAY: Duration = Duration::from_secs(900);
const PRUNE_DELAY: Duration = Duration::from_secs(3);
const PURGE_RETRY: Duration = Duration::from_secs(60);
const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);
const IDLE: Duration = Duration::from_secs(3600);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Queued,
    Downloading,
    Paused,
    Failed,
    Done,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub server: String,
    pub key: String,
    pub kind: String,
    pub title: String,
    pub subtitle: String,
    pub state: State,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub parents: Vec<String>,
    #[serde(default)]
    pub watched: bool,
    #[serde(default)]
    pub(crate) source: String,
    #[serde(default)]
    pub(crate) removing: bool,
}

impl Entry {
    fn new(server: &str, it: &Item) -> Self {
        let (title, subtitle) = it.card_text();
        Entry {
            server: server.into(),
            key: it.rating_key.clone(),
            kind: it.kind.clone(),
            title,
            subtitle,
            state: State::Queued,
            bytes: 0,
            total: 0,
            error: String::new(),
            file: String::new(),
            parents: Vec::new(),
            watched: false,
            source: String::new(),
            removing: false,
        }
    }

    fn is(&self, server: &str, key: &str) -> bool {
        self.server == server && self.key == key
    }

    fn id(&self) -> (String, String) {
        (self.server.clone(), self.key.clone())
    }
}

pub struct Local {
    pub file: PathBuf,
    pub subtitles: Vec<(PathBuf, Stream)>,
}

pub struct Downloads {
    inner: Arc<Inner>,
    worker: Option<JoinHandle<()>>,
}

struct Inner {
    root: PathBuf,
    client: Client,
    free_space: fn(&Path) -> Option<u64>,
    on_change: Box<dyn Fn() + Send + Sync>,
    shared: Mutex<Shared>,
    wake: Condvar,
    abort: AtomicBool,
}

#[derive(Default)]
struct Shared {
    entries: Vec<Entry>,
    pending: Vec<Pending>,
    servers: HashMap<String, Server>,
    items: HashMap<(String, String), Item>,
    images: HashSet<PathBuf>,
    active: Option<(String, String)>,
    auto_remove: bool,
    prune_at: Option<Instant>,
    replay_at: Option<Instant>,
    replay_backoff: Duration,
    last_notify: Option<Instant>,
    shutdown: bool,
}

enum Job {
    Download(Box<Entry>, Box<Server>),
    Replay(Vec<(Server, Pending)>),
    Purge(Vec<Entry>),
}

enum Outcome {
    Expanded(Vec<Item>),
    Done(u64),
}

fn safe(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

fn extension(s: &str, fallback: &str) -> String {
    let e: String = s.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
    match e.as_str() {
        "" => fallback.into(),
        "subrip" => "srt".into(),
        _ => e,
    }
}

fn media_name(media: &Media, part: &Part) -> String {
    let ext = Path::new(&part.file).extension().and_then(|e| e.to_str()).unwrap_or(&media.container);
    format!("media.{}", extension(ext, "mkv"))
}

fn original(it: &Item) -> Option<(&Media, &Part)> {
    it.media.first().and_then(|m| Some((m, m.part.first()?)))
}

fn subtitles(part: &Part) -> impl Iterator<Item = (String, &Stream)> {
    part.stream.iter().enumerate().filter(|(_, s)| s.stream_type == 3 && !s.key.is_empty()).map(|(i, s)| (format!("sub-{i}.{}", extension(&s.codec, "srt")), s))
}

fn image_paths(it: &Item) -> impl Iterator<Item = &String> {
    [&it.thumb, &it.art, &it.parent_thumb, &it.grandparent_thumb, &it.grandparent_art].into_iter().filter(|p| !p.is_empty())
}

fn parent_keys(it: &Item) -> Vec<String> {
    [&it.parent_rating_key, &it.grandparent_rating_key].into_iter().filter(|k| !k.is_empty()).cloned().collect()
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn delete_dir(dir: &Path) -> bool {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => {
            log::warn!("could not delete {}: {e}", dir.display());
            false
        }
    }
}

fn subdirs(dir: &Path) -> impl Iterator<Item = PathBuf> {
    std::fs::read_dir(dir).into_iter().flatten().flatten().map(|d| d.path()).filter(|p| p.is_dir())
}

fn write_item(dir: &Path, it: &Item) -> Result<(), Fail> {
    let bytes = serde_json::to_vec_pretty(it).map_err(|e| Fail::Fatal(e.to_string()))?;
    store::write_atomic(&dir.join(ITEM), &bytes).map_err(disk)
}

fn load_entries(root: &Path) -> Vec<Entry> {
    let path = root.join(QUEUE);
    let Ok(bytes) = std::fs::read(&path) else { return Vec::new() };
    if let Ok(entries) = serde_json::from_slice(&bytes) {
        return entries;
    }
    log::error!("{} is unreadable, rebuilding the queue from the downloaded items", path.display());
    let _ = std::fs::rename(&path, path.with_extension("json.bad"));
    let mut entries = Vec::new();
    for sdir in subdirs(root) {
        let server = sdir.file_name().unwrap_or_default().to_string_lossy().into_owned();
        for kdir in subdirs(&sdir) {
            let Some(it) = store::read_json::<Item>(&kdir.join(ITEM)).filter(|it| matches!(it.kind.as_str(), "movie" | "episode")) else { continue };
            let Some((media, part)) = original(&it) else { continue };
            let mut e = Entry::new(&server, &it);
            e.file = media_name(media, part);
            e.source = part.key.clone();
            e.parents = parent_keys(&it);
            match std::fs::metadata(kdir.join(&e.file)) {
                Ok(m) => (e.state, e.bytes, e.total) = (State::Done, m.len(), m.len()),
                Err(_) => e.state = State::Paused,
            }
            entries.push(e);
        }
    }
    entries
}

impl Downloads {
    pub fn new(root: PathBuf, client: Client, on_change: impl Fn() + Send + Sync + 'static) -> Self {
        Self::start(root, client, fetch::free_space, Box::new(on_change))
    }

    fn start(root: PathBuf, client: Client, free_space: fn(&Path) -> Option<u64>, on_change: Box<dyn Fn() + Send + Sync>) -> Self {
        let mut entries = load_entries(&root);
        for e in &mut entries {
            if e.state == State::Downloading {
                e.state = State::Queued;
            }
            if e.state != State::Done && !e.file.is_empty() {
                e.bytes = std::fs::metadata(root.join(safe(&e.server)).join(safe(&e.key)).join(format!("{}.part", e.file))).map(|m| m.len()).unwrap_or(0);
            }
        }
        let pending: Vec<Pending> = store::read_json(&root.join(PROGRESS)).unwrap_or_default();
        let shared = Shared {
            replay_at: (!pending.is_empty()).then(Instant::now),
            prune_at: entries.iter().any(|e| e.removing).then(Instant::now),
            entries,
            pending,
            replay_backoff: REPLAY_DELAY,
            ..Default::default()
        };
        let inner = Arc::new(Inner { root, client, free_space, on_change, shared: Mutex::new(shared), wake: Condvar::new(), abort: AtomicBool::new(false) });
        let worker = inner.clone();
        let worker = std::thread::Builder::new().name("downloads".into()).spawn(move || worker.run()).expect("spawn downloads worker");
        Self { inner, worker: Some(worker) }
    }

    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    pub fn set_servers(&self, servers: &[Server]) {
        let abort = &self.inner.abort;
        self.inner.update(|s| {
            s.servers = servers.iter().map(|srv| (srv.id.clone(), srv.clone())).collect();
            if let Some((server, key)) = s.active.clone().filter(|(server, _)| !s.servers.contains_key(server)) {
                abort.store(true, Ordering::Relaxed);
                if let Some(e) = s.entries.iter_mut().find(|e| e.is(&server, &key) && e.state == State::Downloading) {
                    e.state = State::Queued;
                }
            }
            if !s.pending.is_empty() {
                s.replay_at = Some(Instant::now());
            }
        });
    }

    pub fn set_auto_remove_watched(&self, on: bool) {
        self.inner.update(|s| {
            s.auto_remove = on;
            if on {
                s.prune_at = Some(Instant::now());
            }
        });
    }

    pub fn snapshot(&self) -> Vec<Entry> {
        self.inner.lock().entries.iter().filter(|e| !e.removing).cloned().collect()
    }

    pub fn enqueue(&self, srv: &Server, item: &Item) {
        if srv.id.is_empty() || item.rating_key.is_empty() {
            return;
        }
        self.inner.update(|s| {
            s.servers.insert(srv.id.clone(), srv.clone());
            match s.entries.iter_mut().find(|e| e.is(&srv.id, &item.rating_key)) {
                Some(e) if e.removing || matches!(e.state, State::Paused | State::Failed) => {
                    (e.removing, e.watched, e.state) = (false, false, State::Queued);
                    e.error.clear();
                }
                Some(_) => {}
                None => s.entries.push(Entry::new(&srv.id, item)),
            }
        });
    }

    pub fn pause(&self, server: &str, key: &str) {
        self.set_state(|e| e.is(server, key), &[State::Queued, State::Downloading], State::Paused);
    }

    pub fn resume(&self, server: &str, key: &str) {
        self.set_state(|e| e.is(server, key), &[State::Paused, State::Failed], State::Queued);
    }

    pub fn pause_all(&self) {
        self.set_state(|_| true, &[State::Queued, State::Downloading], State::Paused);
    }

    pub fn resume_all(&self) {
        self.set_state(|_| true, &[State::Paused, State::Failed], State::Queued);
    }

    fn set_state(&self, matches: impl Fn(&Entry) -> bool, from: &[State], to: State) {
        let abort = &self.inner.abort;
        self.inner.update(|s| {
            for e in s.entries.iter_mut().filter(|e| !e.removing && matches(e) && from.contains(&e.state)) {
                if e.state == State::Downloading {
                    abort.store(true, Ordering::Relaxed);
                }
                e.state = to;
                e.error.clear();
            }
        });
    }

    pub fn remove(&self, server: &str, key: &str) {
        let abort = &self.inner.abort;
        self.inner.update(|s| {
            let Some(e) = s.entries.iter_mut().find(|e| e.is(server, key)) else { return };
            e.removing = true;
            if s.active.as_ref().is_some_and(|(srv, k)| e.is(srv, k)) {
                abort.store(true, Ordering::Relaxed);
            }
            s.prune_at = Some(Instant::now());
        });
    }

    pub fn local(&self, server: &str, key: &str) -> Option<Local> {
        let dir = self.inner.dir(server, key);
        let (file, subs) = {
            let s = self.inner.lock();
            let e = s.entries.iter().find(|e| e.is(server, key) && e.state == State::Done && !e.removing)?;
            let subs: Vec<(PathBuf, Stream)> =
                s.items.get(&e.id()).and_then(original).map(|(_, part)| subtitles(part).map(|(name, st)| (dir.join(name), st.clone())).collect()).unwrap_or_default();
            (dir.join(&e.file), subs)
        };
        Some(Local { file, subtitles: subs.into_iter().filter(|(p, _)| p.exists()).collect() })
    }

    pub fn item(&self, server: &str, key: &str) -> Option<Item> {
        self.inner.lock().items.get(&(server.to_owned(), key.to_owned())).cloned()
    }

    pub fn library(&self) -> Vec<(String, Item)> {
        let s = self.inner.lock();
        s.entries.iter().filter(|e| e.state == State::Done && !e.removing).filter_map(|e| Some((e.server.clone(), s.items.get(&e.id())?.clone()))).collect()
    }

    pub fn image(&self, server: &str, path: &str) -> Option<PathBuf> {
        let p = self.inner.image_path(server, path);
        self.inner.lock().images.contains(&p).then_some(p)
    }

    pub fn record_progress(&self, server: &str, key: &str, state: &str, time_ms: u64, duration_ms: u64, synced: bool) {
        let watched = progress::watched(time_ms, duration_ms);
        let stopped = state == "stopped";
        let snapshot = {
            let mut s = self.inner.lock();
            if !synced {
                progress::coalesce(&mut s.pending, Pending { server: server.into(), key: key.into(), time_ms, duration_ms, updated: unix_now(), watched });
                self.inner.save_progress(&s);
                s.replay_at.get_or_insert_with(|| Instant::now() + REPLAY_DELAY);
            }
            s.items.get_mut(&(server.to_owned(), key.to_owned())).filter(|_| stopped).map(|it| {
                it.view_offset = (!watched).then_some(time_ms);
                if watched {
                    it.view_count = Some(it.view_count.unwrap_or(0) + 1);
                }
                it.clone()
            })
        };
        self.inner.wake.notify_all();
        if let Some(it) = snapshot {
            let _ = write_item(&self.inner.dir(server, key), &it);
        }
        if stopped && watched {
            self.inner.update(|s| {
                if let Some(e) = s.entries.iter_mut().find(|e| e.is(server, key)) {
                    e.watched = true;
                }
                s.prune_at = Some(Instant::now() + PRUNE_DELAY);
            });
        }
    }
}

impl Drop for Downloads {
    fn drop(&mut self) {
        self.inner.lock().shutdown = true;
        self.inner.abort.store(true, Ordering::Relaxed);
        self.inner.wake.notify_all();
        let Some(worker) = self.worker.take() else { return };
        let until = Instant::now() + SHUTDOWN_WAIT;
        while !worker.is_finished() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        if worker.is_finished() {
            let _ = worker.join();
        }
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn dir(&self, server: &str, key: &str) -> PathBuf {
        self.root.join(safe(server)).join(safe(key))
    }

    fn image_path(&self, server: &str, path: &str) -> PathBuf {
        self.root.join(safe(server)).join(IMAGES).join(format!("{:016x}.jpg", store::fnv1a(path)))
    }

    fn save(&self, s: &Shared) {
        if !s.shutdown {
            store::write_json(&self.root.join(QUEUE), &s.entries);
        }
    }

    fn save_progress(&self, s: &Shared) {
        if !s.shutdown {
            store::write_json(&self.root.join(PROGRESS), &s.pending);
        }
    }

    fn update<T>(&self, f: impl FnOnce(&mut Shared) -> T) -> T {
        let r = {
            let mut s = self.lock();
            let r = f(&mut s);
            self.save(&s);
            r
        };
        self.wake.notify_all();
        (self.on_change)();
        r
    }

    fn run(self: Arc<Self>) {
        self.load_cache();
        loop {
            let job = {
                let mut s = self.lock();
                loop {
                    if s.shutdown {
                        return;
                    }
                    if let Some(job) = self.next_job(&mut s) {
                        break job;
                    }
                    let now = Instant::now();
                    let next = [s.replay_at, s.prune_at].into_iter().flatten().min().map_or(IDLE, |t| t.saturating_duration_since(now));
                    s = self.wake.wait_timeout(s, next).unwrap_or_else(|e| e.into_inner()).0;
                }
            };
            match job {
                Job::Download(e, srv) => self.download(*e, &srv),
                Job::Replay(batch) => self.replay(batch),
                Job::Purge(gone) => self.purge(gone),
            }
        }
    }

    fn load_cache(&self) {
        let keys: HashSet<(String, String)> =
            self.lock().entries.iter().flat_map(|e| std::iter::once(e.key.clone()).chain(e.parents.iter().cloned()).map(|k| (e.server.clone(), k))).collect();
        let servers: HashSet<&String> = keys.iter().map(|(srv, _)| srv).collect();
        let images: Vec<PathBuf> = servers.into_iter().flat_map(|srv| std::fs::read_dir(self.root.join(safe(srv)).join(IMAGES)).into_iter().flatten().flatten().map(|f| f.path())).collect();
        let items: Vec<((String, String), Item)> = keys.into_iter().filter_map(|(srv, k)| Some(((srv.clone(), k.clone()), store::read_json(&self.dir(&srv, &k).join(ITEM))?))).collect();
        {
            let mut s = self.lock();
            s.items.extend(items);
            s.images.extend(images);
        }
        (self.on_change)();
    }

    fn next_job(&self, s: &mut Shared) -> Option<Job> {
        let now = Instant::now();
        if s.prune_at.is_some_and(|t| t <= now) {
            s.prune_at = None;
            if s.auto_remove {
                for e in s.entries.iter_mut().filter(|e| e.state == State::Done && e.watched) {
                    e.removing = true;
                }
            }
            let gone: Vec<Entry> = s.entries.iter().filter(|e| e.removing).cloned().collect();
            if !gone.is_empty() {
                return Some(Job::Purge(gone));
            }
        }
        if s.replay_at.is_some_and(|t| t <= now) {
            s.replay_at = None;
            let batch: Vec<(Server, Pending)> = s.pending.iter().filter_map(|p| Some((s.servers.get(&p.server)?.clone(), p.clone()))).collect();
            if !batch.is_empty() {
                return Some(Job::Replay(batch));
            }
            if !s.pending.is_empty() {
                s.replay_at = Some(now + s.replay_backoff);
            }
        }
        let i = s.entries.iter().position(|e| e.state == State::Queued && !e.removing && s.servers.contains_key(&e.server))?;
        let e = &mut s.entries[i];
        e.state = State::Downloading;
        e.error.clear();
        s.active = Some(e.id());
        self.abort.store(false, Ordering::Relaxed);
        let job = Job::Download(Box::new(e.clone()), Box::new(s.servers[&e.server].clone()));
        self.save(s);
        Some(job)
    }

    fn pause_for(&self, d: Duration) -> bool {
        let s = self.lock();
        drop(self.wake.wait_timeout_while(s, d, |s| !s.shutdown && !self.abort.load(Ordering::Relaxed)).unwrap_or_else(|e| e.into_inner()));
        !self.abort.load(Ordering::Relaxed)
    }

    fn download(&self, mut e: Entry, srv: &Server) {
        (self.on_change)();
        let mut delay = FIRST_RETRY;
        let result = loop {
            let before = e.bytes;
            match self.attempt(&mut e, srv) {
                Err(Fail::Net(m)) if delay <= MAX_RETRY_DELAY || e.bytes > before => {
                    if e.bytes > before {
                        delay = FIRST_RETRY;
                    }
                    log::warn!("download {} from {} failed, retrying in {}s: {m}", e.key, srv.name, delay.as_secs());
                    if !self.pause_for(delay) {
                        break Err(Fail::Abort);
                    }
                    delay *= 2;
                }
                r => break r,
            }
        };
        if self.lock().shutdown {
            return;
        }
        self.update(|s| {
            s.active = None;
            let Some(i) = s.entries.iter().position(|x| x.is(&e.server, &e.key) && !x.removing) else { return };
            match result {
                Ok(Outcome::Expanded(items)) if items.is_empty() => {
                    s.entries[i].state = State::Failed;
                    s.entries[i].error = "Nothing to download".into();
                }
                Ok(Outcome::Expanded(items)) => {
                    let fresh: Vec<Entry> = items
                        .iter()
                        .filter(|it| matches!(it.kind.as_str(), "episode" | "movie") && !it.rating_key.is_empty() && !s.entries.iter().any(|x| x.is(&e.server, &it.rating_key)))
                        .map(|it| Entry::new(&e.server, it))
                        .collect();
                    s.entries.splice(i..=i, fresh);
                }
                Ok(Outcome::Done(total)) => {
                    let watched = s.entries[i].watched;
                    s.entries[i] = Entry { state: State::Done, bytes: total, total, error: String::new(), watched, ..e.clone() };
                }
                Err(Fail::Net(m) | Fail::Fatal(m)) => {
                    log::warn!("download {} from {} failed: {m}", e.key, srv.name);
                    s.entries[i].state = State::Failed;
                    s.entries[i].error = m;
                }
                Err(Fail::Abort) => {}
            }
        });
    }

    fn attempt(&self, e: &mut Entry, srv: &Server) -> Result<Outcome, Fail> {
        let cx = Ctx::new(self.client.clone(), Source::Live);
        match e.kind.as_str() {
            "show" => return srv.all_episodes(&cx, &e.key).map(Outcome::Expanded).map_err(Fail::Net),
            "season" => return srv.children(&cx, &e.key).map(Outcome::Expanded).map_err(Fail::Net),
            _ => {}
        }
        let item = srv.metadata(&cx, &e.key).map_err(Fail::Net)?;
        let dir = self.dir(&e.server, &e.key);
        write_item(&dir, &item)?;
        let mut snapshots = vec![(e.key.clone(), item.clone())];
        for key in parent_keys(&item) {
            let pdir = self.dir(&e.server, &key);
            let parent = match store::read_json::<Item>(&pdir.join(ITEM)) {
                Some(p) => p,
                None => {
                    let p = srv.metadata(&cx, &key).map_err(Fail::Net)?;
                    write_item(&pdir, &p)?;
                    p
                }
            };
            snapshots.push((key, parent));
        }
        let (media, part) = original(&item).ok_or_else(|| Fail::Fatal("This item has no file to download".into()))?;
        let name = media_name(media, part);
        if !e.source.is_empty() && (e.source != part.key || e.file != name) {
            let _ = std::fs::remove_file(dir.join(format!("{}.part", e.file)));
            let _ = std::fs::remove_file(dir.join(&e.file));
            (e.bytes, e.total) = (0, 0);
        }
        e.source.clone_from(&part.key);
        e.file = name;
        e.parents = parent_keys(&item);
        let images: Vec<String> = snapshots.iter().flat_map(|(_, it)| image_paths(it).cloned()).collect();
        self.update(|s| {
            if let Some(x) = s.entries.iter_mut().find(|x| x.is(&e.server, &e.key)) {
                (x.file, x.source, x.parents, x.bytes, x.total) = (e.file.clone(), e.source.clone(), e.parents.clone(), e.bytes, e.total);
            }
            s.items.extend(snapshots.into_iter().map(|(k, it)| ((e.server.clone(), k), it)));
        });
        for path in images {
            let file = self.image_path(&e.server, &path);
            if self.lock().images.contains(&file) {
                continue;
            }
            match self.client.fetch(&srv.image_url(&path, IMAGE_SIZE, IMAGE_SIZE)).and_then(|b| store::write_atomic(&file, &b).map_err(|e| e.to_string())) {
                Ok(()) => {
                    self.lock().images.insert(file);
                }
                Err(err) => log::debug!("download artwork for {} failed: {err}", e.key),
            }
        }
        for (name, s) in subtitles(part) {
            let file = dir.join(name);
            if !file.exists()
                && let Err(err) = self.client.fetch(&srv.media_url(&s.key)).and_then(|b| store::write_atomic(&file, &b).map_err(|e| e.to_string()))
            {
                log::warn!("download subtitle {} for {} failed: {err}", s.display_title, e.key);
            }
        }
        let file = dir.join(&e.file);
        if let Ok(m) = std::fs::metadata(&file) {
            return Ok(Outcome::Done(m.len()));
        }
        let part_file = dir.join(format!("{}.part", e.file));
        let (server, key) = e.id();
        let mut seen = (e.bytes, e.total);
        let fetched = fetch::fetch(&self.client.agent, &srv.media_url(&part.key), &part_file, e.total, self.free_space, |done, total| {
            seen = (done, total);
            self.progress(&server, &key, done, total)
        });
        (e.bytes, e.total) = seen;
        let total = fetched?;
        std::fs::rename(&part_file, &file).map_err(disk)?;
        Ok(Outcome::Done(total))
    }

    fn progress(&self, server: &str, key: &str, done: u64, total: u64) -> bool {
        let due = {
            let mut g = self.lock();
            let s = &mut *g;
            if let Some(e) = s.entries.iter_mut().find(|e| e.is(server, key)) {
                e.bytes = done;
                if e.total != total {
                    e.total = total;
                    self.save(s);
                }
            }
            let due = s.last_notify.is_none_or(|t| t.elapsed() >= NOTIFY_EVERY);
            if due {
                s.last_notify = Some(Instant::now());
            }
            due
        };
        if due {
            (self.on_change)();
        }
        !self.abort.load(Ordering::Relaxed)
    }

    fn replay(&self, batch: Vec<(Server, Pending)>) {
        let mut sent = Vec::new();
        let mut down = HashSet::new();
        let mut failed = false;
        for (srv, p) in batch {
            if down.contains(&srv.id) {
                continue;
            }
            let outcome = progress::requests(&srv, &p).iter().try_for_each(|r| progress::send(&self.client, &srv, r));
            match outcome {
                Ok(()) => sent.push(p),
                Err(ureq::Error::StatusCode(c @ (400 | 404))) => {
                    log::warn!("{} dropped offline progress for {} ({c})", srv.name, p.key);
                    sent.push(p);
                }
                Err(e) => {
                    log::debug!("{} offline progress for {} not sent: {e}", srv.name, p.key);
                    failed = true;
                    if !matches!(e, ureq::Error::StatusCode(_)) {
                        down.insert(srv.id.clone());
                    }
                }
            }
        }
        let mut s = self.lock();
        s.pending.retain(|p| !sent.contains(p));
        self.save_progress(&s);
        s.replay_backoff = if failed { (s.replay_backoff * 2).min(MAX_REPLAY_DELAY) } else { REPLAY_DELAY };
        if !s.pending.is_empty() {
            s.replay_at = Some(Instant::now() + s.replay_backoff);
        }
    }

    fn purge(&self, gone: Vec<Entry>) {
        let in_use: HashSet<(String, String)> = self
            .lock()
            .entries
            .iter()
            .filter(|e| !e.removing)
            .flat_map(|e| std::iter::once(e.key.clone()).chain(e.parents.iter().cloned()).map(|k| (e.server.clone(), k)))
            .collect();
        let mut deleted = Vec::new();
        let mut removed = Vec::new();
        for e in &gone {
            let mut ok = true;
            for k in std::iter::once(&e.key).chain(&e.parents) {
                let id = (e.server.clone(), k.clone());
                if k.is_empty() || in_use.contains(&id) || deleted.contains(&id) {
                    continue;
                }
                if delete_dir(&self.dir(&e.server, k)) {
                    deleted.push(id);
                } else if *k == e.key {
                    ok = false;
                }
            }
            if ok {
                removed.push(e.id());
            }
        }
        let (keep, servers): (HashSet<PathBuf>, HashSet<String>) = {
            let mut s = self.lock();
            s.entries.retain(|e| !(e.removing && removed.contains(&e.id())));
            for id in &deleted {
                s.items.remove(id);
            }
            if removed.len() < gone.len() {
                s.prune_at = Some(Instant::now() + PURGE_RETRY);
            }
            self.save(&s);
            let keep = s.items.iter().flat_map(|((srv, _), it)| image_paths(it).map(|p| self.image_path(srv, p)).collect::<Vec<_>>()).collect();
            (keep, gone.iter().map(|e| e.server.clone()).collect())
        };
        let stale: Vec<PathBuf> = servers
            .iter()
            .flat_map(|srv| std::fs::read_dir(self.root.join(safe(srv)).join(IMAGES)).into_iter().flatten().flatten().map(|f| f.path()))
            .filter(|f| !keep.contains(f) && std::fs::remove_file(f).is_ok())
            .collect();
        {
            let mut s = self.lock();
            for f in &stale {
                s.images.remove(f);
            }
        }
        self.wake.notify_all();
        (self.on_change)();
    }
}
