// Copyright 2026 ansiapps. Neumetik Tracker: MIT licensed, see the LICENSE at the repository root.

//! **Neumetik Tracker**: music tracker modules read, played and edited,
//! and every sample they use kept once.
//!
//! - [`load`] reads a module in any of the formats in
//!   [`formats::FORMATS`] into one [`Module`] model, written from the
//!   formats' public descriptions with no dependencies.
//! - [`Player`] plays a module, tick by tick, the way its format's
//!   tracker did.
//! - [`umf`], the Universal Mod Format: a module that refers to its
//!   samples in the repository rather than holding them (or holds them,
//!   to play anywhere).

pub mod canon;
pub mod dsp;
pub mod edit;
pub mod export;
pub mod formats;
pub mod hash;
pub mod io;
pub mod lz;
pub mod model;
#[cfg(feature = "repository")]
pub mod modland;
#[cfg(feature = "repository")]
pub mod parity;
pub mod player;
#[cfg(feature = "repository")]
pub mod repo;
pub mod sample_edit;
pub mod samplefile;
pub mod sauce;
pub mod umf;
pub mod unpack;
pub mod wav;
pub mod vorbis;
pub mod wavefile;
pub mod wavepack;

pub use io::{Error, Result};
pub use model::*;
pub use player::{Player, Position};

/// Reads a module from its bytes; `name`, the file's name, helps only
/// with formats that have no mark of their own.
pub fn load(data: &[u8], name: &str) -> Result<Module> {
    formats::load(data, name)
}

/// `load`, with the files beside the module to hand, for the formats
/// that keep their samples in another file: `companion` gives a file's
/// bytes by its name (`beside` makes one for a module on disk).
pub fn load_with(data: &[u8], name: &str, companion: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Module> {
    formats::load_with(data, name, companion)
}

/// The files in the same folder as `path`, by name, whatever their case.
pub fn beside(path: &std::path::Path) -> impl Fn(&str) -> Option<Vec<u8>> {
    let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    move |name: &str| {
        let exact = dir.join(name);
        if let Ok(b) = std::fs::read(&exact) {
            return Some(b);
        }
        let entries = std::fs::read_dir(&dir).ok()?;
        entries.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name)).and_then(|e| std::fs::read(e.path()).ok())
    }
}
