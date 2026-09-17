//! Display state detection: check whether the main display is asleep (screen
//! off / blackout) so capture can mark those periods as rest without
//! generating screenshots.

/// Return true if the main display is currently asleep (screen off).
/// Uses CoreGraphics `CGDisplayIsAsleep` — macOS's canonical "display is off"
/// signal. Returns false (i.e. "not asleep") on any FFI failure, so a
/// detection hiccup never blocks normal capture.
#[cfg(target_os = "macos")]
pub fn main_display_asleep() -> bool {
    extern "C" {
        fn CGMainDisplayID() -> u32;
        fn CGDisplayIsAsleep(displayID: u32) -> i32;
    }
    let id = unsafe { CGMainDisplayID() };
    if id == 0 {
        return false;
    }
    let asleep = unsafe { CGDisplayIsAsleep(id) };
    asleep == 1
}

#[cfg(not(target_os = "macos"))]
pub fn main_display_asleep() -> bool {
    false
}
