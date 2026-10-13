// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! Digital Mugician 1 and 2 (Rob Hubbard's and Thomas Hermann's?, by
//! Softeyes): up to eight songs, each a sequence of 64-row tracks a
//! channel (transposed); instruments are samples or 128-point waveforms
//! remade as they play by one of fifteen effects (filters, mixes, scrolls,
//! up and down sampling, negation, morphs, gates), under a volume read
//! from a waveform, with arpeggios, a pitch read from a waveform and
//! bends. Version 2 plays seven channels: three as they are, four more
//! mixed into one of the Amiga's (each at its full level), and ticks
//! once a mixing buffer.

use std::sync::Arc;

use crate::paula::{Paula, CIA_50, CLOCK};

/// Version 2's mixing rate, as a period (UADE's player's, about 16 kHz).
const MIX_PERIOD: u32 = 224;
use crate::replay::{Bufs, Driver, Tune};

/// Mugician's periods: 64 notes in each of 16 fine-tunes (a sixteenth of a
/// semitone apart), 4,825 at the bottom; three of its own off the curve.
pub fn period(i: usize) -> u16 {
    let (k, j) = (i / 64, i % 64);
    match (k, j) {
        (0, 4) => 3820,
        (0, 30) => 854,
        (11, 30) => 887,
        _ => (4825.0 * 2f64.powf(-(j as f64) / 12.0) * 2f64.powf(k as f64 / 192.0)).round() as u16,
    }
}

const PERIODS: usize = 1024;
/// Where note 0 is in the table.
const START: i32 = 7;

#[derive(Clone, Copy, Debug, Default)]
pub struct SubSong {
    pub looped: bool,
    pub loop_position: usize,
    pub speed: u8,
    pub length: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Instrument {
    pub waveform: u8,
    pub loop_length: usize,
    pub volume: u8,
    pub volume_speed: u8,
    pub arpeggio: u8,
    pub pitch: u8,
    pub effect_index: usize,
    pub delay: u8,
    pub finetune: u8,
    pub pitch_loop: u8,
    pub pitch_speed: u8,
    pub effect: u8,
    pub source1: u8,
    pub source2: u8,
    pub effect_speed: u8,
    pub volume_loop: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Row {
    pub note: u8,
    pub instrument: u8,
    pub effect: u8,
    pub param: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Sample {
    pub at: usize,
    pub len: usize,
    pub loop_start: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Song {
    /// Version 2 or 3 (seven channels).
    pub v2: bool,
    /// Version 3 (Moonchild's), which ticks faster.
    pub v3: bool,
    pub names: Vec<String>,
    pub subsongs: Vec<SubSong>,
    /// Each sub-song's sequence: per position, each of four channels' (track, transpose).
    pub sequences: Vec<Vec<[(u8, i8); 4]>>,
    pub instruments: Vec<Instrument>,
    /// Where each 128-point waveform is in memory.
    pub waveforms: Vec<usize>,
    pub samples: Vec<Sample>,
    pub tracks: Vec<[Row; 64]>,
    pub arpeggios: [[u8; 32]; 8],
    /// The sub-songs that play (for version 2, each the first of a pair).
    pub playable: Vec<usize>,
    pub mem: Vec<i8>,
    file: Arc<Vec<u8>>,
}

fn be16(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(i..i + 2)?.try_into().ok()?))
}

fn be32(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?))
}

pub fn probe(d: &[u8]) -> bool {
    d.len() >= 204 && matches!(&d[..24], b" MUGICIAN/SOFTEYES 1990 " | b" MUGICIAN2/SOFTEYES 1990" | b" MUGICIAN3/SOFTEYES 1990")
}

impl Song {
    pub fn parse(d: &[u8]) -> Option<Song> {
        if !probe(d) {
            return None;
        }
        let v2 = &d[..24] != b" MUGICIAN/SOFTEYES 1990 ";
        let v3 = &d[..24] == b" MUGICIAN3/SOFTEYES 1990";
        let arps = be16(d, 24)? != 0;
        let ntracks = usize::from(be16(d, 26)?);
        let seq_len: Vec<usize> = (0..8).map(|i| be32(d, 28 + i * 4).map(|v| v as usize)).collect::<Option<_>>()?;
        let ninstr = be32(d, 60)? as usize;
        let nwaves = be32(d, 64)? as usize;
        let nsamples = be32(d, 68)? as usize;
        let samples_size = be32(d, 72)? as usize;
        if ninstr > 256 || nwaves > 256 || nsamples > 256 || ntracks > 1024 {
            return None;
        }
        let mut at = 76;
        let mut subsongs = Vec::with_capacity(8);
        let mut names = Vec::with_capacity(8);
        for _ in 0..8 {
            let b = d.get(at..at + 16)?;
            subsongs.push(SubSong { looped: b[0] != 0, loop_position: usize::from(b[1]), speed: b[2], length: usize::from(b[3]) });
            names.push(b[4..16].iter().map(|&c| if (32..127).contains(&c) { c as char } else { ' ' }).collect::<String>().trim().to_string());
            at += 16;
        }
        let mut sequences = Vec::with_capacity(8);
        for &len in &seq_len {
            let mut seq = Vec::with_capacity(len);
            for _ in 0..len {
                let mut p = [(0u8, 0i8); 4];
                for c in p.iter_mut() {
                    *c = (*d.get(at)?, *d.get(at + 1)? as i8);
                    at += 2;
                }
                seq.push(p);
            }
            sequences.push(seq);
        }
        let mut instruments = Vec::with_capacity(ninstr);
        for _ in 0..ninstr {
            let b = d.get(at..at + 16)?;
            instruments.push(Instrument {
                waveform: b[0],
                loop_length: usize::from(b[1]) * 2,
                volume: b[2],
                volume_speed: b[3],
                arpeggio: b[4],
                pitch: b[5],
                effect_index: usize::from(b[6]),
                delay: b[7],
                finetune: b[8],
                pitch_loop: b[9],
                pitch_speed: b[10],
                effect: b[11],
                source1: b[12],
                source2: b[13],
                effect_speed: b[14],
                volume_loop: b[15] != 0,
            });
            at += 16;
        }
        let mut mem: Vec<i8> = Vec::new();
        let mut waveforms = Vec::with_capacity(nwaves);
        for _ in 0..nwaves {
            waveforms.push(mem.len());
            mem.extend(d.get(at..at + 128)?.iter().map(|&x| x as i8));
            at += 128;
        }
        if waveforms.is_empty() {
            waveforms.push(mem.len());
            mem.resize(128, 0);
        }
        let mut heads = Vec::with_capacity(nsamples);
        for _ in 0..nsamples {
            heads.push((be32(d, at)? as usize, be32(d, at + 4)? as usize, be32(d, at + 8)? as i32));
            at += 32;
        }
        let mut tracks = Vec::with_capacity(ntracks);
        for _ in 0..ntracks {
            let mut t = [Row::default(); 64];
            for r in t.iter_mut() {
                let b = d.get(at..at + 4)?;
                *r = Row { note: b[0], instrument: b[1], effect: b[2], param: b[3] };
                at += 4;
            }
            tracks.push(t);
        }
        if tracks.is_empty() {
            tracks.push([Row::default(); 64]);
        }
        let base = at;
        let mut samples = Vec::with_capacity(nsamples);
        for (start, end, ls) in heads {
            let len = end.saturating_sub(start);
            let s = Sample { at: mem.len(), len, loop_start: (ls != 0).then(|| (ls as usize).saturating_sub(start)).filter(|&l| l < len) };
            mem.extend(d.get(base + start..(base + start + len).min(d.len())).unwrap_or_default().iter().map(|&x| x as i8));
            mem.resize(s.at + len, 0);
            samples.push(s);
        }
        let mut arpeggios = [[0u8; 32]; 8];
        if arps {
            let p = base + samples_size;
            for (k, a) in arpeggios.iter_mut().enumerate() {
                if let Some(b) = d.get(p + k * 32..p + k * 32 + 32) {
                    a.copy_from_slice(b);
                }
            }
        }
        for seq in &mut sequences {
            for p in seq.iter_mut() {
                for c in p.iter_mut() {
                    if usize::from(c.0) >= tracks.len() {
                        c.0 = 0;
                    }
                }
            }
        }
        // The sub-songs worth playing: whole, and not empty.
        let step = if v2 { 2 } else { 1 };
        let playable: Vec<usize> = (0..8).step_by(step).filter(|&i| seq_len[i] == subsongs[i].length && sequences[i].iter().any(|p| p.iter().any(|&(t, tr)| t != 0 || tr != 0))).collect();
        if playable.is_empty() {
            return None;
        }
        Some(Song { v2, v3, names, subsongs, sequences, instruments, waveforms, samples, tracks, arpeggios, playable, mem, file: Arc::new(d.to_vec()) })
    }

    pub fn channels(&self) -> usize {
        if self.v2 {
            7
        } else {
            4
        }
    }

    /// A channel's (track, transpose) at a position of a playable song.
    pub fn step(&self, song: usize, position: usize, channel: usize) -> (u8, i8) {
        let s = self.playable[song.min(self.playable.len() - 1)];
        let (seq, c) = if channel < 3 || !self.v2 { (s, channel) } else { (s + 1, channel - 3) };
        self.sequences.get(seq).and_then(|q| q.get(position)).map_or((0, 0), |p| p[c.min(3)])
    }
}

impl Tune for Song {
    fn format(&self) -> &'static str {
        "dmu"
    }
    fn file(&self) -> &[u8] {
        &self.file
    }
    fn songs(&self) -> usize {
        self.playable.len()
    }
    fn driver(&self, song: usize) -> Box<dyn Driver> {
        Box::new(Player::new(Arc::new(self.clone()), song.min(self.playable.len() - 1)))
    }
}

/// What a row's effect does: a pitch bend (below 64, to that note), or
/// one of these (64 on, less 62).
mod fx {
    pub const BEND: u8 = 1;
    pub const NO_INSTRUMENT_EFFECT: u8 = 2;
    pub const NO_INSTRUMENT_VOLUME: u8 = 3;
    pub const NO_EFFECT_OR_VOLUME: u8 = 4;
    pub const PATTERN_LENGTH: u8 = 5;
    pub const SONG_SPEED: u8 = 6;
    pub const NO_DMA: u8 = 10;
    pub const ARPEGGIO: u8 = 11;
    pub const NO_WANDER: u8 = 12;
    pub const SHUFFLE: u8 = 13;
}

#[derive(Clone, Default)]
struct Voice {
    track: u8,
    transpose: i8,
    note: u8,
    instrument: u16,
    effect: u8,
    param: u8,
    finetune: u8,
    bend_end: u16,
    bend: i16,
    period: u16,
    vol_counter: u8,
    vol_index: usize,
    delay: u8,
    pitch_index: usize,
    arp_index: usize,
    effect_speed: u8,
}

pub struct Player {
    song: Arc<Song>,
    paula: Paula,
    instruments: Vec<Instrument>,
    sub: usize,
    voices: Vec<Voice>,
    speed: u16,
    current_speed: u16,
    shown_speed: u16,
    new_pattern: bool,
    new_row: bool,
    position: usize,
    length: usize,
    row: usize,
    pattern_length: usize,
    shown: (usize, usize),
    done: bool,
}

impl Player {
    fn new(song: Arc<Song>, sub: usize) -> Player {
        let n = song.channels();
        let mut paula = Paula::new(n, song.mem.clone());
        if n == 7 {
            for c in 3..7 {
                paula.set_pan(c, 0.0);
            }
        }
        let mut p = Player { paula, instruments: song.instruments.clone(), sub, voices: vec![Voice::default(); n], speed: 0, current_speed: 0, shown_speed: 0, new_pattern: true, new_row: true, position: 0, length: 0, row: 0, pattern_length: 64, shown: (0, 0), done: false, song };
        p.start();
        p
    }

    fn start(&mut self) {
        let ss = self.song.subsongs[self.song.playable[self.sub]];
        self.speed = u16::from(ss.speed);
        self.current_speed = u16::from(ss.speed & 0x0F) << 4 | u16::from(ss.speed & 0x0F);
        self.shown_speed = self.current_speed;
        self.new_pattern = true;
        self.new_row = true;
        self.position = 0;
        self.length = ss.length;
        self.row = 0;
        self.pattern_length = 64;
        let n = self.voices.len();
        self.voices = vec![Voice::default(); n];
    }

    fn change_speed(&mut self, s: u16) {
        if s != self.shown_speed {
            self.shown_speed = s;
            self.current_speed = s;
        }
    }

    fn get_period(note: i32, transpose: i8, finetune: u8) -> u16 {
        let i = START + note + i32::from(transpose) + i32::from(finetune) * 64;
        if i < 0 {
            0
        } else {
            period((i as usize).min(PERIODS - 1))
        }
    }

    fn wave(&self, n: u8, i: usize) -> i8 {
        let at = self.song.waveforms.get(usize::from(n)).copied().unwrap_or(self.song.waveforms[0]);
        self.paula.mem[at + (i & 127)]
    }

    fn play_note(&mut self, c: usize) {
        let song = self.song.clone();
        if self.new_pattern {
            let (t, tr) = song.step(self.sub, self.position, c);
            self.voices[c].track = t;
            self.voices[c].transpose = tr;
        }
        if self.new_row {
            let row = song.tracks[usize::from(self.voices[c].track)][self.row.min(63)];
            if row.note != 0 {
                let v = &mut self.voices[c];
                if row.effect != fx::NO_WANDER + 62 {
                    v.note = row.note;
                    if row.instrument != 0 {
                        v.instrument = u16::from(row.instrument - 1);
                    }
                }
                v.instrument &= 0x3F;
                v.effect = if row.effect < 64 { fx::BEND } else { row.effect - 62 };
                v.param = row.param;
                let k = usize::from(v.instrument);
                if k < self.instruments.len() {
                    let ins = self.instruments[k];
                    v.finetune = ins.finetune;
                    if v.effect == fx::NO_WANDER {
                        v.bend_end = Self::get_period(i32::from(row.note), v.transpose, v.finetune);
                    } else if v.effect == fx::BEND {
                        v.bend_end = Self::get_period(i32::from(row.effect), v.transpose, v.finetune);
                    }
                    if v.effect == fx::ARPEGGIO {
                        self.instruments[k].arpeggio = v.param & 7;
                    }
                    let effect = v.effect;
                    if effect != fx::NO_WANDER {
                        if ins.waveform >= 32 {
                            if let Some(s) = song.samples.get(usize::from(ins.waveform - 32)) {
                                self.paula.play(c, s.at, s.len);
                                if let Some(ls) = s.loop_start {
                                    self.paula.set_loop(c, s.at + ls, s.len - ls);
                                }
                            }
                        } else {
                            let at = song.waveforms.get(usize::from(ins.waveform)).copied().unwrap_or(song.waveforms[0]);
                            if effect != fx::NO_DMA {
                                self.paula.play(c, at, ins.loop_length);
                            }
                            self.paula.set_loop(c, at, ins.loop_length);
                            if !song.v2 && ins.effect != 0 && effect != fx::NO_INSTRUMENT_EFFECT && effect != fx::NO_EFFECT_OR_VOLUME {
                                let src = song.waveforms.get(usize::from(ins.source1)).copied().unwrap_or(song.waveforms[0]);
                                let copy: Vec<i8> = self.paula.mem[src..src + 128].to_vec();
                                self.paula.mem[at..at + 128].copy_from_slice(&copy);
                                self.instruments[k].effect_index = 0;
                                self.voices[c].effect_speed = ins.effect_speed;
                            }
                        }
                    }
                    let v = &mut self.voices[c];
                    if effect != fx::NO_INSTRUMENT_VOLUME && effect != fx::NO_EFFECT_OR_VOLUME && effect != fx::NO_WANDER {
                        v.vol_counter = 1;
                        v.vol_index = 0;
                    }
                    v.bend = 0;
                    v.delay = ins.delay;
                    v.pitch_index = 0;
                    v.arp_index = 0;
                }
            }
        }
        let v = &mut self.voices[c];
        match v.effect {
            fx::PATTERN_LENGTH => {
                if v.param != 0 && v.param <= 64 {
                    self.pattern_length = usize::from(v.param);
                }
            }
            fx::SONG_SPEED => {
                let p = u16::from(v.param & 0x0F);
                if p != 0 {
                    self.change_speed(p << 4 | p);
                }
            }
            fx::SHUFFLE => {
                v.effect = 0;
                let p = v.param;
                if p & 0x0F != 0 && p & 0xF0 != 0 {
                    self.change_speed(u16::from(p));
                }
            }
            _ => {}
        }
    }

    fn effects(&mut self, c: usize, done: &mut [u16; 4], count: &mut usize) {
        let song = self.song.clone();
        let k = usize::from(self.voices[c].instrument);
        if k >= self.instruments.len() {
            return;
        }
        let ins = self.instruments[k];
        // The instrument's effect on its waveform: once a tick, however
        // many channels play it.
        if ins.effect != 0 && ins.waveform < 32 {
            let n = self.voices[c].instrument + 1;
            if !done.contains(&n) {
                if *count < 4 {
                    done[*count] = n;
                    *count += 1;
                }
                if self.voices[c].effect_speed == 0 {
                    self.voices[c].effect_speed = ins.effect_speed;
                    let at = song.waveforms.get(usize::from(ins.waveform)).copied().unwrap_or(song.waveforms[0]);
                    self.wave_effect(k, at);
                } else {
                    self.voices[c].effect_speed -= 1;
                }
            }
        }
        let ins = self.instruments[k];
        // The volume, read from a waveform.
        if self.voices[c].vol_counter != 0 {
            self.voices[c].vol_counter -= 1;
            if self.voices[c].vol_counter == 0 {
                self.voices[c].vol_counter = ins.volume_speed;
                self.voices[c].vol_index = (self.voices[c].vol_index + 1) & 0x7F;
                if self.voices[c].vol_index == 0 && !ins.volume_loop {
                    self.voices[c].vol_counter = 0;
                } else {
                    let w = self.wave(ins.volume, self.voices[c].vol_index);
                    let vol = ((-i32::from(w.wrapping_add(0x81u8 as i8))) & 0xFF) / 4;
                    self.paula.set_volume(c, vol as u32);
                }
            }
        }
        let v = &mut self.voices[c];
        let mut note = i32::from(v.note);
        if ins.arpeggio != 0 {
            note += i32::from(song.arpeggios[usize::from(ins.arpeggio & 7)][v.arp_index]);
            v.arp_index = (v.arp_index + 1) & 0x1F;
        }
        let base = Self::get_period(note, v.transpose, v.finetune);
        v.period = base;
        if v.effect == fx::NO_WANDER || v.effect == fx::BEND {
            let inc = -i16::from(v.param as i8);
            v.bend = v.bend.wrapping_add(inc);
            v.period = v.period.wrapping_add(v.bend as u16);
            if v.param != 0 {
                let past = if inc < 0 { v.period <= v.bend_end } else { v.period >= v.bend_end };
                if past {
                    v.bend = v.bend_end.wrapping_sub(base) as i16;
                    v.param = 0;
                }
            }
        }
        if ins.pitch != 0 {
            if v.delay == 0 {
                let i = v.pitch_index;
                v.pitch_index = (v.pitch_index + 1) & 0x7F;
                if v.pitch_index == 0 {
                    v.pitch_index = usize::from(ins.pitch_loop);
                }
                let w = self.wave(ins.pitch, i);
                let v = &mut self.voices[c];
                v.period = v.period.wrapping_sub(w as i16 as u16);
            } else {
                v.delay -= 1;
            }
        }
        let p = self.voices[c].period;
        self.paula.set_period(c, u32::from(p));
    }

    /// An instrument's effect on its 128-point waveform (at `at`).
    fn wave_effect(&mut self, k: usize, at: usize) {
        let song = self.song.clone();
        let ins = &mut self.instruments[k];
        let src = |n: u8| song.waveforms.get(usize::from(n)).copied().unwrap_or(song.waveforms[0]);
        let (s1, s2) = (src(ins.source1), src(ins.source2));
        let len = ins.loop_length.min(128);
        let m = &mut self.paula.mem;
        let w = |m: &Vec<i8>, i: usize| i32::from(m[at + i]);
        let filter = |m: &mut Vec<i8>| {
            for i in 0..127 {
                m[at + i] = ((w(m, i) + w(m, i + 1)) / 2) as i8;
            }
        };
        let upsample = |m: &mut Vec<i8>| {
            let mut out = [0i8; 128];
            for i in 0..64 {
                out[i] = m[at + i * 2];
            }
            let mut d = 64;
            for i in 0..64 {
                // The second half from the first as it now is.
                out[d] = if i < 64 { out[i] } else { 0 };
                d += 1;
            }
            m[at..at + 128].copy_from_slice(&out);
        };
        match ins.effect {
            1 => filter(m),
            2 => {
                let mut idx = ins.effect_index;
                ins.effect_index = (ins.effect_index + 1) & 0x7F;
                for i in 0..len {
                    m[at + i] = ((i32::from(m[s1 + i]) + i32::from(m[s2 + idx])) / 2) as i8;
                    idx = (idx + 1) & 0x7F;
                }
            }
            3 => m[at..at + 128].rotate_left(1),
            4 => m[at..at + 128].rotate_right(1),
            5 => upsample(m),
            6 => {
                // Down-sampling: each point of the first half, doubled.
                let half: Vec<i8> = m[at..at + 64].to_vec();
                for (i, &x) in half.iter().enumerate() {
                    m[at + i * 2] = x;
                    m[at + i * 2 + 1] = x;
                }
            }
            7 => {
                let i = ins.effect_index.min(127);
                m[at + i] = m[at + i].wrapping_neg();
                ins.effect_index += 1;
                if ins.effect_index >= len {
                    ins.effect_index = 0;
                }
            }
            8 => {
                ins.effect_index = (ins.effect_index + 1) & 0x7F;
                let inc = m[s2 + ins.effect_index];
                let mut add: i8 = 3;
                for i in 0..len {
                    m[at + i] = m[at + i].wrapping_add(add);
                    add = add.wrapping_add(inc);
                }
            }
            9 => {
                for i in 0..len {
                    m[at + i] = m[s2 + i].wrapping_add(m[at + i]);
                }
            }
            10 => {
                for i in 0..126 {
                    m[at + i + 1] = ((w(m, i) * 3 + w(m, i + 2)) / 4) as i8;
                }
            }
            11 | 12 => {
                let (mask, top, shift) = if ins.effect == 11 { (0x7F, 64usize, 0x3F) } else { (0x1F, 16, 0x0F) };
                ins.effect_index = (ins.effect_index + 1) & mask;
                let e = ins.effect_index;
                let m1 = if e < top { e } else { mask - e } as i32;
                let m2 = (m1 ^ 0xFF) & shift;
                let div = if ins.effect == 11 { 64 } else { 16 };
                for i in 0..len {
                    m[at + i] = ((i32::from(m[s1 + i]) * m1 + i32::from(m[s2 + i]) * m2) / div) as i8;
                }
            }
            13 => {
                for i in 0..126 {
                    m[at + i + 1] = ((w(m, i) + w(m, i + 2)) / 2) as i8;
                }
            }
            14 => {
                let i = ins.effect_index.min(127);
                m[at + i] = m[at + i].wrapping_neg();
                let j = (i + usize::from(ins.source2)) & len.saturating_sub(1);
                m[at + j] = m[at + j].wrapping_neg();
                ins.effect_index += 1;
                if ins.effect_index >= len {
                    ins.effect_index = 0;
                }
            }
            15 => {
                filter(m);
                ins.effect_index += 1;
                if ins.effect_index == usize::from(ins.source2) {
                    ins.effect_index = 0;
                    upsample(m);
                }
            }
            _ => {}
        }
    }
}

impl Driver for Player {
    fn channels(&self) -> usize {
        self.voices.len()
    }

    fn tick(&mut self) {
        let mut done = [0x80u16; 4];
        let mut count = 0;
        if self.new_row {
            self.shown = (self.position, self.row);
        }
        for c in 0..self.voices.len() {
            self.play_note(c);
        }
        for c in 0..self.voices.len() {
            self.effects(c, &mut done, &mut count);
        }
        self.new_pattern = false;
        self.new_row = false;
        self.speed = self.speed.wrapping_sub(1);
        if self.speed == 0 {
            self.speed = self.current_speed & 0x0F;
            let cs = self.current_speed;
            self.change_speed(((cs & 0x0F) << 4) | ((cs & 0xF0) >> 4));
            self.new_row = true;
            self.row += 1;
            if self.row == 64 || self.row == self.pattern_length {
                self.row = 0;
                self.new_pattern = true;
                self.position += 1;
                if self.position >= self.length {
                    let ss = self.song.subsongs[self.song.playable[self.sub]];
                    self.done = true;
                    if ss.looped {
                        self.position = ss.loop_position.min(self.length.saturating_sub(1));
                    } else {
                        self.start();
                    }
                }
            }
        }
    }

    fn tick_seconds(&self) -> f64 {
        if self.song.v2 {
            // A tick a mixing buffer: as many points, at period 224, as an
            // NTSC E-clock fiftieth gives (rounded up to a word); version 3's
            // 130/210 of that.
            let mut n = 71_591 / MIX_PERIOD + 1;
            if self.song.v3 {
                n = n * 130 / 210;
            }
            let bytes = (n >> 1) * 2;
            f64::from(bytes * MIX_PERIOD) / CLOCK
        } else {
            1.0 / CIA_50
        }
    }

    fn render(&mut self, rate: u32, frames: usize, out: &mut Bufs) {
        self.paula.render(rate, frames, out);

    }

    fn position(&self) -> (usize, usize) {
        self.shown
    }

    fn speed(&self) -> (u32, u32) {
        (u32::from(self.current_speed & 0x0F).max(1), 125)
    }

    fn take_done(&mut self) -> bool {
        std::mem::take(&mut self.done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods_are_mugicians() {
        assert_eq!(&[period(0), period(1), period(4), period(6)], &[4825, 4554, 3820, 3412]);
        assert_eq!(period(30), 854);
        assert_eq!(period(64), 4842);
        assert_eq!(period(11 * 64 + 30), 887);
        assert_eq!(period(1023), {
            // The last fine-tune's top note, on the curve.
            (4825.0 * 2f64.powf(-63.0 / 12.0) * 2f64.powf(15.0 / 192.0)).round() as u16
        });
    }
}
