// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! Voodoo Supreme Synthesizer ("VSS0" in its last 64 bytes): each
//! sub-song a table of structures (tracks, volume envelopes, period
//! tables, waveform tables, 32-point waveforms and samples), its first
//! four the channels' tracks: streams of notes, each held its own count
//! of ticks, with calls, loops and jumps. Each channel's sound is two
//! waveforms made into a third every tick, ring-modulated (averaged, one
//! turning against the other), XOR-ed or morphed one into the other over
//! a moving span, or a sample stepped through block by block at the
//! audio interrupt, its length kept as its pitch changes.

use std::sync::Arc;

use crate::paula::{Paula, Stepper};
use crate::replay::{Bufs, Driver, Tune};

/// The periods: one octave (1,008 to 532) from 32 times over down to a
/// quarter, as the replayer's table has them but for its twelfth, which
/// it gives as 17,689 (the octave would say 17,024).
pub fn period(i: usize) -> u16 {
    const OCTAVE: [u32; 12] = [1008, 952, 896, 848, 800, 756, 712, 672, 636, 600, 564, 532];
    let i = i.min(95);
    if i == 11 {
        return 17_689;
    }
    ((OCTAVE[i % 12] * 32) >> (i / 12)) as u16
}

/// The replayer's fractions for a semitone's steps up (each the period's
/// shrink as numerator over denominator), 0 to 12.
const RATIOS: [(u16, u16); 13] = [(1, 1), (107, 101), (55, 49), (44, 37), (160, 127), (4, 3), (140, 99), (218, 146), (100, 63), (111, 66), (98, 55), (168, 89), (2, 1)];

/// One of a sub-song's structures.
#[derive(Clone, Debug, Default)]
pub enum Item {
    #[default]
    Unknown,
    /// A track's bytes (each jump's target made a place in it).
    Track(Vec<u8>),
    Volume(Vec<u8>),
    Periods(Vec<u8>),
    Waves(Vec<u8>),
    /// 32 points at a place in the file.
    Waveform(usize),
    /// Points at a place in the file, and how many.
    Sample(usize, usize),
}

#[derive(Clone, Debug)]
pub struct SubSong {
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct Song {
    pub subsongs: Vec<SubSong>,
    pub mem: Vec<i8>,
    empty: usize,
    file: Arc<Vec<u8>>,
}

fn be32(d: &[u8], i: usize) -> Option<i32> {
    Some(i32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?))
}

/// Where the footer is, and how many songs it lists.
fn footer(d: &[u8]) -> Option<(usize, usize)> {
    if d.len() < 64 {
        return None;
    }
    let base = (d.len() - 64 + 1) & !1;
    let b = d.get(base..base + 64)?;
    let i = (0..56).step_by(2).find(|&i| &b[i..i + 4] == b"VSS0")?;
    let songs = u32::from_be_bytes(b[i + 4..i + 8].try_into().ok()?);
    (songs > 0 && songs < 0x100).then_some((d.len() - 64 + i, songs as usize))
}

pub fn probe(d: &[u8]) -> bool {
    footer(d).is_some()
}

/// A track's bytes from `at`, marking the structures its commands name.
fn read_track(d: &[u8], at: usize, offsets: &[usize], kinds: &mut [u8]) -> Option<Vec<u8>> {
    const TRACK: u8 = 1;
    const VOLUME: u8 = 2;
    const PERIODS: u8 = 3;
    const WAVES: u8 = 4;
    const WAVEFORM: u8 = 5;
    const SAMPLE: u8 = 6;
    let mut t = Vec::new();
    let mut i = at;
    let mut sample_mode = false;
    let mut pending: [Option<usize>; 2] = [None, None];
    let byte = |i: &mut usize| -> Option<u8> {
        let x = *d.get(*i)?;
        *i += 1;
        Some(x)
    };
    loop {
        let cmd = byte(&mut i)?;
        t.push(cmd);
        if cmd < 0x80 {
            for p in pending.iter_mut() {
                if let Some(k) = p.take() {
                    *kinds.get_mut(k)? = if sample_mode { SAMPLE } else { WAVEFORM };
                }
            }
            t.push(byte(&mut i)?);
            continue;
        }
        match cmd {
            0x81 | 0x86 | 0x87 => {
                let a = byte(&mut i)?;
                t.push(a);
                *kinds.get_mut(usize::from(a))? = match cmd {
                    0x81 => TRACK,
                    0x86 => VOLUME,
                    _ => PERIODS,
                };
            }
            0x82 => return Some(t),
            0x84 => {}
            0x83 => {
                t.push(byte(&mut i)?);
                t.push(byte(&mut i)?);
            }
            0x85 => {
                for p in pending.iter_mut() {
                    let a = byte(&mut i)?;
                    t.push(a);
                    *p = Some(usize::from(a));
                }
            }
            0x88 => {
                let a = byte(&mut i)?;
                t.push(a);
                *kinds.get_mut(usize::from(a))? = WAVES;
                let m = byte(&mut i)?;
                t.push(m);
                sample_mode = m & 0x20 != 0;
            }
            0x89 => {
                for _ in 0..3 {
                    t.push(byte(&mut i)?);
                }
            }
            0x8A | 0x8C | 0x8D | 0xFF => t.push(byte(&mut i)?),
            0x8B => {
                let a = byte(&mut i)?;
                let to = *offsets.get(usize::from(a))?;
                let pos = if to >= at && to < at + t.len() { to - at } else { 0 };
                t.push((pos >> 8) as u8);
                t.push(pos as u8);
                return Some(t);
            }
            _ => return None,
        }
        if t.len() > 0x10000 {
            return None;
        }
    }
}

/// A run of (value, ticks) pairs from `at`, ended by a pair starting `end`.
fn read_pairs(d: &[u8], at: usize, end: u8) -> Option<Vec<u8>> {
    let mut t = Vec::new();
    let mut i = at;
    loop {
        let x = *d.get(i)?;
        t.push(x);
        t.push(*d.get(i + 1)?);
        i += 2;
        if x == end {
            return Some(t);
        }
        if t.len() > 0x2000 {
            return None;
        }
    }
}

fn read_subsong(d: &[u8], at: usize, foot: usize) -> Option<SubSong> {
    // The offsets run up to the first structure.
    let mut offsets = Vec::new();
    let mut min = foot;
    let mut i = at;
    loop {
        let o = be32(d, i)?;
        i += 4;
        let to = at as i64 + i64::from(o);
        if o >= 0 {
            min = min.min(to as usize);
        }
        offsets.push(to.max(0) as usize);
        if i >= min || offsets.len() > 1024 {
            break;
        }
    }
    if offsets.len() < 4 {
        return None;
    }
    let mut kinds = vec![0u8; offsets.len()];
    kinds[..4].fill(1);
    let mut items = vec![Item::Unknown; offsets.len()];
    // Tracks, until no call names one not read yet.
    loop {
        let mut more = false;
        for k in 0..offsets.len() {
            if kinds[k] == 1 && matches!(items[k], Item::Unknown) {
                items[k] = Item::Track(read_track(d, offsets[k], &offsets, &mut kinds)?);
                more = true;
            }
        }
        if !more {
            break;
        }
    }
    for k in 0..offsets.len() {
        let o = offsets[k];
        items[k] = match kinds[k] {
            1 => std::mem::take(&mut items[k]),
            2 => Item::Volume(read_pairs(d, o, 0x88)?),
            3 => Item::Periods(read_pairs(d, o, 0xFF)?),
            4 => Item::Waves(d.get(o..o + 28)?.to_vec()),
            5 => {
                d.get(o..o + 32)?;
                Item::Waveform(o)
            }
            6 => {
                let len = usize::from(u16::from_be_bytes(d.get(o.checked_sub(2)?..o)?.try_into().ok()?));
                Item::Sample(o, len)
            }
            _ => Item::Unknown,
        };
    }
    Some(SubSong { items })
}

impl Song {
    pub fn parse(d: &[u8]) -> Option<Song> {
        let (foot, songs) = footer(d)?;
        let subsongs: Vec<SubSong> = (0..songs).map(|k| read_subsong(d, (foot as i64 + i64::from(be32(d, foot + 8 + k * 4)?)).max(0) as usize, foot)).collect::<Option<_>>()?;
        let mut mem: Vec<i8> = d.iter().map(|&x| x as i8).collect();
        // Room past the end, for a block read beyond a sample's.
        mem.resize(mem.len() + 256, 0);
        let empty = mem.len();
        mem.resize(empty + 256, 0);
        Some(Song { subsongs, mem, empty, file: Arc::new(d.to_vec()) })
    }

    /// A channel's notes until its first jump back (or `most` ticks):
    /// (tick, note, ticks held), note 0xFF for a cut.
    pub fn timeline(&self, sub: usize, c: usize, most: usize) -> Vec<(usize, u8, usize)> {
        let items = &self.subsongs[sub].items;
        let mut out = Vec::new();
        let (mut track, mut pos, mut tr, mut t) = (c, 0usize, 0i8, 0usize);
        let mut stack: Vec<(usize, usize, u8)> = Vec::new();
        let mut guard = 0;
        while t < most && guard < 100_000 {
            guard += 1;
            let Some(Item::Track(b)) = items.get(track) else { break };
            let at = |i: usize| b.get(i).copied().unwrap_or(0x82);
            let cmd = at(pos);
            pos += 1;
            let held = |w: u8| if w == 0 { 256 } else { usize::from(w) };
            match cmd {
                0..=0x7F => {
                    let w = held(at(pos));
                    pos += 1;
                    out.push((t, (i32::from(cmd) + i32::from(tr)).clamp(0, 95) as u8, w));
                    t += w;
                }
                0x81 => {
                    stack.push((track, pos + 1, 0));
                    track = usize::from(at(pos));
                    pos = 0;
                }
                0x82 => match stack.pop() {
                    Some((k, p, _)) => (track, pos) = (k, p),
                    None => break,
                },
                0x83 => {
                    stack.push((track, pos + 2, at(pos)));
                    pos += 2;
                }
                0x84 => {
                    if let Some(&(k, p, n)) = stack.last() {
                        stack.pop();
                        if n > 1 {
                            stack.push((k, p, n - 1));
                            pos = p;
                        }
                    }
                }
                0x85 | 0x88 => pos += 2,
                0x86 | 0x87 | 0x8C | 0x8D => pos += 1,
                0x89 => {
                    let w = held(at(pos + 2));
                    out.push((t, (i32::from(at(pos)) + i32::from(tr)).clamp(0, 95) as u8, w));
                    pos += 3;
                    t += w;
                }
                0x8A => {
                    tr = at(pos) as i8;
                    pos += 1;
                }
                0xFF => {
                    let w = held(at(pos));
                    pos += 1;
                    out.push((t, 0xFF, w));
                    t += w;
                }
                _ => break,
            }
        }
        out
    }
}

impl Song {
    /// A sub-song's rows for the screen: the ticks every note's hold is a
    /// multiple of (at most 16), and how many rows its longest channel runs
    /// before its jump back.
    pub fn rows(&self, sub: usize) -> (usize, usize) {
        fn gcd(a: usize, b: usize) -> usize {
            if b == 0 { a } else { gcd(b, a % b) }
        }
        let lines: Vec<_> = (0..4).map(|c| self.timeline(sub, c, 100_000)).collect();
        let grain = lines.iter().flatten().fold(0, |g, &(t, _, w)| gcd(gcd(g, t), w)).clamp(1, 16);
        let end = lines.iter().filter_map(|l| l.last().map(|&(t, _, w)| t + w)).max().unwrap_or(0);
        (grain, end.div_ceil(grain).max(1))
    }
}

impl Tune for Song {
    fn format(&self) -> &'static str {
        "vss"
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

const FREQ_LENGTH: u8 = 1 << 0;
const FREQ_MAPPED: u8 = 1 << 5;
const XOR: u8 = 1 << 6;
const MORPH: u8 = 1 << 7;

const KEEP_WAVES: u8 = 1 << 5;
const KEEP_PERIODS: u8 = 1 << 6;
const KEEP_VOLUME: u8 = 1 << 7;

#[derive(Clone, Copy)]
enum Frame {
    Call(usize, usize),
    Loop(usize, u8),
}

#[derive(Clone, Default)]
struct Voice {
    /// Its two waveforms (or samples): where, and how many points.
    sample1: Option<(usize, usize)>,
    sample2: Option<(usize, usize)>,
    /// Its two buffers, and which plays.
    buffers: [usize; 2],
    use_buffer: usize,
    stack: Vec<Frame>,
    track: usize,
    pos: usize,
    ticks: u8,
    transpose: i8,
    period: u16,
    target_period: u16,
    volume: u8,
    final_volume: u8,
    reset: u8,
    porta_counter: u8,
    porta_step: u8,
    porta_down: bool,
    porta_delay: u8,
    porta_duration: u8,
    env: Option<usize>,
    env_pos: u8,
    env_counter: u8,
    env_delta: u8,
    ptab: Option<usize>,
    ptab_pos: u8,
    ptab_counter: u8,
    ptab_cmd: u8,
    wtab: Option<usize>,
    wtab_pos: u8,
    wave_start: u8,
    wave_pos: u8,
    wave_counter: u8,
    wave_step: u8,
    mask: u8,
    mode: u8,
    morph_speed: u8,
    /// The channel's location and length registers (bytes).
    lc: usize,
    len: usize,
    /// Ticks played, for the screen.
    played: usize,
}

pub struct Player {
    song: Arc<Song>,
    sub: usize,
    /// The screen's rows: ticks a row, and rows in all.
    rows: (usize, usize),
    paula: Paula,
    voices: [Voice; 4],
    /// The channels that have jumped back since the song last ended.
    looped: u8,
    done: bool,
}

impl Player {
    fn new(song: Arc<Song>, sub: usize) -> Player {
        let mut paula = Paula::new(4, song.mem.clone());
        let empty = song.empty;
        let voices = std::array::from_fn(|c| {
            let buffers = [paula.alloc(32), paula.alloc(32)];
            paula.play(c, empty, 32);
            paula.set_loop(c, empty, 32);
            Voice { buffers, track: c, ticks: 1, wave_step: 1, lc: empty, len: 32, ..Default::default() }
        });
        let rows = song.rows(sub);
        Player { song, sub, rows, paula, voices, looped: 0, done: false }
    }

    fn item(&self, k: usize) -> &Item {
        self.song.subsongs[self.sub].items.get(k).unwrap_or(&Item::Unknown)
    }

    fn table(&self, k: Option<usize>) -> &[u8] {
        match k.map(|k| self.item(k)) {
            Some(Item::Volume(t) | Item::Periods(t) | Item::Waves(t)) => t,
            _ => &[],
        }
    }

    /// The channel's next block, from its location register.
    fn set_address(&mut self, c: usize, at: usize) {
        let v = &mut self.voices[c];
        v.lc = at;
        self.paula.set_loop(c, at, v.len);
    }

    fn set_length(&mut self, c: usize, bytes: usize) {
        let v = &mut self.voices[c];
        v.len = bytes;
        self.paula.set_loop(c, v.lc, bytes);
    }

    fn get_period(&self, c: usize, note: u8) -> u16 {
        period((i32::from(note) + i32::from(self.voices[c].transpose)).max(0) as usize)
    }

    fn sound(&self, k: usize) -> Option<(usize, usize)> {
        match self.item(k) {
            Item::Waveform(at) => Some((*at, 32)),
            Item::Sample(at, len) => Some((*at, *len)),
            _ => None,
        }
    }

    fn voice(&mut self, c: usize) {
        let v = &mut self.voices[c];
        v.played += 1;
        v.ticks = v.ticks.wrapping_sub(1);
        if v.ticks == 0 {
            self.track(c);
        }
        self.volume_envelope(c);
        self.period_table(c);
        let v = &mut self.voices[c];
        // Portamento: so many steps over its ticks.
        v.porta_duration = v.porta_duration.wrapping_sub(1);
        if v.porta_duration == 0 {
            v.porta_step = 0;
            v.porta_counter = 0;
        } else {
            v.porta_counter = v.porta_counter.wrapping_sub(1);
            if v.porta_counter == 0 {
                v.porta_counter = v.porta_delay;
                v.period = if v.porta_down { v.period.wrapping_sub(u16::from(v.porta_step)) } else { v.period.wrapping_add(u16::from(v.porta_step)) };
            }
        }
        self.period_command(c);
        // The chip takes last tick's period and volume.
        let v = &mut self.voices[c];
        self.paula.set_period(c, u32::from(v.target_period));
        self.paula.set_volume(c, u32::from(v.final_volume));
        v.target_period = v.period;
        v.final_volume = (u32::from(v.volume) * 64 / 64) as u8;
        let mode = v.mode;
        if mode & XOR != 0 {
            self.xor(c);
        } else if mode & MORPH != 0 {
            self.morph(c);
        } else if mode & FREQ_MAPPED != 0 {
            self.freq_mapped(c);
        } else {
            self.ring(c);
        }
    }

    fn volume_envelope(&mut self, c: usize) {
        let t = self.table(self.voices[c].env).to_vec();
        let v = &mut self.voices[c];
        v.env_counter = v.env_counter.wrapping_sub(1);
        if v.env_counter != 0 {
            v.volume = v.volume.wrapping_add(v.env_delta);
            return;
        }
        if t.is_empty() {
            return;
        }
        let at = |i: u8| t.get(usize::from(i)).copied().unwrap_or(0);
        let mut guard = 0;
        while at(v.env_pos) == 0x88 && guard < 64 {
            v.env_pos = at(v.env_pos.wrapping_add(1));
            guard += 1;
        }
        let (x, n) = (at(v.env_pos), at(v.env_pos.wrapping_add(1)));
        if n == 0 {
            v.volume = x;
            v.env_counter = 1;
        } else {
            v.env_delta = x;
            v.env_counter = n;
            v.volume = v.volume.wrapping_add(x);
        }
        v.env_pos = v.env_pos.wrapping_add(2);
    }

    fn period_table(&mut self, c: usize) {
        let t = self.table(self.voices[c].ptab).to_vec();
        let v = &mut self.voices[c];
        v.ptab_counter = v.ptab_counter.wrapping_sub(1);
        if v.ptab_counter != 0 || t.is_empty() {
            return;
        }
        let at = |i: u8| t.get(usize::from(i)).copied().unwrap_or(0xFF);
        let mut guard = 0;
        while at(v.ptab_pos) == 0xFF && guard < 64 {
            v.ptab_pos = at(v.ptab_pos.wrapping_add(1));
            guard += 1;
        }
        v.ptab_cmd = at(v.ptab_pos);
        v.ptab_counter = at(v.ptab_pos.wrapping_add(1));
        v.ptab_pos = v.ptab_pos.wrapping_add(2);
    }

    /// The period table's command, every tick: an octave up or down, a
    /// shift by semitones (its ticks byte), or a step up or down.
    fn period_command(&mut self, c: usize) {
        let v = &mut self.voices[c];
        match v.ptab_cmd {
            0xFE => v.period = v.period.wrapping_shl(1),
            0x7F => v.period >>= 1,
            0x7E => {
                let mut k = v.ptab_counter;
                v.ptab_counter = 1;
                let up = k & 0x80 != 0;
                k &= 0x7F;
                if k >= 13 {
                    let oct = u32::from(k / 12) & 7;
                    v.period = if up { v.period.wrapping_shl(oct) } else { v.period >> oct };
                    k %= 12;
                }
                let (a, b) = RATIOS[usize::from(k)];
                let (num, den) = if up { (a, b) } else { (b, a) };
                let x = u32::from(v.period) * u32::from(num);
                v.period = (x / u32::from(den)) as u16;
                if up && x % u32::from(den) != 0 {
                    v.period = v.period.wrapping_add(1);
                }
            }
            x if x < 0x7F => v.period = v.period.wrapping_add(u16::from(x)),
            x => v.period = v.period.wrapping_sub(u16::from(x & 0x7F)),
        }
    }

    fn track(&mut self, c: usize) {
        let empty = self.song.empty;
        for _ in 0..4096 {
            let v = &self.voices[c];
            let Item::Track(t) = self.item(v.track).clone() else { return };
            let at = |i: usize| t.get(i).copied().unwrap_or(0x82);
            let cmd = at(v.pos);
            let mut p = v.pos + 1;
            if cmd < 0x80 {
                let per = self.get_period(c, cmd);
                self.voices[c].period = per;
            } else {
                match cmd {
                    0xFF => {
                        let v = &mut self.voices[c];
                        v.volume = 0;
                        v.env_counter = 0;
                        v.ticks = at(p);
                        v.pos = p + 1;
                        v.final_volume = 0;
                        v.env_delta = 0;
                        return;
                    }
                    0x81 => {
                        let v = &mut self.voices[c];
                        v.stack.push(Frame::Call(v.track, p + 1));
                        v.track = usize::from(at(p));
                        v.pos = 0;
                        continue;
                    }
                    0x82 => {
                        let v = &mut self.voices[c];
                        match v.stack.pop() {
                            Some(Frame::Call(k, q)) => (v.track, v.pos) = (k, q),
                            _ => return,
                        }
                        continue;
                    }
                    0x83 => {
                        let v = &mut self.voices[c];
                        v.stack.push(Frame::Loop(p + 2, at(p)));
                        v.pos = p + 2;
                        continue;
                    }
                    0x84 => {
                        let v = &mut self.voices[c];
                        v.pos = p;
                        if let Some(Frame::Loop(q, n)) = v.stack.pop() {
                            let n = n.wrapping_sub(1);
                            if n != 0 {
                                v.stack.push(Frame::Loop(q, n));
                                v.pos = q;
                            }
                        }
                        continue;
                    }
                    0x85 => {
                        let (s1, s2) = (self.sound(usize::from(at(p))), self.sound(usize::from(at(p + 1))));
                        let v = &mut self.voices[c];
                        v.sample1 = s1;
                        v.sample2 = s2;
                        if let Some(st) = self.paula.stepper_mut(c) {
                            st.offset = 0;
                        }
                        self.voices[c].pos = p + 2;
                        continue;
                    }
                    0x86 => {
                        let v = &mut self.voices[c];
                        v.env = Some(usize::from(at(p)));
                        v.env_pos = 0;
                        v.env_counter = 1;
                        v.pos = p + 1;
                        continue;
                    }
                    0x87 => {
                        let v = &mut self.voices[c];
                        v.ptab = Some(usize::from(at(p)));
                        v.ptab_pos = 0;
                        v.ptab_counter = 1;
                        v.pos = p + 1;
                        continue;
                    }
                    0x88 => {
                        let mode = at(p + 1);
                        let v = &mut self.voices[c];
                        v.wtab = Some(usize::from(at(p)));
                        v.wtab_pos = 0;
                        v.wave_counter = 1;
                        v.pos = p + 2;
                        let old = v.mode;
                        v.mode = mode;
                        if mode & FREQ_MAPPED != 0 {
                            self.set_length(c, 128);
                            let st = Stepper { base: empty, offset: 0, step: 128, limit: 0, len: 128, empty, stopped: true };
                            if old & FREQ_MAPPED == 0 {
                                self.paula.set_stepper(c, Some(st));
                            }
                        } else {
                            v.wave_pos = mode & 0x1F;
                            v.wave_start = v.wave_pos;
                            if old & FREQ_MAPPED != 0 {
                                if self.paula.stepper_mut(c).is_some_and(|s| s.stopped) {
                                    v.sample1 = None;
                                }
                                v.wave_step = 0;
                                v.wave_start = 0;
                                self.paula.set_stepper(c, None);
                                let b = self.voices[c].buffers[0];
                                self.voices[c].len = 32;
                                self.set_address(c, b);
                            }
                        }
                        continue;
                    }
                    0x89 => {
                        let (from, to, n) = (self.get_period(c, at(p)), self.get_period(c, at(p + 1)), at(p + 2));
                        let v = &mut self.voices[c];
                        v.period = from;
                        let mut delta = i32::from(to) - i32::from(from);
                        v.porta_down = delta < 0;
                        delta = delta.abs();
                        let step = (delta / i32::from(n).max(1)).max(1);
                        v.porta_step = step as u8;
                        v.porta_delay = (i32::from(n) / delta.max(1)).max(1) as u8;
                        v.porta_counter = 1;
                        v.porta_duration = n;
                        p += 2;
                    }
                    0x8A => {
                        let v = &mut self.voices[c];
                        v.transpose = at(p) as i8;
                        v.pos = p + 1;
                        continue;
                    }
                    0x8B => {
                        let v = &mut self.voices[c];
                        v.pos = (usize::from(at(p)) << 8) | usize::from(at(p + 1));
                        self.looped |= 1 << c;
                        if self.looped == 0x0F {
                            self.looped = 0;
                            self.done = true;
                        }
                        continue;
                    }
                    0x8C => {
                        let v = &mut self.voices[c];
                        v.reset = at(p);
                        v.pos = p + 1;
                        continue;
                    }
                    0x8D => {
                        let v = &mut self.voices[c];
                        v.mask = at(p);
                        v.pos = p + 1;
                        continue;
                    }
                    _ => {
                        self.voices[c].pos = p;
                        return;
                    }
                }
            }
            // A note (or a portamento's): held its ticks, its tables restarted
            // unless kept.
            if let Some(st) = self.paula.stepper_mut(c) {
                st.stopped = false;
            }
            let v = &mut self.voices[c];
            v.ticks = at(p);
            v.pos = p + 1;
            if v.reset & KEEP_WAVES == 0 {
                v.wtab_pos = 0;
                v.wave_counter = 1;
                v.wave_pos = v.wave_start;
            }
            if v.reset & KEEP_VOLUME == 0 {
                v.env_pos = 0;
                v.env_counter = 1;
            }
            if v.reset & KEEP_PERIODS == 0 {
                v.ptab_pos = 0;
                v.ptab_counter = 1;
            }
            return;
        }
    }

    /// The waveform table's next step, when its ticks are up: `width`
    /// bytes, whose first is the step (an entry for which `jump` says so
    /// jumps to the place the next byte gives).
    fn wave_step(&mut self, c: usize, width: usize, jump: fn(u8) -> bool) -> Option<Option<Vec<u8>>> {
        let t = self.table(self.voices[c].wtab).to_vec();
        let v = &mut self.voices[c];
        v.wave_counter = v.wave_counter.wrapping_sub(1);
        if v.wave_counter != 0 {
            return Some(None);
        }
        if t.is_empty() {
            return None;
        }
        let at = |i: u8| t.get(usize::from(i)).copied().unwrap_or(0);
        let mut guard = 0;
        while jump(at(v.wtab_pos)) && guard < 64 {
            v.wtab_pos = at(v.wtab_pos.wrapping_add(1));
            guard += 1;
        }
        let e: Vec<u8> = (0..width).map(|k| at(v.wtab_pos.wrapping_add(k as u8))).collect();
        v.wtab_pos = v.wtab_pos.wrapping_add(width as u8);
        Some(Some(e))
    }

    /// The buffer filled last tick plays next; the other is filled.
    fn swap_buffers(&mut self, c: usize) -> (usize, usize) {
        let v = &mut self.voices[c];
        let playing = v.buffers[v.use_buffer];
        v.use_buffer ^= 1;
        let fill = v.buffers[v.use_buffer];
        self.set_address(c, playing);
        (playing, fill)
    }

    /// Ring modulation: the two averaged, the second turning round by the
    /// step each tick, the result's bits ORed with the mask.
    fn ring(&mut self, c: usize) {
        match self.wave_step(c, 2, |x| x & 0x80 != 0) {
            None => return,
            Some(Some(e)) => {
                let v = &mut self.voices[c];
                v.wave_step = e[0];
                v.wave_counter = e[1];
            }
            Some(None) => {}
        }
        let v = &mut self.voices[c];
        v.wave_pos = v.wave_pos.wrapping_add(v.wave_step) & 0x1F;
        let (_, fill) = self.swap_buffers(c);
        let v = &self.voices[c];
        let (Some((s1, _)), Some((s2, _))) = (v.sample1, v.sample2) else { return };
        let (mut j, mask) = (usize::from(v.wave_pos), v.mask);
        let mem = &mut self.paula.mem;
        for i in 0..32 {
            let x = ((i16::from(mem[s1 + i]) + i16::from(mem[s2 + j])) >> 1) as u8;
            let y = if x & 0x80 != 0 { (-((-i32::from(x as i8)) | i32::from(mask))) as u8 } else { x | mask };
            mem[fill + i] = y as i8;
            j = (j + 1) & 0x1F;
        }
    }

    /// The span from the position, the step's low bits long (wrapping), as
    /// it played or (`f` for the points in the span) remade.
    fn span(&mut self, c: usize, playing: usize, fill: usize, f: impl Fn(usize, i8) -> i8) {
        let v = &self.voices[c];
        let pos = usize::from(v.wave_pos);
        let mut switch = (usize::from(v.wave_step & 0x1F)) + pos;
        let mut on = false;
        if switch & 0x20 != 0 {
            on = true;
            switch &= 0x1F;
        }
        for i in 0..32 {
            let x = self.paula.mem[playing + i];
            if i == pos {
                on = !on;
            }
            if i == switch {
                on = !on;
            }
            self.paula.mem[fill + i] = if on { f(i, x) } else { x };
        }
    }

    /// XOR: the second's points (masked) XORed into the span; a step
    /// with its top bit set starts again from the first.
    fn xor(&mut self, c: usize) {
        match self.wave_step(c, 2, |x| x == 0xFF) {
            None => return,
            Some(Some(e)) => {
                let v = &mut self.voices[c];
                v.wave_step = e[0];
                v.wave_counter = e[1];
            }
            Some(None) => {}
        }
        let (playing, fill) = self.swap_buffers(c);
        let v = &self.voices[c];
        let (Some((s1, _)), Some((s2, _))) = (v.sample1, v.sample2) else { return };
        if v.wave_step & 0x80 != 0 {
            for i in 0..32 {
                self.paula.mem[fill + i] = self.paula.mem[s1 + i];
            }
            return;
        }
        let mask = v.mask as i8;
        let second: Vec<i8> = self.paula.mem[s2..s2 + 32].to_vec();
        self.span(c, playing, fill, |i, x| x ^ (second[i] & mask));
        let v = &mut self.voices[c];
        v.wave_pos = v.wave_step.wrapping_add(v.wave_pos) & 0x1F;
    }

    /// Morphing: the span's points moved towards one of the two by the
    /// speed; 0x80 starts again from the first.
    fn morph(&mut self, c: usize) {
        match self.wave_step(c, 3, |x| x == 0xFF) {
            None => return,
            Some(Some(e)) => {
                let v = &mut self.voices[c];
                v.wave_step = e[0];
                v.morph_speed = e[1];
                v.wave_counter = e[2];
            }
            Some(None) => {}
        }
        let (playing, fill) = self.swap_buffers(c);
        let v = &self.voices[c];
        let (Some((s1, _)), Some((s2, _))) = (v.sample1, v.sample2) else { return };
        if v.wave_step == 0x80 {
            for i in 0..32 {
                self.paula.mem[fill + i] = self.paula.mem[s1 + i];
            }
            return;
        }
        let speed = v.morph_speed;
        let src = if v.wave_step & 0xC0 == 0x40 { s1 } else { s2 };
        let goal: Vec<i8> = self.paula.mem[src..src + 32].to_vec();
        self.span(c, playing, fill, |i, x| {
            let a = (x as u8).wrapping_sub(0x80);
            let b = (goal[i] as u8).wrapping_sub(0x80);
            let diff = b.wrapping_sub(a);
            if b < a {
                if speed >= diff.wrapping_neg() {
                    goal[i]
                } else {
                    a.wrapping_add(0x80).wrapping_sub(speed) as i8
                }
            } else if speed >= diff {
                goal[i]
            } else {
                a.wrapping_add(0x80).wrapping_add(speed) as i8
            }
        });
        let v = &mut self.voices[c];
        if v.wave_step & 0xC0 != 0xC0 {
            v.wave_pos = v.wave_step.wrapping_add(v.wave_pos) & 0x1F;
        }
    }

    /// Frequency mapping: a sample stepped through 128 points a block,
    /// from the table's offsets; with the length kept, each block's step
    /// grows with the period against the mask's note.
    fn freq_mapped(&mut self, c: usize) {
        let e = match self.wave_step(c, 3, |x| x == 0xFF) {
            None => return,
            Some(e) => e,
        };
        let v = &mut self.voices[c];
        if let Some(e) = &e {
            v.sample1 = v.sample2;
            v.wave_counter = e[2];
        }
        let (step, start) = if v.mode & FREQ_LENGTH != 0 {
            let p = u32::from(period(usize::from(v.mask))).max(1);
            let np = u32::from(v.period);
            let r = ((np % p) * 128 / p + (np / p) * 128) as u16;
            ((r >> 8) as u8, r as u8)
        } else {
            (0, 128)
        };
        v.wave_step = step;
        v.wave_start = start;
        // The interrupt plays the first sound, as far as the second's length.
        let base = v.sample1.map_or(self.song.empty, |s| s.0);
        let limit = v.sample2.map_or(0, |s| s.1);
        if let Some(st) = self.paula.stepper_mut(c) {
            st.base = base;
            st.limit = limit;
            st.step = (usize::from(step) << 8) | usize::from(start);
            if let Some(e) = e {
                st.offset = (usize::from(e[0]) << 8) | usize::from(e[1]);
                st.stopped = false;
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
            self.voice(c);
        }
    }

    fn tick_seconds(&self) -> f64 {
        // As UADE's player times it: 14,168 of the CIA's clocks a tick, 50.07
        // a second (measured on five songs over a minute; its player sets
        // no timer that can be read).
        14_168.0 / 709_379.0
    }

    fn render(&mut self, rate: u32, frames: usize, out: &mut Bufs) {
        self.paula.render(rate, frames, out);
    }

    fn position(&self) -> (usize, usize) {
        let (grain, rows) = self.rows;
        let r = (self.voices[0].played.saturating_sub(1) / grain) % rows;
        (r / 64, r % 64)
    }

    fn speed(&self) -> (u32, u32) {
        (self.rows.0 as u32, 125)
    }

    fn take_done(&mut self) -> bool {
        std::mem::take(&mut self.done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods_are_an_octave_shifted() {
        assert_eq!(period(0), 32256);
        assert_eq!(period(10), 18048);
        assert_eq!(period(11), 17689);
        assert_eq!(period(12), 16128);
        assert_eq!(period(95), 133);
    }
}
