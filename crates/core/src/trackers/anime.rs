use super::http::{Request, send};
use super::{EpisodeOrder, Media};
use crate::store;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

const URL: &str = "https://github.com/edde746/anime-mappings/releases/latest/download/anime-mappings.bin.gz";
const REFRESH_SECS: u64 = 86_400;
const RETRY_SECS: u64 = 600;
const SHORT: &str = "anime mappings cut short";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimeMatch {
    pub mal: Option<u64>,
    pub anilist: Option<u64>,
    pub progress: u32,
}

fn unzigzag(n: u64) -> i64 {
    (n >> 1) as i64 ^ -((n & 1) as i64)
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    end: usize,
}

impl<'a> Reader<'a> {
    fn byte(&mut self) -> Result<u8, String> {
        if self.pos >= self.end {
            return Err(SHORT.into());
        }
        self.pos += 1;
        Ok(self.data[self.pos - 1])
    }

    fn varint(&mut self) -> Result<u64, String> {
        let mut n = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.byte()?;
            n |= ((b & 0x7f) as u64) << shift;
            if b < 0x80 {
                return Ok(n);
            }
        }
        Err("anime mappings varint too long".into())
    }

    fn take(&mut self, len: u64) -> Result<&'a [u8], String> {
        let len = usize::try_from(len).ok().filter(|&l| l <= self.end - self.pos).ok_or(SHORT)?;
        self.pos += len;
        Ok(&self.data[self.pos - len..self.pos])
    }

    fn text(&mut self) -> Result<String, String> {
        let len = self.varint()?;
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Int(i64),
    Str(String),
    Ints(Vec<i64>),
    Strs(Vec<String>),
}

struct Table<'a> {
    data: &'a [u8],
    rows: usize,
    cols: HashMap<String, (u8, usize, usize)>,
}

impl Table<'_> {
    fn column(&self, name: &str) -> Result<Vec<Option<Val>>, String> {
        let mut out = vec![None; self.rows];
        let Some(&(kind, start, end)) = self.cols.get(name).filter(|c| (1..=6).contains(&c.0)) else { return Ok(out) };
        let mut r = Reader { data: self.data, pos: start, end };
        let words = if kind == 4 { (0..r.varint()?).map(|_| r.text()).collect::<Result<Vec<_>, _>>()? } else { Vec::new() };
        let mut previous = 0i64;
        for slot in out.iter_mut() {
            let tag = r.varint()?;
            if tag == 0 {
                continue;
            }
            let t = tag - 1;
            *slot = Some(match kind {
                1 => Val::Int(unzigzag(t)),
                2 => {
                    previous = previous.wrapping_add(unzigzag(t));
                    Val::Int(previous)
                }
                3 => Val::Str(String::from_utf8_lossy(r.take(t)?).into_owned()),
                4 => Val::Str(words.get(t as usize).cloned().ok_or("anime mappings dictionary index out of range")?),
                5 => Val::Ints((0..t).map(|_| r.varint().map(unzigzag)).collect::<Result<_, _>>()?),
                _ => Val::Strs((0..t).map(|_| r.text()).collect::<Result<_, _>>()?),
            });
        }
        if r.pos != end {
            return Err("anime mappings column has trailing bytes".into());
        }
        Ok(out)
    }

    fn ints(&self, name: &str) -> Result<Vec<Option<i64>>, String> {
        Ok(self.column(name)?.into_iter().map(|v| if let Some(Val::Int(n)) = v { Some(n) } else { None }).collect())
    }

    fn rows_with(&self, name: &str, id: Option<u64>) -> Result<Vec<usize>, String> {
        let Some(id) = id.and_then(|n| i64::try_from(n).ok()) else { return Ok(Vec::new()) };
        Ok(self.column(name)?.iter().enumerate().filter(|(_, v)| matches!(v, Some(Val::Int(n)) if *n == id) || matches!(v, Some(Val::Ints(ns)) if ns.contains(&id))).map(|(i, _)| i).collect())
    }
}

fn inflate(raw: &[u8]) -> Result<Vec<u8>, String> {
    if !raw.starts_with(&[0x1f, 0x8b]) {
        return Ok(raw.to_vec());
    }
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(raw).read_to_end(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

fn fribb(data: &[u8]) -> Result<Table<'_>, String> {
    let mut r = Reader { data, pos: 0, end: data.len() };
    if r.take(4)? != b"AMAP" {
        return Err("not an anime mappings file".into());
    }
    if r.byte()? != 1 {
        return Err("unsupported anime mappings version".into());
    }
    let mut found = None;
    for _ in 0..r.varint()? {
        let name = r.text()?;
        let rows = usize::try_from(r.varint()?).map_err(|e| e.to_string())?;
        let mut cols = HashMap::new();
        for _ in 0..r.varint()? {
            let col = r.text()?;
            let kind = r.byte()?;
            let len = r.varint()?;
            let start = r.pos;
            r.take(len)?;
            cols.insert(col, (kind, start, r.pos));
        }
        if name == "fribb" {
            found = Some(Table { data, rows, cols });
        }
    }
    found.filter(|t| t.rows <= 1 << 22).ok_or_else(|| "anime mappings have no usable fribb table".into())
}

fn num(media: &Media, key: &str) -> Option<u64> {
    let ids = if media.movie { &media.ids } else { &media.show_ids };
    ids.get(key).and_then(|v| v.parse::<u64>().ok()).filter(|&n| n > 0)
}

fn series_level(media: &Media) -> bool {
    media.movie || matches!(media.season, None | Some(1))
}

fn unit(media: &Media) -> Option<u32> {
    if media.movie { Some(1) } else { media.episode.filter(|&e| e > 0) }
}

pub(super) fn direct(media: &Media) -> Option<AnimeMatch> {
    let (mal, anilist) = (num(media, "mal"), num(media, "anilist"));
    (series_level(media) && (mal.is_some() || anilist.is_some())).then_some(AnimeMatch { mal, anilist, progress: unit(media)? })
}

pub(super) fn mappable(media: &Media) -> bool {
    let ids = if media.movie { &media.ids } else { &media.show_ids };
    ["anidb", "tvdb", "tmdb", "imdb"].iter().any(|k| ids.contains_key(*k))
}

pub(super) fn resolve(raw: &[u8], media: &Media) -> Result<Option<AnimeMatch>, String> {
    if let Some(m) = direct(media) {
        return Ok(Some(m));
    }
    let Some(progress) = unit(media) else { return Ok(None) };
    if media.season == Some(0) && !media.movie {
        return Ok(None);
    }
    let data = inflate(raw)?;
    let t = fribb(&data)?;
    let found = find(&t, media, progress)?;
    let Some((row, progress)) = found else { return Ok(None) };
    let id = |name: &str| -> Result<Option<u64>, String> { Ok(t.ints(name)?[row].and_then(|n| u64::try_from(n).ok()).filter(|&n| n > 0)) };
    Ok(Some(AnimeMatch { mal: id("mal_id")?, anilist: id("anilist_id")?, progress }))
}

fn single(rows: Vec<usize>) -> Option<usize> {
    if let [only] = rows[..] { Some(only) } else { None }
}

fn imdb_rows(t: &Table, media: &Media) -> Result<Vec<usize>, String> {
    let ids = if media.movie { &media.ids } else { &media.show_ids };
    let Some(imdb) = ids.get("imdb") else { return Ok(Vec::new()) };
    Ok(t.column("imdb_id")?.iter().enumerate().filter(|(_, v)| matches!(v, Some(Val::Strs(s)) if s.contains(imdb))).map(|(i, _)| i).collect())
}

fn find(t: &Table, media: &Media, progress: u32) -> Result<Option<(usize, u32)>, String> {
    let series = series_level(media);
    if series && let Some(row) = single(t.rows_with("anidb_id", num(media, "anidb"))?) {
        return Ok(Some((row, progress)));
    }
    if media.movie {
        let mut rows = t.rows_with("themoviedb_id.movie", num(media, "tmdb"))?;
        if rows.is_empty() {
            rows = imdb_rows(t, media)?;
        }
        return Ok(single(rows).map(|r| (r, 1)));
    }
    let order = media.order.unwrap_or(if num(media, "tvdb").is_some() { EpisodeOrder::Tvdb } else { EpisodeOrder::Tmdb });
    let (id_col, season_col, offset_col, key) = match order {
        EpisodeOrder::Tvdb => ("tvdb_id", "season.tvdb", "episode_offset.tvdb", "tvdb"),
        EpisodeOrder::Tmdb => ("themoviedb_id.tv", "season.tmdb", "episode_offset.tmdb", "tmdb"),
    };
    let rows = t.rows_with(id_col, num(media, key))?;
    if rows.is_empty() {
        return Ok(if series { single(imdb_rows(t, media)?).map(|r| (r, progress)) } else { None });
    }
    let (seasons, offsets) = (t.ints(season_col)?, t.ints(offset_col)?);
    if let [only] = rows[..]
        && seasons[only].is_none()
    {
        return Ok(series.then_some((only, progress)));
    }
    let Some(season) = media.season.map(i64::from) else { return Ok(None) };
    let episode = i64::from(progress);
    let candidates: Vec<(usize, i64)> = rows.into_iter().filter(|&r| seasons[r] == Some(season)).map(|r| (r, offsets[r].unwrap_or(0))).filter(|&(_, off)| off < episode).collect();
    let Some(best) = candidates.iter().map(|c| c.1).max() else { return Ok(None) };
    let Some(row) = single(candidates.iter().filter(|c| c.1 == best).map(|c| c.0).collect()) else { return Ok(None) };
    Ok(u32::try_from(episode - best).ok().map(|p| (row, p)))
}

#[derive(Default, Serialize, Deserialize)]
struct Meta {
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    checked: u64,
}

pub(super) struct Mappings {
    dir: PathBuf,
    attempted: u64,
}

impl Mappings {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, attempted: 0 }
    }

    fn file(&self) -> PathBuf {
        self.dir.join("anime-mappings.bin.gz")
    }

    fn meta_path(&self) -> PathBuf {
        self.dir.join("anime-mappings.json")
    }

    pub fn resolve(&mut self, media: &Media, now: u64) -> Result<Option<AnimeMatch>, ()> {
        if let Some(m) = direct(media) {
            return Ok(Some(m));
        }
        if !mappable(media) {
            return Ok(None);
        }
        let raw = self.load(now).ok_or(())?;
        resolve(&raw, media).map_err(|e| {
            log::warn!("cached anime mappings unreadable, will download again: {e}");
            let _ = std::fs::remove_file(self.file());
            let _ = std::fs::remove_file(self.meta_path());
        })
    }

    fn load(&mut self, now: u64) -> Option<Vec<u8>> {
        let cached = std::fs::read(self.file()).ok();
        let meta: Meta = store::read_json(&self.meta_path()).unwrap_or_default();
        let due = cached.is_none() || now >= meta.checked + REFRESH_SECS;
        if due && now >= self.attempted + RETRY_SECS {
            self.attempted = now;
            if let Some(fresh) = self.refresh(meta, now, cached.is_some()) {
                return Some(fresh);
            }
        }
        cached
    }

    fn refresh(&mut self, meta: Meta, now: u64, have_copy: bool) -> Option<Vec<u8>> {
        let mut req = Request::get(URL);
        if let (Some(etag), true) = (&meta.etag, have_copy) {
            req = req.header("If-None-Match", etag.clone());
        }
        let resp = send(&req).inspect_err(|e| log::warn!("anime mappings download failed: {e}")).ok()?;
        match resp.status {
            304 => self.save(Meta { checked: now, ..meta }),
            200 => match inflate(&resp.body).and_then(|d| fribb(&d).map(|t| t.rows)) {
                Ok(rows) => {
                    if let Err(e) = store::write_atomic(&self.file(), &resp.body) {
                        log::warn!("could not cache anime mappings: {e}");
                    }
                    log::info!("anime mappings updated: {rows} entries");
                    self.save(Meta { etag: resp.etag, checked: now });
                    return Some(resp.body);
                }
                Err(e) => log::warn!("anime mappings rejected: {e}"),
            },
            s => log::warn!("anime mappings download returned {s}"),
        }
        None
    }

    fn save(&self, meta: Meta) {
        store::write_json(&self.meta_path(), &meta);
    }
}

#[cfg(test)]
mod tests {
    use super::super::Ids;
    use super::*;
    use std::io::Write;

    fn varint(out: &mut Vec<u8>, mut n: u64) {
        loop {
            let b = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                return out.push(b);
            }
            out.push(b | 0x80);
        }
    }

    fn zig(n: i64) -> u64 {
        ((n << 1) ^ (n >> 63)) as u64
    }

    fn text(out: &mut Vec<u8>, s: &str) {
        varint(out, s.len() as u64);
        out.extend_from_slice(s.as_bytes());
    }

    fn int_column(values: &[Option<i64>], delta: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let mut prev = 0;
        for v in values {
            match v {
                None => varint(&mut out, 0),
                Some(n) => {
                    varint(&mut out, zig(if delta { n - prev } else { *n }) + 1);
                    if delta {
                        prev = *n;
                    }
                }
            }
        }
        out
    }

    fn str_list_column(values: &[&[&str]]) -> Vec<u8> {
        let mut out = Vec::new();
        for v in values {
            varint(&mut out, if v.is_empty() { 0 } else { v.len() as u64 + 1 });
            for s in *v {
                text(&mut out, s);
            }
        }
        out
    }

    fn int_list_column(values: &[&[i64]]) -> Vec<u8> {
        let mut out = Vec::new();
        for v in values {
            varint(&mut out, if v.is_empty() { 0 } else { v.len() as u64 + 1 });
            for n in *v {
                varint(&mut out, zig(*n));
            }
        }
        out
    }

    fn fixture() -> Vec<u8> {
        let o = Option::<i64>::None;
        let s = |n: i64| Some(n);
        let rows = 10;
        let columns: Vec<(&str, u8, Vec<u8>)> = vec![
            ("anidb_id", 2, int_column(&[s(10), s(11), s(12), s(13), s(14), s(15), s(16), s(17), s(18), s(19)], true)),
            ("mal_id", 2, int_column(&[s(100), s(101), s(102), s(103), s(104), o, s(106), s(107), s(108), s(109)], true)),
            ("anilist_id", 2, int_column(&[s(200), s(201), s(202), s(203), o, s(205), s(206), o, o, o], true)),
            ("tvdb_id", 1, int_column(&[s(5000), s(5000), s(5000), o, o, s(6000), s(5000), s(7000), s(7000), s(7000)], false)),
            ("season.tvdb", 1, int_column(&[s(1), s(1), s(2), o, o, o, s(0), s(1), s(1), s(2)], false)),
            ("episode_offset.tvdb", 1, int_column(&[o, s(12), o, o, o, o, o, s(0), o, o], false)),
            ("themoviedb_id.tv", 1, int_column(&[s(70), s(70), s(70), o, o, o, o, o, o, o], false)),
            ("season.tmdb", 1, int_column(&[s(1), s(2), s(3), o, o, o, o, o, o, o], false)),
            ("themoviedb_id.movie", 5, int_list_column(&[&[], &[], &[], &[900, 901], &[], &[], &[], &[], &[], &[]])),
            ("imdb_id", 6, str_list_column(&[&[], &[], &[], &["tt0001"], &["tt0002"], &[], &[], &[], &[], &[]])),
            ("future_column", 9, vec![1, 2, 3]),
        ];
        let mut out = b"AMAP\x01".to_vec();
        varint(&mut out, 2);
        text(&mut out, "anime_lists");
        varint(&mut out, 0);
        varint(&mut out, 0);
        text(&mut out, "fribb");
        varint(&mut out, rows);
        varint(&mut out, columns.len() as u64);
        for (name, kind, data) in columns {
            text(&mut out, name);
            out.push(kind);
            varint(&mut out, data.len() as u64);
            out.extend(data);
        }
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&out).unwrap();
        gz.finish().unwrap()
    }

    fn ids(pairs: &[(&str, &str)]) -> Ids {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn ep(show: &[(&str, &str)], season: u32, episode: u32) -> Media {
        Media { show_ids: ids(show), season: Some(season), episode: Some(episode), ..Media::default() }
    }

    fn movie(pairs: &[(&str, &str)]) -> Media {
        Media { movie: true, ids: ids(pairs), ..Media::default() }
    }

    fn m(mal: Option<u64>, anilist: Option<u64>, progress: u32) -> Option<AnimeMatch> {
        Some(AnimeMatch { mal, anilist, progress })
    }

    #[test]
    fn resolves_against_fixture() {
        let raw = fixture();
        let r = |media: Media| resolve(&raw, &media).unwrap();
        let show = [("tvdb", "5000"), ("tmdb", "70"), ("imdb", "tt9999")];
        assert_eq!(r(ep(&show, 1, 5)), m(Some(100), Some(200), 5));
        assert_eq!(r(ep(&show, 1, 13)), m(Some(101), Some(201), 1));
        assert_eq!(r(ep(&show, 2, 3)), m(Some(102), Some(202), 3));
        assert_eq!(r(ep(&show, 3, 1)), None);
        assert_eq!(r(ep(&show, 0, 5)), None);
        assert_eq!(r(Media { order: Some(EpisodeOrder::Tmdb), ..ep(&show, 2, 4) }), m(Some(101), Some(201), 4));
        assert_eq!(r(ep(&[("tmdb", "70")], 3, 2)), m(Some(102), Some(202), 2));
        assert_eq!(r(ep(&[("tvdb", "6000")], 1, 7)), m(None, Some(205), 7));
        assert_eq!(r(ep(&[("tvdb", "6000")], 4, 7)), None);
        assert_eq!(r(ep(&[("tvdb", "7000")], 1, 2)), None);
        assert_eq!(r(ep(&[("tvdb", "7000")], 2, 2)), m(Some(109), None, 2));
        assert_eq!(r(movie(&[("tmdb", "901")])), m(Some(103), Some(203), 1));
        assert_eq!(r(movie(&[("tmdb", "70")])), None);
        assert_eq!(r(movie(&[("imdb", "tt0002")])), m(Some(104), None, 1));
        assert_eq!(r(ep(&[("anidb", "12"), ("tvdb", "5000")], 1, 2)), m(Some(102), Some(202), 2));
        assert_eq!(r(ep(&[("anidb", "12")], 2, 2)), None);
        assert_eq!(r(ep(&[("anilist", "77")], 1, 3)), m(None, Some(77), 3));
        assert_eq!(r(ep(&[("anilist", "77")], 2, 3)), None);
        assert_eq!(r(ep(&[("tvdb", "1"), ("imdb", "tt5")], 1, 1)), None);
        assert!(!mappable(&ep(&[("plex", "plex://show/1")], 1, 1)));
    }

    #[test]
    fn rejects_garbage() {
        assert!(resolve(b"nope", &ep(&[("tvdb", "1")], 1, 1)).is_err());
        let mut bad = b"AMAP\x01".to_vec();
        bad.push(1);
        assert!(resolve(&bad, &ep(&[("tvdb", "1")], 1, 1)).is_err());
    }

    #[test]
    #[ignore]
    fn live_anime_mappings() {
        let dir = std::env::temp_dir().join(format!("plaivo-mappings-{}", std::process::id()));
        let mut mappings = Mappings::new(dir.clone());
        let now = super::super::now();
        let mut r = |label: &str, media: Media| {
            let hit = mappings.resolve(&media, now).expect("mappings available");
            println!("{label} -> {hit:?}");
            hit
        };
        let bebop = r("Cowboy Bebop tvdb 76885 S1E1", ep(&[("tvdb", "76885")], 1, 1));
        r("Attack on Titan tvdb 267440 S3E13", ep(&[("tvdb", "267440")], 3, 13));
        r("Attack on Titan tvdb 267440 S0E5", ep(&[("tvdb", "267440")], 0, 5));
        r("Frieren tmdb 209867 S1E1", ep(&[("tmdb", "209867")], 1, 1));
        r("Jujutsu Kaisen tmdb order 95479 S1E30", Media { order: Some(EpisodeOrder::Tmdb), ..ep(&[("tvdb", "377543"), ("tmdb", "95479")], 1, 30) });
        assert_eq!(bebop.and_then(|m| m.mal), Some(1));
        assert_eq!(bebop.and_then(|m| m.anilist), Some(1));
        let meta: Meta = store::read_json(&mappings.meta_path()).unwrap();
        println!("etag: {:?}", meta.etag);
        let mut again = Mappings::new(dir.clone());
        assert!(again.load(meta.checked + REFRESH_SECS + RETRY_SECS).is_some());
        let after: Meta = store::read_json(&again.meta_path()).unwrap();
        println!("conditional refresh kept etag: {}", after.etag == meta.etag);
        let _ = std::fs::remove_dir_all(dir);
    }
}
