// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! SidMon 1 (Reinier van Vliet's): a song ripped with its replayer, its
//! data found by the replayer's own instructions that point at it. Each
//! channel steps through its sequence of tracks (each transposed), each
//! track's rows lasting their own durations; instruments play a 32-point
//! waveform (stepped through a list of them) or a sample, under an
//! attack, decay, sustain and release, with a 16-step arpeggio, a phase
//! shift (vibrato by a waveform), a pitch fall and bends. Two waveforms
//! can be remade every so often as a mix of two others, and one inverted
//! a point at a time (the "filter"). The periods are the replayer's own
//! table, read from the file.

use std::sync::Arc;

use crate::paula::{Paula, CIA_50};
use crate::replay::{Bufs, Driver, Tune};

#[derive(Clone, Copy, Debug, Default)]
pub struct Row {
    /// 0 none, -1 off, else a note.
    pub note: i8,
    pub instrument: u8,
    pub effect: u8,
    pub param: u8,
    /// Rows it lasts after its own.
    pub duration: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Step {
    pub track: usize,
    pub transpose: i8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Instrument {
    /// 1 to 15 a waveform list; 16 on a sample (its number + 16).
    pub waveform: u32,
    pub arpeggio: [u8; 16],
    pub attack_speed: u8,
    pub attack_max: u8,
    pub decay_speed: u8,
    pub decay_min: u8,
    pub sustain: u8,
    pub release_speed: u8,
    pub release_min: u8,
    pub phase_shift: u8,
    pub phase_speed: u8,
    pub finetune: u8,
    pub pitch_fall: i8,
    /// A fixed volume (0 to 64; 0 for the envelope's).
    pub volume: u16,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Mix {
    pub source1: u32,
    pub source2: u32,
    pub dest: u32,
    pub speed: u32,
}

/// A sample, where Paula has it.
#[derive(Clone, Debug, Default)]
pub struct Sample {
    pub name: String,
    pub at: usize,
    pub len: usize,
    /// Where it loops from, or none.
    pub loop_start: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Song {
    pub sequences: [Vec<Step>; 4],
    pub tracks: Vec<Vec<Row>>,
    pub instruments: Vec<Instrument>,
    /// 16 bytes each: waveform, speed pairs; FF jumps.
    pub wave_lists: Vec<[u8; 16]>,
    /// Where each 32-point waveform is in memory.
    pub waveforms: Vec<usize>,
    /// By an instrument's waveform number - 16.
    pub samples: Vec<Sample>,
    pub periods: Vec<u16>,
    pub finetune_step: usize,
    pub filter: bool,
    pub mixing: bool,
    pub mixes: [Mix; 2],
    pub rows: u32,
    pub speed: u32,
    /// The speed counter as the rip saved it: the replayer's start leaves
    /// it so (a song saved on 0 reads its first row a tick early, which
    /// gives its first position a row more).
    pub counter: u32,
    pub mem: Vec<i8>,
    file: Arc<Vec<u8>>,
}

fn be16(d: &[u8], i: usize) -> Option<i32> {
    Some(i32::from(*d.get(i)?) << 8 | i32::from(*d.get(i + 1)?))
}

fn sbe16(d: &[u8], i: usize) -> Option<i32> {
    Some(i32::from(*d.get(i)? as i8) << 8 | i32::from(*d.get(i + 1)?))
}

fn be32(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?))
}

fn at2(d: &[u8], i: usize, a: u8, b: u8) -> bool {
    d.get(i) == Some(&a) && d.get(i + 1) == Some(&b)
}

/// Where the replayer's instructions say the data is.
#[derive(Default, Debug)]
struct Found {
    base: i64,
    sequences: [i64; 4],
    track_data: i64,
    track_table: i64,
    instruments: i64,
    wave_list: i64,
    waveforms: i64,
    samples: i64,
    song_data: i64,
    filter: bool,
    mixing: bool,
    /// The older replayer's samples, named in its code: (instrument, offset, length).
    special: Option<Vec<(u32, i64, usize)>>,
    no_loop: u32,
    finetune_step: Option<usize>,
    /// Where the replayer keeps its speed counter.
    counter: usize,
}

/// Follows the init routine (LEA base, ADDA offsets for the sequences,
/// then the tracks' and instruments').
fn from_init(b: &[u8], f: &mut Found) -> Option<()> {
    let n = b.len();
    let mut i = (0..n.saturating_sub(2)).step_by(2).find(|&i| at2(b, i, 0x41, 0xFA))?;
    if i >= n - 4 {
        return None;
    }
    f.base = i64::from(be16(b, i + 2)?) + i as i64 + 2;
    i += 4;
    let mut ch = 0;
    while i < n - 2 && ch < 4 {
        if at2(b, i, 0xD1, 0xE8) {
            f.sequences[ch] = i64::from(sbe16(b, i + 2)?) + f.base;
            ch += 1;
        }
        i += 2;
    }
    if i >= n - 4 {
        return None;
    }
    while i < n - 2 {
        if at2(b, i, 0x70, 0x03) || (at2(b, i, 0x20, 0x3C) && b.get(i + 5) == Some(&0x03)) {
            break;
        }
        i += 2;
    }
    if i >= n - 4 {
        return None;
    }
    if b[i] == 0x20 {
        i += 4;
    }
    if !at2(b, i + 2, 0x41, 0xFA) || !at2(b, i + 6, 0xD1, 0xE8) {
        return None;
    }
    f.track_data = i64::from(sbe16(b, i + 8)?) + f.base;
    i += 10;
    let next = |i: &mut usize| -> Option<i64> {
        while *i < n - 2 && !at2(b, *i, 0xD9, 0xEC) {
            *i += 2;
        }
        if *i >= n - 4 {
            return None;
        }
        let v = i64::from(sbe16(b, *i + 2)?);
        *i += 4;
        Some(v)
    };
    f.track_table = next(&mut i)? + f.base;
    f.instruments = next(&mut i)? + f.base;
    Some(())
}

/// Follows the play routine: its note routine, the song data, the filter
/// and mixing it calls, the waveforms, the period table's fine-tune
/// stride and the samples.
fn from_play(b: &[u8], f: &mut Found) -> Option<()> {
    let n = b.len();
    let mut i = (0..n.saturating_sub(4)).step_by(2).find(|&i| b[i..i + 4] == [0x48, 0xE7, 0xFF, 0xFE])?;
    while i < n - 4 && !at2(b, i, 0x61, 0x00) {
        i += 2;
    }
    if i >= n - 4 {
        return None;
    }
    let play_note = (i64::from(sbe16(b, i + 2)?) + i as i64 + 2) as usize;
    // After the four channels' calls, the speed counter: LEA to it, then one added.
    let mut k = i;
    let mut calls = 1;
    while calls < 4 && k < n - 4 {
        k += 2;
        if at2(b, k, 0x61, 0x00) {
            calls += 1;
        }
    }
    let lea = (k + 4..(k + 64).min(n - 4)).step_by(2).find(|&m| at2(b, m, 0x41, 0xFA))?;
    f.counter = (i64::from(sbe16(b, lea + 2)?) + lea as i64 + 2) as usize;
    while i < n - 4 && !at2(b, i, 0xD3, 0xE9) {
        i += 2;
    }
    if i >= n - 4 {
        return None;
    }
    f.song_data = i64::from(sbe16(b, i + 2)?) + f.base;
    while i < n - 4 && !at2(b, i, 0x4C, 0xDF) {
        i += 2;
    }
    if i >= n - 4 {
        return None;
    }
    let mut i = i.checked_sub(4)?;
    if at2(b, i, 0x00, 0xDF) || at2(b, i, 0x4E, 0x71) {
        i = i.checked_sub(6)?;
    }
    for _ in 0..3 {
        if !at2(b, i, 0x61, 0x00) {
            break;
        }
        let c = (i64::from(sbe16(b, i + 2)?) + i as i64 + 2) as usize;
        if at2(b, c + 14, 0x44, 0x10) {
            f.filter = true;
        } else if at2(b, c + 12, 0x20, 0x10) || at2(b, c + 14, 0x20, 0x10) {
            f.mixing = true;
        }
        i = i.checked_sub(4)?;
    }
    // The waveform list, the waveforms.
    let mut i = play_note;
    while i < n && !at2(b, i, 0xD9, 0xEC) {
        i += 2;
    }
    if i < n - 4 {
        f.wave_list = i64::from(sbe16(b, i + 2)?) + f.base;
    } else {
        i = play_note;
    }
    while i < n && !(b.get(i) == Some(&0xEB) && matches!(b.get(i + 1), Some(0x81 | 0x49))) {
        i += 2;
    }
    if i >= n - 4 {
        return None;
    }
    while i < n && !at2(b, i, 0xDD, 0xEE) {
        i += 2;
    }
    if i >= n - 4 {
        return None;
    }
    f.waveforms = i64::from(sbe16(b, i + 2)?) + f.base;
    // The fine-tune: MOVE.B (fine,A4),D3 then MULU #step (67), or another way to 68.
    let mut k = i;
    while k < n && !at2(b, k, 0x16, 0x2C) {
        k += 2;
    }
    f.finetune_step = if k >= n - 4 {
        None
    } else if at2(b, k + 4, 0xC6, 0xFC) {
        Some(67)
    } else {
        Some(68)
    };
    // The routine that plays a sample.
    while i >= 4 && !at2(b, i, 0x64, 0x00) {
        i -= 2;
    }
    if i < 4 {
        return None;
    }
    if at2(b, i - 2, 0x00, 0x3C) {
        let mut i = (i64::from(sbe16(b, i + 2)?) + i as i64 + 2) as usize;
        let mut special = Vec::new();
        while i < n && !at2(b, i, 0x4E, 0x75) {
            if at2(b, i, 0x0C, 0x00) {
                let ins = be16(b, i + 2)? as u32;
                i += 8;
                if !at2(b, i, 0x4D, 0xFA) {
                    return None;
                }
                let mut start = i64::from(sbe16(b, i + 2)?) + i as i64 + 2;
                if at2(b, i + 4, 0xDD, 0xFC) {
                    start += i64::from(be32(b, i + 6)?);
                    i += 6;
                }
                i += 10;
                if !at2(b, i, 0x33, 0xFC) {
                    return None;
                }
                let len = be16(b, i + 2)? as usize * 2;
                special.push((ins, start, len));
            }
            i += 2;
        }
        if i >= n - 4 {
            return None;
        }
        f.special = Some(special);
    } else {
        let mut i = (i64::from(sbe16(b, i + 2)?) + i as i64 + 2) as usize;
        while i < n && !at2(b, i, 0xD9, 0xEC) {
            i += 2;
        }
        if i >= n - 4 {
            return None;
        }
        f.samples = i64::from(sbe16(b, i + 2)?) + f.base;
        while i < n && !at2(b, i, 0x0C, 0xAB) {
            i += 2;
        }
        if i >= n - 4 {
            return None;
        }
        f.no_loop = be32(b, i + 2)?;
    }
    Some(())
}

fn find(d: &[u8]) -> Option<Found> {
    if d.len() < 1024 || d.starts_with(b"SC68") {
        return None;
    }
    let b = &d[..d.len().min(4096)];
    let mut f = Found::default();
    from_init(b, &mut f)?;
    from_play(b, &mut f)?;
    Some(f)
}

/// The file is a SidMon 1 song (its replayer's instructions say so).
pub fn probe(d: &[u8]) -> bool {
    find(d).is_some()
}

/// The replayer's period table in the file: it starts on 5760 (or, with
/// no fine-tunes, 6848), its fine-tunes one after another.
fn period_table(d: &[u8], step: Option<usize>) -> Option<Vec<u16>> {
    let start: &[u16] = if step.is_some() { &[5760, 5424, 5120, 4832, 4560] } else { &[6848, 6464, 6096, 5760, 5424] };
    let pat: Vec<u8> = start.iter().flat_map(|v| v.to_be_bytes()).collect();
    let at = d.windows(pat.len()).position(|w| w == pat.as_slice())?;
    let count = match step {
        Some(67) => 469,
        Some(_) => 480,
        None => 82,
    };
    // The table counts from 1 (0 is no note).
    let mut t = vec![0u16];
    t.extend((0..count).map(|k| be16(d, at + k * 2).unwrap_or(0) as u16));
    Some(t)
}

impl Song {
    pub fn parse(d: &[u8]) -> Option<Song> {
        let mut f = find(d)?;
        let rd = |o: i64| -> Option<i64> { Some(i64::from(be32(d, usize::try_from(o).ok()?)? as i32) + f.base) };
        for s in f.sequences.iter_mut() {
            *s = rd(*s)?;
        }
        f.track_data = rd(f.track_data)?;
        f.track_table = rd(f.track_table)?;
        f.instruments = rd(f.instruments)?;
        if f.wave_list != 0 {
            f.wave_list = rd(f.wave_list)?;
        }
        f.waveforms = rd(f.waveforms)?;
        if f.samples != 0 {
            f.samples = rd(f.samples)?;
        }
        f.song_data = rd(f.song_data)?;
        let mut sorted = vec![f.sequences[0], f.sequences[1], f.sequences[2], f.sequences[3], f.track_data, f.track_table, f.instruments, f.wave_list, f.waveforms, f.samples, f.song_data];
        sorted.sort_unstable();
        let after = |o: i64| -> Option<i64> {
            let k = sorted.iter().position(|&x| x == o)?;
            sorted.get(k + 1).map(|&x| x - o)
        };
        let ntracks = match after(f.track_table) {
            Some(gap) => (gap / 4) as usize,
            None => {
                // The last block: its offsets counted until one's 0 or past the file.
                let mut k = 1;
                while let Some(o) = be32(d, f.track_table as usize + k * 4) {
                    if o == 0 || o as usize > d.len() {
                        break;
                    }
                    k += 1;
                }
                k
            }
        };
        let ninstr = ((after(f.instruments)? / 32) as usize).min(63);
        let nlists = after(f.wave_list).map_or(0, |g| (g / 16).max(0) as usize);
        let nwaves = after(f.waveforms).map_or(0, |g| (g / 32).max(0) as usize);
        // The song's settings.
        let sd = f.song_data as usize;
        let w = |k: usize| be32(d, sd + k * 4);
        let mixes = [Mix { source1: w(0)?, source2: w(2)?, dest: w(4)?, speed: w(9)? }, Mix { source1: w(1)?, source2: w(3)?, dest: w(5)?, speed: w(10)? }];
        let rows = w(6)?;
        let npos = (w(7)?).saturating_sub(1) as usize;
        let speed = w(8)?;
        // Instruments.
        let nspecial = f.special.as_ref().map_or(0, |s| s.len());
        let mut instruments = Vec::with_capacity(ninstr + nspecial);
        for k in 0..ninstr {
            let o = f.instruments as usize + k * 32;
            let b = d.get(o..o + 32)?;
            let mut ins = Instrument { waveform: be32(b, 0)?, ..Default::default() };
            ins.arpeggio.copy_from_slice(&b[4..20]);
            ins.attack_speed = b[20];
            ins.attack_max = b[21];
            ins.decay_speed = b[22];
            ins.decay_min = b[23];
            ins.sustain = b[24];
            ins.release_speed = b[26];
            ins.release_min = b[27];
            ins.phase_shift = b[28];
            ins.phase_speed = b[29];
            ins.finetune = b[30];
            ins.pitch_fall = b[31] as i8;
            // The older replayer keeps its pitch fall where the fine-tune is.
            if nspecial != 0 {
                ins.pitch_fall = ins.finetune as i8;
                ins.finetune = 0;
            } else if ins.finetune > 6 {
                ins.finetune = 0;
            }
            instruments.push(ins);
        }
        if let Some(special) = &f.special {
            for &(ins, _, _) in special {
                instruments.push(Instrument { waveform: ins.wrapping_sub(60).wrapping_add(16), volume: 64, ..Default::default() });
            }
        }
        // Tracks.
        let mut tracks = Vec::with_capacity(ntracks);
        for k in 0..ntracks {
            let off = be32(d, f.track_table as usize + k * 4)? as i32 as i64;
            let mut at = usize::try_from(f.track_data + off).ok()?;
            let mut rows_ = Vec::new();
            let mut line = 0usize;
            while line < 64 {
                let b = d.get(at..at + 5)?;
                let mut r = Row { note: b[0] as i8, instrument: b[1], effect: b[2], param: b[3], duration: b[4] };
                if nspecial != 0 && r.instrument >= 60 {
                    r.instrument = (instruments.len() - nspecial + usize::from(r.instrument - 60)) as u8;
                }
                rows_.push(r);
                line += usize::from(r.duration) + 1;
                at += 5;
            }
            tracks.push(rows_);
        }
        if tracks.is_empty() {
            return None;
        }
        // Sequences.
        let mut sequences: [Vec<Step>; 4] = Default::default();
        for (c, seq) in sequences.iter_mut().enumerate() {
            let at = f.sequences[c] as usize;
            for k in 0..npos {
                let o = at + k * 6;
                let t = be32(d, o)? as usize;
                seq.push(Step { track: t.min(tracks.len() - 1), transpose: *d.get(o + 5)? as i8 });
            }
        }
        if sequences[0].is_empty() {
            return None;
        }
        // Waveforms (and their lists).
        let mut mem: Vec<i8> = Vec::new();
        let wave_lists: Vec<[u8; 16]> = if f.wave_list == 0 {
            (1..=instruments.len()).map(|i| [i as u8, 0x01, 0xFF, 0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).collect()
        } else {
            (0..nlists).map(|k| d.get(f.wave_list as usize + k * 16..f.wave_list as usize + k * 16 + 16).and_then(|b| b.try_into().ok())).collect::<Option<_>>()?
        };
        let mut waveforms = Vec::with_capacity(nwaves);
        for k in 0..nwaves {
            let o = f.waveforms as usize + k * 32;
            let b = d.get(o..o + 32)?;
            waveforms.push(mem.len());
            mem.extend(b.iter().map(|&x| x as i8));
        }
        // Samples: the ones the instruments play, by number.
        let mut used: Vec<u32> = instruments.iter().filter(|i| i.waveform >= 16).map(|i| i.waveform - 16).collect();
        used.sort_unstable();
        used.dedup();
        let mut samples: Vec<Sample> = Vec::new();
        if let Some(special) = &f.special {
            for &(_, start, len) in special {
                let at = mem.len();
                let s = usize::try_from(start).unwrap_or(usize::MAX);
                mem.extend(d.get(s..(s + len).min(d.len())).unwrap_or_default().iter().map(|&x| x as i8));
                mem.resize(at + len, 0);
                samples.push(Sample { name: String::new(), at, len, loop_start: None });
            }
        } else if f.samples != 0 {
            let so = f.samples as usize;
            let base = be32(d, so)? as usize;
            let info = so + 4;
            let mut first = if base != 0 { base } else { usize::MAX };
            let last = used.last().copied().unwrap_or(0) as usize;
            for k in 0..=last {
                let mut smp = Sample::default();
                if used.contains(&(k as u32)) && k * 32 < first {
                    let o = info + k * 32;
                    let start = be32(d, o)? as usize;
                    let ls = be32(d, o + 4)? as i32;
                    let end = be32(d, o + 8)? as usize;
                    smp.name = d.get(o + 12..o + 32)?.iter().take_while(|&&c| c != 0).map(|&c| if (32..127).contains(&c) { c as char } else { ' ' }).collect::<String>().trim().to_string();
                    smp.loop_start = if ls as u32 == f.no_loop || ls < 0 || ls as usize >= end || start > ls as usize { None } else { Some(ls as usize - start) };
                    let len = end.saturating_sub(start);
                    let pos = info + base + start;
                    first = first.min(pos);
                    smp.at = mem.len();
                    mem.extend(d.get(pos..(pos + len).min(d.len())).unwrap_or_default().iter().map(|&x| x as i8));
                    mem.resize(smp.at + len, 0);
                    smp.len = len;
                }
                samples.push(smp);
            }
        }
        let periods = period_table(d, f.finetune_step)?;
        let counter = be32(d, f.counter).unwrap_or(speed);
        Some(Song {
            sequences,
            tracks,
            instruments,
            wave_lists,
            waveforms,
            samples,
            periods,
            finetune_step: f.finetune_step.unwrap_or(0),
            filter: f.filter,
            mixing: f.mixing,
            mixes,
            rows,
            speed,
            counter,
            mem,
            file: Arc::new(d.to_vec()),
        })
    }

    pub fn positions(&self) -> usize {
        self.sequences[0].len()
    }

    /// A channel's rows at a position, laid out on its rows: each row and where it starts.
    pub fn rows_at(&self, position: usize, channel: usize) -> Vec<(usize, Row)> {
        let Some(step) = self.sequences[channel].get(position) else { return Vec::new() };
        let mut at = 0;
        let mut out = Vec::new();
        for r in &self.tracks[step.track] {
            out.push((at, *r));
            at += usize::from(r.duration) + 1;
        }
        out
    }

    fn period(&self, index: i32) -> u16 {
        usize::try_from(index).ok().and_then(|i| self.periods.get(i)).copied().unwrap_or(0)
    }
}

impl Tune for Song {
    fn format(&self) -> &'static str {
        "sid1"
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
    seq: usize,
    row: usize,
    track: usize,
    transpose: i8,
    instrument: Option<usize>,
    note_period: u16,
    bend_to: u8,
    bend_speed: i8,
    note_offset: i8,
    arp: usize,
    env: Env,
    row_count: u8,
    volume: u8,
    sustain: u8,
    pitch: u16,
    phase: usize,
    phase_speed: u8,
    pitch_fall: u16,
    wave_index: usize,
    wave_number: u8,
    wave_speed: u8,
    /// Playing a sample (not waveforms).
    sample: bool,
}

pub struct Player {
    song: Arc<Song>,
    paula: Paula,
    /// Instruments as the replayer has them (its notes change their attack).
    instruments: Vec<Instrument>,
    voices: [Voice; 4],
    rows: u32,
    speed: u32,
    counter: u32,
    row: i64,
    position: usize,
    new_track: bool,
    loop_song: bool,
    mix_pos: [usize; 2],
    mix_count: [u32; 2],
    shown: (usize, usize),
    done: bool,
}

impl Player {
    fn new(song: Arc<Song>) -> Player {
        let mut voices: [Voice; 4] = Default::default();
        for (c, v) in voices.iter_mut().enumerate() {
            let s = song.sequences[c][0];
            *v = Voice { note_period: 0x9999, env: Env::Done, track: s.track, transpose: s.transpose, ..Default::default() };
            let ins = usize::from(song.tracks[s.track][0].instrument).wrapping_sub(1);
            if ins < song.instruments.len() {
                v.instrument = Some(ins);
            }
        }
        let paula = Paula::new(4, song.mem.clone());
        Player { instruments: song.instruments.clone(), paula, voices, rows: song.rows, speed: song.speed, counter: song.counter, row: -1, position: 1, new_track: false, loop_song: false, mix_pos: [0; 2], mix_count: [0; 2], shown: (0, 0), done: false, song }
    }

    fn play_note(&mut self, c: usize) {
        let song = self.song.clone();
        let mut source: Option<(usize, usize, Option<usize>)> = None;
        if self.counter == 0 {
            let v = &mut self.voices[c];
            if self.new_track {
                v.row_count = 0;
                v.seq += 1;
                if self.loop_song {
                    v.seq = 0;
                }
                let s = song.sequences[c].get(v.seq).copied().unwrap_or_default();
                v.track = s.track;
                v.transpose = s.transpose;
                v.row = 0;
            }
            if v.row_count == 0 {
                let row = song.tracks[v.track].get(v.row).copied().unwrap_or_default();
                let sample_of = |n: u32| -> Option<(usize, usize, Option<usize>)> {
                    let s = song.samples.get(n.checked_sub(16)? as usize)?;
                    Some((s.at, s.len, s.loop_start))
                };
                if row.instrument != 0 {
                    let k = usize::from(row.instrument - 1);
                    let ins = self.instruments.get(k).copied().unwrap_or_default();
                    if v.sample {
                        self.paula.stop(c);
                        v.sample = false;
                    }
                    let wn = ins.waveform;
                    let mut set = false;
                    if wn >= 16 {
                        if let Some(s) = sample_of(wn) {
                            source = Some(s);
                            set = true;
                            v.sample = true;
                        }
                    }
                    v.wave_index = 0;
                    v.wave_number = wn as u8;
                    if !set && wn != 0 {
                        if let Some(list) = song.wave_lists.get(usize::from(v.wave_number).wrapping_sub(1)) {
                            v.wave_speed = list[1];
                            if let Some(&at) = song.waveforms.get(usize::from(list[0]).wrapping_sub(1)) {
                                source = Some((at, 32, Some(0)));
                            }
                        }
                    }
                    v.instrument = (k < self.instruments.len()).then_some(k);
                    v.env = Env::Attack;
                    v.pitch_fall = 0;
                    v.pitch = 0;
                    v.row_count = row.duration;
                } else if row.note != 0 {
                    v.row_count = row.duration;
                    if v.sample && row.note != -1 {
                        self.paula.stop(c);
                        if let Some(s) = sample_of(u32::from(v.wave_number)) {
                            source = Some(s);
                        }
                    }
                }
                let v = &mut self.voices[c];
                let mut note = row.note;
                if note != 0 {
                    v.row_count = row.duration;
                    if note != -1 {
                        if let Some(k) = v.instrument {
                            let ins = &mut self.instruments[k];
                            note = note.wrapping_add(v.transpose);
                            v.note_period = song.period(i32::from(note) + i32::from(ins.finetune) * song.finetune_step as i32 + 1);
                            v.note_offset = note;
                            v.env = Env::Attack;
                            v.volume = 0;
                            v.sustain = 0;
                            v.pitch = 0;
                            v.pitch_fall = 0;
                            v.bend_speed = 0;
                            v.phase_speed = ins.phase_speed;
                            match row.effect {
                                0 => {
                                    if row.param != 0 {
                                        ins.attack_max = row.param;
                                        ins.attack_speed = row.param;
                                        v.wave_speed = 0;
                                    }
                                }
                                2 => {
                                    self.speed = u32::from(row.param);
                                    v.wave_speed = 0;
                                }
                                3 => {
                                    self.rows = u32::from(row.param);
                                    v.wave_speed = 0;
                                }
                                e => {
                                    v.bend_to = e.wrapping_add(v.transpose as u8);
                                    v.bend_speed = row.param as i8;
                                }
                            }
                        }
                    }
                }
                v.row += 1;
            } else {
                v.row_count -= 1;
            }
        }
        let v = &mut self.voices[c];
        let Some(k) = v.instrument else { return };
        if v.wave_number == 0 {
            return;
        }
        let ins = self.instruments[k];
        let step = song.finetune_step as i32 * i32::from(ins.finetune);
        // The envelope.
        let volume: i32 = if ins.volume != 0 {
            i32::from(ins.volume) * 4
        } else {
            match v.env {
                Env::Attack => {
                    let mut vol = i32::from(v.volume) + i32::from(ins.attack_speed);
                    if vol > i32::from(ins.attack_max) {
                        vol = i32::from(ins.attack_max);
                        v.env = Env::Decay;
                    }
                    v.volume = vol as u8;
                    vol
                }
                Env::Decay => {
                    let mut vol = i32::from(v.volume) - i32::from(ins.decay_speed);
                    if vol <= i32::from(ins.decay_min) {
                        vol = i32::from(ins.decay_min);
                        v.sustain = ins.sustain;
                        v.env = Env::Sustain;
                    }
                    v.volume = vol as u8;
                    vol
                }
                Env::Sustain => {
                    v.sustain = v.sustain.wrapping_sub(1);
                    if v.sustain == 0 {
                        v.env = Env::Release;
                    }
                    i32::from(v.volume)
                }
                Env::Release => {
                    let mut vol = i32::from(v.volume) - i32::from(ins.release_speed);
                    if vol <= i32::from(ins.release_min) {
                        vol = i32::from(ins.release_min);
                        v.env = Env::Done;
                    }
                    v.volume = vol as u8;
                    vol
                }
                Env::Done => 0,
            }
        };
        // The arpeggio.
        v.arp = (v.arp + 1) & 0x0F;
        let p = song.period(i32::from(v.note_offset) + i32::from(ins.arpeggio[v.arp]) + step);
        v.note_period = p;
        let period = u32::from(p);
        // Bends.
        if v.bend_speed != 0 {
            let bs = -i16::from(v.bend_speed);
            let index = i32::from(v.bend_to) + step;
            if usize::try_from(index).map_or(true, |i| i >= song.periods.len()) {
                v.bend_speed = 0;
            } else {
                let to = song.periods[index as usize];
                v.pitch = v.pitch.wrapping_add(bs as u16);
                v.note_period = v.note_period.wrapping_add(v.pitch);
                let past = if bs < 0 { v.note_period < to } else { v.note_period > to };
                if past {
                    v.note_offset = v.bend_to as i8;
                    v.note_period = to;
                    v.pitch = 0;
                    v.bend_speed = 0;
                }
            }
        }
        // The phase shift: the period swung by a waveform.
        if ins.phase_shift != 0 {
            if v.phase_speed == 0 {
                if let Some(&at) = song.waveforms.get(usize::from(ins.phase_shift - 1)) {
                    v.phase = (v.phase + 1) & 0x1F;
                    let w = self.paula.mem[at + v.phase];
                    v.note_period = v.note_period.wrapping_add((i32::from(w) / 4) as i16 as u16);
                }
            } else {
                v.phase_speed -= 1;
            }
        }
        v.pitch_fall = v.pitch_fall.wrapping_sub(ins.pitch_fall as i16 as u16);
        v.note_period = v.note_period.wrapping_add(v.pitch_fall);
        // The waveform list.
        if !v.sample {
            if v.wave_speed == 0 {
                if v.wave_index != 16 {
                    if let Some(list) = song.wave_lists.get(usize::from(v.wave_number) - 1) {
                        if list[v.wave_index] == 0xFF {
                            v.wave_index = usize::from(list[v.wave_index + 1] & 0xFE);
                        } else {
                            if let Some(&at) = song.waveforms.get(usize::from(list[v.wave_index]).wrapping_sub(1)) {
                                source = Some((at, 32, Some(0)));
                            }
                            v.wave_speed = list[v.wave_index + 1];
                            v.wave_index += 2;
                        }
                    }
                }
            } else {
                v.wave_speed -= 1;
            }
        }
        if period != 0 {
            let p = u32::from(v.note_period);
            self.paula.set_period(c, p);
        }
        if volume != 0 {
            self.paula.set_volume_fine(c, volume.clamp(0, 256) as u32);
        }
        if let Some((at, len, ls)) = source {
            self.paula.play(c, at, len);
            if let Some(ls) = ls {
                self.paula.set_loop(c, at + ls, len - ls);
            }
        }
    }

    fn mix(&mut self, k: usize) {
        let song = self.song.clone();
        let m = song.mixes[k];
        if !song.mixing || m.speed == 0 {
            return;
        }
        if self.mix_count[k] == 0 {
            self.mix_count[k] = m.speed;
            self.mix_pos[k] = (self.mix_pos[k] + 1) & 0x1F;
            let n = song.waveforms.len() as u32;
            if m.source1 <= n && m.source2 <= n && m.dest <= n && m.source1 > 0 && m.source2 > 0 && m.dest > 0 {
                let (s1, s2, dst) = (song.waveforms[m.source1 as usize - 1], song.waveforms[m.source2 as usize - 1], song.waveforms[m.dest as usize - 1]);
                let mut pos = self.mix_pos[k];
                for i in (0..32).rev() {
                    let x = (i32::from(self.paula.mem[s1 + i]) + i32::from(self.paula.mem[s2 + pos])) / 2;
                    self.paula.mem[dst + i] = x as i8;
                    pos = pos.wrapping_sub(1) & 0x1F;
                }
            }
        }
        self.mix_count[k] = self.mix_count[k].wrapping_sub(1);
    }
}

impl Driver for Player {
    fn channels(&self) -> usize {
        4
    }

    fn tick(&mut self) {
        for c in 0..4 {
            self.play_note(c);
        }
        self.counter += 1;
        if self.counter > self.speed {
            self.counter = 0;
        }
        self.new_track = false;
        self.loop_song = false;
        if self.counter == 0 {
            self.row += 1;
            if self.row == i64::from(self.rows) {
                self.row = 0;
                self.new_track = true;
                self.position += 1;
                if self.position > self.song.positions() {
                    self.position = 1;
                    self.loop_song = true;
                    self.done = true;
                }
            }
            self.shown = (self.position - 1, self.row.max(0) as usize);
        }
        self.mix(0);
        self.mix(1);
        if self.song.filter {
            if let Some(&at) = self.song.waveforms.first() {
                let p = at + self.mix_pos[0];
                self.paula.mem[p] = self.paula.mem[p].wrapping_neg();
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
        (self.speed + 1, 125)
    }

    fn take_done(&mut self) -> bool {
        std::mem::take(&mut self.done)
    }
}
