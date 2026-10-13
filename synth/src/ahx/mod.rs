// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! AHX (Abyss' Highest eXperience, "THX") and HivelyTracker ("HVL"): every
//! sound made as the song plays, from a triangle, sawtooth, square
//! (its pulse width moving) or noise, through one of 63 filters, under
//! an envelope, with a play list stepping each instrument through its
//! waveforms, notes and commands; HivelyTracker adds channels (up to 16),
//! panning and ring modulation.
//!
//! The file is read here into a [`Song`]; `player.rs` plays it, a tick
//! (a 50th of a second, or 2 to 4 times that) at a time, and `waves.rs`
//! makes the waveforms it plays from.

mod player;
mod waves;

use std::sync::Arc;

use crate::replay::{Driver, Tune};

/// Which of the three the file is: AHX 1 ("THX" 0), AHX 2 ("THX" 1) or
/// HivelyTracker ("HVL").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Ahx1,
    Ahx2,
    Hvl,
}

/// A row of a track.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Step {
    /// 0 for none, 1 to 60 (C-1 to B-5).
    pub note: u8,
    /// 0 for none.
    pub instrument: u8,
    pub fx: u8,
    pub param: u8,
    /// HivelyTracker's second effect.
    pub fx2: u8,
    pub param2: u8,
}

/// A position: each channel's track and its transposition.
#[derive(Clone, Debug, Default)]
pub struct Position {
    pub track: Vec<u8>,
    pub transpose: Vec<i8>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Envelope {
    pub a_frames: i32,
    pub a_volume: i32,
    pub d_frames: i32,
    pub d_volume: i32,
    pub s_frames: i32,
    pub r_frames: i32,
    pub r_volume: i32,
}

/// A play list entry: a waveform, a note, two commands.
#[derive(Clone, Copy, Debug, Default)]
pub struct Entry {
    pub fx: [u8; 2],
    pub param: [u8; 2],
    /// 0 keeps the waveform; 1 triangle, 2 sawtooth, 3 square, 4 noise.
    pub waveform: u8,
    /// The note isn't transposed by the track's.
    pub fixed: bool,
    pub note: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Instrument {
    pub name: String,
    pub volume: i32,
    /// The waveform's length: 4 << wave_length points.
    pub wave_length: i32,
    pub envelope: Envelope,
    pub filter_lower: i32,
    pub filter_upper: i32,
    pub filter_speed: i32,
    pub square_lower: i32,
    pub square_upper: i32,
    pub square_speed: i32,
    pub vibrato_delay: i32,
    pub vibrato_depth: i32,
    pub vibrato_speed: i32,
    pub hard_cut_release: bool,
    pub hard_cut_frames: i32,
    pub speed: i32,
    pub list: Vec<Entry>,
}

/// An AHX or HivelyTracker song.
#[derive(Clone, Debug)]
pub struct Song {
    pub kind: Kind,
    pub title: String,
    /// Ticks a 50th of a second.
    pub speed_multiplier: i32,
    pub restart: usize,
    pub channels: usize,
    /// Rows a track.
    pub track_length: usize,
    pub positions: Vec<Position>,
    pub tracks: Vec<Vec<Step>>,
    /// Instrument 0 is none; 1 on are the song's.
    pub instruments: Vec<Instrument>,
    /// Each song's first position (the first is 0).
    pub subsongs: Vec<usize>,
    /// The mix's gain, 256 for 100%.
    pub mix_gain: i32,
    /// HivelyTracker's stereo setting, 0 (mono) to 4 (hard left and right).
    pub stereo: usize,
    file: Arc<Vec<u8>>,
}

struct Reader<'a> {
    d: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> Option<u8> {
        let b = *self.d.get(self.at)?;
        self.at += 1;
        Some(b)
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from(self.u8()?) << 8 | u16::from(self.u8()?))
    }

    /// A string up to its NUL (or the file's end).
    fn text(&mut self) -> String {
        let rest = self.d.get(self.at..).unwrap_or_default();
        let n = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        self.at += (n + 1).min(rest.len());
        rest[..n].iter().map(|&b| if (32..127).contains(&b) || b >= 160 { b as char } else { ' ' }).collect::<String>().trim_end().to_string()
    }
}

/// The file is an AHX or HivelyTracker song (by its mark).
pub fn probe(d: &[u8]) -> bool {
    d.len() >= 14 && matches!((&d[..3], d[3]), (b"THX", 0 | 1) | (b"HVL", 0 | 1))
}

impl Song {
    /// Reads a song; None when it isn't one, or is damaged.
    pub fn parse(d: &[u8]) -> Option<Song> {
        if !probe(d) {
            return None;
        }
        let kind = match (&d[..3], d[3]) {
            (b"THX", 0) => Kind::Ahx1,
            (b"THX", _) => Kind::Ahx2,
            _ => Kind::Hvl,
        };
        let hvl = kind == Kind::Hvl;
        let mut r = Reader { d, at: 4 };
        let names_at = usize::from(r.u16()?);
        let flag = r.u8()?;
        let speed_multiplier = if kind == Kind::Ahx1 { 1 } else { i32::from((flag >> 5) & 3) + 1 };
        let npos = usize::from(flag & 0xF) << 8 | usize::from(r.u8()?);
        let res = usize::from(r.u16()?);
        let (channels, restart) = if hvl { ((res >> 10) + 4, res & 0x3FF) } else { (4, res) };
        let track_length = usize::from(r.u8()?);
        let ntracks = usize::from(r.u8()?);
        let ninstr = usize::from(r.u8()?);
        let nsub = usize::from(r.u8()?);
        let (mix_gain, stereo) = if hvl { ((i32::from(r.u8()?) << 8) / 100, usize::from(r.u8()?).min(4)) } else { (256, 4) };
        if !(1..=999).contains(&npos) || restart >= npos || !(1..=64).contains(&track_length) || ninstr > 63 || channels > 16 {
            return None;
        }
        let mut subsongs = vec![0];
        for _ in 0..nsub {
            let s = usize::from(r.u16()?);
            if s < npos {
                subsongs.push(s);
            }
        }
        let mut positions = Vec::with_capacity(npos);
        for _ in 0..npos {
            let mut p = Position { track: Vec::with_capacity(channels), transpose: Vec::with_capacity(channels) };
            for _ in 0..channels {
                p.track.push(r.u8()?);
                p.transpose.push(r.u8()? as i8);
            }
            positions.push(p);
        }
        let mut tracks = Vec::with_capacity(ntracks + 1);
        for t in 0..=ntracks {
            let mut steps = vec![Step::default(); track_length];
            // Track 0 isn't saved when it's empty (the flag's top bit).
            if !(flag & 0x80 != 0 && t == 0) {
                for s in steps.iter_mut() {
                    *s = if hvl {
                        let b1 = r.u8()?;
                        if b1 == 0x3F {
                            Step::default()
                        } else {
                            let (b2, b3, b4, b5) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
                            Step { note: b1, instrument: b2, fx: b3 >> 4, param: b4, fx2: b3 & 0xF, param2: b5 }
                        }
                    } else {
                        let (b1, b2, b3) = (r.u8()?, r.u8()?, r.u8()?);
                        Step { note: (b1 >> 2) & 0x3F, instrument: (b1 & 3) << 4 | b2 >> 4, fx: b2 & 0xF, param: b3, fx2: 0, param2: 0 }
                    };
                }
            }
            tracks.push(steps);
        }
        let mut instruments = vec![Instrument::default()];
        for _ in 0..ninstr {
            let mut ins = Instrument { volume: i32::from(r.u8()?), ..Default::default() };
            let b1 = r.u8()?;
            ins.wave_length = i32::from(b1 & 7);
            let e = &mut ins.envelope;
            e.a_frames = i32::from(r.u8()?);
            e.a_volume = i32::from(r.u8()?);
            e.d_frames = i32::from(r.u8()?);
            e.d_volume = i32::from(r.u8()?);
            e.s_frames = i32::from(r.u8()?);
            e.r_frames = i32::from(r.u8()?);
            e.r_volume = i32::from(r.u8()?);
            r.at += 3;
            let b12 = r.u8()?;
            if kind != Kind::Ahx1 {
                ins.filter_speed = i32::from((b1 >> 3) & 0x1F) | i32::from((b12 >> 2) & 0x20);
                ins.filter_lower = i32::from(b12 & 0x7F);
            }
            ins.vibrato_delay = i32::from(r.u8()?);
            let b14 = r.u8()?;
            if kind != Kind::Ahx1 {
                ins.hard_cut_frames = i32::from((b14 >> 4) & 7);
                ins.hard_cut_release = b14 & 0x80 != 0;
            }
            ins.vibrato_depth = i32::from(b14 & 0xF);
            ins.vibrato_speed = i32::from(r.u8()?);
            ins.square_lower = i32::from(r.u8()?);
            ins.square_upper = i32::from(r.u8()?);
            ins.square_speed = i32::from(r.u8()?);
            let b19 = r.u8()?;
            if kind != Kind::Ahx1 {
                ins.filter_speed |= i32::from((b19 >> 1) & 0x40);
                ins.filter_upper = i32::from(b19 & 0x3F);
            }
            ins.speed = i32::from(r.u8()?);
            let len = usize::from(r.u8()?);
            for _ in 0..len {
                let (b1, b2, b3, b4) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
                ins.list.push(if hvl {
                    let b5 = r.u8()?;
                    Entry { fx: [b1 & 0xF, (b2 >> 3) & 0xF], waveform: b2 & 7, fixed: (b3 >> 6) & 1 != 0, note: b3 & 0x3F, param: [b4, b5] }
                } else {
                    // AHX's 3-bit commands: 6 and 7 stand for C and F.
                    let wide = |f: u8| match f {
                        6 => 12,
                        7 => 15,
                        f => f,
                    };
                    Entry { fx: [wide((b1 >> 2) & 7), wide((b1 >> 5) & 7)], waveform: (b1 << 1) & 6 | b2 >> 7, fixed: (b2 >> 6) & 1 != 0, note: b2 & 0x3F, param: [b3, b4] }
                });
            }
            instruments.push(ins);
        }
        // The names: the title, then each instrument's.
        if names_at >= r.at && names_at < d.len() {
            r.at = names_at;
        }
        let title = r.text();
        for ins in instruments.iter_mut().skip(1) {
            ins.name = r.text();
        }
        Some(Song { kind, title, speed_multiplier, restart, channels, track_length, positions, tracks, instruments, subsongs, mix_gain, stereo, file: Arc::new(d.to_vec()) })
    }

    /// The step a channel plays at a position and row (an empty one past the tracks).
    pub fn step(&self, position: usize, channel: usize, row: usize) -> Step {
        let Some(p) = self.positions.get(position) else { return Step::default() };
        let t = usize::from(p.track.get(channel).copied().unwrap_or(0));
        self.tracks.get(t).and_then(|t| t.get(row)).copied().unwrap_or_default()
    }
}

impl Tune for Song {
    fn format(&self) -> &'static str {
        if self.kind == Kind::Hvl {
            "hvl"
        } else {
            "ahx"
        }
    }

    fn file(&self) -> &[u8] {
        &self.file
    }

    fn songs(&self) -> usize {
        self.subsongs.len()
    }

    fn driver(&self, song: usize) -> Box<dyn Driver> {
        Box::new(player::Player::new(Arc::new(self.clone()), self.subsongs.get(song).copied().unwrap_or(0)))
    }
}
