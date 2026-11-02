# Utako and the media server

Utako talks to the music side of [mediaserver](https://github.com/v0-0x/mediaserver), from
mediaserver 1.0.8 on (its `music/` folder). This note is what the two say to each other.

Everything here reuses the peer protocol that mediaserver's anime and movie sides already
speak with the Aniko and KumoBox apps (`anime/internal/share`): the same keys, signatures,
headers and sealing. Utako's copy is `src/hub/`, the music side's `music/internal/peer`. Both
are tested against the same vectors, made by the anime side's Go code
(`src/hub/testdata/mediaserver-vectors.json`, copied as mediaserver's
`music/internal/peer/testdata/utako-vectors.json`).

`real_server` in `src/hub/client.rs` runs the whole exchange against a real mediaserver:
- hello before and after the device is let in (through the dashboard's API);
- the library, and covers;
- settings sent from one device and fetched by another;
- a song streamed by mpv with a ticket, a seek, and a `Range` request;
- a ticket for one file refused for another;
- uploads refused until the dashboard lets them, then a file of three pieces, stopped after
  the first and carried on; sent again (already there); another file of the same name (not
  sent over it); and the library has it a moment later.

Run it with `UTAKO_SERVER=<address> UTAKO_DASHBOARD=http://<address>:43211
UTAKO_SESSION=<the dashboard's kumo_session cookie> cargo test --release -- --ignored
real_server`. The library needs a song of 20 seconds or more, and one with a cover. It leaves
an "Utako upload test <number>" folder in the server's first music folder.

## Ports and finding the server

| | anime | movies & TV | music |
|---|---|---|---|
| HTTP | 43211 | 43311 | **43411** |
| beacon (UDP) | 43212 | 43312 | **43412** |

- The music side sends a beacon to the multicast group `239.255.75.77` on port 43412 every
  few seconds. The JSON is the same as the other sides send:
  `{"kumo":1,"id":"…","name":"Pi","port":43411,"key":"…","version":"…","platform":"…","role":"hub"}`.
- Utako listens when "Use a media server" is on in Settings. The first server it hears is
  used, unless an address is typed in. A typed address without a port gets 43411.

## This device

- Utako makes an X25519 key pair the first time and keeps it in
  `<data folder>/server-identity.json` (readable by this user only). It isn't part of
  settings backups: each device has its own.
- Its ID is the same fingerprint as Aniko's: lowercase base32 (no padding) of the first 10
  bytes of SHA-256 of the public key, so 16 characters.
- It sends the name "Utako on <computer name>". Settings shows it with the ID, so it can be
  matched in the dashboard.

## Signed requests

These are exactly as in `identity.go`:

- Headers: `X-Kumo-Peer-Id`, `-Key` (base64 public key), `-Name` (Go `QueryEscape`),
  `-Port` (0), `-Time` (Unix seconds), `-Nonce` (32 hex characters), `-Token`. There is no
  `-User`.
- The token is `mac("request", method, requestURI, time, nonce, name, port, user,
  hex(sha256(body)))`, with key `HMAC(shared, "kumo-share-v2|guest=<Utako>|host=<server>")`.
- Every accepted answer must carry `X-Kumo-Peer-Reply` = `mac("reply", nonce, Date)`.
  Utako refuses any 2xx answer without it. A refusal can be unsigned: Utako shows its
  `{"error": "…"}` text.
- The usual limits apply: 5 minutes of clock skew, each nonce used once, and signed bodies of
  at most 16 MiB. A settings file is usually a few KB, and a large playlist collection makes
  it a few MB.

## Endpoints the music side needs

| Request | Signed | Answer |
|---|---|---|
| `GET /api/peer/whoami` | no | `{"id","name","key","version","role"}`. Utako checks the key against the ID. |
| `GET /api/peer/hello` | yes | `{"id","name","version","shares","files","uploads"}`. See below. |
| `GET /api/peer/music/library` | yes | `{"version","tracks":[…]}`. See below. |
| `GET /api/peer/local/file?path=<QueryEscape(path)>` | ticket | The audio file's bytes. |
| `GET /api/peer/music/art?path=<QueryEscape(path)>` | yes | The song's cover (an image), or 404. |
| `POST /api/peer/music/settings` | yes | 2xx with an empty body. Body: `{"sealed","updatedAt"}`. |
| `GET /api/peer/music/settings` | yes | `{"sealed","updatedAt"}`, or 404 when nothing is kept yet. |
| `GET /api/peer/upload` | yes | `{"folders":[…],"chunk":8388608}`: where uploads go. See below. |
| `GET /api/peer/upload?folder=&path=&total=` | yes | `{"path","have","done"}`: where that file stands. |
| `POST /api/peer/upload?folder=&path=&offset=&total=` | yes | A piece of the file in the body; the same answer. |

Every request but `whoami` is refused with 403 unless it comes from the home network (private
addresses, link-local, Tailscale's). Until the dashboard lets a device in, only `hello` answers
it; a body over 1 MB from it isn't read at all.

### hello

- `shares` is true once this device has been allowed in the dashboard. Until then the answer
  is still signed, with `shares: false`, and Settings tells the user to allow
  "Utako on … (id)" there.
- `files` is any string that changes when the library does (the music side's is a counter).
  Utako asks for the library once per start, then again only when `files` changes.
- `uploads` is true once the dashboard has turned on "Let it upload here" for this device.
- Utako says hello on start, after a settings change, when "Connect again" is pressed, and
  every 2 minutes.

### library

Each track looks like this:

```json
{"path": "/music/Artist/Album/01 Song.flac", "title": "Song", "artist": "Artist",
 "album": "Album", "albumArtist": "Artist", "trackNo": 1, "discNo": 1,
 "durationMs": 215000, "addedAt": 1761955200, "hasArt": true}
```

- Only `path` is needed. The rest comes from the file's tags (read in Go, or by ffprobe for
  the formats Go can't read), else from its name ("01 - Artist - Song.flac"). The lengths
  come from ffprobe, a little after the songs. An empty title shows the file name instead.
- `path` is what `/api/peer/local/file` and `/api/peer/music/art` take. Utako never builds
  paths itself, and the server answers only for the songs in its library.
- `hasArt`: the song has a cover, in the file or as an image next to it (`cover`, `folder`,
  `front` or `album`, `.jpg`/`.png`/`.webp`). Utako fetches one per album folder and keeps it
  in its art cache (`<cache>/art/server/`).

### file (a ticket)

- This is the same as the anime side's file request: a GET with an **empty nonce**, valid for
  12 hours, because mpv repeats the same request whenever it seeks.
- It must support `Range`, or seeking won't work.
- Utako signs a new ticket every time a song starts. On iPhone the app's own player sends
  the same headers.
- Downloading a song (to keep it on the device) is the same request, made once from start to
  end; Utako checks the answer's `X-Kumo-Peer-Reply` and that it got `Content-Length` bytes.

### settings and logins

- Nothing is synced on its own. Two buttons in Settings do it:
  - "Send my settings and logins" posts a settings file, the same JSON as Settings → Back up,
    with keys, logins and playlists.
  - "Get settings and logins from it" fetches it and imports it like a backup.
- When Utako sends it, it is sealed for the server: AES-256-GCM with key
  `HMAC(shared, "kumo-share-v1|seal|guest=<server>|host=<Utako>")`, as
  base64(12-byte nonce ‖ ciphertext). This is the same as `sealFor(hubPub, hub.ID, device.ID)`
  when an Aniko gives the hub its login.
- When the server answers a GET, it seals it for the asking device instead:
  `guest=<that device>`, `host=<server>`.
- The server keeps one copy per account, the latest one sent. It only gives it to devices it
  has allowed.

## uploads

The same as the Aniko and KumoBox apps' uploads to a server (mediaserver's
`*/internal/upload`):

- `GET /api/peer/upload` lists the server's music folders (Utako uses the first) and the most
  a piece may hold (8 MB). It's refused unless the dashboard lets this device upload.
- For each file: `GET …?folder=0&path=<its place, slashes between folders>&total=<size>` says
  how much is there (`have`), or that it's `done` (the same file, by its size, is there). A
  409 means another file has its name there: it isn't sent over it.
- Then `POST …&offset=<have>` with the next piece in the body (`application/octet-stream`,
  signed like any request: its SHA-256 is in the token). Utako sends 2 MB pieces, so its
  progress bar moves. The answer is where it stands now; a 409 means the piece doesn't follow
  on from what's there, and Utako asks again where it stands.
- The file is written as `<name>.part` and takes its name once it's whole. The server's
  library reads it a moment later, and `files` changes.
- A folder keeps its name: `Album/01 Song.flac`. Utako sends its songs, covers, lyrics
  (`.lrc`) and cue sheets, and leaves hidden files behind.
- Downloads go the same way when *Send downloads to the media server* is on (Settings ›
  Downloads) and the server lets this device upload: each song is made and tagged in Utako's
  cache, sent as `<service>/Artist - Song.ext` (`Spotify/`, `SoundCloud/`, and `Music videos/`
  for music videos, as MP4 or MKV) into the first music folder, then removed here. A 409 (another file of that
  name) fails that download rather than replacing the file. Utako asks for the library again
  a few seconds later, so the song shows up in the server's playlist.

## What Utako keeps (cache)

- The server's songs are stored as tracks of their own kind ("Media server", id
  `server:<server id>:<path>`). Folder scans never touch them.
- They also form a playlist named "On <server name>". Unticking "Play its music" in Settings
  removes the playlist, and ticking it again brings it back.
- They stay listed while the server is away. Playing one then says the server isn't
  connected.
- Utako remembers the ID of the first server it connected to. If a different server answers
  at the same address, Utako refuses it.
- A song whose tags (or cover) changed on the server is updated in Utako's library too.
- Lyrics come from Utako's usual sources.

## Later

These aren't in Utako yet:

- Settings that sync on their own. That would need an `updatedAt` comparison, and the
  endpoints above already carry it.
- Uploads from the iPhone app (it has no file picker for them).
- Music videos in the server's library: its music side takes only audio files as songs, so
  the videos Utako sends (MP4, M4V, MKV, WebM, MOV) are kept but not listed yet. Once its
  library lists them (with `"video": true`, say), Utako plays them like the video files here:
  the sound in mpv, the picture through ffmpeg with the same ticket.
