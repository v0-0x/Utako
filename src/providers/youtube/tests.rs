use super::*;

fn video(title: &str, channel: &str) -> Video {
    Video {
        id: "dQw4w9WgXcQ".into(),
        title: title.into(),
        channel: channel.into(),
        duration: Some(213.0),
    }
}

#[test]
fn music_video_titles() {
    let t = video_track(&video(
        "Rick Astley - Never Gonna Give You Up (Official Music Video)",
        "Rick Astley",
    ));
    assert_eq!(
        (t.artist.as_str(), t.title.as_str()),
        ("Rick Astley", "Never Gonna Give You Up")
    );
    assert_eq!(t.id, "youtube:dQw4w9WgXcQ");
    assert_eq!(t.duration_ms, 213_000);
    assert_eq!(t.uri, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    assert!(t.art.unwrap().contains("dQw4w9WgXcQ"));
    // No "artist - title": the channel is the artist; brackets that aren't decorations stay.
    let t = video_track(&video("Blinding Lights [Lyrics] (Remix)", "The Weeknd - Topic"));
    assert_eq!(
        (t.artist.as_str(), t.title.as_str()),
        ("The Weeknd", "Blinding Lights (Remix)")
    );
    let t = video_track(&video("Dua Lipa – Houdini (Official Video)", "DuaLipaVEVO"));
    assert_eq!((t.artist.as_str(), t.title.as_str()), ("Dua Lipa", "Houdini"));
}

#[test]
fn video_ids() {
    assert_eq!(video_id("youtube:dQw4w9WgXcQ"), Some("dQw4w9WgXcQ"));
    assert_eq!(video_id("youtube:../etc/pass"), None);
    assert_eq!(video_id("spotify:track:x"), None);
}

/// A search answer as the website gets it: an older `videoRenderer`, a newer `lockupViewModel`,
/// a live stream, a playlist and the same video twice.
const SEARCH: &str = r#"{
  "responseContext": {"visitorData": "CgtWaXNpdG9ySUQ%3D"},
  "contents": {"twoColumnSearchResultsRenderer": {"primaryContents": {"sectionListRenderer": {"contents": [
    {"itemSectionRenderer": {"contents": [
      {"videoRenderer": {"videoId": "dQw4w9WgXcQ",
        "title": {"runs": [{"text": "Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)"}]},
        "ownerText": {"runs": [{"text": "Rick Astley"}]},
        "lengthText": {"accessibility": {"accessibilityData": {"label": "3 minutes, 34 seconds"}}, "simpleText": "3:34"}}},
      {"lockupViewModel": {"contentId": "fJ9rUzIMcZQ", "contentType": "LOCKUP_CONTENT_TYPE_VIDEO",
        "contentImage": {"thumbnailViewModel": {"overlays": [{"thumbnailBottomOverlayViewModel": {"badges": [
          {"thumbnailBadgeViewModel": {"text": "5:55", "badgeStyle": "THUMBNAIL_OVERLAY_BADGE_STYLE_DEFAULT"}}]}}]}},
        "metadata": {"lockupMetadataViewModel": {"title": {"content": "Queen – Bohemian Rhapsody (Official Video Remastered)"},
          "metadata": {"contentMetadataViewModel": {"metadataRows": [
            {"metadataParts": [{"text": {"content": "Queen Official"}}]},
            {"metadataParts": [{"text": {"content": "2B views"}}, {"text": {"content": "16 years ago"}}]}]}}}}}},
      {"videoRenderer": {"videoId": "jfKfPfyJRdk", "title": {"runs": [{"text": "lofi hip hop radio"}]},
        "ownerText": {"runs": [{"text": "Lofi Girl"}]}}},
      {"lockupViewModel": {"contentId": "PL1234567890", "contentType": "LOCKUP_CONTENT_TYPE_PLAYLIST"}},
      {"videoRenderer": {"videoId": "dQw4w9WgXcQ", "title": {"simpleText": "again"}, "lengthText": {"simpleText": "3:34"}}},
      {"videoRenderer": {"videoId": "kJQP7kiw5Fk", "title": {"runs": [{"text": "Luis Fonsi - Despacito ft. Daddy Yankee"}]},
        "longBylineText": {"runs": [{"text": "Luis Fonsi"}]}, "lengthText": {"simpleText": "1:02:03"}}}
    ]}}
  ]}}}}
}"#;

#[test]
fn parses_search_results() {
    let answer: Value = serde_json::from_str(SEARCH).unwrap();
    let videos = parse_search(&answer);
    let ids: Vec<&str> = videos.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, ["dQw4w9WgXcQ", "fJ9rUzIMcZQ", "kJQP7kiw5Fk"]);
    assert_eq!(videos[0].duration, Some(214.0));
    assert_eq!(videos[0].channel, "Rick Astley");
    assert_eq!(
        (videos[1].title.as_str(), videos[1].channel.as_str(), videos[1].duration),
        (
            "Queen – Bohemian Rhapsody (Official Video Remastered)",
            "Queen Official",
            Some(355.0)
        )
    );
    assert_eq!(
        (videos[2].channel.as_str(), videos[2].duration),
        ("Luis Fonsi", Some(3723.0))
    );
    let t = video_track(&videos[1]);
    assert_eq!((t.artist.as_str(), t.title.as_str()), ("Queen", "Bohemian Rhapsody"));
    assert_eq!(parse_duration("4:13"), Some(253.0));
    assert_eq!(parse_duration("LIVE"), None);
    assert_eq!(parse_duration(""), None);
}

#[test]
fn why_videos_dont_play() {
    let status = |s: &str| playability(&serde_json::from_str(s).unwrap());
    assert_eq!(status(r#"{"playabilityStatus": {"status": "OK"}}"#), Ok(()));
    assert_eq!(
        status(r#"{"playabilityStatus": {"status": "LOGIN_REQUIRED", "reason": "Sign in to confirm your age"}}"#),
        Err(Refusal::Age)
    );
    assert_eq!(
        status(r#"{"playabilityStatus": {"status": "AGE_VERIFICATION_REQUIRED"}}"#),
        Err(Refusal::Age)
    );
    assert_eq!(
        status(
            r#"{"playabilityStatus": {"status": "LOGIN_REQUIRED", "reason": "Sign in to confirm you’re not a bot"}}"#
        ),
        Err(Refusal::Bot)
    );
    assert_eq!(
        status(
            r#"{"playabilityStatus": {"status": "UNPLAYABLE", "errorScreen": {"playerErrorMessageRenderer":
                {"reason": {"runs": [{"text": "Video unavailable"}]}, "subreason": {"simpleText": "This video is private"}}}}}"#
        ),
        Err(Refusal::Said("Video unavailable".into()))
    );
    assert_eq!(
        status(r#"{"playabilityStatus": {"status": "ERROR"}}"#),
        Err(Refusal::Said("this video can't play (ERROR)".into()))
    );
    assert_eq!(status("{}"), Err(Refusal::Said("no answer about this video".into())));
    assert!(Refusal::Age.to_string().contains("signed-in adults"));
    assert_eq!(
        Refusal::Said("Private video".into()).to_string(),
        "YouTube: Private video"
    );
}

/// A player answer's streams: a stream with both, H.264, VP9 and AV1 pictures, AAC and Opus
/// sound, a "stable volume" mix, a dubbed sound, and streams that need YouTube's JavaScript.
fn player_answer(base: &str) -> Value {
    let f = |itag: u32, mime: &str, extra: Value| {
        let mut v = json!({
            "itag": itag,
            "url": format!("{base}/videoplayback?itag={itag}&expire=1"),
            "mimeType": mime,
            "bitrate": 1000 + itag,
        });
        for (k, val) in extra.as_object().unwrap() {
            v[k] = val.clone();
        }
        v
    };
    json!({
        "playabilityStatus": {"status": "OK"},
        "streamingData": {
            "formats": [f(18, "video/mp4; codecs=\"avc1.42001E, mp4a.40.2\"", json!({"width": 640, "height": 360}))],
            "adaptiveFormats": [
                f(137, "video/mp4; codecs=\"avc1.640028\"", json!({"width": 1920, "height": 1080, "fps": 25, "contentLength": "4000"})),
                f(136, "video/mp4; codecs=\"avc1.4d401f\"", json!({"width": 1280, "height": 720, "fps": 25})),
                f(247, "video/webm; codecs=\"vp9\"", json!({"width": 1280, "height": 720, "fps": 25})),
                f(399, "video/mp4; codecs=\"av01.0.08M.08\"", json!({"width": 1920, "height": 1080, "fps": 25})),
                f(140, "audio/mp4; codecs=\"mp4a.40.2\"", json!({"averageBitrate": 129000, "contentLength": "2000"})),
                f(251, "audio/webm; codecs=\"opus\"", json!({"averageBitrate": 135000})),
                f(251, "audio/webm; codecs=\"opus\"", json!({"averageBitrate": 140000, "isDrc": true})),
                f(251, "audio/webm; codecs=\"opus\"", json!({"averageBitrate": 160000,
                    "audioTrack": {"displayName": "French", "audioIsDefault": false}})),
                {"itag": 248, "mimeType": "video/webm; codecs=\"vp9\"", "height": 1080,
                 "signatureCipher": "s=AAA&sp=sig&url=https%3A%2F%2Fx"},
                {"itag": 22, "url": format!("{base}/videoplayback?itag=22&n=abc"), "mimeType": "video/mp4; codecs=\"avc1.64001F, mp4a.40.2\"", "height": 720},
            ]
        },
        "adPlacements": [
            {"adPlacementRenderer": {"config": {"adPlacementConfig": {"kind": "AD_PLACEMENT_KIND_START"}},
              "renderer": {"instreamVideoAdRenderer": {"skipOffsetMilliseconds": 5000}}}},
            {"adPlacementRenderer": {"config": {"adPlacementConfig": {"kind": "AD_PLACEMENT_KIND_START"}},
              "renderer": {"instreamVideoAdRenderer": {"playerVars": "length_seconds=3&x=1"}}}},
            {"adPlacementRenderer": {"config": {"adPlacementConfig": {"kind": "AD_PLACEMENT_KIND_MILLISECONDS"}},
              "renderer": {"instreamVideoAdRenderer": {"skipOffsetMilliseconds": 9000}}}}
        ]
    })
}

#[test]
fn picks_streams() {
    let answer = player_answer("https://rr1.googlevideo.com");
    let formats = parse_formats(&answer);
    // Those that need YouTube's JavaScript are left out.
    assert!(formats.iter().all(|f| f.itag != 248 && f.itag != 22));
    assert_eq!(formats.len(), 9);
    let sound = pick_sound(&formats).unwrap();
    // The best of the video's own sound: not the louder "stable volume" mix, not the dub.
    assert_eq!(
        (sound.itag, sound.bitrate, sound.drc, sound.original),
        (251, 135000, false, true)
    );
    let picture = pick_picture(&formats).unwrap();
    assert_eq!((picture.itag, picture.height), (136, 720));
    assert_eq!(label(picture), "H.264 720p");
    let plan = plan_download(&formats).unwrap();
    assert_eq!(
        (plan.picture.itag, plan.sound.map(|s| s.itag), plan.container),
        (137, Some(140), "mp4")
    );
    assert_eq!(wait_seconds(&answer), 8);

    // No H.264: the best picture and sound in an MKV.
    let no_h264: Vec<Format> = formats.iter().filter(|f| !is_h264(f)).cloned().collect();
    let plan = plan_download(&no_h264).unwrap();
    assert_eq!(
        (
            plan.picture.itag,
            plan.sound.map(|s| (s.itag, s.bitrate)),
            plan.container
        ),
        (247, Some((251, 135000)), "mkv")
    );
    assert_eq!(pick_picture(&no_h264).unwrap().itag, 247);
    // Only a stream with both.
    let muxed: Vec<Format> = formats.iter().filter(|f| f.itag == 18).cloned().collect();
    let plan = plan_download(&muxed).unwrap();
    assert_eq!((plan.picture.itag, plan.sound, plan.container), (18, None, "mp4"));
    assert_eq!(pick_sound(&muxed).unwrap().itag, 18);
    // AV1 only: still a picture.
    let av1: Vec<Format> = formats.iter().filter(|f| f.itag == 399).cloned().collect();
    assert_eq!(pick_picture(&av1).unwrap().itag, 399);
    assert!(parse_formats(&json!({})).is_empty());
}

#[test]
fn requests_look_like_youtubes_apps() {
    let c = context(&VISIONOS);
    assert_eq!(c["client"]["clientName"], "VISIONOS");
    assert_eq!(c["client"]["clientVersion"], "1.02");
    assert_eq!(c["client"]["deviceModel"], "RealityDevice17,1");
    assert_eq!(c["client"]["osName"], "visionOS");
    assert_eq!(c["client"]["hl"], "en");
    assert_eq!(c["client"]["userAgent"], VISIONOS.user_agent);
    let w = context(&WEB);
    assert_eq!(w["client"]["clientName"], "WEB");
    assert!(w["client"]["userAgent"].is_null());
    let page = r#"<script>ytcfg.set({"VISITOR_DATA":"CgtBQkNERUZHSElKSyiA==","X":1});</script>"#;
    assert_eq!(
        json_string(page, "VISITOR_DATA").as_deref(),
        Some("CgtBQkNERUZHSElKSyiA==")
    );
    assert_eq!(json_string(page, "nope"), None);
    let opts = mpv_options(&link("https://x/y"));
    assert_eq!(opts, vec![("user-agent", VISIONOS.user_agent.to_string())]);
}

// ------------------------------------------------------------------------ a fake YouTube

mod fake {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// What the fake was asked: "METHOD path" and the headers.
    pub type Seen = Arc<Mutex<Vec<(String, HashMap<String, String>)>>>;

    /// Serves a watch page, the player API (`player` is its answer, with `{base}` for its own
    /// address) and the files in `media` by name, with `Range`.
    pub async fn serve(player: String, media: HashMap<String, PathBuf>) -> (String, Seen) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen: Seen = Arc::default();
        let (player, seen2) = (player.replace("{base}", &base), seen.clone());
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let (player, media, seen) = (player.clone(), media.clone(), seen2.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 8192];
                    let head_end = loop {
                        let n = stream.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break i + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                    let mut lines = head.lines();
                    let first = lines.next().unwrap_or("").to_string();
                    let headers: HashMap<String, String> = lines
                        .filter_map(|l| l.split_once(':'))
                        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
                        .collect();
                    let length: usize = headers.get("content-length").and_then(|l| l.parse().ok()).unwrap_or(0);
                    while buf.len() < head_end + length {
                        let n = stream.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    let mut parts = first.split_whitespace();
                    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                    seen.lock()
                        .unwrap()
                        .push((format!("{method} {target}"), headers.clone()));
                    let path = target.split('?').next().unwrap_or("");
                    let reply = if path == "/watch" {
                        let page =
                            r#"<html><script>ytcfg.set({"VISITOR_DATA":"CgtGQUtFVklTSVRPUg=="});</script></html>"#;
                        format!(
                            "HTTP/1.1 200 OK\r\nSet-Cookie: VISITOR_INFO1_LIVE=fake; Path=/; Secure\r\nSet-Cookie: YSC=abc; Path=/\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
                            page.len()
                        )
                        .into_bytes()
                    } else if path == "/api/player" {
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{player}",
                            player.len()
                        )
                        .into_bytes()
                    } else if let Some(file) = target
                        .split("itag=")
                        .nth(1)
                        .and_then(|i| media.get(i.split('&').next().unwrap_or("")))
                    {
                        let data = std::fs::read(file).unwrap();
                        let (from, to) = headers
                            .get("range")
                            .and_then(|r| r.strip_prefix("bytes="))
                            .and_then(|r| r.split_once('-'))
                            .map(|(a, b)| {
                                let a: usize = a.parse().unwrap();
                                let b: usize = b
                                    .parse()
                                    .map(|b: usize| b.min(data.len() - 1))
                                    .unwrap_or(data.len() - 1);
                                (a, b)
                            })
                            .unwrap_or((0, data.len() - 1));
                        let mut r = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {from}-{to}/{}\r\nConnection: close\r\n\r\n",
                            to + 1 - from,
                            data.len()
                        )
                        .into_bytes();
                        r.extend_from_slice(&data[from..=to]);
                        r
                    } else {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
                    };
                    let _ = stream.write_all(&reply).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        (base, seen)
    }
}

fn ffmpeg(args: &[&str]) -> bool {
    std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y"])
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

/// Streams and a download from a fake YouTube: the requests carry the visitor ID and cookies
/// a watch page gave, and the saved file has the picture and the sound.
#[tokio::test]
async fn plays_and_saves_from_a_fake_youtube() {
    let tmp = tempfile::tempdir().unwrap();
    let (v, a) = (tmp.path().join("v.mp4"), tmp.path().join("a.m4a"));
    let made = ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=320x240:rate=25",
        "-t",
        "2",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-an",
        v.to_str().unwrap(),
    ]) && ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=2",
        "-c:a",
        "aac",
        "-vn",
        a.to_str().unwrap(),
    ]);
    if !made {
        eprintln!("no ffmpeg with libx264 and aac: skipped");
        return;
    }
    let size = |p: &Path| std::fs::metadata(p).unwrap().len().to_string();
    let mut answer = player_answer("{base}");
    // Only the picture and sound the fake has, of the sizes it has them.
    let streams = answer["streamingData"]["adaptiveFormats"].as_array_mut().unwrap();
    streams.retain(|f| f["itag"] == 137 || f["itag"] == 140);
    streams[0]["contentLength"] = json!(size(&v));
    streams[1]["contentLength"] = json!(size(&a));
    answer["adPlacements"] = json!([]);
    answer["streamingData"]["formats"] = json!([]);
    let media = [("137".to_string(), v.clone()), ("140".to_string(), a.clone())]
        .into_iter()
        .collect();
    let (base, seen) = fake::serve(answer.to_string(), media).await;
    let site = Site {
        web: base.clone(),
        api: format!("{base}/api"),
    };
    let http = reqwest::Client::builder().no_proxy().build().unwrap();

    let s = streams_at(&http, &site, "dQw4w9WgXcQ").await.unwrap();
    assert_eq!(s.sound.url, format!("{base}/videoplayback?itag=140&expire=1"));
    assert_eq!(s.sound.user_agent.as_deref(), Some(VISIONOS.user_agent));
    let picture = s.picture.unwrap();
    assert_eq!((picture.height, picture.label.as_str()), (1080, "H.264 1080p"));
    assert_eq!(s.wait, 0);
    {
        let seen = seen.lock().unwrap();
        let (_, player) = seen.iter().find(|(r, _)| r.starts_with("POST /api/player")).unwrap();
        assert_eq!(
            player.get("x-goog-visitor-id").map(String::as_str),
            Some("CgtGQUtFVklTSVRPUg==")
        );
        assert_eq!(player.get("x-youtube-client-name").map(String::as_str), Some("101"));
        let cookie = player.get("cookie").unwrap();
        assert!(
            cookie.contains("SOCS=CAI") && cookie.contains("VISITOR_INFO1_LIVE=fake") && cookie.contains("YSC=abc")
        );
    }

    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let meta = Metadata {
        title: "Never Gonna Give You Up".into(),
        artist: "Rick Astley".into(),
        ..Metadata::default()
    };
    let last = std::sync::Mutex::new(0.0f32);
    let saved = download_at(&http, &site, "dQw4w9WgXcQ", &meta, &work, &|p| {
        *last.lock().unwrap() = p
    })
    .await
    .unwrap();
    assert_eq!(saved.label, "YouTube, H.264 1080p");
    assert_eq!(saved.path.extension().unwrap(), "mp4");
    assert_eq!(*last.lock().unwrap(), 1.0);
    let probe = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type:format_tags=title,artist",
            "-of",
            "json",
        ])
        .arg(&saved.path)
        .output()
        .unwrap();
    let probe: Value = serde_json::from_slice(&probe.stdout).unwrap();
    let kinds: Vec<&str> = probe["streams"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["codec_type"].as_str())
        .collect();
    assert_eq!(kinds, ["video", "audio"]);
    assert_eq!(probe["format"]["tags"]["title"], "Never Gonna Give You Up");
    assert_eq!(probe["format"]["tags"]["artist"], "Rick Astley");
    // Only the finished file is left.
    let left: Vec<_> = std::fs::read_dir(&work)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(left, vec![std::ffi::OsString::from("video.mp4")]);
    // The streams came in pieces, each asked for with a range.
    let seen = seen.lock().unwrap();
    assert!(seen
        .iter()
        .filter(|(r, _)| r.contains("videoplayback"))
        .all(|(_, h)| h.get("range").is_some_and(|r| r.starts_with("bytes=0-"))));
}
