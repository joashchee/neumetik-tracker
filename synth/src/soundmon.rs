// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! Brian Postma's SoundMon 2.0 ("V.2") and 2.2 ("V.3"): each channel steps
//! through its tracks (16 rows, each transposed in note and instrument);
//! instruments are samples or synth sounds, a 32 to 64-point waveform
//! played from the song's wave tables under an ADSR, an LFO, and three
//! ways of remaking the waveform as it plays (an EG that inverts part of
//! it, an effect that averages or morphs it toward another, a modulation
//! that writes a point). Every table is a wave table (64 points each).

use std::sync::Arc;

use crate::paula::{Paula, CIA_50};
use crate::replay::{Bufs, Driver, Tune};

/// SoundMon's periods: its own octave (ProTracker's rounded to fours)
/// eight, four and two times, itself, half, a quarter and an eighth
/// (rounded up); index 0 is the lowest C.
pub fn period(i: usize) -> u16 {
    const O: [u16; 12] = [856, 808, 760, 720, 680, 640, 604, 572, 540, 508, 480, 452];
    let (oct, k) = (i / 12, i % 12);
    match oct {
        0 => O[k] * 8,
        1 => O[k] * 4,
        2 => O[k] * 2,
        3 => O[k],
        4 => O[k] / 2,
        5 => O[k] / 4,
        _ => O[k].div_ceil(8),
    }
}

const PERIODS: usize = 84;

/// The vibrato's 8 steps.
const VIBRATO: [i16; 8] = [0, 64, 128, 64, 0, -64, -128, -64];

#[derive(Clone, Copy, Debug, Default)]
pub struct Synth {
    pub wave_table: u8,
    pub wave_length: u16,
    pub adsr_control: u8,
    pub adsr_table: u8,
    pub adsr_length: u16,
    pub adsr_speed: u8,
    pub lfo_control: u8,
    pub lfo_table: u8,
    pub lfo_depth: u8,
    pub lfo_length: u16,
    pub lfo_delay: u8,
    pub lfo_speed: u8,
    pub eg_control: u8,
    pub eg_table: u8,
    pub eg_length: u16,
    pub eg_delay: u8,
    pub eg_speed: u8,
    pub fx_control: u8,
    pub fx_speed: u8,
    pub fx_delay: u8,
    pub mod_control: u8,
    pub mod_table: u8,
    pub mod_speed: u8,
    pub mod_delay: u8,
    pub volume: u16,
    pub mod_length: u16,
}

#[derive(Clone, Debug)]
pub enum Instrument {
    Synth(Synth),
    Sample { name: String, at: usize, len: usize, loop_start: usize, loop_len: usize, volume: u16 },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Step {
    pub track: u16,
    pub sound_transpose: i8,
    pub transpose: i8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Row {
    pub note: i8,
    pub instrument: u8,
    pub optional: u8,
    pub data: u8,
}

#[derive(Clone, Debug)]
pub struct Song {
    /// 2.2 ("V.3"), or 2.0.
    pub v22: bool,
    pub title: String,
    pub instruments: Vec<Instrument>,
    pub steps: [Vec<Step>; 4],
    pub tracks: Vec<[Row; 16]>,
    /// The wave tables, then the samples: Paula's memory.
    pub mem: Vec<i8>,
    pub waves: usize,
    file: Arc<Vec<u8>>,
}

pub fn probe(d: &[u8]) -> bool {
    d.len() >= 512 && matches!(&d[26..29], b"V.2" | b"V.3")
}

fn be16(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(i..i + 2)?.try_into().ok()?))
}

impl Song {
    pub fn parse(d: &[u8]) -> Option<Song> {
        if !probe(d) {
            return None;
        }
        let v22 = &d[26..29] == b"V.3";
        let text = |b: &[u8]| b.iter().take_while(|&&c| c != 0).map(|&c| if (32..127).contains(&c) { c as char } else { ' ' }).collect::<String>().trim().to_string();
        let title = text(&d[..26]);
        let waves = usize::from(d[29]);
        let nsteps = usize::from(be16(d, 30)?);
        let mut at = 32;
        let mut raw = Vec::with_capacity(15);
        for _ in 0..15 {
            let b = d.get(at..at + 32)?;
            raw.push(b.to_vec());
            at += 32;
        }
        let mut steps: [Vec<Step>; 4] = Default::default();
        let mut ntracks = 0;
        for _ in 0..nsteps {
            for s in steps.iter_mut() {
                let st = Step { track: be16(d, at)?, sound_transpose: *d.get(at + 2)? as i8, transpose: *d.get(at + 3)? as i8 };
                ntracks = ntracks.max(usize::from(st.track));
                s.push(st);
                at += 4;
            }
        }
        let mut tracks = Vec::with_capacity(ntracks);
        for _ in 0..ntracks {
            let mut t = [Row::default(); 16];
            for r in t.iter_mut() {
                let b = d.get(at..at + 3)?;
                *r = Row { note: b[0] as i8, instrument: b[1] >> 4, optional: b[1] & 0x0F, data: b[2] };
                at += 3;
            }
            tracks.push(t);
        }
        let mut mem: Vec<i8> = d.get(at..at + waves * 64)?.iter().map(|&x| x as i8).collect();
        at += waves * 64;
        let mut instruments = Vec::with_capacity(15);
        for b in &raw {
            if b[0] == 0xFF {
                let w = |i: usize| b[i];
                let h = |i: usize| u16::from(b[i]) << 8 | u16::from(b[i + 1]);
                let s = if !v22 {
                    Synth {
                        wave_table: w(1),
                        wave_length: h(2) * 2,
                        adsr_control: w(4),
                        adsr_table: w(5),
                        adsr_length: h(6),
                        adsr_speed: w(8),
                        lfo_control: w(9),
                        lfo_table: w(10),
                        lfo_depth: w(11),
                        lfo_length: h(12),
                        lfo_delay: w(15),
                        lfo_speed: w(16),
                        eg_control: w(17),
                        eg_table: w(18),
                        eg_length: h(20),
                        eg_delay: w(23),
                        eg_speed: w(24),
                        fx_speed: 1,
                        mod_speed: 1,
                        volume: u16::from(w(25)).min(64),
                        ..Default::default()
                    }
                } else {
                    Synth {
                        wave_table: w(1),
                        wave_length: h(2) * 2,
                        adsr_control: w(4),
                        adsr_table: w(5),
                        adsr_length: h(6),
                        adsr_speed: w(8),
                        lfo_control: w(9),
                        lfo_table: w(10),
                        lfo_depth: w(11),
                        lfo_length: h(12),
                        lfo_delay: w(14),
                        lfo_speed: w(15),
                        eg_control: w(16),
                        eg_table: w(17),
                        eg_length: h(18),
                        eg_delay: w(20),
                        eg_speed: w(21),
                        fx_control: w(22),
                        fx_speed: w(23),
                        fx_delay: w(24),
                        mod_control: w(25),
                        mod_table: w(26),
                        mod_speed: w(27),
                        mod_delay: w(28),
                        volume: u16::from(w(29)).min(64),
                        mod_length: h(30),
                    }
                };
                instruments.push(Instrument::Synth(s));
            } else {
                let len = usize::from(be16(b, 24)?) * 2;
                let ls = usize::from(be16(b, 26)?);
                let mut ll = usize::from(be16(b, 28)?) * 2;
                if ls + ll > len {
                    ll = len.saturating_sub(ls);
                }
                instruments.push(Instrument::Sample { name: text(&b[..24]), at: 0, len, loop_start: ls, loop_len: ll, volume: be16(b, 30)?.min(64) });
            }
        }
        for ins in instruments.iter_mut() {
            if let Instrument::Sample { at: a, len, .. } = ins {
                if *len != 0 {
                    *a = mem.len();
                    mem.extend(d.get(at..(at + *len).min(d.len())).unwrap_or_default().iter().map(|&x| x as i8));
                    mem.resize(*a + *len, 0);
                    at += *len;
                }
            }
        }
        if steps[0].is_empty() || tracks.is_empty() {
            return None;
        }
        Some(Song { v22, title, instruments, steps, tracks, mem, waves, file: Arc::new(d.to_vec()) })
    }

    pub fn row(&self, step: usize, channel: usize, row: usize) -> Row {
        let t = usize::from(self.steps[channel].get(step).map_or(0, |s| s.track));
        self.tracks.get(t.wrapping_sub(1)).map_or(Row::default(), |t| t[row.min(15)])
    }
}

impl Tune for Song {
    fn format(&self) -> &'static str {
        "bp"
    }
    fn file(&self) -> &[u8] {
        &self.file
    }
    fn driver(&self, _song: usize) -> Box<dyn Driver> {
        Box::new(Player::new(Arc::new(self.clone())))
    }
}

#[derive(Clone, Default)]
struct Voice {
    restart: bool,
    default_volume: bool,
    synth_mode: bool,
    synth_offset: Option<usize>,
    period: u16,
    volume: u8,
    instrument: u8,
    note: u8,
    arp_value: u8,
    auto_slide: i8,
    auto_arp: u8,
    vibrato: i8,
    eg_ptr: u16,
    lfo_ptr: u16,
    adsr_ptr: u16,
    mod_ptr: u16,
    eg_count: u8,
    lfo_count: u8,
    adsr_count: u8,
    mod_count: u8,
    fx_count: u8,
    fx_control: u8,
    eg_control: u8,
    lfo_control: u8,
    adsr_control: u8,
    mod_control: u8,
    old_eg: u8,
    buffer: [i8; 32],
}

pub struct Player {
    song: Arc<Song>,
    paula: Paula,
    voices: [Voice; 4],
    arp_count: u8,
    count: u8,
    delay: u8,
    rep_count: u8,
    first_repeat: bool,
    vib_index: usize,
    step: usize,
    row: usize,
    new_pos: usize,
    pos_flag: bool,
    visited: Vec<bool>,
    shown: (usize, usize),
    done: bool,
}

impl Player {
    fn new(song: Arc<Song>) -> Player {
        let mut visited = vec![false; song.steps[0].len()];
        visited[0] = true;
        Player { paula: Paula::new(4, song.mem.clone()), voices: Default::default(), arp_count: 1, count: 1, delay: 6, rep_count: 0, first_repeat: false, vib_index: 0, step: 0, row: 0, new_pos: 0, pos_flag: false, visited, shown: (0, 0), done: false, song }
    }

    fn synth(&self, c: usize) -> Option<Synth> {
        match self.song.instruments.get(usize::from(self.voices[c].instrument).wrapping_sub(1)) {
            Some(Instrument::Synth(s)) => Some(*s),
            _ => None,
        }
    }

    fn next(&mut self) {
        let song = self.song.clone();
        self.shown = (self.step, self.row);
        for c in 0..4 {
            let st = song.steps[c][self.step];
            let (sound_tr, tr) = (st.sound_transpose, st.transpose);
            let row = song.row(self.step, c, self.row);
            let v = &mut self.voices[c];
            let mut note = row.note;
            if note != 0 {
                v.auto_slide = 0;
                v.auto_arp = 0;
                v.vibrato = 0;
                if row.optional != 0xA || row.data & 0xF0 == 0 {
                    note = note.wrapping_add(tr);
                }
                v.note = note as u8;
                v.period = period((i32::from(note) + 36 - 1).clamp(0, PERIODS as i32 - 1) as usize);
                v.restart = false;
                if row.optional < 0xD {
                    v.restart = true;
                    v.default_volume = true;
                }
                let mut inst = row.instrument;
                if inst == 0 {
                    inst = v.instrument;
                }
                if inst != 0 && (row.optional != 0xA || row.data & 0x0F == 0) {
                    inst = inst.wrapping_add(sound_tr as u8);
                    if !(1..=15).contains(&inst) {
                        inst = inst.wrapping_sub(sound_tr as u8);
                    }
                }
                if row.optional < 0xD && (!v.synth_mode || v.instrument != inst) {
                    v.instrument = inst;
                }
            }
            self.optional(c, row.optional, row.data);
        }
        if self.pos_flag {
            self.row = 0;
            self.step = self.new_pos;
        } else {
            self.row += 1;
            if self.row == 16 {
                self.pos_flag = true;
                self.row = 0;
                self.step += 1;
                if self.step >= song.steps[0].len() {
                    self.step = 0;
                }
            }
        }
        if self.pos_flag {
            if self.rep_count == 0 || self.first_repeat {
                let s = self.step.min(self.visited.len() - 1);
                if self.visited[s] {
                    self.done = true;
                    self.visited.iter_mut().for_each(|x| *x = false);
                }
                self.visited[s] = true;
            }
            self.pos_flag = false;
        }
    }

    fn optional(&mut self, c: usize, opt: u8, data: u8) {
        let v22 = self.song.v22;
        let v = &mut self.voices[c];
        match opt {
            0x0 => v.arp_value = data,
            0x1 => {
                let d = data.min(64);
                v.volume = d;
                v.default_volume = false;
                if !v22 || !v.synth_mode {
                    self.paula.set_volume(c, u32::from(d));
                }
            }
            0x2 => {
                self.count = data;
                self.delay = data;
            }
            0x3 => {}
            0x4 => {
                v.period = v.period.wrapping_sub(u16::from(data));
                v.arp_value = 0;
            }
            0x5 => {
                v.period = v.period.wrapping_add(u16::from(data));
                v.arp_value = 0;
            }
            0x6 => {
                if !v22 {
                    if self.rep_count == 0 {
                        self.rep_count = data;
                        if data != 0 {
                            self.first_repeat = true;
                        }
                    }
                } else {
                    v.vibrato = data as i8;
                }
            }
            0x7 => {
                if !v22 {
                    if self.rep_count != 0 {
                        self.first_repeat = false;
                        self.rep_count -= 1;
                        if self.rep_count != 0 {
                            self.new_pos = usize::from(data);
                            self.pos_flag = true;
                        }
                    }
                } else {
                    self.new_pos = usize::from(data);
                    self.pos_flag = true;
                }
                self.new_pos = self.new_pos.min(self.song.steps[0].len() - 1);
            }
            0x8 => v.auto_slide = data as i8,
            0x9 => {
                v.auto_arp = data;
                if v22 {
                    v.adsr_ptr = 0;
                    if v.adsr_control == 0 {
                        v.adsr_control = 1;
                    }
                }
            }
            0xB => v.fx_control = data,
            0xD => {
                v.auto_arp = data;
                v.fx_control ^= 1;
                v.adsr_ptr = 0;
                if v.adsr_control == 0 {
                    v.adsr_control = 1;
                }
            }
            0xE => {
                v.auto_arp = data;
                v.adsr_ptr = 0;
                if v.adsr_control == 0 {
                    v.adsr_control = 1;
                }
            }
            0xF => v.auto_arp = data,
            _ => {}
        }
    }

    fn play_it(&mut self, c: usize) {
        let song = self.song.clone();
        self.voices[c].restart = false;
        let p = self.voices[c].period;
        self.paula.set_period(c, u32::from(p));
        let k = self.voices[c].instrument;
        if k == 0 {
            return;
        }
        match song.instruments.get(usize::from(k) - 1) {
            Some(Instrument::Synth(s)) => {
                let off = usize::from(s.wave_table) * 64;
                let v = &mut self.voices[c];
                v.synth_mode = true;
                v.eg_ptr = 0;
                v.lfo_ptr = 0;
                v.adsr_ptr = 0;
                v.mod_ptr = 0;
                v.eg_count = s.eg_delay.wrapping_add(1);
                v.lfo_count = s.lfo_delay.wrapping_add(1);
                v.adsr_count = 1;
                v.mod_count = s.mod_delay.wrapping_add(1);
                v.fx_count = s.fx_delay.wrapping_add(1);
                v.fx_control = s.fx_control;
                v.eg_control = s.eg_control;
                v.lfo_control = s.lfo_control;
                v.adsr_control = s.adsr_control;
                v.mod_control = s.mod_control;
                v.old_eg = 0;
                let len = usize::from(s.wave_length);
                self.paula.play(c, off, len);
                self.paula.set_loop(c, off, len);
                let v = &mut self.voices[c];
                let vol = if v.adsr_control != 0 {
                    let t = (i32::from(self.paula.mem.get(usize::from(s.adsr_table) * 64).copied().unwrap_or(0)) + 128) / 4;
                    if v.default_volume {
                        v.volume = s.volume as u8;
                        v.default_volume = false;
                    }
                    t * i32::from(v.volume) / 16
                } else {
                    i32::from(if v.default_volume { s.volume } else { u16::from(v.volume) }) * 4
                };
                self.paula.set_volume_fine(c, vol.clamp(0, 256) as u32);
                let v = &mut self.voices[c];
                if v.eg_control != 0 || v.mod_control != 0 || v.fx_control != 0 {
                    v.synth_offset = Some(off);
                    for (i, b) in v.buffer.iter_mut().enumerate() {
                        *b = self.paula.mem.get(off + i).copied().unwrap_or(0);
                    }
                }
            }
            Some(Instrument::Sample { at, len, loop_start, loop_len, volume, .. }) => {
                let v = &mut self.voices[c];
                v.synth_mode = false;
                v.lfo_control = 0;
                if *len == 0 {
                    self.paula.stop(c);
                } else {
                    let vol = i32::from(if v.default_volume { *volume } else { u16::from(v.volume) }) * 4;
                    self.paula.play(c, *at, *len);
                    if *loop_len > 2 {
                        self.paula.set_loop(c, at + loop_start, *loop_len);
                    }
                    self.paula.set_volume_fine(c, vol.clamp(0, 256) as u32);
                }
            }
            None => {}
        }
    }

    fn effects(&mut self) {
        self.arp_count = self.arp_count.wrapping_sub(1) & 3;
        self.vib_index = (self.vib_index + 1) & 7;
        for c in 0..4 {
            let v = &mut self.voices[c];
            v.period = v.period.wrapping_add(v.auto_slide as i16 as u16);
            let p = if v.vibrato != 0 { v.period.wrapping_add((VIBRATO[self.vib_index] / i16::from(v.vibrato)) as u16) } else { v.period };
            self.paula.set_period(c, u32::from(p));
            let v = &mut self.voices[c];
            if v.arp_value != 0 || v.auto_arp != 0 {
                let mut note = i32::from(v.note as i8);
                if self.arp_count == 0 {
                    note += i32::from(v.arp_value >> 4) + i32::from(v.auto_arp >> 4);
                } else if self.arp_count == 1 {
                    note += i32::from(v.arp_value & 0x0F) + i32::from(v.auto_arp & 0x0F);
                }
                v.restart = false;
                v.period = period((note + 36 - 1).clamp(0, PERIODS as i32 - 1) as usize);
                let p = v.period;
                self.paula.set_period(c, u32::from(p));
            }
        }
    }

    fn wave(&self, table: u8, i: usize) -> i8 {
        self.paula.mem.get(usize::from(table) * 64 + i).copied().unwrap_or(0)
    }

    fn synths(&mut self) {
        for c in 0..4 {
            if !self.voices[c].synth_mode {
                continue;
            }
            let Some(s) = self.synth(c) else { continue };
            // ADSR: the volume from a wave table.
            if self.voices[c].adsr_control != 0 {
                let v = &mut self.voices[c];
                v.adsr_count = v.adsr_count.wrapping_sub(1);
                if v.adsr_count == 0 {
                    v.adsr_count = s.adsr_speed;
                    let ptr = usize::from(v.adsr_ptr);
                    let vol = i32::from(v.volume);
                    let t = ((i32::from(self.wave(s.adsr_table, ptr)) + 128) / 4) * vol / 16;
                    self.paula.set_volume_fine(c, t.clamp(0, 256) as u32);
                    let v = &mut self.voices[c];
                    v.adsr_ptr += 1;
                    if v.adsr_ptr == s.adsr_length {
                        v.adsr_ptr = 0;
                        if v.adsr_control == 1 {
                            v.adsr_control = 0;
                        }
                    }
                }
            }
            // LFO: the period swung by a wave table.
            if self.voices[c].lfo_control != 0 {
                let v = &mut self.voices[c];
                v.lfo_count = v.lfo_count.wrapping_sub(1);
                if v.lfo_count == 0 {
                    v.lfo_count = s.lfo_speed;
                    let ptr = usize::from(v.lfo_ptr);
                    let mut t = i32::from(self.wave(s.lfo_table, ptr));
                    if s.lfo_depth != 0 {
                        t /= i32::from(s.lfo_depth);
                    }
                    let p = self.voices[c].period.wrapping_add(t as i16 as u16);
                    self.paula.set_period(c, u32::from(p));
                    let v = &mut self.voices[c];
                    v.lfo_ptr += 1;
                    if v.lfo_ptr == s.lfo_length {
                        v.lfo_ptr = 0;
                        if v.lfo_control == 1 {
                            v.lfo_control = 0;
                        }
                    }
                }
            }
            let Some(off) = self.voices[c].synth_offset else { continue };
            // EG: part of the waveform inverted, as far as a table says.
            if self.voices[c].eg_control != 0 {
                let v = &mut self.voices[c];
                v.eg_count = v.eg_count.wrapping_sub(1);
                if v.eg_count == 0 {
                    v.eg_count = s.eg_speed;
                    let old = usize::from(v.old_eg);
                    let ptr = usize::from(v.eg_ptr);
                    let new = ((i32::from(self.wave(s.eg_table, ptr)) + 128) / 8) as usize;
                    let v = &mut self.voices[c];
                    v.old_eg = new as u8;
                    let buf = v.buffer;
                    let mem = &mut self.paula.mem;
                    if new < old {
                        for j in 0..old - new {
                            let (d, src) = (off + old - 1 - j, old - 1 - j);
                            if let (Some(m), Some(&x)) = (mem.get_mut(d), buf.get(src)) {
                                *m = x;
                            }
                        }
                    } else if new > old {
                        for j in 0..new - old {
                            let (d, src) = (off + old + j, old + j);
                            if let (Some(m), Some(&x)) = (mem.get_mut(d), buf.get(src)) {
                                *m = x.wrapping_neg();
                            }
                        }
                    }
                    let v = &mut self.voices[c];
                    v.eg_ptr += 1;
                    if v.eg_ptr == s.eg_length {
                        v.eg_ptr = 0;
                        if v.eg_control == 1 {
                            v.eg_control = 0;
                        }
                    }
                }
            }
            // Effects that remake the waveform.
            let fx = self.voices[c].fx_control;
            let delta = s.fx_speed as i8;
            let buf = self.voices[c].buffer;
            let mem = &mut self.paula.mem;
            let toward = |mem: &mut Vec<i8>, src: &dyn Fn(usize, &Vec<i8>) -> i8| {
                for i in 0..32 {
                    let sv = src(i, mem);
                    if let Some(m) = mem.get_mut(off + i) {
                        if sv < *m {
                            *m = m.wrapping_sub(delta);
                        } else if sv > *m {
                            *m = m.wrapping_add(delta);
                        }
                    }
                }
            };
            match fx {
                1 => {
                    let v = &mut self.voices[c];
                    v.fx_count = v.fx_count.wrapping_sub(1);
                    if v.fx_count == 0 {
                        v.fx_count = s.fx_speed;
                        // Averaging: each point the mean of it (as remade) and the next.
                        let mut last = mem.get(off).copied().unwrap_or(0);
                        for i in 0..31 {
                            let next = mem.get(off + i + 1).copied().unwrap_or(0);
                            last = ((i16::from(last) + i16::from(next)) / 2) as i8;
                            if let Some(m) = mem.get_mut(off + i) {
                                *m = last;
                            }
                        }
                    }
                }
                2 => toward(mem, &|i, _| buf[31 - i]),
                3 | 5 => toward(mem, &|i, _| buf[i]),
                4 => toward(mem, &|i, m| m.get(off + 64 + i).copied().unwrap_or(0)),
                6 => {
                    let v = &mut self.voices[c];
                    v.fx_count = v.fx_count.wrapping_sub(1);
                    if v.fx_count == 0 {
                        v.fx_control = 0;
                        v.fx_count = 1;
                        for i in 0..32 {
                            if let Some(&x) = mem.get(off + 64 + i) {
                                mem[off + i] = x;
                            }
                        }
                    }
                }
                _ => {}
            }
            // MOD: a point of the waveform's next written from a table.
            if self.voices[c].mod_control != 0 {
                let v = &mut self.voices[c];
                v.mod_count = v.mod_count.wrapping_sub(1);
                if v.mod_count == 0 {
                    v.mod_count = s.mod_speed;
                    let ptr = usize::from(v.mod_ptr);
                    let x = self.wave(s.mod_table, ptr);
                    if let Some(m) = self.paula.mem.get_mut(off + 32) {
                        *m = x;
                    }
                    let v = &mut self.voices[c];
                    v.mod_ptr += 1;
                    if v.mod_ptr == s.mod_length {
                        v.mod_ptr = 0;
                        if v.mod_control == 1 {
                            v.mod_control = 0;
                        }
                    }
                }
            }
        }
    }
}

impl Driver for Player {
    fn channels(&self) -> usize {
        4
    }

    fn tick(&mut self) {
        self.effects();
        self.synths();
        self.count = self.count.wrapping_sub(1);
        if self.count == 0 {
            self.count = self.delay;
            self.next();
            for c in 0..4 {
                if self.voices[c].restart {
                    // The waveform a synth sound remade, put back.
                    if let Some(off) = self.voices[c].synth_offset.take() {
                        let buf = self.voices[c].buffer;
                        for (i, &x) in buf.iter().enumerate() {
                            if let Some(m) = self.paula.mem.get_mut(off + i) {
                                *m = x;
                            }
                        }
                    }
                    self.play_it(c);
                }
            }
        }
    }

    fn tick_seconds(&self) -> f64 {
        1.0 / CIA_50
    }

    fn render(&mut self, rate: u32, frames: usize, out: &mut Bufs) {
        self.paula.render(rate, frames, out);
    }

    fn position(&self) -> (usize, usize) {
        self.shown
    }

    fn speed(&self) -> (u32, u32) {
        (u32::from(self.delay), 125)
    }

    fn take_done(&mut self) -> bool {
        std::mem::take(&mut self.done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods_are_soundmons() {
        let t: Vec<u16> = (0..PERIODS).map(period).collect();
        assert_eq!(&t[..4], &[6848, 6464, 6080, 5760]);
        assert_eq!(t[36], 856);
        assert_eq!(&t[60..64], &[214, 202, 190, 180]);
        assert_eq!(&t[78..], &[76, 72, 68, 64, 60, 57]);
    }
}
