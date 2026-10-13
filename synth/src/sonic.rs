// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! Sonic Arranger ("SOARV1.0", and its "final" form: the replayer with
//! the song after it): sub-songs of positions, each channel a run of the
//! song's rows (transposed in note and instrument); instruments are
//! samples or 128-point waveforms remade as they play by one of 17
//! effects (negators, rotations, morphs, filters, noise, an FM drum),
//! under ADSR and AMF (pitch) tables, vibrato, portamento, slides and
//! three arpeggio tables. The song ticks at its own tempo.

use std::sync::Arc;

use crate::paula::Paula;
use crate::replay::{Bufs, Driver, Tune};

/// Sonic Arranger's periods: ProTracker's octave 1 sixteen, eight, four
/// and two times, itself, its octaves 2 and 3, then 3 halved and halved
/// again (rounded down).
pub fn period(i: usize) -> u16 {
    const O1: [u16; 12] = [856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453];
    const O2: [u16; 12] = [428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226];
    const O3: [u16; 12] = [214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113];
    if i == 0 || i > 108 {
        return 0;
    }
    let (oct, k) = ((i - 1) / 12, (i - 1) % 12);
    match oct {
        0 => O1[k] * 16,
        1 => O1[k] * 8,
        2 => O1[k] * 4,
        3 => O1[k] * 2,
        4 => O1[k],
        5 => O2[k],
        6 => O3[k],
        7 => O3[k] / 2,
        _ => O3[k] / 2 / 2,
    }
}

/// The replayer's vibrato: a sine of 256 steps as it has it (127 at the
/// top, -126 at the bottom; not a sine to the point, so kept as written).
pub(crate) const VIBRATO: [i8; 256] = [
    0, 3, 6, 9, 12, 16, 19, 22, 25, 28, 31, 34, 37, 40, 43, 46, 49, 52, 54, 57, 60, 63, 66, 68, 71, 73, 76, 78, 81, 83, 86, 88, 90, 92, 94, 96, 98, 100, 102, 104, 106, 108, 109, 111, 112, 114, 115, 116, 118, 119, 120, 121, 122, 123, 123, 124, 125, 125, 126,
    126, 126, 127, 127, 127, 127, 127, 127, 127, 126, 126, 125, 125, 124, 124, 123, 122, 121, 120, 119, 118, 117, 116, 114, 113, 112, 110, 108, 107, 105, 103, 101, 99, 97, 95, 93, 91, 89, 87, 84, 82, 80, 77, 75, 72, 69, 67, 64, 61, 59, 56, 53, 50, 47, 44, 41, 39, 36, 32,
    29, 26, 23, 20, 17, 14, 11, 8, 5, 2, -1, -4, -7, -10, -14, -17, -20, -23, -26, -29, -32, -35, -38, -41, -44, -47, -50, -53, -55, -58, -61, -64, -66, -69, -72, -74, -77, -79, -82, -84, -86, -88, -91, -93, -95, -97, -99, -101, -103, -104, -106, -108, -109, -111, -112,
    -114, -115, -116, -118, -119, -120, -121, -122, -122, -123, -124, -124, -125, -125, -126, -126, -126, -126, -126, -126, -126, -126, -126, -126, -125, -125, -124, -124, -123, -122, -121, -120, -119, -118, -117, -116, -115, -113, -112, -110, -109, -107, -105, -104, -102, -100,
    -98, -96, -94, -92, -90, -87, -85, -83, -80, -78, -75, -73, -70, -68, -65, -62, -60, -57, -54, -51, -48, -45, -42, -39, -37, -34, -30, -27, -24, -21, -18, -15, -12, -9, -6, -3, 0,
];

#[derive(Clone, Copy, Debug, Default)]
pub struct SubSong {
    pub speed: u16,
    pub rows: u16,
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

#[derive(Clone, Copy, Debug, Default)]
pub struct Arp {
    pub length: u8,
    pub repeat: u8,
    pub values: [i8; 14],
}

#[derive(Clone, Debug, Default)]
pub struct Instrument {
    pub name: String,
    pub synth: bool,
    pub waveform: u16,
    pub length: u16,
    pub repeat: u16,
    pub volume: u16,
    pub finetune: i8,
    pub portamento: u16,
    pub vib_delay: u16,
    pub vib_speed: u16,
    pub vib_level: u16,
    pub amf: u16,
    pub amf_delay: u16,
    pub amf_length: u16,
    pub amf_repeat: u16,
    pub adsr: u16,
    pub adsr_delay: u16,
    pub adsr_length: u16,
    pub adsr_repeat: u16,
    pub sustain_point: u16,
    pub sustain_delay: u16,
    pub arg1: u16,
    pub effect: u16,
    pub arg2: u16,
    pub arg3: u16,
    pub effect_delay: u16,
    pub arps: [Arp; 3],
}

#[derive(Clone, Debug)]
pub struct Song {
    pub subsongs: Vec<SubSong>,
    /// Each position's four (first row, sound transpose, note transpose).
    pub positions: Vec<[(u16, i8, i8); 4]>,
    pub lines: Vec<Line>,
    pub instruments: Vec<Instrument>,
    /// Each sample's (start in memory, length), empty when it has none.
    pub samples: Vec<(usize, usize)>,
    /// Where each 128-point waveform is in memory.
    pub waveforms: Vec<usize>,
    pub adsr: Vec<[u8; 128]>,
    pub amf: Vec<[i8; 128]>,
    pub mem: Vec<i8>,
    file: Arc<Vec<u8>>,
}

fn be16(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(i..i + 2)?.try_into().ok()?))
}

fn be32(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?))
}

/// Where a "final" file's song is (after its replayer's seven jumps and
/// the LEA its start routine points with), or 0 for one with no replayer.
fn final_offset(d: &[u8]) -> Option<usize> {
    let check = be32(d, 0)?;
    if check & 0xFFFF_0000 == 0x4EFA_0000 {
        if d.len() < 0x1630 {
            return None;
        }
        let init = ((check & 0xFFFF) as i16 as i64 + 2) as usize;
        if (1..7).any(|i| be16(d, i * 4) != Some(0x4EFA)) {
            return None;
        }
        if be32(d, init)? != 0x48E7_FFFE || be16(d, init + 4)? != 0x41FA {
            return None;
        }
        let off = init + 6 + usize::from(be16(d, init + 6)?);
        return (off < d.len()).then_some(off);
    }
    // No replayer: eight offsets that climb as a song's blocks do.
    if check & 0xFFFF_0000 != 0 || check == 0 || check & 0xF8 != check {
        return None;
    }
    let o: Vec<u32> = (0..6).map(|k| be32(d, 4 + k * 4)).collect::<Option<_>>()?;
    let ok = o.iter().all(|&x| x != 0 && x & 0xFFFF_0000 == 0)
        && (o[0] - check).is_multiple_of(12)
        && (o[0] - check) / 12 <= 32
        && o.windows(2).all(|w| w[0] <= w[1])
        && ((o[1] - o[0]) & 0xFFF0) == o[1] - o[0];
    ok.then_some(0)
}

pub fn probe(d: &[u8]) -> bool {
    d.len() >= 16 && (d.starts_with(b"SOARV1.0") || final_offset(d).is_some())
}

fn read_subsongs(d: &[u8], at: usize, n: usize) -> Option<Vec<SubSong>> {
    let mut out: Vec<SubSong> = Vec::new();
    for k in 0..n {
        let o = at + k * 12;
        let s = SubSong { speed: be16(d, o)?, rows: be16(d, o + 2)?, first: be16(d, o + 4)?, last: be16(d, o + 6)?, restart: be16(d, o + 8)?, tempo: be16(d, o + 10)? };
        if s.last == 0xFFFF || s.restart == 0xFFFF || out.iter().any(|x| x.first == s.first && x.last == s.last) {
            continue;
        }
        out.push(s);
    }
    Some(out)
}

fn read_instrument(b: &[u8]) -> Instrument {
    let h = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
    let mut arps = [Arp::default(); 3];
    for (j, a) in arps.iter_mut().enumerate() {
        let o = 74 + j * 16;
        a.length = b[o];
        a.repeat = b[o + 1];
        for (k, v) in a.values.iter_mut().enumerate() {
            *v = b[o + 2 + k] as i8;
        }
    }
    Instrument {
        name: b[122..152].iter().take_while(|&&c| c != 0).map(|&c| if (32..127).contains(&c) { c as char } else { ' ' }).collect::<String>().trim().to_string(),
        synth: h(0) != 0,
        waveform: h(2),
        length: h(4),
        repeat: h(6),
        volume: h(16) & 0xFF,
        finetune: h(18) as i8,
        portamento: h(20),
        vib_delay: h(22),
        vib_speed: h(24),
        vib_level: h(26),
        amf: h(28),
        amf_delay: h(30),
        amf_length: h(32),
        amf_repeat: h(34),
        adsr: h(36),
        adsr_delay: h(38),
        adsr_length: h(40),
        adsr_repeat: h(42),
        sustain_point: h(44),
        sustain_delay: h(46),
        arg1: h(64),
        effect: h(66),
        arg2: h(68),
        arg3: h(70),
        effect_delay: h(72),
        arps,
    }
}

/// The blocks a song is made of, wherever they are.
struct Blocks {
    subsongs: Vec<SubSong>,
    positions: (usize, usize),
    lines: (usize, usize),
    instruments: (usize, usize),
    /// Each sample's length and where its points are.
    samples: Vec<(usize, usize)>,
    waves: (usize, usize),
    adsr: (usize, usize),
    amf: (usize, usize),
}

/// A block of the "SOARV1.0" file: its mark, its count, its elements.
fn block(d: &[u8], at: &mut usize, mark: &[u8], size: usize) -> Option<(usize, usize)> {
    if d.get(*at..*at + 4)? != mark {
        return None;
    }
    let n = be32(d, *at + 4)? as usize;
    let start = *at + 8;
    *at = start + n * size;
    Some((start, n))
}

fn blocks_soar(d: &[u8]) -> Option<Blocks> {
    let mut at = 8;
    let s = block(d, &mut at, b"STBL", 12)?;
    let subsongs = read_subsongs(d, s.0, s.1)?;
    let positions = block(d, &mut at, b"OVTB", 16)?;
    let lines = block(d, &mut at, b"NTBL", 4)?;
    let instruments = block(d, &mut at, b"INST", 152)?;
    let sd = block(d, &mut at, b"SD8B", 0)?;
    // The samples: their lengths and loops in words and their names, then their lengths in bytes and points.
    let n = sd.1;
    let mut p = sd.0 + n * 8 + n * 30;
    let lens: Vec<usize> = (0..n).map(|k| be32(d, p + k * 4).map(|v| v as usize)).collect::<Option<_>>()?;
    p += n * 4;
    let mut samples = Vec::with_capacity(n);
    for &l in &lens {
        samples.push((l, p));
        p += l;
    }
    at = p;
    let waves = block(d, &mut at, b"SYWT", 128)?;
    let adsr = block(d, &mut at, b"SYAR", 128)?;
    let amf = block(d, &mut at, b"SYAF", 128)?;
    Some(Blocks { subsongs, positions, lines, instruments, samples, waves, adsr, amf })
}

fn blocks_final(d: &[u8], base: usize) -> Option<Blocks> {
    let o: Vec<usize> = (0..8).map(|k| be32(d, base + k * 4).map(|v| v as usize)).collect::<Option<_>>()?;
    if o.windows(2).take(7).any(|w| w[1] < w[0]) {
        return None;
    }
    let span = |a: usize, b: usize, size: usize| (base + o[a], (o[b] - o[a]) / size);
    let s = span(0, 1, 12);
    let subsongs = read_subsongs(d, s.0, s.1)?;
    let sp = base + o[7];
    let n = be32(d, sp)? as usize;
    let lens: Vec<usize> = (0..n).map(|k| be32(d, sp + 4 + k * 4).map(|v| v as usize)).collect::<Option<_>>()?;
    let mut p = sp + 4 + n * 4;
    let mut samples = Vec::with_capacity(n);
    for &l in &lens {
        samples.push((l, p));
        p += l;
    }
    Some(Blocks { subsongs, positions: span(1, 2, 16), lines: span(2, 3, 4), instruments: span(3, 4, 152), samples, waves: span(4, 5, 128), adsr: span(5, 6, 128), amf: span(6, 7, 128) })
}

impl Song {
    pub fn parse(d: &[u8]) -> Option<Song> {
        let b = if d.starts_with(b"SOARV1.0") { blocks_soar(d)? } else { blocks_final(d, final_offset(d)?)? };
        if b.subsongs.is_empty() || b.positions.1 == 0 || b.lines.1 == 0 || b.instruments.1 == 0 {
            return None;
        }
        let positions = (0..b.positions.1)
            .map(|k| {
                let o = b.positions.0 + k * 16;
                let mut p = [(0u16, 0i8, 0i8); 4];
                for (c, x) in p.iter_mut().enumerate() {
                    *x = (be16(d, o + c * 4)?, *d.get(o + c * 4 + 2)? as i8, *d.get(o + c * 4 + 3)? as i8);
                }
                Some(p)
            })
            .collect::<Option<Vec<_>>>()?;
        let lines = (0..b.lines.1)
            .map(|k| {
                let x = d.get(b.lines.0 + k * 4..b.lines.0 + k * 4 + 4)?;
                Some(Line { note: x[0], instrument: x[1], no_sound_transpose: x[2] & 0x80 != 0, no_note_transpose: x[2] & 0x40 != 0, arpeggio: (x[2] & 0x30) >> 4, effect: x[2] & 0x0F, arg: x[3] })
            })
            .collect::<Option<Vec<_>>>()?;
        let instruments = (0..b.instruments.1).map(|k| d.get(b.instruments.0 + k * 152..b.instruments.0 + k * 152 + 152).map(read_instrument)).collect::<Option<Vec<_>>>()?;
        let mut mem: Vec<i8> = Vec::new();
        let mut samples = Vec::with_capacity(b.samples.len());
        for &(l, p) in &b.samples {
            let at = mem.len();
            mem.extend(d.get(p..(p + l).min(d.len())).unwrap_or_default().iter().map(|&x| x as i8));
            mem.resize(at + l, 0);
            samples.push((at, l));
        }
        let mut waveforms = Vec::with_capacity(b.waves.1.max(1));
        for k in 0..b.waves.1 {
            waveforms.push(mem.len());
            mem.extend(d.get(b.waves.0 + k * 128..b.waves.0 + k * 128 + 128)?.iter().map(|&x| x as i8));
        }
        if waveforms.is_empty() {
            waveforms.push(mem.len());
            mem.resize(mem.len() + 128, 0);
        }
        let adsr = (0..b.adsr.1).map(|k| d.get(b.adsr.0 + k * 128..b.adsr.0 + k * 128 + 128).and_then(|x| x.try_into().ok())).collect::<Option<Vec<[u8; 128]>>>()?;
        let amf = (0..b.amf.1)
            .map(|k| {
                let x = d.get(b.amf.0 + k * 128..b.amf.0 + k * 128 + 128)?;
                let mut a = [0i8; 128];
                for (o, &v) in a.iter_mut().zip(x) {
                    *o = v as i8;
                }
                Some(a)
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Song { subsongs: b.subsongs, positions, lines, instruments, samples, waveforms, adsr, amf, mem, file: Arc::new(d.to_vec()) })
    }

    pub fn line(&self, position: usize, channel: usize, row: usize) -> Line {
        let start = self.positions.get(position).map_or(0, |p| usize::from(p[channel].0));
        self.lines.get(start + row).copied().unwrap_or_default()
    }
}

impl Tune for Song {
    fn format(&self) -> &'static str {
        "sa"
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
    note: u16,
    prev_note: u16,
    instrument: Option<usize>,
    volume: u16,
    vol_slide: i8,
    vib_pos: usize,
    vib_delay: u16,
    vib_speed: u16,
    vib_level: u16,
    porta_speed: u16,
    porta_period: u16,
    arp_pos: usize,
    slide_speed: i8,
    slide: i16,
    adsr_pos: usize,
    adsr_delay: u16,
    sustain_delay: u16,
    amf_pos: usize,
    amf_delay: u16,
    fx_pos: u16,
    fx_wave_pos: u16,
    fx_delay: u16,
    flag: u8,
}

pub struct Player {
    song: Arc<Song>,
    paula: Paula,
    sub: SubSong,
    voices: [Voice; 4],
    /// Where each channel's own 128-point waveform is in memory.
    buffers: [usize; 4],
    master: u16,
    counter: u16,
    speed: u16,
    position: i32,
    row: u16,
    rows: u16,
    random: u32,
    visited: Vec<bool>,
    shown: (usize, usize),
    done: bool,
}

impl Player {
    fn new(song: Arc<Song>, k: usize) -> Player {
        let mut paula = Paula::new(4, song.mem.clone());
        let buffers = std::array::from_fn(|_| paula.alloc(128));
        let sub = song.subsongs[k];
        let voices = std::array::from_fn(|_| Voice { flag: 1, ..Default::default() });
        Player { paula, buffers, sub, voices, master: 64, counter: sub.speed, speed: sub.speed, position: i32::from(sub.first) - 1, row: sub.rows, rows: sub.rows, random: 0x1234_5679, visited: vec![false; song.positions.len()], shown: (0, 0), done: false, song }
    }

    fn next_position(&mut self) {
        self.position += 1;
        if self.position > i32::from(self.sub.last) || self.position as usize >= self.song.positions.len() {
            self.position = i32::from(self.sub.restart);
        }
        let p = (self.position.max(0) as usize).min(self.song.positions.len() - 1);
        if self.visited[p] {
            self.done = true;
            self.visited.iter_mut().for_each(|x| *x = false);
        }
        self.visited[p] = true;
        let pos = self.song.positions[p];
        for (v, &(start, st, nt)) in self.voices.iter_mut().zip(&pos) {
            v.start = start;
            v.sound_tr = st;
            v.note_tr = nt;
        }
    }

    fn next_row(&mut self) {
        self.row += 1;
        if self.row >= self.rows {
            self.row = 0;
            self.next_position();
        }
        self.shown = ((self.position.max(0) as usize).saturating_sub(usize::from(self.sub.first)), usize::from(self.row));
        let song = self.song.clone();
        for c in 0..4 {
            let at = usize::from(self.voices[c].start) + usize::from(self.row);
            self.voices[c].line = song.lines.get(at).copied().unwrap_or_default();
            self.play_voice(c);
        }
    }

    fn quiet(&mut self, c: usize) {
        self.paula.stop(c);
        let v = &mut self.voices[c];
        v.volume = 0;
        v.instrument = None;
    }

    fn set_synth(v: &mut Voice, ins: &Instrument) {
        v.slide = i16::from(ins.finetune);
        v.adsr_delay = ins.adsr_delay;
        v.adsr_pos = 0;
        v.amf_delay = ins.amf_delay;
        v.amf_pos = 0;
        v.fx_pos = ins.arg2;
        v.fx_wave_pos = 0;
        v.fx_delay = ins.effect_delay;
        v.arp_pos = 0;
        v.flag = 0;
    }

    fn set_instrument(&mut self, c: usize, n: u16) -> Option<usize> {
        let k = usize::from(n & 0xFF).checked_sub(1)?;
        let ins = self.song.instruments.get(k)?;
        let v = &mut self.voices[c];
        v.instrument = Some(k);
        v.volume = ins.volume;
        v.porta_speed = ins.portamento;
        v.porta_period = 0;
        v.vib_pos = 0;
        v.vib_delay = ins.vib_delay;
        v.vib_speed = ins.vib_speed;
        v.vib_level = ins.vib_level;
        Self::set_synth(v, ins);
        Some(k)
    }

    fn play_voice(&mut self, c: usize) {
        let song = self.song.clone();
        let line = self.voices[c].line;
        let mut note = u16::from(line.note);
        let mut n = i16::from(line.instrument);
        if note == 0 {
            if n != 0 {
                self.set_instrument(c, n as u16);
            }
        } else if note != 0x80 {
            if note == 0x7F {
                self.quiet(c);
            } else {
                let v = &mut self.voices[c];
                if !line.no_note_transpose {
                    note = note.wrapping_add(v.note_tr as i16 as u16);
                }
                if n != 0 && !line.no_sound_transpose {
                    n += i16::from(v.sound_tr);
                }
                v.prev_note = v.note;
                v.note = note;
                if v.prev_note == 0 {
                    v.prev_note = note;
                }
                let k = if n < 0 {
                    None
                } else if n == 0 {
                    match v.instrument {
                        Some(k) => {
                            Self::set_synth(v, &song.instruments[k]);
                            Some(k)
                        }
                        None => None,
                    }
                } else {
                    self.set_instrument(c, n as u16)
                };
                match k {
                    None => {
                        self.voices[c].flag |= 1;
                        self.quiet(c);
                        return self.set_effects(c);
                    }
                    Some(k) => {
                        let ins = &song.instruments[k];
                        if ins.synth {
                            let len = usize::from(ins.length) * 2;
                            let buf = self.buffers[c];
                            let src = song.waveforms.get(usize::from(ins.waveform)).copied().unwrap_or(song.waveforms[0]);
                            let len = len.min(128);
                            self.paula.play(c, buf, len);
                            self.paula.set_loop(c, buf, len);
                            let w: Vec<i8> = self.paula.mem[src..src + 128].to_vec();
                            self.paula.mem[buf..buf + 128].copy_from_slice(&w);
                        } else {
                            match song.samples.get(usize::from(ins.waveform)).filter(|s| s.1 > 0) {
                                None => {
                                    self.voices[c].flag |= 1;
                                    self.quiet(c);
                                }
                                Some(&(at, slen)) => {
                                    let mut play = u32::from(ins.length);
                                    let (mut ls, mut ll) = (0u32, 0u32);
                                    if ins.repeat == 0 {
                                        ll = u32::from(ins.length) * 2;
                                    } else if ins.repeat != 1 {
                                        play += u32::from(ins.repeat);
                                        ls = u32::from(ins.length) * 2;
                                        ll = u32::from(ins.repeat) * 2;
                                    }
                                    let play = (play * 2).min(slen as u32);
                                    if ls + ll > play {
                                        ll = play.saturating_sub(ls);
                                    }
                                    if play > 0 {
                                        self.paula.play(c, at, play as usize);
                                        if ll != 0 {
                                            self.paula.set_loop(c, at + ls as usize, ll as usize);
                                        }
                                    } else {
                                        self.voices[c].flag |= 1;
                                        self.quiet(c);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        self.set_effects(c);
    }

    fn set_effects(&mut self, c: usize) {
        let v = &mut self.voices[c];
        v.slide_speed = 0;
        v.vol_slide = 0;
        let a = v.line.arg;
        match v.line.effect {
            0x1 => v.slide_speed = a as i8,
            0x2 => v.adsr_pos = usize::from(a),
            0x4 => {
                v.vib_delay = 0;
                v.vib_speed = u16::from((a & 0xF0) >> 3);
                v.vib_level = (160 - (i32::from(a & 0x0F) << 4)) as u16;
            }
            0x6 => self.master = if a == 64 { 64 } else { u16::from(a & 0x3F) },
            0x7 => v.porta_speed = u16::from(a),
            0x8 => v.porta_speed = 0,
            0x9 => {
                if a <= 64 {
                    self.rows = u16::from(a);
                }
            }
            0xA => v.vol_slide = a as i8,
            0xB => {
                self.position = i32::from(a) - 1;
                self.row = self.rows;
            }
            0xC => v.volume = u16::from(a.min(64)),
            0xD => self.row = self.rows,
            0xF => {
                if (1..=16).contains(&a) {
                    self.speed = u16::from(a);
                }
            }
            _ => {}
        }
    }

    fn update(&mut self, c: usize) {
        let song = self.song.clone();
        let (Some(k), false) = (self.voices[c].instrument, self.voices[c].flag & 1 != 0) else {
            self.paula.stop(c);
            return;
        };
        let ins = &song.instruments[k];
        let v = &mut self.voices[c];
        // The arpeggio: one of the instrument's tables, or the effect's.
        let (mut note, mut prev) = (v.note, v.prev_note);
        if v.line.arpeggio != 0 {
            let arp = ins.arps[usize::from(v.line.arpeggio - 1)];
            let a = arp.values[v.arp_pos.min(13)];
            note = note.wrapping_add(a as i16 as u16);
            prev = prev.wrapping_add(a as i16 as u16);
            v.arp_pos += 1;
            let most = usize::from(arp.length + arp.repeat).min(13);
            if v.arp_pos > most {
                v.arp_pos = usize::from(arp.length);
            }
        } else if v.line.effect == 0 && v.line.arg != 0 {
            let a = match self.counter % 3 {
                1 => u16::from(v.line.arg >> 4),
                2 => u16::from(v.line.arg & 0x0F),
                _ => 0,
            };
            note = note.wrapping_add(a);
            prev = prev.wrapping_add(a);
        }
        if note >= 109 {
            note = 0;
        }
        if prev >= 109 {
            prev = 0;
        }
        let mut p = period(usize::from(note));
        let prev_p = period(usize::from(prev));
        // Portamento from the last note.
        if v.porta_speed != 0 {
            if v.porta_period == 0 {
                v.porta_period = prev_p;
            }
            let diff = (i32::from(p) - i32::from(v.porta_period)).abs() - i32::from(v.porta_speed);
            if diff < 0 {
                v.porta_speed = 0;
            } else {
                let np = if v.porta_period >= p { v.porta_period.wrapping_sub(v.porta_speed) } else { v.porta_period.wrapping_add(v.porta_speed) };
                v.porta_period = np;
                p = np;
            }
        }
        // Vibrato.
        if v.vib_delay != 255 {
            if v.vib_delay == 0 {
                let x = VIBRATO[v.vib_pos & 0xFF];
                if x != 0 && v.vib_level != 0 {
                    p = p.wrapping_add(((i32::from(x) * 4) / i32::from(v.vib_level)) as u16);
                }
                v.vib_pos = (v.vib_pos + usize::from(v.vib_speed)) & 0xFF;
            } else {
                v.vib_delay -= 1;
            }
        }
        // AMF: the period from a table.
        if ins.amf_length + ins.amf_repeat != 0 {
            let x = song.amf.get(usize::from(ins.amf)).map_or(0, |t| t[v.amf_pos.min(127)]);
            p = p.wrapping_sub(x as i16 as u16);
            v.amf_delay = v.amf_delay.wrapping_sub(1);
            if v.amf_delay == 0 {
                v.amf_delay = ins.amf_delay;
                v.amf_pos += 1;
                if v.amf_pos >= usize::from(ins.amf_length + ins.amf_repeat) {
                    v.amf_pos = usize::from(ins.amf_length);
                    if ins.amf_repeat == 0 {
                        v.amf_pos = v.amf_pos.saturating_sub(1);
                    }
                }
            }
        }
        // The slide.
        p = p.wrapping_sub(v.slide as u16);
        if p < 113 {
            p = 113;
        }
        if self.counter != 0 {
            v.slide = v.slide.wrapping_add(i16::from(v.slide_speed));
        }
        self.paula.set_period(c, u32::from(p));
        if ins.synth {
            self.synth_effect(c, k);
        }
        // ADSR: the volume from a table, held at the sustain point while the note's held.
        let v = &mut self.voices[c];
        let vol: u16;
        if ins.adsr_length + ins.adsr_repeat == 0 {
            vol = v.volume * self.master / 64;
            self.paula.set_volume(c, u32::from(vol));
        } else {
            let a = song.adsr.get(usize::from(ins.adsr)).map_or(0, |t| t[v.adsr_pos.min(127)]);
            vol = (u32::from(self.master) * u32::from(v.volume) * u32::from(a) / 4096).min(64) as u16;
            self.paula.set_volume(c, u32::from(vol));
            let v = &mut self.voices[c];
            let mut hold = false;
            if v.line.note == 0x80 && v.adsr_pos >= usize::from(ins.sustain_point) {
                if ins.sustain_delay == 0 {
                    hold = true;
                } else if v.sustain_delay != 0 {
                    v.sustain_delay -= 1;
                    hold = true;
                } else {
                    v.sustain_delay = ins.sustain_delay;
                }
            }
            if !hold {
                v.adsr_delay = v.adsr_delay.wrapping_sub(1);
                if v.adsr_delay == 0 {
                    v.adsr_delay = ins.adsr_delay;
                    v.adsr_pos += 1;
                    if v.adsr_pos >= usize::from(ins.adsr_length + ins.adsr_repeat) {
                        v.adsr_pos = usize::from(ins.adsr_length);
                        if ins.adsr_repeat == 0 {
                            v.adsr_pos = v.adsr_pos.saturating_sub(1);
                            if vol == 0 {
                                v.flag |= 1;
                            }
                        }
                    }
                }
            }
        }
        let v = &mut self.voices[c];
        v.volume = (i32::from(v.volume) + i32::from(v.vol_slide)).clamp(0, 64) as u16;
    }

    /// The instrument's effect on the channel's own waveform.
    fn synth_effect(&mut self, c: usize, k: usize) {
        let song = self.song.clone();
        let ins = &song.instruments[k];
        self.voices[c].fx_delay = self.voices[c].fx_delay.wrapping_sub(1);
        if self.voices[c].fx_delay != 0 {
            return;
        }
        self.voices[c].fx_delay = ins.effect_delay;
        let buf = self.buffers[c];
        let wave = |n: u16| song.waveforms.get(usize::from(n)).copied().unwrap_or(song.waveforms[0]);
        let (start, stop) = (usize::from(ins.arg2).min(127), usize::from(ins.arg3).min(127));
        let span = |m: &mut Vec<i8>, f: &mut dyn FnMut(&mut Vec<i8>, usize)| {
            let mut i = start;
            loop {
                f(m, buf + i.min(127));
                if i >= stop {
                    break;
                }
                i += 1;
            }
        };
        let step = |v: &mut Voice| {
            v.fx_pos += 1;
            if v.fx_pos >= ins.arg3 {
                v.fx_pos = ins.arg2;
            }
        };
        let towards = |m: &mut Vec<i8>, src: usize, flag: &mut u8| {
            let mut changed = false;
            let mut i = start;
            loop {
                let (d, s) = (m[buf + i], m[src + i]);
                if d != s {
                    changed = true;
                    m[buf + i] = if d < s { d + 1 } else { d - 1 };
                }
                if i >= stop {
                    break;
                }
                i += 1;
            }
            if !changed {
                *flag |= 2;
            }
        };
        let m = &mut self.paula.mem;
        let v = &mut self.voices[c];
        match ins.effect {
            // Wave negator: a point a time inverted.
            1 => {
                let i = buf + usize::from(v.fx_pos).min(127);
                m[i] = m[i].wrapping_neg();
                step(v);
            }
            // Free negator: the waveform inverted from a point a table says on.
            2 => {
                if v.flag & 4 == 0 {
                    let tab = wave(ins.arg1);
                    let val = i32::from(m[tab + usize::from(v.fx_wave_pos).min(127)] & 0x7F);
                    let src = wave(ins.waveform);
                    let len = (i32::from(ins.length) * 2).min(128);
                    let mut count = len;
                    let mut off = len;
                    while count > 0 && count >= val {
                        off -= 1;
                        m[buf + off as usize] = m[src + off as usize];
                        count -= 1;
                    }
                    let left = val - count;
                    count += left;
                    off += left;
                    while count > 0 {
                        off -= 1;
                        if (0..128).contains(&off) {
                            m[buf + off as usize] = m[src + off as usize].wrapping_neg();
                        }
                        count -= 1;
                    }
                    v.fx_wave_pos += 1;
                    if v.fx_wave_pos > ins.arg2 + ins.arg3 {
                        v.fx_wave_pos = ins.arg2;
                        if ins.arg3 == 0 && val == 0 {
                            v.flag |= 4;
                        }
                    }
                }
            }
            // Rotate vertical: a value added.
            3 => {
                let dv = ins.arg1 as i8;
                span(m, &mut |m, i| m[i] = m[i].wrapping_add(dv));
                step(v);
            }
            // Rotate horizontal: the points moved one along.
            4 => {
                let first = m[buf + start];
                for i in start..=stop.min(126) {
                    m[buf + i] = m[buf + i + 1];
                }
                m[buf + (stop + 1).min(127)] = first;
                step(v);
            }
            // Alien voice: another waveform added.
            5 => {
                let src = wave(ins.arg1);
                let mut i = start;
                loop {
                    m[buf + i] = m[buf + i].wrapping_add(m[src + i]);
                    if i >= stop {
                        break;
                    }
                    i += 1;
                }
            }
            // Poly negator: one point put back, the next inverted.
            6 => {
                let src = wave(ins.waveform);
                let mut pos = usize::from(v.fx_pos).min(127);
                m[buf + pos] = m[src + pos];
                if pos >= stop {
                    pos = start.wrapping_sub(1);
                }
                let j = buf + (pos.wrapping_add(1)).min(127);
                m[j] = m[j].wrapping_neg();
                step(v);
            }
            // Shack waves: a value from another waveform added (2 also inverts a point).
            7 | 8 => {
                let src = wave(ins.arg1);
                let dv = m[src + (start + usize::from(v.fx_pos)).min(127)];
                span(m, &mut |m, i| m[i] = m[i].wrapping_add(dv));
                if ins.effect == 8 {
                    let i = buf + (start + usize::from(v.fx_wave_pos)).min(127);
                    m[i] = m[i].wrapping_neg();
                    v.fx_wave_pos += 1;
                    if usize::from(v.fx_wave_pos) > stop.saturating_sub(start) {
                        v.fx_wave_pos = 0;
                    }
                }
                step(v);
            }
            // Metamorph: toward another waveform a point at a time.
            9 => {
                if v.flag & 1 == 0 {
                    towards(m, wave(ins.arg1), &mut v.flag);
                }
            }
            // Laser: the slide detuned so many times.
            10 => {
                if v.fx_wave_pos < ins.arg3 {
                    v.slide = v.slide.wrapping_add(i16::from(ins.arg2 as i8));
                    v.fx_wave_pos += 1;
                }
                step(v);
            }
            // Wave alias: each point nudged toward the next.
            11 => {
                let dv = ins.arg1 as i8;
                let mut i = start;
                loop {
                    let x = m[buf + i];
                    let next = if i + 1 >= 128 { None } else { Some(m[buf + i + 1]) };
                    m[buf + i] = if next.is_none_or(|n| x > n) { x.wrapping_sub(dv) } else { x.wrapping_add(dv) };
                    if i >= stop {
                        break;
                    }
                    i += 1;
                }
                step(v);
            }
            // Noise generator 1: a point mixed with noise.
            12 => {
                let r = {
                    self.random = self.random.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    (self.random >> 16) as i8
                };
                let i = buf + usize::from(v.fx_pos).min(127);
                m[i] ^= r;
                step(v);
            }
            // Low-pass filters: each point toward the next where they differ by more than a limit.
            13 | 14 => {
                let tab = wave(ins.arg1);
                let mut idx = stop;
                for i in start..=stop {
                    let a = m[buf + i];
                    let b = if i == stop { m[buf + start] } else { m[buf + i + 1] };
                    let up = a <= b;
                    let d = a.wrapping_sub(b).wrapping_abs();
                    let limit = if ins.effect == 13 {
                        i32::from(ins.arg1)
                    } else {
                        let l = i32::from(m[tab + idx] & 0x7F);
                        idx = (idx + 1) & 0x7F;
                        l
                    };
                    if i32::from(d) > limit {
                        m[buf + i] = if up { m[buf + i].wrapping_add(2) } else { m[buf + i].wrapping_sub(2) };
                    }
                }
                step(v);
            }
            // Oscillator: toward one waveform, then back toward the instrument's.
            15 => {
                if v.flag & 2 != 0 {
                    v.flag ^= 8;
                    v.flag &= !2;
                }
                let src = if v.flag & 8 != 0 { wave(ins.waveform) } else { wave(ins.arg1) };
                towards(m, src, &mut v.flag);
            }
            // Noise generator 2: every point turned and mixed with noise.
            16 => {
                let mut i = stop;
                loop {
                    let mut x = m[buf + i] ^ 0x05;
                    x = (x as u8).rotate_left(2) as i8;
                    self.random = self.random.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    x = x.wrapping_add((self.random >> 16) as i8);
                    m[buf + i] = x;
                    if i <= start {
                        break;
                    }
                    i -= 1;
                }
            }
            // FM drum: the slide falling, round and round.
            17 => {
                if v.fx_wave_pos >= ins.arg3 {
                    v.slide = i16::from(ins.finetune);
                    v.fx_wave_pos = 0;
                }
                let dec = (ins.arg2 << 8) | (ins.arg1 & 0xFF);
                v.slide = v.slide.wrapping_sub(dec as i16);
                v.fx_wave_pos += 1;
                step(v);
            }
            _ => {}
        }
    }
}

impl Driver for Player {
    fn channels(&self) -> usize {
        4
    }

    fn tick(&mut self) {
        self.counter += 1;
        if self.counter >= self.speed {
            self.counter = 0;
            self.next_row();
        }
        for c in 0..4 {
            self.update(c);
        }
    }

    fn tick_seconds(&self) -> f64 {
        // The CIA timer at the song's tempo (ticks a second).
        let t = u32::from(self.sub.tempo.max(1));
        f64::from(709_379 / t + 1) / 709_379.0
    }

    fn render(&mut self, rate: u32, frames: usize, out: &mut Bufs) {
        self.paula.render(rate, frames, out);
    }

    fn position(&self) -> (usize, usize) {
        self.shown
    }

    fn speed(&self) -> (u32, u32) {
        (u32::from(self.speed.max(1)), u32::from(self.sub.tempo) * 5 / 2)
    }

    fn take_done(&mut self) -> bool {
        std::mem::take(&mut self.done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods_are_sonic_arrangers() {
        assert_eq!(&[period(0), period(1), period(13), period(36)], &[0, 13696, 6848, 1812]);
        assert_eq!(period(61), 428);
        assert_eq!(&[period(85), period(91), period(96)], &[107, 75, 56]);
        assert_eq!(&[period(97), period(108)], &[53, 28]);
    }
}
