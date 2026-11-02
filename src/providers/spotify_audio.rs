//! Spotify songs saved straight from Spotify: the same Ogg Vorbis file Utako's Spotify player
//! streams (320 kbps with Premium), fetched and decrypted by librespot through Utako's own
//! Spotify connection, and written out without the header Spotify puts in front of it.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use librespot_audio::{AudioDecrypt, AudioFile};
use librespot_core::{session::Session, FileId, SpotifyId, SpotifyUri};
use librespot_metadata::audio::{AudioFileFormat, AudioFiles, AudioItem};

use crate::downloader::Progress;

/// Where the Ogg data begins in Spotify's files: before it is Spotify's own header (with its
/// loudness data), which players don't understand.
const OGG_START: usize = 0xa7;

/// The files tried, best first: those Spotify's own player and librespot use.
const FORMATS: [AudioFileFormat; 3] = [
    AudioFileFormat::OGG_VORBIS_320,
    AudioFileFormat::OGG_VORBIS_160,
    AudioFileFormat::OGG_VORBIS_96,
];

/// Saves Spotify song `id` (base62) in `dir` as `<id>.ogg` and returns the file.
pub async fn download(session: &Session, id: &str, dir: &Path, progress: Progress<'_>) -> Result<PathBuf> {
    let uri = SpotifyUri::from_uri(&format!("spotify:track:{id}")).map_err(|e| anyhow!("not a Spotify song ({e})"))?;
    let track_id = SpotifyId::try_from(&uri).map_err(|e| anyhow!("not a Spotify song ({e})"))?;
    let item = AudioItem::get_file(session, uri)
        .await
        .map_err(|e| anyhow!("Spotify didn't find the song: {e}"))?;
    let item = playable(session, item)
        .await
        .ok_or_else(|| anyhow!("Spotify can't play this song here (not available in your country?)"))?;
    let (format, file_id) = pick(&item.files).ok_or_else(|| anyhow!("Spotify has no Ogg Vorbis file of this song"))?;
    // As Utako's Spotify player does: the key is asked for with the song asked for, also when
    // the file is an alternative's (the same recording, released again).
    let key = session
        .audio_key()
        .request(track_id, file_id)
        .await
        .map_err(|e| anyhow!("Spotify didn't give the key to the song's file: {e}"))?;
    let file = AudioFile::open(session, file_id, bytes_per_second(format))
        .await
        .map_err(|e| anyhow!("couldn't get the song's file from Spotify: {e}"))?;
    let controller = file
        .get_stream_loader_controller()
        .map_err(|e| anyhow!("couldn't get the song's file from Spotify: {e}"))?;
    let size = controller.len();
    // All of it at once, as fast as Spotify sends it rather than at playback speed.
    let _ = controller.fetch_next_and_wait(size, 0);
    tracing::info!("saving {id} from Spotify: {format:?}, {size} bytes");

    let out = dir.join(format!("{id}.ogg"));
    let target = out.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<f32>();
    // librespot's files are read the way a player reads them: blocking until the data is in.
    let job = tokio::task::spawn_blocking(move || {
        save_ogg(AudioDecrypt::new(Some(key), file), &target, size, |done| {
            tx.send(done).is_ok()
        })
    });
    while let Some(done) = rx.recv().await {
        progress(done);
    }
    job.await.context("the download stopped")??;
    Ok(out)
}

/// The song itself, or the first playable alternative (the same recording released again)
/// when the song has no files, as librespot's player picks.
async fn playable(session: &Session, item: AudioItem) -> Option<AudioItem> {
    if item.availability.is_err() {
        return None;
    }
    if !item.files.is_empty() {
        return Some(item);
    }
    for alternative in item.alternatives.iter().flat_map(|a| a.iter()) {
        if let Ok(alt) = AudioItem::get_file(session, alternative.clone()).await {
            if alt.availability.is_ok() && !alt.files.is_empty() {
                return Some(alt);
            }
        }
    }
    None
}

/// The best Ogg Vorbis file there is.
fn pick(files: &AudioFiles) -> Option<(AudioFileFormat, FileId)> {
    FORMATS
        .iter()
        .find_map(|format| files.get(format).map(|id| (*format, *id)))
}

/// How fast a file plays, which librespot reads ahead by.
fn bytes_per_second(format: AudioFileFormat) -> usize {
    let kbps = match format {
        AudioFileFormat::OGG_VORBIS_320 => 40,
        AudioFileFormat::OGG_VORBIS_160 => 20,
        _ => 12,
    };
    kbps * 1024
}

/// Writes Spotify's file as a plain Ogg file: without its header, and only when what follows
/// is Ogg (it isn't when the key was wrong). `sent` hears the fraction done, and says whether
/// to go on.
fn save_ogg(mut reader: impl Read, out: &Path, size: usize, sent: impl Fn(f32) -> bool) -> Result<()> {
    let mut header = [0u8; OGG_START];
    let mut magic = [0u8; 4];
    reader
        .read_exact(&mut header)
        .and_then(|()| reader.read_exact(&mut magic))
        .context("the song's file from Spotify ended too soon")?;
    if &magic != b"OggS" {
        bail!("the song's file from Spotify isn't Ogg Vorbis (wrong key?)");
    }
    let mut file = std::fs::File::create(out).with_context(|| format!("couldn't create {}", out.display()))?;
    file.write_all(&magic)?;
    let mut done = OGG_START + magic.len();
    let mut buf = vec![0u8; 64 << 10];
    loop {
        let n = reader.read(&mut buf).context("the download from Spotify broke off")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .with_context(|| format!("couldn't write {}", out.display()))?;
        done += n;
        if !sent((done as f32 / size.max(1) as f32).min(1.0)) {
            bail!("stopped");
        }
    }
    if done < size {
        bail!("the download from Spotify stopped at {done} of {size} bytes");
    }
    file.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::sync::Mutex;

    #[test]
    fn the_best_ogg_file_is_picked() {
        let id = |n: u8| FileId::from(&[n; 20][..]);
        let files = AudioFiles(HashMap::from([
            (AudioFileFormat::MP3_320, id(1)),
            (AudioFileFormat::OGG_VORBIS_96, id(2)),
            (AudioFileFormat::OGG_VORBIS_160, id(3)),
        ]));
        assert_eq!(pick(&files), Some((AudioFileFormat::OGG_VORBIS_160, id(3))));
        let mp3_only = AudioFiles(HashMap::from([(AudioFileFormat::MP3_320, id(1))]));
        assert_eq!(pick(&mp3_only), None);
    }

    fn spotify_file(body: &[u8]) -> Vec<u8> {
        let mut data = vec![0x5a; OGG_START];
        data.extend_from_slice(body);
        data
    }

    #[test]
    fn spotify_files_become_plain_ogg() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("song.ogg");
        let body: Vec<u8> = b"OggS"
            .iter()
            .copied()
            .chain((0..200_000u32).map(|i| i as u8))
            .collect();
        let data = spotify_file(&body);
        let seen = Mutex::new(Vec::new());
        save_ogg(Cursor::new(&data), &out, data.len(), |p| {
            seen.lock().unwrap().push(p);
            true
        })
        .unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), body);
        let seen = seen.into_inner().unwrap();
        assert!(seen.len() > 1, "{seen:?}");
        assert_eq!(seen.last(), Some(&1.0));
    }

    #[test]
    fn what_isnt_a_whole_ogg_file_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("song.ogg");
        // Decrypted with the wrong key.
        let garbage = spotify_file(b"\x13\x37garbage");
        let err = save_ogg(Cursor::new(&garbage), &out, garbage.len(), |_| true).unwrap_err();
        assert!(err.to_string().contains("isn't Ogg Vorbis"), "{err:#}");
        // Cut short.
        let short = spotify_file(b"Og");
        assert!(save_ogg(Cursor::new(&short), &out, short.len(), |_| true).is_err());
        let partial = spotify_file(b"OggS and a bit");
        let err = save_ogg(Cursor::new(&partial), &out, partial.len() + 1000, |_| true).unwrap_err();
        assert!(err.to_string().contains("stopped at"), "{err:#}");
        // Cancelled.
        let whole = spotify_file(b"OggS and the rest");
        assert!(save_ogg(Cursor::new(&whole), &out, whole.len(), |_| false).is_err());
    }
}
