//! zotify (github.com/zotify-dev/zotify) saves Spotify songs straight from Spotify: Spotify's own
//! Ogg Vorbis, at 320 kbps with Premium. It logs in with Utako's Spotify login (the reusable
//! credentials librespot keeps once it has connected), so it needs no login of its own.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use librespot_core::authentication::Credentials;
use librespot_protocol::authentication::AuthenticationType;
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::downloader::Progress;

/// Where zotify is installed from.
pub const SOURCE: &str = "git+https://github.com/zotify-dev/zotify";

/// librespot's reusable credentials, in Utako's Spotify folder.
const LIBRESPOT_LOGIN: &str = "credentials.json";
/// The same login as zotify reads it, next to them.
pub const ZOTIFY_LOGIN: &str = "zotify-credentials.json";

/// The files zotify saves (its `DOWNLOAD_FORMAT`s).
const AUDIO: &[&str] = &["ogg", "mp3", "m4a", "opus", "aac", "flac"];

/// How much of the progress bar the download itself takes; zotify then remuxes (or converts)
/// the file with ffmpeg.
const DOWNLOADED: f32 = 0.95;

/// zotify isn't installed (or not where the settings say).
#[derive(Debug)]
pub struct NotInstalled;

impl std::fmt::Display for NotInstalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("zotify is not installed")
    }
}

impl std::error::Error for NotInstalled {}

/// The zotify program and the user's extra options (e.g. `--download-real-time=True`).
#[derive(Debug, Clone, PartialEq)]
pub struct Zotify {
    pub program: String,
    pub args: Vec<String>,
    /// Convert to MP3 instead of keeping Spotify's Ogg Vorbis.
    pub mp3: bool,
    /// Utako's Spotify folder, where librespot keeps the login zotify borrows.
    pub spotify_dir: PathBuf,
}

impl Zotify {
    /// From the settings: the program and its extra options as typed (quotes allowed).
    pub fn new(program: &str, args: &str, spotify_dir: &Path) -> Zotify {
        Zotify {
            program: crate::tools::resolve(program, "zotify"),
            args: crate::providers::youtube::split_args(args),
            mp3: false,
            spotify_dir: spotify_dir.to_path_buf(),
        }
    }

    pub fn with_mp3(mut self, mp3: bool) -> Zotify {
        self.mp3 = mp3;
        self
    }

    fn command(&self) -> tokio::process::Command {
        let mut cmd = crate::tools::command(&self.program);
        // Python: no buffering, so the progress comes as it goes, and UTF-8 even where the
        // console isn't (song names on Windows).
        cmd.env("PYTHONUNBUFFERED", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUTF8", "1");
        // zotify runs ffmpeg from the PATH; the one that comes with Utako isn't on it.
        if let Some(dir) = crate::tools::bundled_ffmpeg_dir() {
            let path = std::env::var_os("PATH").unwrap_or_default();
            if let Ok(joined) = std::env::join_paths(std::iter::once(dir).chain(std::env::split_paths(&path))) {
                cmd.env("PATH", joined);
            }
        }
        cmd.stdin(Stdio::null()).kill_on_drop(true);
        cmd
    }

    /// Whether zotify is installed and starts. It has no `--version`; `--help` loads all of it.
    pub async fn check(&self) -> Result<()> {
        let mut cmd = self.command();
        cmd.arg("--help").stdout(Stdio::piped()).stderr(Stdio::piped());
        let out = tokio::time::timeout(Duration::from_secs(30), cmd.output())
            .await
            .map_err(|_| anyhow!("zotify didn't answer"))?
            .map_err(spawn_error)?;
        if !out.status.success() || !String::from_utf8_lossy(&out.stdout).contains("zotify") {
            // The last line of Python's traceback says why.
            let stderr = String::from_utf8_lossy(&out.stderr);
            match stderr.lines().rev().map(str::trim).find(|l| !l.is_empty()) {
                Some(why) => bail!("doesn't start: {why}"),
                None => bail!("doesn't start"),
            }
        }
        Ok(())
    }

    /// Saves Spotify song `id` (base62) in `work`, a folder of its own, and returns the file.
    pub async fn download(&self, id: &str, work: &Path, progress: Progress<'_>) -> Result<PathBuf> {
        let login = self.login_file().await?;
        let root = work.join("zotify");
        tokio::fs::create_dir_all(&root)
            .await
            .with_context(|| format!("couldn't create {}", root.display()))?;
        let mut cmd = self.command();
        cmd.args(self.download_args(id, work, &root, &login))
            .current_dir(work)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(spawn_error)?;
        crate::tools::end_with_us(&child);
        let (stdout, stderr) = (child.stdout.take(), child.stderr.take());

        // zotify says what went wrong (and still exits with 0); its progress bar is tqdm's.
        let said = Mutex::new(VecDeque::<String>::new());
        let on_line = |line: &str| {
            if let Some(p) = progress_of(line) {
                progress(p * DOWNLOADED);
                return;
            }
            let mut said = said.lock().unwrap();
            if said.len() == 300 {
                said.pop_front();
            }
            said.push_back(line.to_string());
        };
        let (_, _, status) = tokio::join!(read_lines(stdout, &on_line), read_lines(stderr, &on_line), child.wait());
        let status = status.context("zotify stopped")?;
        if let Some(file) = saved_file(&root) {
            progress(1.0);
            return Ok(file);
        }
        let said: Vec<String> = said.into_inner().unwrap().into();
        let said = said.join("\n");
        tracing::warn!("zotify saved nothing for {id} ({status}):\n{said}");
        bail!("{}", failure(&said))
    }

    fn download_args(&self, id: &str, work: &Path, root: &Path, login: &Path) -> Vec<OsString> {
        let opt = |name: &str, value: &Path| {
            let mut arg = OsString::from(format!("--{name}="));
            arg.push(value.as_os_str());
            arg
        };
        let mut args: Vec<OsString> = vec!["--no-splash".into()];
        // What the user's own options may change (the last one given wins).
        if self.mp3 {
            args.push("--download-format=mp3".into());
        }
        args.extend(self.args.iter().map(OsString::from));
        // What Utako needs, after them. Its own config file keeps the user's zotify settings
        // (their folders and file names) out of it.
        args.push(opt("config-location", &work.join("zotify-config.json")));
        args.push(opt("credentials-location", login));
        args.push("--save-credentials=False".into());
        args.push(opt("root-path", root));
        args.push(opt("song-archive", &work.join("song-archive")));
        for fixed in [
            "--output={track_id}.{ext}",
            // Utako embeds lyrics itself.
            "--download-lyrics=False",
            "--skip-existing=False",
            "--skip-previously-downloaded=False",
            "--bulk-wait-time=0",
            "--print-progress-info=False",
            "--print-download-progress=True",
            "--print-skips=True",
            "--print-errors=True",
            "--print-api-errors=True",
        ] {
            args.push(fixed.into());
        }
        args.push(format!("https://open.spotify.com/track/{id}").into());
        args
    }

    /// Utako's Spotify login as zotify reads it.
    async fn login_file(&self) -> Result<PathBuf> {
        let from = self.spotify_dir.join(LIBRESPOT_LOGIN);
        // Just after logging in, Utako's Spotify connection may still be on its way.
        let mut text = None;
        for _ in 0..40 {
            if let Ok(t) = tokio::fs::read_to_string(&from).await {
                text = Some(t);
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let text = text.ok_or_else(|| {
            anyhow!("Spotify isn't connected: check Settings › Spotify (or play a Spotify song), then try again")
        })?;
        let to = self.spotify_dir.join(ZOTIFY_LOGIN);
        write_private(&to, zotify_login(&text)?.as_bytes()).await?;
        Ok(to)
    }
}

/// The login file zotify (librespot-python) reads, made from librespot's: its reusable
/// credentials, which any librespot client can log in with.
fn zotify_login(librespot: &str) -> Result<String> {
    let creds: Credentials = serde_json::from_str(librespot)
        .map_err(|_| anyhow!("Utako's Spotify login can't be read: log out of Spotify and in again"))?;
    let username = creds.username.filter(|u| !u.is_empty());
    let (Some(username), AuthenticationType::AUTHENTICATION_STORED_SPOTIFY_CREDENTIALS) = (username, creds.auth_type)
    else {
        bail!("Utako's Spotify login can't be lent to zotify: log out of Spotify and in again");
    };
    Ok(serde_json::json!({
        "username": username,
        "credentials": base64::engine::general_purpose::STANDARD.encode(&creds.auth_data),
        "type": "AUTHENTICATION_STORED_SPOTIFY_CREDENTIALS",
    })
    .to_string())
}

/// Writes a file only this user can read, all at once (another download may be reading it).
async fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    if tokio::fs::read(path).await.is_ok_and(|have| have == bytes) {
        return Ok(());
    }
    static N: AtomicU64 = AtomicU64::new(0);
    let tmp = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let (file, bytes) = (tmp.clone(), bytes.to_vec());
    let written = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        std::io::Write::write_all(&mut opts.open(&file)?, &bytes)
    })
    .await?;
    let result = match written {
        Ok(()) => tokio::fs::rename(&tmp, path).await,
        Err(e) => Err(e),
    };
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e).with_context(|| format!("couldn't write {}", path.display()));
    }
    Ok(())
}

/// Hands each line of `pipe` to `each`; tqdm ends its progress lines with `\r`.
async fn read_lines(pipe: Option<impl AsyncRead + Unpin>, each: impl Fn(&str)) {
    let Some(mut pipe) = pipe else { return };
    let mut buf = [0u8; 4096];
    let mut pending: Vec<u8> = Vec::new();
    loop {
        let n = match pipe.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        pending.extend_from_slice(&buf[..n]);
        while let Some(end) = pending.iter().position(|b| matches!(b, b'\r' | b'\n')) {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let text = String::from_utf8_lossy(&line[..end]);
            if !text.trim().is_empty() {
                each(&text);
            }
        }
    }
    let text = String::from_utf8_lossy(&pending);
    if !text.trim().is_empty() {
        each(&text);
    }
}

/// The fraction done on one of tqdm's lines: `Artist - Song:  45%|####5     | 1.2M/2.7M […]`.
fn progress_of(line: &str) -> Option<f32> {
    let bar = line.rfind("%|")?;
    let digits = line[..bar].trim_end();
    let start = digits
        .rfind(|c: char| !(c.is_ascii_digit() || c == '.'))
        .map_or(0, |i| i + 1);
    let percent: f32 = digits[start..].parse().ok()?;
    Some((percent / 100.0).clamp(0.0, 1.0))
}

/// The song zotify saved: the largest audio file under `root`.
fn saved_file(root: &Path) -> Option<PathBuf> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| AUDIO.contains(&x.to_string_lossy().to_ascii_lowercase().as_str()))
        })
        .filter_map(|e| Some((e.metadata().ok()?.len(), e.into_path())))
        .filter(|(size, _)| *size > 0)
        .max_by_key(|(size, _)| *size)
        .map(|(_, path)| path)
}

/// Why zotify saved nothing, from what it said.
fn failure(said: &str) -> String {
    let lines: Vec<&str> = said
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let has = |needle: &str| lines.iter().any(|l| l.contains(needle));
    if has("SONG IS UNAVAILABLE") {
        return "Spotify can't play this song here (not available in your country?)".into();
    }
    if has("FFMPEG NOT FOUND") || has("ensure ffmpeg is installed") {
        return format!("zotify needs ffmpeg. {}", crate::tools::install_hint("ffmpeg"));
    }
    if has("AuthenticationException") || has("BadCredentials") || has("Username:") || has("EOFError") {
        return "zotify couldn't log in with your Spotify login: log out of Spotify in Settings and in again".into();
    }
    if has("Max retries exceeded") || has("ConnectionError") || has("Connection refused") {
        return "zotify couldn't reach Spotify (offline?)".into();
    }
    // "###   SKIPPING …   ###", "Track_ID: …", then the reason and its traceback.
    if let Some(at) = lines.iter().position(|l| l.starts_with("Track_ID:")) {
        let reason = lines[at + 1..]
            .iter()
            .find(|l| !l.starts_with("Traceback") && !l.starts_with(char::is_whitespace));
        if let Some(reason) = reason {
            return format!("Spotify: {}", reason.trim());
        }
    }
    // Anything else: the last line (of Python's traceback, when it crashed) says what.
    match lines.last() {
        Some(last) => format!("zotify: {}", last.trim()),
        None => "zotify saved nothing".into(),
    }
}

fn spawn_error(e: std::io::Error) -> anyhow::Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        anyhow::Error::new(NotInstalled)
    } else {
        anyhow!("couldn't start zotify: {e}")
    }
}

/// How to get zotify on this system.
pub fn install_hint() -> String {
    if cfg!(target_os = "linux") {
        format!("Install it with: sudo pacman -S --needed python-pipx git && pipx install {SOURCE}")
    } else if cfg!(target_os = "macos") {
        format!("Install it with: brew install pipx git && pipx install {SOURCE}")
    } else {
        format!("Install Python and Git, then run: py -m pip install --user pipx && py -m pipx install {SOURCE}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utakos_login_is_lent_to_zotify() {
        let creds = Credentials {
            username: Some("someone".into()),
            auth_type: AuthenticationType::AUTHENTICATION_STORED_SPOTIFY_CREDENTIALS,
            auth_data: vec![1, 2, 3, 250],
        };
        // As librespot keeps it.
        let librespot = serde_json::to_string(&creds).unwrap();
        let zotify: serde_json::Value = serde_json::from_str(&zotify_login(&librespot).unwrap()).unwrap();
        assert_eq!(zotify["username"], "someone");
        assert_eq!(zotify["credentials"], "AQID+g==");
        assert_eq!(zotify["type"], "AUTHENTICATION_STORED_SPOTIFY_CREDENTIALS");

        // Only reusable credentials of a known user.
        let token = Credentials::with_access_token("token");
        assert!(zotify_login(&serde_json::to_string(&token).unwrap()).is_err());
        assert!(zotify_login("not json").is_err());
    }

    #[tokio::test]
    async fn the_login_file_is_written_once_and_private() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join(ZOTIFY_LOGIN);
        write_private(&file, b"one").await.unwrap();
        write_private(&file, b"one").await.unwrap();
        write_private(&file, b"two").await.unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"two");
        assert_eq!(
            std::fs::read_dir(tmp.path()).unwrap().count(),
            1,
            "no temporary files left"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn progress_from_tqdm() {
        assert_eq!(
            progress_of("Artist - Song:  45%|####5     | 1.23M/2.73M [00:01<00:01, 1.20MB/s]"),
            Some(0.45)
        );
        assert_eq!(progress_of("100%|##########| 2.73M/2.73M [00:02<00:00]"), Some(1.0));
        // A song named like a progress bar doesn't fool it.
        assert_eq!(
            progress_of("50%|Song:   7%|7         | 0.2M/2.7M [00:00<00:02]"),
            Some(0.07)
        );
        assert_eq!(progress_of("###   SKIPPING: A - B (SONG IS UNAVAILABLE)   ###"), None);
    }

    #[test]
    fn why_nothing_was_saved() {
        assert!(failure("\n###   SKIPPING: A - B (SONG IS UNAVAILABLE)   ###\n").contains("can't play this song"));
        assert!(failure("###   SKIPPING MP3 CONVERSION - FFMPEG NOT FOUND   ###").contains("needs ffmpeg"));
        let crash = "Traceback (most recent call last):\n  File \"zotify.py\", line 32, in login\n    \
                     raise Session.SpotifyAuthenticationException(ap_login_failed)\n\
                     librespot.core.Session.SpotifyAuthenticationException: BadCredentials";
        assert!(failure(crash).contains("couldn't log in"));
        assert!(
            failure("Username: Traceback (most recent call last):\nEOFError: EOF when reading a line")
                .contains("couldn't log in")
        );
        let offline = "requests.exceptions.ConnectionError: HTTPSConnectionPool(host='apresolve.spotify.com', \
                       port=443): Max retries exceeded with url: /?type=accesspoint";
        assert_eq!(failure(offline), "zotify couldn't reach Spotify (offline?)");
        let skipped = "###   SKIPPING: A - B (GENERAL DOWNLOAD ERROR)   ###\nTrack_ID: abc\n\n\
                       Failed fetching audio key! gid: x, fileId: y\n\nTraceback (most recent call last):\n  \
                       File \"track.py\", line 1\nRuntimeError: Failed fetching audio key!";
        assert_eq!(
            failure(skipped),
            "Spotify: Failed fetching audio key! gid: x, fileId: y"
        );
        assert_eq!(
            failure("Traceback (most recent call last):\n  File \"x\"\nModuleNotFoundError: No module named 'x'"),
            "zotify: ModuleNotFoundError: No module named 'x'"
        );
        assert_eq!(failure(""), "zotify saved nothing");
    }

    #[test]
    fn utakos_options_win_and_the_song_is_last() {
        let z = Zotify {
            program: "zotify".into(),
            args: vec!["--download-real-time=True".into(), "--root-path=/elsewhere".into()],
            mp3: true,
            spotify_dir: PathBuf::from("/data/spotify"),
        };
        let work = Path::new("/music/.multimusic-1-1");
        let args: Vec<String> = z
            .download_args(
                "abc",
                work,
                &work.join("zotify"),
                Path::new("/data/spotify/zotify-credentials.json"),
            )
            .into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let at = |a: &str| args.iter().position(|x| x == a).unwrap();
        assert!(at("--download-format=mp3") < at("--download-real-time=True"));
        assert!(at("--root-path=/elsewhere") < at("--root-path=/music/.multimusic-1-1/zotify"));
        assert!(args.contains(&"--credentials-location=/data/spotify/zotify-credentials.json".to_string()));
        assert_eq!(args.last().unwrap(), "https://open.spotify.com/track/abc");
    }

    /// A stand-in zotify that prints like the real one: the song arrives with its progress.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_download_through_a_stand_in() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let spotify = tmp.path().join("spotify");
        std::fs::create_dir_all(&spotify).unwrap();
        let creds = Credentials {
            username: Some("someone".into()),
            auth_type: AuthenticationType::AUTHENTICATION_STORED_SPOTIFY_CREDENTIALS,
            auth_data: vec![9; 8],
        };
        std::fs::write(spotify.join(LIBRESPOT_LOGIN), serde_json::to_string(&creds).unwrap()).unwrap();
        let program = tmp.path().join("zotify");
        std::fs::write(
            &program,
            "#!/bin/sh\n\
             for a in \"$@\"; do case \"$a\" in --root-path=*) root=\"${a#--root-path=}\";; \
             --credentials-location=*) login=\"${a#--credentials-location=}\";; esac; done\n\
             grep -q someone \"$login\" || { echo 'EOFError: EOF when reading a line' >&2; exit 1; }\n\
             printf 'A - B:  10%%|#         | 1k/10k\\rA - B: 100%%|##########| 10k/10k\\n' >&2\n\
             case \"$*\" in *nothing*) echo '###   SKIPPING: A - B (SONG IS UNAVAILABLE)   ###'; exit 0;; esac\n\
             echo audio > \"$root/abc.ogg\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let z = Zotify::new(&program.to_string_lossy(), "", &spotify);
        let seen = Mutex::new(Vec::new());
        let progress = |p: f32| seen.lock().unwrap().push(p);

        let work = tmp.path().join("work");
        let file = z.download("abc", &work, &progress).await.unwrap();
        assert_eq!(file, work.join("zotify").join("abc.ogg"));
        assert_eq!(*seen.lock().unwrap(), vec![0.1 * DOWNLOADED, DOWNLOADED, 1.0]);

        let other = tmp.path().join("other");
        let z2 = Zotify::new(&program.to_string_lossy(), "--nothing", &spotify);
        let err = z2.download("abc", &other, &progress).await.unwrap_err();
        assert!(err.to_string().contains("can't play this song"), "{err:#}");

        let missing = Zotify::new("/no/such/zotify", "", &spotify);
        let err = missing
            .download("abc", &tmp.path().join("w3"), &progress)
            .await
            .unwrap_err();
        assert!(err.is::<NotInstalled>(), "{err:#}");
    }
}
