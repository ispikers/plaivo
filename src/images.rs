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
    pub size: (u32, u32),
}

#[derive(Default)]
pub struct Queue {
    jobs: Mutex<VecDeque<Job>>,
    cv: Condvar,
}

impl Queue {
    // Overflow drops the oldest request no longer drawn; if all are still wanted, the new one waits a frame.
    pub fn push(&self, job: Job, wanted: impl Fn(&str) -> bool) -> Option<Job> {
        let mut jobs = self.jobs.lock().unwrap();
        let dropped = if jobs.len() < QUEUE_CAP {
            None
        } else {
            match jobs.iter().position(|j| !wanted(&j.key)) {
                Some(i) => jobs.remove(i),
                None => return Some(job),
            }
        };
        jobs.push_back(job);
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

fn fit(img: image::DynamicImage, (w, h): (u32, u32)) -> image::DynamicImage {
    if img.width() > w && img.height() > h {
        img.resize_to_fill(w, h, image::imageops::FilterType::Triangle)
    } else {
        img
    }
}

fn load(
    c: &Client,
    Job {
        key,
        url,
        local,
        size,
    }: &Job,
) -> Option<Arc<RenderImage>> {
    let local = *local;
    let file = if local {
        url.into()
    } else {
        store::cache_path("img", key)
    };
    let bytes = match std::fs::read(&file) {
        Ok(b) => {
            if !local {
                store::touch(&file);
            }
            b
        }
        Err(e) if local => {
            log::warn!("image {key}: {e}");
            return None;
        }
        Err(_) => {
            let b = c
                .fetch(url)
                .inspect_err(|e| log::warn!("image {key}: {e}"))
                .ok()?;
            let _ = store::write_atomic(&file, &b);
            b
        }
    };
    let img = image::load_from_memory(&bytes)
        .inspect_err(|e| log::warn!("image {key}: decode failed: {e}"))
        .ok()?;
    let mut rgba = if local { fit(img, *size) } else { img }.into_rgba8();
    for px in rgba.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(smallvec::smallvec![
        image::Frame::new(rgba)
    ])))
}

pub fn spawn_workers(
    q: Arc<Queue>,
    c: Client,
    done: impl Fn(String, Option<Arc<RenderImage>>) + Send + Sync + Clone + 'static,
) {
    for _ in 0..WORKERS {
        let (q, c, done) = (q.clone(), c.clone(), done.clone());
        std::thread::spawn(move || {
            loop {
                let job = q.pop();
                let img = load(&c, &job);
                done(job.key, img);
            }
        });
    }
}

pub struct Lru {
    map: HashMap<String, (Arc<RenderImage>, u64, usize)>,
    tick: u64,
    frame: u64,
    prev_frame: u64,
    bytes: usize,
    budget: usize,
}

impl Lru {
    pub fn new(budget: usize) -> Self {
        Self {
            map: HashMap::new(),
            tick: 0,
            frame: 0,
            prev_frame: 0,
            bytes: 0,
            budget,
        }
    }

    pub fn start_frame(&mut self) {
        self.prev_frame = self.frame;
        self.frame = self.tick;
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn used_last_frame(&self, key: &str) -> bool {
        self.map.get(key).is_some_and(|e| e.1 > self.prev_frame)
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
        self.frame = self.tick;
        self.evict()
    }

    // Images drawn since the frame started stay even over budget, or on-screen art would be evicted and re-requested forever.
    fn evict(&mut self) -> Vec<Arc<RenderImage>> {
        let mut evicted = Vec::new();
        while self.bytes > self.budget {
            let Some(oldest) = self
                .map
                .iter()
                .filter(|e| e.1.1 <= self.frame)
                .min_by_key(|e| e.1.1)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
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
        Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
            image::RgbaImage::new(w, 10)
        )]))
    }

    #[test]
    fn lru_evicts_least_recently_used_within_budget() {
        let mut lru = Lru::new(10 * 10 * 4 * 2);
        assert!(lru.insert("a".into(), img(10)).is_empty());
        assert!(lru.insert("b".into(), img(10)).is_empty());
        assert!(lru.get("a").is_some());
        lru.start_frame();
        assert_eq!(lru.insert("c".into(), img(10)).len(), 1);
        assert!(lru.get("b").is_none());
        assert!(lru.get("a").is_some() && lru.get("c").is_some());
        assert_eq!(lru.set_budget(10 * 10 * 4).len(), 1);
        assert!(lru.get("c").is_some() && !lru.contains("a"));
        assert!(lru.set_budget(10 * 10 * 4 * 2).is_empty());
    }

    #[test]
    fn images_drawn_this_frame_survive_over_budget() {
        let mut lru = Lru::new(10 * 10 * 4);
        lru.insert("a".into(), img(10));
        lru.start_frame();
        assert!(lru.get("a").is_some());
        assert!(lru.insert("b".into(), img(10)).is_empty());
        lru.start_frame();
        assert!(lru.get("b").is_some());
        assert_eq!(lru.insert("c".into(), img(10)).len(), 1);
        assert!(!lru.contains("a") && lru.contains("b"));
    }

    #[test]
    fn used_last_frame_tracks_draws_and_fresh_inserts() {
        let mut lru = Lru::new(usize::MAX);
        lru.insert("drawn".into(), img(10));
        lru.insert("hidden".into(), img(10));
        lru.start_frame();
        assert!(lru.used_last_frame("drawn") && lru.used_last_frame("hidden"));
        lru.get("drawn");
        lru.start_frame();
        assert!(lru.used_last_frame("drawn") && !lru.used_last_frame("hidden"));
        lru.insert("new".into(), img(10));
        lru.start_frame();
        assert!(lru.used_last_frame("new") && !lru.used_last_frame("drawn"));
    }

    #[test]
    fn local_art_shrinks_to_cover_the_drawn_size() {
        let big = image::DynamicImage::new_rgba8(960, 1440);
        let fitted = fit(big, (250, 375));
        assert_eq!((fitted.width(), fitted.height()), (250, 375));
        let wide = fit(image::DynamicImage::new_rgba8(960, 540), (400, 600));
        assert_eq!((wide.width(), wide.height()), (960, 540));
    }

    #[test]
    fn queue_drops_oldest_when_full() {
        let q = Queue::default();
        let job = |i: usize| Job {
            key: i.to_string(),
            url: String::new(),
            local: false,
            size: (0, 0),
        };
        let dropped: Vec<_> = (0..QUEUE_CAP + 2)
            .filter_map(|i| q.push(job(i), |_| false))
            .map(|j| j.key)
            .collect();
        assert_eq!(dropped, ["0", "1"]);
        let new = q.push(job(99), |_| true).unwrap();
        assert_eq!(new.key, "99");
        assert_eq!(q.push(job(99), |k| k != "5").unwrap().key, "5");
        assert_eq!(q.pop().key, "2");
    }
}
