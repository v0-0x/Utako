//! The media server (v0-0x/mediaserver), its music side: finding it, connecting (the server's
//! dashboard lets each device in), its music library as a playlist named after it, with its
//! covers, its songs streamed, settings and logins sent to it or fetched from it when asked,
//! and songs uploaded to it.

use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::hub::client::{self as hubc, Client as HubClient, Server, Uploaded};
use crate::hub::discovery::{self, Found};
use anyhow::Context as _;
use md5::{Digest as _, Md5};

/// What Settings shows about the media server.
#[derive(Debug, Clone, Default)]
pub struct ServerFeed {
    /// Servers heard on the network: (name, address).
    pub found: Vec<(String, String)>,
    pub status: String,
    /// Connected and let in by the server.
    pub allowed: bool,
    /// This device, as the server's dashboard shows it: "Utako on <computer> (<id>)".
    pub device: String,
    /// The server's name, once it has let this device in.
    pub name: String,
    /// The server takes songs from this device (its dashboard's "Let it upload here").
    pub uploads: bool,
    /// The upload under way.
    pub upload: Option<UploadFeed>,
}

/// An upload under way, as Settings shows it.
#[derive(Debug, Clone, Default)]
pub struct UploadFeed {
    /// The file being sent, as it goes on the server.
    pub file: String,
    /// Which file of how many.
    pub file_no: usize,
    pub files: usize,
    /// Bytes there of them all.
    pub sent: u64,
    pub total: u64,
}

/// How an upload went.
#[derive(Debug, Clone, Default)]
pub struct UploadSummary {
    pub server: String,
    pub sent: usize,
    /// The same file was there already.
    pub there: usize,
    /// Not sent: another file has its name there, or it was refused (the first reason).
    pub skipped: usize,
    pub first_error: Option<String>,
}

/// What an upload takes from a folder: songs, and their covers, lyrics and cue sheets.
const UPLOADED: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "opus", "m4a", "m4b", "aac", "alac", "wav", "aif", "aiff", "ape", "wv", "wma", "dsf",
    "mka", "jpg", "jpeg", "png", "webp", "lrc", "cue",
];

/// The songs a picker offers to upload.
pub const SONG_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "opus", "m4a", "m4b", "aac", "alac", "wav", "aif", "aiff", "ape", "wv", "wma", "dsf",
    "mka",
];

/// How often the server is asked what changed.
const SERVER_CHECK: Duration = Duration::from_secs(120);
/// The server first connected to: another one at the same address is refused.
const PIN_KEY: &str = "server:pin";

/// What a connection attempt found out.
pub(super) struct ServerState {
    pub server: Option<Server>,
    pub allowed: bool,
    pub library: Option<hubc::Library>,
    /// The library's version as the hello gave it (asked for again only when it changes).
    pub files: String,
    /// It takes songs from this device.
    pub uploads: bool,
    pub error: Option<String>,
}

/// "Utako on <this computer's name>".
pub(super) fn device_name() -> String {
    let host = std::env::var("COMPUTERNAME").ok().or_else(|| {
        #[cfg(unix)]
        {
            let mut buf = [0u8; 256];
            // SAFETY: the buffer and its length are valid; the name is NUL-terminated or cut.
            let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
            ok.then(|| {
                let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
                String::from_utf8_lossy(&buf[..end])
                    .trim_end_matches(".local")
                    .to_string()
            })
        }
        #[cfg(not(unix))]
        None
    });
    match host.filter(|h| !h.is_empty()) {
        Some(h) => format!("Utako on {h}"),
        None => "Utako".into(),
    }
}

impl Service {
    /// Called every tick: listens for servers and asks the one in use what changed.
    pub(super) fn server_tick(&mut self) {
        if !self.cfg.server.enabled {
            return;
        }
        if !self.server_listening {
            self.server_listening = true;
            let (tx, mut rx) = mpsc::unbounded_channel();
            if let Err(e) = discovery::listen(tx) {
                tracing::debug!("media server discovery: {e}");
            }
            let internal = self.internal_tx.clone();
            tokio::spawn(async move {
                while let Some(found) = rx.recv().await {
                    if internal.send(Internal::ServerFound(found)).is_err() {
                        break;
                    }
                }
            });
        }
        if self.server_checked.is_none_or(|t| t.elapsed() > SERVER_CHECK) {
            self.server_connect();
        }
    }

    pub(super) fn server_found(&mut self, found: Found) {
        let entry = (found.beacon.name.clone(), found.address.clone());
        let mut feed = self.shared.feed.write().unwrap();
        if !feed.server.found.contains(&entry) {
            feed.server.found.push(entry);
            drop(feed);
            self.shared.repaint();
            // Nothing typed: the first one heard is used.
            if self.server.is_none() && self.cfg.server.address.trim().is_empty() {
                self.server_connect();
            }
        }
    }

    /// Connects (again): who answers, whether it lets this device in, and its library.
    pub(super) fn server_connect(&mut self) {
        self.server_checked = Some(Instant::now());
        let typed = self.cfg.server.address.trim().to_string();
        let address = if typed.is_empty() {
            self.shared
                .feed
                .read()
                .unwrap()
                .server
                .found
                .first()
                .map(|(_, a)| a.clone())
        } else {
            Some(typed)
        };
        let Some(address) = address else {
            self.set_server_status("Looking for a media server on your network…", false);
            return;
        };
        let client = self.hub.clone();
        let pin = self.db.get_kv(PIN_KEY).unwrap_or_default();
        let want_library = self.cfg.server.library;
        let known_version = self.server_library_version.clone();
        let tx = self.internal_tx.clone();
        tokio::spawn(async move {
            let state = connect(&client, &address, &pin, want_library, &known_version).await;
            let _ = tx.send(Internal::ServerState(Box::new(state)));
        });
    }

    pub(super) fn server_state(&mut self, state: ServerState) {
        // Turned off while it was connecting.
        if !self.cfg.server.enabled {
            return;
        }
        let name = state.server.as_ref().map(|s| s.name.clone()).unwrap_or_default();
        let status = match (&state.server, state.allowed, &state.error) {
            (_, _, Some(e)) => e.clone(),
            (Some(_), true, None) => format!("Connected to {name}"),
            (Some(_), false, None) => format!(
                "{name} hasn't let this device in yet: allow “{}” in its dashboard",
                self.shared.feed.read().unwrap().server.device
            ),
            (None, _, None) => "Not connected".into(),
        };
        if let Some(server) = &state.server {
            let _ = self.db.set_kv(PIN_KEY, &server.host.id);
        }
        if let (Some(server), Some(lib)) = (&state.server, state.library) {
            if self.cfg.server.library {
                self.server_library = Some(lib);
                self.server_library_version = state.files;
                self.apply_server_library(server);
            }
        }
        self.server = state.server.filter(|_| state.allowed);
        self.set_server_status(&status, self.server.is_some());
        let mut feed = self.shared.feed.write().unwrap();
        feed.server.uploads = state.allowed && state.uploads;
        feed.server.name = self.server.as_ref().map(|s| s.name.clone()).unwrap_or_default();
    }

    fn set_server_status(&self, status: &str, allowed: bool) {
        let mut feed = self.shared.feed.write().unwrap();
        feed.server.status = status.to_string();
        feed.server.allowed = allowed;
        drop(feed);
        self.shared.repaint();
    }

    /// Where a server song's cover is kept: one per album folder (its songs share it).
    fn server_art_file(&self, server: &Server, t: &hubc::ServerTrack) -> (String, PathBuf) {
        self.server_art_at(&server.host.id, &t.path, &t.album)
    }

    fn server_art_at(&self, server_id: &str, path: &str, album: &str) -> (String, PathBuf) {
        let dir = path.rsplit_once(['/', '\\']).map(|(d, _)| d).unwrap_or("");
        let key = format!("{server_id}\0{dir}\0{album}");
        let name: String = Md5::digest(key.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        (name.clone(), self.server_art_cached(&name))
    }

    fn server_art_cached(&self, key: &str) -> PathBuf {
        self.paths.art_cache().join("server").join(format!("{key}.jpg"))
    }

    /// The server's songs (`server_library`), kept as a playlist named after it, with the
    /// covers fetched so far; the others are fetched, and put on them once they're here.
    fn apply_server_library(&mut self, server: &Server) {
        let Some(lib) = self.server_library.take() else { return };
        let mut fetch: Vec<(String, String, PathBuf)> = Vec::new();
        let tracks: Vec<Track> = lib
            .tracks
            .iter()
            .map(|t| {
                let file_name = t.path.rsplit(['/', '\\']).next().unwrap_or(&t.path).to_string();
                let mut art = None;
                if t.has_art {
                    let (key, file) = self.server_art_file(server, t);
                    if file.is_file() {
                        art = Some(file.to_string_lossy().into_owned());
                    } else if self.server_art_asked.insert(key.clone()) {
                        fetch.push((key, t.path.clone(), file));
                    }
                }
                Track {
                    id: Track::server_id(&server.host.id, &t.path),
                    source: Source::Server,
                    title: if t.title.is_empty() { file_name } else { t.title.clone() },
                    artist: t.artist.clone(),
                    album: t.album.clone(),
                    duration_ms: t.duration_ms,
                    track_no: t.track_no,
                    art,
                    uri: t.path.clone(),
                    added_at: t.added_at,
                    album_artist: t.album_artist.clone(),
                    disc_no: t.disc_no,
                }
            })
            .collect();
        self.server_library = Some(lib);
        self.store_server_tracks(&tracks);
        if !fetch.is_empty() {
            self.fetch_server_art(server.clone(), fetch);
        }
        let id = format!("server:{}", server.host.id);
        // A name, description or picture given to it here stays.
        let kept = self
            .shared
            .library
            .read()
            .unwrap()
            .playlist(&id)
            .map(|p| (p.name.clone(), p.description.clone(), p.art.clone()));
        let (name, description, art) = kept.unwrap_or_else(|| {
            (
                format!("On {}", server.name),
                format!("The music on {}", server.name),
                None,
            )
        });
        let playlist = Playlist {
            id,
            name,
            kind: PlaylistKind::Custom,
            remote_id: None,
            description,
            art,
            track_ids: tracks.iter().map(|t| t.id.clone()).collect(),
        };
        self.save_playlists(vec![playlist], &[]);
    }

    /// Keeps the server's songs: the new ones, and the ones that changed there (their tags, a
    /// cover come since).
    fn store_server_tracks(&mut self, tracks: &[Track]) {
        let changed: Vec<Track> = {
            let lib = self.shared.library.read().unwrap();
            tracks
                .iter()
                .filter(|t| lib.tracks.get(&t.id) != Some(*t))
                .cloned()
                .collect()
        };
        if changed.is_empty() {
            return;
        }
        if let Err(e) = self.db.upsert_tracks(&changed, &HashMap::new()) {
            self.shared.error(format!("Database error: {e:#}"));
        }
        let mut lib = self.shared.library.write().unwrap();
        for t in changed {
            lib.tracks.insert(t.id.clone(), t);
        }
    }

    /// Fetches the covers of the server's albums, a few at a time; the songs get them after.
    fn fetch_server_art(&mut self, server: Server, wanted: Vec<(String, String, PathBuf)>) {
        let client = self.hub.clone();
        let tx = self.internal_tx.clone();
        tokio::spawn(async move {
            use futures_util::StreamExt;
            let got: Vec<String> = futures_util::stream::iter(wanted)
                .map(|(key, path, file)| {
                    let (client, server) = (client.clone(), server.clone());
                    async move {
                        match client.art(&server, &path).await {
                            Ok(Some(bytes)) if !bytes.is_empty() => {
                                if let Some(dir) = file.parent() {
                                    let _ = tokio::fs::create_dir_all(dir).await;
                                }
                                // Whole or not at all: the art cache may read it any time.
                                let part = file.with_extension("part");
                                let ok = tokio::fs::write(&part, &bytes).await.is_ok()
                                    && tokio::fs::rename(&part, &file).await.is_ok();
                                ok.then_some(key)
                            }
                            Ok(_) => None,
                            Err(e) => {
                                tracing::debug!("cover of {path} on the media server: {e:#}");
                                None
                            }
                        }
                    }
                })
                .buffer_unordered(4)
                .filter_map(|k| async move { k })
                .collect()
                .await;
            let _ = tx.send(Internal::ServerArt(got));
        });
    }

    /// Covers fetched: the songs get them. A cover fetched again (it changed on the server)
    /// replaces the one the window may still show.
    pub(super) fn server_art(&mut self, got: Vec<String>) {
        if got.is_empty() {
            return;
        }
        {
            let files: Vec<String> = got
                .iter()
                .map(|key| self.server_art_cached(key).to_string_lossy().into_owned())
                .collect();
            let mut feed = self.shared.feed.write().unwrap();
            feed.art_changed.0 += 1;
            feed.art_changed.1 = files;
        }
        if let Some(server) = self.server.clone() {
            self.apply_server_library(&server);
            self.shared.repaint();
        }
    }

    // ---------------------------------------------------------------- song details

    /// Where copies of the server's songs are kept while their details are edited.
    pub(super) fn tagging_dir(&self) -> PathBuf {
        self.paths.cache_dir.join("tagging")
    }

    /// The connected server, when `tracks` are all its songs.
    fn server_of(&self, tracks: &[Track]) -> std::result::Result<Server, String> {
        let server = self.server.clone().ok_or("The media server isn't connected")?;
        let prefix = format!("server:{}:", server.host.id);
        if tracks.iter().any(|t| !t.id.starts_with(&prefix)) {
            return Err("The media server these songs are on isn't connected".into());
        }
        Ok(server)
    }

    /// Reads the details of the server's songs `tracks`, from fresh copies of them.
    pub(super) fn read_server_tags(&mut self, request: u64, tracks: Vec<Track>) {
        let tx = self.internal_tx.clone();
        let server = match self.server_of(&tracks) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(Internal::ServerTagsRead {
                    request,
                    result: Err(e),
                });
                return;
            }
        };
        let (client, dir) = (self.hub.clone(), self.tagging_dir());
        tokio::spawn(async move {
            let result = async {
                let mut out = Vec::with_capacity(tracks.len());
                for t in &tracks {
                    let copy = fetch_copy(&client, &server, &dir, t, true).await?;
                    out.push(tokio::task::spawn_blocking(move || library::tags::read(&copy)).await??);
                }
                anyhow::Ok(out)
            }
            .await
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Internal::ServerTagsRead { request, result });
        });
    }

    /// Changes the details of the server's songs `tracks`: in a copy of each (the one read for
    /// the editor, or a new one), sent back in the song's place.
    pub(super) fn edit_server_tags(
        &mut self,
        tracks: Vec<Track>,
        changes: Vec<(library::tags::Field, String)>,
        cover: Option<CoverChange>,
    ) {
        use library::tags::CoverEdit;
        let server = match self.server_of(&tracks) {
            Ok(s) => s,
            Err(e) => {
                self.shared.error(e);
                return;
            }
        };
        let (client, dir, http) = (self.hub.clone(), self.tagging_dir(), self.http.clone());
        let (tx, shared) = (self.internal_tx.clone(), self.shared.clone());
        shared.info(format!(
            "Saving the details of {} on {}…",
            songs_text(tracks.len()),
            server.name
        ));
        tokio::spawn(async move {
            let cover = match cover {
                Some(CoverChange::Remove) => Some(CoverEdit::Remove),
                Some(CoverChange::From(src)) => match load_cover(&http, &src).await {
                    Ok(data) => Some(CoverEdit::Set(data)),
                    Err(e) => {
                        shared.error(format!("Couldn't use that cover: {e:#}"));
                        return;
                    }
                },
                None => None,
            };
            let cover_changed = cover.is_some();
            let cover = std::sync::Arc::new(cover);
            let changes = std::sync::Arc::new(changes);
            let folders = match client.upload_target(&server).await {
                Ok(target) => target.folders,
                Err(e) => {
                    shared.error(format!("Couldn't change songs on {}: {e:#}", server.name));
                    return;
                }
            };
            let mut saved = Vec::new();
            let mut errors = Vec::new();
            for t in tracks {
                let done = async {
                    let (folder, rel) = hubc::place_in(&folders, &t.uri)
                        .ok_or_else(|| anyhow!("it isn't in one of {}'s music folders", server.name))?;
                    let copy = fetch_copy(&client, &server, &dir, &t, false).await?;
                    let (file, ch, cv) = (copy.clone(), changes.clone(), cover.clone());
                    let read = tokio::task::spawn_blocking(move || {
                        library::tags::edit(&file, &ch, cv.as_ref().as_ref())?;
                        anyhow::Ok(library::scanner::read_track(&file, None, 0))
                    })
                    .await??;
                    client
                        .replace_file(&server, folder, &rel, &copy, |_| {})
                        .await
                        .with_context(|| format!("couldn't send it back to {}", server.name))?;
                    let _ = tokio::fs::remove_file(&copy).await;
                    anyhow::Ok(read)
                }
                .await;
                match done {
                    Ok(read) => saved.push(Track {
                        title: read.title,
                        artist: read.artist,
                        album: read.album,
                        album_artist: read.album_artist,
                        track_no: read.track_no,
                        disc_no: read.disc_no,
                        art: if cover_changed { None } else { t.art.clone() },
                        ..t
                    }),
                    Err(e) => errors.push(format!("“{}”: {e:#}", t.title)),
                }
            }
            let _ = tx.send(Internal::ServerTagsSaved {
                saved,
                errors,
                cover_changed,
            });
        });
    }

    /// Songs changed on the server: shown with their new details at once (the server's
    /// library says the same a moment later), a new cover fetched again.
    pub(super) fn server_tags_saved(&mut self, saved: Vec<Track>, errors: Vec<String>, cover_changed: bool) {
        let name = self.server.as_ref().map(|s| s.name.clone()).unwrap_or_default();
        if cover_changed {
            if let Some(server) = self.server.clone() {
                // Its album's cover, kept under the album's old name or its new one.
                let mut albums: Vec<(String, String)> =
                    saved.iter().map(|t| (t.uri.clone(), t.album.clone())).collect();
                {
                    let lib = self.shared.library.read().unwrap();
                    albums.extend(
                        saved
                            .iter()
                            .filter_map(|t| lib.tracks.get(&t.id).map(|o| (o.uri.clone(), o.album.clone()))),
                    );
                }
                for (path, album) in albums {
                    let (key, file) = self.server_art_at(&server.host.id, &path, &album);
                    let _ = std::fs::remove_file(file);
                    self.server_art_asked.remove(&key);
                }
            }
        }
        if !saved.is_empty() {
            let updated: HashMap<String, Track> = saved.iter().map(|t| (t.id.clone(), t.clone())).collect();
            if let Err(e) = self.db.upsert_tracks(&saved, &HashMap::new()) {
                self.shared.error(format!("Database error: {e:#}"));
            }
            {
                let mut lib = self.shared.library.write().unwrap();
                for t in saved.iter() {
                    lib.tracks.insert(t.id.clone(), t.clone());
                }
                lib.reindex();
            }
            self.queue.refresh(&updated);
            if let Some(new) = self.playing.as_ref().and_then(|p| updated.get(&p.id)) {
                self.playing = Some(new.clone());
            }
            self.publish_player();
            self.publish_queue();
            self.shared
                .info(format!("Saved the details of {} on {name}", songs_text(saved.len())));
        }
        if let Some(first) = errors.first() {
            let more = match errors.len() {
                1 => String::new(),
                n => format!(" (and {} more)", n - 1),
            };
            self.shared.error(format!("Couldn't save {first}{more}"));
        }
        self.server_library_soon();
    }

    /// The media server downloads go to: when that's on in Settings, it's connected and it lets
    /// this device upload.
    pub(super) fn download_server(&self) -> Option<Server> {
        if !(self.cfg.downloads.to_server && self.cfg.server.enabled) {
            return None;
        }
        let uploads = self.shared.feed.read().unwrap().server.uploads;
        self.server.clone().filter(|_| uploads)
    }

    /// Asks the server what changed in a few seconds: it reads songs sent to it a moment after
    /// they arrive (at once if this computer started less than 2 minutes ago).
    pub(super) fn server_library_soon(&mut self) {
        self.server_checked = Instant::now().checked_sub(SERVER_CHECK - Duration::from_secs(6));
    }

    /// Uploads `path` (a song, or a folder: it keeps its name and what's in it) into the
    /// server's library.
    pub(super) fn upload_to_server(&mut self, path: PathBuf) {
        let Some(server) = self.server.clone() else {
            self.shared.error("Connect to the media server first");
            return;
        };
        if self.server_upload_stop.is_some() {
            self.shared.error("An upload is under way already");
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        self.server_upload_stop = Some(stop.clone());
        self.shared.feed.write().unwrap().server.upload = Some(UploadFeed {
            file: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            ..Default::default()
        });
        self.shared.repaint();
        let client = self.hub.clone();
        let tx = self.internal_tx.clone();
        tokio::spawn(async move {
            let result = upload(&client, &server, &path, &stop, &tx).await;
            let _ = tx.send(Internal::ServerUploadDone(result));
        });
    }

    pub(super) fn stop_server_upload(&mut self) {
        if let Some(stop) = &self.server_upload_stop {
            stop.store(true, Ordering::Relaxed);
        }
    }

    pub(super) fn server_upload_progress(&mut self, feed: UploadFeed) {
        if self.server_upload_stop.is_some() {
            self.shared.feed.write().unwrap().server.upload = Some(feed);
            self.shared.repaint();
        }
    }

    pub(super) fn server_upload_done(&mut self, result: Result<UploadSummary>) {
        let stopped = self
            .server_upload_stop
            .take()
            .is_some_and(|stop| stop.load(Ordering::Relaxed));
        self.shared.feed.write().unwrap().server.upload = None;
        match result {
            Ok(s) => {
                let mut text = match (s.sent, s.there) {
                    (0, 0) => "Nothing was uploaded".to_string(),
                    (n, 0) => format!("Uploaded {n} {} to {}", files(n), s.server),
                    (0, t) => format!("{t} {} already on {}", files(t), s.server),
                    (n, t) => format!("Uploaded {n} {} to {} ({t} already there)", files(n), s.server),
                };
                if s.skipped > 0 {
                    text.push_str(&format!(
                        "; {} not sent: {}",
                        s.skipped,
                        s.first_error
                            .unwrap_or_else(|| "another file has its name there".into())
                    ));
                    self.shared.error(text);
                } else {
                    self.shared.info(text);
                }
                // The server reads them in a moment: then its playlist has them.
                self.server_library_soon();
            }
            Err(_) if stopped => self
                .shared
                .info("Upload stopped: sending the same again carries on where it stopped"),
            Err(e) => self.shared.error(format!("Upload: {e:#}")),
        }
        self.shared.repaint();
    }

    /// Removes the server's playlist (its songs stay known, for the next time).
    pub(super) fn drop_server_playlists(&mut self) {
        let ids: Vec<String> = {
            let lib = self.shared.library.read().unwrap();
            lib.playlists
                .iter()
                .filter(|p| p.id.starts_with("server:"))
                .map(|p| p.id.clone())
                .collect()
        };
        if !ids.is_empty() {
            self.save_playlists(vec![], &ids);
        }
    }

    /// Plays a song from the server: mpv streams it with a signed ticket.
    pub(super) async fn start_server_track(&mut self, track: Track) {
        let Some(server) = self
            .server
            .clone()
            .filter(|s| track.id.starts_with(&format!("server:{}:", s.host.id)))
        else {
            self.load_failed("The media server this song is on isn't connected".into())
                .await;
            return;
        };
        match server.file(&self.hub, &track.uri) {
            Ok((url, headers)) => {
                let fields = headers
                    .iter()
                    .map(|(name, value)| format!("{name}: {value}"))
                    .collect::<Vec<_>>()
                    .join(",");
                self.start_mpv_with(track, &url, 0.0, &[("http-header-fields", fields)])
                    .await;
            }
            Err(e) => self.load_failed(format!("Media server: {e:#}")).await,
        }
    }

    /// Gives the server this device's settings and logins (as a settings file, sealed for it).
    pub(super) fn send_settings_to_server(&mut self) {
        let Some(server) = self.server.clone() else {
            self.shared.error("Connect to the media server first");
            return;
        };
        let file = {
            let lib = self.shared.library.read().unwrap();
            crate::backup::build(
                &self.cfg,
                &self.paths.spotify_dir(),
                &lib,
                crate::backup::Include {
                    keys: true,
                    playlists: true,
                },
            )
        };
        let Ok(bytes) = serde_json::to_vec(&file) else { return };
        let client = self.hub.clone();
        let shared = self.shared.clone();
        tokio::spawn(async move {
            match client.send_settings(&server, &bytes).await {
                Ok(()) => shared.info(format!("Your settings and logins are on {}", server.name)),
                Err(e) => shared.error(format!("Couldn't send your settings: {e:#}")),
            }
        });
    }

    /// Takes the settings and logins the server keeps (sent from another device).
    pub(super) fn fetch_settings_from_server(&mut self) {
        let Some(server) = self.server.clone() else {
            self.shared.error("Connect to the media server first");
            return;
        };
        let client = self.hub.clone();
        let tx = self.internal_tx.clone();
        tokio::spawn(async move {
            let result = client.fetch_settings(&server).await;
            let _ = tx.send(Internal::ServerSettings(result));
        });
    }

    pub(super) fn server_settings(&mut self, result: Result<Vec<u8>>) {
        let file = result.and_then(|bytes| {
            serde_json::from_slice::<crate::backup::SettingsFile>(&bytes).context("not a settings file")
        });
        match file {
            Ok(file) => self.import_settings_file(file),
            Err(e) => self
                .shared
                .error(format!("Couldn't get the settings from the server: {e:#}")),
        }
    }
}

fn files(n: usize) -> &'static str {
    if n == 1 {
        "file"
    } else {
        "files"
    }
}

/// The files an upload of `root` sends, with their place on the server ("Album/01 Song.flac")
/// and size: a song alone, or a folder's songs (and covers, lyrics, cue sheets), keeping the
/// folder's name. Hidden files and folders stay behind.
fn files_to_upload(root: &Path) -> Result<Vec<(PathBuf, String, u64)>> {
    let meta = std::fs::metadata(root).with_context(|| format!("couldn't read {}", root.display()))?;
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| anyhow!("{} has no name", root.display()))?;
    if meta.is_file() {
        return Ok(vec![(root.to_path_buf(), name, meta.len())]);
    }
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'))
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let ext = entry
            .path()
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if !UPLOADED.contains(&ext.as_str()) {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        let rel: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        out.push((entry.path().to_path_buf(), format!("{name}/{}", rel.join("/")), size));
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(out)
}

/// Sends the files of `root` to the server, one after the other; one refused doesn't stop
/// the others.
async fn upload(
    client: &HubClient,
    server: &Server,
    root: &Path,
    stop: &AtomicBool,
    tx: &UnboundedSender<Internal>,
) -> Result<UploadSummary> {
    let target = client.upload_target(server).await?;
    if target.folders.is_empty() {
        anyhow::bail!(
            "{} has no folder for music yet: add one in its dashboard (Music › Library)",
            server.name
        );
    }
    let root_owned = root.to_path_buf();
    let list = tokio::task::spawn_blocking(move || files_to_upload(&root_owned)).await??;
    if list.is_empty() {
        anyhow::bail!("there are no songs in {}", root.display());
    }
    let total: u64 = list.iter().map(|f| f.2).sum();
    let mut summary = UploadSummary {
        server: server.name.clone(),
        ..Default::default()
    };
    let mut before = 0u64;
    for (i, (path, rel, size)) in list.iter().enumerate() {
        if stop.load(Ordering::Relaxed) {
            anyhow::bail!("stopped");
        }
        let progress = |have: u64| {
            let _ = tx.send(Internal::ServerUpload(UploadFeed {
                file: rel.clone(),
                file_no: i + 1,
                files: list.len(),
                sent: before + have.min(*size),
                total,
            }));
        };
        match client
            .upload_file(
                server,
                0,
                rel,
                path,
                target.chunk.min(hubc::UPLOAD_PIECE),
                stop,
                progress,
            )
            .await
        {
            Ok(Uploaded::Sent) => summary.sent += 1,
            Ok(Uploaded::AlreadyThere) => summary.there += 1,
            Ok(Uploaded::Different) => {
                summary.skipped += 1;
                summary
                    .first_error
                    .get_or_insert_with(|| format!("{rel} is there already, and isn't the same file"));
            }
            Err(e) if stop.load(Ordering::Relaxed) => return Err(e),
            Err(e) => {
                summary.skipped += 1;
                summary.first_error.get_or_insert_with(|| format!("{rel}: {e:#}"));
            }
        }
        before += size;
    }
    Ok(summary)
}

async fn connect(client: &HubClient, address: &str, pin: &str, want_library: bool, known_version: &str) -> ServerState {
    let mut state = ServerState {
        server: None,
        allowed: false,
        library: None,
        files: String::new(),
        uploads: false,
        error: None,
    };
    let server = match client.whoami(&hubc::base_url(address)).await {
        Ok(s) => s,
        Err(e) => {
            // The reason in Settings; the details in the log.
            tracing::info!("media server at {address}: {e:#}");
            state.error = Some(e.to_string());
            return state;
        }
    };
    if !pin.is_empty() && pin != server.host.id {
        state.error = Some(format!(
            "A different server answers at {address} than the one Utako connected to before"
        ));
        return state;
    }
    match client.hello(&server).await {
        Ok(hello) => {
            state.allowed = hello.shares;
            state.uploads = hello.uploads;
            state.files = hello.files.clone();
            if hello.shares && want_library && hello.files != known_version {
                match client.library(&server).await {
                    Ok(lib) => state.library = Some(lib),
                    Err(e) => state.error = Some(format!("Couldn't read {}'s music: {e:#}", server.name)),
                }
            }
        }
        Err(e) => state.error = Some(format!("{}: {e:#}", server.name)),
    }
    state.server = Some(server);
    state
}

/// A copy of the server's song `t` in `dir` (fetched again when `fresh`, or when there's none).
async fn fetch_copy(client: &HubClient, server: &Server, dir: &Path, t: &Track, fresh: bool) -> Result<PathBuf> {
    let name: String = Md5::digest(t.id.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let copy = dir.join(format!("{name}.{}", crate::downloader::extension_of(&t.uri)));
    if fresh || !copy.is_file() {
        tokio::fs::create_dir_all(dir).await?;
        let part = dir.join(format!("{name}.part"));
        client
            .download(server, &t.uri, &part, &|_| {})
            .await
            .with_context(|| format!("couldn't fetch “{}” from {}", t.title, server.name))?;
        tokio::fs::rename(&part, &copy).await?;
    }
    Ok(copy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_an_upload_takes() {
        let tmp = tempfile::tempdir().unwrap();
        let album = tmp.path().join("Band - Album");
        for f in [
            "01 One.flac",
            "02 Two.MP3",
            "cover.jpg",
            "notes.txt",
            ".hidden.flac",
            "Disc 2/01 Three.opus",
            "01 One.lrc",
        ] {
            let p = album.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
        }
        std::fs::create_dir_all(album.join(".stuff")).unwrap();
        std::fs::write(album.join(".stuff/a.mp3"), b"x").unwrap();
        let names: Vec<String> = files_to_upload(&album).unwrap().into_iter().map(|f| f.1).collect();
        assert_eq!(
            names,
            [
                "Band - Album/01 One.flac",
                "Band - Album/01 One.lrc",
                "Band - Album/02 Two.MP3",
                "Band - Album/Disc 2/01 Three.opus",
                "Band - Album/cover.jpg",
            ]
        );
        // A song alone goes by its name.
        let one = files_to_upload(&album.join("01 One.flac")).unwrap();
        assert_eq!((one[0].1.as_str(), one[0].2), ("01 One.flac", 1));
    }
}
