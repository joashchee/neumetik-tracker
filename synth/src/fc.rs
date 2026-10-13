// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! Future Composer 1.3 ("SMOD") and 1.4 ("FC14"), Jochen Hippel's and
//! SuperSero's: each channel steps through its own patterns (32 rows,
//! each transposed by the sequence), each note an instrument of two
//! sequences, ticked every frame: a frequency sequence (which waveform
//! or sample plays, its loop, transpositions, vibrato, pitch bends,
//! jumps) and a volume sequence (levels, slides, sustains). The
//! waveforms are short cycles (1.4 keeps 80 in the file; 1.3 has 47 of
//! its own, made here); the samples play as they are, 1.4's in sets
//! of 20 ("SSMP").

use std::sync::Arc;

use crate::paula::{Paula, CIA_50};
use crate::replay::{Bufs, Driver, Tune};

/// A sample or a waveform, where Paula has it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sound {
    /// Where its points start in the song's memory.
    pub at: usize,
    pub len: usize,
    pub loop_start: usize,
    pub loop_len: usize,
}

impl Sound {
    pub fn present(&self) -> bool {
        self.len > 0
    }
}

/// A position: each channel's pattern, its transposition and its
/// instruments' (sound) transposition, and a new speed (0 none).
#[derive(Clone, Copy, Debug, Default)]
pub struct Position {
    pub pattern: [u8; 4],
    pub transpose: [i8; 4],
    pub sound_transpose: [i8; 4],
    pub speed: u8,
}

/// A volume sequence: its speed, frequency sequence and vibrato, then its
/// 59 commands and levels.
#[derive(Clone, Debug)]
pub struct VolSeq {
    pub speed: u8,
    pub frequency: u8,
    pub vib_speed: i8,
    pub vib_depth: i8,
    pub vib_delay: u8,
    pub values: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Song {
    /// 1.3's file (its waveforms the replayer's) or 1.4's.
    pub v13: bool,
    pub positions: Vec<Position>,
    /// 32 rows of (note, info) each.
    pub patterns: Vec<[(u8, u8); 32]>,
    /// Every frequency sequence, after a silent one, 64 bytes each.
    pub frequencies: Vec<u8>,
    /// The volume sequences, a silent one first.
    pub volumes: Vec<VolSeq>,
    /// Ten samples, then 80 waveforms.
    pub sounds: Vec<Sound>,
    /// The samples kept in sets of 20 (1.4's "SSMP"), by sample.
    pub multi: Vec<Option<Vec<Sound>>>,
    /// Every sample's and waveform's points.
    pub mem: Vec<i8>,
    file: Arc<Vec<u8>>,
}

/// The silent sequence the frequency and volume sequences start with.
const SILENT: [u8; 8] = [0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xE1];

/// Future Composer 1.3's own 47 waveforms (1.4's files keep theirs).
fn waves_13() -> Vec<Vec<i8>> {
    let b = |v: &[u8]| v.iter().map(|&x| x as i8).collect::<Vec<i8>>();
    let mut w = Vec::new();
    // 0 to 15: one cycle, its second half's first k points run down and
    // up again in eighths.
    let base = [0xC0u8, 0xC0, 0xD0, 0xD8, 0xE0, 0xE8, 0xF0, 0xF8, 0x00, 0xF8, 0xF0, 0xE8, 0xE0, 0xD8, 0xD0, 0xC8, 0x3F, 0x37, 0x2F, 0x27, 0x1F, 0x17, 0x0F, 0x07, 0xFF, 0x07, 0x0F, 0x17, 0x1F, 0x27, 0x2F, 0x37];
    let run = [0xC0u8, 0xB8, 0xB0, 0xA8, 0xA0, 0x98, 0x90, 0x88, 0x80, 0x88, 0x90, 0x98, 0xA0, 0xA8, 0xB0];
    for k in 0..16 {
        let mut c = base;
        c[16..16 + k].copy_from_slice(&run[..k]);
        w.push(b(&c));
    }
    // 16 to 29: pulses of 32, 16 to 29 points low; 30 and 31, 30 and 31 lower.
    for low in 16..30 {
        w.push((0..32).map(|i| if i < low { -127 } else { 127 }).collect());
    }
    for low in 30..32 {
        w.push((0..32).map(|i| if i < low { -128 } else { 127 }).collect());
    }
    // 32 to 39: pulses of 16.
    for low in [8, 7, 6, 5, 4, 3, 2, 2] {
        w.push((0..16).map(|i| if i < low { -128 } else { 127 }).collect());
    }
    // 40 and 41: sawtooths of 32 and 16 (each starting on two lows; the
    // longer ending at the top).
    let saw32: Vec<i8> = (0..32).map(|i| if i == 1 { -128 } else if i == 31 { 127 } else { (-128 + 8 * i) as i8 }).collect();
    let saw16: Vec<i8> = (0..16).map(|i| if i == 1 { -128 } else { (-128 + 16 * i) as i8 }).collect();
    w.push(saw32.clone());
    w.push(saw16.clone());
    // 42 and 43: drawn.
    w.push(b(&[0x45, 0x45, 0x79, 0x7D, 0x7A, 0x77, 0x70, 0x66, 0x61, 0x58, 0x53, 0x4D, 0x2C, 0x20, 0x18, 0x12, 0x04, 0xDB, 0xD3, 0xCD, 0xC6, 0xBC, 0xB5, 0xAE, 0xA8, 0xA3, 0x9D, 0x99, 0x93, 0x8E, 0x8B, 0x8A]));
    w.push(b(&[0x45, 0x45, 0x79, 0x7D, 0x7A, 0x77, 0x70, 0x66, 0x5B, 0x4B, 0x43, 0x37, 0x2C, 0x20, 0x18, 0x12, 0x04, 0xF8, 0xE8, 0xDB, 0xCF, 0xC6, 0xBE, 0xB0, 0xA8, 0xA4, 0x9E, 0x9A, 0x95, 0x94, 0x8D, 0x83]));
    // 44 and 45: a triangle of 16 (its second point held).
    let tri: Vec<i8> = b(&[0x00, 0x00, 0x40, 0x60, 0x7F, 0x60, 0x40, 0x20, 0x00, 0xE0, 0xC0, 0xA0, 0x80, 0xA0, 0xC0, 0xE0]);
    w.push(tri.clone());
    w.push(tri);
    // 46: the two sawtooths one after the other.
    w.push(saw32.into_iter().chain(saw16).collect());
    w
}

/// FC's periods, as its replayer's table has them: ProTracker's octaves
/// 0 to 3, then the top held; an octave below 0, and another below that
/// (for notes transposed under 0, beyond the Amiga's range, so held at
/// its bottom); then 1.4's octaves 0 to 3 again. 1.3's table ends after
/// the lowest octave, its next words small enough to be held at the top.
pub fn period(n: usize, v13: bool) -> u16 {
    const OCT: [u16; 12] = [1712, 1616, 1524, 1440, 1356, 1280, 1208, 1140, 1076, 1016, 960, 906];
    // ProTracker's own rounding in its higher octaves.
    const O2: [u16; 12] = [428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226];
    const O3: [u16; 12] = [214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113];
    let (row, k) = (n / 12, n % 12);
    match (row, v13) {
        (0, _) | (7, false) => OCT[k],
        (1, _) | (8, false) => OCT[k] / 2,
        (2, _) | (9, false) => O2[k],
        (3, _) | (10, false) => O3[k],
        (5, _) => OCT[k] * 2,
        (6, _) => OCT[k] * 4,
        _ => 113,
    }
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
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from(self.u16()?) << 16 | u32::from(self.u16()?))
    }
}

/// The file is a Future Composer 1.3 or 1.4 song (its mark, and offsets
/// inside it).
pub fn probe(d: &[u8]) -> bool {
    if d.len() < 180 || !(d.starts_with(b"FC14") || d.starts_with(b"SMOD")) {
        return false;
    }
    (0..8).all(|i| {
        let o = 8 + i * 4;
        u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as usize <= d.len()
    })
}

impl Song {
    pub fn parse(d: &[u8]) -> Option<Song> {
        if !probe(d) {
            return None;
        }
        let v13 = d.starts_with(b"SMOD");
        let mut r = Reader { d, at: 4 };
        let mut seq_len = r.u32()? as usize;
        let mut offs = [0usize; 8];
        for o in offs.iter_mut() {
            *o = r.u32()? as usize;
        }
        let [pat_off, pat_len, frq_off, frq_len, vol_off, vol_len, smp_off, wav_off] = offs;
        if v13 && seq_len == 0 {
            seq_len = pat_off.saturating_sub(0x64);
        }
        let mut sample_info = Vec::new();
        for _ in 0..10 {
            let len = usize::from(r.u16()?) * 2;
            let mut ls = usize::from(r.u16()?);
            let mut ll = usize::from(r.u16()?) * 2;
            if ls >= len {
                ls = 0;
                ll = 2;
            }
            sample_info.push((len, ls, ll));
        }
        let waves13 = if v13 { waves_13() } else { Vec::new() };
        let wave_lens: Vec<usize> = if v13 { (0..80).map(|i| waves13.get(i).map_or(0, |w| w.len())).collect() } else { (0..80).map(|_| r.u8().map(|b| usize::from(b) * 2)).collect::<Option<_>>()? };
        let npos = seq_len / 13;
        let mut positions = Vec::with_capacity(npos.max(1));
        for _ in 0..npos {
            let mut p = Position::default();
            for c in 0..4 {
                p.pattern[c] = r.u8()?;
                p.transpose[c] = r.u8()? as i8;
                p.sound_transpose[c] = r.u8()? as i8;
            }
            p.speed = r.u8()?;
            positions.push(p);
        }
        if positions.is_empty() {
            positions.push(Position::default());
        }
        let mut pat = d.get(pat_off..pat_off + pat_len)?.to_vec();
        // 1.3's portamento speeds are half 1.4's.
        if v13 {
            let mut i = 1;
            while i + 2 < pat.len() {
                if pat[i] & 0x80 != 0 {
                    let p = pat[i + 2];
                    pat[i + 2] = (((p & 0x1F) * 2) & 0x1F) | (p & 0x20);
                }
                i += 2;
            }
        }
        let patterns: Vec<[(u8, u8); 32]> = pat
            .chunks_exact(64)
            .map(|c| {
                let mut rows = [(0u8, 0u8); 32];
                for (j, row) in rows.iter_mut().enumerate() {
                    *row = (c[j * 2], c[j * 2 + 1]);
                }
                rows
            })
            .collect();
        let mut frequencies = SILENT.to_vec();
        frequencies.extend_from_slice(d.get(frq_off..frq_off + frq_len)?);
        frequencies.push(0xE1);
        let mut volumes = vec![VolSeq { speed: SILENT[0], frequency: SILENT[1], vib_speed: SILENT[2] as i8, vib_depth: SILENT[3] as i8, vib_delay: SILENT[4], values: SILENT[4..].to_vec() }];
        let vols = d.get(vol_off..vol_off + vol_len)?;
        for v in vols.chunks_exact(64) {
            volumes.push(VolSeq { speed: v[0], frequency: v[1], vib_speed: v[2] as i8, vib_depth: v[3] as i8, vib_delay: v[4], values: v[5..].to_vec() });
        }
        // The samples (each with two pad bytes after it in 1.4), then the waveforms.
        let mut mem: Vec<i8> = Vec::new();
        let mut sounds = vec![Sound::default(); 90];
        let mut multi = vec![None; 10];
        let mut r = Reader { d, at: smp_off };
        for (i, &(len, ls, ll)) in sample_info.iter().enumerate() {
            if len != 0 {
                if !v13 && d.get(r.at..r.at + 4) == Some(b"SSMP") {
                    r.at += 4;
                    let mut heads = Vec::new();
                    for _ in 0..20 {
                        let off = r.u32()? as usize;
                        let l = usize::from(r.u16()?) * 2;
                        let s = usize::from(r.u16()?);
                        let n = usize::from(r.u16()?) * 2;
                        r.at += 6;
                        heads.push((off, l, s, n));
                    }
                    let base = r.at;
                    let mut set = Vec::new();
                    for (off, l, s, n) in heads {
                        let mut snd = Sound { at: mem.len(), len: l, loop_start: s, loop_len: n };
                        if l != 0 {
                            let pts = d.get(base + off..(base + off + l).min(d.len())).unwrap_or_default();
                            mem.extend(pts.iter().map(|&x| x as i8));
                            mem.resize(snd.at + l, 0);
                            r.at = base + off + l + 2;
                        } else {
                            snd = Sound::default();
                        }
                        set.push(snd);
                    }
                    multi[i] = Some(set);
                } else {
                    // A sample cut short by the file's end plays what's there.
                    let pts = d.get(r.at..(r.at + len).min(d.len())).unwrap_or_default();
                    let at = mem.len();
                    mem.extend(pts.iter().map(|&x| x as i8));
                    mem.resize(at + len, 0);
                    sounds[i] = Sound { at, len, loop_start: ls, loop_len: ll };
                    r.at += len;
                }
            }
            if !v13 {
                r.at += 2;
            }
        }
        for (k, &len) in wave_lens.iter().enumerate() {
            if len == 0 {
                continue;
            }
            let at = mem.len();
            if v13 {
                mem.extend_from_slice(&waves13[k]);
            } else {
                let o = wav_off + wave_lens[..k].iter().sum::<usize>();
                let pts = d.get(o..(o + len).min(d.len())).unwrap_or_default();
                mem.extend(pts.iter().map(|&x| x as i8));
                mem.resize(at + len, 0);
            }
            sounds[10 + k] = Sound { at, len, loop_start: 0, loop_len: len };
        }
        Some(Song { v13, positions, patterns, frequencies, volumes, sounds, multi, mem, file: Arc::new(d.to_vec()) })
    }

    /// A channel's row at a position, as the pattern has it.
    pub fn row(&self, position: usize, channel: usize, row: usize) -> (u8, u8) {
        let p = self.positions.get(position).map_or(0, |p| usize::from(p.pattern[channel]));
        self.patterns.get(p).map_or((0, 0), |r| r[row.min(31)])
    }
}

impl Tune for Song {
    fn format(&self) -> &'static str {
        "fc"
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
    pitch_bend_speed: i8,
    pitch_bend_time: u8,
    pos: usize,
    note: i8,
    vol_seq: usize,
    vol_bend_speed: u8,
    vol_bend_time: u8,
    vol_pos: usize,
    sound_transpose: i8,
    vol_counter: u8,
    vol_speed: u8,
    vol_sustain: u8,
    sustain: u8,
    vib_speed: i8,
    vib_depth: i8,
    vib_value: i8,
    vib_delay: i8,
    pattern: usize,
    vol_bend_flag: bool,
    port_flag: bool,
    row: usize,
    pitch_bend_flag: bool,
    patt_transpose: i8,
    transpose: i8,
    volume: i8,
    vib_flag: u8,
    portamento: u8,
    frq_start: usize,
    frq_pos: usize,
    pitch: u16,
    /// Whether a waveform or sample was set since the note.
    active: bool,
}

pub struct Player {
    song: Arc<Song>,
    paula: Paula,
    voices: [Voice; 4],
    count: u16,
    speed: u16,
    speed_count: u16,
    shown: (usize, usize),
    visited: Vec<bool>,
    done: bool,
}

impl Player {
    fn new(song: Arc<Song>) -> Player {
        let spd = match song.positions[0].speed {
            0 => 3,
            s => u16::from(s),
        };
        let mut voices: [Voice; 4] = Default::default();
        for (c, v) in voices.iter_mut().enumerate() {
            let p = song.positions[0];
            *v = Voice { vol_counter: 1, vol_speed: 1, pattern: usize::from(p.pattern[c]), transpose: p.transpose[c], sound_transpose: p.sound_transpose[c], ..Default::default() };
        }
        let paula = Paula::new(4, song.mem.clone());
        let mut visited = vec![false; song.positions.len()];
        visited[0] = true;
        Player { song, paula, voices, count: spd, speed: spd, speed_count: 1, shown: (0, 0), visited, done: false }
    }

    fn new_note(&mut self, c: usize) {
        let song = self.song.clone();
        let v = &mut self.voices[c];
        let rows = song.patterns.get(v.pattern);
        let row_note = |v: &Voice| rows.map_or(0, |r| r[v.row.min(31)].0);
        if v.row == 32 || row_note(v) == 0x49 {
            v.pos += 1;
            v.row = 0;
            if v.pos >= song.positions.len() {
                v.pos = 0;
            }
            self.speed_count += 1;
            if self.speed_count == 5 {
                self.speed_count = 1;
                let s = song.positions[v.pos].speed;
                if s != 0 {
                    self.count = u16::from(s);
                    self.speed = self.count;
                }
            }
            let p = song.positions[v.pos];
            v.transpose = p.transpose[c];
            v.sound_transpose = p.sound_transpose[c];
            v.pattern = if usize::from(p.pattern[c]) < song.patterns.len() { usize::from(p.pattern[c]) } else { 0 };
            if c == 0 {
                if self.visited[v.pos] {
                    self.done = true;
                    self.visited.iter_mut().for_each(|x| *x = false);
                }
                self.visited[v.pos] = true;
            }
        }
        let rows = song.patterns.get(v.pattern);
        let (note, info) = rows.map_or((0, 0), |r| r[v.row.min(31)]);
        if c == 0 {
            self.shown = (v.pos, v.row);
        }
        if note != 0 || info & 0xC0 != 0 {
            if note != 0 {
                v.pitch = 0;
            }
            v.portamento = if info & 0x80 != 0 { if v.row < 31 { rows.map_or(0, |r| r[v.row + 1].1) } else { 0 } } else { 0 };
        }
        let note = note & 0x7F;
        if note != 0 {
            v.note = note as i8;
            v.active = false;
            self.paula.stop(c);
            let mut inst = ((info & 0x3F) as i8).wrapping_add(v.sound_transpose) as u8 as usize;
            if inst >= song.volumes.len() - 1 {
                inst = 0;
            } else {
                inst += 1;
            }
            let vs = &song.volumes[inst];
            v.vol_pos = 0;
            v.vol_counter = vs.speed;
            v.vol_speed = vs.speed;
            v.vol_sustain = 0;
            v.vib_speed = vs.vib_speed;
            v.vib_flag = 0x40;
            v.vib_depth = vs.vib_depth;
            v.vib_value = vs.vib_depth;
            v.vib_delay = vs.vib_delay as i8;
            v.vol_seq = inst;
            v.frq_start = SILENT.len() + usize::from(vs.frequency) * 64;
            v.frq_pos = 0;
            v.sustain = 0;
        }
        v.row += 1;
    }

    /// A sound starts on the channel: its points, then its loop.
    fn start(&mut self, c: usize, s: Sound) {
        if !s.present() {
            return;
        }
        self.paula.play(c, s.at, s.len);
        if s.loop_len > 2 {
            let ll = if s.loop_start + s.loop_len > s.len { s.len - s.loop_start } else { s.loop_len };
            self.paula.set_loop(c, s.at + s.loop_start, ll);
        }
    }

    fn effect(&mut self, c: usize) {
        let song = self.song.clone();
        let f = &song.frequencies;
        // The frequency sequence.
        loop {
            let mut again = false;
            let v = &mut self.voices[c];
            if v.sustain != 0 {
                v.sustain -= 1;
                break;
            }
            let mut at = v.frq_start + v.frq_pos;
            if at >= f.len() {
                break;
            }
            loop {
                let mut jumped = false;
                let v = &mut self.voices[c];
                let mut cmd = f[at];
                at += 1;
                if cmd == 0xE1 {
                    break;
                }
                if cmd == 0xE0 {
                    let to = usize::from(f.get(at).copied().unwrap_or(0) & 0x3F);
                    v.frq_pos = to;
                    at = v.frq_start + to;
                    cmd = f.get(at).copied().unwrap_or(0xE1);
                    at += 1;
                }
                let get = |i: usize| f.get(i).copied().unwrap_or(0);
                match cmd {
                    // A waveform or sample.
                    0xE2 => {
                        let n = usize::from(get(at));
                        at += 1;
                        if n < 90 {
                            let s = song.sounds[n];
                            self.start(c, s);
                        }
                        let v = &mut self.voices[c];
                        v.vol_pos = 0;
                        v.vol_counter = 1;
                        v.frq_pos += 2;
                        v.active = true;
                    }
                    // Its loop: another waveform or sample's.
                    0xE4 => {
                        if v.active {
                            let n = usize::from(get(at));
                            at += 1;
                            if n < 90 {
                                let s = song.sounds[n];
                                self.paula.set_loop(c, s.at + s.loop_start, s.loop_len);
                            }
                            self.voices[c].frq_pos += 2;
                        }
                    }
                    // One of a set of samples.
                    0xE9 => {
                        v.active = true;
                        let n = usize::from(get(at));
                        at += 1;
                        if let Some(Some(set)) = song.multi.get(n) {
                            let k = usize::from(get(at));
                            at += 1;
                            if let Some(&s) = set.get(k) {
                                self.start(c, s);
                            }
                            let v = &mut self.voices[c];
                            v.vol_pos = 0;
                            v.vol_counter = 1;
                        }
                        self.voices[c].frq_pos += 3;
                    }
                    // On to another sequence.
                    0xE7 => {
                        jumped = true;
                        let to = SILENT.len() + usize::from(get(at)) * 64;
                        at = if to >= f.len() { 0 } else { to };
                        v.frq_start = at;
                        v.frq_pos = 0;
                    }
                    0xEA => {
                        v.pitch_bend_speed = get(at) as i8;
                        v.pitch_bend_time = get(at + 1);
                        at += 2;
                        v.frq_pos += 3;
                    }
                    0xE8 => {
                        v.sustain = get(at);
                        at += 1;
                        v.frq_pos += 2;
                        again = true;
                    }
                    0xE3 => {
                        v.vib_speed = get(at) as i8;
                        v.vib_depth = get(at + 1) as i8;
                        at += 2;
                        v.frq_pos += 3;
                    }
                    _ => {}
                }
                if !jumped && !again {
                    let v = &mut self.voices[c];
                    at = v.frq_start + v.frq_pos;
                    v.patt_transpose = get(at) as i8;
                    v.frq_pos += 1;
                }
                if !jumped {
                    break;
                }
            }
            if !again {
                break;
            }
        }
        // The volume sequence.
        let v = &mut self.voices[c];
        let vals = &song.volumes[v.vol_seq].values;
        if v.vol_sustain != 0 {
            v.vol_sustain -= 1;
        } else if v.vol_bend_time != 0 {
            vol_bend(v);
        } else {
            v.vol_counter = v.vol_counter.wrapping_sub(1);
            if v.vol_counter == 0 {
                v.vol_counter = v.vol_speed;
                for _ in 0..64 {
                    if v.vol_pos >= vals.len() || vals[v.vol_pos] == 0xE1 {
                        break;
                    }
                    let at = |i: usize| vals.get(i).copied().unwrap_or(0);
                    match vals[v.vol_pos] {
                        0xEA => {
                            v.vol_bend_speed = at(v.vol_pos + 1);
                            v.vol_bend_time = at(v.vol_pos + 2);
                            v.vol_pos += 3;
                            vol_bend(v);
                        }
                        0xE8 => {
                            v.vol_sustain = at(v.vol_pos + 1);
                            v.vol_pos += 2;
                        }
                        0xE0 => {
                            // The position counts from the sequence's header.
                            v.vol_pos = usize::from(at(v.vol_pos + 1) & 0x3F).wrapping_sub(5);
                            continue;
                        }
                        x => {
                            v.volume = (x & 0x7F) as i8;
                            v.vol_pos += 1;
                        }
                    }
                    break;
                }
            }
        }
        // The period.
        let mut note = v.patt_transpose;
        if note >= 0 {
            note = note.wrapping_add(v.note).wrapping_add(v.transpose);
        }
        let note = (note as u8 & 0x7F) as usize;
        let mut period = period(note, song.v13);
        let mut flag = v.vib_flag;
        if v.vib_delay != 0 {
            v.vib_delay -= 1;
        } else {
            let mut base = (note * 2) as u16;
            let mut dep = v.vib_depth.wrapping_mul(2);
            let mut val = v.vib_value;
            if flag & 0x80 == 0 || flag & 0x01 == 0 {
                if flag & 0x20 != 0 {
                    val = val.wrapping_add(v.vib_speed);
                    if val >= dep {
                        flag &= !0x20;
                        val = dep;
                    }
                } else {
                    val = val.wrapping_sub(v.vib_speed);
                    if val < 0 {
                        flag |= 0x20;
                        val = 0;
                    }
                }
                v.vib_value = val;
            }
            dep /= 2;
            val = val.wrapping_sub(dep);
            base += 160;
            while base < 256 {
                val = val.wrapping_mul(2);
                base += 24;
            }
            period = period.wrapping_add(val as i16 as u16);
        }
        v.vib_flag = flag ^ 0x01;
        v.port_flag = !v.port_flag;
        if v.port_flag && v.portamento != 0 {
            if v.portamento <= 31 {
                v.pitch = v.pitch.wrapping_sub(u16::from(v.portamento));
            } else {
                v.pitch = v.pitch.wrapping_add(u16::from(v.portamento & 0x1F));
            }
        }
        v.pitch_bend_flag = !v.pitch_bend_flag;
        if v.pitch_bend_flag && v.pitch_bend_time != 0 {
            v.pitch_bend_time -= 1;
            v.pitch = v.pitch.wrapping_sub(v.pitch_bend_speed as i16 as u16);
        }
        // Held to the Amiga's range as a word (one below 0 is one too low).
        period = period.wrapping_add(v.pitch);
        let period = u32::from(period.clamp(113, 3424));
        v.volume = v.volume.clamp(0, 64);
        let vol = v.volume as u32;
        self.paula.set_period(c, period);
        self.paula.set_volume(c, vol);
    }
}

fn vol_bend(v: &mut Voice) {
    v.vol_bend_flag = !v.vol_bend_flag;
    if v.vol_bend_flag {
        v.vol_bend_time -= 1;
        v.volume = v.volume.wrapping_add(v.vol_bend_speed as i8);
        if v.volume > 64 {
            v.volume = 64;
            v.vol_bend_time = 0;
        } else if v.volume < 0 {
            v.volume = 0;
            v.vol_bend_time = 0;
        }
    }
}

impl Driver for Player {
    fn channels(&self) -> usize {
        4
    }

    fn tick(&mut self) {
        self.count -= 1;
        if self.count == 0 {
            self.count = self.speed;
            for c in 0..4 {
                self.new_note(c);
            }
        }
        for c in 0..4 {
            self.effect(c);
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
    fn periods_are_the_replayers() {
        assert_eq!(period(0, false), 1712);
        assert_eq!(period(12, false), 856);
        assert_eq!(period(24, false), 428);
        assert_eq!(period(47, false), 113);
        assert_eq!(period(55, false), 113);
        assert_eq!(period(60, false), 3424);
        assert_eq!(period(73, false), 6464);
        assert_eq!(period(84, false), 1712);
        assert_eq!(period(131, false), 113);
        assert_eq!(period(73, true), 6464);
        assert_eq!(period(84, true), 113);
    }

    #[test]
    fn the_13_waves_have_their_shapes() {
        let w = waves_13();
        assert_eq!(w.len(), 47);
        assert_eq!(w[1][16], -64);
        assert_eq!(w[15][30], 0xB0u8 as i8);
        assert_eq!((w[16].iter().filter(|&&x| x == -127).count(), w[29].iter().filter(|&&x| x == 127).count()), (16, 3));
        assert_eq!(w[39], w[38]);
        assert_eq!((w[40][0], w[40][1], w[40][2], w[40][31]), (-128, -128, -112, 127));
        assert_eq!(w[46].len(), 48);
    }
}
