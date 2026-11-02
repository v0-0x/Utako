//! Music videos from YouTube, through YouTube's own API (InnerTube) the way its apps use it:
//! no sign-in, no yt-dlp and none of YouTube's JavaScript. Search goes through the website's
//! client. A video's streams come from the visionOS app's client, whose links play as they
//! are: yt-dlp uses the same one when it has no JavaScript runtime. Videos YouTube only shows
//! to signed-in adults, and some made for kids, don't play this way; they say so.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::library::tags::Metadata;

/// Where YouTube is: its website (watch pages) and its API. Tests point them elsewhere.
#[derive(Debug, Clone)]
pub struct Site {
    pub web: String,
    pub api: String,
}

impl Default for Site {
    fn default() -> Self {
        Site {
            web: "https://www.youtube.com".into(),
            api: "https://www.youtube.com/youtubei/v1".into(),
        }
    }
}

/// An InnerTube client: who Utako says it is to YouTube's API.
struct Client {
    name: &'static str,
    /// Its number, for the `X-YouTube-Client-Name` header.
    id: u32,
    version: &'static str,
    user_agent: &'static str,
    /// More about the device (`deviceMake`, `osName`…).
    device: &'static [(&'static str, &'static str)],
}

/// The website, for search.
const WEB: Client = Client {
    name: "WEB",
    id: 1,
    version: "2.20260708.00.00",
    user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/140.0.0.0 Safari/537.36",
    device: &[],
};

/// The visionOS app, for a video's streams: its links need no signature and no proof of being
/// a browser ("PO token"), so they play as they come.
const VISIONOS: Client = Client {
    name: "VISIONOS",
    id: 101,
    version: "1.02",
    user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) \
                 Version/26.0 Safari/605.1.15",
    device: &[
        ("deviceMake", "Apple"),
        ("deviceModel", "RealityDevice17,1"),
        ("osName", "visionOS"),
        ("osVersion", "26.5.23O471"),
    ],
};

/// Search results: videos only.
const VIDEOS_ONLY: &str = "EgIQAfABAQ==";

/// Cookies every request carries, as yt-dlp sends them: the cookie banner answered (so Europe
/// gets the page, not the question) and English.
const BASE_COOKIES: &str = "SOCS=CAI; PREF=hl=en&tz=UTC";

/// A visitor: the ID and cookies YouTube gives a browser on its first page, sent with every
/// request after it, as a browser does. Kept for a few hours.
#[derive(Debug, Clone, Default)]
struct Visitor {
    id: Option<String>,
    cookies: String,
}

static VISITOR: Mutex<Option<(Visitor, Instant)>> = Mutex::new(None);
const VISITOR_FOR: Duration = Duration::from_secs(6 * 3600);

/// A search result.
#[derive(Debug, Clone, PartialEq)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: String,
    /// Seconds.
    pub duration: Option<f64>,
}

/// A link and what to send with it: the User-Agent and any headers the link was made for.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Link {
    pub url: String,
    pub user_agent: Option<String>,
    pub headers: Vec<(String, String)>,
}

/// A music video's picture, for the player in the app's centre panel: a stream on YouTube, or a
/// video file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VideoLink {
    pub link: Link,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// "VP9 720p"
    pub label: String,
}

/// One of a video's streams.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Format {
    pub itag: u32,
    pub url: String,
    /// "video/mp4", "audio/webm"
    pub mime: String,
    /// "avc1.640028", "opus"; the first of the codecs of one with picture and sound.
    pub codec: String,
    pub video: bool,
    pub audio: bool,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// Bits a second.
    pub bitrate: u64,
    /// Bytes, when YouTube says.
    pub size: Option<u64>,
    /// "Stable volume": a quieter mix of the sound, never chosen.
    pub drc: bool,
    /// The video's own language (a dubbed sound track isn't).
    pub original: bool,
}

/// What a video plays from.
#[derive(Debug, Clone, PartialEq)]
pub struct Streams {
    /// The sound, for mpv.
    pub sound: Link,
    /// The picture, for the player in the centre panel.
    pub picture: Option<VideoLink>,
    /// Seconds YouTube makes an app wait before the streams answer (its ad's length).
    pub wait: u64,
}

/// A saved music video.
#[derive(Debug)]
pub struct Saved {
    pub path: PathBuf,
    /// What was saved, for the Downloads list: "YouTube, H.264 1080p".
    pub label: String,
}

// ------------------------------------------------------------------------------------- API

fn context(client: &Client) -> Value {
    let mut c = json!({
        "clientName": client.name,
        "clientVersion": client.version,
        "hl": "en",
        "timeZone": "UTC",
        "utcOffsetMinutes": 0,
    });
    for (k, v) in client.device {
        c[*k] = json!(v);
    }
    if client.id != WEB.id {
        c["userAgent"] = json!(client.user_agent);
    }
    json!({ "client": c })
}

/// A call to YouTube's API (`search`, `player`) as `client`.
async fn call(
    http: &reqwest::Client,
    site: &Site,
    client: &Client,
    visitor: &Visitor,
    endpoint: &str,
    mut body: Value,
) -> Result<Value> {
    body["context"] = context(client);
    let mut req = http
        .post(format!("{}/{endpoint}?prettyPrint=false", site.api))
        .header(reqwest::header::USER_AGENT, client.user_agent)
        .header("X-YouTube-Client-Name", client.id.to_string())
        .header("X-YouTube-Client-Version", client.version)
        .header("Origin", "https://www.youtube.com")
        .header(reqwest::header::COOKIE, cookies(visitor))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .timeout(Duration::from_secs(20))
        .body(body.to_string());
    if let Some(id) = &visitor.id {
        req = req.header("X-Goog-Visitor-Id", id);
    }
    let resp = req.send().await.map_err(|e| {
        if e.is_connect() || e.is_timeout() {
            anyhow!("couldn't reach YouTube. Check your internet connection")
        } else {
            anyhow!("couldn't reach YouTube ({})", e.without_url())
        }
    })?;
    let status = resp.status();
    let bytes = crate::http::read_capped(resp, 16 << 20).await?;
    if !status.is_success() {
        let v: Value = serde_json::from_slice(&bytes).unwrap_or_default();
        match v["error"]["message"].as_str().filter(|m| !m.is_empty()) {
            Some(msg) => bail!("YouTube answered {status}: {msg}"),
            None => bail!("YouTube answered {status}"),
        }
    }
    serde_json::from_slice(&bytes).context("YouTube's answer wasn't what Utako expected")
}

fn cookies(visitor: &Visitor) -> String {
    if visitor.cookies.is_empty() {
        BASE_COOKIES.to_string()
    } else {
        format!("{BASE_COOKIES}; {}", visitor.cookies)
    }
}

/// The visitor ID and cookies from a watch page, as a browser gets them on its first visit,
/// and the page.
async fn visit(http: &reqwest::Client, site: &Site, video: &str) -> Result<(Visitor, String)> {
    let resp = http
        .get(format!("{}/watch?v={video}&bpctr=9999999999&has_verified=1", site.web))
        .header(reqwest::header::USER_AGENT, WEB.user_agent)
        .header(reqwest::header::COOKIE, BASE_COOKIES)
        .timeout(Duration::from_secs(15))
        .send()
        .await?
        .error_for_status()?;
    let cookies: Vec<String> = resp
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .map(|pair| pair.trim().to_string())
        .filter(|pair| {
            pair.split_once('=')
                .is_some_and(|(name, value)| !name.is_empty() && !value.is_empty() && name != "SOCS" && name != "PREF")
        })
        .collect();
    let page = String::from_utf8_lossy(&crate::http::read_capped(resp, 8 << 20).await?).into_owned();
    let id = json_string(&page, "VISITOR_DATA").or_else(|| json_string(&page, "visitorData"));
    Ok((
        Visitor {
            id,
            cookies: cookies.join("; "),
        },
        page,
    ))
}

/// The visitor kept, or a new one (from `video`'s watch page) when there's none or `fresh`.
async fn visitor(http: &reqwest::Client, site: &Site, video: &str, fresh: bool) -> Visitor {
    if !fresh {
        if let Some((v, at)) = VISITOR.lock().unwrap().clone() {
            if at.elapsed() < VISITOR_FOR {
                return v;
            }
        }
    }
    match visit(http, site, video).await {
        Ok((v, _)) => {
            *VISITOR.lock().unwrap() = Some((v.clone(), Instant::now()));
            v
        }
        Err(e) => {
            warn!("YouTube's watch page: {e:#}");
            Visitor::default()
        }
    }
}

/// The text of `"key":"…"` in a page's scripts.
fn json_string(page: &str, key: &str) -> Option<String> {
    let start = page.find(&format!("\"{key}\":\""))? + key.len() + 4;
    let rest = &page[start..];
    let end = rest.find('"')?;
    let value = rest[..end].replace("\\u003d", "=");
    (!value.is_empty()).then_some(value)
}

// ---------------------------------------------------------------------------------- search

/// Searches YouTube's videos.
pub async fn search(http: &reqwest::Client, query: &str, limit: usize) -> Result<Vec<Video>> {
    search_at(http, &Site::default(), query, limit).await
}

pub async fn search_at(http: &reqwest::Client, site: &Site, query: &str, limit: usize) -> Result<Vec<Video>> {
    let known = VISITOR.lock().unwrap().clone().map(|(v, _)| v).unwrap_or_default();
    let body = json!({ "query": query, "params": VIDEOS_ONLY });
    let answer = call(http, site, &WEB, &known, "search", body).await?;
    // A visitor ID from the search does for streams too, until a watch page gives one.
    if let Some(id) = answer["responseContext"]["visitorData"].as_str() {
        let mut kept = VISITOR.lock().unwrap();
        if kept.is_none() {
            let v = Visitor {
                id: Some(id.to_string()),
                cookies: String::new(),
            };
            *kept = Some((v, Instant::now()));
        }
    }
    let mut videos = parse_search(&answer);
    videos.truncate(limit);
    Ok(videos)
}

/// The videos in a search answer, in order: the older `videoRenderer`s and the newer
/// `lockupViewModel`s alike. Live streams (no length) are left out.
pub fn parse_search(answer: &Value) -> Vec<Video> {
    let results = answer
        .pointer("/contents/twoColumnSearchResultsRenderer/primaryContents")
        .unwrap_or(answer);
    let mut found = Vec::new();
    collect_videos(results, &mut found);
    let mut seen = std::collections::HashSet::new();
    found
        .into_iter()
        .filter(|v| v.duration.is_some() && is_video_id(&v.id) && seen.insert(v.id.clone()))
        .collect()
}

fn collect_videos(v: &Value, out: &mut Vec<Video>) {
    match v {
        Value::Object(map) => {
            for (key, value) in map {
                match key.as_str() {
                    "videoRenderer" => out.extend(video_renderer(value)),
                    "lockupViewModel" => out.extend(lockup(value)),
                    _ => collect_videos(value, out),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|i| collect_videos(i, out)),
        _ => {}
    }
}

/// `{"simpleText": "…"}` or `{"runs": [{"text": "…"}, …]}`.
fn text(v: &Value) -> String {
    if let Some(s) = v["simpleText"].as_str() {
        return s.to_string();
    }
    v["runs"]
        .as_array()
        .map(|runs| runs.iter().filter_map(|r| r["text"].as_str()).collect())
        .unwrap_or_default()
}

fn video_renderer(r: &Value) -> Option<Video> {
    let id = r["videoId"].as_str()?.to_string();
    let channel = [&r["ownerText"], &r["shortBylineText"], &r["longBylineText"]]
        .into_iter()
        .map(text)
        .find(|t| !t.is_empty())
        .unwrap_or_default();
    Some(Video {
        id,
        title: text(&r["title"]),
        channel,
        duration: parse_duration(&text(&r["lengthText"])),
    })
}

fn lockup(r: &Value) -> Option<Video> {
    if r["contentType"].as_str() != Some("LOCKUP_CONTENT_TYPE_VIDEO") {
        return None;
    }
    let meta = &r["metadata"]["lockupMetadataViewModel"];
    let channel = meta["metadata"]["contentMetadataViewModel"]["metadataRows"]
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row["metadataParts"].as_array()?.first()?["text"]["content"].as_str())
        .unwrap_or("")
        .to_string();
    let mut badges = Vec::new();
    find_badges(&r["contentImage"], &mut badges);
    Some(Video {
        id: r["contentId"].as_str()?.to_string(),
        title: meta["title"]["content"].as_str().unwrap_or("").to_string(),
        channel,
        duration: badges.iter().find_map(|b| parse_duration(b)),
    })
}

fn find_badges(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (key, value) in map {
                match (key.as_str(), value["text"].as_str()) {
                    ("thumbnailBadgeViewModel", Some(t)) => out.push(t.to_string()),
                    _ => find_badges(value, out),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|i| find_badges(i, out)),
        _ => {}
    }
}

/// "4:13" or "1:02:03" in seconds.
fn parse_duration(s: &str) -> Option<f64> {
    let s = s.trim();
    if !s.contains(':') {
        return None;
    }
    let mut total = 0u64;
    for part in s.split(':') {
        total = total * 60 + part.trim().parse::<u64>().ok()?;
    }
    Some(total as f64)
}

// ---------------------------------------------------------------------------------- player

/// Why a video doesn't play, in plain words.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    /// Only for signed-in adults.
    Age,
    /// YouTube's bot check wants a sign-in from this network.
    Bot,
    /// Made for kids: apps can't play it this way.
    Kids,
    /// What YouTube said ("This video is private").
    Said(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Age => {
                f.write_str("YouTube only shows this video to signed-in adults, and Utako doesn't sign in to YouTube")
            }
            Refusal::Bot => f.write_str(
                "YouTube wants this network to sign in before it plays videos (its bot check). Try again in \
                 a while",
            ),
            Refusal::Kids => f.write_str("YouTube doesn't let apps play this video: it's made for kids"),
            Refusal::Said(why) => write!(f, "YouTube: {why}"),
        }
    }
}

impl std::error::Error for Refusal {}

/// Whether a player answer lets the video play, and if not, why.
pub fn playability(answer: &Value) -> std::result::Result<(), Refusal> {
    let p = &answer["playabilityStatus"];
    let status = p["status"].as_str().unwrap_or("");
    if status == "OK" {
        return Ok(());
    }
    let error = &p["errorScreen"]["playerErrorMessageRenderer"];
    let reason = [
        p["reason"].as_str().map(str::to_string).unwrap_or_default(),
        text(&error["reason"]),
        text(&error["subreason"]),
    ]
    .into_iter()
    .find(|r| !r.trim().is_empty())
    .unwrap_or_default();
    let lower = reason.to_lowercase();
    if status.starts_with("AGE_")
        || !p["desktopLegacyAgeGateReason"].is_null()
        || ["confirm your age", "age-restricted", "inappropriate"]
            .iter()
            .any(|s| lower.contains(s))
    {
        return Err(Refusal::Age);
    }
    if lower.contains("not a bot") {
        return Err(Refusal::Bot);
    }
    if lower.contains("made for kids") {
        return Err(Refusal::Kids);
    }
    Err(Refusal::Said(match (reason.is_empty(), status) {
        (false, _) => reason,
        (true, "LIVE_STREAM_OFFLINE") => "this live stream hasn't started".into(),
        (true, "") => "no answer about this video".into(),
        (true, s) => format!("this video can't play ({s})"),
    }))
}

/// The streams in a player answer that play as they are. Those that need YouTube's JavaScript
/// (a signature, or an `n` to work out), copy protection, or live pieces are left out.
pub fn parse_formats(answer: &Value) -> Vec<Format> {
    let data = &answer["streamingData"];
    let muxed = data["formats"].as_array().into_iter().flatten();
    let adaptive = data["adaptiveFormats"].as_array().into_iter().flatten();
    muxed.chain(adaptive).filter_map(format).collect()
}

fn format(f: &Value) -> Option<Format> {
    let url = f["url"].as_str()?;
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    let needs_js = query.split('&').any(|kv| kv.starts_with("n=") || kv.starts_with("s="));
    if needs_js || !f["drmFamilies"].is_null() || !f["targetDurationSec"].is_null() {
        return None;
    }
    let mime_type = f["mimeType"].as_str()?;
    let (mime, codecs) = mime_type.split_once(';').unwrap_or((mime_type, ""));
    let codecs: Vec<String> = codecs
        .split_once("codecs=")
        .map(|(_, c)| c.trim_matches(|c: char| c == '"' || c.is_whitespace()))
        .unwrap_or("")
        .split(',')
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();
    let mime = mime.trim().to_string();
    let video = mime.starts_with("video/");
    let audio = mime.starts_with("audio/") || codecs.len() > 1;
    let num = |k: &str| -> u64 {
        f[k].as_u64()
            .or_else(|| f[k].as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(0)
    };
    let track = &f["audioTrack"];
    Some(Format {
        itag: num("itag") as u32,
        url: url.to_string(),
        mime,
        codec: codecs.first().cloned().unwrap_or_default(),
        video,
        audio,
        width: num("width") as u32,
        height: num("height") as u32,
        fps: num("fps") as f64,
        bitrate: num("averageBitrate").max(num("bitrate")),
        size: Some(num("contentLength")).filter(|&n| n > 0),
        drc: f["isDrc"].as_bool().unwrap_or(false),
        original: track.is_null()
            || track["audioIsDefault"].as_bool().unwrap_or(false)
            || track["displayName"].as_str().is_some_and(|n| n.contains("original")),
    })
}

fn is_av1(f: &Format) -> bool {
    f.codec.starts_with("av01")
}

fn is_h264(f: &Format) -> bool {
    f.codec.starts_with("avc1")
}

/// The sound to play: sound only, in the video's own language, the best there is; else a
/// stream with both.
pub fn pick_sound(formats: &[Format]) -> Option<&Format> {
    let sound_only = || formats.iter().filter(|f| f.audio && !f.video && !f.drc);
    let rank = |f: &&Format| (f.bitrate, f.codec == "opus");
    sound_only()
        .filter(|f| f.original)
        .max_by_key(rank)
        .or_else(|| sound_only().max_by_key(rank))
        .or_else(|| formats.iter().filter(|f| f.audio && f.video).max_by_key(rank))
}

/// The picture for the centre panel: up to 720p (the panel is never much bigger), not AV1
/// (older computers can't decode it smoothly) when there's another.
pub fn pick_picture(formats: &[Format]) -> Option<&Format> {
    let pictures: Vec<&Format> = formats.iter().filter(|f| f.video && f.height > 0).collect();
    let usable: Vec<&Format> = if pictures.iter().any(|f| !is_av1(f)) {
        pictures.into_iter().filter(|f| !is_av1(f)).collect()
    } else {
        pictures
    };
    usable
        .iter()
        .filter(|f| f.height <= 720)
        .max_by_key(|f| (f.height, f.fps as u32, !f.audio, is_h264(f), f.bitrate))
        .or_else(|| usable.iter().min_by_key(|f| f.height))
        .copied()
}

/// What a download is made of: a picture and a sound to put together, or one stream with both.
#[derive(Debug, PartialEq)]
pub struct Plan<'a> {
    pub picture: &'a Format,
    pub sound: Option<&'a Format>,
    /// "mp4" or "mkv".
    pub container: &'static str,
}

/// The best picture up to 1080p and the best sound: H.264 with AAC (an MP4 that plays
/// everywhere) when YouTube has them, else whatever is best in an MKV.
pub fn plan_download(formats: &[Format]) -> Option<Plan<'_>> {
    let pictures: Vec<&Format> = formats
        .iter()
        .filter(|f| f.video && !f.audio && f.height > 0 && f.height <= 1080)
        .collect();
    let sounds: Vec<&Format> = formats.iter().filter(|f| f.audio && !f.video && !f.drc).collect();
    let best_sound = |aac: bool| {
        let pick = |own: bool| {
            sounds
                .iter()
                .filter(|f| (!aac || f.codec.starts_with("mp4a")) && (!own || f.original))
                .max_by_key(|f| f.bitrate)
                .copied()
        };
        pick(true).or_else(|| pick(false))
    };
    let h264 = pictures
        .iter()
        .filter(|f| is_h264(f))
        .max_by_key(|f| (f.height, f.fps as u32, f.bitrate));
    if let (Some(picture), Some(sound)) = (h264, best_sound(true)) {
        return Some(Plan {
            picture,
            sound: Some(sound),
            container: "mp4",
        });
    }
    // AV1 only when there's nothing else: older computers and TVs can't play it smoothly.
    let picture = pictures
        .iter()
        .max_by_key(|f| (!is_av1(f), f.height, f.fps as u32, f.bitrate))
        .copied();
    if let (Some(picture), Some(sound)) = (picture, best_sound(false)) {
        return Some(Plan {
            picture,
            sound: Some(sound),
            container: "mkv",
        });
    }
    // Only streams with both (itag 18, 360p MP4).
    formats
        .iter()
        .filter(|f| f.video && f.audio)
        .max_by_key(|f| (f.height, f.bitrate))
        .map(|picture| Plan {
            picture,
            sound: None,
            container: if picture.mime.ends_with("mp4") { "mp4" } else { "mkv" },
        })
}

/// Seconds YouTube makes an app wait before a video plays: the ads it would show first, as
/// yt-dlp works them out (until an ad may be skipped, or its whole length).
pub fn wait_seconds(answer: &Value) -> u64 {
    let mut total = 0.0;
    for ad in answer["adPlacements"].as_array().into_iter().flatten() {
        let r = &ad["adPlacementRenderer"];
        if r["config"]["adPlacementConfig"]["kind"].as_str() == Some("AD_PLACEMENT_KIND_START") {
            total += ad_length(&r["renderer"]["instreamVideoAdRenderer"]);
        }
    }
    total.ceil() as u64
}

fn ad_length(r: &Value) -> f64 {
    if let Some(skip) = r["skipOffsetMilliseconds"].as_f64() {
        return skip / 1000.0;
    }
    r["playerVars"]
        .as_str()
        .and_then(|vars| vars.split('&').find_map(|kv| kv.strip_prefix("length_seconds=")))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0.0)
}

/// "vp09.00.40.08" → "VP9", "avc1.4d401f" → "H.264".
fn codec_name(codec: &str) -> &str {
    match codec.split('.').next().unwrap_or("") {
        "vp09" | "vp9" => "VP9",
        "avc1" | "h264" => "H.264",
        "av01" => "AV1",
        "hev1" | "hvc1" => "HEVC",
        _ => "Video",
    }
}

fn label(f: &Format) -> String {
    match f.height {
        0 => codec_name(&f.codec).to_string(),
        h => format!("{} {h}p", codec_name(&f.codec)),
    }
}

fn link(url: &str) -> Link {
    Link {
        url: url.to_string(),
        user_agent: Some(VISIONOS.user_agent.to_string()),
        headers: Vec::new(),
    }
}

/// The player answer for video `id`. A bot check is tried once more as a new visitor.
async fn player(http: &reqwest::Client, site: &Site, id: &str) -> Result<Value> {
    if !is_video_id(id) {
        bail!("not a YouTube video");
    }
    let body = json!({
        "videoId": id,
        "contentCheckOk": true,
        "racyCheckOk": true,
        "playbackContext": { "contentPlaybackContext": { "html5Preference": "HTML5_PREF_WANTS" } },
    });
    let mut fresh = false;
    loop {
        let who = visitor(http, site, id, fresh).await;
        let answer = call(http, site, &VISIONOS, &who, "player", body.clone()).await?;
        match playability(&answer) {
            Ok(()) => return Ok(answer),
            Err(Refusal::Bot) if !fresh => fresh = true,
            Err(Refusal::Said(why)) => {
                // "Made for kids" shows on the video's watch page rather than in the answer.
                let kids = visit(http, site, id)
                    .await
                    .is_ok_and(|(_, page)| page.contains("made for kids"));
                return Err(if kids { Refusal::Kids } else { Refusal::Said(why) }.into());
            }
            Err(refusal) => return Err(refusal.into()),
        }
    }
}

/// Video `id`'s sound and picture.
pub async fn streams(http: &reqwest::Client, id: &str) -> Result<Streams> {
    streams_at(http, &Site::default(), id).await
}

pub async fn streams_at(http: &reqwest::Client, site: &Site, id: &str) -> Result<Streams> {
    let answer = player(http, site, id).await?;
    let formats = parse_formats(&answer);
    let sound = pick_sound(&formats).ok_or_else(|| anyhow!("YouTube gave no stream of this video Utako can play"))?;
    let picture = pick_picture(&formats).map(|f| VideoLink {
        link: link(&f.url),
        width: f.width,
        height: f.height,
        fps: f.fps,
        label: label(f),
    });
    Ok(Streams {
        sound: link(&sound.url),
        picture,
        wait: wait_seconds(&answer),
    })
}

/// mpv's options for a stream: the User-Agent it was made for.
pub fn mpv_options(link: &Link) -> Vec<(&'static str, String)> {
    link.user_agent
        .iter()
        .map(|agent| ("user-agent", agent.clone()))
        .collect()
}

// -------------------------------------------------------------------------------- download

/// Pieces a stream is fetched in (yt-dlp's size: YouTube slows down bigger requests).
const PIECE: u64 = 10 << 20;

/// Saves video `id` (picture and sound) in `dir`, with `meta`'s title and artist.
pub async fn download(
    http: &reqwest::Client,
    id: &str,
    meta: &Metadata,
    dir: &Path,
    progress: &(dyn Fn(f32) + Send + Sync),
) -> Result<Saved> {
    download_at(http, &Site::default(), id, meta, dir, progress).await
}

pub async fn download_at(
    http: &reqwest::Client,
    site: &Site,
    id: &str,
    meta: &Metadata,
    dir: &Path,
    progress: &(dyn Fn(f32) + Send + Sync),
) -> Result<Saved> {
    let answer = player(http, site, id).await?;
    let wait = wait_seconds(&answer);
    if wait > 0 {
        info!("YouTube makes {id} wait {wait} s");
        tokio::time::sleep(Duration::from_secs(wait)).await;
    }
    let formats = parse_formats(&answer);
    let plan = plan_download(&formats).ok_or_else(|| anyhow!("YouTube gave no stream of this video Utako can save"))?;
    let label = format!("YouTube, {}", label(plan.picture));
    let parts: Vec<&Format> = std::iter::once(plan.picture).chain(plan.sound).collect();
    let total = parts.iter().map(|f| f.size.unwrap_or(0)).sum::<u64>().max(1);
    let mut done = 0u64;
    let mut files = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let file = dir.join(format!("part{i}"));
        let before = done;
        let shown = |n: u64| progress((0.97 * (before + n) as f32 / total as f32).min(0.97));
        let what = if part.video { "its picture" } else { "its sound" };
        done += fetch(http, &part.url, part.size, &file, &shown)
            .await
            .with_context(|| format!("couldn't download {what} from YouTube"))?;
        files.push(file);
    }
    let out = dir.join(format!("video.{}", plan.container));
    join(&files, &out, meta).await?;
    for f in files {
        let _ = tokio::fs::remove_file(f).await;
    }
    progress(1.0);
    Ok(Saved { path: out, label })
}

/// Downloads `url` into `file`, in pieces when its size is known; returns how much came.
async fn fetch(
    http: &reqwest::Client,
    url: &str,
    size: Option<u64>,
    file: &Path,
    progress: &(dyn Fn(u64) + Send + Sync),
) -> Result<u64> {
    use tokio::io::AsyncWriteExt;
    let mut out = tokio::fs::File::create(file).await?;
    let mut have = 0u64;
    loop {
        let mut req = http
            .get(url)
            .header(reqwest::header::USER_AGENT, VISIONOS.user_agent)
            .timeout(Duration::from_secs(600));
        if size.is_some() {
            req = req.header(reqwest::header::RANGE, format!("bytes={have}-{}", have + PIECE - 1));
        }
        let mut resp = req.send().await?.error_for_status()?;
        let mut got = 0u64;
        loop {
            let chunk = tokio::time::timeout(Duration::from_secs(60), resp.chunk())
                .await
                .map_err(|_| anyhow!("YouTube stopped sending"))??;
            let Some(chunk) = chunk else { break };
            out.write_all(&chunk).await?;
            got += chunk.len() as u64;
            progress(have + got);
        }
        have += got;
        match size {
            Some(total) if have < total && got > 0 => continue,
            Some(total) if have < total => bail!("YouTube sent {have} of {total} bytes"),
            _ => break,
        }
    }
    out.flush().await?;
    Ok(have)
}

/// Puts a picture and a sound into one file as they are (nothing is converted), with the
/// song's title and artist.
async fn join(parts: &[PathBuf], out: &Path, meta: &Metadata) -> Result<()> {
    let mut cmd = crate::tools::command(crate::tools::resolve("ffmpeg", "ffmpeg"));
    cmd.args(["-v", "error", "-y"]);
    for p in parts {
        cmd.arg("-i").arg(p);
    }
    cmd.args([
        "-map",
        "0:v:0",
        "-map",
        if parts.len() > 1 { "1:a:0" } else { "0:a:0?" },
    ]);
    cmd.args(["-c", "copy"]);
    if out.extension().is_some_and(|e| e == "mp4") {
        cmd.args(["-movflags", "+faststart"]);
    }
    for (key, value) in [("title", &meta.title), ("artist", &meta.artist), ("comment", &meta.url)] {
        if !value.trim().is_empty() {
            cmd.arg("-metadata").arg(format!("{key}={}", value.trim()));
        }
    }
    let output = cmd
        .arg(out)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => anyhow!("saving music videos needs ffmpeg"),
            _ => anyhow!("couldn't run ffmpeg: {e}"),
        })?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        bail!("ffmpeg couldn't put the video together: {}", err.trim());
    }
    Ok(())
}

// -------------------------------------------------------------------------- videos as songs

/// What music videos add to a song's name, cut from the title Utako shows.
const VIDEO_DECORATIONS: &[&str] = &[
    "official music video",
    "official video",
    "official audio",
    "official lyric video",
    "official visualizer",
    "music video",
    "lyric video",
    "lyrics video",
    "lyrics",
    "visualizer",
    "audio",
    "video",
    "hd",
    "4k",
    "mv",
];

/// A search result as a song: "Artist - Title (Official Video)" becomes its artist and title;
/// otherwise the channel is the artist.
pub fn video_track(v: &Video) -> crate::model::Track {
    let (artist, title) = split_video_title(&v.title, &v.channel);
    crate::model::Track {
        id: crate::model::Track::youtube_id(&v.id),
        source: crate::model::Source::YouTube,
        title,
        artist,
        album: String::new(),
        duration_ms: v.duration.map(|d| (d * 1000.0) as u64).unwrap_or(0),
        track_no: None,
        art: Some(format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", v.id)),
        uri: watch_url(&v.id),
        added_at: 0,
        album_artist: String::new(),
        disc_no: None,
    }
}

pub fn watch_url(id: &str) -> String {
    format!("https://www.youtube.com/watch?v={id}")
}

/// The video id of a YouTube track (`youtube:<id>`).
pub fn video_id(track_id: &str) -> Option<&str> {
    track_id.strip_prefix("youtube:").filter(|id| is_video_id(id))
}

/// YouTube's video ids: 11 letters, digits, '-' and '_'.
pub fn is_video_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn split_video_title(title: &str, channel: &str) -> (String, String) {
    let clean = strip_decorations(title);
    let channel = channel
        .trim()
        .trim_end_matches(" - Topic")
        .trim_end_matches("VEVO")
        .trim()
        .to_string();
    for sep in [" - ", " – ", " — ", " | "] {
        if let Some((artist, song)) = clean.split_once(sep) {
            let (artist, song) = (artist.trim(), song.trim());
            if !artist.is_empty() && !song.is_empty() {
                return (artist.to_string(), song.to_string());
            }
        }
    }
    (channel, clean)
}

/// Removes "(Official Video)", "[Lyrics]" and the like (in brackets only, so a song really
/// called "Video" keeps its name).
fn strip_decorations(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut rest = title;
    while let Some(open) = rest.find(['(', '[']) {
        let close_char = if rest.as_bytes()[open] == b'(' { ')' } else { ']' };
        let Some(close) = rest[open..].find(close_char).map(|c| open + c) else {
            break;
        };
        let inner = rest[open + 1..close].to_lowercase();
        let inner = inner.trim();
        let drop = VIDEO_DECORATIONS
            .iter()
            .any(|d| inner == *d || inner.starts_with(&format!("{d} ")))
            || (inner.starts_with("official") && (inner.contains("video") || inner.contains("audio")));
        out.push_str(&rest[..if drop { open } else { close + 1 }]);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests;
