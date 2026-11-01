//! Talking to a media server: signed requests and checked answers (identity.rs), and the few
//! things Utako asks of it. What a music side answers is in docs/media-server.md.

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
            req = req.header("Content-Type", "application/json").body(body);
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
        // Refusals are reported as they are; anything accepted must come from the server.
        if !status.is_success() {
            let error = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_owned))
                .unwrap_or_else(|| status.to_string());
            bail!("{error}");
        }
        self.identity
            .check_reply(&server.host, &signed.nonce, &date, &reply)
            .map_err(|e| anyhow!("{}: {e}", server.name))?;
        Ok(bytes)
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

    /// Against a stand-in music side made from mediaserver's own share code (UTAKO_STANDIN
    /// is its address): hello before and after it allows this device, the library, settings
    /// there and back, and a song mpv streams with a ticket and seeks in.
    /// `UTAKO_STANDIN=127.0.0.1:43499 cargo test -- --ignored stand_in_server`
    #[tokio::test]
    #[ignore]
    async fn stand_in_server() {
        use crate::player::mpv::{Mpv, MpvOptions};
        use serde_json::{json, Value};

        let address = std::env::var("UTAKO_STANDIN").expect("UTAKO_STANDIN");
        let client = Client::new(
            Arc::new(Identity::from_private(rand::random())),
            "Utako on test, with a comma".into(),
        );
        let base = base_url(&address);
        let server = client.whoami(&base).await.unwrap();
        assert_eq!(server.name, "Test Pi");

        let hello = client.hello(&server).await.unwrap();
        assert!(!hello.shares, "not allowed yet");
        assert!(client.library(&server).await.is_err(), "no library before it's allowed");
        reqwest::get(format!("{base}/test/allow?id={}", client.identity.id))
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let hello = client.hello(&server).await.unwrap();
        assert!(hello.shares);
        let lib = client.library(&server).await.unwrap();
        assert_eq!(lib.tracks.len(), 3);
        assert_eq!(lib.version, hello.files);

        // Settings: sent from this device, and fetched by another one the server allowed.
        let file = r#"{"version":1,"hasKeys":true,"note":"é and commas, too"}"#.as_bytes();
        client.send_settings(&server, file).await.unwrap();
        assert_eq!(client.fetch_settings(&server).await.unwrap(), file);
        let other_device = Client::new(
            Arc::new(Identity::from_private(rand::random())),
            "Utako on phone".into(),
        );
        assert!(
            other_device.fetch_settings(&server).await.is_err(),
            "only for allowed devices"
        );
        reqwest::get(format!("{base}/test/allow?id={}", other_device.identity.id))
            .await
            .unwrap();
        assert_eq!(other_device.fetch_settings(&server).await.unwrap(), file);

        // A song, as playback starts it: a ticket in mpv's headers, then a seek.
        let (url, headers) = server.file(&client, &lib.tracks[1].path).unwrap();
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
        assert!((duration - 30.0).abs() < 1.0, "it plays (duration {duration})");
        mpv.command(json!(["seek", 20.0, "absolute"])).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let at = prop("time-pos").await.as_f64().unwrap_or(0.0);
        assert!(at >= 20.0, "it seeks (at {at})");
        mpv.quit().await;

        // The same ticket again, as mpv sends it when it seeks past what it has.
        let mut again = reqwest::Client::new().get(&url).header("Range", "bytes=1000-");
        for (name, value) in &headers {
            again = again.header(*name, value);
        }
        assert_eq!(again.send().await.unwrap().status(), 206);

        // A ticket for one file doesn't open another.
        let (other, _) = server.file(&client, &lib.tracks[0].path).unwrap();
        let mut req = reqwest::Client::new().get(&other);
        for (name, value) in &headers {
            req = req.header(*name, value);
        }
        assert_eq!(req.send().await.unwrap().status(), 403);
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
