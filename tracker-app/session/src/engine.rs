// Copyright 2026 ansiapps. Neumetik Tracker: MIT licensed, see the LICENSE at the repository root.

//! The sound: the song's player and a sample heard on its own, run where
//! the platform makes its sound (an audio callback, an AudioWorklet).
//! The session (`Session`) sends it `Cmd`s; it never allocates as it
//! renders.

use std::sync::Arc;

use neumetik_tracker::dsp::Rack;
use neumetik_tracker::player::Interp;
use neumetik_tracker::{Module, Player, NOTE_OFF};

use crate::dto::Status;

/// What the session asks of the sound.
pub enum Cmd {
    /// A song to play (made idle: only notes played on it sound until
    /// `Play`), in place of the one there; None takes it away.
    Load(Option<Box<Player>>),
    /// The song changed (an edit): it plays on from where it is. When its
    /// effect slots changed, they come made ready, made off the audio thread.
    Module(Arc<Module>, Option<Option<Box<Rack>>>),
    /// One row played, its notes let go after it.
    PlayRow { order: usize, row: usize },
    /// A channel's fader as it plays: a gain, and a panning or the song's.
    Fader(usize, f32, Option<f32>),
    /// How samples are read, and the Amiga's filters on a MOD.
    Playback(Interp, bool),
    /// Plays from an order and row; `hold` keeps to that order's pattern.
    Play { order: usize, row: usize, hold: bool },
    Stop,
    /// A note played from the keyboard, with the instrument (or sample);
    /// `NOTE_OFF` lets it go.
    Jam(u8, u8),
    Mute(usize, bool),
    /// A sample heard on its own (None stops it).
    Preview(Option<Box<Player>>),
}

/// The most channels a song has: room for their levels, made up front.
const CHANNELS: usize = 256;

#[derive(Default)]
pub struct Engine {
    song: Option<Box<Player>>,
    preview: Option<Box<Player>>,
    levels: Vec<f32>,
}

impl Engine {
    pub fn new() -> Engine {
        Engine { song: None, preview: None, levels: vec![0.0; CHANNELS] }
    }

    pub fn apply(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Load(p) => self.song = p,
            Cmd::Preview(p) => self.preview = p,
            cmd => {
                let Some(p) = self.song.as_deref_mut() else { return };
                match cmd {
                    Cmd::Module(m, rack) => p.set_module_with(m, rack),
                    Cmd::PlayRow { order, row } => p.play_row(order, row),
                    Cmd::Fader(ch, gain, pan) => p.set_fader(ch, gain, pan),
                    Cmd::Playback(interp, amiga) => {
                        p.interp = interp;
                        p.amiga = amiga;
                    }
                    Cmd::Play { order, row, hold } => p.play_from(order, row, hold),
                    Cmd::Stop => p.stop(),
                    Cmd::Jam(NOTE_OFF, _) => p.jam_off(),
                    Cmd::Jam(note, instr) => p.jam(note, instr),
                    Cmd::Mute(ch, on) => p.set_muted(ch, on),
                    Cmd::Load(_) | Cmd::Preview(_) => {}
                }
            }
        }
    }

    /// Adds the next `left.len()` frames into `left` and `right`.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        if let Some(p) = self.song.as_deref_mut() {
            p.render(left, right);
        }
        if let Some(p) = self.preview.as_deref_mut() {
            p.render(left, right);
            if p.ended() {
                self.preview = None;
            }
        }
    }

    /// Where the song is, and each channel's level, into `out` (its levels
    /// resized only when the song has more channels than it had room for).
    pub fn status(&mut self, out: &mut Status) {
        match self.song.as_deref() {
            Some(p) => {
                let pos = p.position();
                out.playing = p.playing();
                out.order = pos.order;
                out.row = pos.row;
                let n = p.module().channels.len().min(self.levels.len());
                p.levels(&mut self.levels[..n]);
                out.levels.clear();
                out.levels.extend_from_slice(&self.levels[..n]);
            }
            None => {
                out.playing = false;
                out.levels.clear();
            }
        }
    }
}
