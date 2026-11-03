<p align="center"><img src="assets/icon-256.png" width="128" alt="Utako logo"></p>

# Utako

A fast, lightweight, native music player (called MultiMusic until version 0.4 and Sumo until 0.9) for **Windows, macOS and Arch Linux**, with a **beta for iPhone and iPad**, that puts **your local files, Spotify, SoundCloud, music videos and your media server** in one place, with **synced lyrics**, **Discord Rich Presence**, **Last.fm scrobbling** and media-key / MPRIS support.

It's written in Rust with [egui](https://github.com/emilk/egui), like [Spotifast](https://spotifast.rocks/). There's no Electron and no web view, so it starts instantly and stays small in memory.

![Home](docs/screenshots/home.png)

| Search (songs + artists everywhere) | Playlist |
| --- | --- |
| ![Search](docs/screenshots/search.png) | ![Playlist](docs/screenshots/playlist.png) |
| **Artist page** | **Full-screen lyrics** |
| ![Artist](docs/screenshots/artist.png) | ![Now playing](docs/screenshots/now-playing.png) |

## Features

- **One library for every source**
  - **Local files**: MP3, FLAC, Opus, OGG, M4A/ALAC, WAV, AIFF, APE, WavPack and more. Tags and embedded or folder cover art are read automatically, and rescans are incremental. **Edit a file's details** (title, artist, album, album artist, genre, year, track and disc numbers, lyrics and cover) from its right-click menu (*Edit details…*), for one song or many at once; *Look up on Spotify* fills them in from Spotify, and a cover can come from an image file, a link or an image dropped on the window. Untagged files get their artist, title and track number from the file name (`Artist - Title.mp3`, `01 - Artist - Title [Free DL].mp3`, `artist_-_title.opus`, `Title [id].opus` as downloaders name them) and their artist and album from folders like `Artist/Album/` or `Artist - Album (2019)/`. Tags always win.
  - **Spotify**: imports all your playlists and Liked Songs and plays them in the app through [librespot](https://github.com/librespot-org/librespot) (Spotify Premium is needed for playback). Gapless playback, 96/160/320 kbps, volume normalisation.
  - **SoundCloud**: imports your likes and playlists (your own and liked ones), searches the catalogue and plays streams.
  - **M3U/M3U8** playlist import and export.
- **Search everything at once**: one search box finds songs, artists, **albums** and **public playlists** in your library, on Spotify and on SoundCloud (artist profiles included, so small SoundCloud artists show up too). Results appear per service as soon as each one answers, Spotify and SoundCloud artists first; press **Enter** on an artist's exact name to jump straight to their page. Click an album or playlist to open it, play it, download it or save it.
- **Music videos**: search has a *Music videos* section, from YouTube. A video plays in the middle of the window (never over the sidebar or the player) in a player like Aniko's: the title at the top, a seek bar and round buttons at the bottom that fade while it plays, and a click on the picture to pause. Its back arrow returns to your page while the song plays on; the screen button in the player brings the video back. Music videos go into playlists, Liked Songs and the queue like any song, and **download** like one: as a video file (up to 1080p, MP4 when YouTube has it), to this device or, with *Send downloads to the media server* on, into a *Music videos* folder on the media server (its music list shows them once mediaserver lists video files). **Video files** in your music folders (MP4, M4V, MKV, WebM, MOV) play the same way. Utako talks to YouTube's own API the way YouTube's apps do (the visionOS app's client, the one yt-dlp uses when it can't run JavaScript): no YouTube account, no yt-dlp, nothing to install but ffmpeg (which comes with mpv; the Windows and macOS apps include it). Videos YouTube only shows to signed-in adults don't play this way, and say so. *Settings → Playback → Open music videos in the player when they start* decides whether a video opens by itself.
- **Albums**: the Albums page shows the albums in your music folders together with the Spotify and SoundCloud albums you save, with tabs for each (*All*, *Local files*, *Spotify*, *SoundCloud*). Open any Spotify or SoundCloud album and click **Save album** to add it; Spotify albums are also saved to your Spotify library when you're logged in, and the albums you've saved on Spotify come in when Spotify syncs. Saved albums open from the Albums page even offline, and **Remove** takes them out again (from Spotify's library too).
- **Artist pages and links**: click any artist name to see all their songs in your library, or open their Spotify / SoundCloud page. Artist pages bring Spotify and SoundCloud together: a Spotify artist also shows their SoundCloud uploads, a SoundCloud profile also shows their Spotify releases, and a library artist gets a *More on Spotify & SoundCloud* section. The same song is shown once (a single that is also on an album, or a song on both services), while remixes, live and slowed versions stay. Profiles are matched by exact name only, so you never get someone else's songs. **Paste a link** into the search box to open it, e.g. `https://open.spotify.com/artist/…`, `https://soundcloud.com/someone`, a SoundCloud set, a `spotify:` URI, or a `spotify.link` / `on.soundcloud.com` short link. Any page can be saved as a Utako playlist.
- **Custom playlists that mix sources**: drop a Spotify song, a SoundCloud upload and a FLAC into the same playlist. **Copy and paste songs** between lists: in any song list (a Spotify or SoundCloud playlist, an album, search results) press **Ctrl+A** (or Ctrl/Shift-click songs) and **Ctrl+C**, open one of your playlists and press **Ctrl+V**. Pasting Spotify or SoundCloud links (songs, albums or playlists, e.g. copied in the Spotify app) or music files from a file manager works too. In your playlists, **Delete** removes the selected songs and **Ctrl+X** cuts them. A cross-source **Liked Songs** collection (♥) also syncs likes back to Spotify and Last.fm.
- **Synced lyrics**: reads `.lrc` files next to your music, then embedded lyrics tags, then [LRCLIB](https://lrclib.net), and finally [Genius](https://genius.com) (plain lyrics) for songs LRCLIB doesn't have. Lyrics are shown in a side panel and in a full-screen *Now playing* view with a large cover. Click a line to jump to it.
- **Discord Rich Presence** shows "Listening to <song>" with the album cover, a progress bar and an "Open in Spotify/SoundCloud" button. Works with the Discord app, Vesktop and arRPC.
- **Crossfade** (Settings → Playback, up to 12 s) between any two sources: local file into Spotify, Spotify into SoundCloud, Spotify into Spotify, and so on. The next song starts on a second player while the current one fades out. Songs of the same album stay gapless unless you tick *Also crossfade between songs of the same album*. Skipping, seeking or pausing during a fade ends it right away.
- **Downloads from every service**: right-click a song and choose *Download*, use the download button on a playlist, album, artist or profile page, or the one next to the song in *Now playing*.
  - **SoundCloud** songs download directly: the uploader's original file when they allow it (often WAV or FLAC), otherwise the stream SoundCloud plays.
  - **Spotify** songs are saved straight from Spotify, through your Spotify login in Utako: the same Ogg Vorbis file Utako's Spotify player streams (320 kbps with Premium), fetched and decrypted by [librespot](https://github.com/librespot-org/librespot) like when it plays, never a lookalike from another site. Nothing else to install. Songs Utako has played are often saved at once from its Spotify cache. Tick *Save Spotify downloads as MP3* if your devices don't play Ogg (not on iOS). Saving Spotify's decrypted audio isn't allowed by Spotify's terms, and an account could be banned for it. While Utako isn't logged in to Spotify, the same recording is found on SoundCloud instead (the upload with the same title, artist and length), then tagged with the song's details.
  - **Full metadata**: every file is tagged with title, all artists, album, album artist, track and disc number (with totals), release date, ISRC, label, copyright, genre (SoundCloud), a link to the song and the full-size cover. For Spotify songs these come from Spotify itself. Lyrics from LRCLIB (time-synced when available) or Genius are embedded too. SoundCloud uploads get their "Artist - Title" names cleaned up and the album and release date from Genius.
  - Files are named "Artist - Title" and go to a *SoundCloud*, *Spotify* or *Music videos* folder in your first library folder, so they also show up in *Local Files*; choose another folder under *Settings → Downloads*. Downloaded songs play from the file, even offline, and the **Downloads** page in the sidebar shows progress (cancel one song or all), failures (with *Try again*) and everything saved earlier. Unfinished downloads never land in your library: each one works in its own hidden folder until it is done.
  - **To the media server instead**: with *Settings → Downloads → Send downloads to the media server* on, songs you download go into the [media server](#media-server)'s music (a *Spotify*, *SoundCloud* or *Music videos* folder in its first music folder) instead of on this device, while it's connected and lets this device upload. They then play from its "On <server>" playlist on all your devices. When it isn't connected, downloads are saved here as usual.
- **Last.fm scrobbling**: by default every song scrobbles the moment it starts playing. Untick *Scrobble as soon as a song starts* to use Last.fm's usual rule instead (half the song or 4 minutes). Also sends "now playing" updates, and queues scrobbles offline to send later. SoundCloud uploads are scrobbled as the song they are: "Artist - Title [Free DL]" titles are cleaned up, Genius supplies the real artist, title and album (re-upload channels like "Nightcore …" get the original artist), and songs still without an album get the one Last.fm knows, so apps like .fmbot show the cover. Local files without tags are looked up on Spotify by their file name and scrobbled with Spotify's artist, title and album (or Last.fm's album instead of the folder's name). If Last.fm ignores a scrobble (e.g. a file tagged "Unknown Artist"), Utako says why.
- **Your Last.fm profile**: once your account is connected, your profile picture and name sit at the bottom of the sidebar (click them for your stats; the gear next to them opens Settings), and Home ends with your week on Last.fm. The profile page shows your scrobbles, artists, albums, tracks and daily average, a **listening chart** (the last 7 or 30 days by day, the last 12 months by month, or every year since you joined), your **top artists, albums and tracks**, your **top genres** (from the tags on your top artists) and your **recent scrobbles**, including what's playing now and loved songs. Each section has its own menu: **Today, the last 7 or 30 days, 3, 6 or 12 months, or all time** (Last.fm has no "today", so Utako counts the day's scrobbles itself). Click an artist to open them in your library (or search for them), and an album or song to search for it. The stats come from Last.fm with your API key; a profile with a private listening history can't be shown.
- **MPRIS / media keys**: works with `playerctl`, waybar, polybar, KDE/GNOME media widgets and headset buttons.
- **Your playlists, your way**: right-click one of your playlists (or click its picture on its page) and choose *Edit details…* to change its name, description and picture: an image file, a link, or an image dropped on the window.
- **Its own look**: graphite and off-white like the record logo, rounded "tile" panels, a floating player dock, header cards that glow in the colours of the cover, and soft hover and page transitions. **Drag the edges** of the sidebar and the lyrics / queue panel to resize them (sizes are remembered); drag the sidebar narrow and it becomes a strip of icons and covers. Also: a queue, back / forward navigation, albums and artists grids, gapless playback, ReplayGain, session restore, and drag and drop (drop a folder or an `.m3u` onto the window). **Themes** (the Themes button above Settings): eight color themes shared with Kumo (Utako, Midnight, Forest, Sakura, Ember, Ocean, Dusk and true Black), any accent colour, and holiday themes that come on by themselves in their month (New Year, Valentine's Day, cherry blossoms, a summer festival, Halloween, Christmas) with their own colour, decorations and an animation (fireworks, hearts, petals, fireflies, bats, snow). Pick *Off* or one holiday all year, and switch the animation off on its own; it only moves while Utako's window has focus.
- **Queue and more**: the queue shows *Queued by you* apart from *Next from <playlist>*; drag songs to reorder them, and clear either part. In a narrow window the queue and lyrics open over the page. A **sleep timer** (the moon in the player) pauses after 15 minutes to 1½ hours, or after the current song. Home shows your **most played** songs and **recently added** albums. Albums with an album artist (compilations) stay together, multi-disc albums play disc by disc, and the Albums page sorts by artist, title or date added. The window opens at the size you left it, and starting Utako again brings the running one to the front. On Linux, Utako can show a desktop notification when a song starts (Settings → Appearance).
- **Low memory use**: cover art is decoded at the size it's drawn and kept in a small LRU cache, fonts are memory-mapped from your system, and only two runtime threads are used. The window doesn't redraw at all while nothing changes.

## Install on Windows or macOS

**[Download the newest version](https://github.com/v0-0x/Utako/releases/latest)** (under *Assets* on that page):

- **Windows 10/11 (64-bit):** `Utako-Setup-<version>-x64.exe`. It installs for your user (no administrator prompt), with everything Utako needs: mpv for playback, ffmpeg for music videos and downloads, and the Inter font. Start menu entry and an optional desktop shortcut included; uninstall from *Settings → Apps*. Your settings stay in `%APPDATA%\multimusic`.
- **macOS 11 or newer:** `Utako-<version>-macos-arm64.dmg` for Apple Silicon (M1/M2/M3/M4) or `Utako-<version>-macos-intel.dmg` for Intel Macs. Open it and drag Utako to Applications; mpv and ffmpeg are inside the app. Utako isn't notarized by Apple (that needs a paid developer account), so the first time macOS asks: right-click Utako → **Open** → **Open**, or *System Settings → Privacy & Security → Open Anyway*.

**Updates install themselves:** Utako looks for a new release when it starts (and every few hours), shows a bar when one is out, and **Update now** downloads it, checks its SHA-256 checksum, installs it and starts the new version. Turn this off or check by hand under *Settings → Updates*. No GitHub account is needed.

Moving from another computer? Export your settings there and import them here (see [Back up or move your settings](#back-up-or-move-your-settings)).

The installers are built by GitHub Actions ([`.github/workflows/release.yml`](.github/workflows/release.yml)) from this repository: the Windows installer with Inno Setup, the macOS apps for Apple Silicon and Intel. Releases come from the version in `Cargo.toml`.

## Install on iPhone or iPad (beta)

Each release has `Utako-<version>-ios.ipa` (iOS 15 or newer). It isn't on the App Store and isn't signed, so you install it yourself:

- **[Sideloadly](https://sideloadly.io)** or **[AltStore](https://altstore.io)** on a Mac or PC: connect the iPhone, drop the `.ipa` in and sign in with your Apple ID. With a free Apple ID the app has to be signed again every 7 days (AltStore can do it by itself over Wi-Fi), and iOS asks you to trust your Apple ID once under *Settings → General → VPN & Device Management*. iOS 16 and newer also need *Settings → Privacy & Security → Developer Mode* on.
- **TrollStore**, on the iOS versions it supports, installs it for good.

What's different on iOS:

- **Your music** goes in the Files app under **On My iPhone → Utako** (copy it from a computer with Finder or iTunes file sharing, or from iCloud Drive), then *Settings → Local library → Rescan now*.
- **Spotify, SoundCloud, Last.fm and lyrics** work as on the desktop. The easiest way to bring your logins over: on your computer use *Settings → Back up or move your settings → Export*, put the file in the same Utako folder in Files, and import it on the phone.
- Local files and SoundCloud play through Utako's own player (iOS doesn't allow mpv): MP3, AAC and ALAC, FLAC, Ogg Vorbis, WAV and AIFF. Opus files and SoundCloud's Opus streams don't play there.
- Music keeps playing with the screen locked, and the lock screen, Control Center and headphone buttons control it.
- Not on iOS: music videos, Discord status, choosing the output device (iOS picks it: speaker, headphones, Bluetooth or AirPlay) and updating itself (Utako says when a new version is out; install its `.ipa` the same way).

The app is built by [`.github/workflows/ios.yml`](.github/workflows/ios.yml), which also starts it in the iOS simulator and plays a test song before a release is published.

## Install (Arch Linux)

```sh
sudo pacman -S --needed base-devel git rust mpv
git clone https://github.com/v0-0x/Utako.git sumo-music
cd sumo-music
makepkg -si          # builds and installs the `multimusic` package
```

Then launch **Utako** from your app launcher, or run `utako` (`sumo` works too). The `PKGBUILD` also works on its own (in a clean chroot with `extra-x86_64-build`, or with an AUR helper): outside the repository it builds the release tagged `v<version>`.

**Updates:** when a new version is out, Utako shows a bar with **Update now**. It downloads the release's ready-made package (built by GitHub Actions for x86_64), checks its SHA-256 checksum, asks for your password and installs it with `sudo pacman -U`, then starts again. To update by hand instead:

```sh
cd sumo-music
git pull
makepkg -sif
```

Optional extras:

```sh
sudo pacman -S noto-fonts noto-fonts-cjk inter-font
```

- `noto-fonts-cjk`: Japanese, Chinese and Korean titles.
- `inter-font`: a nicer UI font. Utako uses Inter, Noto Sans, Cantarell or DejaVu, whichever is installed.
- `noto-fonts` (or `ttf-dejavu`): symbols in names such as ☆ ✞ ♡ instead of empty boxes.

To run without packaging: `cargo build --release && ./target/release/multimusic`.

## Setting up your accounts

Everything is under **Settings** (bottom of the sidebar).

| Service | What to do |
| --- | --- |
| **Local files** | `~/Music` is scanned by default. Add more folders in Settings, or drag a folder onto the window. |
| **Spotify** | Click **Log in with Spotify**. Your browser opens Spotify's login page; approve it and your playlists and Liked Songs import automatically. The import goes through Utako's own Spotify connection, so it isn't affected by Web API rate limits. Playback needs **Spotify Premium**. **Search** uses Spotify's Web API, whose shared key is often rate limited (HTTP 429). To fix that, create a free app at [developer.spotify.com](https://developer.spotify.com/dashboard) (tick *Web API*), then paste its **Client ID** and **Client secret** (app → Settings → *View client secret*) under *Settings → Spotify → Advanced*. That's all: no browser login and no Redirect URI. (Prefer a browser login to your app instead? Open *Or log in to your app in the browser*, add the Redirect URI shown there to your app, **Save**, and click **Authorize**.) |
| **SoundCloud** | Paste your profile URL (`https://soundcloud.com/you`) and click **Sync**. Private likes and playlists also need your `oauth_token` cookie from soundcloud.com (DevTools → Application → Cookies). |
| **Last.fm** | Create an API account at [last.fm/api/account/create](https://www.last.fm/api/account/create), paste the key and secret, click **Connect account** and approve in the browser. |
| **Discord** | Create an application at [discord.com/developers/applications](https://discord.com/developers/applications) (call it "Utako", or anything you like), and paste its **Application ID**. |

## Lossless & hi-res

- Local **FLAC, ALAC, WAV, AIFF, APE and WavPack** play at their native bit depth and sample rate. The player dock shows the format (e.g. `FLAC · 24-bit / 96 kHz`) with a **LOSSLESS** or **HI-RES** badge.
- **Settings → Playback → Output device** picks the exact output (e.g. your USB DAC). **Bit-perfect output** opens the device exclusively and skips ReplayGain. For true bit-perfect playback, choose an `alsa/hw:…` device, keep Utako's volume at 100% and use your DAC or amp for volume.
- On **PipeWire**, everything is resampled to PipeWire's rate unless you let it switch rates. Create `~/.config/pipewire/pipewire.conf.d/10-rates.conf`:

  ```
  context.properties = {
      default.clock.allowed-rates = [ 44100 48000 88200 96000 176400 192000 ]
  }
  ```

  Then run `systemctl --user restart pipewire`.
- **Spotify lossless isn't possible.** Spotify only streams its FLAC files to the official Spotify apps, so librespot-based players (including Utako) get 320 kbps Ogg Vorbis at most.

## Back up or move your settings

*Settings → Back up or move your settings* saves everything on the Settings page to one file (`Utako settings <date>.json` in your Documents folder), optionally with your **keys and logins** (Last.fm, SoundCloud, Discord, your Spotify app keys and the Spotify login itself) and **your own playlists and Liked Songs**. On another computer (Linux, Windows or macOS) import it from the same section, or drop the file onto the window: Utako takes the settings over, adds the playlists and restarts. Library and download folders that don't exist on the new computer, the mpv program and the audio device are left as they are there. A file with keys holds your passwords and tokens, so keep it private.

## Media server

Utako works with [mediaserver](https://github.com/v0-0x/mediaserver) (1.0.8 or newer), the home server that also holds the anime and movies for the Aniko and KumoBox apps. Its music side is for Utako:

- Turn on *Settings → Media server → Use a media server*. Utako finds the server on your network (or type its address, like `pi.local`), and you let this device in from the server's dashboard (*Music → Devices*). Settings shows the name to look for ("Utako on <computer> (<id>)").
- The server's music shows up as a playlist named after it ("On <server>"), with its covers. Its songs stream from the server, and seeking works.
- **Download** a server song (its menu, the download button next to it, or the playlist's download button for all of them) to keep it on this device and play it without the server, on a trip or on your iPhone. The server's file comes as it is (a FLAC stays a FLAC), into a *Media server* folder in your download folder; its own tags are kept, and only what it lacks (title, artist, album, cover, lyrics) is filled in.
- **Edit a server song's details** (title, artist, album, genre, year, track numbers, lyrics and cover) from its right-click menu, one song or many, like a local file's. Utako fetches the song, writes the new details into it and sends it back; the server keeps the song as it was until the new copy is all there, then swaps it in, so every device sees the change. It needs mediaserver 1.0.13 or newer and *Let it upload here* for this device.
- *Upload a folder…* and *Upload a song…* send music from this computer into the server's library, once the dashboard turns on *Let it upload here* for this device. A folder keeps its name and what's in it (songs, covers, lyrics); what's there already isn't sent again, and an upload cut short carries on where it stopped.
- *Settings → Downloads → Send downloads to the media server* sends the songs you download there instead of keeping them on this device (it needs *Let it upload here* too). Each song is made in Utako's cache folder, tagged, sent into a folder named after its service (`Spotify/Artist - Song.ogg`) and removed here. A file of the same name that's already there isn't replaced.
- *Send my settings and logins* puts your settings, keys, logins and playlists on the server. *Get settings and logins from it* brings them to another device, such as an iPhone. They're encrypted so that only the server can read them, and the server encrypts them again for each device it has let in.

Requests are signed the way the Aniko and KumoBox apps sign theirs. [docs/media-server.md](docs/media-server.md) describes what Utako and the music side say to each other.

## Keyboard shortcuts

| Key | Action |
| --- | --- |
| `Space` | Play / pause |
| `←` / `→` | Seek −5 s / +5 s |
| `Ctrl+←` / `Ctrl+→` | Previous / next track |
| `↑` / `↓` | Volume up / down |
| `Ctrl+K`, `Ctrl+F` or `/` | Search (paste a link to open it) |
| `Alt+←` / `Alt+→`, mouse back / forward buttons | Back / forward |
| `L` | Toggle the full-screen *Now playing* / lyrics view |
| `Esc` | Leave the *Now playing* view, or let go of selected songs |
| `Ctrl+A` / `Ctrl+C` | Select every song in a list / copy the selected songs |
| `Ctrl+V` | Paste copied songs (or Spotify / SoundCloud links, or music files) into the open playlist |
| `Ctrl+X` / `Delete` | Cut / remove the selected songs from your playlist |
| `M` | Mute / unmute |
| `S` / `R` | Shuffle on or off / repeat all, one, off |
| `H` | Like the playing song |
| `Ctrl++` / `Ctrl+−` / `Ctrl+0` | Bigger / smaller / normal interface (remembered) |
| `?` | Show every shortcut |
| `Ctrl+Q` | Quit |

Double-click a song to play it. Ctrl-click or Shift-click songs to select several. Click an artist name to open the artist. Right-click a song for *Play next*, *Add to queue*, *Add to playlist*, *Like*, *Go to artist / album* and *Open in Spotify / Show in folder*; right-click an album, artist or search result for its own menu. Click a column name (*Title*, *Album*, the clock) to sort a list, or `#` for every way to sort (artist, recently added, …). In your own playlists, drag songs to move them. Scroll the mouse wheel over the volume to change it, and hover the seek bar to see the time under the pointer. Drag the sidebar or the right panel by its edge to resize it; drag the sidebar narrow (or click the logo) for the compact icons-and-covers strip.

## Memory use

I measured this in a VM while a local song was playing, with the album view and lyrics open:

| Process | Resident memory |
| --- | --- |
| `multimusic` | ~152 MB in total, but most of that is the VM's *software* OpenGL renderer (llvmpipe: about 66 MB of libraries plus its frame buffers on the heap). With a real GPU that work moves to the graphics driver; I haven't measured it on real hardware. Utako's own heap is under 15 MB. |
| `mpv` (playback of local files & SoundCloud) | ~54 MB RSS, of which only ~13 MB is private; the rest is shared ffmpeg libraries. It only runs while you play local files or SoundCloud. |

In the same VM and scenario, the version before these memory changes used ~197 MB; most of the saving holds on any machine:

- glibc's allocator is limited to two arenas and hands large freed blocks straight back (`mallopt`), and freed memory is returned after scans, syncs and every 30 s while the window is active (`malloc_trim`).
- egui's glyph atlas is capped at 2048 px wide instead of the GPU's maximum (often 16384 px), where one big cover letter reserved a whole 16384 px row: 2 MiB instead of 8 MiB, in RAM and on the GPU.
- Covers are shrunk straight from a cropped view of the decoded image, without two more full-size copies (a 3000 px cover used to take ~90 MB for a moment).

Spotify playback runs inside the Utako process (librespot) and doesn't start another process. The official Spotify client usually uses 400–800 MB.

To keep memory low: lower *Covers kept in memory* in Settings → Appearance, and close the lyrics panel (it repaints 10× per second while playing, compared with twice per second otherwise).

## Files

| What | Linux | Windows | macOS |
| --- | --- | --- | --- |
| Settings (`config.toml`, also editable by hand) | `~/.config/multimusic/` | `%APPDATA%\multimusic\config\` | `~/Library/Application Support/multimusic/` |
| Library database, Spotify login, scrobble queue, media server key (`server-identity.json`), log | `~/.local/share/multimusic/` | `%APPDATA%\multimusic\data\` | `~/Library/Application Support/multimusic/` |
| Cover art and lyrics cache | `~/.cache/multimusic/` | `%LOCALAPPDATA%\multimusic\cache\` | `~/Library/Caches/multimusic/` |

Logs: on Linux run `MULTIMUSIC_LOG=multimusic=debug multimusic` in a terminal. The Windows and macOS apps write `multimusic.log` in the data folder (set `MULTIMUSIC_LOG=multimusic=debug` for more detail).

## Troubleshooting

- **No sound from Spotify**: Spotify plays through PipeWire/PulseAudio (`pipewire-pulse`) and follows your default output device. If you use plain ALSA without a sound server, set *Settings → Spotify → Audio output* to ALSA.
- **Spotify login says "redirect_uri: Not matching configuration"**: the *Redirect URI* in *Settings → Spotify → Advanced* and one of the Redirect URIs in your app on developer.spotify.com must be identical, character for character. Either add Utako's (`http://127.0.0.1:8899/login` by default) to your app, under Settings → Edit → Redirect URIs → Add, then **Save** at the bottom; or paste the one your app already lists into Utako. Use `127.0.0.1`, not `localhost` (Spotify rejects `localhost`). Put your app's ID in *Your own Spotify app → Client ID*, not in *Login client ID*. Use `http://`, not `https://` (Spotify only wants https for internet addresses). Utako asks Spotify before opening the browser, uses the form of the URI your app has saved (e.g. with a trailing slash), and otherwise says what's missing. Easiest of all: paste your app's **Client secret** instead, which needs no Redirect URI. A login you abandon times out after 5 minutes; clicking Log in / Authorize again restarts it right away.
- **Upgrading from Medley**: your settings, library and logins move to the new `multimusic` folders automatically on first start.
- **Spotify "HTTP 429 Too Many Requests"** or **"rate limited" under Spotify in search**: that's the shared Web API key being rate limited. Library import and playback don't use it. For search, add your own Spotify app (see *Setting up your accounts*); search now says so right away instead of spinning.
- **Boxes (□) in song or artist names**: install `noto-fonts` (or `ttf-dejavu`) for symbol characters, and `noto-fonts-cjk` for Japanese, Chinese and Korean.
- **Crossfade does nothing**: it's off in bit-perfect mode and with `alsa/hw:` output devices, because those can't be opened twice at the same time.
- **Spotify says Premium is required**: playlists and search work on free accounts, but librespot can only stream with Premium.
- **A pasted Spotify link says "Log in to Spotify"**: Spotify pages load through your Spotify login, so log in under Settings first. SoundCloud links work without an account.
- **`makepkg` fails in `check()`**: update to the latest commit (`git pull`); an older test could fail when a network port was busy. `makepkg -si --nocheck` skips the tests.
- **Some SoundCloud tracks won't play**: SoundCloud Go+ tracks only offer 30-second previews to third-party apps, and some tracks are region-locked.
- **Spotify songs download from SoundCloud, not from Spotify**: Utako isn't logged in to Spotify (*Settings → Spotify*). Spotify songs come straight from Spotify through that login.
- **A Spotify download fails**: the Downloads page says why (hover over it for all of it). "Couldn't connect to Spotify": see *Settings → Spotify*, then *Try again*. "Spotify can't play this song here": Spotify has no playable copy of it for your account or country. "Spotify didn't give the key": saving, like playing, needs Premium.
- **A music video doesn't play**: Utako says YouTube's reason (a private or removed video). Videos YouTube only shows to signed-in adults, and some made for kids, can't play without a YouTube account, which Utako doesn't use. "YouTube wants this network to sign in" is YouTube's bot check, which some networks (VPNs, data centres) get: try again later or without the VPN. A video with no picture in the app needs ffmpeg (`sudo pacman -S ffmpeg`).
- **A SoundCloud download fails**: Go+ songs only offer a 30 second preview or an encrypted stream, and neither is saved. Region-locked songs fail too. Other failures can be retried from the Downloads page.
- **Japanese/Korean/Chinese text shows boxes**: install `noto-fonts-cjk`. Utako loads a CJK font only when your library needs it.
- **Media keys don't work**: on Linux Utako registers as `org.mpris.MediaPlayer2.multimusic`; check with `playerctl -l`. It needs a D-Bus session, which every normal desktop session has. Windows (media overlay and keys) and macOS (Now Playing and keys) need nothing.
- **macOS says Utako "can't be opened" or "is damaged"**: it isn't notarized by Apple. Right-click it → Open → Open once, or run `xattr -dr com.apple.quarantine /Applications/Utako.app` in Terminal.
- **Windows: the window stays black or Utako closes right away**: Utako draws with OpenGL; update your graphics driver. Remote Desktop sessions and virtual machines without a graphics driver only offer OpenGL 1.1, which is too old. `%APPDATA%\multimusic\data\multimusic.log` says what went wrong.
- **Beta updates**: *Settings → Updates → Beta updates* gets new versions first, as betas (GitHub pre-releases), before they go out to everyone. Turning it off keeps the beta you have until a newer stable version comes out.
- **An update doesn't install**: *Settings → Updates* shows why. You can always download the newest version from the [Releases page](https://github.com/v0-0x/Utako/releases/latest) and install it over the old one; settings are kept.
- **Interface too small or too large**: Settings → Appearance → *Interface scale*.

## How it works

```
src/
  main.rs              window + runtime setup
  service/             background service: commands and shared state (mod.rs), playback and
                       crossfade (playback.rs), library scans and syncs (sync.rs), search and
                       pages (pages.rs), downloads (downloads.rs), updates and settings files (updates.rs)
  links.rs             recognises pasted Spotify / SoundCloud links
  downloader.rs        downloads: finds the audio, then tags it with metadata, cover and lyrics, and
                       keeps it or sends it to the media server
  player/              play queue, mpv (JSON IPC) engine, librespot engine + Spotify OAuth, and
                       native.rs: the iPhone app's own player (rodio + symphonia) in mpv's place
  library/             SQLite store, incremental tag scanner and tag writer (lofty), M3U import/export
  providers/           Spotify (session + Web API, and saving its audio: spotify_audio.rs), SoundCloud api-v2,
                       YouTube music videos (its own API: search, streams, saving)
  integrations/        lyrics (LRC/LRCLIB/Genius), Genius song details, Last.fm (scrobbling, profile stats),
                       Discord RPC, MPRIS, notifications, ios.rs (audio session, lock screen)
  ui/                  egui pages (views/), theme, color and holiday themes (themes.rs, holiday.rs),
                       widgets, cover art cache
```

The UI thread only draws. Playback, network and disk work run on a small tokio runtime and publish snapshots that the UI reads each frame.

## Disclaimer

Utako is an unofficial client and isn't affiliated with Spotify, SoundCloud, YouTube, Apple or Discord. Spotify support uses librespot. Use the integrations in line with each service's terms.

## License

MIT
