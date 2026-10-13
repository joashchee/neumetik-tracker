// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! Music Assembler (Oscar Giesen and Marco Swagerman): a rip, its
//! replayer before its data, which is found by the replayer's own
//! instructions. Sub-songs of a position list a channel (each position a
//! track, transposed and repeated); tracks of packed rows, each held for
//! its own count of the song's rows; instruments over a sample, or over a
//! short waveform whose start sweeps back and forth (the "wave level"),
//! with an attack to a sustain held for a while then released, an
//! arpeggio of three notes, vibrato, and portamento from the track.

use std::sync::Arc;

use crate::paula::{Paula, CIA_50};
use crate::replay::{Bufs, Driver, Tune};

/// Three octaves and more from C-1 (1712), as ProTracker's.
pub const PERIODS: [u16; 48] = [
    1712, 1616, 1524, 1440, 1356, 1280, 1208, 1140, 1076, 1016, 960, 906, 856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453, 428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226, 214, 202, 190, 180, 170,
    160, 151, 143, 135, 127, 120, 113,
];

#[derive(Clone, Copy, Debug, Default)]
pub struct Position {
    pub track: u8,
    pub transpose: u8,
    /// Plays 1 + this many times.
    pub repeat: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Instrument {
    pub sample: u8,
    pub attack: u8,
    pub decay_sustain: u8,
    pub vibrato_delay: u8,
    pub release: u8,
    pub vibrato_speed: u8,
    pub vibrato_level: u8,
    pub arpeggio: u8,
    pub arp_speed: u8,
    pub hold: u8,
    pub key_wave_rate: u8,
    pub wave_level_speed: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Sample {
    pub name: String,
    /// Where its points are in memory (the file's own place for them).
    pub at: usize,
    /// In words, as the replayer keeps them.
    pub len: usize,
    pub loop_len: usize,
}

impl Sample {
    /// A short waveform the replayer sweeps, not a sample played through.
    pub fn synth(&self) -> bool {
        self.len > 1 && self.len <= 128
    }
}

/// A track's row, decoded.
#[derive(Clone, Copy, Debug, Default)]
pub struct Row {
    pub note: Option<u8>,
    pub instrument: Option<u8>,
    pub release: bool,
    pub portamento: u8,
    /// Held for 1 + this many of the song's rows.
    pub delay: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct SubSong {
    pub speed: u8,
    pub lists: [usize; 4],
}

#[derive(Clone, Debug)]
pub struct Song {
    pub subsongs: Vec<SubSong>,
    /// Position lists, by their place in the data.
    pub lists: Vec<(usize, Vec<Position>)>,
    pub tracks: Vec<Vec<u8>>,
    pub instruments: Vec<Instrument>,
    pub samples: Vec<Sample>,
    pub mem: Vec<i8>,
    empty: usize,
    file: Arc<Vec<u8>>,
}

/// Each Paula channel's position list in the sub-song's table.
const CHANNEL_MAP: [usize; 4] = [0, 2, 3, 1];

fn be16(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(i..i + 2)?.try_into().ok()?))
}

fn be32(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?))
}

fn rel16(d: &[u8], i: usize) -> Option<usize> {
    usize::try_from(i as i64 + 2 + i64::from(be16(d, i + 2)? as i16)).ok()
}

/// Where the replayer keeps its data, from its instructions.
#[derive(Clone, Copy, Debug)]
struct Layout {
    subsongs: usize,
    speeds: usize,
    positions: usize,
    start: usize,
    instruments: usize,
    samples: usize,
    tracks: usize,
}

fn find(d: &[u8], from: usize, end: usize, op: [u8; 2]) -> Option<usize> {
    (from..end.min(d.len()).saturating_sub(4)).step_by(2).find(|&i| d[i] == op[0] && d[i + 1] == op[1])
}

fn layout(d: &[u8]) -> Option<Layout> {
    if d.len() < 0x622 || d[0] != 0x60 || d[1] != 0 || d[4] != 0x60 || d[5] != 0 || d[8] != 0x60 || d[9] != 0 || d[12] != 0x48 || d[13] != 0xE7 {
        return None;
    }
    let end = 0x700;
    // The init routine: CMP.W #n,D0, then LEA speeds(PC),A4 and LEA lists(PC,Dx),A4.
    let init = rel16(d, 0)?;
    let i = find(d, init & !1, end, [0xB0, 0x7C])?;
    let subsongs = usize::from(be16(d, i + 2)?);
    let i = find(d, i + 4, end, [0x49, 0xFA])?;
    let speeds = rel16(d, i)?;
    let i = find(d, i + 4, end, [0x49, 0xFB])?;
    let positions = (i as i64 + 2 + i64::from(d[i + 3] as i8)) as usize;
    // The play routine: LEA data(PC),A1, ADDA.L instruments(PC),A1 and ADDA.L samples(PC),A2,
    // then a BSR to where ADDA.L tracks(PC),A5 is.
    let i = find(d, 12, end, [0x43, 0xFA])?;
    let start = rel16(d, i)?;
    let i = (i + 4..end.min(d.len()).saturating_sub(8)).step_by(2).find(|&i| d[i] == 0xD3 && d[i + 1] == 0xFA)?;
    let instruments = rel16(d, i)?;
    if d[i + 4] != 0xD5 || d[i + 5] != 0xFA {
        return None;
    }
    let samples = rel16(d, i + 4)?;
    let j = (i + 8..end.min(d.len()).saturating_sub(2)).step_by(2).find(|&j| d[j] == 0x61)?;
    let j = (j as i64 + 2 + i64::from(d[j + 1] as i8)) as usize;
    let j = find(d, j, end, [0xDB, 0xFA])?;
    let tracks = rel16(d, j)?;
    Some(Layout { subsongs, speeds, positions, start, instruments, samples, tracks })
}

pub fn probe(d: &[u8]) -> bool {
    layout(d).is_some_and(|l| l.subsongs > 0 && l.subsongs < 256 && l.start < d.len() && l.speeds < d.len() && l.positions < d.len())
}

/// A track's bytes: each row 1 to 4 of them; FF after a row ends it.
fn read_track(d: &[u8], mut at: usize) -> Option<Vec<u8>> {
    let mut t = Vec::new();
    loop {
        let b = *d.get(at)?;
        t.push(b);
        at += 1;
        let more = if b & 0x80 == 0 { 1 } else if b & 0x40 != 0 { 2 } else { 0 };
        for k in 0..more {
            let x = *d.get(at)?;
            t.push(x);
            at += 1;
            if k + 1 == more && x & 0x80 != 0 {
                t.push(*d.get(at)?);
                at += 1;
            }
        }
        if *d.get(at)? == 0xFF {
            t.push(0xFF);
            return Some(t);
        }
        if t.len() > 0x10000 {
            return None;
        }
    }
}

impl Song {
    pub fn parse(d: &[u8]) -> Option<Song> {
        if !probe(d) {
            return None;
        }
        let l = layout(d)?;
        let mut subsongs = Vec::new();
        for i in 0..l.subsongs {
            let p: Vec<usize> = (0..4).map(|c| be16(d, l.positions + i * 8 + c * 2).map(usize::from)).collect::<Option<_>>()?;
            // Lists side by side two bytes apart are empty ones: no song.
            if p[0] + 2 == p[1] && p[1] + 2 == p[2] && p[2] + 2 == p[3] {
                continue;
            }
            subsongs.push(SubSong { speed: *d.get(l.speeds + i)?, lists: [p[0], p[1], p[2], p[3]] });
        }
        if subsongs.is_empty() {
            return None;
        }
        let mut lists: Vec<(usize, Vec<Position>)> = Vec::new();
        for s in &subsongs {
            for &o in &s.lists {
                if lists.iter().any(|(k, _)| *k == o) {
                    continue;
                }
                let mut at = l.start + o;
                let mut list = Vec::new();
                loop {
                    let t = *d.get(at)?;
                    let b = *d.get(at + 1)?;
                    at += 2;
                    list.push(Position { track: t, transpose: b >> 4, repeat: b & 0x0F });
                    if t >= 0xFE {
                        break;
                    }
                    if list.len() > 4096 {
                        return None;
                    }
                }
                lists.push((o, list));
            }
        }
        let count = lists.iter().flat_map(|(_, l)| l.iter()).filter(|p| p.track < 0xFE).map(|p| usize::from(p.track) + 1).max().unwrap_or(1);
        let tracks_at = l.start + be32(d, l.tracks)? as usize;
        let tracks: Vec<Vec<u8>> = (0..count).map(|i| read_track(d, tracks_at + usize::from(be16(d, l.start + i * 2)?))).collect::<Option<_>>()?;
        let ins_at = be32(d, l.instruments)? as usize;
        let smp_at = be32(d, l.samples)? as usize;
        let instruments: Vec<Instrument> = (0..smp_at.saturating_sub(ins_at) / 16)
            .map(|i| {
                let b = d.get(l.start + ins_at + i * 16..l.start + ins_at + i * 16 + 12)?;
                Some(Instrument {
                    sample: b[0],
                    attack: b[1],
                    decay_sustain: b[2],
                    vibrato_delay: b[3],
                    release: b[4],
                    vibrato_speed: b[5],
                    vibrato_level: b[6],
                    arpeggio: b[7],
                    arp_speed: b[8],
                    hold: b[9],
                    key_wave_rate: b[10],
                    wave_level_speed: b[11],
                })
            })
            .collect::<Option<_>>()?;
        if instruments.is_empty() {
            return None;
        }
        // The samples' table runs up to the first position list.
        let smp = l.start + smp_at;
        let first = l.start + lists.iter().map(|(o, _)| *o).min()?;
        let mut mem: Vec<i8> = d.iter().map(|&x| x as i8).collect();
        // Room past the end, for a waveform read beyond it, then the replayer's empty sample.
        mem.resize(mem.len() + 0x2_0000, 0);
        let empty = mem.len();
        mem.resize(empty + 4, 0);
        let mut samples = Vec::new();
        for i in 0..first.saturating_sub(smp) / 24 {
            let b = d.get(smp + i * 24..smp + i * 24 + 24)?;
            let off = i32::from_be_bytes(b[0..4].try_into().ok()?);
            let name = String::from_utf8_lossy(&b[8..24]).trim_end_matches(['\0', ' ']).to_string();
            let s = if off < 0 {
                Sample { name, at: empty, len: 1, loop_len: 0 }
            } else {
                let len = usize::from(u16::from_be_bytes([b[4], b[5]]));
                Sample { name, at: smp + off as usize, len, loop_len: usize::from(u16::from_be_bytes([b[6], b[7]])) }
            };
            if s.at != empty && s.at + s.len * 2 > d.len() + 0x2_0000 {
                return None;
            }
            samples.push(s);
        }
        if samples.is_empty() {
            return None;
        }
        Some(Song { subsongs, lists, tracks, instruments, samples, mem, empty, file: Arc::new(d.to_vec()) })
    }

    fn list(&self, o: usize) -> &[Position] {
        self.lists.iter().find(|(k, _)| *k == o).map_or(&[], |(_, l)| l.as_slice())
    }

    /// A voice's position list in a sub-song.
    pub fn voice_list(&self, sub: usize, voice: usize) -> &[Position] {
        self.list(self.subsongs[sub].lists[CHANNEL_MAP[voice]])
    }

    /// A voice's tracks in order, each repeat its own: (track, transpose).
    pub fn steps(&self, sub: usize, voice: usize) -> Vec<(usize, u8)> {
        let mut out = Vec::new();
        for p in self.voice_list(sub, voice) {
            if p.track >= 0xFE {
                break;
            }
            for _ in 0..=p.repeat {
                out.push((usize::from(p.track).min(self.tracks.len() - 1), p.transpose));
            }
        }
        out
    }

    /// A track's rows.
    pub fn rows(&self, track: usize) -> Vec<Row> {
        let t = &self.tracks[track];
        let mut out = Vec::new();
        let mut i = 0;
        let at = |i: usize| t.get(i).copied().unwrap_or(0);
        while i < t.len() && at(i) != 0xFF {
            let b = at(i);
            i += 1;
            let mut r = Row::default();
            let note = |r: &mut Row, n: u8, i: &mut usize| {
                r.note = Some(n & 0x3F);
                let mut x = at(*i);
                *i += 1;
                if x & 0x80 != 0 {
                    x &= 0x7F;
                    r.portamento = at(*i);
                    *i += 1;
                }
                r.delay = x;
            };
            if b & 0x80 == 0 {
                note(&mut r, b, &mut i);
            } else if b & 0x40 == 0 {
                r.release = true;
                r.delay = b & 0x3F;
            } else {
                r.instrument = Some(b & 0x3F);
                let n = at(i);
                i += 1;
                note(&mut r, n, &mut i);
            }
            out.push(r);
        }
        out
    }
}

impl Tune for Song {
    fn format(&self) -> &'static str {
        "ma"
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

const SET_LOOP: u8 = 0x02;
const SYNTH: u8 = 0x04;
const PORTA: u8 = 0x20;
const RELEASE: u8 = 0x40;
const RETRIG: u8 = 0x80;

#[derive(Clone, Default)]
struct Voice {
    list: usize,
    position: usize,
    row: usize,
    repeat: i8,
    delay: i8,
    flag: u8,
    instrument: usize,
    note: u8,
    transpose: u8,
    volume: u8,
    quieter: bool,
    sustain: u8,
    arp_counter: u8,
    arp_step: u8,
    porta: u8,
    porta_add: i16,
    vib_delay: u8,
    vib_dir: u8,
    vib_counter: u8,
    vib_add: i16,
    wave_mod: u16,
    wave_dir: bool,
    wave_counter: u8,
    /// The sample registers the replayer keeps: start and words.
    at: usize,
    len: usize,
    /// Tracks started and rows into this one, for the screen (the next
    /// track's count, once the last row of this one is read).
    steps: usize,
    ticks: usize,
    next: Option<usize>,
    stopped: bool,
}

pub struct Player {
    song: Arc<Song>,
    paula: Paula,
    voices: [Voice; 4],
    speed: u8,
    counter: u8,
    shown: (usize, usize),
    done: bool,
}

impl Player {
    fn new(song: Arc<Song>, sub: usize) -> Player {
        let voices = std::array::from_fn(|c| {
            let o = song.subsongs[sub].lists[CHANNEL_MAP[c]];
            let list = song.lists.iter().position(|(k, _)| *k == o).unwrap_or(0);
            let first = song.lists[list].1.first().copied().unwrap_or_default();
            Voice { list, repeat: (first.repeat as i8) << 3, transpose: first.transpose, ..Default::default() }
        });
        let speed = song.subsongs[sub].speed;
        Player { paula: Paula::new(4, song.mem.clone()), voices, speed, counter: 1, shown: (0, 0), done: false, song }
    }

    /// The voice's next row, when its last is over: false when one was read.
    fn next_row(&mut self, c: usize) -> bool {
        let song = self.song.clone();
        let v = &mut self.voices[c];
        v.ticks += 1;
        v.delay -= 1;
        if v.delay >= 0 {
            return true;
        }
        let list = &song.lists[v.list].1;
        let Some(pos) = list.get(v.position).copied() else { return true };
        if pos.track == 0xFE {
            if !v.stopped {
                v.stopped = true;
                if c == 0 {
                    self.done = true;
                }
            }
            return true;
        }
        if let Some(n) = v.next.take() {
            v.steps = n;
            v.ticks = 1;
        }
        let t = &song.tracks[usize::from(pos.track).min(song.tracks.len() - 1)];
        let at = |i: usize| t.get(i).copied().unwrap_or(0xFF);
        let mut row = v.row;
        let mut b = at(row);
        row += 1;
        let ins_count = song.instruments.len();
        let note = |v: &mut Voice, b: &mut u8, row: &mut usize| {
            v.porta_add = 0;
            v.note = (*b & 0x3F).wrapping_add(v.transpose);
            *b = at(*row);
            *row += 1;
            if *b & 0x80 == 0 {
                v.porta = 0;
            } else {
                *b &= 0x7F;
                v.porta = at(*row);
                *row += 1;
            }
        };
        if b & 0x80 == 0 {
            if b & 0x40 == 0 {
                init_voice(v, &song.instruments[v.instrument]);
                retrigger(v);
                self.paula.stop(c);
            }
            note(v, &mut b, &mut row);
        } else if b & 0x40 == 0 {
            b &= 0x3F;
            v.flag |= RELEASE;
            v.sustain = 0;
        } else {
            v.instrument = usize::from(b & 0x3F).min(ins_count - 1);
            b = at(row);
            row += 1;
            if b & 0x40 == 0 {
                v.wave_mod = 0;
                v.wave_dir = false;
                v.wave_counter = 1;
                init_voice(v, &song.instruments[v.instrument]);
            }
            retrigger(v);
            self.paula.stop(c);
            note(v, &mut b, &mut row);
        }
        v.delay = b as i8;
        if at(row) == 0xFF {
            v.repeat -= 8;
            if v.repeat < 0 {
                v.position += 1;
                v.next = Some(v.steps + 1);
                let mut p = list.get(v.position).copied().unwrap_or(Position { track: 0xFF, ..Default::default() });
                if p.track == 0xFF {
                    v.position = 0;
                    v.next = Some(0);
                    p = list[0];
                    if c == 0 {
                        self.done = true;
                    }
                }
                v.transpose = p.transpose;
                v.repeat = (p.repeat as i8) << 3;
            } else {
                v.next = Some(v.steps + 1);
            }
            row = 0;
        }
        v.row = row;
        false
    }

    fn voice(&mut self, c: usize) {
        let song = self.song.clone();
        let ins = song.instruments[self.voices[c].instrument];
        self.setup_sample(c, &ins);
        let v = &mut self.voices[c];
        // The arpeggio: the note, then its two nibbles above it.
        let mut arp = ins.arpeggio;
        let mut step = v.arp_step.wrapping_sub(2) as i8;
        if arp != 0 {
            arp = match step {
                0 => arp >> 4,
                s if s < 0 => 0,
                _ => arp & 0x0F,
            };
        }
        if ins.arp_speed != 0 {
            v.arp_counter = v.arp_counter.wrapping_add(0x10);
            if v.arp_counter > ins.arp_speed {
                v.arp_counter = 0;
                let mut spd = ins.arp_speed & 3;
                if spd == 0 {
                    v.quieter = !v.quieter;
                    spd = 1;
                }
                step += 3;
                if step >= 4 {
                    step = spd as i8;
                }
                v.arp_step = step as u8;
            }
        }
        let mut n = v.note.wrapping_add(arp);
        if v.flag & SYNTH == 0 {
            n = n.wrapping_add(ins.key_wave_rate);
        }
        let mut period = PERIODS[usize::from(n).min(47)];
        // Portamento, every other tick at an odd rate.
        if v.porta != 0 {
            let was = v.flag & PORTA != 0;
            v.flag ^= PORTA;
            if !was || v.porta & 1 == 0 {
                period = ((i32::from(period) * 2 + i32::from(v.porta_add)) >> 1) as u16;
                if v.porta & 0x80 != 0 {
                    v.porta_add = v.porta_add.wrapping_add(((-i16::from(v.porta as i8)) + 1) >> 1);
                } else {
                    v.porta_add = v.porta_add.wrapping_sub((i16::from(v.porta) + 1) >> 1);
                }
            }
        }
        // Vibrato: a triangle, after its delay.
        if v.vib_delay == 0 {
            if v.porta == 0 && ins.vibrato_level != 0 {
                let mut level = i16::from(ins.vibrato_level);
                let dir = v.vib_dir & 3;
                if dir == 1 || dir == 2 {
                    level = -level;
                }
                v.vib_add = v.vib_add.wrapping_add(level);
                v.vib_counter = v.vib_counter.wrapping_add(1);
                if v.vib_counter == ins.vibrato_speed {
                    v.vib_counter = 0;
                    v.vib_dir = v.vib_dir.wrapping_add(1);
                }
            }
        } else {
            v.vib_delay -= 1;
        }
        // The envelope: up by the attack to the sustain, held, then released.
        if v.flag & RELEASE != 0 {
            if v.sustain == 0 {
                v.volume = v.volume.saturating_sub(ins.release);
            } else {
                v.sustain -= 1;
            }
        } else {
            let mut vol = i32::from(v.volume) + i32::from(ins.attack);
            if vol > i32::from(ins.decay_sustain | 0x0F) {
                v.flag |= RELEASE;
                vol = i32::from(ins.decay_sustain) << 4;
            }
            v.volume = vol as u8;
        }
        let mut volume = u32::from(v.volume) * 64 / 256;
        if v.vib_delay == 0 && v.quieter {
            volume /= 4;
        }
        // The vibrato's offset, kept through the portamento and its delay.
        let period = ((i32::from(period) * 2 + i32::from(v.vib_add)) >> 1) as u16;
        self.paula.set_volume(c, volume);
        self.paula.set_period(c, u32::from(period.max(127)));
    }

    /// The sample registers: a retriggered voice starts its sound, the
    /// tick after its loop (or a sweeping waveform's next start) follows.
    fn setup_sample(&mut self, c: usize, ins: &Instrument) {
        let song = &self.song;
        let s = &song.samples[usize::from(ins.sample).min(song.samples.len() - 1)];
        let v = &mut self.voices[c];
        if v.flag & SET_LOOP != 0 {
            v.flag &= !SET_LOOP;
            if s.loop_len != 0 {
                v.len = s.loop_len;
                v.at = if s.len <= 128 { s.at + wave_offset(v, ins, s) } else { s.at + (s.len - s.loop_len.min(s.len)) * 2 };
            } else {
                v.at = song.empty;
                v.len = 2;
                v.flag &= !SYNTH;
            }
            self.paula.set_loop(c, v.at, v.len * 2);
        } else if v.flag & RETRIG != 0 {
            v.flag &= RELEASE;
            v.flag |= SET_LOOP;
            let (mut at, mut len) = (s.at, s.len);
            if s.len <= 128 && s.at != song.empty {
                v.flag |= SYNTH;
                at += usize::from(ins.key_wave_rate);
                len = s.loop_len.max(1);
            }
            v.at = at;
            v.len = len;
            self.paula.play(c, at, len * 2);
            if at != song.empty {
                self.paula.set_loop(c, at, len * 2);
            }
        } else if v.flag & SYNTH != 0 {
            v.at = s.at + wave_offset(v, ins, s);
            self.paula.set_loop(c, v.at, v.len * 2);
        }
    }
}

fn init_voice(v: &mut Voice, ins: &Instrument) {
    v.quieter = false;
    v.arp_step = 0;
    v.volume = 0;
    v.vib_dir = 0;
    v.vib_counter = 0;
    v.vib_add = 0;
    v.flag = 0;
    v.vib_delay = ins.vibrato_delay;
    v.arp_counter = ins.arp_speed;
    v.sustain = ins.hold;
}

fn retrigger(v: &mut Voice) {
    v.flag &= RELEASE;
    v.flag |= RETRIG;
}

/// A sweeping waveform's next start: its key rate on, back and forth by
/// its level each tick, turning every so many ticks.
fn wave_offset(v: &mut Voice, ins: &Instrument, s: &Sample) -> usize {
    let mut level = (ins.wave_level_speed >> 4) as i8;
    let speed = ins.wave_level_speed & 0x0F;
    if speed != 0 {
        v.wave_counter = v.wave_counter.wrapping_add(1);
        if speed * 2 < v.wave_counter {
            v.wave_counter = 0;
            v.wave_dir = !v.wave_dir;
        }
        if v.wave_dir {
            level = -level;
        }
    }
    v.wave_mod = v.wave_mod.wrapping_add(level as u16);
    (usize::from(v.wave_mod) / 4 + usize::from(ins.key_wave_rate)) & s.len.wrapping_sub(1)
}

impl Driver for Player {
    fn channels(&self) -> usize {
        4
    }

    fn tick(&mut self) {
        self.counter = self.counter.wrapping_sub(1);
        let row = self.counter == 0;
        if row {
            self.counter = self.speed;
        }
        for c in 0..4 {
            if !row || self.next_row(c) {
                self.voice(c);
            }
        }
        let v = &self.voices[0];
        self.shown = (v.steps, v.ticks.saturating_sub(1));
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
        (u32::from(self.speed).max(1), 125)
    }

    fn take_done(&mut self) -> bool {
        std::mem::take(&mut self.done)
    }
}
