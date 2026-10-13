// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! InStereo! 1.0 ("ISM!V1.2") and 2.0 ("IS20DF10"), Sonic Arranger's
//! forerunners: sub-songs of positions, each channel a run of the song's
//! rows (transposed); samples and synth sounds, 1.0's a waveform with its
//! start inverted as far as an envelope table says, 2.0's two waveforms
//! stepped through by an envelope generator; ADSR tables, vibrato,
//! portamento between notes, slides and arpeggio tables.

use std::sync::Arc;

use crate::paula::{Paula, CIA_50};
use crate::replay::{Bufs, Driver, Tune};
use crate::sonic::{period, VIBRATO};

#[derive(Clone, Copy, Debug, Default)]
pub struct SubSong {
    pub speed: u8,
    pub rows: u8,
    pub first: u16,
    pub last: u16,
    pub restart: u16,
    pub tempo: u16,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Line {
    pub note: u8,
    pub instrument: u8,
    pub no_sound_transpose: bool,
    pub no_note_transpose: bool,
    pub arpeggio: u8,
    pub effect: u8,
    pub arg: u8,
}

/// 1.0's instrument.
#[derive(Clone, Copy, Debug, Default)]
pub struct Instrument1 {
    pub waveform: u8,
    pub synth: bool,
    pub length: u16,
    pub repeat: u16,
    pub volume: u8,
    pub portamento: i8,
    pub adsr: bool,
    pub adsr_table: u8,
    pub adsr_length: u16,
    pub porta_on: bool,
    pub vib_delay: u8,
    pub vib_speed: u8,
    pub vib_level: u8,
    pub egc_offset: u8,
    /// 0 off, 1 once, 2 round and round.
    pub egc_mode: u8,
    pub egc_table: u8,
    pub egc_length: u16,
    /// Synthesis's (whose instruments are laid out as 1.0's): its
    /// arpeggio's start, length and repeat in the tables, and its
    /// waveform effect with three arguments.
    pub arp_start: u8,
    pub arp_length: u8,
    pub arp_repeat: u8,
    pub effect: u8,
    pub args: [u8; 3],
}

/// 2.0's sample instrument.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample2 {
    pub oneshot: u16,
    pub repeat: u16,
    pub number: i8,
    pub volume: u8,
    pub vib_delay: u8,
    pub vib_speed: u8,
    pub vib_level: u8,
    pub portamento: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Arp {
    pub length: u8,
    pub repeat: u8,
    pub values: [i8; 14],
}

/// 2.0's synth instrument.
#[derive(Clone, Debug)]
pub struct Synth2 {
    pub name: String,
    pub length: u16,
    pub volume: u8,
    pub vib_delay: u8,
    pub vib_speed: u8,
    pub vib_level: u8,
    pub portamento: u8,
    pub adsr_length: u8,
    pub adsr_repeat: u8,
    pub sustain_point: u8,
    pub sustain_speed: u8,
    pub amf_length: u8,
    pub amf_repeat: u8,
    /// 0 off, 1 calculated, 2 free.
    pub eg_mode: u8,
    pub start_len: u8,
    pub stop_rep: u8,
    pub speed_up: u8,
    pub speed_down: u8,
    pub adsr: Vec<u8>,
    pub lfo: Vec<i8>,
    pub arps: [Arp; 3],
    pub eg: Vec<u8>,
    /// Where its two 256-point waveforms are in memory.
    pub waves: [usize; 2],
}

#[derive(Clone, Debug)]
pub enum Kind {
    V1 { instruments: Vec<Instrument1>, waveforms: Vec<usize>, egc: Vec<u8>, adsr: Vec<u8>, arps: Vec<u8>, samples: Vec<(usize, usize)> },
    V2 { samples: Vec<Sample2>, data: Vec<(usize, usize)>, synths: Vec<Synth2> },
}

#[derive(Clone, Debug)]
pub struct Song {
    pub title: String,
    pub subsongs: Vec<SubSong>,
    pub positions: Vec<[(u16, i8, i8); 4]>,
    pub lines: Vec<Line>,
    pub kind: Kind,
    /// Sample names, for the screen.
    pub names: Vec<String>,
    pub mem: Vec<i8>,
    file: Arc<Vec<u8>>,
}

fn be16(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(i..i + 2)?.try_into().ok()?))
}

fn be32(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?))
}

fn text(b: &[u8]) -> String {
    b.iter().take_while(|&&c| c != 0).map(|&c| if (32..127).contains(&c) { c as char } else { ' ' }).collect::<String>().trim().to_string()
}

/// A block of a 2.0 file: its mark and count; `at` moved past them.
fn block(d: &[u8], at: &mut usize, mark: &[u8]) -> Option<usize> {
    if d.get(*at..*at + 4)? != mark {
        return None;
    }
    let n = be32(d, *at + 4)? as usize;
    *at += 8;
    Some(n)
}

pub fn probe(d: &[u8]) -> bool {
    (d.len() >= 204 && d.starts_with(b"ISM!V1.2")) || (d.len() >= 16 && d.starts_with(b"IS20DF10"))
}

/// 1.0's vibrato: a triangle of 256 steps, up by 2 to 126, from 127 down by 2
/// to -127, from -128 up again (its byte wrapping as the replayer's does).
pub(crate) fn vibrato1() -> [i8; 256] {
    let mut t = [0i8; 256];
    let mut v: i8 = 0;
    let mut k = 0;
    for _ in 0..64 {
        t[k] = v;
        k += 1;
        v = v.wrapping_add(2);
    }
    v = v.wrapping_add(1);
    for _ in 0..128 {
        v = v.wrapping_sub(2);
        t[k] = v;
        k += 1;
    }
    v = v.wrapping_sub(1);
    for _ in 0..64 {
        t[k] = v;
        k += 1;
        v = v.wrapping_add(2);
    }
    t
}

impl Song {
    pub fn parse(d: &[u8]) -> Option<Song> {
        if d.starts_with(b"ISM!V1.2") {
            Self::parse1(d)
        } else if d.starts_with(b"IS20DF10") {
            Self::parse2(d)
        } else {
            None
        }
    }

    /// 1.0's layout, which Synthesis shares.
    pub(crate) fn parse1(d: &[u8]) -> Option<Song> {
        let npos = usize::from(be16(d, 8)?);
        let nrows = usize::from(be16(d, 10)?);
        let (nsmp, nwav, nins, nsub, negc, nadsr) = (usize::from(d[16]), usize::from(d[17]), usize::from(d[18]), usize::from(d[19]), usize::from(d[20]), usize::from(d[21]));
        let title = text(d.get(36..64)?);
        let mut at = 204;
        let mut names = Vec::with_capacity(nsmp);
        for _ in 0..nsmp {
            names.push(text(d.get(at + 1..at + 24)?));
            at += 28;
        }
        let lens: Vec<usize> = (0..nsmp).map(|k| be32(d, at + k * 4).map(|v| v as usize)).collect::<Option<_>>()?;
        at += nsmp * 4;
        let egc = d.get(at..at + negc * 128)?.to_vec();
        at += negc * 128;
        let adsr = d.get(at..at + nadsr * 256)?.to_vec();
        at += nadsr * 256;
        let mut instruments = Vec::with_capacity(nins);
        for _ in 0..nins {
            let b = d.get(at..at + 28)?;
            let h = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
            instruments.push(Instrument1 {
                waveform: b[0],
                synth: b[1] != 0,
                length: h(2),
                repeat: h(4),
                volume: b[6],
                portamento: b[7] as i8,
                adsr: b[8] != 0,
                adsr_table: b[9],
                adsr_length: h(10),
                porta_on: b[14] != 0,
                vib_delay: b[20],
                vib_speed: b[21],
                vib_level: b[22],
                egc_offset: b[23],
                egc_mode: b[24],
                egc_table: b[25],
                egc_length: h(26),
                arp_start: b[13],
                arp_length: b[14],
                arp_repeat: b[15],
                effect: b[16],
                args: [b[17], b[18], b[19]],
            });
            at += 28;
        }
        let arps = d.get(at..at + 16 * 16)?.to_vec();
        at += 256;
        let mut subsongs = Vec::with_capacity(nsub);
        for _ in 0..nsub {
            let b = d.get(at..at + 14)?;
            subsongs.push(SubSong { speed: b[4], rows: b[5], first: u16::from_be_bytes([b[6], b[7]]), last: u16::from_be_bytes([b[8], b[9]]), restart: u16::from_be_bytes([b[10], b[11]]), tempo: 50 });
            at += 14;
        }
        at += 14;
        let mut mem: Vec<i8> = Vec::new();
        let mut waveforms = Vec::with_capacity(nwav);
        for _ in 0..nwav {
            waveforms.push(mem.len());
            mem.extend(d.get(at..at + 256)?.iter().map(|&x| x as i8));
            at += 256;
        }
        let positions = (0..npos)
            .map(|k| {
                let o = at + k * 16;
                let mut p = [(0u16, 0i8, 0i8); 4];
                for (c, x) in p.iter_mut().enumerate() {
                    *x = (be16(d, o + c * 4)?, *d.get(o + c * 4 + 2)? as i8, *d.get(o + c * 4 + 3)? as i8);
                }
                Some(p)
            })
            .collect::<Option<Vec<_>>>()?;
        at += npos * 16;
        // The rows, and 64 empty ones after them.
        let mut lines = Vec::with_capacity(nrows + 64);
        for _ in 0..nrows + 64 {
            let x = d.get(at..at + 4).unwrap_or(&[0, 0, 0, 0]);
            lines.push(Line { note: x[0], instrument: x[1], arpeggio: x[2] >> 4, effect: x[2] & 0x0F, arg: x[3], ..Default::default() });
            at += 4;
        }
        let mut samples = Vec::with_capacity(nsmp);
        for &l in &lens {
            let s = mem.len();
            mem.extend(d.get(at..(at + l).min(d.len())).unwrap_or_default().iter().map(|&x| x as i8));
            mem.resize(s + l, 0);
            samples.push((s, l));
            at += l;
        }
        if subsongs.is_empty() || positions.is_empty() {
            return None;
        }
        Some(Song { title, subsongs, positions, lines, kind: Kind::V1 { instruments, waveforms, egc, adsr, arps, samples }, names, mem, file: Arc::new(d.to_vec()) })
    }

    fn parse2(d: &[u8]) -> Option<Song> {
        let mut at = 8;
        let n = block(d, &mut at, b"STBL")?;
        let mut subsongs = Vec::with_capacity(n);
        for _ in 0..n {
            let b = d.get(at..at + 10)?;
            subsongs.push(SubSong { speed: b[0], rows: b[1], first: u16::from_be_bytes([b[2], b[3]]), last: u16::from_be_bytes([b[4], b[5]]), restart: u16::from_be_bytes([b[6], b[7]]), tempo: u16::from_be_bytes([b[8], b[9]]) });
            at += 10;
        }
        let n = block(d, &mut at, b"OVTB")?;
        let positions = (0..n)
            .map(|k| {
                let o = at + k * 16;
                let mut p = [(0u16, 0i8, 0i8); 4];
                for (c, x) in p.iter_mut().enumerate() {
                    *x = (be16(d, o + c * 4)?, *d.get(o + c * 4 + 2)? as i8, *d.get(o + c * 4 + 3)? as i8);
                }
                Some(p)
            })
            .collect::<Option<Vec<_>>>()?;
        at += n * 16;
        let n = block(d, &mut at, b"NTBL")?;
        let lines = (0..n)
            .map(|k| {
                let x = d.get(at + k * 4..at + k * 4 + 4)?;
                Some(Line { note: x[0], instrument: x[1], no_sound_transpose: x[2] & 0x80 != 0, no_note_transpose: x[2] & 0x40 != 0, arpeggio: (x[2] & 0x30) >> 4, effect: x[2] & 0x0F, arg: x[3] })
            })
            .collect::<Option<Vec<_>>>()?;
        at += n * 4;
        let n = block(d, &mut at, b"SAMP")?;
        let mut samples = Vec::with_capacity(n);
        for _ in 0..n {
            let b = d.get(at..at + 16)?;
            samples.push(Sample2 { oneshot: u16::from_be_bytes([b[0], b[1]]), repeat: u16::from_be_bytes([b[2], b[3]]), number: b[4] as i8, volume: b[5], vib_delay: b[6], vib_speed: b[7], vib_level: b[8], portamento: b[9] });
            at += 16;
        }
        let mut names = Vec::with_capacity(n);
        for _ in 0..n {
            names.push(text(d.get(at..at + 20)?));
            at += 20;
        }
        at += n * 8;
        let lens: Vec<usize> = (0..n).map(|k| be32(d, at + k * 4).map(|v| v as usize)).collect::<Option<_>>()?;
        at += n * 4;
        // The points, the last sample's first.
        let mut mem: Vec<i8> = Vec::new();
        let mut data = vec![(0usize, 0usize); n];
        for k in (0..n).rev() {
            let l = lens[k];
            let s = mem.len();
            mem.extend(d.get(at..(at + l).min(d.len())).unwrap_or_default().iter().map(|&x| x as i8));
            mem.resize(s + l, 0);
            data[k] = (s, l);
            at += l;
        }
        let ns = block(d, &mut at, b"SYNT")?;
        let mut synths = Vec::with_capacity(ns);
        for _ in 0..ns {
            if d.get(at..at + 4)? != b"IS20" {
                return None;
            }
            let b = d.get(at + 4..at + 4 + 1006)?;
            let mut arps = [Arp::default(); 3];
            for (j, a) in arps.iter_mut().enumerate() {
                let o = 318 + j * 16;
                a.length = b[o];
                a.repeat = b[o + 1];
                for (k, v) in a.values.iter_mut().enumerate() {
                    *v = b[o + 2 + k] as i8;
                }
            }
            let w1 = mem.len();
            mem.extend(b[494..750].iter().map(|&x| x as i8));
            let w2 = mem.len();
            mem.extend(b[750..1006].iter().map(|&x| x as i8));
            synths.push(Synth2 {
                name: text(&b[..20]),
                length: u16::from_be_bytes([b[20], b[21]]),
                volume: b[22],
                vib_delay: b[23],
                vib_speed: b[24],
                vib_level: b[25],
                portamento: b[26],
                adsr_length: b[27],
                adsr_repeat: b[28],
                sustain_point: b[33],
                sustain_speed: b[34],
                amf_length: b[35],
                amf_repeat: b[36],
                eg_mode: if b[38] == 0 { 0 } else if b[37] == 0 { 1 } else { 2 },
                start_len: b[39],
                stop_rep: b[40],
                speed_up: b[41],
                speed_down: b[42],
                adsr: b[62..190].to_vec(),
                lfo: b[190..318].iter().map(|&x| x as i8).collect(),
                arps,
                eg: b[366..494].to_vec(),
                waves: [w1, w2],
            });
            at += 4 + 1006;
        }
        if subsongs.is_empty() || positions.is_empty() || lines.is_empty() {
            return None;
        }
        Some(Song { title: String::new(), subsongs, positions, lines, kind: Kind::V2 { samples, data, synths }, names, mem, file: Arc::new(d.to_vec()) })
    }

    pub fn line(&self, position: usize, channel: usize, row: usize) -> Line {
        let start = self.positions.get(position).map_or(0, |p| usize::from(p[channel].0));
        self.lines.get(start + row).copied().unwrap_or_default()
    }

    pub fn v2(&self) -> bool {
        matches!(self.kind, Kind::V2 { .. })
    }
}

impl Tune for Song {
    fn format(&self) -> &'static str {
        "is"
    }
    fn file(&self) -> &[u8] {
        &self.file
    }
    fn songs(&self) -> usize {
        self.subsongs.len()
    }
    fn driver(&self, song: usize) -> Box<dyn Driver> {
        Box::new(Player::new(Arc::new(self.clone()), song.min(self.subsongs.len() - 1)))
    }
}

#[derive(Clone, Default)]
struct Voice {
    start: u16,
    sound_tr: i8,
    note_tr: i8,
    line: Line,
    note: u8,
    prev_note: u8,
    /// The instrument playing (1-based; 0 none).
    instrument: u8,
    synth: bool,
    volume: u8,
    slide_speed: i8,
    slide: i16,
    porta_on: bool,
    porta_speed: u8,
    porta_counter: u8,
    vib_delay: u8,
    vib_speed: u8,
    vib_level: u8,
    vib_pos: usize,
    adsr_on: bool,
    adsr_pos: usize,
    sustain: u8,
    egc_mode: u8,
    egc_pos: usize,
    eg_dir: i8,
    eg_pos: u16,
    lfo_pos: usize,
    arp_pos: usize,
    arp_nibble: bool,
    buffer: u8,
}

pub struct Player {
    song: Arc<Song>,
    paula: Paula,
    sub: SubSong,
    voices: [Voice; 4],
    /// Each channel's two waveform buffers.
    buffers: [[usize; 2]; 4],
    vibrato: [i8; 256],
    counter: u8,
    speed: u8,
    position: i32,
    row: u8,
    rows: u8,
    visited: Vec<bool>,
    shown: (usize, usize),
    done: bool,
}

impl Player {
    fn new(song: Arc<Song>, k: usize) -> Player {
        let mut paula = Paula::new(4, song.mem.clone());
        let buffers = std::array::from_fn(|_| [paula.alloc(256), paula.alloc(256)]);
        let sub = song.subsongs[k];
        let v2 = song.v2();
        let vibrato = if v2 { VIBRATO } else { vibrato1() };
        let position = if v2 { i32::from(sub.first) - 1 } else { i32::from(sub.first) };
        Player { paula, buffers, sub, voices: Default::default(), vibrato, counter: sub.speed, speed: sub.speed, position, row: sub.rows, rows: sub.rows, visited: vec![false; song.positions.len()], shown: (0, 0), done: false, song }
    }

    fn visit(&mut self, p: usize) {
        let p = p.min(self.visited.len() - 1);
        if self.visited[p] {
            self.done = true;
            self.visited.iter_mut().for_each(|x| *x = false);
        }
        self.visited[p] = true;
    }

    fn take_position(&mut self, p: usize) {
        let pos = self.song.positions[p.min(self.song.positions.len() - 1)];
        for (v, &(start, st, nt)) in self.voices.iter_mut().zip(&pos) {
            v.start = start;
            v.sound_tr = st;
            v.note_tr = nt;
        }
    }

    fn read_lines(&mut self) {
        let song = self.song.clone();
        for v in self.voices.iter_mut() {
            v.line = song.lines.get(usize::from(v.start) + usize::from(self.row)).copied().unwrap_or_default();
        }
    }
}

// ---- 1.0 ----

impl Player {
    fn row1(&mut self) {
        self.counter = 0;
        if self.row >= self.rows {
            self.row = 0;
            if self.position > i32::from(self.sub.last) {
                self.position = i32::from(self.sub.restart);
            }
            let p = self.position.max(0) as usize;
            self.visit(p);
            self.position += 1;
            self.take_position(p);
        }
        self.shown = (((self.position - 1).max(0) as usize).saturating_sub(usize::from(self.sub.first)), usize::from(self.row));
        self.read_lines();
        for c in 0..4 {
            self.play_row1(c);
        }
        self.row += 1;
    }

    fn play_row1(&mut self, c: usize) {
        let song = self.song.clone();
        let Kind::V1 { instruments, waveforms, samples, .. } = &song.kind else { return };
        let v = &mut self.voices[c];
        v.volume = 0;
        v.slide_speed = 0;
        let (e, a) = (v.line.effect, v.line.arg);
        match e {
            0x1 => v.slide_speed = a as i8,
            0x2 => {
                v.adsr_pos = usize::from(a);
                v.adsr_on = true;
            }
            0x3 => {
                v.egc_pos = usize::from(a);
                v.egc_mode = 1;
            }
            0x4 => v.slide = i16::from(a as i8),
            0x5 => {
                v.vib_delay = a;
                v.vib_pos = 0;
            }
            0x6 => v.vib_pos = usize::from(a),
            0x7 => v.volume = a & 0x3F,
            0xA => {
                if a <= 64 {
                    self.rows = a;
                }
            }
            0xF => {
                if (1..=16).contains(&a) {
                    self.speed = a;
                }
            }
            _ => {}
        }
        let v = &mut self.voices[c];
        let mut note = v.line.note;
        if note == 0 {
            return;
        }
        if note == 0x7F {
            self.paula.stop(c);
            self.voices[c].volume = 0;
            return;
        }
        if e != 0x8 {
            note = note.wrapping_add(v.note_tr as u8);
        }
        v.prev_note = v.note;
        v.note = note;
        let p = period(usize::from(note).min(108));
        self.paula.set_period(c, u32::from(p));
        let mut n = v.line.instrument;
        if n == 0 {
            return;
        }
        if e != 0x9 {
            n = n.wrapping_add(v.sound_tr as u8);
        }
        v.instrument = n;
        let Some(ins) = instruments.get(usize::from(n).wrapping_sub(1)).copied() else {
            self.paula.stop(c);
            return;
        };
        v.adsr_on = false;
        v.adsr_pos = 0;
        v.vib_pos = 0;
        v.egc_mode = 0;
        v.egc_pos = 0;
        v.slide = 0;
        v.porta_on = ins.porta_on;
        v.porta_speed = ins.portamento as u8;
        v.porta_counter = ins.portamento as u8;
        if e == 0xB {
            v.porta_on = false;
            v.porta_speed = 0;
            v.porta_counter = 0;
        }
        v.vib_delay = ins.vib_delay;
        if ins.adsr {
            v.adsr_on = true;
        }
        if ins.synth {
            v.buffer = 1;
            v.egc_mode = ins.egc_mode;
            if ins.egc_mode == 0 {
                let len = usize::from(ins.length).min(256);
                let buf = self.buffers[c][0];
                let src = waveforms.get(usize::from(ins.waveform)).copied().unwrap_or(0);
                self.paula.play(c, buf, len);
                self.paula.set_loop(c, buf, len);
                for i in 0..len {
                    let x = self.paula.mem.get(src + i).copied().unwrap_or(0);
                    self.paula.mem[buf + i] = if i < usize::from(ins.egc_offset) { x.wrapping_neg() } else { x };
                }
            }
            self.voices[c].volume = ins.volume;
            self.paula.set_volume(c, u32::from(ins.volume));
        } else {
            let k = usize::from(ins.waveform & 0x3F);
            let Some(&(at, slen)) = samples.get(k) else {
                self.paula.stop(c);
                return;
            };
            let mut play = usize::from(ins.length);
            let (mut ls, mut ll) = (0usize, 0usize);
            if ins.repeat == 0 {
                ll = usize::from(ins.length);
            } else if ins.repeat != 2 {
                play += usize::from(ins.repeat);
                ls = usize::from(ins.length);
                ll = usize::from(ins.repeat);
            }
            self.paula.play(c, at, play.min(slen));
            if ll != 0 {
                self.paula.set_loop(c, at + ls, ll.min(slen.saturating_sub(ls)));
            }
            self.paula.set_volume(c, u32::from(ins.volume));
            self.voices[c].volume = if k == 7 { a & 0x3F } else { ins.volume };
        }
    }

    fn effects1(&mut self, c: usize) {
        let song = self.song.clone();
        let Kind::V1 { instruments, waveforms, egc, adsr, arps, .. } = &song.kind else { return };
        let v = &mut self.voices[c];
        let a = arps.get(usize::from(v.line.arpeggio) * 16 + usize::from(self.counter)).copied().unwrap_or(0);
        let (note, prev) = (v.note.wrapping_add(a), v.prev_note.wrapping_add(a));
        let (mut p, mut pp) = (period(usize::from(note).min(108)), period(usize::from(prev).min(108)));
        // Portamento: from the last note to this one, over its speed in ticks.
        if v.porta_on && v.porta_counter != 0 && p != pp {
            v.porta_counter -= 1;
            std::mem::swap(&mut p, &mut pp);
            let mut np = (i32::from(p) - i32::from(pp)) * i32::from(v.porta_counter);
            if v.porta_speed != 0 {
                np /= i32::from(v.porta_speed);
            }
            p = (np + i32::from(pp)) as u16;
        }
        let ins = instruments.get(usize::from(v.instrument).wrapping_sub(1)).copied();
        let mut vib_ok = true;
        if v.vib_delay == 0 {
            match ins {
                None => vib_ok = false,
                Some(ins) => {
                    let x = self.vibrato[v.vib_pos & 0xFF];
                    if x < 0 {
                        if ins.vib_level != 0 {
                            p = p.wrapping_sub(((-i32::from(x) * 4) / i32::from(ins.vib_level)) as u16);
                        }
                    } else if ins.vib_level != 0 {
                        p = p.wrapping_add(((i32::from(x) * 4) / i32::from(ins.vib_level)) as u16);
                    } else {
                        p = 0;
                    }
                    v.vib_pos = (v.vib_pos + usize::from(ins.vib_speed)) & 0xFF;
                }
            }
        } else {
            v.vib_delay -= 1;
        }
        if vib_ok {
            p = p.wrapping_add(v.slide as u16);
            self.paula.set_period(c, u32::from(p));
            let v = &mut self.voices[c];
            v.slide = v.slide.wrapping_sub(i16::from(v.slide_speed));
            if let Some(ins) = ins {
                if v.adsr_on {
                    if v.adsr_pos >= usize::from(ins.adsr_length) {
                        v.adsr_on = false;
                    }
                    let x = u32::from(adsr.get(usize::from(ins.adsr_table) * 256 + v.adsr_pos).copied().unwrap_or(0)) + 1;
                    let vol = (u32::from(ins.volume) * x / 128).min(64);
                    self.paula.set_volume(c, vol);
                    self.voices[c].adsr_pos += 1;
                }
            }
        }
        // The envelope generator: the waveform copied out with its start inverted.
        let v = &mut self.voices[c];
        if v.egc_mode != 0 {
            let Some(ins) = ins else { return };
            v.buffer ^= 1;
            let buf = self.buffers[c][usize::from(v.buffer == 0)];
            let len = usize::from(ins.length).min(256);
            // A channel not playing yet starts on it.
            if !self.paula.playing(c) {
                self.paula.play(c, buf, len);
            }
            self.paula.set_loop(c, buf, len);
            let src = waveforms.get(usize::from(ins.waveform)).copied().unwrap_or(0);
            let x = egc.get(usize::from(ins.egc_table) * 128 + v.egc_pos).copied().unwrap_or(0).wrapping_add(ins.egc_offset);
            for i in 0..len {
                let w = self.paula.mem.get(src + i).copied().unwrap_or(0);
                self.paula.mem[buf + i] = if i < usize::from(x) { w.wrapping_neg() } else { w };
            }
            let v = &mut self.voices[c];
            v.egc_pos += 1;
            if v.egc_pos >= usize::from(ins.egc_length) {
                if v.egc_mode == 1 {
                    v.egc_mode = 0;
                } else {
                    v.egc_pos = 0;
                }
            }
        }
    }
}

// ---- 2.0 ----

impl Player {
    fn row2(&mut self) {
        self.row += 1;
        if self.row >= self.rows {
            self.row = 0;
            self.position += 1;
            if self.position > i32::from(self.sub.last) {
                self.position = i32::from(self.sub.restart);
            }
            let p = self.position.max(0) as usize;
            self.visit(p);
            self.take_position(p);
        }
        self.shown = ((self.position.max(0) as usize).saturating_sub(usize::from(self.sub.first)), usize::from(self.row));
        self.read_lines();
        for v in self.voices.iter_mut() {
            if v.line.note != 0 && v.line.instrument != 0 {
                v.instrument = 0;
            }
        }
        for c in 0..4 {
            self.play_voice2(c);
        }
    }

    fn play_voice2(&mut self, c: usize) {
        let song = self.song.clone();
        let Kind::V2 { samples, data, synths } = &song.kind else { return };
        // The row's effect first.
        let v = &mut self.voices[c];
        let (e, a) = (v.line.effect, v.line.arg);
        match e {
            0x1 => v.slide_speed = a as i8,
            0x2 => v.adsr_pos = usize::from(a),
            0x4 => {
                v.vib_delay = 0;
                v.vib_speed = ((a >> 4) & 0x0F) * 2;
                v.vib_level = (160 - (i32::from(a & 0x0F) << 4)) as u8;
            }
            0x7 => {
                v.porta_counter = a;
                v.porta_speed = a;
            }
            0xA => {
                let lim = if v.synth { 255 } else { 64 };
                v.volume = (i32::from(v.volume) + i32::from(a as i8)).clamp(0, lim) as u8;
            }
            0xB => {
                self.position = i32::from(a);
                self.row = self.rows;
            }
            0xC => v.volume = if !v.synth { a.min(64) } else { a },
            0xD => self.row = self.rows,
            0x9 => {
                if a <= 64 {
                    self.rows = a;
                }
            }
            0xF => {
                if (1..=16).contains(&a) {
                    self.speed = a;
                }
            }
            _ => {}
        }
        let v = &mut self.voices[c];
        let line = v.line;
        let mut note = line.note;
        let mut n = line.instrument;
        // What an instrument's settings give the channel (unless the row's effect says otherwise).
        let apply = |v: &mut Voice, vib: (u8, u8, u8), porta: u8, vol: u8| {
            if e != 0x4 {
                (v.vib_delay, v.vib_speed, v.vib_level) = vib;
            }
            if e != 0x8 && e != 0x7 {
                v.porta_counter = porta;
                v.porta_speed = porta;
            }
            if e != 0xC && e != 0xA {
                v.volume = vol;
            }
        };
        if note == 0 {
            if n != 0 {
                let k = n & 0x3F;
                v.instrument = k;
                let i = usize::from(k).wrapping_sub(1);
                if !v.synth {
                    if let Some(s) = samples.get(i) {
                        apply(v, (s.vib_delay, s.vib_speed, s.vib_level), s.portamento, s.volume);
                    }
                } else if let Some(s) = synths.get(i) {
                    apply(v, (s.vib_delay, s.vib_speed, s.vib_level), s.portamento, s.volume);
                }
            }
            return;
        }
        if note == 0x80 {
            return;
        }
        if note == 0x7F {
            self.paula.stop(c);
            let v = &mut self.voices[c];
            v.volume = 0;
            v.instrument = 0;
            v.synth = false;
            return;
        }
        if !line.no_note_transpose || line.no_sound_transpose {
            note = note.wrapping_add(v.note_tr as u8);
        }
        if !line.no_sound_transpose || line.no_note_transpose {
            n = n.wrapping_add(v.sound_tr as u8);
        }
        v.prev_note = v.note;
        v.note = note;
        if n > 127 {
            return;
        }
        // A new note resets the channel.
        v.instrument = 0;
        v.arp_pos = 0;
        v.slide_speed = 0;
        v.slide = 0;
        v.vib_pos = 0;
        v.adsr_pos = 0;
        v.sustain = 0;
        v.eg_dir = 0;
        v.eg_pos = 0;
        v.lfo_pos = 0;
        if e != 0x7 {
            v.porta_counter = 0;
            v.porta_speed = 0;
        }
        if e != 0x4 {
            v.vib_delay = 0;
            v.vib_speed = 0;
            v.vib_level = 0;
        }
        let k = n & 0x3F;
        if n >= 64 {
            v.synth = false;
            v.instrument = k;
            let Some(s) = samples.get(usize::from(k).wrapping_sub(1)).copied() else {
                return self.quiet2(c);
            };
            if e != 0x4 {
                (v.vib_delay, v.vib_speed, v.vib_level) = (s.vib_delay, s.vib_speed, s.vib_level);
            }
            if e != 0x8 && e != 0x7 {
                v.porta_counter = s.portamento;
                v.porta_speed = s.portamento;
            }
            let Some(&(at, slen)) = usize::try_from(s.number).ok().and_then(|i| data.get(i)).filter(|d| d.1 > 0) else {
                return self.quiet2(c);
            };
            let mut play = u32::from(s.oneshot);
            let (mut ls, mut ll) = (0u32, 0u32);
            if s.repeat == 0 {
                ll = u32::from(s.oneshot);
            } else if s.repeat != 1 {
                play += u32::from(s.repeat);
                ls = u32::from(s.oneshot);
                ll = u32::from(s.repeat);
            }
            let play = ((play * 2) as usize).min(slen);
            self.paula.play(c, at, play);
            if ll != 0 {
                let ls = (ls * 2) as usize;
                self.paula.set_loop(c, at + ls, ((ll * 2) as usize).min(slen.saturating_sub(ls)));
            }
            if e != 0xC && e != 0xA {
                self.voices[c].volume = s.volume;
                self.paula.set_volume(c, u32::from(s.volume));
            }
        } else {
            v.synth = true;
            v.instrument = k;
            if k == 0 {
                return self.quiet2(c);
            }
            let Some(s) = synths.get(usize::from(k) - 1) else { return self.quiet2(c) };
            if e != 0x4 {
                (v.vib_delay, v.vib_speed, v.vib_level) = (s.vib_delay, s.vib_speed, s.vib_level);
            }
            if e != 0x8 && e != 0x7 {
                v.porta_counter = s.portamento;
                v.porta_speed = s.portamento;
            }
            let eg = match s.eg_mode {
                2 if s.start_len.wrapping_add(s.stop_rep) != 0 => {
                    v.eg_pos = 0;
                    s.eg[0]
                }
                1 => {
                    v.eg_pos = u16::from(s.start_len) << 8;
                    v.eg_dir = 1;
                    s.start_len
                }
                _ => {
                    v.eg_dir = 0;
                    0
                }
            };
            let wave = s.waves[usize::from(eg & 1)];
            let start = usize::from(eg & 0xFE);
            let len = usize::from(s.length).min(256 - start.min(256));
            self.paula.play(c, wave + start, len);
            self.paula.set_loop(c, wave + start, len);
            if e != 0xC && e != 0xA {
                self.voices[c].volume = s.volume;
            }
        }
    }

    fn quiet2(&mut self, c: usize) {
        self.paula.stop(c);
        let v = &mut self.voices[c];
        v.volume = 0;
        v.instrument = 0;
        v.synth = false;
    }

    fn effects2(&mut self, c: usize) {
        let song = self.song.clone();
        let Kind::V2 { synths, .. } = &song.kind else { return };
        let v = &mut self.voices[c];
        if v.instrument == 0 {
            self.paula.stop(c);
            return;
        }
        let synth = if v.synth { synths.get(usize::from(v.instrument) - 1) } else { None };
        // The arpeggio: the effect's nibbles in turn, or a synth's tables.
        let (mut note, mut prev) = (v.note, v.prev_note);
        match synth {
            None => {
                if v.line.effect == 0 {
                    let a = if v.arp_nibble { v.line.arg & 0x0F } else { v.line.arg >> 4 };
                    v.arp_nibble = !v.arp_nibble;
                    note = note.wrapping_add(a);
                    prev = prev.wrapping_add(a);
                }
            }
            Some(s) => {
                if v.line.arpeggio != 0 {
                    let arp = s.arps[usize::from(v.line.arpeggio - 1)];
                    let a = arp.values[v.arp_pos.min(13)] as u8;
                    note = note.wrapping_add(a);
                    prev = prev.wrapping_add(a);
                    if v.arp_pos == usize::from(arp.length + arp.repeat) {
                        v.arp_pos = usize::from(arp.length);
                    } else {
                        v.arp_pos += 1;
                    }
                }
            }
        }
        let (mut p, mut pp) = (period(usize::from(note)), period(usize::from(prev)));
        if v.porta_speed != 0 && v.porta_counter != 0 && p != pp {
            v.porta_counter -= 1;
            std::mem::swap(&mut p, &mut pp);
            let np = (i32::from(p) - i32::from(pp)) * i32::from(v.porta_counter) / i32::from(v.porta_speed) + i32::from(pp);
            p = np as u16;
        }
        if v.vib_delay != 255 {
            if v.vib_delay == 0 {
                let x = self.vibrato[v.vib_pos & 0xFF];
                if v.vib_level != 0 {
                    if x < 0 {
                        p = p.wrapping_sub(((-i32::from(x) * 4) / i32::from(v.vib_level)) as u16);
                    } else {
                        p = p.wrapping_add(((i32::from(x) * 4) / i32::from(v.vib_level)) as u16);
                    }
                }
                v.vib_pos = (v.vib_pos + usize::from(v.vib_speed)) & 0xFF;
            } else {
                v.vib_delay -= 1;
            }
        }
        p = p.wrapping_add(v.slide as u16);
        match synth {
            None => {
                self.paula.set_period(c, u32::from(p));
                self.paula.set_volume(c, u32::from(v.volume));
            }
            Some(s) => {
                // The LFO.
                if s.amf_length.wrapping_add(s.amf_repeat) != 0 {
                    p = p.wrapping_sub(s.lfo.get(v.lfo_pos).copied().unwrap_or(0) as i16 as u16);
                    if v.lfo_pos == usize::from(s.amf_length) + usize::from(s.amf_repeat) {
                        v.lfo_pos = usize::from(s.amf_length);
                    } else {
                        v.lfo_pos += 1;
                    }
                }
                self.paula.set_period(c, u32::from(p));
                // The ADSR, held at its sustain point while the note's held.
                if u16::from(s.adsr_length) + u16::from(s.adsr_repeat) == 0 {
                    self.paula.set_volume_fine(c, u32::from(v.volume));
                } else {
                    let x = u32::from(s.adsr.get(v.adsr_pos).copied().unwrap_or(0));
                    self.paula.set_volume_fine(c, u32::from(v.volume) * x / 256);
                    if v.adsr_pos >= usize::from(s.adsr_length) + usize::from(s.adsr_repeat) {
                        v.adsr_pos = usize::from(s.adsr_length);
                    } else if v.line.note != 0x80 || s.sustain_speed == 1 || v.adsr_pos < usize::from(s.sustain_point) {
                        v.adsr_pos += 1;
                    } else if s.sustain_speed != 0 {
                        if v.sustain == 0 {
                            v.sustain = s.sustain_speed;
                            v.adsr_pos += 1;
                        } else {
                            v.sustain -= 1;
                        }
                    }
                }
                // The envelope generator: where in the two waveforms to play.
                if s.eg_mode != 0 {
                    let eg: u8;
                    if s.eg_mode == 2 {
                        let len = u16::from(s.start_len) + u16::from(s.stop_rep);
                        if len == 0 {
                            v.slide = v.slide.wrapping_sub(i16::from(v.slide_speed));
                            return;
                        }
                        if v.eg_pos >= len {
                            v.eg_pos = u16::from(s.start_len);
                        } else {
                            v.eg_pos += 1;
                        }
                        eg = s.eg.get(usize::from(v.eg_pos)).copied().unwrap_or(0);
                    } else {
                        if v.eg_dir == 0 {
                            v.slide = v.slide.wrapping_sub(i16::from(v.slide_speed));
                            return;
                        }
                        if v.eg_dir > 0 {
                            let np = v.eg_pos.wrapping_add(u16::from(s.speed_up) * 32);
                            if (np >> 8) >= u16::from(s.stop_rep) {
                                v.eg_pos = u16::from(s.stop_rep) << 8;
                                v.eg_dir = -1;
                            } else {
                                v.eg_pos = np;
                            }
                        } else {
                            let np = v.eg_pos.wrapping_sub(u16::from(s.speed_down) * 32);
                            if (np >> 8) >= u16::from(s.start_len) {
                                v.eg_pos = u16::from(s.start_len) << 8;
                                v.eg_dir = 1;
                            } else {
                                v.eg_pos = np;
                            }
                        }
                        eg = (v.eg_pos >> 8) as u8;
                    }
                    let wave = s.waves[usize::from(eg & 1)];
                    let start = usize::from(eg & 0xFE);
                    let len = usize::from(s.length).min(256 - start.min(256));
                    self.paula.set_loop(c, wave + start, len);
                }
            }
        }
        let v = &mut self.voices[c];
        v.slide = v.slide.wrapping_sub(i16::from(v.slide_speed));
    }
}

impl Driver for Player {
    fn channels(&self) -> usize {
        4
    }

    fn tick(&mut self) {
        let v2 = self.song.v2();
        self.counter += 1;
        if self.counter >= self.speed {
            if v2 {
                self.counter = 0;
                self.row2();
            } else {
                self.row1();
            }
        }
        for c in 0..4 {
            if v2 {
                self.effects2(c);
            } else {
                self.effects1(c);
            }
        }
    }

    fn tick_seconds(&self) -> f64 {
        if self.song.v2() {
            let t = u32::from(if self.sub.tempo == 0 { 50 } else { self.sub.tempo });
            // As UADE's player times it: 722,500 / tempo of the CIA's clocks
            // a tick (measured; it's 1.85% slower than the tempo in Hz).
            f64::from(722_500 / t) / 709_379.0
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
        (u32::from(self.speed.max(1)), 125)
    }

    fn take_done(&mut self) -> bool {
        std::mem::take(&mut self.done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_10_vibrato_is_a_triangle() {
        let t = vibrato1();
        assert_eq!(&t[..4], &[0, 2, 4, 6]);
        assert_eq!(t[63], 126);
        assert_eq!(t[64], 127);
        assert_eq!(t[65], 125);
        assert_eq!(t[191], -127);
        assert_eq!(t[192], -128);
        assert_eq!(t[255], -2);
    }
}
