//! Playlists keep a media server's songs by where they were (`server:<server id>:<path>`).
//! When the same files turn up somewhere else (the drive plugged into this computer, the
//! server set up again or its drive mounted at another path), those entries would stay dead:
//! here they're pointed at the same songs where they are now, so nothing is added again by
//! hand.

use std::collections::{HashMap, HashSet};

use super::Library;
use crate::model::{Playlist, Source, Track};

/// The playlists that change, and how many of their songs were found again.
///
/// A playlist's server song goes to a file of this device's library when it's there (it
/// plays without the server). Otherwise, with a server connected (`live`: its ID and the IDs
/// of the songs it has now), one it no longer has goes to the same song it has now. The
/// server's own playlist ("On <server>") is left alone: it's the server's list.
pub fn relink(lib: &Library, live: Option<(&str, &HashSet<String>)>) -> (Vec<Playlist>, usize) {
    let mut local: HashMap<String, Vec<&Track>> = HashMap::new();
    let mut on_server: HashMap<String, Vec<&Track>> = HashMap::new();
    for t in lib.tracks.values() {
        match t.source {
            Source::Local => local.entry(file_name(&t.uri)).or_default().push(t),
            Source::Server if live.is_some_and(|(_, ids)| ids.contains(&t.id)) => {
                on_server.entry(file_name(&t.uri)).or_default().push(t)
            }
            _ => {}
        }
    }
    let mut changed = Vec::new();
    let mut found = 0;
    for p in lib.playlists.iter().filter(|p| !p.id.starts_with("server:")) {
        let had: HashSet<&String> = p.track_ids.iter().collect();
        let mut new_ids: HashSet<String> = HashSet::new();
        let mut ids = Vec::with_capacity(p.track_ids.len());
        let mut moved = 0;
        for id in &p.track_ids {
            if !id.starts_with("server:") {
                ids.push(id.clone());
                continue;
            }
            // Gone from the library altogether: its path is still in its ID.
            let stub;
            let old = match lib.tracks.get(id.as_str()) {
                Some(t) => t,
                None => {
                    stub = stub_of(id);
                    &stub
                }
            };
            let gone = live.is_some_and(|(_, ids)| !ids.contains(id.as_str()));
            let twin = twin_of(old, &local).or_else(|| if gone { twin_of(old, &on_server) } else { None });
            match twin {
                Some(t) => {
                    moved += 1;
                    // Already in the playlist where it is now: once.
                    if !had.contains(&t.id) && new_ids.insert(t.id.clone()) {
                        ids.push(t.id.clone());
                    }
                }
                None => ids.push(id.clone()),
            }
        }
        if moved == 0 {
            continue;
        }
        found += moved;
        changed.push(Playlist {
            track_ids: ids,
            ..p.clone()
        });
    }
    (changed, found)
}

/// The same song among `by_name` (songs by their file name): the one whose folders match
/// furthest up; with only the file name in common, its length or its title and artist have
/// to match too.
fn twin_of<'a>(old: &Track, by_name: &HashMap<String, Vec<&'a Track>>) -> Option<&'a Track> {
    let mut best: Option<(&Track, usize)> = None;
    for &t in by_name.get(&file_name(&old.uri))? {
        let depth = common_tail(&old.uri, &t.uri);
        if depth < 2 && !same_length(old, t) && !same_tags(old, t) {
            continue;
        }
        if best.is_none_or(|(_, d)| depth > d) {
            best = Some((t, depth));
        }
    }
    best.map(|(t, _)| t)
}

fn parts(path: &str) -> impl DoubleEndedIterator<Item = &str> {
    path.split(['/', '\\']).filter(|s| !s.is_empty())
}

fn file_name(path: &str) -> String {
    parts(path).next_back().unwrap_or(path).to_lowercase()
}

/// How many of the paths' last parts are the same (the file name, its folder, …).
fn common_tail(a: &str, b: &str) -> usize {
    parts(a)
        .rev()
        .zip(parts(b).rev())
        .take_while(|(x, y)| x.eq_ignore_ascii_case(y) || x.to_lowercase() == y.to_lowercase())
        .count()
}

fn same_length(a: &Track, b: &Track) -> bool {
    a.duration_ms > 0 && b.duration_ms > 0 && a.duration_ms.abs_diff(b.duration_ms) <= 2000
}

fn same_tags(a: &Track, b: &Track) -> bool {
    let norm = |s: &str| s.trim().to_lowercase();
    !a.title.trim().is_empty()
        && !a.artist.trim().is_empty()
        && norm(&a.title) == norm(&b.title)
        && norm(&a.artist) == norm(&b.artist)
}

/// A server song known only by its ID (`server:<server id>:<path>`).
fn stub_of(id: &str) -> Track {
    let path = id.splitn(3, ':').nth(2).unwrap_or_default();
    Track {
        id: id.to_string(),
        source: Source::Server,
        title: String::new(),
        artist: String::new(),
        album: String::new(),
        duration_ms: 0,
        track_no: None,
        art: None,
        uri: path.to_string(),
        added_at: 0,
        album_artist: String::new(),
        disc_no: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PlaylistKind;

    fn track(id: &str, source: Source, uri: &str, title: &str, ms: u64) -> Track {
        Track {
            title: title.into(),
            artist: "A".into(),
            duration_ms: ms,
            source,
            ..stub_of(id)
        }
        .with_uri(uri)
    }

    impl Track {
        fn with_uri(mut self, uri: &str) -> Track {
            self.uri = uri.into();
            self
        }
    }

    fn library(tracks: Vec<Track>, lists: Vec<(&str, Vec<&str>)>) -> Library {
        let mut lib = Library::default();
        for t in tracks {
            lib.tracks.insert(t.id.clone(), t);
        }
        for (id, ids) in lists {
            lib.playlists.push(Playlist {
                id: id.into(),
                name: id.into(),
                kind: PlaylistKind::Custom,
                remote_id: None,
                description: String::new(),
                art: None,
                track_ids: ids.into_iter().map(String::from).collect(),
            });
        }
        lib
    }

    #[test]
    fn the_drive_plugged_into_this_computer() {
        let lib = library(
            vec![
                track(
                    "server:S:/mnt/usb/Music/Band/One.flac",
                    Source::Server,
                    "/mnt/usb/Music/Band/One.flac",
                    "One",
                    200_000,
                ),
                track(
                    "server:S:/mnt/usb/Music/Band/Two.flac",
                    Source::Server,
                    "/mnt/usb/Music/Band/Two.flac",
                    "Two",
                    100_000,
                ),
                track(
                    "server:S:/mnt/usb/Music/Band/Gone.flac",
                    Source::Server,
                    "/mnt/usb/Music/Band/Gone.flac",
                    "Gone",
                    100_000,
                ),
                track(
                    "local:/run/media/me/USB/Music/Band/One.flac",
                    Source::Local,
                    "/run/media/me/USB/Music/Band/One.flac",
                    "One",
                    200_000,
                ),
                // Renamed folder, same file name and length.
                track(
                    "local:/run/media/me/USB/Music/Other/Two.flac",
                    Source::Local,
                    "/run/media/me/USB/Music/Other/Two.flac",
                    "2",
                    100_500,
                ),
                // Same file name only: another song.
                track("local:/x/Gone.flac", Source::Local, "/x/Gone.flac", "Else", 300_000),
                track("spotify:track:1", Source::Spotify, "spotify:track:1", "S", 1),
            ],
            vec![
                (
                    "custom:1",
                    vec![
                        "spotify:track:1",
                        "server:S:/mnt/usb/Music/Band/One.flac",
                        "server:S:/mnt/usb/Music/Band/Two.flac",
                        "server:S:/mnt/usb/Music/Band/Gone.flac",
                    ],
                ),
                ("server:S", vec!["server:S:/mnt/usb/Music/Band/One.flac"]),
                ("custom:2", vec!["spotify:track:1"]),
            ],
        );
        let (changed, found) = relink(&lib, None);
        assert_eq!(found, 2);
        assert_eq!(
            changed.len(),
            1,
            "only the playlist with server songs, never the server's own"
        );
        assert_eq!(
            changed[0].track_ids,
            [
                "spotify:track:1",
                "local:/run/media/me/USB/Music/Band/One.flac",
                "local:/run/media/me/USB/Music/Other/Two.flac",
                "server:S:/mnt/usb/Music/Band/Gone.flac"
            ]
        );
        // Done once: nothing left to move.
        let mut lib = lib;
        lib.playlists[0] = changed[0].clone();
        assert_eq!(relink(&lib, None).1, 0);
    }

    #[test]
    fn the_server_has_the_songs_at_another_path() {
        let new = "server:S:/media/disk/Music/Band/One.flac";
        let lib = library(
            vec![
                track(new, Source::Server, "/media/disk/Music/Band/One.flac", "One", 200_000),
                track(
                    "server:S:/media/disk/Music/Band/Stays.flac",
                    Source::Server,
                    "/media/disk/Music/Band/Stays.flac",
                    "Stays",
                    1,
                ),
            ],
            // The old one isn't in the library any more (only its ID is left), nor is the
            // other server's.
            vec![(
                "custom:1",
                vec![
                    "server:S:/mnt/usb/Music/Band/One.flac",
                    "server:OLD:/mnt/usb/Music/Band/One.flac",
                    "server:S:/media/disk/Music/Band/Stays.flac",
                ],
            )],
        );
        let live: HashSet<String> = [
            new.to_string(),
            "server:S:/media/disk/Music/Band/Stays.flac".to_string(),
        ]
        .into();
        // Not connected: nothing is known to be gone.
        assert_eq!(relink(&lib, None).1, 0);
        let (changed, found) = relink(&lib, Some(("S", &live)));
        assert_eq!(found, 2);
        // Both were the same song: in the playlist once.
        assert_eq!(
            changed[0].track_ids,
            [new, "server:S:/media/disk/Music/Band/Stays.flac"]
        );
    }
}
