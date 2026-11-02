//! Talking to a media server: signed requests and checked answers (identity.rs), and the few
//! things Utako asks of it. What a music side answers is in docs/media-server.md.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::identity::{query_escape, Host, Identity, HEADER_REPLY};

/// The music side's port (the anime side is 43211, movies & TV 43311).
pub const PORT: u16 = 43411;

/// Who answers at an address (unsigned: for finding out which server is there).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Whoami {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub key: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub role: String,
}

/// The server's answer to a signed hello: whether it lets this device in, and what changed.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Hello {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    /// It shares its music with this device (allowed in its dashboard).
    #[serde(default)]
    pub shares: bool,
    /// Changes when its library does.
    #[serde(default)]
    pub files: String,
    /// It takes songs from this device too (its dashboard's "Let it upload here").
    #[serde(default)]
    pub uploads: bool,
}

/// A song in the server's library.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServerTrack {
    /// Its path on the server: what `/api/peer/local/file?path=` streams.
    pub path: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub album_artist: String,
    #[serde(default)]
    pub track_no: Option<u32>,
    #[serde(default)]
    pub disc_no: Option<u32>,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub added_at: i64,
    /// It has a cover (`/api/peer/music/art?path=`).
    #[serde(default)]
    pub has_art: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Library {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub tracks: Vec<ServerTrack>,
}

/// Settings and logins, sealed (for the server when sent, for this device when fetched).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sealed {
    pub sealed: String,
    #[serde(default)]
    pub updated_at: i64,
}

/// Where uploads go on the server: its library folders, and the most one piece may hold.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UploadTarget {
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub chunk: u64,
}

/// Where a file being uploaded stands on the server.
#[derive(Debug, Clone, Default, Deserialize)]
struct UploadAnswer {
    #[serde(default)]
    have: u64,
    #[serde(default)]
    done: bool,
}

/// What became of a file sent to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Uploaded {
    Sent,
    /// The same file (by its size) was there already.
    AlreadyThere,
    /// Another file has its name there: it isn't sent over it.
    Different,
}

/// The most a piece of an upload holds (the server's limit too).
pub const MAX_PIECE: u64 = 8 << 20;

/// The pieces Utako sends: smaller than the server takes, so a progress bar moves.
pub const UPLOAD_PIECE: u64 = 2 << 20;

/// A server found or typed: where it is and who it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub host: Host,
    pub name: String,
    /// `http://<address>:<port>`
    pub base: String,
}

impl Server {
    /// The link mpv streams `path` from, and the headers that let it (a "ticket": the same
    /// request may be repeated for hours, as mpv does when it seeks).
    pub fn file(&self, client: &Client, path: &str) -> Result<(String, Vec<(&'static str, String)>)> {
        let uri = format!("/api/peer/local/file?path={}", query_escape(path));
        let signed = client
            .identity
            .sign(&self.host, "GET", &uri, b"", &client.name, 0, 0, now(), true)?;
        Ok((format!("{}{uri}", self.base), signed.headers))
    }
}

/// This device, as the servers know it.
pub struct Client {
    pub identity: Arc<Identity>,
    /// What the server's dashboard shows ("Utako on <computer>").
    pub name: String,
    http: reqwest::Client,
}

fn now() -> i64 {
    crate::model::now_unix()
}

/// `http://pi.local` or `pi.local:43411` or `192.168.1.5`: the address with its port.
pub fn base_url(address: &str) -> String {
    let a = address.trim().trim_end_matches('/');
    let a = a.strip_prefix("http://").unwrap_or(a);
    let has_port = a.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok());
    if has_port {
        format!("http://{a}")
    } else {
        format!("http://{a}:{PORT}")
    }
}

impl Client {
    pub fn new(identity: Arc<Identity>, name: String) -> Client {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(5))
            // The server is on the home network: never through a proxy.
            .no_proxy()
            .build()
            .expect("a plain client builds");
        Client { identity, name, http }
    }

    /// Who is at `base`. The answer's key is checked against its ID.
    pub async fn whoami(&self, base: &str) -> Result<Server> {
        let at = base.trim_start_matches("http://");
        let not_music = || format!("What answers at {at} isn't a media server with a music side");
        let w: Whoami = self
            .http
            .get(format!("{base}/api/peer/whoami"))
            .send()
            .await
            .with_context(|| format!("Nothing answers at {at}: is the media server on, with a music side?"))?
            .error_for_status()
            .with_context(not_music)?
            .json()
            .await
            .with_context(not_music)?;
        Ok(Server {
            host: Host::new(&w.id, &w.key)?,
            name: if w.name.is_empty() {
                "Media server".into()
            } else {
                w.name
            },
            base: base.to_string(),
        })
    }

    /// A signed request; its answer must be signed by the server.
    async fn request(&self, server: &Server, method: reqwest::Method, uri: &str, body: Vec<u8>) -> Result<Vec<u8>> {
        let (status, bytes) = self.send(server, method, uri, body, "application/json", None).await?;
        // Refusals are reported as they are.
        if !(200..300).contains(&status) {
            bail!("{}", error_text(status, &bytes));
        }
        Ok(bytes)
    }

    /// A signed request: its status and body. An accepted answer must be signed by the server;
    /// a refusal is given as it is.
    async fn send(
        &self,
        server: &Server,
        method: reqwest::Method,
        uri: &str,
        body: Vec<u8>,
        content_type: &str,
        timeout: Option<Duration>,
    ) -> Result<(u16, Vec<u8>)> {
        let signed = self.identity.sign(
            &server.host,
            method.as_str(),
            uri,
            &body,
            &self.name,
            0,
            0,
            now(),
            false,
        )?;
        let mut req = self.http.request(method, format!("{}{uri}", server.base));
        for (name, value) in &signed.headers {
            req = req.header(*name, value);
        }
        if !body.is_empty() {
            req = req.header("Content-Type", content_type).body(body);
        }
        if let Some(t) = timeout {
            req = req.timeout(t);
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("{} didn't answer", server.name))?;
        let status = resp.status();
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string()
        };
        let (date, reply) = (header("Date"), header(HEADER_REPLY));
        let bytes = crate::http::read_capped(resp, 64 << 20).await?;
        // Anything accepted must come from the server.
        if status.is_success() {
            self.identity
                .check_reply(&server.host, &signed.nonce, &date, &reply)
                .map_err(|e| anyhow!("{}: {e}", server.name))?;
        }
        Ok((status.as_u16(), bytes))
    }

    pub async fn get_json<T: DeserializeOwned>(&self, server: &Server, uri: &str) -> Result<T> {
        let bytes = self.request(server, reqwest::Method::GET, uri, Vec::new()).await?;
        serde_json::from_slice(&bytes).context("unexpected answer from the server")
    }

    pub async fn post_json<B: Serialize>(&self, server: &Server, uri: &str, body: &B) -> Result<()> {
        let body = serde_json::to_vec(body)?;
        self.request(server, reqwest::Method::POST, uri, body).await.map(|_| ())
    }

    pub async fn hello(&self, server: &Server) -> Result<Hello> {
        self.get_json(server, "/api/peer/hello").await
    }

    pub async fn library(&self, server: &Server) -> Result<Library> {
        self.get_json(server, "/api/peer/music/library").await
    }

    /// Gives the server this device's settings and logins (a settings file), sealed for it.
    pub async fn send_settings(&self, server: &Server, file: &[u8]) -> Result<()> {
        let sealed = self.identity.seal_for_host(&server.host, file)?;
        self.post_json(
            server,
            "/api/peer/music/settings",
            &Sealed {
                sealed,
                updated_at: now(),
            },
        )
        .await
    }

    /// The settings and logins the server keeps (a settings file), opened.
    pub async fn fetch_settings(&self, server: &Server) -> Result<Vec<u8>> {
        let s: Sealed = self.get_json(server, "/api/peer/music/settings").await?;
        self.identity.open_from(&server.host, &s.sealed)
    }

    /// Saves the server's file `path` as `dest`, as it comes (a big FLAC never sits in memory),
    /// with the share done so far going to `progress`. Its answer must be signed by the
    /// server; it gives up when the server stops sending for a minute.
    pub async fn download(
        &self,
        server: &Server,
        path: &str,
        dest: &Path,
        progress: &(dyn Fn(f32) + Send + Sync),
    ) -> Result<u64> {
        use tokio::io::AsyncWriteExt;
        // A ticket, as playback asks for the file: what the music side serves files for.
        let uri = format!("/api/peer/local/file?path={}", query_escape(path));
        let signed = self
            .identity
            .sign(&server.host, "GET", &uri, b"", &self.name, 0, 0, now(), true)?;
        let mut req = self
            .http
            .get(format!("{}{uri}", server.base))
            // The whole song, however long it takes; a stall is caught below.
            .timeout(Duration::from_secs(6 * 60 * 60));
        for (name, value) in &signed.headers {
            req = req.header(*name, value);
        }
        let mut resp = req
            .send()
            .await
            .with_context(|| format!("{} didn't answer", server.name))?;
        let status = resp.status();
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string()
        };
        let (date, reply) = (header("Date"), header(HEADER_REPLY));
        if !status.is_success() {
            let bytes = crate::http::read_capped(resp, 64 << 10).await.unwrap_or_default();
            bail!("{}", error_text(status.as_u16(), &bytes));
        }
        self.identity
            .check_reply(&server.host, &signed.nonce, &date, &reply)
            .map_err(|e| anyhow!("{}: {e}", server.name))?;
        let total = resp.content_length().filter(|n| *n > 0);
        let mut file = tokio::fs::File::create(dest)
            .await
            .with_context(|| format!("couldn't write {}", dest.display()))?;
        let mut done = 0u64;
        loop {
            let chunk = tokio::time::timeout(Duration::from_secs(60), resp.chunk())
                .await
                .map_err(|_| anyhow!("{} stopped sending", server.name))?
                .with_context(|| format!("{} stopped sending", server.name))?;
            let Some(chunk) = chunk else { break };
            file.write_all(&chunk).await?;
            done += chunk.len() as u64;
            if let Some(total) = total {
                progress((done as f32 / total as f32).min(1.0));
            }
        }
        file.flush().await?;
        if total.is_some_and(|t| done < t) {
            bail!("{} sent only part of the song", server.name);
        }
        Ok(done)
    }

    /// A song's cover on the server (`None`: it has none).
    pub async fn art(&self, server: &Server, path: &str) -> Result<Option<Vec<u8>>> {
        let uri = format!("/api/peer/music/art?path={}", query_escape(path));
        let (status, bytes) = self
            .send(server, reqwest::Method::GET, &uri, Vec::new(), "", None)
            .await?;
        match status {
            200..=299 => Ok(Some(bytes)),
            404 => Ok(None),
            _ => bail!("{}", error_text(status, &bytes)),
        }
    }

    /// Where uploads go on the server; refused unless it lets this device upload.
    pub async fn upload_target(&self, server: &Server) -> Result<UploadTarget> {
        self.get_json(server, "/api/peer/upload").await
    }

    /// Sends `file` into the server's library folder `folder`, as `rel` there (slashes between
    /// folders), a piece at a time from where the server has it: an upload cut short carries
    /// on where it stopped. `sent` hears how much of it is there after each piece.
    #[allow(clippy::too_many_arguments)]
    pub async fn upload_file(
        &self,
        server: &Server,
        folder: usize,
        rel: &str,
        file: &std::path::Path,
        chunk: u64,
        stop: &std::sync::atomic::AtomicBool,
        mut sent: impl FnMut(u64),
    ) -> Result<Uploaded> {
        use std::sync::atomic::Ordering;
        use tokio::io::{AsyncReadExt, AsyncSeekExt};

        let total = tokio::fs::metadata(file)
            .await
            .with_context(|| format!("couldn't read {}", file.display()))?
            .len();
        let chunk = chunk.clamp(64 << 10, MAX_PIECE);
        let path = query_escape(rel);
        let ask = |offset: Option<u64>| {
            let at = offset.map(|o| format!("&offset={o}")).unwrap_or_default();
            format!("/api/peer/upload?folder={folder}&path={path}&total={total}{at}")
        };
        // Where it stands there (a piece may come back refused: then again from there).
        let stands = |status: u16, bytes: &[u8]| -> Result<Option<UploadAnswer>> {
            match status {
                200..=299 => Ok(Some(
                    serde_json::from_slice(bytes).context("unexpected answer from the server")?,
                )),
                409 => Ok(None),
                _ => bail!("{}", error_text(status, bytes)),
            }
        };
        let (status, bytes) = self
            .send(server, reqwest::Method::GET, &ask(None), Vec::new(), "", None)
            .await?;
        let Some(first) = stands(status, &bytes)? else {
            return Ok(Uploaded::Different);
        };
        if first.done {
            sent(total);
            return Ok(Uploaded::AlreadyThere);
        }
        let mut have = first.have;
        let mut f = tokio::fs::File::open(file)
            .await
            .with_context(|| format!("couldn't read {}", file.display()))?;
        let mut refused = 0;
        loop {
            sent(have);
            if stop.load(Ordering::Relaxed) {
                bail!("stopped");
            }
            let n = chunk.min(total.saturating_sub(have));
            let mut piece = vec![0u8; n as usize];
            f.seek(std::io::SeekFrom::Start(have)).await?;
            f.read_exact(&mut piece)
                .await
                .with_context(|| format!("couldn't read {}", file.display()))?;
            // A piece takes as long as the network needs.
            let (status, bytes) = self
                .send(
                    server,
                    reqwest::Method::POST,
                    &ask(Some(have)),
                    piece,
                    "application/octet-stream",
                    Some(Duration::from_secs(600)),
                )
                .await?;
            match stands(status, &bytes)? {
                Some(a) if a.done => {
                    sent(total);
                    return Ok(Uploaded::Sent);
                }
                Some(a) => {
                    refused = 0;
                    have = a.have;
                }
                // Refused: it doesn't follow on from what's there (asked again), or another
                // file has its name.
                None => {
                    refused += 1;
                    if refused > 3 {
                        bail!("{}", error_text(status, &bytes));
                    }
                    let (status, bytes) = self
                        .send(server, reqwest::Method::GET, &ask(None), Vec::new(), "", None)
                        .await?;
                    match stands(status, &bytes)? {
                        None => return Ok(Uploaded::Different),
                        Some(a) if a.done => return Ok(Uploaded::AlreadyThere),
                        Some(a) => have = a.have,
                    }
                }
            }
        }
    }
}

/// A refusal's `{"error": "…"}` text, or its status.
fn error_text(status: u16, bytes: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_owned))
        .unwrap_or_else(|| format!("the server answered {status}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(base_url("pi.local"), "http://pi.local:43411");
        assert_eq!(base_url("http://192.168.1.5/"), "http://192.168.1.5:43411");
        assert_eq!(base_url("192.168.1.5:5000"), "http://192.168.1.5:5000");
    }

    /// Downloads a song from a stand-in music side made from mediaserver's own share code
    /// (UTAKO_STANDIN is its address; it serves the files of a folder here, and lets a device in
    /// at /test/allow): the file arrives whole, byte for byte, in a signed answer.
    /// `UTAKO_STANDIN=127.0.0.1:43499 cargo test -- --ignored download_from_stand_in`
    #[tokio::test]
    #[ignore]
    async fn download_from_stand_in() {
        let address = std::env::var("UTAKO_STANDIN").expect("UTAKO_STANDIN");
        let client = Client::new(
            Arc::new(Identity::from_private(rand::random())),
            "Utako download test".into(),
        );
        let base = base_url(&address);
        let server = client.whoami(&base).await.unwrap();
        reqwest::get(format!("{base}/test/allow?id={}", client.identity.id))
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let lib = client.library(&server).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        for song in &lib.tracks {
            let dest = dir.path().join("song");
            let seen = std::sync::Mutex::new(0.0f32);
            let size = client
                .download(&server, &song.path, &dest, &|p| *seen.lock().unwrap() = p)
                .await
                .unwrap();
            // The stand-in serves files of this computer, so the original is here to compare.
            let original = std::fs::read(&song.path).unwrap();
            assert_eq!(size as usize, original.len());
            assert!(
                std::fs::read(&dest).unwrap() == original,
                "{} arrived changed",
                song.path
            );
            assert_eq!(*seen.lock().unwrap(), 1.0);
        }
        // A song it doesn't have is refused.
        let missing = client
            .download(&server, "/no/such/song.flac", &dir.path().join("x"), &|_| {})
            .await;
        assert!(missing.is_err());
    }

    /// Against a real mediaserver's music side (UTAKO_SERVER is its address; UTAKO_DASHBOARD
    /// and UTAKO_SESSION its dashboard and a signed-in `kumo_session` cookie, which let the
    /// test's devices in): hello before and after it allows this device, the library, covers,
    /// settings there and back, a song mpv streams with a ticket and seeks in, and an upload
    /// stopped and carried on. Its library needs a song of 20 s or more, and one with a cover.
    /// `UTAKO_SERVER=10.77.0.1 UTAKO_DASHBOARD=http://10.77.0.1:43211 UTAKO_SESSION=… \
    ///  cargo test --release -- --ignored real_server`
    #[tokio::test]
    #[ignore]
    async fn real_server() {
        use crate::player::mpv::{Mpv, MpvOptions};
        use serde_json::{json, Value};
        use std::sync::atomic::{AtomicBool, Ordering};

        let address = std::env::var("UTAKO_SERVER").expect("UTAKO_SERVER");
        let dashboard = std::env::var("UTAKO_DASHBOARD").expect("UTAKO_DASHBOARD");
        let session = std::env::var("UTAKO_SESSION").expect("UTAKO_SESSION");
        let let_in = |id: String, change: Value| {
            let (dashboard, session) = (dashboard.clone(), session.clone());
            async move {
                reqwest::Client::builder()
                    .no_proxy()
                    .build()
                    .unwrap()
                    .post(format!("{dashboard}/api/music/devices/{id}"))
                    .header("Cookie", format!("kumo_session={session}"))
                    .json(&change)
                    .send()
                    .await
                    .unwrap()
                    .error_for_status()
                    .unwrap();
            }
        };
        let client = Client::new(
            Arc::new(Identity::from_private(rand::random())),
            "Utako on test, with a comma".into(),
        );
        let base = base_url(&address);
        let server = client.whoami(&base).await.unwrap();
        assert!(!server.name.is_empty());

        // Not let in yet: hello says so, the rest is refused.
        let hello = client.hello(&server).await.unwrap();
        assert!(!hello.shares, "not allowed yet");
        let refused = client.library(&server).await.unwrap_err().to_string();
        assert!(refused.contains("Utako on test, with a comma"), "{refused}");
        let_in(client.identity.id.clone(), json!({"allowed": true})).await;
        let hello = client.hello(&server).await.unwrap();
        assert!(hello.shares && !hello.uploads);
        let lib = client.library(&server).await.unwrap();
        assert!(!lib.tracks.is_empty());
        assert_eq!(lib.version, hello.files);

        // Covers: one there, one not (or none at all).
        let with = lib.tracks.iter().find(|t| t.has_art).expect("a song with a cover");
        let art = client.art(&server, &with.path).await.unwrap().expect("its cover");
        assert!(image::load_from_memory(&art).is_ok(), "the cover is a picture");
        if let Some(without) = lib.tracks.iter().find(|t| !t.has_art) {
            assert!(client.art(&server, &without.path).await.unwrap().is_none());
        }

        // Settings: sent from this device, and fetched by another one the server allowed.
        let file = r#"{"version":1,"hasKeys":true,"note":"é and commas, too"}"#.as_bytes();
        client.send_settings(&server, file).await.unwrap();
        assert_eq!(client.fetch_settings(&server).await.unwrap(), file);
        let other_device = Client::new(
            Arc::new(Identity::from_private(rand::random())),
            "Utako on phone".into(),
        );
        assert!(other_device.hello(&server).await.is_ok());
        assert!(
            other_device.fetch_settings(&server).await.is_err(),
            "only for allowed devices"
        );
        let_in(other_device.identity.id.clone(), json!({"allowed": true})).await;
        assert_eq!(other_device.fetch_settings(&server).await.unwrap(), file);

        // A song, as playback starts it: a ticket in mpv's headers, then a seek.
        let song = lib
            .tracks
            .iter()
            .find(|t| t.duration_ms >= 20_000)
            .expect("a song of 20 s or more");
        let (url, headers) = server.file(&client, &song.path).unwrap();
        let fields = headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect::<Vec<_>>()
            .join(",");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let opts = MpvOptions {
            binary: "mpv".into(),
            volume: 0.0,
            replaygain: false,
            gapless: true,
            audio_device: String::new(),
            exclusive: false,
            ytdl: "yt-dlp".into(),
        };
        let mpv = Mpv::spawn(&opts, 11, tx).await.unwrap();
        mpv.command(json!(["set_property", "ao", "null"])).await.unwrap();
        mpv.load_with(&url, 0.0, &[("http-header-fields", fields)])
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let prop = |name: &'static str| {
            let mpv = &mpv;
            async move { mpv.command(json!(["get_property", name])).await.unwrap_or(Value::Null) }
        };
        let duration = prop("duration").await.as_f64().unwrap_or(0.0);
        let want = song.duration_ms as f64 / 1000.0;
        assert!(
            (duration - want).abs() < 1.0,
            "it plays (duration {duration}, {want} said)"
        );
        mpv.command(json!(["seek", want - 5.0, "absolute"])).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let at = prop("time-pos").await.as_f64().unwrap_or(0.0);
        assert!(at >= want - 5.0, "it seeks (at {at})");
        mpv.quit().await;

        // The same ticket again, as mpv sends it when it seeks past what it has.
        let plain = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut again = plain.get(&url).header("Range", "bytes=1000-");
        for (name, value) in &headers {
            again = again.header(*name, value);
        }
        assert_eq!(again.send().await.unwrap().status(), 206);
        // A ticket for one file doesn't open another.
        if let Some(other) = lib.tracks.iter().find(|t| t.path != song.path) {
            let (other, _) = server.file(&client, &other.path).unwrap();
            let mut req = plain.get(&other);
            for (name, value) in &headers {
                req = req.header(*name, value);
            }
            assert_eq!(req.send().await.unwrap().status(), 403);
        }

        // Uploads: refused until the dashboard lets it, then a song of three pieces, stopped
        // after the first and carried on; sent again, it's there; another file of that name
        // isn't sent over it.
        assert!(client.upload_target(&server).await.is_err());
        let_in(client.identity.id.clone(), json!({"uploads": true})).await;
        assert!(client.hello(&server).await.unwrap().uploads);
        let target = client.upload_target(&server).await.unwrap();
        assert!(!target.folders.is_empty() && target.chunk > 0);
        let tmp = tempfile::tempdir().unwrap();
        let big = tmp.path().join("big.mp3");
        let bytes: Vec<u8> = (0..(MAX_PIECE * 2 + 12_345)).map(|i| (i % 251) as u8).collect();
        std::fs::write(&big, &bytes).unwrap();
        let rel = format!("Utako upload test {}/big.mp3", rand::random::<u32>());
        let stop = AtomicBool::new(false);
        let mut seen = Vec::new();
        let first = client
            .upload_file(&server, 0, &rel, &big, target.chunk, &stop, |have| {
                seen.push(have);
                if have > 0 {
                    stop.store(true, Ordering::Relaxed);
                }
            })
            .await;
        assert!(first.is_err(), "stopped");
        assert_eq!(seen, vec![0, MAX_PIECE]);
        stop.store(false, Ordering::Relaxed);
        let mut seen = Vec::new();
        let done = client
            .upload_file(&server, 0, &rel, &big, target.chunk, &stop, |have| seen.push(have))
            .await
            .unwrap();
        assert_eq!(done, Uploaded::Sent);
        assert_eq!(seen.first(), Some(&MAX_PIECE), "carried on where it stopped");
        assert_eq!(seen.last(), Some(&(bytes.len() as u64)));
        let again = client
            .upload_file(&server, 0, &rel, &big, target.chunk, &stop, |_| {})
            .await
            .unwrap();
        assert_eq!(again, Uploaded::AlreadyThere);
        let other = tmp.path().join("other.mp3");
        std::fs::write(&other, b"another file").unwrap();
        let different = client
            .upload_file(&server, 0, &rel, &other, target.chunk, &stop, |_| {})
            .await
            .unwrap();
        assert_eq!(different, Uploaded::Different);
        // The server reads it in a moment: its library has it then.
        let mut found = false;
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if client.hello(&server).await.unwrap().files != lib.version {
                found = client
                    .library(&server)
                    .await
                    .unwrap()
                    .tracks
                    .iter()
                    .any(|t| t.path.ends_with(&rel));
                if found {
                    break;
                }
            }
        }
        assert!(found, "the upload is in the library");
    }

    #[test]
    fn library_json() {
        let lib: Library = serde_json::from_str(
            r#"{"version":"3","tracks":[{"path":"/m/a.flac","title":"A","artist":"B","trackNo":2,"durationMs":1000}]}"#,
        )
        .unwrap();
        assert_eq!(lib.tracks[0].track_no, Some(2));
        assert_eq!(lib.tracks[0].duration_ms, 1000);
    }
}
