// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! Delta Music 2 (Bent Nielsen's): each channel its own list of 16-row
//! blocks (each transposed), looping on its own; instruments are samples
//! or synth sounds stepping through a table of 256-point waveforms (the
//! first remade as noise every tick), under a five-stage volume and a
//! five-stage vibrato, with a pitch bend, portamento and an arpeggio
//! table.

use std::sync::Arc;

use crate::paula::{Paula, CIA_50};
use crate::replay::{Bufs, Driver, Tune};

/// Delta Music 2's periods: ProTracker's octave 2 sixteen, eight, four and
/// two times, itself, its octave 3, then its top held.
pub fn period(i: usize) -> u16 {
    const O2: [u16; 12] = [428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226];
    const O3: [u16; 12] = [214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113];
    if i == 0 {
        return 0;
    }
    let (oct, k) = ((i - 1) / 12, (i - 1) % 12);
    match oct {
        0 => O2[k] * 16,
        1 => O2[k] * 8,
        2 => O2[k] * 4,
        3 => O2[k] * 2,
        4 => O2[k],
        5 => O3[k],
        _ => 113,
    }
}

const PERIODS: usize = 85;

#[derive(Clone, Copy, Debug, Default)]
pub struct Line {
    pub note: u8,
    pub instrument: u8,
    pub effect: u8,
    pub arg: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stage {
    pub speed: u8,
    pub level: u8,
    pub sustain: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct VibStage {
    pub speed: u8,
    pub delay: u8,
    pub sustain: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Instrument {
    pub number: usize,
    pub length: usize,
    pub repeat_start: usize,
    pub repeat_length: usize,
    pub volume: [Stage; 5],
    pub vibrato: [VibStage; 5],
    pub pitch_bend: u16,
    pub is_sample: bool,
    /// A sample's slot, or a synth sound's table speed.
    pub sample_number: u8,
    /// 48 entries.
    pub table: Vec<u8>,
    /// A sample's points in memory.
    pub at: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Track {
    pub loop_position: usize,
    /// (block, transpose).
    pub steps: Vec<(usize, i8)>,
}

#[derive(Clone, Debug)]
pub struct Song {
    pub speed: i8,
    pub arpeggios: Vec<[i8; 16]>,
    pub tracks: [Track; 4],
    pub blocks: Vec<[Line; 16]>,
    pub instruments: Vec<Option<Instrument>>,
    /// Where each waveform starts in memory (256 points each).
    pub waveforms: Vec<usize>,
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
    if d.len() < 0xFDA || &d[0xBC6..0xBCA] != b".FNL" {
        return false;
    }
    let tracks: i64 = (0..4).map(|i| i64::from(be16(d, 0xFCA + i * 4 + 2).unwrap_or(0) as i16)).sum();
    let mut at = 0xFDA + tracks;
    let blocks = i64::from(be32(d, at as usize).unwrap_or(u32::MAX) as i32);
    at += 4 + blocks + 256 - 2;
    let instr = i64::from(be16(d, at as usize).unwrap_or(u16::MAX) as i16);
    at += 2 + instr;
    let waves = i64::from(be32(d, at as usize).unwrap_or(u32::MAX) as i32);
    let total = 0xFDA + tracks + blocks + 4 + instr + 256 + waves + 4 + 64;
    tracks >= 0 && blocks >= 0 && instr >= 0 && waves >= 0 && total <= d.len() as i64
}

impl Song {
    pub fn parse(d: &[u8]) -> Option<Song> {
        if !probe(d) {
            return None;
        }
        let speed = *d.get(0xBBB)? as i8;
        let mut at = 0xBCA;
        let mut arpeggios = Vec::with_capacity(64);
        for _ in 0..64 {
            let mut a = [0i8; 16];
            for (k, x) in a.iter_mut().enumerate() {
                *x = *d.get(at + k)? as i8;
            }
            arpeggios.push(a);
            at += 16;
        }
        let mut tracks: [Track; 4] = Default::default();
        let mut lens = [0usize; 4];
        for (i, t) in tracks.iter_mut().enumerate() {
            t.loop_position = usize::from(be16(d, at)?);
            lens[i] = usize::from(be16(d, at + 2)?) / 2;
            at += 4;
        }
        for (i, t) in tracks.iter_mut().enumerate() {
            for _ in 0..lens[i] {
                t.steps.push((usize::from(*d.get(at)?), *d.get(at + 1)? as i8));
                at += 2;
            }
        }
        let nblocks = be32(d, at)? as usize / 64;
        at += 4;
        let mut blocks = Vec::with_capacity(nblocks);
        for _ in 0..nblocks {
            let mut b = [Line::default(); 16];
            for l in b.iter_mut() {
                let x = d.get(at..at + 4)?;
                *l = Line { note: x[0], instrument: x[1], effect: x[2], arg: x[3] };
                at += 4;
            }
            blocks.push(b);
        }
        if blocks.is_empty() {
            return None;
        }
        for t in tracks.iter_mut() {
            for s in t.steps.iter_mut() {
                s.0 = s.0.min(blocks.len() - 1);
            }
        }
        // Instruments: 127 offsets (the first is 0), then the one that ends them.
        let mut offs = vec![0usize; 128];
        for (k, o) in offs.iter_mut().enumerate().skip(1) {
            *o = usize::from(be16(d, at + (k - 1) * 2)?);
        }
        at += 127 * 2;
        let end = usize::from(be16(d, at)?);
        at += 2;
        let base = at;
        let mut instruments: Vec<Option<Instrument>> = vec![None; 128];
        for (k, &o) in offs.iter().enumerate() {
            if o == end {
                break;
            }
            let p = base + o;
            let b = d.get(p..p + 88)?;
            let h = |i: usize| usize::from(u16::from_be_bytes([b[i], b[i + 1]]));
            let mut ins = Instrument { number: k, length: h(0) * 2, repeat_start: h(2), repeat_length: h(4) * 2, ..Default::default() };
            if ins.repeat_start + ins.repeat_length >= ins.length {
                ins.repeat_length = ins.length.saturating_sub(ins.repeat_start);
            }
            for j in 0..5 {
                ins.volume[j] = Stage { speed: b[6 + j * 3], level: b[7 + j * 3], sustain: b[8 + j * 3] };
                ins.vibrato[j] = VibStage { speed: b[21 + j * 3], delay: b[22 + j * 3], sustain: b[23 + j * 3] };
            }
            ins.pitch_bend = h(36) as u16;
            ins.is_sample = b[38] == 0xFF;
            ins.sample_number = b[39];
            ins.table = b[40..88].to_vec();
            instruments[k] = Some(ins);
        }
        at = base + end;
        // Waveforms of 256 points, then 64 bytes, then the samples' eight offsets.
        let nwaves = be32(d, at)? as usize / 256;
        at += 4;
        let mut mem: Vec<i8> = Vec::new();
        let mut waveforms = Vec::with_capacity(nwaves.max(1));
        for _ in 0..nwaves {
            waveforms.push(mem.len());
            mem.extend(d.get(at..at + 256)?.iter().map(|&x| x as i8));
            at += 256;
        }
        if waveforms.is_empty() {
            waveforms.push(mem.len());
            mem.resize(256, 0);
        }
        at += 64;
        let slots: Vec<usize> = (0..8).map(|k| be32(d, at + k * 4).map(|v| v as usize)).collect::<Option<_>>()?;
        at += 32;
        for ins in instruments.iter_mut().flatten().filter(|i| i.is_sample) {
            let p = at + slots[usize::from(ins.sample_number & 7)];
            ins.at = mem.len();
            mem.extend(d.get(p..(p + ins.length).min(d.len())).unwrap_or_default().iter().map(|&x| x as i8));
            mem.resize(ins.at + ins.length, 0);
        }
        Some(Song { speed, arpeggios, tracks, blocks, instruments, waveforms, mem, file: Arc::new(d.to_vec()) })
    }

    /// How many positions the screen shows: the longest channel's.
    pub fn positions(&self) -> usize {
        self.tracks.iter().map(|t| t.steps.len()).max().unwrap_or(0).max(1)
    }

    /// The step a channel plays at a position, its own list looped.
    pub fn step_at(&self, channel: usize, position: usize) -> Option<(usize, i8)> {
        let t = &self.tracks[channel];
        let n = t.steps.len();
        if n == 0 {
            return None;
        }
        let lp = if t.loop_position >= n { 0 } else { t.loop_position };
        let k = if position < n { position } else { lp + (position - n) % (n - lp) };
        t.steps.get(k).copied()
    }
}

impl Tune for Song {
    fn format(&self) -> &'static str {
        "dm2"
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
    block: usize,
    block_pos: usize,
    current: i32,
    next: usize,
    transpose: i8,
    instrument: Option<usize>,
    note: u8,
    period: u16,
    final_period: u16,
    vibrato_period: u16,
    arp: usize,
    arp_pos: usize,
    volume: i16,
    vol_pos: usize,
    vol_sustain: u8,
    portamento: u8,
    pitch_bend: i16,
    max_volume: u8,
    retrigger: bool,
    table_delay: u8,
    table_pos: usize,
    vib_dir: bool,
    vib_delay: u8,
    vib_pos: usize,
    vib_sustain: u8,
}

pub struct Player {
    song: Arc<Song>,
    paula: Paula,
    voices: [Voice; 4],
    speed: i8,
    tick: i8,
    global_volume: u8,
    noise: u32,
    /// The channel the screen follows (the longest list).
    lead: usize,
    shown: (usize, usize),
    done: bool,
}

impl Player {
    fn new(song: Arc<Song>) -> Player {
        let lead = (0..4).max_by_key(|&c| (song.tracks[c].steps.len(), 3 - c)).unwrap_or(0);
        let voices = std::array::from_fn(|_| Voice { current: -1, max_volume: 63, ..Default::default() });
        Player { paula: Paula::new(4, song.mem.clone()), voices, speed: song.speed, tick: 1, global_volume: 63, noise: 0, lead, shown: (0, 0), done: false, song }
    }

    /// The first waveform remade as noise: 16 longs from a rotated,
    /// added and mixed register.
    fn noise(&mut self) {
        let at = self.song.waveforms[0];
        let mut n = self.noise;
        for i in 0..16 {
            n = n.rotate_left(7).wrapping_add(0x6ECA_756D) ^ 0x9E59_A92B;
            for (k, b) in n.to_be_bytes().iter().enumerate() {
                self.paula.mem[at + i * 4 + k] = *b as i8;
            }
        }
        self.noise = n;
    }

    fn channel(&mut self, c: usize) {
        let song = self.song.clone();
        let track = &song.tracks[c];
        if track.steps.is_empty() {
            return;
        }
        if self.tick == 0 {
            let v = &mut self.voices[c];
            if v.block_pos == 0 {
                let (b, tr) = track.steps[v.next];
                v.transpose = tr;
                v.block = b;
                if c == self.lead && v.next as i32 <= v.current {
                    self.done = true;
                }
                v.current = v.next as i32;
                v.next += 1;
                if v.next >= track.steps.len() {
                    v.next = if track.loop_position >= track.steps.len() { 0 } else { track.loop_position };
                }
            }
            if c == self.lead {
                self.shown = (v.current.max(0) as usize, v.block_pos);
            }
            let line = song.blocks[v.block][v.block_pos];
            if line.note != 0 {
                v.note = line.note;
                v.period = period((usize::from(line.note) as i32 + i32::from(v.transpose)).clamp(0, PERIODS as i32 - 1) as usize);
                v.instrument = song.instruments.get(usize::from(line.instrument)).and_then(|i| i.as_ref()).map(|i| i.number);
                if let Some(ins) = v.instrument.and_then(|k| song.instruments[k].as_ref()) {
                    if ins.is_sample {
                        self.paula.play(c, ins.at, ins.length);
                        if ins.repeat_length > 1 {
                            self.paula.set_loop(c, ins.at + ins.repeat_start, ins.repeat_length);
                        }
                    } else {
                        v.retrigger = true;
                    }
                    v.table_delay = 0;
                    v.table_pos = 0;
                    v.volume = 0;
                    v.vol_sustain = 0;
                    v.vol_pos = 0;
                    v.arp_pos = 0;
                    v.vib_dir = false;
                    v.vibrato_period = 0;
                    v.vib_delay = ins.vibrato[0].delay;
                    v.vib_pos = 0;
                    v.vib_sustain = ins.vibrato[0].sustain;
                } else {
                    self.paula.stop(c);
                }
            }
            let v = &mut self.voices[c];
            match line.effect {
                1 => {
                    let s = (line.arg & 0x0F) as i8;
                    if s != self.speed {
                        self.speed = s;
                    }
                }
                3 => v.pitch_bend = -i16::from(line.arg),
                4 => v.pitch_bend = i16::from(line.arg),
                5 => v.portamento = line.arg,
                6 => v.max_volume = line.arg & 0x3F,
                7 => self.global_volume = line.arg & 0x3F,
                8 => v.arp = usize::from(line.arg & 0x3F),
                _ => {}
            }
            v.block_pos = (v.block_pos + 1) & 0x0F;
        }
        let Some(ins) = self.voices[c].instrument.and_then(|k| song.instruments[k].as_ref()) else { return };
        // The sound table: the waveform to play next.
        if !ins.is_sample {
            let v = &mut self.voices[c];
            if v.table_delay != 0 {
                v.table_delay -= 1;
            } else {
                v.table_delay = ins.sample_number;
                let mut data = ins.table[v.table_pos.min(47)];
                let mut skip = false;
                if data == 0xFF {
                    v.table_pos = usize::from(ins.table[(v.table_pos + 1).min(47)]);
                    data = ins.table[v.table_pos.min(47)];
                    skip = data == 0xFF;
                }
                if !skip {
                    let at = song.waveforms.get(usize::from(data)).copied().unwrap_or(song.waveforms[0]);
                    if v.retrigger {
                        if ins.length > 0 {
                            self.paula.play(c, at, ins.length);
                            self.paula.set_loop(c, at, ins.length);
                        } else {
                            self.paula.stop(c);
                        }
                        self.voices[c].retrigger = false;
                    } else if ins.length > 0 {
                        self.paula.set_loop(c, at, ins.length);
                    }
                    let v = &mut self.voices[c];
                    v.table_pos += 1;
                    if v.table_pos >= 48 {
                        v.table_pos = 0;
                    }
                }
            }
        }
        let v = &mut self.voices[c];
        // The vibrato's stages.
        let vi = ins.vibrato[v.vib_pos];
        if v.vib_dir {
            v.vibrato_period = v.vibrato_period.wrapping_sub(u16::from(vi.speed));
        } else {
            v.vibrato_period = v.vibrato_period.wrapping_add(u16::from(vi.speed));
        }
        v.vib_delay = v.vib_delay.wrapping_sub(1);
        if v.vib_delay == 0 {
            v.vib_delay = vi.delay;
            v.vib_dir = !v.vib_dir;
        }
        if v.vib_sustain != 0 {
            v.vib_sustain -= 1;
        } else {
            v.vib_pos = (v.vib_pos + 1).min(4);
            v.vib_sustain = ins.vibrato[v.vib_pos].sustain;
        }
        // The volume's stages.
        if v.vol_sustain != 0 {
            v.vol_sustain -= 1;
        } else {
            let st = ins.volume[v.vol_pos];
            let level = i16::from(st.level);
            let reached = if v.volume >= level {
                v.volume -= i16::from(st.speed);
                v.volume < level
            } else {
                v.volume += i16::from(st.speed);
                v.volume > level
            };
            if reached {
                v.volume = level;
                v.vol_pos = (v.vol_pos + 1).min(4);
                v.vol_sustain = st.sustain;
            }
        }
        // Portamento toward the note.
        if v.portamento != 0 {
            let p = u16::from(v.portamento);
            if v.final_period >= v.period {
                v.final_period = v.final_period.wrapping_sub(p);
                if v.final_period < v.period {
                    v.final_period = v.period;
                }
            } else {
                v.final_period += p;
                if v.final_period > v.period {
                    v.final_period = v.period;
                }
            }
        }
        // The arpeggio.
        let tab = song.arpeggios[v.arp.min(63)];
        let mut a = tab[v.arp_pos];
        if v.arp_pos != 0 && a == -128 {
            v.arp_pos = 0;
            a = tab[0];
        }
        v.arp_pos = (v.arp_pos + 1) & 0x0F;
        if v.portamento == 0 {
            let idx = (a as u8).wrapping_add(v.note).wrapping_add(v.transpose as u8);
            v.final_period = period(usize::from(idx).min(PERIODS - 1));
        }
        v.vibrato_period = v.vibrato_period.wrapping_sub(ins.pitch_bend.wrapping_sub(v.pitch_bend as u16));
        let p = v.final_period.wrapping_add(v.vibrato_period);
        let vol = ((v.volume >> 2) & 0x3F).min(i16::from(v.max_volume)).min(i16::from(self.global_volume));
        self.paula.set_period(c, u32::from(p));
        self.paula.set_volume(c, vol as u32);
    }
}

impl Driver for Player {
    fn channels(&self) -> usize {
        4
    }

    fn tick(&mut self) {
        self.noise();
        self.tick -= 1;
        if self.tick < 0 {
            self.tick = self.speed;
        }
        for c in 0..4 {
            self.channel(c);
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
        (self.speed.max(0) as u32 + 1, 125)
    }

    fn take_done(&mut self) -> bool {
        std::mem::take(&mut self.done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods_are_delta_musics() {
        let t: Vec<u16> = (0..PERIODS).map(period).collect();
        assert_eq!(&t[..4], &[0, 6848, 6464, 6096]);
        assert_eq!(t[24], 1808);
        assert_eq!(t[48], 452);
        assert_eq!(t[72], 113);
        assert_eq!(t[84], 113);
    }
}
