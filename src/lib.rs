//! Run a Windows game (.exe) through Proton, outside Steam, inside a bubblewrap sandbox that shows
//! it the system read-only and lets it write to its own folders alone.
//!
//! The command line is [`cli`]: [`cli::run`] is a whole run, from the arguments to the game's exit
//! status, and [`cli::Embedding`] is what a program running it brings along (the name its lines
//! are signed with, a Proton of its own). Below it, each part on its own:
//!
//! - [`proton`] — every Proton on the computer, and which one a run takes;
//! - [`steam`] — what Steam keeps: its libraries, its games, and the prefixes it made for them;
//! - `prefix` — each game's own folder, and going on from another launcher's prefix;
//! - `folders` — the folders too broad for a game to write to: the user's own, a whole filesystem;
//! - `sandbox` — bubblewrap's arguments, and the sandbox's first step, which sets the syscall
//!   filter (`seccomp`) in place before Proton starts.
//!
//! That first step is the running program started again inside the sandbox: a program embedding
//! this hands its process to [`serve_filtered_if_asked`] before anything else, as the
//! `proton_sandboxed` binary does.

#[cfg(feature = "cli")]
pub mod cli;
#[cfg(feature = "cli")]
mod folders;
#[cfg(feature = "cli")]
mod game;
#[cfg(feature = "cli")]
mod prefix;
pub mod proton;
#[cfg(feature = "cli")]
mod sandbox;
#[cfg(feature = "cli")]
mod seccomp;
pub mod steam;

use std::path::{Path, PathBuf};

#[cfg(feature = "cli")]
pub use sandbox::serve_filtered_if_asked;

/// The user's home folder (empty if somehow unset — callers join onto it).
pub fn home() -> PathBuf {
    #[allow(deprecated)] // no longer deprecated from Rust 1.86; this crate's minimum, 1.85, warns
    std::env::home_dir().unwrap_or_default()
}

/// `path` with its links resolved, where it exists; as it is otherwise.
pub(crate) fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
