//! The music video player in the centre panel, in the style of Aniko's in-app player: the
//! picture fills the panel (never the whole window), with the title at the top and a seek bar
//! and round buttons at the bottom that fade away while it plays.
//!
//! mpv plays the sound, as for every song. ffmpeg decodes the picture into frames that are
//! drawn as a texture, each when the sound reaches it; a seek starts ffmpeg again from there.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use egui_phosphor::regular as icon;

use super::theme::{self, pal};
use super::{Action, Cx};
use crate::model::Source;
use crate::providers::youtube::VideoLink;
use crate::service::{Command, PlayStatus};

/// The biggest picture decoded: the panel is rarely larger, and every frame goes to the GPU.
const MAX_W: u32 = 960;
const MAX_H: u32 = 540;
const MAX_FPS: f64 = 30.0;
/// Frames decoded ahead of the sound (ffmpeg waits once they're there).
const AHEAD: usize = 3;
/// How long the controls stay after the pointer stops moving (Aniko's player: 2.6 s).
const CONTROLS_FOR: f64 = 2.6;

/// A decoded frame, due when the sound reaches `pts` seconds.
pub struct Frame {
    pub pts: f64,
    pub rgba: Vec<u8>,
}

/// ffmpeg decoding a video's picture from one position on.
pub struct Feed {
    child: Child,
    frames: Receiver<Frame>,
    pending: Option<Frame>,
    /// Where it started.
    pub start: f64,
    pub size: [usize; 2],
    ended: Arc<AtomicBool>,
    error: Arc<Mutex<String>>,
    started: Instant,
}

/// The frame size for a `w`×`h` video: as it is, or smaller to fit 960×540, in even numbers.
pub fn frame_size(w: u32, h: u32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (MAX_W, MAX_H);
    }
    let scale = (MAX_W as f64 / w as f64).min(MAX_H as f64 / h as f64).min(1.0);
    let even = |x: f64| ((x.round() as u32) / 2 * 2).max(2);
    (even(w as f64 * scale), even(h as f64 * scale))
}

/// ffmpeg's options to decode `video` from `at` seconds into raw RGBA frames.
pub fn ffmpeg_args(video: &VideoLink, at: f64) -> (Vec<String>, (u32, u32), f64) {
    let (w, h) = frame_size(video.width, video.height);
    let fps = if video.fps > 0.0 {
        video.fps.min(MAX_FPS)
    } else {
        MAX_FPS
    };
    let mut args: Vec<String> = ["-nostdin", "-hide_banner", "-loglevel", "error"]
        .map(String::from)
        .to_vec();
    let url = &video.link.url;
    if url.starts_with("http://") || url.starts_with("https://") {
        args.extend(
            [
                "-reconnect",
                "1",
                "-reconnect_streamed",
                "1",
                "-reconnect_delay_max",
                "4",
            ]
            .map(String::from),
        );
        if let Some(agent) = &video.link.user_agent {
            args.extend(["-user_agent".into(), agent.clone()]);
        }
        if !video.link.headers.is_empty() {
            let headers: String = video
                .link
                .headers
                .iter()
                .map(|(n, v)| format!("{n}: {v}\r\n"))
                .collect();
            args.extend(["-headers".into(), headers]);
        }
    }
    let scale = if video.width == 0 || video.height == 0 {
        format!("scale={w}:{h}:force_original_aspect_ratio=decrease:flags=bilinear,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2")
    } else {
        format!("scale={w}:{h}:flags=bilinear")
    };
    args.extend([
        "-ss".into(),
        format!("{:.3}", at.max(0.0)),
        "-i".into(),
        url.clone(),
        "-map".into(),
        "0:v:0".into(),
        "-an".into(),
        "-sn".into(),
        "-dn".into(),
        "-vf".into(),
        format!("fps={fps},{scale},format=rgba"),
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "rgba".into(),
        "pipe:1".into(),
    ]);
    (args, (w, h), fps)
}

impl Feed {
    pub fn start(ffmpeg: &str, video: &VideoLink, at: f64) -> std::io::Result<Feed> {
        let (args, (w, h), fps) = ffmpeg_args(video, at);
        let mut child = crate::tools::std_command(ffmpeg)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let (tx, frames) = sync_channel(AHEAD);
        let ended = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(String::new()));
        let bytes = w as usize * h as usize * 4;
        {
            let ended = ended.clone();
            std::thread::Builder::new().name("video-frames".into()).spawn(move || {
                let mut n = 0u64;
                loop {
                    let mut rgba = vec![0u8; bytes];
                    if stdout.read_exact(&mut rgba).is_err() {
                        break;
                    }
                    let pts = at + n as f64 / fps;
                    n += 1;
                    // The player went away (another song, a seek): ffmpeg is being stopped.
                    if tx.send(Frame { pts, rgba }).is_err() {
                        break;
                    }
                }
                ended.store(true, Ordering::Relaxed);
            })?;
        }
        {
            // ffmpeg's last complaint, for the panel; read all along so it never blocks on it.
            let error = error.clone();
            std::thread::Builder::new().name("video-errors".into()).spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if !line.trim().is_empty() {
                        *error.lock().unwrap() = line.trim().to_string();
                    }
                }
            })?;
        }
        Ok(Feed {
            child,
            frames,
            pending: None,
            start: at,
            size: [w as usize, h as usize],
            ended,
            error,
            started: Instant::now(),
        })
    }

    /// The newest frame due by `pos` (those before it are skipped), if one came since.
    pub fn frame_at(&mut self, pos: f64) -> Option<Frame> {
        let mut newest = None;
        loop {
            let frame = match self.pending.take() {
                Some(f) => f,
                None => match self.frames.try_recv() {
                    Ok(f) => f,
                    Err(_) => break,
                },
            };
            if frame.pts <= pos + 0.01 {
                newest = Some(frame);
            } else {
                self.pending = Some(frame);
                break;
            }
        }
        newest
    }

    /// When the next frame is due, if it's decoded.
    pub fn next_pts(&mut self) -> Option<f64> {
        if self.pending.is_none() {
            self.pending = self.frames.try_recv().ok();
        }
        self.pending.as_ref().map(|f| f.pts)
    }

    /// ffmpeg stopped (the end of the video, or an error) and every frame has been taken.
    pub fn finished(&self) -> bool {
        self.ended.load(Ordering::Relaxed) && self.pending.is_none()
    }

    pub fn error(&self) -> String {
        self.error.lock().unwrap().clone()
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The player's state, kept across frames while one music video plays.
pub struct VideoPlayer {
    pub track_id: String,
    video: Arc<VideoLink>,
    feed: Option<Feed>,
    texture: Option<egui::TextureHandle>,
    /// The frame shown (its size), once there is one.
    shown: Option<[usize; 2]>,
    failures: u32,
    retry_at: Option<Instant>,
    error: Option<String>,
    controls_until: f64,
    last_pointer: Option<Pos2>,
}

impl VideoPlayer {
    pub fn new(track_id: String, video: Arc<VideoLink>) -> VideoPlayer {
        VideoPlayer {
            track_id,
            video,
            feed: None,
            texture: None,
            shown: None,
            failures: 0,
            retry_at: None,
            error: None,
            controls_until: 0.0,
            last_pointer: None,
        }
    }

    pub fn is_for(&self, track_id: &str, video: &Arc<VideoLink>) -> bool {
        self.track_id == track_id && Arc::ptr_eq(&self.video, video)
    }

    /// Keeps ffmpeg where the sound is and takes the frame due now. Returns when to draw again.
    fn advance(&mut self, ctx: &egui::Context, ffmpeg: &str, pos: f64, duration: f64, playing: bool) -> Duration {
        // Started again after a seek, or when it fell behind, or stopped before the end.
        let restart = match &mut self.feed {
            None => true,
            Some(feed) => {
                let seeked_back = pos < feed.start - 0.5;
                let behind = feed.next_pts().is_some_and(|next| pos - next > 3.0);
                let stopped =
                    feed.finished() && pos < duration - 1.0 && feed.started.elapsed() > Duration::from_secs(2);
                if stopped && self.shown.is_none() {
                    self.failures += 1;
                    self.error = Some(feed.error());
                }
                seeked_back || behind || stopped
            }
        };
        if restart && self.retry_at.is_none_or(|t| Instant::now() >= t) && self.failures < 3 {
            self.feed = None;
            match Feed::start(ffmpeg, &self.video, pos) {
                Ok(feed) => self.feed = Some(feed),
                Err(e) => {
                    self.failures = 3;
                    self.error = Some(format!(
                        "ffmpeg didn't start ({e}). {}",
                        crate::tools::install_hint("ffmpeg")
                    ));
                }
            }
            // Not again for a while, if this one stops too.
            self.retry_at = Some(Instant::now() + Duration::from_secs(3));
        }
        let Some(feed) = &mut self.feed else {
            return Duration::from_millis(250);
        };
        if let Some(frame) = feed.frame_at(pos) {
            let image = egui::ColorImage::from_rgba_premultiplied(feed.size, &frame.rgba);
            match &mut self.texture {
                Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                None => self.texture = Some(ctx.load_texture("music-video", image, egui::TextureOptions::LINEAR)),
            }
            self.shown = Some(feed.size);
            self.error = None;
            self.failures = 0;
        }
        if !playing {
            return Duration::from_millis(250);
        }
        match feed.next_pts() {
            Some(next) => Duration::from_secs_f64((next - pos).clamp(0.004, 0.05)),
            None => Duration::from_millis(15),
        }
    }

    /// Draws the player over `rect` (the centre panel).
    pub fn show(&mut self, ui: &mut Ui, cx: &mut Cx, rect: Rect, ffmpeg: &str) {
        let player = cx.player;
        let pos = player.position_now();
        let playing = player.status == PlayStatus::Playing;
        let wait = self.advance(ui.ctx(), ffmpeg, pos, player.duration, playing);
        ui.ctx().request_repaint_after(wait);

        let painter = ui.painter_at(rect);
        let radius = CornerRadius::same(theme::RADIUS);
        painter.rect_filled(rect, radius, Color32::BLACK);

        // The picture, as big as fits, centred.
        if let (Some(tex), Some([w, h])) = (&self.texture, self.shown) {
            let scale = (rect.width() / w as f32).min(rect.height() / h as f32);
            let size = Vec2::new(w as f32 * scale, h as f32 * scale);
            let img = Rect::from_center_size(rect.center(), size);
            painter.image(
                tex.id(),
                img,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        } else if let Some(err) = &self.error {
            let text = if err.is_empty() {
                "Can't show this video's picture; the song plays on".to_string()
            } else {
                format!("Can't show this video's picture: {err}")
            };
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                theme::font(14.0),
                pal().text_dim,
            );
        } else {
            // Loading: a turning arc, like the player's own spinner.
            let t = ui.input(|i| i.time) as f32;
            let c = rect.center();
            let r = 16.0;
            let points: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a = t * 5.0 + i as f32 / 24.0 * std::f32::consts::PI * 1.4;
                    c + Vec2::new(a.cos(), a.sin()) * r
                })
                .collect();
            painter.add(egui::Shape::line(
                points,
                Stroke::new(3.0, Color32::from_white_alpha(200)),
            ));
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }

        // The controls: shown while paused, or for a moment after the pointer moves.
        let now = ui.input(|i| i.time);
        let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p));
        if pointer.is_some() && pointer != self.last_pointer {
            self.controls_until = now + CONTROLS_FOR;
        }
        self.last_pointer = pointer;
        let wanted = !playing || now < self.controls_until;
        let shown = ui
            .ctx()
            .animate_bool_with_time(ui.id().with("video-controls"), wanted, 0.3);
        if wanted && playing {
            ui.ctx()
                .request_repaint_after(Duration::from_secs_f64((self.controls_until - now).max(0.05)));
        }

        // A click on the picture plays or pauses (under the controls, which come after).
        let surface = ui.interact(rect, ui.id().with("video-surface"), Sense::click());
        if surface.clicked() {
            cx.actions.push(Action::Cmd(Command::TogglePause));
            self.controls_until = now + CONTROLS_FOR;
        }
        if shown > 0.01 {
            self.controls(ui, cx, rect, shown, pos, playing);
        }
    }

    fn controls(&mut self, ui: &mut Ui, cx: &mut Cx, rect: Rect, alpha: f32, pos: f64, playing: bool) {
        let painter = ui.painter_at(rect);
        let a = |c: Color32| c.gamma_multiply(alpha);
        // Shades behind the text, as in Aniko's player.
        gradient(
            &painter,
            Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + 96.0)),
            0.65,
            0.0,
            alpha,
        );
        gradient(
            &painter,
            Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - 132.0), rect.max),
            0.0,
            0.9,
            alpha,
        );

        // Top: back, then the title and how it plays.
        let back = Rect::from_center_size(Pos2::new(rect.min.x + 44.0, rect.min.y + 42.0), Vec2::splat(40.0));
        if round_button(
            ui,
            back,
            icon::ARROW_LEFT,
            false,
            alpha,
            "Back to your page (the song plays on)",
        )
        .clicked()
        {
            cx.actions.push(Action::ShowVideo(false));
        }
        if let Some(track) = &cx.player.current {
            let x = back.max.x + 16.0;
            painter.text(
                Pos2::new(x, back.center().y - 9.0),
                egui::Align2::LEFT_CENTER,
                &track.title,
                theme::bold_font(18.0),
                a(Color32::WHITE),
            );
            // Where it plays from: YouTube, the media server, or a file here.
            let from = match track.source {
                Source::Local => "Video file",
                other => other.label(),
            };
            let mut how = vec![track.artist.clone(), from.to_string(), self.video.label.clone()];
            if let Some(q) = &cx.player.quality {
                how.push(q.label());
            }
            how.retain(|s| !s.is_empty());
            painter.text(
                Pos2::new(x, back.center().y + 12.0),
                egui::Align2::LEFT_CENTER,
                how.join(" · "),
                theme::font(12.5),
                a(Color32::from_white_alpha(170)),
            );
        }

        // The seek bar.
        let duration = cx.player.duration.max(0.0);
        let bar = Rect::from_min_max(
            Pos2::new(rect.min.x + 24.0, rect.max.y - 70.0),
            Pos2::new(rect.max.x - 24.0, rect.max.y - 58.0),
        );
        let seek = ui.interact(bar, ui.id().with("video-seek"), Sense::click_and_drag());
        let hovered = seek.hovered() || seek.dragged();
        let fraction_at = |x: f32| ((x - bar.min.x) / bar.width()).clamp(0.0, 1.0);
        let dragging = seek.dragged().then(|| seek.interact_pointer_pos()).flatten();
        let shown_pos = match dragging {
            Some(p) => fraction_at(p.x) as f64 * duration,
            None => pos,
        };
        if seek.drag_stopped() || seek.clicked() {
            if let Some(p) = seek.interact_pointer_pos() {
                cx.actions
                    .push(Action::Cmd(Command::Seek(fraction_at(p.x) as f64 * duration)));
            }
        }
        let thick = if hovered { 6.0 } else { 4.0 };
        let line = Rect::from_center_size(bar.center(), Vec2::new(bar.width(), thick));
        painter.rect_filled(line, CornerRadius::same(3), a(Color32::from_white_alpha(77)));
        let done = if duration > 0.0 {
            (shown_pos / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let played = Rect::from_min_max(line.min, Pos2::new(line.min.x + line.width() * done, line.max.y));
        painter.rect_filled(played, CornerRadius::same(3), a(cx.accent));
        if hovered {
            painter.circle_filled(Pos2::new(played.max.x, line.center().y), 7.0, a(Color32::WHITE));
            if let Some(p) = seek.hover_pos() {
                let at = fraction_at(p.x) as f64 * duration;
                painter.text(
                    Pos2::new(p.x, bar.min.y - 8.0),
                    egui::Align2::CENTER_BOTTOM,
                    theme::fmt_time(at),
                    theme::font(12.0),
                    a(Color32::WHITE),
                );
            }
        }

        // The buttons: play/pause, previous, back 10 s, forward 10 s, next, and the time.
        let y = rect.max.y - 28.0;
        let mut x = rect.min.x + 24.0 + 20.0;
        let mut button = |ui: &mut Ui, glyph: &str, fill: bool, tip: &str| {
            let r = Rect::from_center_size(Pos2::new(x, y), Vec2::splat(40.0));
            x += 44.0;
            round_button(ui, r, glyph, fill, alpha, tip).clicked()
        };
        if button(
            ui,
            if playing {
                egui_phosphor::fill::PAUSE
            } else {
                egui_phosphor::fill::PLAY
            },
            true,
            if playing { "Pause" } else { "Play" },
        ) {
            cx.actions.push(Action::Cmd(Command::TogglePause));
        }
        if button(ui, icon::SKIP_BACK, false, "Previous") {
            cx.actions.push(Action::Cmd(Command::Previous));
        }
        if button(ui, icon::ARROW_COUNTER_CLOCKWISE, false, "Back 10 seconds") {
            cx.actions.push(Action::Cmd(Command::SeekRelative(-10.0)));
        }
        if button(ui, icon::ARROW_CLOCKWISE, false, "Forward 10 seconds") {
            cx.actions.push(Action::Cmd(Command::SeekRelative(10.0)));
        }
        if button(ui, icon::SKIP_FORWARD, false, "Next") {
            cx.actions.push(Action::Cmd(Command::Next));
        }
        let time = format!("{} / {}", theme::fmt_time(shown_pos), theme::fmt_time(duration));
        painter.text(
            Pos2::new(x - 8.0, y),
            egui::Align2::LEFT_CENTER,
            time,
            theme::font(13.5),
            a(Color32::WHITE),
        );
        // On the right: a YouTube video on YouTube.
        if let Some(t) = cx.player.current.as_ref().filter(|t| t.source == Source::YouTube) {
            let open = Rect::from_center_size(Pos2::new(rect.max.x - 44.0, y), Vec2::splat(40.0));
            if round_button(ui, open, icon::ARROW_SQUARE_OUT, false, alpha, "Open on YouTube").clicked() {
                cx.actions.push(Action::OpenUrl(t.uri.clone()));
            }
        }
        // Moving over the controls keeps them up.
        if ui.rect_contains_pointer(Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - 132.0), rect.max))
            || ui.rect_contains_pointer(Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + 96.0)))
        {
            self.controls_until = self.controls_until.max(ui.input(|i| i.time) + 0.5);
        }
    }
}

/// A 40 px round button on the picture: white 10% behind the icon on hover (always for back).
fn round_button(ui: &mut Ui, rect: Rect, glyph: &str, fill: bool, alpha: f32, tip: &str) -> egui::Response {
    let resp = ui.interact(rect, ui.id().with(("video-button", glyph)), Sense::click());
    let painter = ui.painter();
    let back = glyph == icon::ARROW_LEFT;
    if resp.hovered() || back {
        let a = if resp.hovered() { 46 } else { 26 };
        painter.circle_filled(rect.center(), 20.0, Color32::from_white_alpha(a).gamma_multiply(alpha));
    }
    let font = if fill {
        theme::fill_icon_font(22.0)
    } else {
        theme::icon_font(20.0)
    };
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        font,
        Color32::WHITE.gamma_multiply(alpha),
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.on_hover_text(tip)
}

/// Black from `top` to `bottom` opacity, down `rect`.
fn gradient(painter: &egui::Painter, rect: Rect, top: f32, bottom: f32, alpha: f32) {
    let mut mesh = egui::Mesh::default();
    let c = |o: f32| Color32::from_black_alpha((o * alpha * 255.0) as u8);
    mesh.colored_vertex(rect.left_top(), c(top));
    mesh.colored_vertex(rect.right_top(), c(top));
    mesh.colored_vertex(rect.right_bottom(), c(bottom));
    mesh.colored_vertex(rect.left_bottom(), c(bottom));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::youtube::Link;

    #[test]
    fn frame_sizes() {
        assert_eq!(frame_size(1280, 720), (960, 540));
        assert_eq!(frame_size(640, 360), (640, 360));
        // Portrait videos fit by their height.
        assert_eq!(frame_size(1080, 1920), (304, 540));
        assert_eq!(frame_size(0, 0), (960, 540));
        assert_eq!(frame_size(853, 480), (852, 480));
    }

    #[test]
    fn ffmpeg_options() {
        let video = VideoLink {
            link: Link {
                url: "https://v.example/v?x=1".into(),
                user_agent: Some("UA".into()),
                headers: vec![("Referer".into(), "https://www.youtube.com/".into())],
            },
            width: 1920,
            height: 1080,
            fps: 60.0,
            label: "VP9 1080p".into(),
        };
        let (args, size, fps) = ffmpeg_args(&video, 12.5);
        assert_eq!((size, fps), ((960, 540), 30.0));
        let joined = args.join(" ");
        assert!(joined.contains("-user_agent UA -headers Referer: https://www.youtube.com/\r\n"));
        assert!(joined.contains("-ss 12.500 -i https://v.example/v?x=1"));
        assert!(joined.contains("fps=30,scale=960:540:flags=bilinear,format=rgba"));
        // A file (the tests' videos): no network options.
        let file = VideoLink {
            link: Link {
                url: "/tmp/v.mp4".into(),
                ..Default::default()
            },
            ..video
        };
        assert!(!ffmpeg_args(&file, 0.0).0.iter().any(|a| a == "-reconnect"));
    }

    /// Decodes a generated video with the real ffmpeg (if installed): frames of the right
    /// size, due one after the other from where it started, and skipped up to the position.
    #[test]
    fn real_ffmpeg_frames() {
        if std::process::Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("v.mp4");
        let ok = std::process::Command::new("ffmpeg")
            .args([
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x180:rate=25:duration=6",
            ])
            .arg(&file)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let video = VideoLink {
            link: Link {
                url: file.to_string_lossy().into_owned(),
                ..Default::default()
            },
            width: 320,
            height: 180,
            fps: 25.0,
            label: "H.264 180p".into(),
        };
        let mut feed = Feed::start("ffmpeg", &video, 2.0).unwrap();
        assert_eq!(feed.size, [320, 180]);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut got = None;
        while got.is_none() && Instant::now() < deadline {
            got = feed.frame_at(2.0);
            std::thread::sleep(Duration::from_millis(20));
        }
        let first = got.expect("a frame");
        assert!((first.pts - 2.0).abs() < 1e-9);
        assert_eq!(first.rgba.len(), 320 * 180 * 4);
        // Later: the newest frame due by then, the ones before it skipped.
        std::thread::sleep(Duration::from_millis(300));
        let mut later: Option<Frame> = None;
        while later.as_ref().is_none_or(|f| f.pts < 2.35) && Instant::now() < deadline {
            if let Some(f) = feed.frame_at(2.4) {
                later = Some(f);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let later = later.expect("a later frame");
        assert!(later.pts > 2.3 && later.pts <= 2.41, "pts {}", later.pts);
        // The next one comes after it (once ffmpeg has decoded it).
        let mut next = None;
        while next.is_none() && Instant::now() < deadline {
            next = feed.next_pts();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(next.is_some_and(|p| p > 2.4), "next {next:?}");
    }
}
