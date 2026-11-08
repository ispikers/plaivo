use crate::plex::Client;
use crate::store;
use gpui::RenderImage;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};

const QUEUE_CAP: usize = 96;
const WORKERS: usize = 6;

pub struct Job {
    pub key: String,
    pub url: String,
    pub local: bool,
}

#[derive(Default)]
pub struct Queue {
    jobs: Mutex<VecDeque<Job>>,
    cv: Condvar,
}

impl Queue {
    // Overflow drops the oldest request: that row is the one most likely to have scrolled away.
    pub fn push(&self, job: Job) -> Option<Job> {
        let mut jobs = self.jobs.lock().unwrap();
        jobs.push_back(job);
        let dropped = (jobs.len() > QUEUE_CAP).then(|| jobs.pop_front()).flatten();
        self.cv.notify_one();
        dropped
    }

    fn pop(&self) -> Job {
        let mut jobs = self.jobs.lock().unwrap();
        loop {
            if let Some(j) = jobs.pop_front() {
                return j;
            }
            jobs = self.cv.wait(jobs).unwrap();
        }
    }
}

fn load(c: &Client, key: &str, url: &str, local: bool) -> Option<Arc<RenderImage>> {
    let file = if local { url.into() } else { store::cache_path("img", key) };
    let bytes = match std::fs::read(&file) {
        Ok(b) => b,
        Err(e) if local => {
            log::warn!("image {key}: {e}");
            return None;
        }
        Err(_) => {
            let b = c.fetch(url).inspect_err(|e| log::warn!("image {key}: {e}")).ok()?;
            let _ = store::write_atomic(&file, &b);
            b
        }
    };
    let mut rgba = image::load_from_memory(&bytes).inspect_err(|e| log::warn!("image {key}: decode failed: {e}")).ok()?.into_rgba8();
    for px in rgba.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(rgba)])))
}

pub fn spawn_workers(q: Arc<Queue>, c: Client, done: impl Fn(String, Option<Arc<RenderImage>>) + Send + Sync + Clone + 'static) {
    for _ in 0..WORKERS {
        let (q, c, done) = (q.clone(), c.clone(), done.clone());
        std::thread::spawn(move || {
            loop {
                let job = q.pop();
                let img = load(&c, &job.key, &job.url, job.local);
                done(job.key, img);
            }
        });
    }
}

pub struct Lru {
    map: HashMap<String, (Arc<RenderImage>, u64, usize)>,
    tick: u64,
    bytes: usize,
    budget: usize,
}

impl Lru {
    pub fn new(budget: usize) -> Self {
        Self { map: HashMap::new(), tick: 0, bytes: 0, budget }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    pub fn get(&mut self, key: &str) -> Option<Arc<RenderImage>> {
        self.tick += 1;
        let e = self.map.get_mut(key)?;
        e.1 = self.tick;
        Some(e.0.clone())
    }

    pub fn insert(&mut self, key: String, img: Arc<RenderImage>) -> Vec<Arc<RenderImage>> {
        let size = img.size(0);
        let cost = size.width.0 as usize * size.height.0 as usize * 4;
        self.tick += 1;
        let mut evicted = Vec::new();
        if let Some(old) = self.map.insert(key, (img, self.tick, cost)) {
            self.bytes -= old.2;
            evicted.push(old.0);
        }
        self.bytes += cost;
        evicted.extend(self.evict());
        evicted
    }

    pub fn set_budget(&mut self, budget: usize) -> Vec<Arc<RenderImage>> {
        self.budget = budget;
        self.evict()
    }

    fn evict(&mut self) -> Vec<Arc<RenderImage>> {
        let mut evicted = Vec::new();
        while self.bytes > self.budget && self.map.len() > 1 {
            let oldest = self.map.iter().min_by_key(|e| e.1.1).map(|(k, _)| k.clone()).unwrap();
            let (img, _, cost) = self.map.remove(&oldest).unwrap();
            self.bytes -= cost;
            evicted.push(img);
        }
        evicted
    }

    pub fn drain(&mut self) -> Vec<Arc<RenderImage>> {
        self.bytes = 0;
        self.map.drain().map(|(_, v)| v.0).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32) -> Arc<RenderImage> {
        Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(image::RgbaImage::new(w, 10))]))
    }

    #[test]
    fn lru_evicts_least_recently_used_within_budget() {
        let mut lru = Lru::new(10 * 10 * 4 * 2);
        assert!(lru.insert("a".into(), img(10)).is_empty());
        assert!(lru.insert("b".into(), img(10)).is_empty());
        assert!(lru.get("a").is_some());
        assert_eq!(lru.insert("c".into(), img(10)).len(), 1);
        assert!(lru.get("b").is_none());
        assert!(lru.get("a").is_some() && lru.get("c").is_some());
        assert_eq!(lru.set_budget(10 * 10 * 4).len(), 1);
        assert!(lru.get("c").is_some() && !lru.contains("a"));
        assert!(lru.set_budget(10 * 10 * 4 * 2).is_empty());
    }

    #[test]
    fn queue_drops_oldest_when_full() {
        let q = Queue::default();
        let job = |i: usize| Job { key: i.to_string(), url: String::new(), local: false };
        let dropped: Vec<_> = (0..QUEUE_CAP + 2).filter_map(|i| q.push(job(i))).map(|j| j.key).collect();
        assert_eq!(dropped, ["0", "1"]);
        assert_eq!(q.pop().key, "2");
    }
}
