//! Files the operating system asks the running program to open.

use std::path::PathBuf;
use std::sync::Mutex;

static QUEUE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Called by the platform code when a file should be opened.
#[allow(dead_code)]
pub fn push(path: PathBuf) {
    if let Ok(mut queue) = QUEUE.lock() {
        queue.push(path);
    }
}

/// The files that have arrived since the last call.
pub fn take() -> Vec<PathBuf> {
    QUEUE.lock().map(|mut queue| std::mem::take(&mut *queue)).unwrap_or_default()
}

#[cfg(target_os = "macos")]
#[path = "open_files_mac.rs"]
mod mac;

/// Start listening for "open this file" requests. Call once at start-up.
pub fn install() {
    #[cfg(target_os = "macos")]
    mac::install();
}
