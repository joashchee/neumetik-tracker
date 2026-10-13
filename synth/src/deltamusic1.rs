// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! Delta Music 1 ("ALL "): each channel its own list of 16-row blocks
//! (each transposed, with jumps), every channel at its own speed counter;
//! instruments are samples, or synth sounds stepping through a table of
//! 32-point waveforms, under an attack, decay, sustain and release, an
//! eight-step arpeggio, vibrato, a bend rate and portamento, every one of
//! which the song's effects can change as it plays.

use std::sync::Arc;

use crate::paula::{Paula, CIA_50};
use crate::replay::{Bufs, Driver, Tune};

/// Delta Music 1's periods: Delta Music 2's, but for the 1,016 its
/// replayer leaves out (so every note from there on is a place lower).
pub fn period(i: usize) -> u16 {
    crate::deltamusic2::period(if i < 34 { i } else { i + 1 })
}

const PERIODS: usize = 84;

#[derive(Clone, Copy, Debug, Default)]
pub struct Line {
    pub instrument: u8,
    pub note: u8,
    pub effect: u8,
    pub arg: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Instrument {
    pub number: usize,
    pub attack_step: u8,
    pub attack_delay: u8,
    pub decay_step: u8,
    pub decay_delay: u8,
    pub sustain: u16,
    pub release_step: u8,
    pub release_delay: u8,
    pub volume: u8,
    pub vibrato_wait: u8,
    pub vibrato_step: u8,
    pub vibrato_length: u8,
    pub bend_rate: i8,
    pub portamento: u8,
    pub is_sample: bool,
    pub table_delay: u8,
    pub arpeggio: [u8; 8],
    pub length: usize,
    pub repeat_start: usize,
    pub repeat_length: usize,
    /// A synth sound's 48-step table of waveforms.
    pub table: Vec<u8>,
    /// Where its points are in memory, and how many.
    pub at: usize,
    pub size: usize,
}

#[derive(Clone, Debug)]
pub struct Song {
    /// Each channel's list: (block, transpose); (FF, -1) jumps to the next entry's place.
    pub tracks: [Vec<(u8, i8)>; 4],
    pub blocks: Vec<[Line; 16]>,
    pub instruments: Vec<Option<Instrument>>,
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
    if d.len() < 104 || &d[..4] != b"ALL " {
        return false;
    }
    let total: u64 = 104 + (0..25).map(|i| u64::from(be32(d, 4 + i * 4).unwrap_or(u32::MAX))).sum::<u64>();
    total <= d.len() as u64
}

impl Song {
    pub fn parse(d: &[u8]) -> Option<Song> {
        if !probe(d) {
            return None;
        }
        let tl: Vec<usize> = (0..4).map(|i| be32(d, 4 + i * 4).map(|v| v as usize)).collect::<Option<_>>()?;
        let bl = be32(d, 20)? as usize;
        let il: Vec<usize> = (0..20).map(|i| be32(d, 24 + i * 4).map(|v| v as usize)).collect::<Option<_>>()?;
        let mut at = 104;
        let mut tracks: [Vec<(u8, i8)>; 4] = Default::default();
        for (c, t) in tracks.iter_mut().enumerate() {
            for _ in 0..tl[c] / 2 {
                t.push((*d.get(at)?, *d.get(at + 1)? as i8));
                at += 2;
            }
        }
        let mut blocks = Vec::with_capacity(bl / 64);
        for _ in 0..bl / 64 {
            let mut b = [Line::default(); 16];
            for l in b.iter_mut() {
                let x = d.get(at..at + 4)?;
                *l = Line { instrument: x[0], note: x[1], effect: x[2], arg: x[3] };
                at += 4;
            }
            blocks.push(b);
        }
        if blocks.is_empty() {
            return None;
        }
        let mut mem = Vec::new();
        let mut instruments = vec![None; 20];
        for (i, &len) in il.iter().enumerate() {
            if len == 0 {
                continue;
            }
            let b = d.get(at..at + 30)?;
            let mut ins = Instrument {
                number: i,
                attack_step: b[0],
                attack_delay: b[1],
                decay_step: b[2],
                decay_delay: b[3],
                sustain: u16::from_be_bytes([b[4], b[5]]),
                release_step: b[6],
                release_delay: b[7],
                volume: b[8],
                vibrato_wait: b[9],
                vibrato_step: b[10],
                vibrato_length: b[11],
                bend_rate: b[12] as i8,
                portamento: b[13],
                is_sample: b[14] != 0,
                table_delay: b[15],
                length: usize::from(be16(b, 24)?),
                repeat_start: usize::from(be16(b, 26)?),
                repeat_length: usize::from(be16(b, 28)?),
                ..Default::default()
            };
            ins.arpeggio.copy_from_slice(&b[16..24]);
            let mut p = at + 30;
            if !ins.is_sample {
                ins.table = d.get(p..p + 48)?.to_vec();
                p += 48;
            }
            let size = len.saturating_sub(if ins.is_sample { 30 } else { 78 });
            ins.at = mem.len();
            ins.size = size;
            mem.extend(d.get(p..(p + size).min(d.len())).unwrap_or_default().iter().map(|&x| x as i8));
            mem.resize(ins.at + size, 0);
            at += len;
            instruments[i] = Some(ins);
        }
        // Room past the last sound, for a waveform read beyond it.
        mem.resize(mem.len() + 256, 0);
        Some(Song { tracks, blocks, instruments, mem, file: Arc::new(d.to_vec()) })
    }

    /// A channel's blocks in order, its jumps followed, until one comes
    /// round (at most `most`): (block, transpose).
    pub fn walk(&self, channel: usize, most: usize) -> Vec<(usize, i8)> {
        let t = &self.tracks[channel];
        let mut out = Vec::new();
        let mut k = 0usize;
        let mut seen = vec![false; t.len() + 1];
        while out.len() < most && !t.is_empty() {
            if k >= t.len() {
                break;
            }
            let (b, tr) = t[k];
            if b == 0xFF && tr == -1 {
                let Some(&(hi, lo)) = t.get(k + 1) else { break };
                let to = ((usize::from(hi) << 8) | usize::from(lo as u8)) & 0x7FF;
                if seen[to.min(t.len())] {
                    break;
                }
                k = to;
                continue;
            }
            if seen[k] {
                break;
            }
            seen[k] = true;
            out.push((usize::from(b).min(self.blocks.len() - 1), tr));
            k += 1;
        }
        out
    }
}

impl Tune for Song {
    fn format(&self) -> &'static str {
        "dm1"
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
    instrument: Option<usize>,
    period: u16,
    table_pos: usize,
    table_delay: u8,
    track_pos: usize,
    block: usize,
    block_pos: usize,
    vib_wait: u8,
    vib_length: u8,
    vib_pos: u8,
    vib_compare: u8,
    vib_freq: u16,
    freq: u8,
    volume: u8,
    attack_delay: u8,
    decay_delay: u8,
    sustain: u16,
    release_delay: u8,
    speed: u8,
    bend: i16,
    transpose: i8,
    status: u8,
    arp: usize,
    effect: u8,
    arg: u8,
    retrigger: bool,
    /// Blocks started, for the screen.
    steps: usize,
}

pub struct Player {
    song: Arc<Song>,
    paula: Paula,
    instruments: Vec<Option<Instrument>>,
    voices: [Voice; 4],
    speed: u8,
    lead: usize,
    shown: (usize, usize),
    done: bool,
}

impl Player {
    fn new(song: Arc<Song>) -> Player {
        let lead = (0..4).max_by_key(|&c| (song.tracks[c].len(), 3 - c)).unwrap_or(0);
        let voices = std::array::from_fn(|_| Voice { speed: 1, ..Default::default() });
        Player { paula: Paula::new(4, song.mem.clone()), instruments: song.instruments.clone(), voices, speed: 6, lead, shown: (0, 0), done: false, song }
    }

    fn channel(&mut self, c: usize) {
        let song = self.song.clone();
        let track = &song.tracks[c];
        let v = &mut self.voices[c];
        v.speed = v.speed.wrapping_sub(1);
        if v.speed == 0 {
            v.speed = self.speed;
            if v.block_pos == 0 && !track.is_empty() {
                if v.track_pos >= track.len() {
                    v.track_pos = 0;
                    if c == self.lead {
                        self.done = true;
                    }
                }
                let mut guard = 0;
                let mut step = track[v.track_pos];
                while step.0 == 0xFF && step.1 == -1 && guard < track.len() {
                    let old = v.track_pos;
                    let (hi, lo) = track.get(v.track_pos + 1).copied().unwrap_or((0, 0));
                    v.track_pos = ((usize::from(hi) << 8) | usize::from(lo as u8)) & 0x7FF;
                    if v.track_pos < old && c == self.lead {
                        self.done = true;
                    }
                    if v.track_pos >= track.len() {
                        v.track_pos = 0;
                    }
                    step = track[v.track_pos];
                    guard += 1;
                }
                v.transpose = step.1;
                v.block = usize::from(step.0).min(song.blocks.len() - 1);
                v.track_pos += 1;
                v.steps += 1;
            }
            if c == self.lead {
                self.shown = (v.steps.saturating_sub(1), v.block_pos);
            }
            let line = song.blocks[v.block][v.block_pos];
            if line.effect != 0 {
                v.effect = line.effect;
                v.arg = line.arg;
            }
            if line.note != 0 {
                v.freq = line.note.wrapping_add(v.transpose as u8);
                v.status = 0;
                v.bend = 0;
                v.arp = 0;
                v.effect = line.effect;
                v.arg = line.arg;
                let k = usize::from(line.instrument);
                if let Some(ins) = self.instruments.get(k).and_then(|i| i.as_ref()) {
                    v.instrument = Some(k);
                    v.table_pos = 0;
                    if ins.is_sample {
                        self.paula.play(c, ins.at, ins.length.min(ins.size));
                        if ins.repeat_length > 1 {
                            self.paula.set_loop(c, ins.at + ins.repeat_start, ins.repeat_length);
                        }
                    } else {
                        v.retrigger = true;
                    }
                    v.vib_wait = ins.vibrato_wait;
                    v.vib_length = ins.vibrato_length;
                    v.vib_pos = ins.vibrato_length;
                    v.vib_compare = ins.vibrato_length.wrapping_mul(2);
                    v.volume = 0;
                    v.table_delay = 0;
                    v.attack_delay = 0;
                    v.decay_delay = 0;
                    v.sustain = ins.sustain;
                    v.release_delay = 0;
                }
            }
            let v = &mut self.voices[c];
            v.block_pos = (v.block_pos + 1) & 0x0F;
        }
        let Some(k) = self.voices[c].instrument else { return };
        // The sound table.
        if !self.instruments[k].as_ref().is_some_and(|i| i.is_sample) {
            if self.voices[c].table_delay == 0 {
                self.sound_table(c, k);
            } else {
                self.voices[c].table_delay -= 1;
            }
        }
        let freq = self.voices[c].freq;
        let Some(ins) = self.instruments[k].as_mut() else { return };
        let v = &mut self.voices[c];
        // Portamento.
        if ins.portamento != 0 {
            let want = period(usize::from(freq).min(PERIODS - 1)).wrapping_add(v.bend as u16);
            if v.period == 0 {
                v.period = want;
            } else if v.period > want {
                let p = v.period.wrapping_sub(u16::from(ins.portamento));
                v.period = if p < want { want } else { p };
            } else if v.period < want {
                let p = v.period.wrapping_add(u16::from(ins.portamento));
                v.period = if p > want { want } else { p };
            }
        }
        // Vibrato: a triangle between 0 and twice its length.
        if v.vib_wait == 0 {
            v.vib_freq = u16::from(v.vib_pos).wrapping_mul(u16::from(ins.vibrato_step));
            if v.status & 1 != 0 {
                v.vib_pos = v.vib_pos.wrapping_sub(1);
                if v.vib_pos == 0 {
                    v.status ^= 1;
                }
            } else {
                v.vib_pos = v.vib_pos.wrapping_add(1);
                if v.vib_pos == v.vib_compare {
                    v.status ^= 1;
                }
            }
        } else {
            v.vib_wait -= 1;
        }
        // The bend rate.
        v.bend = v.bend.wrapping_sub(i16::from(ins.bend_rate));
        // The effects (each changes its instrument as it plays).
        let x = v.arg;
        match v.effect {
            0x01 => {
                if x != 0 {
                    self.speed = x;
                }
            }
            0x02 => v.bend = v.bend.wrapping_sub(i16::from(x)),
            0x03 => v.bend = v.bend.wrapping_add(i16::from(x)),
            0x05 => ins.vibrato_wait = x,
            0x06 => ins.vibrato_step = x,
            0x07 => ins.vibrato_length = x,
            0x08 => ins.bend_rate = x as i8,
            0x09 => ins.portamento = x,
            0x0A => ins.volume = x.min(64),
            0x0B..=0x12 => ins.arpeggio[usize::from(v.effect - 0x0B)] = x,
            0x13..=0x16 => {
                let k = usize::from(v.effect - 0x13);
                ins.arpeggio[k] = x;
                ins.arpeggio[k + 4] = x;
            }
            0x17 => ins.attack_step = x.min(64),
            0x18 => ins.attack_delay = x,
            0x19 => ins.decay_step = x.min(64),
            0x1A => ins.decay_delay = x,
            0x1B => ins.sustain = (ins.sustain & 0x00FF) | (u16::from(x) << 8),
            0x1C => ins.sustain = (ins.sustain & 0xFF00) | u16::from(x),
            0x1D => ins.release_step = x.min(64),
            0x1E => ins.release_delay = x,
            _ => {}
        }
        // The arpeggio and the period played.
        let a = ins.arpeggio[v.arp];
        v.arp = (v.arp + 1) & 7;
        let mut p = period((usize::from(freq) + usize::from(a)).min(PERIODS - 1));
        p = p.wrapping_sub(u16::from(v.vib_length).wrapping_mul(u16::from(ins.vibrato_step))).wrapping_add(v.bend as u16);
        if ins.portamento != 0 {
            p = v.period;
        } else {
            v.period = 0;
        }
        p = p.wrapping_add(v.vib_freq);
        // The envelope: attack, decay to the volume, sustain, release.
        let mut vol = i32::from(v.volume);
        let mut status = v.status & 0b1110;
        if status == 0 {
            if v.attack_delay == 0 {
                v.attack_delay = ins.attack_delay;
                vol += i32::from(ins.attack_step);
                if vol >= 64 {
                    vol = 64;
                    status |= 0b0010;
                    v.status |= 0b0010;
                }
            } else {
                v.attack_delay -= 1;
            }
        }
        if status == 0b0010 {
            if v.decay_delay == 0 {
                v.decay_delay = ins.decay_delay;
                vol -= i32::from(ins.decay_step);
                if vol <= i32::from(ins.volume) {
                    vol = i32::from(ins.volume);
                    status |= 0b0110;
                    v.status |= 0b0110;
                }
            } else {
                v.decay_delay -= 1;
            }
        }
        if status == 0b0110 {
            if v.sustain == 0 {
                status |= 0b1110;
                v.status |= 0b1110;
            } else {
                v.sustain -= 1;
            }
        }
        if status == 0b1110 {
            if v.release_delay == 0 {
                v.release_delay = ins.release_delay;
                vol -= i32::from(ins.release_step);
                if vol <= 0 {
                    vol = 0;
                    v.status &= 0b1001;
                }
            } else {
                v.release_delay -= 1;
            }
        }
        v.volume = vol as u8;
        let vol = u32::from(v.volume);
        self.paula.set_period(c, u32::from(p));
        self.paula.set_volume(c, vol);
    }

    /// The sound table's next step: a waveform (32 points into the
    /// sound), a new delay, or a jump.
    fn sound_table(&mut self, c: usize, k: usize) {
        let Some(ins) = self.instruments[k].as_mut() else { return };
        let v = &mut self.voices[c];
        v.table_delay = ins.table_delay;
        for _ in 0..96 {
            if v.table_pos >= 48 {
                v.table_pos = 0;
            }
            let data = ins.table.get(v.table_pos).copied().unwrap_or(0);
            if data == 0xFF {
                v.table_pos = usize::from(ins.table.get(v.table_pos + 1).copied().unwrap_or(0));
            } else if data >= 0x80 {
                ins.table_delay = data & 0x7F;
                v.table_pos += 1;
            } else {
                v.table_pos += 1;
                let at = ins.at + usize::from(data) * 32;
                let len = ins.length;
                if v.retrigger {
                    self.paula.play(c, at, len);
                    v.retrigger = false;
                }
                self.paula.set_loop(c, at, len);
                break;
            }
        }
    }
}

impl Driver for Player {
    fn channels(&self) -> usize {
        4
    }

    fn tick(&mut self) {
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
        (u32::from(self.speed), 125)
    }

    fn take_done(&mut self) -> bool {
        std::mem::take(&mut self.done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods_leave_one_out() {
        assert_eq!(period(33), 1076);
        assert_eq!(period(34), 960);
        assert_eq!(period(36), 856);
        assert_eq!(period(83), 113);
    }
}
