use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

use crate::first;
use crate::plex::{
    Backend, Client, Ctx, Directory, Extras, Guid, Hub, Image, Item, Marker, Media, OnDeck,
    PRODUCT, Part, Person, Quality, R, Rating, Server, Stream, Tag, encode, err,
};

const TICKS_PER_MS: u64 = 10_000;
const LIST_FIELDS: &str = "ChildCount,RecursiveItemCount";
const DEFAULT_PORT: u16 = 8096;

fn ticks_to_ms(ticks: u64) -> u64 {
    ticks / TICKS_PER_MS
}

fn ms_to_ticks(ms: u64) -> u64 {
    ms * TICKS_PER_MS
}

fn auth_header(client_id: &str, token: Option<&str>) -> String {
    let mut h = format!(
        "MediaBrowser Client=\"{PRODUCT}\", Device=\"{}\", DeviceId=\"{}\", Version=\"{}\"",
        encode(std::env::consts::OS),
        encode(client_id),
        env!("CARGO_PKG_VERSION")
    );
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        h.push_str(&format!(", Token=\"{}\"", encode(t)));
    }
    h
}

pub(crate) fn authed<B>(
    r: ureq::RequestBuilder<B>,
    c: &Client,
    token: Option<&str>,
) -> ureq::RequestBuilder<B> {
    r.header("Accept", "application/json")
        .header("Authorization", auth_header(&c.client_id, token))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct Page {
    items: Vec<Dto>,
    total_record_count: Option<usize>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct Dto {
    id: String,
    name: String,
    #[serde(rename = "Type")]
    kind: String,
    collection_type: String,
    series_name: String,
    season_name: String,
    series_id: String,
    season_id: String,
    parent_id: String,
    index_number: Option<i64>,
    parent_index_number: Option<i64>,
    production_year: Option<i64>,
    overview: String,
    taglines: Vec<String>,
    official_rating: String,
    community_rating: Option<f64>,
    critic_rating: Option<f64>,
    people: Vec<JPerson>,
    run_time_ticks: Option<u64>,
    child_count: Option<u64>,
    recursive_item_count: Option<u64>,
    premiere_date: String,
    genres: Vec<String>,
    studios: Vec<Named>,
    image_tags: ImageTags,
    backdrop_image_tags: Vec<String>,
    parent_backdrop_item_id: String,
    parent_backdrop_image_tags: Vec<String>,
    series_primary_image_tag: String,
    parent_primary_image_item_id: String,
    parent_primary_image_tag: String,
    user_data: UserData,
    provider_ids: BTreeMap<String, String>,
    media_sources: Vec<Source>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct Named {
    name: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct JPerson {
    id: String,
    name: String,
    role: String,
    #[serde(rename = "Type")]
    kind: String,
    primary_image_tag: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct ImageTags {
    primary: String,
    logo: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct UserData {
    playback_position_ticks: u64,
    play_count: u64,
    played: bool,
    unplayed_item_count: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct Source {
    id: String,
    path: String,
    container: String,
    bitrate: Option<u64>,
    media_streams: Vec<MediaStream>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct MediaStream {
    #[serde(rename = "Type")]
    kind: String,
    index: i64,
    codec: String,
    language: String,
    display_title: String,
    is_external: bool,
    is_default: bool,
    is_forced: bool,
    width: Option<u64>,
    height: Option<u64>,
    channels: Option<u32>,
    video_range: String,
    video_range_type: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct Segments {
    items: Vec<Segment>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct Segment {
    #[serde(rename = "Type")]
    kind: String,
    start_ticks: u64,
    end_ticks: u64,
}

fn image(id: &str, kind: &str, tag: &str) -> String {
    if id.is_empty() || tag.is_empty() {
        String::new()
    } else {
        format!("/Items/{id}/Images/{kind}?tag={}", encode(tag))
    }
}

fn resolution(width: u64, height: u64) -> &'static str {
    match (width, height) {
        (w, h) if w >= 3200 || h >= 1700 => "4k",
        (w, h) if w >= 1800 || h >= 1000 => "1080",
        (w, h) if w >= 1200 || h >= 700 => "720",
        (_, h) if h >= 480 => "480",
        _ => "sd",
    }
}

fn dynamic_range(v: &MediaStream) -> String {
    match v.video_range_type.as_str() {
        t if t.starts_with("DOVI") => "Dolby Vision".into(),
        "HDR10Plus" => "HDR10+".into(),
        "" => v.video_range.clone(),
        t => t.into(),
    }
}

impl Source {
    fn into_media(self, item_id: &str) -> Media {
        let video = self.media_streams.iter().find(|s| s.kind == "Video");
        let audio = self
            .media_streams
            .iter()
            .filter(|s| s.kind == "Audio")
            .min_by_key(|s| !s.is_default);
        let stream = self
            .media_streams
            .iter()
            .filter_map(|s| {
                let stream_type = match s.kind.as_str() {
                    "Video" => 1,
                    "Audio" => 2,
                    "Subtitle" => 3,
                    _ => return None,
                };
                let format = if s.codec == "subrip" { "srt" } else { &s.codec };
                let key = if stream_type == 3 && s.is_external {
                    format!(
                        "/Videos/{item_id}/{}/Subtitles/{}/Stream.{format}",
                        self.id, s.index
                    )
                } else {
                    String::new()
                };
                Some(Stream {
                    stream_type,
                    key,
                    codec: s.codec.clone(),
                    language_code: s.language.clone(),
                    display_title: s.display_title.clone(),
                    forced: s.is_forced,
                    selected: s.is_default,
                })
            })
            .collect();
        Media {
            bitrate: self.bitrate.map(|b| b / 1000),
            height: video.and_then(|v| v.height),
            container: self.container,
            video_codec: video.map(|v| v.codec.clone()).unwrap_or_default(),
            video_resolution: video
                .map(|v| resolution(v.width.unwrap_or(0), v.height.unwrap_or(0)).to_owned())
                .unwrap_or_default(),
            video_dynamic_range: video.map(dynamic_range).unwrap_or_default(),
            audio_codec: audio.map(|a| a.codec.clone()).unwrap_or_default(),
            audio_channels: audio.and_then(|a| a.channels),
            source_id: self.id.clone(),
            part: vec![Part {
                key: format!(
                    "/Videos/{item_id}/stream?Static=true&MediaSourceId={}",
                    self.id
                ),
                file: self.path,
                stream,
            }],
        }
    }
}

impl Dto {
    fn into_item(self) -> Item {
        let kind = match self.kind.as_str() {
            "Movie" => "movie".to_owned(),
            "Series" => "show".to_owned(),
            "Season" => "season".to_owned(),
            "Episode" => "episode".to_owned(),
            "BoxSet" => "collection".to_owned(),
            k => k.to_ascii_lowercase(),
        };
        let backdrop = self
            .backdrop_image_tags
            .first()
            .map(|t| image(&self.id, "Backdrop/0", t))
            .unwrap_or_default();
        let parent_backdrop = self
            .parent_backdrop_image_tags
            .first()
            .map(|t| image(&self.parent_backdrop_item_id, "Backdrop/0", t))
            .unwrap_or_default();
        let series_thumb = image(&self.series_id, "Primary", &self.series_primary_image_tag);
        let parent_key = first(&[&self.season_id, &self.parent_id]).to_owned();
        let (
            parent_title,
            parent_rating_key,
            parent_thumb,
            grandparent_title,
            grandparent_rating_key,
            grandparent_thumb,
            grandparent_art,
        ) = match kind.as_str() {
            "episode" => (
                self.season_name,
                parent_key,
                image(
                    &self.parent_primary_image_item_id,
                    "Primary",
                    &self.parent_primary_image_tag,
                ),
                self.series_name,
                self.series_id,
                series_thumb,
                parent_backdrop.clone(),
            ),
            "season" => (
                self.series_name,
                first(&[&self.series_id, &self.parent_id]).to_owned(),
                series_thumb,
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
            _ => Default::default(),
        };
        let ud = self.user_data;
        let leaf_count = match kind.as_str() {
            "show" => self.recursive_item_count,
            "season" => self.recursive_item_count.or(self.child_count),
            _ => None,
        };
        let viewed_leaf_count = leaf_count.map(|n| match ud.unplayed_item_count {
            Some(u) => n.saturating_sub(u),
            None if ud.played => n,
            None => 0,
        });
        let guids = self
            .provider_ids
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| Guid {
                id: format!("{}://{v}", k.to_ascii_lowercase()),
            })
            .collect();
        let crew = |kind: &str| {
            self.people
                .iter()
                .filter(|p| p.kind == kind)
                .map(|p| Tag {
                    tag: p.name.clone(),
                })
                .collect::<Vec<_>>()
        };
        let (director, writer) = (crew("Director"), crew("Writer"));
        let role = self
            .people
            .iter()
            .filter(|p| p.kind == "Actor")
            .map(|p| Person {
                tag: p.name.clone(),
                role: p.role.clone(),
                thumb: image(&p.id, "Primary", &p.primary_image_tag),
            })
            .collect();
        let ratings = self
            .critic_rating
            .map(|r| Rating {
                image: format!(
                    "rottentomatoes://image.rating.{}",
                    if r >= 60. { "ripe" } else { "rotten" }
                ),
                value: r / 10.,
            })
            .into_iter()
            .collect();
        let images = Some(image(&self.id, "Logo", &self.image_tags.logo))
            .filter(|u| !u.is_empty())
            .map(|url| Image {
                kind: "clearLogo".into(),
                url,
            })
            .into_iter()
            .collect();
        let id = self.id;
        Item {
            role,
            director,
            writer,
            ratings,
            images,
            thumb: image(&id, "Primary", &self.image_tags.primary),
            art: first(&[&backdrop, &parent_backdrop]).to_owned(),
            media: self
                .media_sources
                .into_iter()
                .map(|s| s.into_media(&id))
                .collect(),
            rating_key: id,
            kind,
            title: self.name,
            parent_title,
            grandparent_title,
            parent_rating_key,
            grandparent_rating_key,
            parent_thumb,
            grandparent_thumb,
            grandparent_art,
            parent_index: self.parent_index_number,
            index: self.index_number,
            year: self.production_year,
            summary: self.overview,
            tagline: self.taglines.into_iter().next().unwrap_or_default(),
            content_rating: self.official_rating,
            audience_rating: self.community_rating,
            duration: self.run_time_ticks.map(ticks_to_ms),
            view_offset: Some(ticks_to_ms(ud.playback_position_ticks)).filter(|&o| o > 0),
            view_count: Some(if ud.played { ud.play_count.max(1) } else { 0 }),
            leaf_count,
            viewed_leaf_count,
            child_count: self.child_count,
            studio: self
                .studios
                .into_iter()
                .next()
                .map(|s| s.name)
                .unwrap_or_default(),
            originally_available_at: self.premiere_date.get(..10).unwrap_or_default().to_owned(),
            genre: self.genres.into_iter().map(|tag| Tag { tag }).collect(),
            guids,
            ..Default::default()
        }
    }
}

fn markers(s: Segments) -> Vec<Marker> {
    s.items
        .into_iter()
        .filter_map(|s| {
            let kind = match s.kind.as_str() {
                "Intro" => "intro",
                "Outro" => "credits",
                _ => return None,
            };
            Some(Marker {
                kind: kind.into(),
                start_time_offset: ticks_to_ms(s.start_ticks),
                end_time_offset: ticks_to_ms(s.end_ticks),
            })
        })
        .collect()
}

fn get<T: DeserializeOwned>(srv: &Server, c: &Ctx, path: &str) -> R<T> {
    serde_json::from_slice(&srv.fetch_cached(c, path)?).map_err(err)
}

fn get_quiet<T: DeserializeOwned>(srv: &Server, c: &Ctx, path: &str) -> R<T> {
    serde_json::from_slice(&srv.try_fetch_cached(c, path)?).map_err(err)
}

fn into_items(page: Page) -> Vec<Item> {
    page.items.into_iter().map(Dto::into_item).collect()
}

fn items(srv: &Server, c: &Ctx, path: &str) -> R<Vec<Item>> {
    get::<Page>(srv, c, path).map(into_items)
}

fn section_kind(collection_type: &str) -> Option<&'static str> {
    match collection_type {
        "movies" => Some("movie"),
        "tvshows" => Some("show"),
        _ => None,
    }
}

fn continue_hub(resume: Vec<Item>, next_up: Vec<Item>, parent: &str) -> Hub {
    let shows: HashSet<String> = resume
        .iter()
        .map(|i| i.grandparent_rating_key.clone())
        .filter(|k| !k.is_empty())
        .collect();
    let mut metadata = resume;
    metadata.extend(
        next_up
            .into_iter()
            .filter(|i| !shows.contains(&i.grandparent_rating_key)),
    );
    Hub {
        title: "Continue Watching".into(),
        key: format!("continue:{parent}"),
        id: "continueWatching".into(),
        kind: "mixed".into(),
        metadata,
    }
}

pub fn sections(srv: &Server, c: &Ctx) -> R<Vec<Directory>> {
    let views = get::<Page>(srv, c, &format!("/UserViews?userId={}", srv.user_id))?.items;
    Ok(views
        .into_iter()
        .filter_map(|v| {
            Some(Directory {
                kind: section_kind(&v.collection_type)?.into(),
                key: v.id,
                title: v.name,
            })
        })
        .collect())
}

pub fn hubs(srv: &Server, c: &Ctx) -> R<Vec<Hub>> {
    let u = &srv.user_id;
    let (views, resume, next_up, latest) = std::thread::scope(|s| {
        let resume = s.spawn(|| {
            items(
                srv,
                c,
                &format!(
                    "/UserItems/Resume?userId={u}&Limit=20&MediaTypes=Video&Fields={LIST_FIELDS}"
                ),
            )
        });
        let next_up = s.spawn(|| {
            items(
                srv,
                c,
                &format!("/Shows/NextUp?UserId={u}&Limit=20&Fields={LIST_FIELDS}"),
            )
        });
        let views = sections(srv, c);
        let latest: Vec<_> = views
            .iter()
            .flatten()
            .map(|v| {
                let path = format!("/Items/Latest?userId={u}&ParentId={}&Limit=20&GroupItems=true&Fields={LIST_FIELDS}", v.key);
                s.spawn(move || get::<Vec<Dto>>(srv, c, &path))
            })
            .collect();
        let ok =
            |r: std::thread::Result<R<Vec<Item>>>| r.ok().and_then(Result::ok).unwrap_or_default();
        (
            views,
            ok(resume.join()),
            ok(next_up.join()),
            latest
                .into_iter()
                .map(|j| j.join().ok().and_then(Result::ok).unwrap_or_default())
                .collect::<Vec<_>>(),
        )
    });
    let views = views?;
    let mut hubs = vec![continue_hub(resume, next_up, "")];
    hubs.extend(views.into_iter().zip(latest).map(|(v, dtos)| Hub {
        title: format!("Recently Added in {}", v.title),
        key: format!("latest:{}", v.key),
        id: format!("latest.{}", v.key),
        kind: v.kind,
        metadata: dtos.into_iter().map(Dto::into_item).collect(),
    }));
    hubs.retain(|h| !h.metadata.is_empty());
    Ok(hubs)
}

fn sort_params(sort: &str) -> (&'static str, &'static str) {
    let (field, dir) = sort.split_once(':').unwrap_or((sort, "asc"));
    let by = match field {
        "addedAt" => "DateCreated",
        "originallyAvailableAt" => "PremiereDate",
        "audienceRating" => "CommunityRating",
        _ => "SortName",
    };
    (
        by,
        if dir == "desc" {
            "Descending"
        } else {
            "Ascending"
        },
    )
}

pub fn section_items(
    srv: &Server,
    c: &Ctx,
    key: &str,
    start: usize,
    size: usize,
    sort: &str,
    summaries: bool,
) -> R<(Vec<Item>, usize)> {
    let (by, order) = sort_params(sort);
    paged(
        srv,
        c,
        &format!(
            "ParentId={key}&Recursive=true&IncludeItemTypes=Movie,Series&SortBy={by},SortName&SortOrder={order}"
        ),
        start,
        size,
        summaries,
    )
}

pub fn collections(srv: &Server, c: &Ctx, start: usize, size: usize) -> R<(Vec<Item>, usize)> {
    // BoxSets live in one server-wide folder; Jellyfin ignores ParentId for them.
    paged(
        srv,
        c,
        "IncludeItemTypes=BoxSet&Recursive=true&SortBy=SortName&SortOrder=Ascending",
        start,
        size,
        false,
    )
}

pub fn collection_items(
    srv: &Server,
    c: &Ctx,
    key: &str,
    start: usize,
    size: usize,
    summaries: bool,
) -> R<(Vec<Item>, usize)> {
    paged(srv, c, &format!("ParentId={key}"), start, size, summaries)
}

fn paged(
    srv: &Server,
    c: &Ctx,
    query: &str,
    start: usize,
    size: usize,
    summaries: bool,
) -> R<(Vec<Item>, usize)> {
    let overview = if summaries { ",Overview" } else { "" };
    let page: Page = get(
        srv,
        c,
        &format!(
            "/Items?userId={}&{query}&StartIndex={start}&Limit={size}&Fields={LIST_FIELDS}{overview}&ImageTypeLimit=1&EnableTotalRecordCount=true",
            srv.user_id
        ),
    )?;
    let total = page.total_record_count.unwrap_or(start + page.items.len());
    Ok((into_items(page), total))
}

pub fn section_hubs(srv: &Server, c: &Ctx, key: &str, kind: &str) -> R<Vec<Hub>> {
    let u = &srv.user_id;
    let (resume, next_up, latest) = std::thread::scope(|s| {
        let resume = s.spawn(|| items(srv, c, &format!("/UserItems/Resume?userId={u}&ParentId={key}&Limit=20&MediaTypes=Video&Fields={LIST_FIELDS}")));
        let next_up = s.spawn(|| {
            if kind == "show" {
                items(
                    srv,
                    c,
                    &format!(
                        "/Shows/NextUp?UserId={u}&ParentId={key}&Limit=20&Fields={LIST_FIELDS}"
                    ),
                )
            } else {
                Ok(Vec::new())
            }
        });
        let latest = get::<Vec<Dto>>(
            srv,
            c,
            &format!(
                "/Items/Latest?userId={u}&ParentId={key}&Limit=20&GroupItems=true&Fields={LIST_FIELDS}"
            ),
        );
        let ok =
            |r: std::thread::Result<R<Vec<Item>>>| r.ok().and_then(Result::ok).unwrap_or_default();
        (ok(resume.join()), ok(next_up.join()), latest)
    });
    let latest = Hub {
        title: "Recently Added".into(),
        key: format!("latest:{key}"),
        id: format!("latest.{key}"),
        kind: kind.into(),
        metadata: latest?.into_iter().map(Dto::into_item).collect(),
    };
    let mut hubs = vec![continue_hub(resume, next_up, key), latest];
    hubs.retain(|h| !h.metadata.is_empty());
    Ok(hubs)
}

pub fn hub_items(srv: &Server, c: &Ctx, key: &str) -> R<Vec<Item>> {
    let u = &srv.user_id;
    let max = crate::plex::HUB_MAX;
    if let Some(parent) = key.strip_prefix("continue:") {
        let scope = if parent.is_empty() {
            String::new()
        } else {
            format!("&ParentId={parent}")
        };
        let resume = items(
            srv,
            c,
            &format!(
                "/UserItems/Resume?userId={u}{scope}&Limit={max}&MediaTypes=Video&Fields={LIST_FIELDS}"
            ),
        )?;
        let next_up = items(
            srv,
            c,
            &format!("/Shows/NextUp?UserId={u}{scope}&Limit={max}&Fields={LIST_FIELDS}"),
        )
        .unwrap_or_default();
        return Ok(continue_hub(resume, next_up, parent).metadata);
    }
    let parent = key.strip_prefix("latest:").ok_or("Unknown list")?;
    Ok(get::<Vec<Dto>>(srv, c, &format!("/Items/Latest?userId={u}&ParentId={parent}&Limit={max}&GroupItems=true&Fields={LIST_FIELDS}"))?.into_iter().map(Dto::into_item).collect())
}

pub fn metadata(srv: &Server, c: &Ctx, key: &str) -> R<Item> {
    let u = &srv.user_id;
    let mut it = get::<Dto>(srv, c, &format!("/Items/{key}?userId={u}"))?.into_item();
    match it.kind.as_str() {
        "show" => {
            let next = items(
                srv,
                c,
                &format!("/Shows/NextUp?UserId={u}&SeriesId={key}&Limit=1&Fields={LIST_FIELDS}"),
            )
            .ok()
            .and_then(|v| v.into_iter().next());
            it.on_deck = next.map(|m| Box::new(OnDeck { metadata: Some(m) }));
        }
        "movie" | "episode" => {
            it.marker = get_quiet::<Segments>(srv, c, &format!("/MediaSegments/{key}"))
                .map(markers)
                .unwrap_or_default()
        }
        _ => {}
    }
    if matches!(it.kind.as_str(), "movie" | "show") {
        let list = |path: &str| {
            get_quiet::<Vec<Dto>>(srv, c, &format!("/Items/{key}/{path}?userId={u}"))
                .unwrap_or_default()
                .into_iter()
                .map(Dto::into_item)
        };
        let trailers = list("LocalTrailers").map(|t| Item {
            subtype: "trailer".into(),
            ..t
        });
        let metadata: Vec<Item> = trailers.chain(list("SpecialFeatures")).collect();
        it.extras = (!metadata.is_empty()).then_some(Extras { metadata });
    }
    Ok(it)
}

pub fn related(srv: &Server, c: &Ctx, key: &str) -> R<Vec<Hub>> {
    let metadata = items(
        srv,
        c,
        &format!(
            "/Items/{key}/Similar?userId={}&Limit=20&Fields={LIST_FIELDS}",
            srv.user_id
        ),
    )?;
    Ok((!metadata.is_empty())
        .then(|| Hub {
            title: "More Like This".into(),
            key: String::new(),
            id: "similar".into(),
            kind: "mixed".into(),
            metadata,
        })
        .into_iter()
        .collect())
}

pub fn clear_progress(srv: &Server, c: &Client, key: &str) -> R<()> {
    let url = format!(
        "{}/UserItems/{key}/UserData?userId={}",
        srv.uri, srv.user_id
    );
    authed(c.agent.post(&url), c, Some(&srv.token))
        .send_json(json!({ "PlaybackPositionTicks": 0 }))
        .map(drop)
        .map_err(err)
}

pub fn set_watched(srv: &Server, c: &Client, key: &str, watched: bool) -> R<()> {
    let url = format!("{}/UserPlayedItems/{key}?userId={}", srv.uri, srv.user_id);
    let r = if watched {
        authed(c.agent.post(&url), c, Some(&srv.token)).send_empty()
    } else {
        authed(c.agent.delete(&url), c, Some(&srv.token)).call()
    };
    r.map(drop).map_err(err)
}

pub fn children(srv: &Server, c: &Ctx, key: &str) -> R<Vec<Item>> {
    let u = &srv.user_id;
    match get_quiet::<Page>(
        srv,
        c,
        &format!("/Shows/{key}/Seasons?UserId={u}&Fields={LIST_FIELDS}"),
    ) {
        Ok(seasons) => Ok(into_items(seasons)),
        Err(_) => items(
            srv,
            c,
            &format!(
                "/Shows/{key}/Episodes?UserId={u}&SeasonId={key}&Fields={LIST_FIELDS},Overview"
            ),
        ),
    }
}

pub fn all_episodes(srv: &Server, c: &Ctx, show: &str) -> R<Vec<Item>> {
    items(
        srv,
        c,
        &format!(
            "/Shows/{show}/Episodes?UserId={}&Fields={LIST_FIELDS},Overview",
            srv.user_id
        ),
    )
}

pub fn search(srv: &Server, c: &Ctx, q: &str) -> R<Vec<(Item, f64)>> {
    let found = items(
        srv,
        c,
        &format!(
            "/Items?userId={}&SearchTerm={}&IncludeItemTypes=Movie,Series,Episode&Recursive=true&Limit=40&Fields={LIST_FIELDS}",
            srv.user_id,
            encode(q)
        ),
    )?;
    Ok(found
        .into_iter()
        .enumerate()
        .map(|(i, it)| (it, 1.0 - i as f64 * 0.01))
        .collect())
}

static STARTED: Mutex<String> = Mutex::new(String::new());

fn progress_path(key: &str, state: &str) -> &'static str {
    let mut started = STARTED.lock().unwrap_or_else(|e| e.into_inner());
    match state {
        "stopped" => {
            started.clear();
            "/Sessions/Playing/Stopped"
        }
        _ if *started != key => {
            key.clone_into(&mut started);
            "/Sessions/Playing"
        }
        _ => "/Sessions/Playing/Progress",
    }
}

fn progress_body(key: &str, state: &str, time_ms: u64, transcode_session: Option<&str>) -> Value {
    json!({
        "ItemId": key,
        "PositionTicks": ms_to_ticks(time_ms),
        "IsPaused": state == "paused",
        "CanSeek": true,
        "PlayMethod": if transcode_session.is_some() { "Transcode" } else { "DirectPlay" },
        "PlaySessionId": transcode_session,
    })
}

pub fn timeline(
    srv: &Server,
    c: &Client,
    key: &str,
    state: &str,
    time_ms: u64,
    transcode_session: Option<&str>,
) -> bool {
    let path = progress_path(key, state);
    match authed(
        c.agent.post(format!("{}{path}", srv.uri)),
        c,
        Some(&srv.token),
    )
    .send_json(progress_body(key, state, time_ms, transcode_session))
    {
        Ok(_) => {
            log::debug!("{} {path} {key} at {}s", srv.name, time_ms / 1000);
            true
        }
        Err(e) => {
            log::warn!("{} {path} for {key} failed: {e}", srv.name);
            false
        }
    }
}

fn sep(path: &str) -> char {
    if path.contains('?') { '&' } else { '?' }
}

pub fn image_url(srv: &Server, path: &str, w: u32, h: u32) -> String {
    let bound = if path.contains("/Images/Logo") {
        "max"
    } else {
        "fill"
    };
    format!(
        "{}{path}{}{bound}Width={w}&{bound}Height={h}&quality=90",
        srv.uri,
        sep(path)
    )
}

pub fn media_url(srv: &Server, key: &str) -> String {
    format!("{}{key}{}ApiKey={}", srv.uri, sep(key), encode(&srv.token))
}

pub fn transcode_url(
    srv: &Server,
    c: &Client,
    key: &str,
    source_id: &str,
    q: &Quality,
    session: &str,
    audio_codecs: &str,
) -> String {
    let (bps, height) = ((q.kbps as u64 * 1000).to_string(), q.height.to_string());
    let width = q.resolution.split('x').next().unwrap_or_default();
    let params = [
        ("MediaSourceId", first(&[source_id, key])),
        ("DeviceId", &c.client_id),
        ("PlaySessionId", session),
        ("VideoCodec", "h264"),
        ("AudioCodec", audio_codecs),
        ("MaxStreamingBitrate", &bps),
        ("VideoBitrate", &bps),
        ("MaxWidth", width),
        ("MaxHeight", &height),
        ("SegmentContainer", "ts"),
        ("EnableAdaptiveBitrateStreaming", "false"),
        ("ApiKey", &srv.token),
    ];
    let query: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{k}={}", encode(v)))
        .collect();
    format!("{}/Videos/{key}/master.m3u8?{}", srv.uri, query.join("&"))
}

pub fn stop_transcode(srv: &Server, c: &Client, session: &str) {
    let url = format!(
        "{}/Videos/ActiveEncodings?DeviceId={}&PlaySessionId={}",
        srv.uri,
        encode(&c.client_id),
        encode(session)
    );
    if let Err(e) = authed(c.agent.delete(&url), c, Some(&srv.token)).call() {
        log::debug!("{} stop transcode {session}: {e}", srv.name);
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerInfo {
    pub uri: String,
    pub name: String,
    pub id: String,
    pub version: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct PublicInfo {
    server_name: String,
    id: String,
    version: String,
    product_name: String,
}

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct QuickConnect {
    pub code: String,
    pub secret: String,
    pub authenticated: bool,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct AuthResult {
    user: User,
    access_token: String,
    server_id: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct User {
    id: String,
    name: String,
    policy: Policy,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct Policy {
    is_administrator: bool,
}

fn candidates(address: &str) -> Vec<String> {
    let a = address.trim().trim_end_matches('/');
    if a.is_empty() {
        return Vec::new();
    }
    if a.contains("://") {
        return vec![a.to_owned()];
    }
    let (host, rest) = a
        .split_once('/')
        .map_or((a, String::new()), |(h, r)| (h, format!("/{r}")));
    let has_port = host
        .rsplit_once(':')
        .is_some_and(|(_, p)| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    let mut v = vec![format!("https://{a}"), format!("http://{a}")];
    if !has_port {
        v.push(format!("http://{host}:{DEFAULT_PORT}{rest}"));
    }
    v
}

fn read_json<T: DeserializeOwned>(
    r: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    rejected: &str,
) -> R<T> {
    match r {
        Ok(mut r) => r.body_mut().read_json().map_err(err),
        Err(ureq::Error::StatusCode(401 | 403)) => Err(rejected.into()),
        Err(e) => Err(err(e)),
    }
}

fn session(info: &ServerInfo, auth: AuthResult) -> R<Server> {
    if auth.access_token.is_empty() || auth.user.id.is_empty() {
        return Err("The server did not return a session".into());
    }
    Ok(Server {
        name: first(&[&info.name, &info.uri]).to_owned(),
        id: first(&[&auth.server_id, &info.id]).to_owned(),
        uri: info.uri.clone(),
        token: auth.access_token,
        owned: auth.user.policy.is_administrator,
        backend: Backend::Jellyfin,
        user_id: auth.user.id,
        user: auth.user.name,
    })
}

pub fn probe(c: &Client, address: &str) -> R<ServerInfo> {
    let mut last = String::from("Enter the server address");
    for uri in candidates(address) {
        let r = c
            .agent
            .get(format!("{uri}/System/Info/Public"))
            .config()
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .header("Accept", "application/json")
            .call();
        match read_json::<PublicInfo>(r, "Access denied") {
            Ok(info) if info.product_name.contains("Jellyfin") => {
                return Ok(ServerInfo {
                    uri,
                    name: info.server_name,
                    id: info.id,
                    version: info.version,
                });
            }
            Ok(_) => last = format!("{uri} is not a Jellyfin server"),
            Err(e) => last = e,
        }
        log::debug!("jellyfin probe {uri}: {last}");
    }
    Err(last)
}

pub fn sign_in(c: &Client, info: &ServerInfo, username: &str, password: &str) -> R<Server> {
    let r = authed(
        c.agent
            .post(format!("{}/Users/AuthenticateByName", info.uri)),
        c,
        None,
    )
    .send_json(json!({ "Username": username, "Pw": password }));
    session(info, read_json(r, "Wrong username or password")?)
}

pub fn quick_connect_start(c: &Client, info: &ServerInfo) -> R<QuickConnect> {
    let url = format!("{}/QuickConnect/Initiate", info.uri);
    let r = match authed(c.agent.post(&url), c, None).send_empty() {
        Err(ureq::Error::StatusCode(405)) => authed(c.agent.get(&url), c, None).call(),
        r => r,
    };
    let qc: QuickConnect = read_json(r, "Quick Connect is turned off on this server")?;
    if qc.code.is_empty() || qc.secret.is_empty() {
        return Err("The server did not return a Quick Connect code".into());
    }
    Ok(qc)
}

pub fn quick_connect_ready(c: &Client, info: &ServerInfo, secret: &str) -> R<bool> {
    let r = authed(
        c.agent.get(format!(
            "{}/QuickConnect/Connect?Secret={}",
            info.uri,
            encode(secret)
        )),
        c,
        None,
    )
    .call();
    if matches!(r, Err(ureq::Error::StatusCode(404))) {
        return Err("The Quick Connect code expired".into());
    }
    Ok(read_json::<QuickConnect>(r, "Quick Connect is turned off on this server")?.authenticated)
}

pub fn quick_connect_finish(c: &Client, info: &ServerInfo, secret: &str) -> R<Server> {
    let r = authed(
        c.agent
            .post(format!("{}/Users/AuthenticateWithQuickConnect", info.uri)),
        c,
        None,
    )
    .send_json(json!({ "Secret": secret }));
    session(info, read_json(r, "Quick Connect was not approved")?)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubtitleMode {
    Always,
    OnlyForced,
    None,
    Smart,
    #[default]
    #[serde(other)]
    Default,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct LanguagePrefs {
    #[serde(rename = "AudioLanguagePreference")]
    pub audio: String,
    #[serde(rename = "SubtitleLanguagePreference")]
    pub subtitles: String,
    #[serde(rename = "SubtitleMode")]
    pub subtitle_mode: SubtitleMode,
}

fn user_config(srv: &Server, c: &Client) -> R<Value> {
    let r = authed(
        c.agent.get(format!("{}/Users/Me", srv.uri)),
        c,
        Some(&srv.token),
    )
    .call();
    let mut user: Value = read_json(r, "Signed out of the Jellyfin server")?;
    user.get_mut("Configuration")
        .map(Value::take)
        .filter(Value::is_object)
        .ok_or_else(|| "The server did not return the user configuration".into())
}

fn apply_prefs(config: &mut Value, prefs: &LanguagePrefs) {
    config["AudioLanguagePreference"] = json!(prefs.audio);
    config["SubtitleLanguagePreference"] = json!(prefs.subtitles);
    config["SubtitleMode"] = json!(prefs.subtitle_mode);
}

pub fn language_prefs(srv: &Server, c: &Client) -> R<LanguagePrefs> {
    serde_json::from_value(user_config(srv, c)?).map_err(err)
}

pub fn set_language_prefs(srv: &Server, c: &Client, prefs: &LanguagePrefs) -> R<()> {
    let mut config = user_config(srv, c)?;
    apply_prefs(&mut config, prefs);
    authed(
        c.agent.post(format!(
            "{}/Users/Configuration?userId={}",
            srv.uri, srv.user_id
        )),
        c,
        Some(&srv.token),
    )
    .send_json(&config)
    .map_err(err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture<T: DeserializeOwned>(name: &str) -> T {
        let path = format!(
            "{}/tests/fixtures/jellyfin/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap()
    }

    fn page(name: &str) -> Vec<Item> {
        fixture::<Page>(name)
            .items
            .into_iter()
            .map(Dto::into_item)
            .collect()
    }

    fn server() -> Server {
        Server {
            name: "nas".into(),
            id: "sid".into(),
            uri: "http://nas:8096".into(),
            token: "tok en".into(),
            backend: Backend::Jellyfin,
            user_id: "u1".into(),
            ..Default::default()
        }
    }

    #[test]
    fn maps_movie() {
        let it = fixture::<Dto>("movie").into_item();
        assert_eq!(
            (it.kind.as_str(), it.rating_key.as_str(), it.title.as_str()),
            ("movie", "8a1d7b5e3c4f4e0e9d2b6a7c1f0e3d21", "Inception")
        );
        assert_eq!(
            (it.year, it.duration, it.view_offset, it.view_count),
            (Some(2010), Some(8_880_000), Some(3_600_000), Some(0))
        );
        assert_eq!(
            (
                it.tagline.as_str(),
                it.content_rating.as_str(),
                it.audience_rating
            ),
            ("Your mind is the scene of the crime.", "PG-13", Some(8.4))
        );
        assert_eq!(
            (it.studio.as_str(), it.originally_available_at.as_str()),
            ("Legendary Pictures", "2010-07-15")
        );
        assert_eq!(
            it.genre.iter().map(|g| g.tag.as_str()).collect::<Vec<_>>(),
            ["Action", "Science Fiction"]
        );
        assert_eq!(
            it.thumb,
            "/Items/8a1d7b5e3c4f4e0e9d2b6a7c1f0e3d21/Images/Primary?tag=a3f1c2d4e5b6"
        );
        assert_eq!(
            it.art,
            "/Items/8a1d7b5e3c4f4e0e9d2b6a7c1f0e3d21/Images/Backdrop/0?tag=bd01"
        );
        let ids: Vec<&str> = it.guids.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, ["imdb://tt1375666", "tmdb://27205"]);
        assert!((it.progress() - 0.405).abs() < 0.001);

        let m = &it.media[0];
        assert_eq!(
            (m.bitrate, m.height, m.container.as_str()),
            (Some(25_312), Some(2160), "mkv")
        );
        assert_eq!(
            (m.video_resolution.as_str(), m.video_dynamic_range.as_str()),
            ("4k", "HDR10")
        );
        assert_eq!(
            (
                m.audio_codec.as_str(),
                m.audio_channels,
                m.source_id.as_str()
            ),
            ("eac3", Some(6), "8a1d7b5e3c4f4e0e9d2b6a7c1f0e3d21")
        );
        let part = &m.part[0];
        assert_eq!(
            part.key,
            "/Videos/8a1d7b5e3c4f4e0e9d2b6a7c1f0e3d21/stream?Static=true&MediaSourceId=8a1d7b5e3c4f4e0e9d2b6a7c1f0e3d21"
        );
        assert_eq!(
            part.file,
            "/media/movies/Inception (2010)/Inception (2010).mkv"
        );
        let types: Vec<u8> = part.stream.iter().map(|s| s.stream_type).collect();
        assert_eq!(types, [1, 2, 2, 3, 3]);
        let ext = &part.stream[3];
        assert_eq!(
            (
                ext.key.as_str(),
                ext.language_code.as_str(),
                ext.display_title.as_str()
            ),
            (
                "/Videos/8a1d7b5e3c4f4e0e9d2b6a7c1f0e3d21/8a1d7b5e3c4f4e0e9d2b6a7c1f0e3d21/Subtitles/5/Stream.srt",
                "eng",
                "English - SUBRIP - External"
            )
        );
        assert!(
            part.stream[4].key.is_empty(),
            "embedded subtitles stay in the file"
        );
    }

    #[test]
    fn maps_series_with_watch_counts() {
        let it = fixture::<Dto>("series").into_item();
        assert_eq!(
            (
                it.kind.as_str(),
                it.child_count,
                it.leaf_count,
                it.viewed_leaf_count
            ),
            ("show", Some(2), Some(20), Some(5))
        );
        assert_eq!(it.unwatched(), 15);
        assert!(!it.watched());
        assert_eq!(
            it.guids.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
            ["imdb://tt0903747", "tmdb://1396", "tvdb://81189"]
        );
    }

    #[test]
    fn maps_seasons() {
        let seasons = page("seasons");
        assert_eq!(seasons.len(), 2);
        let s1 = &seasons[0];
        assert_eq!(
            (
                s1.kind.as_str(),
                s1.title.as_str(),
                s1.index,
                s1.parent_title.as_str()
            ),
            ("season", "Season 1", Some(1), "Breaking Bad")
        );
        assert_eq!(
            (
                s1.parent_rating_key.as_str(),
                s1.leaf_count,
                s1.viewed_leaf_count
            ),
            ("3c9e1a7b2d4f4a8e8b6c5d4e3f2a1b0c", Some(7), Some(7))
        );
        assert!(s1.watched());
        assert_eq!(
            s1.parent_thumb,
            "/Items/3c9e1a7b2d4f4a8e8b6c5d4e3f2a1b0c/Images/Primary?tag=series01"
        );
        assert_eq!(
            s1.art,
            "/Items/3c9e1a7b2d4f4a8e8b6c5d4e3f2a1b0c/Images/Backdrop/0?tag=sbd01"
        );
        let show = seasons[1].clone().as_show();
        assert_eq!(
            (show.rating_key.as_str(), show.title.as_str()),
            ("3c9e1a7b2d4f4a8e8b6c5d4e3f2a1b0c", "Breaking Bad")
        );
        assert!(!seasons[1].watched());
    }

    #[test]
    fn maps_episodes() {
        let eps = page("episodes");
        assert_eq!(eps.len(), 3);
        let e = &eps[1];
        assert_eq!(
            (e.kind.as_str(), e.episode_label(), e.title.as_str()),
            ("episode", "S2 · E2".to_owned(), "Grilled")
        );
        assert_eq!(
            (e.grandparent_title.as_str(), e.parent_title.as_str()),
            ("Breaking Bad", "Season 2")
        );
        assert_eq!(
            (
                e.grandparent_rating_key.as_str(),
                e.parent_rating_key.as_str()
            ),
            (
                "3c9e1a7b2d4f4a8e8b6c5d4e3f2a1b0c",
                "5d2e8f1a3b4c4d6e9f0a1b2c3d4e5f60"
            )
        );
        assert_eq!(
            e.grandparent_thumb,
            "/Items/3c9e1a7b2d4f4a8e8b6c5d4e3f2a1b0c/Images/Primary?tag=series01"
        );
        assert_eq!(
            e.card_art(true),
            "/Items/e2000000000000000000000000000002/Images/Primary?tag=ep02"
        );
        assert_eq!(e.card_art(false), e.grandparent_thumb);
        assert_eq!(
            e.card_text(),
            ("Breaking Bad".to_owned(), "S2 · E2 · Grilled".to_owned())
        );
        assert!(eps[0].watched() && !eps[1].watched());
        assert_eq!(eps[1].view_offset, Some(754_000));
        assert_eq!(
            eps[2].summary,
            "Walt and Jesse try to find a way to get to their RV."
        );
    }

    #[test]
    fn maps_views_and_continue_hub() {
        let views: Page = fixture("views");
        let kinds: Vec<_> = views
            .items
            .iter()
            .filter_map(|v| Some((v.name.as_str(), section_kind(&v.collection_type)?)))
            .collect();
        assert_eq!(kinds, [("Movies", "movie"), ("Shows", "show")]);
        let resume = page("resume");
        assert_eq!(
            resume.iter().map(|i| i.kind.as_str()).collect::<Vec<_>>(),
            ["movie", "episode"]
        );
        let next_up = page("episodes");
        let hub = continue_hub(resume, next_up, "");
        assert!(hub.is_continue() && hub.wide());
        assert_eq!(
            hub.metadata.len(),
            2,
            "next-up episodes of a show already resuming are dropped"
        );
        let latest: Vec<Dto> = fixture("latest");
        assert_eq!(
            latest
                .into_iter()
                .map(|d| d.into_item().kind)
                .collect::<Vec<_>>(),
            ["movie", "show"]
        );
    }

    #[test]
    fn markers_from_segments() {
        let s: Segments = serde_json::from_str(r#"{"Items":[{"Id":"a","ItemId":"e","Type":"Intro","StartTicks":0,"EndTicks":900000000},{"Type":"Recap","StartTicks":1,"EndTicks":2},{"Type":"Outro","StartTicks":25000000000,"EndTicks":26400000000}],"TotalRecordCount":3}"#).unwrap();
        let m = markers(s);
        assert_eq!(
            m.iter()
                .map(|m| (m.kind.as_str(), m.start_time_offset, m.end_time_offset))
                .collect::<Vec<_>>(),
            [("intro", 0, 90_000), ("credits", 2_500_000, 2_640_000)]
        );
    }

    #[test]
    fn ticks_round_trip() {
        assert_eq!(ms_to_ticks(1500), 15_000_000);
        assert_eq!(ticks_to_ms(88_800_000_000), 8_880_000);
        assert_eq!(ticks_to_ms(ms_to_ticks(123_456)), 123_456);
        let body = progress_body("abc", "paused", 61_000, None);
        assert_eq!(
            (
                body["PositionTicks"].as_u64(),
                body["IsPaused"].as_bool(),
                body["ItemId"].as_str()
            ),
            (Some(610_000_000), Some(true), Some("abc"))
        );
        assert_eq!(
            (body["PlayMethod"].as_str(), body["PlaySessionId"].is_null()),
            (Some("DirectPlay"), true)
        );
        let body = progress_body("abc", "playing", 0, Some("plaivo-1f"));
        assert_eq!(
            (body["PlayMethod"].as_str(), body["PlaySessionId"].as_str()),
            (Some("Transcode"), Some("plaivo-1f"))
        );
    }

    #[test]
    fn progress_starts_once_per_item() {
        assert_eq!(progress_path("x1", "playing"), "/Sessions/Playing");
        assert_eq!(progress_path("x1", "paused"), "/Sessions/Playing/Progress");
        assert_eq!(progress_path("x1", "stopped"), "/Sessions/Playing/Stopped");
        assert_eq!(progress_path("x1", "playing"), "/Sessions/Playing");
        assert_eq!(progress_path("x2", "playing"), "/Sessions/Playing");
    }

    #[test]
    fn urls() {
        let srv = server();
        let c = Client::new("dev 1".into());
        assert_eq!(
            media_url(&srv, "/Videos/i/stream?Static=true&MediaSourceId=i"),
            "http://nas:8096/Videos/i/stream?Static=true&MediaSourceId=i&ApiKey=tok%20en"
        );
        assert_eq!(
            media_url(&srv, "/Videos/i/s/Subtitles/3/Stream.srt"),
            "http://nas:8096/Videos/i/s/Subtitles/3/Stream.srt?ApiKey=tok%20en"
        );
        assert_eq!(
            image_url(&srv, "/Items/i/Images/Primary?tag=t", 300, 450),
            "http://nas:8096/Items/i/Images/Primary?tag=t&fillWidth=300&fillHeight=450&quality=90"
        );
        let url = transcode_url(
            &srv,
            &c,
            "i",
            "v2",
            Quality::find(4000).unwrap(),
            "s1",
            "aac,ac3,eac3,mp3",
        );
        assert!(url.starts_with("http://nas:8096/Videos/i/master.m3u8?MediaSourceId=v2&DeviceId=dev%201&PlaySessionId=s1&"), "{url}");
        assert!(
            transcode_url(&srv, &c, "i", "", Quality::find(4000).unwrap(), "s1", "aac")
                .contains("MediaSourceId=i&")
        );
        for p in [
            "MaxStreamingBitrate=4000000",
            "VideoBitrate=4000000",
            "MaxWidth=1280",
            "MaxHeight=720",
            "VideoCodec=h264",
            "AudioCodec=aac%2Cac3%2Ceac3%2Cmp3",
            "ApiKey=tok%20en",
        ] {
            assert!(url.contains(p), "{p} missing from {url}");
        }
        assert_eq!(sort_params("addedAt:desc"), ("DateCreated", "Descending"));
        assert_eq!(sort_params("titleSort"), ("SortName", "Ascending"));
    }

    #[test]
    fn server_methods_dispatch_to_jellyfin() {
        let srv = server();
        assert_eq!(
            srv.media_url("/Videos/i/stream?Static=true"),
            media_url(&srv, "/Videos/i/stream?Static=true")
        );
        assert_eq!(
            srv.image_url("/Items/i/Images/Primary?tag=t", 10, 20),
            image_url(&srv, "/Items/i/Images/Primary?tag=t", 10, 20)
        );
        let plex = Server {
            backend: Backend::Plex,
            ..srv
        };
        assert!(plex.media_url("/library/parts/1").contains("X-Plex-Token="));
    }

    #[test]
    fn auth_header_shape() {
        assert_eq!(
            auth_header("abc 1", None),
            format!(
                "MediaBrowser Client=\"Plaivo\", Device=\"{}\", DeviceId=\"abc%201\", Version=\"{}\"",
                std::env::consts::OS,
                env!("CARGO_PKG_VERSION")
            )
        );
        assert!(auth_header("abc", Some("t\"k")).ends_with(", Token=\"t%22k\""));
        assert!(!auth_header("abc", Some("")).contains("Token"));
    }

    #[test]
    fn address_candidates() {
        assert_eq!(
            candidates(" https://jf.example.com/ "),
            ["https://jf.example.com"]
        );
        assert_eq!(
            candidates("192.168.1.5"),
            [
                "https://192.168.1.5",
                "http://192.168.1.5",
                "http://192.168.1.5:8096"
            ]
        );
        assert_eq!(
            candidates("nas:8920"),
            ["https://nas:8920", "http://nas:8920"]
        );
        assert_eq!(
            candidates("host/jellyfin"),
            [
                "https://host/jellyfin",
                "http://host/jellyfin",
                "http://host:8096/jellyfin"
            ]
        );
        assert!(candidates("  ").is_empty());
    }

    #[test]
    fn sign_in_result_becomes_server() {
        let info = ServerInfo {
            uri: "http://nas:8096".into(),
            name: "nas".into(),
            id: "probe-id".into(),
            version: "10.10.7".into(),
        };
        let srv = session(&info, fixture("auth")).unwrap();
        assert_eq!(
            (srv.backend, srv.user_id.as_str(), srv.token.as_str()),
            (
                Backend::Jellyfin,
                "4a2b6c8d0e1f4a3b5c7d9e0f1a2b3c4d",
                "0b5d8e2f7a1c4b9e8d3f6a2c5b7e9d1f"
            )
        );
        assert_eq!(
            (
                srv.id.as_str(),
                srv.name.as_str(),
                srv.owned,
                srv.user.as_str()
            ),
            ("f2a9c37b5e1d4c8a9b6e3d2c1a0f9e8d", "nas", true, "alice")
        );
        assert!(session(&info, AuthResult::default()).is_err());
        let qc: QuickConnect = serde_json::from_str(r#"{"Authenticated":true,"Secret":"s3cr3t","Code":"482913","DeviceId":"d","DeviceName":"windows","AppName":"Plaivo","AppVersion":"0.1.0","DateAdded":"2026-11-08T10:00:00.0000000Z"}"#).unwrap();
        assert_eq!(
            (qc.code.as_str(), qc.secret.as_str(), qc.authenticated),
            ("482913", "s3cr3t", true)
        );
    }

    #[test]
    fn language_prefs_read_and_patch() {
        let mut user: Value = fixture("auth");
        let mut config = user["User"]["Configuration"].take();
        let prefs: LanguagePrefs = serde_json::from_value(config.clone()).unwrap();
        assert_eq!(
            prefs,
            LanguagePrefs {
                audio: "jpn".into(),
                subtitles: "eng".into(),
                subtitle_mode: SubtitleMode::Smart
            }
        );
        apply_prefs(
            &mut config,
            &LanguagePrefs {
                audio: "eng".into(),
                subtitles: String::new(),
                subtitle_mode: SubtitleMode::OnlyForced,
            },
        );
        assert_eq!(
            (
                config["AudioLanguagePreference"].as_str(),
                config["SubtitleLanguagePreference"].as_str(),
                config["SubtitleMode"].as_str()
            ),
            (Some("eng"), Some(""), Some("OnlyForced"))
        );
        assert_eq!(
            config["PlayDefaultAudioTrack"],
            json!(true),
            "untouched fields survive the write"
        );
        let odd: LanguagePrefs = serde_json::from_str(r#"{"SubtitleMode":"Whatever"}"#).unwrap();
        assert_eq!(odd.subtitle_mode, SubtitleMode::Default);
    }
}
