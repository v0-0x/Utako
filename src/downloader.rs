//! Saving songs as files. SoundCloud songs are downloaded directly. Spotify songs are saved
//! straight from Spotify through Utako's Spotify connection; while Utako isn't logged in to
//! Spotify, the same recording is found on SoundCloud instead. Music videos are saved as video
//! files (picture and sound) from YouTube. Every file is then tagged with the song's details
//! (from Spotify when it is a Spotify song), its cover and its lyrics, and kept in the download
//! folder or sent to the media server.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use librespot_core::session::Session;
use tracing::{info, warn};

use crate::hub::client::{self as hubc, Uploaded};
use crate::integrations::genius::{self, Match};
use crate::integrations::lyrics::{self, LyricsFetcher};
use crate::library::tags::{self, Metadata};
use crate::model::{Source, Track};
use crate::providers::soundcloud::{self, DownloadKind, SoundCloud};
use crate::providers::youtube;
use crate::providers::{spotify_audio, spotify_internal};

pub type Progress<'a> = &'a (dyn Fn(f32) + Send + Sync);

/// How much of the progress bar the download takes when the song then goes to the media server.
const DOWNLOAD_SHARE: f32 = 0.8;

pub struct Downloader {
    pub soundcloud: Arc<SoundCloud>,
    pub http: reqwest::Client,
    /// Lyrics to embed; `None` leaves them out.
    pub lyrics: Option<Arc<LyricsFetcher>>,
    /// For Spotify songs: their audio and all their details. `None`: not logged in to Spotify.
    pub spotify: Option<Session>,
    /// Logged in to Spotify (`spotify` is `None` only when it couldn't connect).
    pub spotify_logged_in: bool,
    /// Convert Spotify's Ogg Vorbis to MP3 (with ffmpeg).
    pub spotify_mp3: bool,
    /// Send the song to the media server instead of keeping it on this device.
    pub to_server: Option<ToServer>,
    /// The media server connected: where its own songs are downloaded from.
    pub from_server: Option<ToServer>,
}

/// The media server a download goes to.
pub struct ToServer {
    pub client: Arc<hubc::Client>,
    pub server: hubc::Server,
}

/// A saved song.
#[derive(Debug)]
pub struct Saved {
    /// The file, or its place on the media server ("Spotify/Artist - Song.ogg").
    pub path: PathBuf,
    /// Where the audio came from, for the Downloads list.
    pub from: String,
    /// The media server it was sent to.
    pub server: Option<String>,
}

impl Downloader {
    /// Downloads `track` into `dir`. Work happens in `work` (a folder of its own inside
    /// `dir`), which is removed afterwards, so a failed or cancelled download leaves nothing.
    pub async fn download(&self, track: &Track, dir: &Path, work: &Path, progress: Progress<'_>) -> Result<Saved> {
        tokio::fs::create_dir_all(work)
            .await
            .with_context(|| format!("couldn't create {}", dir.display()))?;
        let result = self.download_in(track, dir, work, progress).await;
        let _ = tokio::fs::remove_dir_all(work).await;
        result
    }

    async fn download_in(&self, track: &Track, dir: &Path, work: &Path, progress: Progress<'_>) -> Result<Saved> {
        // Going to the media server, the download is the first part of the bar.
        let share = if self.to_server.is_some() && track.source != Source::Server {
            DOWNLOAD_SHARE
        } else {
            1.0
        };
        let scaled = move |p: f32| progress(p * share);
        let fetching: Progress = &scaled;
        // A media server song's cover: the one already fetched for the library, or the server's.
        let mut server_cover = None;
        let (path, mut meta, from, keep_existing) = match track.source {
            Source::SoundCloud => {
                let saved = self.soundcloud.download(track, work, fetching).await?;
                let original = saved.kind == DownloadKind::Original;
                let from = if original { "original file" } else { "SoundCloud" };
                let meta = upload_details(track, saved.meta).await;
                (saved.path, meta, from.to_string(), original)
            }
            Source::Spotify => {
                let (path, from) = self.spotify_or_found(track, work, fetching).await?;
                (path, self.details(track).await, from, false)
            }
            Source::YouTube => {
                let id = youtube::video_id(&track.id).ok_or_else(|| anyhow!("not a YouTube video"))?;
                let saved = youtube::download(&self.http, id, &basic_metadata(track), work, fetching).await?;
                (saved.path, basic_metadata(track), saved.label, false)
            }
            Source::Local => bail!("this song is already a file on your computer"),
            Source::Server => {
                let on = self
                    .from_server
                    .as_ref()
                    .filter(|s| track.id.starts_with(&format!("server:{}:", s.server.host.id)))
                    .ok_or_else(|| anyhow!("the media server this song is on isn't connected"))?;
                let path = work.join(format!("server.{}", extension_of(&track.uri)));
                on.client
                    .download(&on.server, &track.uri, &path, fetching)
                    .await
                    .with_context(|| format!("couldn't download it from {}", on.server.name))?;
                server_cover = match track.art.as_deref().filter(|a| Path::new(a).is_file()) {
                    Some(art) => tokio::fs::read(art).await.ok(),
                    None => on.client.art(&on.server, &track.uri).await.ok().flatten(),
                };
                // The server's file as it is: its own tags win, only what it lacks is filled in.
                let meta = Metadata {
                    album_artist: track.album_artist.clone(),
                    disc: track.disc_no,
                    cover_urls: Vec::new(),
                    url: String::new(),
                    ..basic_metadata(track)
                };
                (path, meta, on.server.name.clone(), true)
            }
        };
        if meta.album.trim().is_empty() {
            // Released on its own: a single.
            meta.album = meta.title.clone();
        }
        if let Some(fetcher) = &self.lyrics {
            if let Some(found) = fetcher.fetch(track).await {
                meta.lyrics = lyrics::to_tag_text(&found);
            }
        }
        let cover = match server_cover {
            Some(cover) => Some(cover),
            None => self.cover(&meta.cover_urls).await,
        };
        if make_taggable(&path).await {
            let (file, meta_, cover_) = (path.clone(), meta.clone(), cover);
            let tagged =
                tokio::task::spawn_blocking(move || tags::write(&file, &meta_, cover_.as_deref(), keep_existing)).await;
            match tagged {
                Ok(Ok(())) => {}
                Ok(Err(e)) => warn!("{e:#}"),
                Err(e) => warn!("tagging {} failed: {e}", path.display()),
            }
        }
        // A server song isn't sent back to the server it came from.
        if let Some(to) = self.to_server.as_ref().filter(|_| track.source != Source::Server) {
            let place = self.send(to, track, &path, &meta, progress).await?;
            info!("sent \"{}\" ({from}) to {} as {place}", meta.title, to.server.name);
            return Ok(Saved {
                path: PathBuf::from(place),
                from,
                server: Some(to.server.name.clone()),
            });
        }
        let path = move_into(&path, dir, &soundcloud::file_stem(&meta.artist, &meta.title)).await?;
        info!("saved \"{}\" ({from}) to {}", meta.title, path.display());
        Ok(Saved {
            path,
            from,
            server: None,
        })
    }

    /// Sends the finished file into the media server's first music folder, as
    /// "<service>/Artist - Song.ext", and returns that place.
    async fn send(
        &self,
        to: &ToServer,
        track: &Track,
        file: &Path,
        meta: &Metadata,
        progress: Progress<'_>,
    ) -> Result<String> {
        let target = to
            .client
            .upload_target(&to.server)
            .await
            .with_context(|| format!("couldn't send it to {}", to.server.name))?;
        if target.folders.is_empty() {
            bail!(
                "{} has no folder for music yet: add one in its dashboard (Music › Library)",
                to.server.name
            );
        }
        let place = server_place(track.source, file, meta);
        let total = tokio::fs::metadata(file).await.map(|m| m.len()).unwrap_or(0).max(1);
        let sent =
            |have: u64| progress(DOWNLOAD_SHARE + (1.0 - DOWNLOAD_SHARE) * (have as f32 / total as f32).min(1.0));
        // Cancelling a download stops its task, and this with it.
        let stop = AtomicBool::new(false);
        let chunk = target.chunk.min(hubc::UPLOAD_PIECE);
        let sent = to
            .client
            .upload_file(&to.server, 0, &place, file, chunk, &stop, sent)
            .await
            .with_context(|| format!("couldn't send it to {}", to.server.name))?;
        match sent {
            Uploaded::Sent | Uploaded::AlreadyThere => Ok(place),
            Uploaded::Different => bail!("{} has another file named “{place}” already", to.server.name),
        }
    }

    /// Spotify's own audio, through Utako's Spotify connection. Without one (not logged in to
    /// Spotify), the same recording is found on SoundCloud instead.
    async fn spotify_or_found(&self, track: &Track, dir: &Path, progress: Progress<'_>) -> Result<(PathBuf, String)> {
        let id = track.id.strip_prefix("spotify:track:");
        if let (Some(session), Some(id)) = (&self.spotify, id) {
            let path = spotify_audio::download(session, id, dir, progress).await?;
            return Ok(if self.spotify_mp3 {
                (to_mp3(&path).await, "Spotify".into())
            } else {
                (path, "Spotify".into())
            });
        }
        if track.source == Source::Spotify && self.spotify_logged_in {
            bail!("Utako couldn't connect to Spotify (see Settings › Spotify); try again");
        }
        self.find_and_download(track, dir, progress).await
    }

    /// The song's details: everything Spotify knows for Spotify songs, else what the library
    /// has.
    async fn details(&self, track: &Track) -> Metadata {
        if let (Source::Spotify, Some(session)) = (track.source, &self.spotify) {
            match spotify_internal::track_details(session, &track.id).await {
                Ok(meta) if !meta.title.is_empty() => return meta,
                Ok(_) => {}
                Err(e) => warn!("Spotify details of {}: {e:#}", track.id),
            }
        }
        basic_metadata(track)
    }

    /// Finds the same recording on SoundCloud and downloads it.
    async fn find_and_download(&self, track: &Track, dir: &Path, progress: Progress<'_>) -> Result<(PathBuf, String)> {
        let artist = lyrics::first_artist(&track.artist);
        let title = lyrics::clean_title(&track.title);
        let found = self
            .soundcloud
            .search(&format!("{artist} {title}"), 10)
            .await
            .map_err(|e| anyhow!("Not found: SoundCloud: {e:#}"))?;
        let mut problem = "no full-length upload on SoundCloud".to_string();
        for candidate in crate::service::ranked_matches(track, &found).into_iter().take(2) {
            match self.soundcloud.download(&candidate, dir, progress).await {
                Ok(saved) => return Ok((saved.path, "SoundCloud".into())),
                Err(e) => problem = format!("SoundCloud: {e:#}"),
            }
        }
        bail!("Not found: {problem}")
    }

    /// The first cover that downloads.
    async fn cover(&self, urls: &[String]) -> Option<Vec<u8>> {
        for url in urls.iter().filter(|u| u.starts_with("http")) {
            if let Some(bytes) = fetch_image(&self.http, url).await {
                return Some(bytes);
            }
        }
        None
    }
}

/// A SoundCloud upload without release details: "Artist - Title [Free DL]" titles cleaned up,
/// then the album and release date from Genius when it has the song by the same artist.
async fn upload_details(track: &Track, mut meta: Metadata) -> Metadata {
    if !meta.album.trim().is_empty() {
        return meta;
    }
    let (artist, title) = crate::integrations::lastfm::scrobble_names(track);
    if !artist.is_empty() && !title.is_empty() {
        meta.artist = artist.clone();
        meta.album_artist = lyrics::first_artist(&artist);
        meta.title = title.clone();
    }
    let found = genius::shared()
        .find(
            &lyrics::first_artist(&artist),
            &lyrics::clean_title(&title),
            Match::SameArtist,
        )
        .await;
    if let Some(song) = found {
        meta.album = song.album;
        if meta.date.is_empty() {
            meta.date = song.release_date;
        }
        // The upload's own artwork stays first; the album cover is the fallback.
        meta.cover_urls.extend(song.art);
    }
    meta
}

/// Details from the library's copy of the song (no Spotify session).
pub fn basic_metadata(track: &Track) -> Metadata {
    let url = match track.source {
        Source::Spotify => track
            .id
            .strip_prefix("spotify:track:")
            .map(|id| format!("https://open.spotify.com/track/{id}"))
            .unwrap_or_default(),
        _ if track.uri.starts_with("http") => track.uri.clone(),
        _ => String::new(),
    };
    Metadata {
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        album_artist: lyrics::first_artist(&track.artist),
        track: track.track_no,
        url,
        cover_urls: track.art.iter().cloned().collect(),
        ..Metadata::default()
    }
}

/// Cover art, if it can be had.
async fn fetch_image(http: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
    crate::http::get_image(http, url).await.ok()
}

/// Spotify's Ogg Vorbis as MP3 (VBR, ~245 kbps): converting can't add quality, but MP3 plays on
/// every device. The Ogg file stays when ffmpeg can't do it.
async fn to_mp3(ogg: &Path) -> PathBuf {
    let mp3 = ogg.with_extension("mp3");
    let status = crate::tools::command(crate::tools::resolve("ffmpeg", "ffmpeg"))
        .args(["-v", "error", "-y", "-i"])
        .arg(ogg)
        .args(["-map", "0:a", "-c:a", "libmp3lame", "-q:a", "0"])
        .arg(&mp3)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .status()
        .await;
    match status {
        Ok(s) if s.success() => {
            let _ = tokio::fs::remove_file(ogg).await;
            mp3
        }
        _ => {
            let _ = tokio::fs::remove_file(&mp3).await;
            warn!(
                "ffmpeg couldn't convert {} to MP3; keeping the Ogg Vorbis file",
                ogg.display()
            );
            ogg.to_path_buf()
        }
    }
}

/// Fragmented MP4 (SoundCloud's AAC streams) can't take tags; ffmpeg rewrites it as a regular
/// MP4 without re-encoding. Returns whether the file can be tagged.
async fn make_taggable(path: &Path) -> bool {
    let is_mp4 = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("m4a"));
    if !is_mp4 || !is_fragmented_mp4(path).await {
        return true;
    }
    let tmp = path.with_extension("remux.m4a");
    let status = crate::tools::command(crate::tools::resolve("ffmpeg", "ffmpeg"))
        .args(["-v", "error", "-y", "-i"])
        .arg(path)
        .args(["-map", "0:a", "-c", "copy", "-movflags", "+faststart"])
        .arg(&tmp)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .status()
        .await;
    match status {
        Ok(s) if s.success() && tokio::fs::rename(&tmp, path).await.is_ok() => true,
        _ => {
            let _ = tokio::fs::remove_file(&tmp).await;
            warn!(
                "{} is fragmented MP4 and ffmpeg couldn't rewrite it; leaving it untagged",
                path.display()
            );
            false
        }
    }
}

/// Whether the top level of an MP4 file has fragments (`moof` boxes).
async fn is_fragmented_mp4(path: &Path) -> bool {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let Ok(mut file) = tokio::fs::File::open(path).await else {
        return false;
    };
    let mut pos = 0u64;
    for _ in 0..64 {
        let mut header = [0u8; 16];
        if file.seek(std::io::SeekFrom::Start(pos)).await.is_err() || file.read_exact(&mut header[..8]).await.is_err() {
            return false;
        }
        let kind = &header[4..8];
        if kind == b"moof" {
            return true;
        }
        let mut size = u32::from_be_bytes(header[..4].try_into().unwrap()) as u64;
        if size == 1 {
            if file.read_exact(&mut header[8..16]).await.is_err() {
                return false;
            }
            size = u64::from_be_bytes(header[8..16].try_into().unwrap());
        }
        if size < 8 {
            return false;
        }
        pos += size;
    }
    false
}

/// Moves a finished download from its work folder into `dir` as "Artist - Title.ext".
/// A file's extension from its path on the server ("mp3" when it has none).
pub(crate) fn extension_of(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty()
                && !ext.is_empty()
                && ext.len() <= 5
                && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            ext.to_ascii_lowercase()
        }
        _ => "mp3".into(),
    }
}

async fn move_into(path: &Path, dir: &Path, stem: &str) -> Result<PathBuf> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_else(|| "mp3".into());
    let (path, dir2, stem2) = (path.to_path_buf(), dir.to_path_buf(), stem.to_string());
    tokio::task::spawn_blocking(move || soundcloud::save_unique(&path, &dir2, &stem2, &ext))
        .await?
        .with_context(|| format!("couldn't save it in {}", dir.display()))
}

/// Where a download goes on the media server: "<service>/Artist - Song.ext", in its first
/// music folder (as the download folder here has a folder for each service).
fn server_place(source: Source, file: &Path, meta: &Metadata) -> String {
    let ext = file
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "mp3".into());
    format!(
        "{}/{}.{ext}",
        source.folder_name(),
        soundcloud::file_stem(&meta.artist, &meta.title)
    )
}

/// Name of the work folder of download number `n`. Anything named like this in a download
/// folder is a leftover (the scanner skips it, and it is cleared on start).
pub fn work_dir_name(n: u64) -> String {
    format!("{WORK_PREFIX}{}-{n}", std::process::id())
}

pub const WORK_PREFIX: &str = ".multimusic-";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_file_extensions() {
        assert_eq!(extension_of("/music/Artist/01 Song.FLAC"), "flac");
        assert_eq!(extension_of("C:\\Music\\a.m4a"), "m4a");
        // A dot in a folder, not the file; a name that's only a dot; nonsense: mp3.
        assert_eq!(extension_of("/music/Mr. Big/Song"), "mp3");
        assert_eq!(extension_of("/music/.hidden"), "mp3");
        assert_eq!(extension_of("/music/a.b c"), "mp3");
    }

    #[test]
    fn basic_details_from_the_library() {
        let t = Track {
            album_artist: String::new(),
            disc_no: None,
            id: "spotify:track:abc".into(),
            source: Source::Spotify,
            title: "Song".into(),
            artist: "A, B".into(),
            album: "Album".into(),
            duration_ms: 1000,
            track_no: Some(4),
            art: Some("https://i.scdn.co/image/x".into()),
            uri: "spotify:track:abc".into(),
            added_at: 0,
        };
        let m = basic_metadata(&t);
        assert_eq!(m.url, "https://open.spotify.com/track/abc");
        assert_eq!((m.album_artist.as_str(), m.track), ("A", Some(4)));
        assert_eq!(m.cover_urls, vec!["https://i.scdn.co/image/x".to_string()]);
    }

    #[test]
    fn place_on_the_server() {
        let meta = Metadata {
            title: "Song / Part 2".into(),
            artist: "Artist".into(),
            ..Metadata::default()
        };
        assert_eq!(
            server_place(Source::Spotify, Path::new("/w/abc.OGG"), &meta),
            "Spotify/Artist - Song _ Part 2.ogg"
        );
        assert_eq!(
            server_place(Source::YouTube, Path::new("/w/x.mp4"), &meta),
            "Music videos/Artist - Song _ Part 2.mp4"
        );
    }

    #[tokio::test]
    async fn fragmented_mp4_is_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let mp4 = |boxes: &[(&[u8; 4], u32)]| {
            let mut out = Vec::new();
            for (kind, size) in boxes {
                out.extend_from_slice(&size.to_be_bytes());
                out.extend_from_slice(&kind[..]);
                out.resize(out.len() + *size as usize - 8, 0);
            }
            out
        };
        let plain = dir.join("plain.m4a");
        std::fs::write(&plain, mp4(&[(b"ftyp", 16), (b"moov", 24), (b"mdat", 40)])).unwrap();
        assert!(!is_fragmented_mp4(&plain).await);
        let fragmented = dir.join("frag.m4a");
        std::fs::write(
            &fragmented,
            mp4(&[(b"ftyp", 16), (b"moov", 24), (b"moof", 16), (b"mdat", 40)]),
        )
        .unwrap();
        assert!(is_fragmented_mp4(&fragmented).await);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A real fragmented AAC file (like SoundCloud's HLS streams) is rewritten and then tagged.
    /// Needs ffmpeg, which mpv depends on.
    #[tokio::test]
    async fn fragmented_aac_gets_tags() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("song.m4a");
        let made = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=44100:cl=mono",
                "-t",
                "1",
            ])
            .args(["-c:a", "aac", "-movflags", "frag_keyframe+empty_moov"])
            .arg(&path)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("ffmpeg not available, skipping");
            return;
        }
        assert!(is_fragmented_mp4(&path).await);
        assert!(make_taggable(&path).await);
        assert!(!is_fragmented_mp4(&path).await);
        let meta = Metadata {
            title: "Song".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            ..Metadata::default()
        };
        tags::write(&path, &meta, None, false).unwrap();
        let track = crate::library::scanner::read_track(&path, None, 0);
        assert_eq!((track.title.as_str(), track.album.as_str()), ("Song", "Album"));
        assert!(track.duration_ms > 900, "{}", track.duration_ms);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
