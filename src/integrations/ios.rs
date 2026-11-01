//! iOS: the audio session that keeps Utako playing with the screen locked or another app in
//! front, and the song and controls on the lock screen, in Control Center and on headphones
//! (MediaPlayer's Now Playing and remote commands). Plain Objective-C messages through objc2,
//! so no binding crates are needed.

use std::ffi::{c_void, CStr, CString};

use block2::RcBlock;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use tokio::sync::mpsc::UnboundedSender;

use crate::model::Track;
use crate::service::Command;

#[link(name = "AVFoundation", kind = "framework")]
#[link(name = "MediaPlayer", kind = "framework")]
extern "C" {}

/// libdispatch's queue type, only ever used through a pointer.
#[repr(C)]
struct DispatchQueue {
    _private: [u8; 0],
}

extern "C" {
    static _dispatch_main_q: DispatchQueue;
    fn dispatch_async_f(queue: *const DispatchQueue, context: *mut c_void, work: extern "C" fn(*mut c_void));
}

/// Runs `work` on the main thread, where UIKit and MediaPlayer expect to be called.
fn on_main(work: impl FnOnce() + Send + 'static) {
    extern "C" fn run(context: *mut c_void) {
        // SAFETY: `context` is the box made below, handed over exactly once.
        let work = unsafe { Box::from_raw(context.cast::<Box<dyn FnOnce() + Send>>()) };
        work();
    }
    let work: Box<Box<dyn FnOnce() + Send>> = Box::new(Box::new(work));
    // SAFETY: the main queue lives for the whole process; `run` takes the box back.
    unsafe { dispatch_async_f(std::ptr::addr_of!(_dispatch_main_q), Box::into_raw(work).cast(), run) };
}

fn class(name: &CStr) -> Option<&'static AnyClass> {
    AnyClass::get(name)
}

fn ns_string(text: &str) -> Option<Retained<AnyObject>> {
    let text = CString::new(text.replace('\0', "")).ok()?;
    let cls = class(c"NSString")?;
    // SAFETY: a valid NUL-terminated UTF-8 string; the result is autoreleased and retained here.
    unsafe { msg_send![cls, stringWithUTF8String: text.as_ptr()] }
}

fn ns_number(value: f64) -> Option<Retained<AnyObject>> {
    let cls = class(c"NSNumber")?;
    // SAFETY: numberWithDouble: takes a double and returns an autoreleased NSNumber.
    unsafe { msg_send![cls, numberWithDouble: value] }
}

/// Sets the audio session to "playback": Utako keeps playing in the background (with the
/// audio background mode in Info.plist) and ignores the ring/silent switch, like other music
/// apps. Called once at start, before any audio is opened.
pub fn start_audio_session() {
    let Some(av) = class(c"AVAudioSession") else {
        tracing::warn!("no AVAudioSession");
        return;
    };
    let Some(category) = ns_string("AVAudioSessionCategoryPlayback") else {
        return;
    };
    // SAFETY: plain AVAudioSession calls; errors are reported through the return value only.
    unsafe {
        let session: Option<Retained<AnyObject>> = msg_send![av, sharedInstance];
        let Some(session) = session else { return };
        let none: *mut *mut AnyObject = std::ptr::null_mut();
        let set: bool = msg_send![&session, setCategory: &*category, error: none];
        let active: bool = msg_send![&session, setActive: true, error: none];
        tracing::info!("audio session: playback category {set}, active {active}");
    }
}

/// The lock screen's Now Playing and remote controls.
pub struct NowPlaying;

impl NowPlaying {
    pub fn new(commands: UnboundedSender<Command>) -> NowPlaying {
        on_main(move || register_commands(commands));
        NowPlaying
    }

    pub fn update(&mut self, track: Option<&Track>, playing: bool, position: f64) {
        let song = track.map(|t| {
            (
                t.title.clone(),
                t.artist.clone(),
                t.album.clone(),
                t.duration_ms as f64 / 1000.0,
            )
        });
        on_main(move || set_now_playing(song, playing, position));
    }
}

fn set_now_playing(song: Option<(String, String, String, f64)>, playing: bool, position: f64) {
    let Some(center_class) = class(c"MPNowPlayingInfoCenter") else {
        return;
    };
    // SAFETY: MediaPlayer calls on the main thread with Foundation objects made here.
    unsafe {
        let center: Option<Retained<AnyObject>> = msg_send![center_class, defaultCenter];
        let Some(center) = center else { return };
        let Some((title, artist, album, duration)) = song else {
            let nothing: *const AnyObject = std::ptr::null();
            let _: () = msg_send![&center, setNowPlayingInfo: nothing];
            return;
        };
        let Some(dict_class) = class(c"NSMutableDictionary") else {
            return;
        };
        let info: Option<Retained<AnyObject>> = msg_send![dict_class, new];
        let Some(info) = info else { return };
        let put = |key: &str, value: Option<Retained<AnyObject>>| {
            if let (Some(key), Some(value)) = (ns_string(key), value) {
                let _: () = msg_send![&info, setObject: &*value, forKey: &*key];
            }
        };
        // The values of MPMediaItemPropertyTitle and friends.
        put("title", ns_string(&title));
        put("artist", ns_string(&artist));
        if !album.is_empty() {
            put("albumTitle", ns_string(&album));
        }
        if duration > 0.0 {
            put("playbackDuration", ns_number(duration));
        }
        put(
            "MPNowPlayingInfoPropertyElapsedPlaybackTime",
            ns_number(position.max(0.0)),
        );
        put(
            "MPNowPlayingInfoPropertyPlaybackRate",
            ns_number(if playing { 1.0 } else { 0.0 }),
        );
        let _: () = msg_send![&center, setNowPlayingInfo: &*info];
    }
}

/// Play, pause, next, previous and the lock screen's seek bar, sent to the service.
fn register_commands(commands: UnboundedSender<Command>) {
    let Some(center_class) = class(c"MPRemoteCommandCenter") else {
        return;
    };
    // SAFETY: MediaPlayer calls on the main thread; the handlers live as long as the commands.
    unsafe {
        let center: Option<Retained<AnyObject>> = msg_send![center_class, sharedCommandCenter];
        let Some(center) = center else { return };
        let simple: [(Option<Retained<AnyObject>>, fn() -> Command); 5] = [
            (msg_send![&center, playCommand], || Command::Resume),
            (msg_send![&center, pauseCommand], || Command::Pause),
            (msg_send![&center, togglePlayPauseCommand], || Command::TogglePause),
            (msg_send![&center, nextTrackCommand], || Command::Next),
            (msg_send![&center, previousTrackCommand], || Command::Previous),
        ];
        for (remote, command) in simple {
            let Some(remote) = remote else { continue };
            let tx = commands.clone();
            let handler = RcBlock::new(move |_event: *mut AnyObject| -> isize {
                let _ = tx.send(command());
                0 // MPRemoteCommandHandlerStatusSuccess
            });
            let _: () = msg_send![&remote, setEnabled: true];
            let _: Option<Retained<AnyObject>> = msg_send![&remote, addTargetWithHandler: &*handler];
        }
        let seek: Option<Retained<AnyObject>> = msg_send![&center, changePlaybackPositionCommand];
        if let Some(seek) = seek {
            let tx = commands.clone();
            let handler = RcBlock::new(move |event: *mut AnyObject| -> isize {
                if event.is_null() {
                    return 1; // MPRemoteCommandHandlerStatusCommandFailed
                }
                let at: f64 = msg_send![&*event, positionTime];
                let _ = tx.send(Command::Seek(at));
                0
            });
            let _: () = msg_send![&seek, setEnabled: true];
            let _: Option<Retained<AnyObject>> = msg_send![&seek, addTargetWithHandler: &*handler];
        }
    }
}
