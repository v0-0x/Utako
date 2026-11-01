// iOS can't run mpv: a player inside the app takes its place, with the same interface.
#[cfg(not(target_os = "ios"))]
pub mod mpv;
#[cfg(target_os = "ios")]
#[path = "native.rs"]
pub mod mpv;
pub mod oauth;
pub mod queue;
pub mod spotify;
