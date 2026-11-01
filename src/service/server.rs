//! The media server (v0-0x/mediaserver), once it has a music side: finding it, connecting
//! (the server's dashboard lets each device in), its music library as a playlist named after
//! it, its songs streamed, and settings and logins sent to it or fetched from it when asked.

use super::*;
use crate::hub::client::{self as hubc, Client as HubClient, Server};
use crate::hub::discovery::{self, Found};
use anyhow::Context as _;

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
}

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
                self.apply_server_library(server, lib);
                self.server_library_version = state.files;
            }
        }
        self.server = state.server.filter(|_| state.allowed);
        self.set_server_status(&status, self.server.is_some());
    }

    fn set_server_status(&self, status: &str, allowed: bool) {
        let mut feed = self.shared.feed.write().unwrap();
        feed.server.status = status.to_string();
        feed.server.allowed = allowed;
        drop(feed);
        self.shared.repaint();
    }

    /// The server's songs, kept as a playlist named after it.
    fn apply_server_library(&mut self, server: &Server, lib: hubc::Library) {
        let tracks: Vec<Track> = lib
            .tracks
            .iter()
            .map(|t| {
                let file_name = t.path.rsplit(['/', '\\']).next().unwrap_or(&t.path).to_string();
                Track {
                    id: Track::server_id(&server.host.id, &t.path),
                    source: Source::Server,
                    title: if t.title.is_empty() { file_name } else { t.title.clone() },
                    artist: t.artist.clone(),
                    album: t.album.clone(),
                    duration_ms: t.duration_ms,
                    track_no: t.track_no,
                    art: None,
                    uri: t.path.clone(),
                    added_at: t.added_at,
                    album_artist: t.album_artist.clone(),
                    disc_no: t.disc_no,
                }
            })
            .collect();
        self.store_tracks(&tracks);
        let playlist = Playlist {
            id: format!("server:{}", server.host.id),
            name: format!("On {}", server.name),
            kind: PlaylistKind::Custom,
            remote_id: None,
            description: format!("The music on {}", server.name),
            art: None,
            track_ids: tracks.iter().map(|t| t.id.clone()).collect(),
        };
        self.save_playlists(vec![playlist], &[]);
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

async fn connect(client: &HubClient, address: &str, pin: &str, want_library: bool, known_version: &str) -> ServerState {
    let mut state = ServerState {
        server: None,
        allowed: false,
        library: None,
        files: String::new(),
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
