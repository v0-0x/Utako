# Utako and the media server

Utako can already talk to a music side of [mediaserver](https://github.com/v0-0x/mediaserver).
That side doesn't exist yet. This note describes what it needs to answer, so it can be added
later without changing Utako.

Everything here reuses the peer protocol that mediaserver's anime and movie sides already
speak with the Aniko and KumoBox apps (`anime/internal/share`): the same keys, signatures,
headers and sealing. Utako's copy is `src/hub/`. Its tests check it against vectors made by
mediaserver's own Go code (`src/hub/testdata/mediaserver-vectors.json`).

`stand_in_server` in `src/hub/client.rs` runs the whole exchange against a stand-in built from
that Go code:
- hello before and after the device is allowed;
- the library;
- settings sent from one device and fetched by another;
- a song streamed by mpv with a ticket, with a `Range` request;
- a ticket for one file refused for another.

Run it with `UTAKO_STANDIN=<address> cargo test -- --ignored stand_in_server`.

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
| `GET /api/peer/hello` | yes | `{"id","name","version","shares","files"}`. See below. |
| `GET /api/peer/music/library` | yes | `{"version","tracks":[…]}`. See below. |
| `GET /api/peer/local/file?path=<QueryEscape(path)>` | ticket | The audio file's bytes. |
| `POST /api/peer/music/settings` | yes | 2xx with an empty body. Body: `{"sealed","updatedAt"}`. |
| `GET /api/peer/music/settings` | yes | `{"sealed","updatedAt"}`, or 404 when nothing is kept yet. |

### hello

- `shares` is true once this device has been allowed in the dashboard. Until then the answer
  is still signed, with `shares: false`, and Settings tells the user to allow
  "Utako on … (id)" there.
- `files` is any string that changes when the library does, such as a counter or a hash.
  Utako asks for the library once per start, then again only when `files` changes.
- Utako says hello on start, after a settings change, when "Connect again" is pressed, and
  every 2 minutes.

### library

Each track looks like this:

```json
{"path": "/music/Artist/Album/01 Song.flac", "title": "Song", "artist": "Artist",
 "album": "Album", "albumArtist": "Artist", "trackNo": 1, "discNo": 1,
 "durationMs": 215000, "addedAt": 1761955200}
```

- Only `path` is needed. The rest should come from the file's tags. An empty title shows
  the file name instead.
- `path` is what `/api/peer/local/file` takes. Utako never builds paths itself.

### file (a ticket)

- This is the same as the anime side's file request: a GET with an **empty nonce**, valid for
  12 hours, because mpv repeats the same request whenever it seeks.
- It must support `Range`, or seeking won't work.
- Utako signs a new ticket every time a song starts. On iPhone the app's own player sends
  the same headers.

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

## What Utako keeps (cache)

- The server's songs are stored as tracks of their own kind ("Media server", id
  `server:<server id>:<path>`). Folder scans never touch them.
- They also form a playlist named "On <server name>". Unticking "Play its music" in Settings
  removes the playlist, and ticking it again brings it back.
- They stay listed while the server is away. Playing one then says the server isn't
  connected.
- Utako remembers the ID of the first server it connected to. If a different server answers
  at the same address, Utako refuses it.
- Lyrics come from Utako's usual sources. There is no cover art for server songs yet.

## Later

These aren't in Utako yet:

- Cover art: something like `GET /api/peer/music/art?path=…`, answered as an image.
- Settings that sync on their own. That would need an `updatedAt` comparison, and the
  endpoints above already carry it.
- Downloading songs for offline play, on iPhone in particular.
