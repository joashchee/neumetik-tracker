// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! Synthesis ("Synth4.0", or "Synth4.2" after its own player), InStereo!
//! 1.0's sibling and laid out as it is: sub-songs of positions, each
//! channel a run of the song's rows (transposed); samples and synth
//! sounds, a waveform with its start inverted as far as an envelope table
//! says, or reshaped as it plays by one of 16 effects (rotations,
//! negations, mixes of other waveforms, octave folds, smoothing, a laser
//! and an FM drum's pitch sweeps); ADSR tables, arpeggios from the
//! instrument or the row, vibrato, portamento between notes and slides.

use std::sync::Arc;

use crate::instereo::{self, Instrument1, Kind, Line, SubSong};
use crate::paula::{Paula, CIA_50};
use crate::replay::{Bufs, Driver, Tune};
use crate::sonic::period;

/// Where "Synth4.2" files keep their song, after the player.
const AFTER_PLAYER: usize = 0x1F0E;

pub fn probe(d: &[u8]) -> bool {
    (d.len() >= 204 && d.starts_with(b"Synth4.0")) || (d.len() >= AFTER_PLAYER + 204 && &d[AFTER_PLAYER..AFTER_PLAYER + 8] == b"Synth4.2")
}

#[derive(Clone, Debug)]
pub struct Song {
    pub song: instereo::Song,
    file: Arc<Vec<u8>>,
}

impl Song {
    pub fn parse(d: &[u8]) -> Option<Song> {
        if !probe(d) {
            return None;
        }
        let at = if d.starts_with(b"Synth4.0") { 0 } else { AFTER_PLAYER };
        let mut song = instereo::Song::parse1(&d[at..])?;
        // Room past the last sample, for one played beyond its end.
        song.mem.resize(song.mem.len() + 0x1_0000, 0);
        Some(Song { song, file: Arc::new(d.to_vec()) })
    }

    pub fn instruments(&self) -> &[Instrument1] {
        match &self.song.kind {
            Kind::V1 { instruments, .. } => instruments,
            Kind::V2 { .. } => &[],
        }
    }
}

impl Tune for Song {
    fn format(&self) -> &'static str {
        "syn"
    }
    fn file(&self) -> &[u8] {
        &self.file
    }
    fn songs(&self) -> usize {
        self.song.subsongs.len()
    }
    fn driver(&self, song: usize) -> Box<dyn Driver> {
        Box::new(Player::new(Arc::new(self.clone()), song.min(self.song.subsongs.len() - 1)))
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
    new_volume: u8,
    slide_speed: i8,
    slide: i16,
    porta_speed: i8,
    porta_counter: i16,
    vib_delay: u8,
    vib_pos: u8,
    adsr_on: bool,
    adsr_pos: usize,
    egc_off: bool,
    egc_mode: u8,
    egc_pos: usize,
    arp_pos: u8,
    fx_off: bool,
    fx: u8,
    fx_args: [u8; 3],
    synth_pos: u8,
    slow: u8,
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
    /// 0 every transposition, 1 none of the instruments', 2 none of the
    /// notes', 3 neither.
    transpose: u8,
    visited: Vec<bool>,
    shown: (usize, usize),
    done: bool,
}

impl Player {
    fn new(song: Arc<Song>, k: usize) -> Player {
        let mut paula = Paula::new(4, song.song.mem.clone());
        let buffers = std::array::from_fn(|_| [paula.alloc(256), paula.alloc(256)]);
        let sub = song.song.subsongs[k];
        let visited = vec![false; song.song.positions.len()];
        Player { paula, buffers, sub, voices: Default::default(), vibrato: instereo::vibrato1(), counter: sub.speed, speed: sub.speed, position: i32::from(sub.first), row: sub.rows, rows: sub.rows, transpose: 0, visited, shown: (0, 0), done: false, song }
    }

    fn row(&mut self) {
        self.counter = 0;
        let song = self.song.clone();
        if self.row >= self.rows {
            self.row = 0;
            if self.position > i32::from(self.sub.last) {
                self.position = i32::from(self.sub.restart);
            }
            let p = (self.position.max(0) as usize).min(self.visited.len() - 1);
            if self.visited[p] {
                self.done = true;
                self.visited.iter_mut().for_each(|x| *x = false);
            }
            self.visited[p] = true;
            self.position += 1;
            let pos = song.song.positions[p];
            for (v, &(start, st, nt)) in self.voices.iter_mut().zip(&pos) {
                v.start = start;
                v.sound_tr = st;
                v.note_tr = nt;
            }
        }
        self.shown = (((self.position - 1).max(0) as usize).saturating_sub(usize::from(self.sub.first)), usize::from(self.row));
        for v in self.voices.iter_mut() {
            v.line = song.song.lines.get(usize::from(v.start) + usize::from(self.row)).copied().unwrap_or_default();
        }
        for c in 0..4 {
            self.play_row(c);
        }
        self.row += 1;
    }

    fn play_row(&mut self, c: usize) {
        let song = self.song.clone();
        let Kind::V1 { instruments, waveforms, samples, .. } = &song.song.kind else { return };
        let v = &mut self.voices[c];
        v.slide_speed = 0;
        let (e, a) = (v.line.effect, v.line.arg);
        if e != 0 {
            v.fx_off = false;
            v.fx = 0;
            v.fx_args = [0; 3];
            v.egc_off = false;
            v.new_volume = 0;
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
                0x4 => {
                    if a <= 64 {
                        self.rows = a;
                    }
                }
                0x5 => self.transpose = a,
                0x8 => {
                    if (1..=16).contains(&a) {
                        self.speed = a;
                    }
                }
                0x9 => v.fx_off = a != 0,
                0xA => v.fx = a,
                0xB..=0xD => v.fx_args[usize::from(e - 0xB)] = a,
                0xE => v.egc_off = a != 0,
                0xF => v.new_volume = a,
                _ => {}
            }
        }
        let v = &mut self.voices[c];
        let mut note = v.line.note;
        if note == 0 {
            return;
        }
        if note == 0x7F {
            self.paula.stop(c);
            return;
        }
        if self.transpose != 2 && self.transpose != 3 {
            note = note.wrapping_add(v.note_tr as u8);
        }
        v.prev_note = v.note;
        v.note = note;
        self.paula.set_period(c, u32::from(period(usize::from(note))));
        let mut n = v.line.instrument;
        if n == 0 {
            return;
        }
        if self.transpose != 1 && self.transpose != 3 {
            n = n.wrapping_add(v.sound_tr as u8);
        }
        v.instrument = n;
        let Some(ins) = instruments.get(usize::from(n).wrapping_sub(1)).copied() else {
            v.instrument = 0;
            self.paula.stop(c);
            return;
        };
        v.adsr_on = ins.adsr;
        v.adsr_pos = 0;
        v.vib_pos = 0;
        v.egc_mode = 0;
        v.egc_pos = 0;
        v.slide = 0;
        v.arp_pos = 0;
        v.porta_speed = ins.portamento;
        v.porta_counter = i16::from(ins.portamento);
        // Change Argument 1 on a note plays it without its portamento.
        if e == 0xB {
            v.porta_speed = 0;
            v.porta_counter = 0;
        }
        v.vib_delay = ins.vib_delay;
        let volume = if v.new_volume != 0 { v.new_volume } else { ins.volume };
        if ins.synth {
            if ins.effect != 0 {
                v.slow = 0;
                v.synth_pos = 0;
            }
            v.buffer = 1;
            v.egc_mode = ins.egc_mode;
            if ins.egc_mode == 0 {
                v.slow = 0;
                let len = usize::from(ins.length).min(256);
                let buf = self.buffers[c][0];
                let src = waveforms.get(usize::from(ins.waveform)).copied();
                self.paula.play(c, buf, len);
                self.paula.set_loop(c, buf, len);
                for i in 0..len {
                    let x = src.and_then(|s| self.paula.mem.get(s + i).copied()).unwrap_or(0);
                    self.paula.mem[buf + i] = if i < usize::from(ins.egc_offset) { x.wrapping_neg() } else { x };
                }
            }
            self.paula.set_volume(c, u32::from(volume));
        } else if ins.length != 0 {
            v.slow = 0;
            v.synth_pos = 0;
            let k = usize::from(ins.waveform & 0x3F);
            let Some(&(at, _)) = samples.get(k) else {
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
            self.paula.play(c, at, play);
            if ll != 0 {
                self.paula.set_loop(c, at + ls, ll);
            }
            self.paula.set_volume(c, u32::from(volume));
        }
    }

    fn effects(&mut self, c: usize) {
        let song = self.song.clone();
        let Kind::V1 { instruments, waveforms, egc, adsr, arps, .. } = &song.song.kind else { return };
        let v = &mut self.voices[c];
        let Some(ins) = instruments.get(usize::from(v.instrument).wrapping_sub(1)).copied() else {
            self.paula.stop(c);
            return;
        };
        // The arpeggio: the row's table stepped by the speed counter, or the
        // instrument's own, its repeat looping.
        let arp = |i: usize| arps.get(i).copied().unwrap_or(0);
        let (mut note, mut prev) = (v.note, v.prev_note);
        if v.line.arpeggio == 0 {
            if note == 0 {
                self.paula.stop(c);
                return;
            }
            let len = ins.arp_length.wrapping_add(ins.arp_repeat);
            if len != 0 {
                let x = arp(usize::from(ins.arp_start) + usize::from(v.arp_pos));
                v.arp_pos = if v.arp_pos == len { ins.arp_length } else { v.arp_pos.wrapping_add(1) };
                note = note.wrapping_add(x);
                prev = prev.wrapping_add(x);
            }
        } else {
            let x = arp(usize::from(v.line.arpeggio) * 16 + usize::from(self.counter));
            note = note.wrapping_add(x);
            prev = prev.wrapping_add(x);
        }
        let (mut p, mut pp) = (period(usize::from(note)), period(usize::from(prev)));
        if ins.effect != 0 && (ins.egc_mode == 0 || v.egc_off) && !v.fx_off {
            self.synth_effect(c, &ins, waveforms, egc);
        }
        let v = &mut self.voices[c];
        // Portamento: from the last note to this one, over its speed in ticks.
        if v.porta_counter != 0 && p != pp {
            v.porta_counter -= 1;
            std::mem::swap(&mut p, &mut pp);
            let mut np = (i32::from(p) - i32::from(pp)) * i32::from(v.porta_counter);
            if v.porta_speed != 0 {
                np /= i32::from(v.porta_speed);
            }
            p = (np + i32::from(pp)) as u16;
        }
        if ins.vib_level != 0 {
            if v.vib_delay == 0 {
                let x = i32::from(self.vibrato[usize::from(v.vib_pos)]);
                let d = (x.abs() * 4) / i32::from(ins.vib_level);
                p = if x < 0 { p.wrapping_sub(d as u16) } else { p.wrapping_add(d as u16) };
                v.vib_pos = v.vib_pos.wrapping_add(ins.vib_speed);
            } else {
                v.vib_delay -= 1;
            }
        }
        p = p.wrapping_add(v.slide as u16);
        self.paula.set_period(c, u32::from(p));
        v.slide = v.slide.wrapping_sub(i16::from(v.slide_speed));
        if v.adsr_on {
            if v.adsr_pos >= usize::from(ins.adsr_length) {
                v.instrument = 0;
                self.paula.stop(c);
                return;
            }
            let x = u32::from(adsr.get(usize::from(ins.adsr_table) * 256 + v.adsr_pos).copied().unwrap_or(0)) + 1;
            let vol = if v.new_volume != 0 { v.new_volume } else { ins.volume };
            self.paula.set_volume(c, (u32::from(vol) * x / 128).min(64));
            v.adsr_pos += 1;
        }
        // The envelope generator: the waveform copied out (16 points at a
        // time) with its start inverted, into one buffer and the other in turn.
        if v.egc_mode != 0 && !v.egc_off {
            v.buffer ^= 1;
            let buf = self.buffers[c][usize::from(v.buffer == 0)];
            let len = usize::from(ins.length).min(256);
            // A channel not playing yet starts on it.
            if !self.paula.playing(c) {
                self.paula.play(c, buf, len);
            }
            self.paula.set_loop(c, buf, len);
            let src = waveforms.get(usize::from(ins.waveform)).copied();
            for i in 0..len / 16 * 16 {
                self.paula.mem[buf + i] = src.and_then(|s| self.paula.mem.get(s + i).copied()).unwrap_or(0);
            }
            let x = egc.get(usize::from(ins.egc_table) * 128 + v.egc_pos).copied().unwrap_or(0).wrapping_add(ins.egc_offset);
            for i in 0..usize::from(x) {
                self.paula.mem[buf + i] = self.paula.mem[buf + i].wrapping_neg();
            }
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

    /// The instrument's waveform effect (or the row's), on the channel's
    /// first buffer, the one playing while the envelope generator is off.
    fn synth_effect(&mut self, c: usize, ins: &Instrument1, waveforms: &[usize], egc: &[u8]) {
        let v = &mut self.voices[c];
        let fx = if v.fx != 0 { v.fx } else { ins.effect };
        let [a1, a2, a3] = std::array::from_fn(|k| if v.fx_args[k] != 0 { v.fx_args[k] } else { ins.args[k] });
        let buf = self.buffers[c][0];
        let mem = &mut self.paula.mem;
        // The buffer, and room past it for an effect reaching beyond.
        let mut s = [0i8; 512];
        s[..256].copy_from_slice(&mem[buf..buf + 256]);
        let wave = |k: u8, i: usize| -> i8 { waveforms.get(usize::from(k)).and_then(|&w| mem.get(w + (i & 0xFF)).copied()).unwrap_or(0) };
        let eg = |k: u8, i: usize| -> u8 { egc.get(usize::from(k) * 128 + i).copied().unwrap_or(0) };
        // Most effects wait their slow-motion level in ticks between steps.
        let slowed = |v: &mut Voice, level: u8| -> bool {
            if v.slow == 0 {
                v.slow = level;
                true
            } else {
                v.slow -= 1;
                false
            }
        };
        let step = |v: &mut Voice, last: u8| v.synth_pos = if v.synth_pos == last { 0 } else { v.synth_pos.wrapping_add(1) };
        match fx {
            // Rotate 1: a span's points raised.
            1 => {
                if a1 <= a2 {
                    for i in usize::from(a1)..=usize::from(a2 - a1) {
                        s[i] = s[i].wrapping_add(a3 as i8);
                    }
                }
            }
            // Rotate 2: a span's points turned round by one.
            2 => {
                if slowed(v, a3) && a1 <= a2 {
                    let n = usize::from(a2 - a1);
                    let first = s[usize::from(a1)];
                    for i in usize::from(a1)..n {
                        s[i] = s[i + 1];
                    }
                    s[n] = first;
                }
            }
            // Alien: another waveform mixed in.
            3 => {
                if slowed(v, a3) {
                    for i in 0..=usize::from(a2) {
                        s[i] = s[i].wrapping_add(wave(a1, i));
                    }
                }
            }
            // Negator: a point inverted, moving along the span.
            4 => {
                if slowed(v, a3) && a1 <= a2 {
                    let n = a2 - a1;
                    let at = usize::from(a1) + usize::from(v.synth_pos);
                    s[at] = s[at].wrapping_neg();
                    v.synth_pos = if at == usize::from(n) { 0 } else { v.synth_pos.wrapping_add(1) };
                }
            }
            // Poly negator: the last one put back, the next inverted.
            5 => {
                if slowed(v, a3) && a1 <= a2 {
                    let n = usize::from(a2 - a1);
                    let back = if v.synth_pos == 0 { usize::from(a1) + n } else { usize::from(a1) + usize::from(v.synth_pos) - 1 };
                    s[back] = s[back].wrapping_neg();
                    let at = usize::from(a1) + usize::from(v.synth_pos);
                    s[at] = s[at].wrapping_neg();
                    step(v, n as u8);
                }
            }
            // Shakers: another waveform's next point added to them all (and
            // in 2, the point under it inverted).
            6 | 7 => {
                if slowed(v, a3) {
                    let x = wave(a1, usize::from(v.synth_pos));
                    for i in 0..=usize::from(a2) {
                        s[i] = s[i].wrapping_add(x);
                        if fx == 7 && i == usize::from(v.synth_pos) {
                            s[i] = s[i].wrapping_neg();
                        }
                    }
                    step(v, a2);
                }
            }
            // AMF LFO: another waveform as the pitch's slide.
            8 => {
                if slowed(v, a3) {
                    v.slide = -i16::from(wave(a1, usize::from(v.synth_pos)));
                    step(v, a2);
                }
            }
            // Laser: the pitch swept, back after so many steps.
            9 => {
                if slowed(v, a3) {
                    v.slide = v.slide.wrapping_sub(i16::from(a1 as i8));
                    if v.synth_pos == a2 {
                        v.synth_pos = 0;
                        v.slide = 0;
                    } else {
                        v.synth_pos += 1;
                    }
                }
            }
            // Octave 1: every other point, twice over (an octave up).
            10 => {
                if v.slow == 0 {
                    v.slow = a3;
                    let n = usize::from(a1) / 2;
                    let mut j = 0;
                    let mut i = 0;
                    while j <= n {
                        s[j] = s[i];
                        i += 2;
                        j += 1;
                    }
                    for i in 0..=n {
                        s[j] = s[i];
                        j += 1;
                    }
                } else if a1 != 0 {
                    v.slow -= 1;
                }
            }
            // Octave 2: each point doubled (an octave down).
            11 => {
                if slowed(v, a3) {
                    let mut m = a1;
                    for i in (0..=usize::from(a1) / 2).rev() {
                        let x = s[i];
                        m = m.wrapping_sub(1);
                        s[usize::from(m)] = x;
                        m = m.wrapping_sub(1);
                        s[usize::from(m)] = x;
                    }
                }
            }
            // Aliasing: each point moved towards the next by a level.
            12 => {
                if slowed(v, a3) {
                    let mut at = 0;
                    for i in 0..=usize::from(a1) {
                        let (x, y) = (s[at], s[i + 1]);
                        if y > x {
                            s[at] = x.wrapping_add(a2 as i8);
                            at += 1;
                        } else if y < x {
                            s[at] = x.wrapping_sub(a2 as i8);
                            at += 1;
                        }
                    }
                }
            }
            // Envelope effects: each point moved towards the next by an
            // envelope table's step (1: its next for them all; 2: down it).
            13 | 14 => {
                if slowed(v, a3) {
                    let mut n = a1;
                    if fx == 13 {
                        n = n.min((egc.len().saturating_sub(usize::from(a2) * 128)).saturating_sub(1).min(255) as u8);
                    }
                    for i in 0..=usize::from(n) {
                        let d = if fx == 13 { eg(a2, usize::from(v.synth_pos)) } else { eg(a2, usize::from(n) - i) } as i8;
                        let (x, y) = (s[i], s[i + 1]);
                        if y > x {
                            s[i] = x.wrapping_add(d);
                        } else if y < x {
                            s[i] = x.wrapping_sub(d);
                        }
                    }
                    if fx == 13 {
                        step(v, n);
                    }
                }
            }
            // Changer: each point a step towards another waveform's.
            15 => {
                if slowed(v, a3) {
                    for i in 0..=usize::from(a2) {
                        let (x, y) = (s[i] as u8, wave(a1, i) as u8);
                        if y > x {
                            s[i] = x.wrapping_add(1) as i8;
                        } else if y < x {
                            s[i] = x.wrapping_sub(1) as i8;
                        }
                    }
                }
            }
            // FM drum: the pitch swept by level times factor, back after its depth.
            16 => {
                v.slide = v.slide.wrapping_add((u16::from(a1) * u16::from(a2)) as i16);
                if v.synth_pos == a3 {
                    v.synth_pos = 0;
                    v.slide = 0;
                } else {
                    v.synth_pos += 1;
                }
            }
            _ => {}
        }
        mem[buf..buf + 256].copy_from_slice(&s[..256]);
    }
}

impl Driver for Player {
    fn channels(&self) -> usize {
        4
    }

    fn tick(&mut self) {
        self.counter += 1;
        if self.counter >= self.speed {
            self.row();
        }
        for c in 0..4 {
            self.effects(c);
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
        (u32::from(self.speed.max(1)), 125)
    }

    fn take_done(&mut self) -> bool {
        std::mem::take(&mut self.done)
    }
}
