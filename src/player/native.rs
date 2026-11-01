//! The iPhone app's playback engine for local files and SoundCloud streams. iOS doesn't let an
//! app start other programs, so what mpv does on the desktops is done here inside the app:
//! symphonia decodes, rodio plays. It has mpv.rs's interface and sends the same events, so the
//! service drives it the same way (it is compiled as `player::mpv` on iOS).
//!
//! Streams are saved to a file first and played from there (an HLS stream as its segments
//! joined together), which also makes seeking work. Each instance has its own rodio sink on one
//! shared output, so two can play at once during a crossfade.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, Result};
use rodio::Source as _;
use serde_json::Value;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub enum MpvEvent {
    /// A new file started (playlist advanced, possibly gaplessly).
    StartFile {
        playlist_entry_id: i64,
    },
    FileLoaded,
    EndFile {
        reason: String,
        error: Option<String>,
        /// The playlist entry that ended (see [`MpvEvent::StartFile`]).
        playlist_entry_id: i64,
    },
    Pause(bool),
    Duration(f64),
    /// The player stopped working.
    Died,
}

#[derive(Debug, Clone)]
pub struct MpvOptions {
    pub binary: String,
    pub volume: f32,
    pub replaygain: bool,
    pub gapless: bool,
    pub audio_device: String,
    pub exclusive: bool,
}

/// Events from one player, tagged with its [`Mpv::id`] (crossfades run two at once).
pub type MpvSender = mpsc::UnboundedSender<(u64, MpvEvent)>;

/// Same levelling as mpv's on the desktops: ReplayGain tags lifted to Spotify's level, and
/// untagged songs (SoundCloud streams, mostly) turned down to it.
const REPLAYGAIN_PREAMP_DB: f32 = 4.0;
const REPLAYGAIN_FALLBACK_DB: f32 = -6.0;

/// How often the player thread looks at its sink: when a song ends, the next one is added
/// within this time, so songs follow each other almost without a gap.
const TICK: Duration = Duration::from_millis(15);

enum Cmd {
    Open {
        entry: i64,
        path: PathBuf,
        start: f64,
    },
    /// `after`: the entry it goes after; dropped if another song was asked for since.
    Append {
        entry: i64,
        after: i64,
        path: PathBuf,
    },
    ClearUpcoming,
    Pause(bool),
    Seek(f64),
    Volume(f32),
    ReplayGain(bool),
    Stop,
    Quit,
}

/// What the service may ask about the song playing.
#[derive(Default)]
struct Now {
    position: f64,
    codec: Option<&'static str>,
    rate: Option<u32>,
    bitrate: Option<f64>,
}

pub struct Mpv {
    id: u64,
    commands: std_mpsc::Sender<Cmd>,
    events: MpvSender,
    now: Arc<Mutex<Now>>,
    next_entry: AtomicI64,
    /// The entry playing (or asked for last with [`Mpv::load`]); the player thread moves it on
    /// when it continues into an appended song.
    playing: Arc<AtomicI64>,
    /// The download of the song asked for last, and of the one appended after it.
    loading: Mutex<Option<tokio::task::JoinHandle<()>>>,
    appending: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Mpv {
    /// Starts a player. `id` tags its events.
    pub async fn spawn(opts: &MpvOptions, id: u64, events: MpvSender) -> Result<Mpv> {
        let mixer = mixer().ok_or_else(|| anyhow!("no audio output"))?;
        let (commands, inbox) = std_mpsc::channel();
        let now = Arc::new(Mutex::new(Now::default()));
        let playing = Arc::new(AtomicI64::new(0));
        let player = Player {
            id,
            events: events.clone(),
            now: now.clone(),
            playing: playing.clone(),
            sink: rodio::Sink::connect_new(&mixer),
            volume: opts.volume,
            replaygain: opts.replaygain,
            current: None,
            upcoming: VecDeque::new(),
            gain: 1.0,
        };
        std::thread::Builder::new()
            .name(format!("player-{id}"))
            .spawn(move || player.run(inbox))?;
        Ok(Mpv {
            id,
            commands,
            events,
            now,
            next_entry: AtomicI64::new(1),
            playing,
            loading: Mutex::new(None),
            appending: Mutex::new(None),
        })
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    /// The few mpv properties the service asks for; anything else is an error, as from mpv.
    pub async fn command(&self, args: Value) -> Result<Value> {
        let now = self.now.lock().unwrap();
        match (args.get(0).and_then(Value::as_str), args.get(1).and_then(Value::as_str)) {
            (Some("get_property"), Some("time-pos")) => Ok(now.position.into()),
            (Some("get_property"), Some("audio-codec-name")) => now.codec.map(Value::from).ok_or_else(unavailable),
            (Some("get_property"), Some("audio-params/samplerate")) => {
                now.rate.map(Value::from).ok_or_else(unavailable)
            }
            (Some("get_property"), Some("audio-bitrate")) => now.bitrate.map(Value::from).ok_or_else(unavailable),
            _ => Err(unavailable()),
        }
    }

    /// Plays `url` (a file or a stream) from `start` seconds, replacing what plays.
    pub async fn load(&self, url: &str, start: f64) -> Result<()> {
        let entry = self.next_entry.fetch_add(1, Ordering::Relaxed);
        self.playing.store(entry, Ordering::Relaxed);
        let _ = self.events.send((
            self.id,
            MpvEvent::StartFile {
                playlist_entry_id: entry,
            },
        ));
        if let Some(task) = self.appending.lock().unwrap().take() {
            task.abort();
        }
        let task = self.fetch(url, entry, move |entry, path| Cmd::Open { entry, path, start });
        if let Some(old) = self.loading.lock().unwrap().replace(task) {
            old.abort();
        }
        Ok(())
    }

    /// iOS plays no YouTube videos (they need yt-dlp): the extra options are for those only.
    pub async fn load_with(&self, url: &str, start: f64, _extra: &[(&str, String)]) -> Result<()> {
        self.load(url, start).await
    }

    /// Adds `url` to play right after the current song.
    pub async fn append(&self, url: &str) -> Result<()> {
        let entry = self.next_entry.fetch_add(1, Ordering::Relaxed);
        let after = self.playing.load(Ordering::Relaxed);
        let task = self.fetch(url, entry, move |entry, path| Cmd::Append { entry, after, path });
        if let Some(old) = self.appending.lock().unwrap().replace(task) {
            old.abort();
        }
        Ok(())
    }

    /// Gets `url` ready to play (a stream is saved to a file first), then hands it to the
    /// player thread; a stream that can't be saved ends the entry with an error.
    fn fetch(
        &self,
        url: &str,
        entry: i64,
        cmd: impl FnOnce(i64, PathBuf) -> Cmd + Send + 'static,
    ) -> tokio::task::JoinHandle<()> {
        let url = url.to_owned();
        let (id, commands, events) = (self.id, self.commands.clone(), self.events.clone());
        tokio::spawn(async move {
            let path = if url.starts_with("http://") || url.starts_with("https://") {
                let path = stream_file(id, entry);
                if let Err(e) = crate::providers::soundcloud::save_stream(&crate::http::client(), &url, &path).await {
                    let _ = std::fs::remove_file(&path);
                    let _ = events.send((
                        id,
                        MpvEvent::EndFile {
                            reason: "error".into(),
                            error: Some(format!("{e:#}")),
                            playlist_entry_id: entry,
                        },
                    ));
                    return;
                }
                path
            } else {
                PathBuf::from(url.strip_prefix("file://").unwrap_or(&url))
            };
            let _ = commands.send(cmd(entry, path));
        })
    }

    /// Forgets the songs added with [`Mpv::append`].
    pub async fn clear_upcoming(&self) -> Result<()> {
        if let Some(task) = self.appending.lock().unwrap().take() {
            task.abort();
        }
        self.send(Cmd::ClearUpcoming)
    }

    pub async fn set_pause(&self, pause: bool) -> Result<()> {
        self.send(Cmd::Pause(pause))
    }

    pub async fn seek(&self, secs: f64) -> Result<()> {
        self.send(Cmd::Seek(secs.max(0.0)))
    }

    pub async fn set_volume(&self, volume: f32) -> Result<()> {
        self.send(Cmd::Volume(volume.clamp(0.0, 100.0)))
    }

    pub async fn set_replaygain(&self, on: bool) -> Result<()> {
        self.send(Cmd::ReplayGain(on))
    }

    pub async fn stop(&self) -> Result<()> {
        for task in [&self.loading, &self.appending] {
            if let Some(task) = task.lock().unwrap().take() {
                task.abort();
            }
        }
        self.send(Cmd::Stop)
    }

    pub async fn time_pos(&self) -> Option<f64> {
        Some(self.now.lock().unwrap().position)
    }

    pub async fn quit(self) {
        let _ = self.stop().await;
        let _ = self.commands.send(Cmd::Quit);
    }

    fn send(&self, cmd: Cmd) -> Result<()> {
        self.commands.send(cmd).map_err(|_| anyhow!("the player stopped"))
    }
}

fn unavailable() -> anyhow::Error {
    anyhow!("property unavailable")
}

/// Where a stream is saved while it plays. Each player keeps the files of its current and next
/// song; older ones are removed when a song starts.
fn stream_file(id: u64, entry: i64) -> PathBuf {
    stream_dir().join(format!("{id}-{entry}.audio"))
}

fn stream_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("sumo-streams");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// The one audio output every player mixes into. rodio's stream has to stay on the thread that
/// opened it, so that thread keeps it for the life of the app.
fn mixer() -> Option<rodio::mixer::Mixer> {
    static MIXER: OnceLock<Option<rodio::mixer::Mixer>> = OnceLock::new();
    MIXER
        .get_or_init(|| {
            let (tx, rx) = std_mpsc::channel();
            let opened = std::thread::Builder::new().name("audio-out".into()).spawn(move || {
                match rodio::OutputStreamBuilder::open_default_stream() {
                    Ok(mut stream) => {
                        stream.log_on_drop(false);
                        let _ = tx.send(Some(stream.mixer().clone()));
                        loop {
                            std::thread::park();
                        }
                    }
                    Err(e) => {
                        tracing::warn!("no audio output: {e}");
                        let _ = tx.send(None);
                    }
                }
            });
            opened.ok().and_then(|_| rx.recv().ok().flatten())
        })
        .clone()
}

/// A song ready to play.
struct Opened {
    source: rodio::source::Amplify<rodio::Decoder<std::io::BufReader<std::fs::File>>>,
    duration: Option<f64>,
    codec: Option<&'static str>,
    rate: u32,
    bitrate: Option<f64>,
}

fn open(path: &Path, replaygain: bool) -> Result<(Opened, f32)> {
    let file = std::fs::File::open(path)?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let decoder = rodio::Decoder::try_from(file).map_err(|e| anyhow!("can't decode it ({e})"))?;
    let tagged = lofty::read_from_path(path).ok();
    let duration = decoder
        .total_duration()
        .map(|d| d.as_secs_f64())
        .or_else(|| {
            use lofty::file::AudioFile;
            tagged.as_ref().map(|t| t.properties().duration().as_secs_f64())
        })
        .filter(|d| *d > 0.0);
    let rate = decoder.sample_rate();
    let codec = codec_of(path, tagged.as_ref());
    let bitrate = duration.map(|d| size as f64 * 8.0 / d);
    let gain = if replaygain { level(tagged.as_ref()) } else { 1.0 };
    Ok((
        Opened {
            source: decoder.amplify(gain),
            duration,
            codec,
            rate,
            bitrate,
        },
        gain,
    ))
}

/// The codec name mpv would report, for the quality badge.
fn codec_of(path: &Path, tagged: Option<&lofty::file::TaggedFile>) -> Option<&'static str> {
    use lofty::file::{FileType, TaggedFileExt};
    let kind = tagged.map(|t| t.file_type()).or_else(|| {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        FileType::from_ext(ext)
    })?;
    Some(match kind {
        FileType::Mpeg => "mp3",
        FileType::Flac => "flac",
        FileType::Mp4 => "aac",
        FileType::Vorbis => "vorbis",
        FileType::Wav => "pcm_s16le",
        FileType::Aiff => "pcm_s16be",
        _ => return None,
    })
}

/// The gain for a song: its ReplayGain tag lifted to Spotify's level (but not past its peak),
/// or the fallback for untagged songs.
fn level(tagged: Option<&lofty::file::TaggedFile>) -> f32 {
    use lofty::file::TaggedFileExt;
    use lofty::prelude::ItemKey;
    let number = |key: ItemKey| -> Option<f32> {
        let tag = tagged?.primary_tag().or_else(|| tagged?.first_tag())?;
        let text = tag.get_string(key)?;
        text.trim()
            .trim_end_matches("dB")
            .trim_end_matches("db")
            .trim()
            .parse()
            .ok()
    };
    match number(ItemKey::ReplayGainTrackGain) {
        Some(db) => {
            let gain = db_to_amp(db + REPLAYGAIN_PREAMP_DB);
            match number(ItemKey::ReplayGainTrackPeak).filter(|p| *p > 0.0) {
                Some(peak) => gain.min(1.0 / peak),
                None => gain,
            }
        }
        None => db_to_amp(REPLAYGAIN_FALLBACK_DB),
    }
}

fn db_to_amp(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// mpv's volume scale is cubic; the same here, so the slider sounds the same as on the desktops.
fn volume_factor(volume: f32) -> f32 {
    (volume / 100.0).clamp(0.0, 1.0).powi(3)
}

/// The thread that owns one sink.
struct Player {
    id: u64,
    events: MpvSender,
    now: Arc<Mutex<Now>>,
    playing: Arc<AtomicI64>,
    sink: rodio::Sink,
    volume: f32,
    replaygain: bool,
    /// The entry playing, with its file.
    current: Option<(i64, PathBuf)>,
    /// Appended songs, ready to play after it.
    upcoming: VecDeque<(i64, PathBuf, Opened)>,
    gain: f32,
}

impl Player {
    fn run(mut self, inbox: std_mpsc::Receiver<Cmd>) {
        self.sink.set_volume(volume_factor(self.volume));
        loop {
            match inbox.recv_timeout(TICK) {
                Ok(Cmd::Quit) | Err(std_mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(cmd) => self.handle(cmd),
                Err(std_mpsc::RecvTimeoutError::Timeout) => {}
            }
            self.tick();
        }
        self.sink.stop();
        self.forget_files(None);
    }

    fn emit(&self, event: MpvEvent) {
        let _ = self.events.send((self.id, event));
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Open { entry, path, start } => {
                self.sink.clear();
                self.upcoming.clear();
                match open(&path, self.replaygain) {
                    Ok((song, gain)) => {
                        self.gain = gain;
                        self.start(entry, path, song);
                        if start > 0.5 {
                            let _ = self.sink.try_seek(Duration::from_secs_f64(start));
                        }
                        self.sink.play();
                        self.emit(MpvEvent::Pause(false));
                    }
                    Err(e) => {
                        tracing::warn!("player: can't play {}: {e:#}", path.display());
                        self.current = None;
                        self.emit(MpvEvent::EndFile {
                            reason: "error".into(),
                            error: Some(format!("{e:#}")),
                            playlist_entry_id: entry,
                        });
                    }
                }
            }
            Cmd::Append { entry, after, path } => {
                // Appended after a song that has since been replaced: not wanted any more.
                if self.current.as_ref().map(|(e, _)| *e) != Some(after) {
                    remove_stream(&path);
                    return;
                }
                match open(&path, self.replaygain) {
                    Ok((song, _)) => self.upcoming.push_back((entry, path, song)),
                    Err(e) => tracing::debug!("player: can't open the next song: {e:#}"),
                }
            }
            Cmd::ClearUpcoming => {
                for (_, path, _) in self.upcoming.drain(..) {
                    remove_stream(&path);
                }
            }
            Cmd::Pause(pause) => {
                if pause {
                    self.sink.pause();
                } else {
                    self.sink.play();
                }
                self.emit(MpvEvent::Pause(pause));
            }
            Cmd::Seek(secs) => {
                if let Err(e) = self.sink.try_seek(Duration::from_secs_f64(secs)) {
                    tracing::debug!("player: can't seek: {e}");
                }
            }
            Cmd::Volume(volume) => {
                self.volume = volume;
                self.sink.set_volume(volume_factor(volume));
            }
            Cmd::ReplayGain(on) => self.replaygain = on,
            Cmd::Stop => {
                self.sink.clear();
                self.upcoming.clear();
                if let Some((entry, _)) = self.current.take() {
                    self.emit(MpvEvent::EndFile {
                        reason: "stop".into(),
                        error: None,
                        playlist_entry_id: entry,
                    });
                }
                self.forget_files(None);
            }
            Cmd::Quit => {}
        }
    }

    /// Starts `song` as entry `entry`.
    fn start(&mut self, entry: i64, path: PathBuf, song: Opened) {
        {
            let mut now = self.now.lock().unwrap();
            *now = Now {
                position: 0.0,
                codec: song.codec,
                rate: Some(song.rate),
                bitrate: song.bitrate,
            };
        }
        tracing::info!(
            "player: playing {} ({}, {} Hz)",
            path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
            song.codec.unwrap_or("?"),
            song.rate
        );
        self.sink.append(song.source);
        self.forget_files(Some(&path));
        self.playing.store(entry, Ordering::Relaxed);
        self.current = Some((entry, path));
        self.emit(MpvEvent::FileLoaded);
        if let Some(d) = song.duration {
            self.emit(MpvEvent::Duration(d));
        }
    }

    fn tick(&mut self) {
        let Some((entry, _)) = &self.current else { return };
        let entry = *entry;
        if !self.sink.empty() {
            self.now.lock().unwrap().position = self.sink.get_pos().as_secs_f64();
            return;
        }
        // The song ended: on to the appended one, if any, as mpv does.
        tracing::info!("player: entry {entry} ended");
        self.emit(MpvEvent::EndFile {
            reason: "eof".into(),
            error: None,
            playlist_entry_id: entry,
        });
        match self.upcoming.pop_front() {
            Some((next, path, song)) => {
                self.emit(MpvEvent::StartFile {
                    playlist_entry_id: next,
                });
                self.start(next, path, song);
                self.sink.play();
            }
            None => {
                self.current = None;
                self.forget_files(None);
            }
        }
    }

    /// Removes this player's saved streams except `keep` and the upcoming ones.
    fn forget_files(&self, keep: Option<&Path>) {
        let prefix = format!("{}-", self.id);
        let Ok(entries) = std::fs::read_dir(stream_dir()) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let ours = entry.file_name().to_string_lossy().starts_with(&prefix);
            let kept = Some(path.as_path()) == keep || self.upcoming.iter().any(|(_, p, _)| *p == path);
            if ours && !kept {
                remove_stream(&path);
            }
        }
    }
}

/// Removes a saved stream (never a local file: those aren't in the streams folder).
fn remove_stream(path: &Path) {
    if path.starts_with(stream_dir()) {
        let _ = std::fs::remove_file(path);
    }
}

/// Players a previous run left behind: on iOS only saved streams.
pub fn stop_orphans() {
    let _ = std::fs::remove_dir_all(stream_dir());
}

/// iOS picks the output itself (speaker, headphones, AirPlay, Bluetooth).
pub async fn list_audio_devices(_binary: &str) -> Vec<(String, String)> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels() {
        assert!((db_to_amp(-6.0) - 0.501).abs() < 0.01);
        assert_eq!(volume_factor(100.0), 1.0);
        assert!((volume_factor(50.0) - 0.125).abs() < 1e-6);
        assert_eq!(level(None), db_to_amp(REPLAYGAIN_FALLBACK_DB));
    }
}
