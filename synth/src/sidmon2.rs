// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! SidMon 2 ("SIDMON II - THE MIDI VERSION"): each channel steps through
//! its tracks (each transposed in note and instrument), packed rows of
//! notes, instruments, effects and runs of empty rows. Instruments play
//! samples stepped through a waveform list, under an attack, decay,
//! sustain and release, with arpeggio and vibrato lists and a pitch bend;
//! a sample can have one of its points inverted, the point moving each
//! tick (its "negation"); ProTracker's arpeggio, slides, volumes and speed
//! as effects.

use std::sync::Arc;

use crate::paula::{Paula, CIA_50};
use crate::replay::{Bufs, Driver, Tune};

/// SidMon 2's periods: from ProTracker's octave 2 (428 to 226) four,
/// eight and sixteen times (the last from its fourth note), its octave 1,
/// that octave 2, its octave 3, and three a quarter of octave 2's.
pub fn period(i: usize) -> u16 {
    const O1: [u16; 12] = [856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453];
    const O2: [u16; 12] = [428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226];
    const O3: [u16; 12] = [214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113];
    match i {
        0 => 0,
        1..=9 => O2[i + 2] * 16,
        10..=21 => O2[i - 10] * 8,
        22..=33 => O2[i - 22] * 4,
        34..=45 => O1[i - 34],
        46..=57 => O2[i - 46],
        58..=69 => O3[i - 58],
        70..=72 => O2[i - 70] / 4,
        _ => 0,
    }
}

const PERIODS: usize = 73;

#[derive(Clone, Copy, Debug, Default)]
pub struct Step {
    pub track: usize,
    pub note_transpose: i8,
    pub instrument_transpose: i8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Instrument {
    pub wave_list: u8,
    pub wave_length: u8,
    pub wave_speed: u8,
    pub wave_delay: u8,
    pub arpeggio: u8,
    pub arpeggio_length: u8,
    pub arpeggio_speed: u8,
    pub arpeggio_delay: u8,
    pub vibrato: u8,
    pub vibrato_length: u8,
    pub vibrato_speed: u8,
    pub vibrato_delay: u8,
    pub pitch_bend_speed: i8,
    pub pitch_bend_delay: u8,
    pub attack_max: u8,
    pub attack_speed: u8,
    pub decay_min: u8,
    pub decay_speed: u8,
    pub sustain: u8,
    pub release_min: u8,
    pub release_speed: u8,
}

/// How a sample's negation moves.
#[derive(Clone, Copy, Debug, Default)]
pub struct Negate {
    pub start: u32,
    pub end: u32,
    pub loop_index: u16,
    pub status: u16,
    pub speed: i16,
    pub position: i32,
    pub index: u16,
    pub flag: i16,
}

#[derive(Clone, Debug, Default)]
pub struct Sample {
    pub name: String,
    pub at: usize,
    pub len: usize,
    pub loop_start: usize,
    pub loop_len: usize,
    pub negate: Negate,
}

/// A track's row as the screen shows it: (note, instrument, effect, argument, row).
pub type Event = (u8, u8, u8, u8, usize);

#[derive(Clone, Debug)]
pub struct Song {
    pub positions: usize,
    pub speed: u8,
    pub sequences: [Vec<Step>; 4],
    pub tracks: Vec<Vec<u8>>,
    pub instruments: Vec<Instrument>,
    pub wave_lists: Vec<[u8; 16]>,
    pub arpeggios: Vec<[i8; 16]>,
    pub vibratos: Vec<[i8; 16]>,
    pub samples: Vec<Sample>,
    pub mem: Vec<i8>,
    file: Arc<Vec<u8>>,
}

const MARK: &[u8] = b"SIDMON II - THE MIDI VERSION";

pub fn probe(d: &[u8]) -> bool {
    d.len() >= 86 && &d[58..86] == MARK
}

fn be16(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(i..i + 2)?.try_into().ok()?))
}

fn be32(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?))
}

fn lists<T: Copy + Default>(d: &[u8], at: usize, len: usize, conv: fn(u8) -> T) -> Option<Vec<[T; 16]>> {
    if !len.is_multiple_of(16) {
        return None;
    }
    (0..len / 16)
        .map(|k| {
            let b = d.get(at + k * 16..at + k * 16 + 16)?;
            let mut l = [T::default(); 16];
            for (o, &x) in l.iter_mut().zip(b) {
                *o = conv(x);
            }
            Some(l)
        })
        .collect()
}

impl Song {
    pub fn parse(d: &[u8]) -> Option<Song> {
        if !probe(d) {
            return None;
        }
        let positions = usize::from(*d.get(2)?) + 1;
        let speed = *d.get(3)?;
        let nsamples = usize::from(be16(d, 4)?) / 64;
        // The blocks, one after another from the mark.
        let mut off = 58 + be32(d, 6)? as usize + be32(d, 10)? as usize;
        let mut blocks = [(0usize, 0usize); 10];
        for (k, b) in blocks.iter_mut().enumerate() {
            let len = be32(d, 14 + k * 4)? as usize;
            *b = (off, len);
            off += len;
        }
        let [pos_t, note_t, ins_t, instr, wave_l, arp_l, vib_l, smp_i, trk_t, trks] = blocks;
        let wave_lists = lists(d, wave_l.0, wave_l.1, |x| x)?;
        let arpeggios = lists(d, arp_l.0, arp_l.1, |x| x as i8)?;
        let vibratos = lists(d, vib_l.0, vib_l.1, |x| x as i8)?;
        if pos_t.1 != note_t.1 || note_t.1 != ins_t.1 || pos_t.1 != positions * 4 {
            return None;
        }
        let mut sequences: [Vec<Step>; 4] = Default::default();
        for (c, seq) in sequences.iter_mut().enumerate() {
            for j in 0..positions {
                let k = c * positions + j;
                seq.push(Step { track: usize::from(*d.get(pos_t.0 + k)?), note_transpose: *d.get(note_t.0 + k)? as i8, instrument_transpose: *d.get(ins_t.0 + k)? as i8 });
            }
        }
        let ntracks = trk_t.1 / 2;
        let offs: Vec<usize> = (0..ntracks).map(|k| be16(d, trk_t.0 + k * 2).map(usize::from)).collect::<Option<_>>()?;
        let mut tracks = Vec::with_capacity(ntracks);
        for k in 0..ntracks {
            let end = if k + 1 == ntracks { trks.1 } else { offs[k + 1] };
            tracks.push(d.get(trks.0 + offs[k]..trks.0 + end.max(offs[k]))?.to_vec());
        }
        for seq in &mut sequences {
            for s in seq.iter_mut() {
                s.track = s.track.min(tracks.len().saturating_sub(1));
            }
        }
        if tracks.is_empty() || !instr.1.is_multiple_of(32) {
            return None;
        }
        let instruments: Vec<Instrument> = (0..instr.1 / 32)
            .map(|k| {
                let b = d.get(instr.0 + k * 32..instr.0 + k * 32 + 32)?;
                Some(Instrument {
                    wave_list: b[0],
                    wave_length: b[1],
                    wave_speed: b[2],
                    wave_delay: b[3],
                    arpeggio: b[4],
                    arpeggio_length: b[5],
                    arpeggio_speed: b[6],
                    arpeggio_delay: b[7],
                    vibrato: b[8],
                    vibrato_length: b[9],
                    vibrato_speed: b[10],
                    vibrato_delay: b[11],
                    pitch_bend_speed: b[12] as i8,
                    pitch_bend_delay: b[13],
                    attack_max: b[16],
                    attack_speed: b[17],
                    decay_min: b[18],
                    decay_speed: b[19],
                    sustain: b[20],
                    release_min: b[21],
                    release_speed: b[22],
                })
            })
            .collect::<Option<_>>()?;
        if instruments.is_empty() || wave_lists.is_empty() || arpeggios.is_empty() || vibratos.is_empty() {
            return None;
        }
        // The samples' settings, then their points after the tracks.
        let mut samples = Vec::with_capacity(nsamples);
        for k in 0..nsamples {
            let o = smp_i.0 + k * 64;
            let b = d.get(o..o + 64)?;
            let len = usize::from(be16(b, 4)?) * 2;
            let mut ls = usize::from(be16(b, 6)?) * 2;
            let mut ll = usize::from(be16(b, 8)?) * 2;
            if ls > len {
                ls = 0;
                ll = 0;
            } else if ls + ll > len {
                ll = len - ls;
            }
            let negate = Negate {
                start: u32::from(be16(b, 10)?) * 2,
                end: u32::from(be16(b, 12)?) * 2,
                loop_index: be16(b, 14)?,
                status: be16(b, 16)?,
                speed: be16(b, 18)? as i16,
                position: be32(b, 20)? as i32,
                index: be16(b, 24)?,
                flag: be16(b, 26)? as i16,
            };
            let name = b[32..64].iter().take_while(|&&c| c != 0).map(|&c| if (32..127).contains(&c) { c as char } else { ' ' }).collect::<String>().trim().to_string();
            samples.push(Sample { name, at: 0, len, loop_start: ls, loop_len: ll, negate });
        }
        let mut mem = Vec::new();
        let mut at = trks.0 + trks.1;
        for s in samples.iter_mut() {
            s.at = mem.len();
            mem.extend(d.get(at..(at + s.len).min(d.len())).unwrap_or_default().iter().map(|&x| x as i8));
            mem.resize(s.at + s.len, 0);
            at += s.len;
        }
        if samples.is_empty() {
            return None;
        }
        Some(Song { positions, speed, sequences, tracks, instruments, wave_lists, arpeggios, vibratos, samples, mem, file: Arc::new(d.to_vec()) })
    }

    /// A track's rows as the replayer reads them: each event and the row it's on.
    pub fn events(&self, track: usize) -> Vec<Event> {
        let t = &self.tracks[track];
        let mut out = Vec::new();
        let (mut at, mut row) = (0usize, 0usize);
        while at < t.len() && row < 256 {
            let v = t[at] as i8;
            at += 1;
            if v == 0 {
                out.push((0, 0, *t.get(at).unwrap_or(&0), *t.get(at + 1).unwrap_or(&0), row));
                at += 2;
                row += 1;
                continue;
            }
            if v > 0 {
                if v >= 0x70 {
                    out.push((0, 0, v as u8, *t.get(at).unwrap_or(&0), row));
                    at += 1;
                    row += 1;
                    continue;
                }
                let note = v as u8;
                let w = *t.get(at).unwrap_or(&0xFF) as i8;
                at += 1;
                if w >= 0 {
                    if w >= 0x70 {
                        out.push((note, 0, w as u8, *t.get(at).unwrap_or(&0), row));
                        at += 1;
                        row += 1;
                        continue;
                    }
                    let x = *t.get(at).unwrap_or(&0xFF) as i8;
                    at += 1;
                    if x >= 0 {
                        out.push((note, w as u8, x as u8, *t.get(at).unwrap_or(&0), row));
                        at += 1;
                        row += 1;
                        continue;
                    }
                    out.push((note, w as u8, 0, 0, row));
                    row += 1 + usize::from(!x as u8);
                    continue;
                }
                out.push((note, 0, 0, 0, row));
                row += 1 + usize::from(!w as u8);
                continue;
            }
            row += 1 + usize::from(!v as u8);
        }
        out
    }
}

impl Tune for Song {
    fn format(&self) -> &'static str {
        "sid2"
    }
    fn file(&self) -> &[u8] {
        &self.file
    }
    fn driver(&self, _song: usize) -> Box<dyn Driver> {
        Box::new(Player::new(Arc::new(self.clone())))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Env {
    #[default]
    Attack,
    Decay,
    Sustain,
    Release,
    Done,
}

#[derive(Clone, Default)]
struct Voice {
    track: usize,
    track_pos: usize,
    note_transpose: i8,
    instrument_transpose: i8,
    empty: u16,
    pitch_bend: i16,
    volume: i16,
    wave_delay: i32,
    wave_offset: i32,
    arp_delay: i32,
    arp_offset: i32,
    vib_delay: i32,
    vib_offset: i32,
    bend_counter: u8,
    slide_speed: i16,
    env: Env,
    sustain: u8,
    instrument: usize,
    sample: usize,
    period: u16,
    original_note: u16,
    note: u16,
    ins: u16,
    effect: u8,
    arg: u8,
    slide_note: u16,
}

pub struct Player {
    song: Arc<Song>,
    paula: Paula,
    instruments: Vec<Instrument>,
    negates: Vec<Negate>,
    voices: [Voice; 4],
    position: usize,
    row: usize,
    length: usize,
    rast: u32,
    rast2: usize,
    speed: u32,
    shown: (usize, usize),
    done: bool,
}

impl Player {
    fn new(song: Arc<Song>) -> Player {
        let mut p = Player {
            paula: Paula::new(4, song.mem.clone()),
            instruments: song.instruments.clone(),
            negates: song.samples.iter().map(|s| s.negate).collect(),
            voices: Default::default(),
            position: 0,
            row: 0,
            length: 64,
            rast: 0,
            rast2: 0,
            speed: u32::from(song.speed),
            shown: (0, 0),
            done: false,
            song,
        };
        let first = usize::from(p.song.wave_lists[usize::from(p.instruments[0].wave_list).min(p.song.wave_lists.len() - 1)][0]);
        for c in 0..4 {
            p.voices[c] = Voice { env: Env::Attack, instrument: 0, sample: first.min(p.song.samples.len() - 1), ..Default::default() };
            p.find_note(c);
        }
        p
    }

    fn find_note(&mut self, c: usize) {
        let s = self.song.sequences[c].get(self.position).copied().unwrap_or_default();
        let v = &mut self.voices[c];
        v.track = s.track;
        v.track_pos = 0;
        v.note_transpose = s.note_transpose;
        v.instrument_transpose = s.instrument_transpose;
        v.empty = 0;
    }

    fn get_note(&mut self, c: usize) {
        let song = self.song.clone();
        let t = &song.tracks[self.voices[c].track];
        let v = &mut self.voices[c];
        v.note = 0;
        v.ins = 0;
        v.effect = 0;
        v.arg = 0;
        let next = |v: &mut Voice| -> u8 {
            let b = t.get(v.track_pos).copied();
            v.track_pos += 1;
            b.unwrap_or(0)
        };
        if v.empty == 0 {
            let mut x = if v.track_pos < t.len() { next(v) as i8 } else { -1 };
            'read: {
                if x == 0 {
                    v.effect = next(v);
                    v.arg = next(v);
                    break 'read;
                }
                if x > 0 {
                    if x >= 0x70 {
                        v.effect = x as u8;
                        v.arg = next(v);
                        break 'read;
                    }
                    v.note = x as u16;
                    x = next(v) as i8;
                    if x >= 0 {
                        if x >= 0x70 {
                            v.effect = x as u8;
                            v.arg = next(v);
                            break 'read;
                        }
                        v.ins = x as u16;
                        x = next(v) as i8;
                        if x >= 0 {
                            v.effect = x as u8;
                            v.arg = next(v);
                            break 'read;
                        }
                    }
                }
                v.empty = !x as u8 as u16;
            }
        } else {
            v.empty -= 1;
        }
        if v.note != 0 {
            v.note = v.note.wrapping_add(v.note_transpose as i16 as u16);
        }
    }

    fn play_voice(&mut self, c: usize) {
        let song = self.song.clone();
        let v = &mut self.voices[c];
        v.pitch_bend = 0;
        if v.note == 0 {
            return;
        }
        v.volume = 0;
        v.wave_delay = 0;
        v.wave_offset = 0;
        v.arp_delay = 0;
        v.arp_offset = 0;
        v.vib_delay = 0;
        v.vib_offset = 0;
        v.bend_counter = 0;
        v.slide_speed = 0;
        v.env = Env::Attack;
        v.sustain = 0;
        if v.ins != 0 {
            let k = (i32::from(v.ins) - 1 + i32::from(v.instrument_transpose)) as u16 as usize;
            if k < self.instruments.len() {
                v.instrument = k;
                let list = song.wave_lists.get(usize::from(self.instruments[k].wave_list)).copied().unwrap_or_default();
                v.sample = usize::from(list[0]).min(song.samples.len() - 1);
            }
        }
        let ins = self.instruments[v.instrument];
        let arp = song.arpeggios.get(usize::from(ins.arpeggio)).copied().unwrap_or_default();
        let note = i32::from(v.note) + i32::from(arp[0]);
        if (0..PERIODS as i32).contains(&note) {
            v.original_note = note as u16;
            v.period = period(note as usize);
            let s = &song.samples[v.sample];
            if s.len != 0 {
                self.paula.play(c, s.at, s.len);
                if s.loop_len > 2 {
                    self.paula.set_loop(c, s.at + s.loop_start, s.loop_len);
                }
            } else {
                self.paula.stop(c);
            }
        } else {
            self.paula.stop(c);
        }
        let p = self.voices[c].period;
        self.paula.set_period(c, u32::from(p));
    }

    /// Each sample a voice plays has a point inverted, the point moving
    /// as its settings say (each sample once, however many play it).
    fn negation(&mut self) {
        let mut working = [usize::MAX; 4];
        for c in 0..4 {
            let n = self.voices[c].sample;
            working[c] = n;
            let Some(g) = self.negates.get_mut(n) else { continue };
            if g.flag != 0 {
                continue;
            }
            g.flag = -1;
            if g.index == 0 {
                g.index = g.loop_index;
                if g.status != 0 {
                    let s = &self.song.samples[n];
                    let end = i64::from(g.end) - 1;
                    let at = i64::from(g.start) + i64::from(g.position);
                    if let Some(x) = usize::try_from(at).ok().filter(|&a| a < s.len).and_then(|a| self.paula.mem.get_mut(s.at + a)) {
                        *x = !*x;
                    }
                    g.position += i32::from(g.speed);
                    if g.position < 0 {
                        if g.status == 2 {
                            g.position = end as i32;
                        } else {
                            g.position -= i32::from(g.speed);
                            g.speed = -g.speed;
                        }
                    } else if i64::from(g.position) > end {
                        if g.status == 1 {
                            g.position = 0;
                        } else {
                            g.position -= i32::from(g.speed);
                            g.speed = -g.speed;
                        }
                    }
                }
            } else {
                g.index = (g.index + 1) & 0x1F;
            }
        }
        for n in working {
            if let Some(g) = self.negates.get_mut(n) {
                g.flag = 0;
            }
        }
    }

    fn effects(&mut self, c: usize) {
        let song = self.song.clone();
        let ins = self.instruments[self.voices[c].instrument];
        let v = &mut self.voices[c];
        // The envelope.
        match v.env {
            Env::Attack => {
                v.volume += i16::from(ins.attack_speed);
                if v.volume >= i16::from(ins.attack_max) {
                    v.volume = i16::from(ins.attack_max);
                    v.env = Env::Decay;
                }
            }
            Env::Decay => {
                if ins.decay_speed == 0 {
                    v.env = Env::Sustain;
                } else {
                    v.volume -= i16::from(ins.decay_speed);
                    if v.volume <= i16::from(ins.decay_min) {
                        v.volume = i16::from(ins.decay_min);
                        v.env = Env::Sustain;
                    }
                }
            }
            Env::Sustain => {
                if v.sustain == ins.sustain {
                    v.env = Env::Release;
                } else {
                    v.sustain = v.sustain.wrapping_add(1);
                }
            }
            Env::Release => {
                if ins.release_speed == 0 {
                    v.env = Env::Done;
                } else {
                    v.volume -= i16::from(ins.release_speed);
                    if v.volume <= i16::from(ins.release_min) {
                        v.volume = i16::from(ins.release_min);
                        v.env = Env::Done;
                    }
                }
            }
            Env::Done => {}
        }
        let vol = v.volume;
        self.paula.set_volume_fine(c, vol.clamp(0, 256) as u32);
        // The waveform list: the next sample, after the one playing.
        let v = &mut self.voices[c];
        if ins.wave_length != 0 {
            if v.wave_delay == i32::from(ins.wave_delay) {
                v.wave_delay -= i32::from(ins.wave_speed);
                if v.wave_offset == i32::from(ins.wave_length) {
                    v.wave_offset = -1;
                }
                v.wave_offset += 1;
                let list = song.wave_lists.get(usize::from(ins.wave_list)).copied().unwrap_or_default();
                let w = list[(v.wave_offset as usize).min(15)] as i8;
                if w >= 0 {
                    v.sample = (w as usize).min(song.samples.len() - 1);
                    let s = &song.samples[v.sample];
                    if self.paula.playing(c) {
                        self.paula.set_loop(c, s.at + s.loop_start, s.loop_len);
                    }
                } else {
                    v.wave_offset -= 1;
                }
            } else {
                v.wave_delay += 1;
            }
        }
        let v = &mut self.voices[c];
        // The arpeggio list.
        if ins.arpeggio_length != 0 {
            if v.arp_delay == i32::from(ins.arpeggio_delay) {
                v.arp_delay -= i32::from(ins.arpeggio_speed);
                if v.arp_offset == i32::from(ins.arpeggio_length) {
                    v.arp_offset = -1;
                }
                v.arp_offset += 1;
                let a = song.arpeggios.get(usize::from(ins.arpeggio)).map_or(0, |l| l[(v.arp_offset as usize).min(15)]);
                v.period = period((i32::from(v.original_note) + i32::from(a)).max(0) as usize);
            } else {
                v.arp_delay += 1;
            }
        }
        // ProTracker's effects.
        if v.effect >= 0x70 {
            let e = v.effect & 0x0F;
            if self.rast != 0 || e >= 5 {
                match e {
                    0x0 => {
                        let tab = [v.arg >> 4, 0, v.arg & 0x0F, 0];
                        v.period = period(usize::from(v.original_note) + usize::from(tab[self.rast2]));
                    }
                    0x1 => v.pitch_bend = -i16::from(v.arg),
                    0x2 => v.pitch_bend = i16::from(v.arg),
                    0x3 | 0x4 => {
                        if v.env == Env::Done {
                            if self.rast == 0 && v.ins != 0 {
                                v.volume = i16::from(ins.attack_speed);
                            }
                            let d = i16::from(v.arg) * 4;
                            v.volume = if e == 3 { (v.volume + d).min(255) } else { (v.volume - d).max(0) };
                        }
                    }
                    0x5 => {
                        let k = v.instrument;
                        self.instruments[k].attack_max = v.arg;
                        self.instruments[k].attack_speed = v.arg;
                    }
                    0x6 => {
                        if v.arg != 0 {
                            self.length = usize::from(v.arg);
                        }
                    }
                    0xC => {
                        let vol = u32::from(v.arg);
                        self.paula.set_volume(c, vol.min(64));
                        v.volume = (vol * 4).min(255) as i16;
                    }
                    0xF => {
                        let s = u32::from(v.arg & 0x0F);
                        if s != 0 && s != self.speed {
                            self.speed = s;
                        }
                    }
                    _ => {}
                }
            }
        }
        let v = &mut self.voices[c];
        // The vibrato list.
        if ins.vibrato_length != 0 {
            if v.vib_delay == i32::from(ins.vibrato_delay) {
                v.vib_delay -= i32::from(ins.vibrato_speed);
                if v.vib_offset == i32::from(ins.vibrato_length) {
                    v.vib_offset = -1;
                }
                v.vib_offset += 1;
                let x = song.vibratos.get(usize::from(ins.vibrato)).map_or(0, |l| l[(v.vib_offset as usize).min(15)]);
                v.period = v.period.wrapping_add(x as i16 as u16);
            } else {
                v.vib_delay += 1;
            }
        }
        // The pitch bend.
        if ins.pitch_bend_speed != 0 {
            if v.bend_counter == ins.pitch_bend_delay {
                v.pitch_bend = v.pitch_bend.wrapping_add(i16::from(ins.pitch_bend_speed));
            } else {
                v.bend_counter = v.bend_counter.wrapping_add(1);
            }
        }
        // A slide to a note (none while it's there).
        let mut at_note = false;
        if v.effect != 0 && v.effect < 0x70 && v.arg != 0 {
            v.slide_note = period(usize::from(v.effect));
            let dir = i32::from(v.slide_note) - i32::from(v.period);
            if dir == 0 {
                at_note = true;
            } else {
                v.slide_speed = if dir < 0 { -i16::from(v.arg) } else { i16::from(v.arg) };
            }
        }
        if v.slide_speed != 0 && !at_note {
            v.period = v.period.wrapping_add(v.slide_speed as u16);
            let past = if v.slide_speed < 0 { v.period <= v.slide_note } else { v.period >= v.slide_note };
            if past {
                v.period = v.slide_note;
                v.slide_speed = 0;
            }
        }
        v.period = v.period.wrapping_add(v.pitch_bend as u16).clamp(95, 5760);
        let p = v.period;
        self.paula.set_period(c, u32::from(p));
    }
}

impl Driver for Player {
    fn channels(&self) -> usize {
        4
    }

    fn tick(&mut self) {
        self.rast2 += 1;
        if self.rast2 == 3 {
            self.rast2 = 0;
        }
        self.rast += 1;
        if self.rast >= self.speed {
            self.rast = 0;
            self.rast2 = 0;
            self.shown = (self.position, self.row);
            for c in 0..4 {
                self.get_note(c);
            }
            for c in 0..4 {
                self.play_voice(c);
            }
            self.negation();
            self.row += 1;
            if self.row == self.length {
                self.row = 0;
                if self.position + 1 >= self.song.positions {
                    self.position = 0;
                    self.done = true;
                } else {
                    self.position += 1;
                }
                for c in 0..4 {
                    self.find_note(c);
                }
            }
        }
        for c in 0..4 {
            self.effects(c);
        }
        if self.rast != 0 {
            self.negation();
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
        (self.speed, 125)
    }

    fn take_done(&mut self) -> bool {
        std::mem::take(&mut self.done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods_are_sidmons() {
        let t: Vec<u16> = (0..PERIODS).map(period).collect();
        assert_eq!(&t[..10], &[0, 5760, 5424, 5120, 4832, 4560, 4304, 4064, 3840, 3616]);
        assert_eq!(&t[21..25], &[1808, 1712, 1616, 1524]);
        assert_eq!(t[33], 904);
        assert_eq!(t[45], 453);
        assert_eq!(t[69], 113);
        assert_eq!(&t[70..], &[107, 101, 95]);
    }
}
