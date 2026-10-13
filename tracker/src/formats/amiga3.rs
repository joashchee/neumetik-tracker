//! Still more of the Amiga's trackers: Symphonie and Symphonie Pro
//! (.symmod), Face The Music (.ftm), Fashion Tracker (.ex), Digital
//! Sound Studio (.dss), and StarTrekker's AM synth instruments (a MOD's
//! .nt beside it).

use std::collections::HashMap;
use std::sync::Arc;

use crate::io::{bad, decode, Error, Pcm, Reader, Result};
use crate::model::*;

// ---- Symphonie ----
//
// "SymM", version 1, then a list of hunks, each a negative number (or a
// small positive one for Symphonie Pro's settings) and its value or
// data: the channels, the track length, the tempo, the positions (a
// pattern, a start row, a length, a speed and a transpose each), the
// sequences (runs of positions, each with a transpose), the events (four
// bytes a cell: a command, a note, a parameter, an instrument), the
// instruments (256 bytes each) and the samples (IFF, WAV, AIFF or raw;
// delta-packed in 8 or 16 bits). Blocks may be run-length packed
// ("PACK" FF FF). Its effects run at the output's rate, not by ticks;
// here they're turned into the nearest ticked effects row by row, as
// OpenMPT's loader (BSD) does. Virtual instruments (a mix of others)
// and transwaves (a morph between two loops) are rendered when read.
// Its DSP (echo, delay, the filter) isn't kept.

pub fn probe_symmod(d: &[u8]) -> bool {
    d.len() >= 16
        && &d[..4] == b"SymM"
        && u32::from_be_bytes([d[4], d[5], d[6], d[7]]) == 1
        && i32::from_be_bytes([d[8], d[9], d[10], d[11]]) == -1
        && (1..=256).contains(&u32::from_be_bytes([d[12], d[13], d[14], d[15]]))
}

/// A hunk's block: its length, then the bytes, run-length packed when
/// it starts "PACK" FF FF.
fn sym_block(r: &mut Reader) -> Result<Vec<u8>> {
    let n = r.u32be()? as usize;
    let b = r.bytes_upto(n);
    if b.len() < 10 || &b[..6] != b"PACK\xFF\xFF" {
        return Ok(b.to_vec());
    }
    let want = (u32::from_be_bytes([b[6], b[7], b[8], b[9]]) as usize).min((b.len() - 10).saturating_mul(170)).min(256 << 20);
    let mut out = Vec::with_capacity(want);
    let mut r = Reader::at(b, 10);
    while out.len() < want {
        let Ok(kind) = r.i8() else { break };
        match kind {
            0 => {
                let n = usize::from(r.u8()?);
                out.extend_from_slice(r.bytes(n.min(want - out.len()))?);
            }
            1 => {
                let n = usize::from(r.u8()?);
                let dw = r.bytes(4)?;
                for _ in 0..n {
                    out.extend_from_slice(dw);
                }
            }
            2 => {
                let dw = r.bytes(4)?;
                out.extend_from_slice(dw);
                out.extend_from_slice(dw);
            }
            3 => {
                let n = usize::from(r.u8()?);
                out.resize(out.len() + n, 0);
            }
            _ => break,
        }
    }
    out.truncate(want);
    Ok(out)
}

/// A sample hunk's waveform: IFF, WAV or AIFF, or raw (MAESTRO's
/// 16-bit, "16BT" 16-bit, or 8-bit), each half of a stereo one apart.
fn sym_wave(b: &[u8]) -> Vec<Arc<Wave>> {
    if let Ok(l) = crate::samplefile::read(b, "") {
        if let Some(s) = l.samples.into_iter().next() {
            let w = s.wave;
            if w.channels == 2 {
                let (l, r): (Vec<i16>, Vec<i16>) = w.pcm.chunks_exact(2).map(|f| (f[0], f[1])).unzip();
                return vec![Arc::new(Wave::new(w.bits, 1, l)), Arc::new(Wave::new(w.bits, 1, r))];
            }
            return vec![w];
        }
    }
    if b.len() >= 24 && &b[..7] == b"MAESTRO" {
        let stereo = u32::from_be_bytes([b[12], b[13], b[14], b[15]]) == 0;
        let pcm = decode(&b[24..], Pcm::S16be, (b.len() - 24) / 2);
        if stereo {
            let (l, r): (Vec<i16>, Vec<i16>) = pcm.chunks_exact(2).map(|f| (f[0], f[1])).unzip();
            return vec![Arc::new(Wave::new(16, 1, l)), Arc::new(Wave::new(16, 1, r))];
        }
        return vec![Arc::new(Wave::new(16, 1, pcm))];
    }
    if b.len() >= 4 && &b[..4] == b"16BT" {
        let mut pcm = decode(b, Pcm::S16be, b.len() / 2);
        for v in pcm.iter_mut().take(2) {
            *v = 0;
        }
        return vec![Arc::new(Wave::new(16, 1, pcm))];
    }
    vec![Arc::new(Wave::new(8, 1, decode(b, Pcm::S8, b.len())))]
}

/// One of Symphonie's 256-byte instruments.
struct SymIns<'a>(&'a [u8]);

impl SymIns<'_> {
    fn kind(&self) -> i8 {
        self.0[128] as i8
    }
    fn is_virtual(&self) -> bool {
        &self.0[..4] == b"ViRT"
            && self.0[4] == 0
            && u16::from_be_bytes([self.0[6], self.0[7]]) <= 1
            && u16::from_be_bytes([self.0[12], self.0[13]]) == 0
            && u16::from_be_bytes([self.0[16], self.0[17]]) == 20
    }
    fn virt_version(&self) -> u16 {
        u16::from_be_bytes([self.0[6], self.0[7]])
    }
    fn virt_events(&self) -> u16 {
        u16::from_be_bytes([self.0[14], self.0[15]])
    }
    fn virt_event_size(&self) -> u16 {
        u16::from_be_bytes([self.0[18], self.0[19]])
    }
    fn empty(&self) -> bool {
        self.0[0] == 0 || self.kind() < 0
    }
    fn channel(&self) -> u8 {
        self.0[132]
    }
    fn volume(&self) -> u8 {
        self.0[134]
    }
    fn finetune(&self) -> i8 {
        self.0[138] as i8
    }
    fn transpose(&self) -> i8 {
        self.0[139] as i8
    }
    fn sample_flags(&self) -> u8 {
        self.0[140]
    }
    fn filter(&self) -> i8 {
        self.0[141] as i8
    }
    fn inst_flags(&self) -> u8 {
        self.0[142]
    }
    fn downsample(&self) -> u8 {
        self.0[143]
    }
}

/// The loop a Loop or Sustain instrument sets, from its percentages of
/// the sample (fine ones in the new loop system; else rounded as the
/// Amiga's code did, and the end moved back to where the wave meets its
/// start).
fn sym_loop(ins: &SymIns, w: &Wave) -> (u32, u32) {
    if !matches!(ins.kind(), 4 | 8) {
        return (0, 0);
    }
    let n = w.frames() as u64;
    let (mut start, mut len) = (u64::from(ins.0[129].min(100)), u64::from(ins.0[130].min(100)));
    if ins.sample_flags() & 16 != 0 {
        start = (start << 16) + u64::from(u16::from_be_bytes([ins.0[150], ins.0[151]]));
        len = (len << 16) + u64::from(u16::from_be_bytes([ins.0[152], ins.0[153]]));
        let s = (start * n / (100 << 16)).min(n);
        let l = (len * n / (100 << 16)).min(n - s);
        return (s as u32, l as u32);
    }
    if n == 0 {
        return (0, 0);
    }
    let s = ((start << 7) / 100 * (n >> 7)).min(n);
    let mut l = ((len << 7) / 100 * (n >> 7)).min(n - s);
    let at = |i: u64| i32::from(w.pcm.get(i as usize).copied().unwrap_or(0));
    let first = at(s);
    while l > 0 && (at(s + l) - first).abs() >= 6 * 256 {
        l -= 1;
    }
    (s as u32, l as u32)
}

/// A sample read through a rate change (linear), `step` source frames
/// an output frame.
fn resample_at(w: &Wave, pos: f64) -> f64 {
    let i = pos as usize;
    let f = pos - i as f64;
    let a = f64::from(w.pcm.get(i).copied().unwrap_or(0));
    let b = f64::from(w.pcm.get(i + 1).copied().unwrap_or(a as i16));
    a + (b - a) * f
}

/// A virtual instrument: up to 20 notes of other instruments mixed (or,
/// as a queue, played one after another) into one sample, pitched from
/// the first note's.
fn sym_render_virtual(ins: &SymIns, waves: &[Arc<Wave>], rates: &[f64], queue: bool, boost: u16) -> Option<Wave> {
    let n = usize::from(ins.virt_events()).min(20);
    let ev = |k: usize| {
        let e = &ins.0[20 + k * 4..24 + k * 4];
        (e[0], e[1] as i8, e[2], usize::from(e[3]))
    };
    if n == 0 || ev(0).3 >= waves.len() {
        return None;
    }
    let base_note = f64::from(ev(0).1);
    let base_rate = rates[ev(0).3].max(1.0);
    let mut voices = Vec::new();
    let mut total = 0.0f64;
    let mut longest = 0.0f64;
    for k in 0..if queue { n } else { 1.min(n) } {
        let (_, note, _, src) = ev(k);
        if src >= waves.len() || note < 0 {
            continue;
        }
        total += waves[src].frames() as f64 * 2f64.powf((f64::from(note) - base_note) / -12.0) * rates[src] / base_rate;
    }
    for k in 0..n {
        let (_, note, param, src) = ev(k);
        if src >= waves.len() || note < 0 {
            continue;
        }
        let fine = match param {
            242..=247 => [-4.0, 4.0, -2.0, 2.0, -1.0, 1.0][usize::from(param - 242)],
            _ => 0.0,
        };
        let step = 2f64.powf((f64::from(note) - base_note) / 12.0 + fine / 96.0) * rates[src] / base_rate;
        if step <= 0.0 {
            continue;
        }
        let frames = waves[src].frames() as f64 / step;
        let start = if queue { longest } else { 0.0 };
        if queue {
            longest += frames.round();
        }
        voices.push((src, step, start as usize, frames as usize));
    }
    let len = (total.round() as usize).min(16 << 20);
    if len == 0 {
        return None;
    }
    let mut vol = 4096.0 * f64::from(boost) / 10000.0;
    if !queue {
        vol /= f64::from(ins.virt_events().max(1));
    }
    let mut mix = vec![0f64; len];
    for &(src, step, start, frames) in &voices {
        let w = &waves[src];
        for i in 0..frames.min(len.saturating_sub(start)) {
            mix[start + i] += resample_at(w, i as f64 * step) * vol / 4096.0;
        }
    }
    Some(Wave::new(16, 1, mix.iter().map(|&v| v.round().clamp(-32768.0, 32767.0) as i16).collect()))
}

/// A transwave: 80 cycles of the first loop turning into the second.
fn sym_render_transwave(ins: &SymIns, a: &Wave, b: &Wave) -> Option<Wave> {
    let point = |k: usize, w: &Wave| {
        let p = &ins.0[20 + k * 16..36 + k * 16];
        let n = w.frames() as f64;
        let scale = n / f64::from(100u32 << 16);
        let start = (scale * f64::from(u32::from_be_bytes([p[4], p[5], p[6], p[7]]).min(100 << 16))) as usize;
        let len = (scale * f64::from(u32::from_be_bytes([p[8], p[9], p[10], p[11]]).min(100 << 16))) as usize;
        (start, len.min(w.frames().saturating_sub(start)))
    };
    let (s1, l1) = point(0, a);
    let (s2, l2) = point(1, b);
    if !(1..=(16 << 20) / 320).contains(&l1) {
        return None;
    }
    let cycle = l1 * 4;
    let (f1, f2) = (l1 as f64 / cycle as f64, l2 as f64 / cycle as f64);
    let len = cycle * 80;
    let mut out = vec![0i16; len];
    for i in 0..cycle {
        let v1 = if a.is_empty() { 0.0 } else { resample_at(a, s1 as f64 + i as f64 * f1) };
        let v2 = if b.is_empty() { 0.0 } else { resample_at(b, s2 as f64 + i as f64 * f2) };
        let mut at = i;
        for _ in 0..80 {
            let amp = at as f64 / len as f64;
            out[at] = (v1 * (1.0 - amp) + v2 * amp).round().clamp(-32768.0, 32767.0) as i16;
            at += cycle;
        }
    }
    Some(Wave::new(16, 1, out))
}

/// An instrument's processing of its sample, as Symphonie did it when
/// loading: reversed, inverted, filtered (repeated one-pole passes and a
/// resonant sweep), faded, normalized, over-amplified with a soft knee,
/// and its sustain loop's repeats written out.
fn sym_process(ins: &SymIns, w: &Wave, looped: (u32, u32)) -> (Wave, Loop) {
    let mut pcm = w.pcm.clone();
    let mut bits = w.bits;
    let flags = ins.sample_flags();
    if flags & 1 != 0 {
        pcm.reverse();
    }
    if flags & 4 != 0 {
        for v in pcm.iter_mut() {
            *v = v.saturating_neg();
        }
    }
    let fade = (ins.0[166] == 2 && ins.0[167] <= 100 && ins.0[168] <= 100).then_some((ins.0[167], ins.0[168]));
    let filter_flags = ins.0[156];
    if filter_flags != 0 || fade.is_some() || ins.filter() != 0 {
        bits = 16;
    }
    if ins.filter() < 0 && !pcm.is_empty() {
        for _ in 0..-i32::from(ins.filter()) {
            let mut mix = i32::from(pcm[0]);
            for v in pcm.iter_mut() {
                mix = (i32::from(*v) - (mix >> 1)) >> 1;
                *v = mix as i16;
            }
        }
    }
    if let Some((from, to)) = fade {
        let n = pcm.len().max(1) as f64;
        for (i, v) in pcm.iter_mut().enumerate() {
            let amp = f64::from(from) + (f64::from(to) - f64::from(from)) * i as f64 / n;
            *v = (f64::from(*v) * amp / 100.0).round() as i16;
        }
    }
    if filter_flags != 0 && !pcm.is_empty() {
        let pts = ins.0[157];
        let (c0, r0) = (i64::from(ins.0[158]), i64::from(ins.0[159]));
        let (c1, r1) = if pts > 1 { (i64::from(ins.0[160]), i64::from(ins.0[161])) } else { (c0, r0) };
        let n = pcm.len() as i64;
        let high = usize::from(filter_flags & 2 != 0) * 2;
        let mut st = [0i64; 3];
        for (i, v) in pcm.iter_mut().enumerate() {
            let cutoff = c0 + (c1 - c0) * i as i64 / n;
            let reso = r0 + (r1 - r0) * i as i64 / n;
            st[2] = (i64::from(*v) >> 1) - st[0];
            st[1] += (cutoff * st[2]) >> 8;
            st[0] += (cutoff * st[1]) >> 6;
            st[0] += (reso * st[0]) >> 6;
            st[0] >>= 2;
            *v = st[if high != 0 { 1 } else { 0 }].clamp(-32768, 32767) as i16;
        }
    }
    if ins.filter() > 0 && !pcm.is_empty() {
        for _ in 0..ins.filter() {
            let mut mix = i32::from(pcm[0]);
            for v in pcm.iter_mut() {
                mix = (2 * i32::from(*v) + mix) / 3;
                *v = mix as i16;
            }
        }
    }
    // Normalized to the whole range of its depth.
    let peak = pcm.iter().map(|&v| i32::from(v).abs()).max().unwrap_or(0);
    let full = if bits == 8 { 127 << 8 } else { 32767 };
    if peak > 0 && peak < full {
        for v in pcm.iter_mut() {
            *v = (i32::from(*v) * full / peak) as i16;
        }
    }
    let vol = ins.volume();
    if vol > 100 && vol < 200 {
        let knee = 16384 * (200 - i32::from(vol)) / 100;
        let inv = 32768 - knee;
        for s in pcm.iter_mut() {
            let v = i32::from(*s);
            let v = if v > knee {
                (v - knee) * knee / inv + inv
            } else if v < -knee {
                (v + knee) * knee / inv - inv
            } else {
                v * inv / knee
            };
            *s = v.clamp(-32768, 32767) as i16;
        }
    }
    // An 8-bit sample stays 8-bit, as Symphonie kept it.
    if bits == 8 {
        for v in pcm.iter_mut() {
            *v = *v / 256 * 256;
        }
    }
    let (ls, ll) = (looped.0 as usize, looped.1 as usize);
    let reps = usize::from(ins.0[131]);
    let mut lp = Loop::default();
    if ins.kind() == 4 && ll > 0 {
        lp = Loop::forward(ls as u32, (ls + ll) as u32);
    } else if ins.kind() == 8 && reps > 0 && ll > 0 && ls + ll <= pcm.len() && pcm.len() + ll * reps <= 16 << 20 {
        let mut out = Vec::with_capacity(pcm.len() + ll * reps);
        out.extend_from_slice(&pcm[..ls + ll]);
        for _ in 0..reps {
            out.extend_from_slice(&pcm[ls..ls + ll]);
        }
        out.extend_from_slice(&pcm[ls + ll..]);
        pcm = out;
    }
    (Wave::new(bits, 1, pcm), lp)
}

/// A position: what the order plays (Symphonie's positions are pieces of
/// patterns, transposed and at a speed of their own).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SymPos {
    pattern: u16,
    start: u16,
    length: u16,
    speed: u16,
    transpose: i16,
}

/// A channel as the events are turned into effects row by row.
#[derive(Clone, Copy)]
struct SymChan {
    vol_slide: f32,
    vol_slide_amt: f32,
    pitch_slide: f32,
    pitch_slide_amt: f32,
    stopped: bool,
    last_note: u8,
    last_ins: u8,
    last_vol: u8,
    channel_vol: u8,
    calc_vol: u8,
    from_add: u8,
    retrig_vib: bool,
    vibrato: u8,
    retrig_trem: bool,
    tremolo: u8,
    svib_speed: u8,
    svib_depth: u8,
    porta_amt: u8,
    svib_phase: u16,
    retrig_left: u16,
    porta_left: u16,
}

impl Default for SymChan {
    fn default() -> SymChan {
        SymChan {
            vol_slide: 0.0,
            vol_slide_amt: 0.0,
            pitch_slide: 0.0,
            pitch_slide_amt: 0.0,
            stopped: false,
            last_note: 0,
            last_ins: 0,
            last_vol: 64,
            channel_vol: 100,
            calc_vol: 64,
            from_add: 0,
            retrig_vib: false,
            vibrato: 0,
            retrig_trem: false,
            tremolo: 0,
            svib_speed: 0,
            svib_depth: 0,
            porta_amt: 0,
            svib_phase: 0,
            retrig_left: 0,
            porta_left: 0,
        }
    }
}

fn set_fx(c: &mut Cell, fx: Fx, p: u8) {
    (c.fx, c.param) = (fx, p);
}

pub fn load_symmod(d: &[u8]) -> Result<Module> {
    let nch_file = u32::from_be_bytes([d[12], d[13], d[14], d[15]]) as usize;
    let nch = nch_file.min(64);
    let mut r = Reader::at(d, 16);
    let mut track_len = 0usize;
    let mut tempo = 125u16;
    let mut boost = 2500u16;
    let mut pro = false;
    let mut external = false;
    let (mut positions, mut sequences, mut events, mut instruments) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut message = String::new();
    let mut waves: Vec<Arc<Wave>> = Vec::new();
    // Slots filled in order as sample hunks come: a stereo sample takes
    // two, and a mono one before a right-hand instrument skips one.
    while r.left() >= 4 {
        let kind = r.u32be()? as i32;
        match kind {
            -1 | -3 | -4 => {
                r.u32be()?;
            }
            -2 => {
                track_len = r.u32be()? as usize;
                if track_len > 1024 {
                    return bad("its tracks are too long");
                }
            }
            -5 => {
                if r.u32be()? & 0xFFFF != 4 {
                    return Err(Error::Unsupported("its events aren't four bytes".into()));
                }
            }
            -6 => tempo = (1.24 * f64::from(r.u32be()?.min(800))).round().clamp(32.0, 999.0) as u16,
            10 => {
                boost = r.u32be()?.min(10000) as u16;
                pro = true;
            }
            11 | 12 => {
                pro = true;
                r.u32be()?;
            }
            -7 => {
                r.u32be()?;
                if waves.is_empty() {
                    external = true;
                }
            }
            -10 => positions = sym_block(&mut r)?,
            -11 | -17 | -18 => {
                let mut b = sym_block(&mut r)?;
                if waves.len() >= instruments.len() / 256 {
                    continue;
                }
                if kind == -17 {
                    let mut last = 0u8;
                    for v in b.iter_mut() {
                        last = last.wrapping_add(*v);
                        *v = last;
                    }
                } else if kind == -18 {
                    // Blocks of 2048 points: the low bytes' deltas, then the high bytes'.
                    for blk in b.chunks_exact_mut(4096) {
                        let mut buf = [0u8; 4096];
                        let mut last = 0u8;
                        for i in 0..2048 {
                            last = last.wrapping_add(blk[i]);
                            buf[i * 2 + 1] = last;
                        }
                        for i in 0..2048 {
                            last = last.wrapping_add(blk[i + 2048]);
                            buf[i * 2] = last;
                        }
                        blk.copy_from_slice(&buf);
                    }
                }
                let got = sym_wave(&b);
                let slot = waves.len();
                if got.len() == 2 {
                    // A stereo sample: its halves for the left and right instruments.
                    waves.push(got[0].clone());
                    waves.push(got[1].clone());
                } else {
                    waves.push(got[0].clone());
                    if instruments.get((slot + 1) * 256 + 132) == Some(&2) {
                        waves.push(Arc::default());
                    }
                }
            }
            -12 => waves.push(Arc::default()),
            -13 => events = sym_block(&mut r)?,
            -14 => {
                if instruments.is_empty() {
                    instruments = sym_block(&mut r)?;
                } else {
                    sym_block(&mut r)?;
                }
            }
            -15 => sequences = sym_block(&mut r)?,
            -16 => {
                let t = sym_block(&mut r)?;
                message = crate::io::message(&t);
            }
            -21..=-19 => {
                sym_block(&mut r)?;
            }
            _ => break,
        }
    }
    let nins = (instruments.len() / 256).min(255);
    if track_len == 0 || nins == 0 || positions.is_empty() || events.is_empty() || sequences.is_empty() {
        return bad("it's missing its tracks, instruments or song");
    }
    if external {
        return Err(Error::Unsupported("its samples are in files of their own".into()));
    }
    let ins = |i: usize| SymIns(&instruments[i * 256..i * 256 + 256]);
    waves.resize(waves.len().max(nins), Arc::default());

    // The samples, an instrument each: the plain ones first, then the
    // virtual ones made from them.
    let mut samples: Vec<Sample> = (0..nins).map(|_| Sample::empty()).collect();
    let mut transposes: Vec<i32> = (0..nins).map(|i| i32::from(ins(i).transpose())).collect();
    let mut downsamples: Vec<i32> = (0..nins).map(|i| i32::from(ins(i).downsample())).collect();
    let rate = |i: usize, downsample: i32, transpose: i32| 40460.0 * 2f64.powf(-f64::from(downsample) + f64::from(transpose) / 12.0 + f64::from(ins(i).finetune()) / 1536.0);
    let mut source: Vec<usize> = (0..nins).collect();
    let mut rendered: Vec<Option<Wave>> = vec![None; nins];
    for (i, slot) in rendered.iter_mut().enumerate() {
        let s = ins(i);
        if s.empty() || !s.is_virtual() {
            continue;
        }
        if s.virt_version() == 0 && s.virt_events() <= 20 && s.virt_event_size() == 4 {
            let rates: Vec<f64> = (0..waves.len()).map(|k| if k < nins { rate(k, downsamples[k], transposes[k]) } else { 40460.0 }).collect();
            let first = usize::from(s.0[23]);
            match sym_render_virtual(&s, &waves, &rates, s.sample_flags() & 2 != 0, boost) {
                Some(w) => {
                    *slot = Some(w);
                    if first < nins {
                        downsamples[i] += downsamples[first];
                    }
                }
                None => source[i] = first,
            }
        } else if s.virt_version() == 1 && s.virt_events() == 2 && s.virt_event_size() == 10 {
            let a = usize::from(u16::from_be_bytes([s.0[20], s.0[21]]));
            let b = usize::from(u16::from_be_bytes([s.0[36], s.0[37]]));
            if a < waves.len() {
                let empty = Wave::default();
                let wb = waves.get(b).map(|w| w.as_ref()).unwrap_or(&empty);
                if let Some(w) = sym_render_transwave(&s, &waves[a], wb) {
                    *slot = Some(w);
                    // An octave down, oversampled four times.
                    transposes[i] += 12;
                }
            }
        }
    }
    for i in 0..nins {
        let s = ins(i);
        if s.empty() {
            continue;
        }
        let w = match rendered[i].take() {
            Some(w) => Arc::new(w),
            None => waves.get(source[i]).cloned().unwrap_or_default(),
        };
        let looped = sym_loop(&s, &w);
        let (wave, lp) = if w.is_empty() { (Wave::default(), Loop::default()) } else { sym_process(&s, &w, looped) };
        let vol = s.volume();
        let eff = if vol > 0 && vol < 200 { u32::from(vol.min(100)) * 64 / 100 } else { 64 };
        samples[i] = Sample {
            name: if s.is_virtual() {
                if s.virt_version() == 1 {
                    "Transwave".into()
                } else {
                    "Virtual".into()
                }
            } else {
                crate::io::text(&s.0[..128]).rsplit(['/', ':']).next().unwrap_or("").to_string()
            },
            wave: Arc::new(wave),
            c5: rate(i, downsamples[i], transposes[i]).round().clamp(1.0, 2_000_000.0) as u32,
            global_volume: eff as u8,
            looped: lp,
            ..Sample::empty()
        };
    }
    // Where a sustain instrument's written-out repeats end (a key off
    // goes there).
    let cue: Vec<Option<u32>> = (0..nins)
        .map(|i| {
            let s = ins(i);
            let (ls, ll) = sym_loop(&s, &waves.get(source[i]).cloned().unwrap_or_default());
            (s.kind() == 8 && s.0[131] > 0 && ll > 0).then(|| ls + ll * (u32::from(s.0[131]) + 1))
        })
        .collect();

    let ev = |k: usize| -> (u8, i8, u8, u8) { events.get(k * 4..k * 4 + 4).map_or((0, -1, 0, 0), |e| (e[0], e[1] as i8, e[2], e[3])) };
    let pattern_size = nch_file * track_len;
    let npat_file = events.len() / 4 / pattern_size.max(1);
    let pos_at = |k: usize| -> Option<SymPos> {
        let p = positions.get(k * 32..k * 32 + 32)?;
        let w = |o: usize| u16::from_be_bytes([p[o], p[o + 1]]);
        Some(SymPos { pattern: w(8), start: w(10), length: w(12), speed: w(14), transpose: w(16) as i16 })
    };
    let loops_at = |k: usize| positions.get(k * 32 + 4..k * 32 + 6).map_or(1, |b| u16::from_be_bytes([b[0], b[1]]).max(1));
    let npos = positions.len() / 32;

    let mut patterns: Vec<Arc<Pattern>> = Vec::new();
    let mut made: HashMap<SymPos, u16> = HashMap::new();
    let mut orders = Vec::new();
    let mut st = vec![SymChan::default(); nch];
    let instr_of = |i: u8| -> Option<SymIns> { (usize::from(i) < nins).then(|| ins(usize::from(i))) };
    for q in sequences.chunks_exact(16) {
        let w = |o: usize| u16::from_be_bytes([q[o], q[o + 1]]);
        let (start, length, info, seq_tr) = (usize::from(w(0)), usize::from(w(2)), w(6) as i16, w(8) as i16);
        if info == 1 {
            continue;
        }
        if info == -1 {
            break;
        }
        if start >= npos || length == 0 || length > npos || npos - length < start {
            continue;
        }
        if !orders.is_empty() {
            orders.push(ORDER_SKIP);
        }
        for k in start..start + length {
            let Some(mut pos) = pos_at(k) else { break };
            pos.transpose = pos.transpose.wrapping_add(seq_tr);
            let index = if let Some(&i) = made.get(&pos) {
                i
            } else {
                // Damaged positions could ask for more than any song has.
                if patterns.len() >= 4096 {
                    break;
                }
                let rows = usize::from(pos.length).clamp(1, 1024);
                let mut pat = Pattern::new(rows, nch);
                let index = patterns.len() as u16;
                made.insert(pos, index);
                if usize::from(pos.pattern) < npat_file && usize::from(pos.start) < track_len {
                    let mut speed = pos.speed.min(255) as u8;
                    let mut src = usize::from(pos.pattern) * pattern_size + usize::from(pos.start) * nch_file;
                    let mut sync: Option<Cell> = None;
                    for row in 0..rows {
                        let mut apply_sync = false;
                        for ch in 0..nch_file {
                            let (cmd, enote, param, inst) = ev(src);
                            src += 1;
                            if ch >= nch {
                                continue;
                            }
                            let global = matches!(cmd, 9 | 24 | 25) || (cmd == 0 && matches!(param, 249 | 250));
                            let mut note: i32 = if (0..=84).contains(&enote) { i32::from(enote) + 25 } else { -1 };
                            let mapped = if usize::from(inst) < nins {
                                if ins(usize::from(inst)).inst_flags() & 1 == 0 && note >= 0 {
                                    note = (note + i32::from(pos.transpose)).clamp(1, 120);
                                }
                                inst + 1
                            } else {
                                0
                            };
                            let mut m = pat.cell(row, ch);
                            if m.note != NOTE_NONE && (cmd != 0 || enote != -1 || inst != 0 || param != 0) && m.instr > 0 && instr_of(m.instr - 1).is_some_and(|s| s.channel() == 2) {
                                m = Cell::default();
                            }
                            if apply_sync {
                                apply_sync = false;
                                m = sync.unwrap_or_default();
                                let prev = st[ch - 1].calc_vol;
                                if m.fx == Fx::None && st[ch].calc_vol != prev {
                                    st[ch].calc_vol = prev;
                                    set_fx(&mut m, Fx::ChannelVolume, prev);
                                }
                                if !global {
                                    if let Some(c) = pat.cell_mut(row, ch) {
                                        *c = m;
                                    }
                                    continue;
                                }
                            }
                            let note_u = note.max(0) as u8;
                            let c = &mut st[ch];
                            let mut apply_volume = false;
                            match cmd {
                                0 => {
                                    if param > 200 {
                                        match param {
                                            254 => c.stopped = true,
                                            253 => c.stopped = false,
                                            251 => {
                                                // Key off: on to past the written-out repeats.
                                                let last = usize::from(c.last_ins);
                                                match last.checked_sub(1).and_then(|i| cue.get(i).copied().flatten()) {
                                                    Some(at) => {
                                                        if m.note == NOTE_NONE {
                                                            m.note = c.last_note;
                                                        }
                                                        let frames = samples.get(last - 1).map_or(0, |s| s.frames() as u32);
                                                        if at < frames {
                                                            set_fx(&mut m, Fx::Offset, (at / 256).min(255) as u8);
                                                        }
                                                    }
                                                    None => m.note = NOTE_OFF,
                                                }
                                            }
                                            250 if speed > 1 => {
                                                speed -= 1;
                                                set_fx(&mut m, Fx::Speed, speed);
                                            }
                                            249 if speed < 255 => {
                                                speed += 1;
                                                set_fx(&mut m, Fx::Speed, speed);
                                            }
                                            248 => {
                                                c.last_note = note_u;
                                                if mapped == c.last_ins {
                                                    m.note = note_u;
                                                    set_fx(&mut m, Fx::TonePorta, 0xFF);
                                                    c.pitch_slide = 0.0;
                                                    c.porta_left = 0;
                                                    c.retrig_vib = true;
                                                    c.retrig_trem = true;
                                                }
                                            }
                                            247 => set_fx(&mut m, Fx::PortaUp, 0xF2),
                                            246 => set_fx(&mut m, Fx::PortaDown, 0xF2),
                                            245 => set_fx(&mut m, Fx::PortaUp, 0xF4),
                                            244 => set_fx(&mut m, Fx::PortaDown, 0xF4),
                                            243 => set_fx(&mut m, Fx::PortaUp, 0xF8),
                                            242 => set_fx(&mut m, Fx::PortaDown, 0xF8),
                                            _ => {}
                                        }
                                    } else if enote >= 0 || param < 100 {
                                        if enote >= 0 {
                                            m.note = note_u;
                                            c.last_note = note_u;
                                            m.instr = mapped;
                                            c.last_ins = mapped;
                                            c.pitch_slide = 0.0;
                                            c.porta_left = 0;
                                            c.retrig_vib = true;
                                            c.retrig_trem = true;
                                        }
                                        if param > 0 {
                                            c.last_vol = (f32::from(param) * 0.64).round().min(255.0) as u8;
                                            if c.vol_slide != 0.0 {
                                                apply_volume = true;
                                            }
                                            c.vol_slide = 0.0;
                                        }
                                    }
                                    let new_vol = ((u32::from(c.last_vol) * u32::from(c.channel_vol) + 50) / 100).min(255) as u8;
                                    if apply_volume || c.calc_vol != new_vol {
                                        c.calc_vol = new_vol;
                                        set_fx(&mut m, Fx::ChannelVolume, new_vol.min(64));
                                    }
                                }
                                1 => {
                                    c.vol_slide_amt = 0.0;
                                    c.vol_slide = f32::from(param) * 0.0333;
                                }
                                2 => {
                                    c.vol_slide_amt = 0.0;
                                    c.vol_slide = f32::from(param) * -0.0333;
                                }
                                12 => {
                                    c.tremolo = (inst >> 3).min(15) << 4 | (param >> 3).min(15);
                                    if c.tremolo != 0 {
                                        c.retrig_trem = true;
                                    } else {
                                        set_fx(&mut m, Fx::Tremolo, 0);
                                    }
                                }
                                3 | 4 => {
                                    c.pitch_slide_amt = 0.0;
                                    c.pitch_slide = f32::from(param) * if cmd == 3 { 0.0333 } else { -0.0333 };
                                    c.porta_left = 0;
                                }
                                15 if note >= 0 && param > 0 => {
                                    let distance = (i32::from(note_u) - i32::from(c.last_note)).abs() * 32;
                                    c.pitch_slide = 0.0;
                                    m.note = note_u;
                                    c.last_note = note_u;
                                    c.porta_amt = (distance / (2 * i32::from(param))).min(255) as u8;
                                    set_fx(&mut m, Fx::TonePorta, c.porta_amt);
                                    c.porta_left = (distance - distance.min(i32::from(c.porta_amt) * (i32::from(speed) - 1))) as u16;
                                }
                                13 => {
                                    c.vibrato = (inst >> 3).min(15) << 4 | param.min(15);
                                    if c.vibrato != 0 {
                                        c.retrig_vib = true;
                                    } else {
                                        set_fx(&mut m, Fx::Vibrato, 0);
                                    }
                                }
                                18 => {
                                    c.last_note = (i32::from(c.last_note) + i32::from(param)).clamp(1, 120) as u8;
                                    m.note = c.last_note;
                                    set_fx(&mut m, Fx::TonePorta, 0xFF);
                                    c.porta_left = 0;
                                }
                                16 if param >= 1 => {
                                    set_fx(&mut m, Fx::Retrig, (inst + 1).min(15));
                                    c.retrig_left = u16::from(param) * (u16::from(inst) + 1);
                                }
                                9 => {
                                    speed = if param != 0 { param } else { 4 };
                                    set_fx(&mut m, Fx::Speed, speed);
                                }
                                19 if enote == 0 || enote == 4 => {
                                    let pan = if enote == 4 { u32::from(inst) } else { 128 };
                                    let vol = u32::from(param.min(100));
                                    let vl = (vol * (256 - pan).min(128) / 128) as u8;
                                    let vr = (vol * pan.min(128) / 128) as u8;
                                    if vl != c.channel_vol {
                                        c.channel_vol = vl;
                                        c.calc_vol = ((u32::from(c.last_vol) * u32::from(c.channel_vol) + 50) / 100) as u8;
                                        set_fx(&mut m, Fx::ChannelVolume, c.calc_vol.min(64));
                                    }
                                    let calc = c.calc_vol;
                                    if enote == 4 && ch + 1 < nch && st[ch + 1].channel_vol != vr {
                                        st[ch + 1].channel_vol = vr;
                                        if let Some(n) = pat.cell_mut(row, ch + 1) {
                                            set_fx(n, Fx::ChannelVolume, calc.min(64));
                                        }
                                    }
                                }
                                7 => {
                                    c.from_add = param;
                                    c.svib_speed = 0;
                                    c.svib_depth = 0;
                                }
                                8 => c.from_add = c.from_add.wrapping_add(param),
                                14 => {
                                    c.svib_speed = inst;
                                    c.svib_depth = param;
                                }
                                5 | 6 => {
                                    if cmd == 6 {
                                        c.last_note = note_u;
                                        m.instr = mapped;
                                        c.last_ins = mapped;
                                    }
                                    m.note = c.last_note;
                                    if note >= 0 {
                                        m.instr = mapped;
                                        c.last_ins = mapped;
                                    }
                                    if cmd == 5 {
                                        (m.vol, m.volp) = (Vol::TonePorta, 1);
                                    }
                                    if param != 0 || c.from_add != 0 || c.svib_depth != 0 {
                                        let svib = if c.svib_depth != 0 {
                                            f64::from(c.svib_depth) * ((f64::from(c.svib_phase) * std::f64::consts::TAU / 1024.0 + 1.5 * std::f64::consts::PI).sin() - 1.0) / 4.0
                                        } else {
                                            0.0
                                        };
                                        // A percentage (of 256) of the sample made an offset.
                                        let pct = (f64::from(param) + f64::from(c.from_add) + svib).round().clamp(0.0, 255.0);
                                        let frames = usize::from(c.last_ins).checked_sub(1).and_then(|i| samples.get(i)).map_or(0, |s| s.frames()) as f64;
                                        set_fx(&mut m, Fx::Offset, (frames * pct / 256.0 / 256.0).round().min(255.0) as u8);
                                    }
                                    c.porta_left = 0;
                                }
                                _ => {}
                            }
                            let c = &mut st[ch];
                            if m.note != NOTE_NONE {
                                c.stopped = false;
                            } else if c.stopped {
                                if let Some(x) = pat.cell_mut(row, ch) {
                                    *x = m;
                                }
                                continue;
                            }
                            let spd = f32::from(speed);
                            if c.retrig_left > 0 {
                                c.retrig_left = c.retrig_left.max(u16::from(speed)) - u16::from(speed);
                                if m.fx == Fx::None {
                                    set_fx(&mut m, Fx::Retrig, 0);
                                }
                            }
                            if c.vol_slide != 0.0 {
                                c.vol_slide_amt += c.vol_slide * spd;
                                if m.fx == Fx::None {
                                    let a = c.vol_slide_amt;
                                    if speed > 1 && a >= spd - 1.0 {
                                        let s = (a / (spd - 1.0)).round().min(15.0) as u8;
                                        c.vol_slide_amt -= f32::from(s) * (spd - 1.0);
                                        set_fx(&mut m, Fx::ChannelVolSlide, s << 4);
                                    } else if a >= 1.0 {
                                        let s = a.round().min(15.0) as u8;
                                        c.vol_slide_amt -= f32::from(s);
                                        set_fx(&mut m, Fx::ChannelVolSlide, s << 4 | 0x0F);
                                    } else if speed > 1 && a <= -(spd - 1.0) {
                                        let s = (-a / (spd - 1.0)).round().min(15.0) as u8;
                                        c.vol_slide_amt += f32::from(s) * (spd - 1.0);
                                        set_fx(&mut m, Fx::ChannelVolSlide, s);
                                    } else if a <= -1.0 {
                                        let s = (-a).round().min(14.0) as u8;
                                        c.vol_slide_amt += f32::from(s);
                                        set_fx(&mut m, Fx::ChannelVolSlide, s | 0xF0);
                                    }
                                }
                            }
                            if c.pitch_slide != 0.0 {
                                c.pitch_slide_amt += c.pitch_slide * spd;
                                let a = c.pitch_slide_amt;
                                if m.fx == Fx::None {
                                    if speed > 1 && a >= spd - 1.0 {
                                        let s = (a / (spd - 1.0)).round().min(223.0) as u8;
                                        c.pitch_slide_amt -= f32::from(s) * (spd - 1.0);
                                        set_fx(&mut m, Fx::PortaUp, s);
                                    } else if a >= 1.0 {
                                        let s = a.round().min(15.0) as u8;
                                        c.pitch_slide_amt -= f32::from(s);
                                        set_fx(&mut m, Fx::PortaUp, s | 0xF0);
                                    } else if speed > 1 && a <= -(spd - 1.0) {
                                        let s = (-a / (spd - 1.0)).round().min(223.0) as u8;
                                        c.pitch_slide_amt += f32::from(s) * (spd - 1.0);
                                        set_fx(&mut m, Fx::PortaDown, s);
                                    } else if a <= -1.0 {
                                        let s = (-a).round().min(14.0) as u8;
                                        c.pitch_slide_amt += f32::from(s);
                                        set_fx(&mut m, Fx::PortaDown, s | 0xF0);
                                    }
                                } else if m.vol == Vol::None && speed > 1 {
                                    if a / 4.0 >= spd - 1.0 {
                                        let s = ((a / (spd - 1.0)).round() as u32 / 4).min(9) as u8;
                                        c.pitch_slide_amt -= f32::from(s) * (spd - 1.0) * 4.0;
                                        (m.vol, m.volp) = (Vol::PortaUp, s);
                                    } else if a / 4.0 <= -(spd - 1.0) {
                                        let s = ((-a / (spd - 1.0)).round() as u32 / 4).min(9) as u8;
                                        c.pitch_slide_amt += f32::from(s) * (spd - 1.0) * 4.0;
                                        (m.vol, m.volp) = (Vol::PortaDown, s);
                                    }
                                }
                            }
                            if m.fx == Fx::None && c.vibrato != 0 && c.retrig_vib {
                                set_fx(&mut m, Fx::Vibrato, c.vibrato);
                                c.retrig_vib = false;
                            }
                            if m.fx == Fx::None && c.tremolo != 0 && c.retrig_trem {
                                set_fx(&mut m, Fx::Tremolo, c.tremolo);
                                c.retrig_trem = false;
                            }
                            if m.fx != Fx::TonePorta && c.porta_left > 0 {
                                if m.fx == Fx::None {
                                    set_fx(&mut m, Fx::TonePorta, 0);
                                } else {
                                    (m.vol, m.volp) = (Vol::TonePorta, 0);
                                }
                                c.porta_left -= c.porta_left.min(u16::from(c.porta_amt) * (u16::from(speed).max(1) - 1));
                            }
                            c.svib_phase = (c.svib_phase + u16::from(c.svib_speed) * u16::from(speed)) & 1023;
                            if ch % 2 == 0 && c.last_ins > 0 && instr_of(c.last_ins - 1).is_some_and(|s| s.inst_flags() & 4 != 0) {
                                let mut s = m;
                                if s.instr > 0 && instr_of(c.last_ins - 1).is_some_and(|i| i.channel() == 1) {
                                    s.instr += 1;
                                }
                                sync = Some(s);
                                apply_sync = true;
                            }
                            // A stereo instrument's note plays its right half on the next channel.
                            if enote > 0 && cmd == 0 && param <= 200 && ch + 1 < nch && ch % 2 == 0 && instr_of(inst).is_some_and(|s| s.channel() == 1) {
                                let mut next = m;
                                next.instr += 1;
                                if let Some(n) = pat.cell_mut(row, ch + 1) {
                                    *n = next;
                                }
                                let (lv, vs, vsa, ps, psa, rl) = (st[ch].last_vol, st[ch].vol_slide, st[ch].vol_slide_amt, st[ch].pitch_slide, st[ch].pitch_slide_amt, st[ch].retrig_left);
                                let n = &mut st[ch + 1];
                                (n.last_vol, n.vol_slide, n.vol_slide_amt, n.pitch_slide, n.pitch_slide_amt, n.retrig_left) = (lv, vs, vsa, ps, psa, rl);
                            }
                            if let Some(x) = pat.cell_mut(row, ch) {
                                *x = m;
                            }
                        }
                    }
                    // The position's speed on its first row with room for it.
                    let sp = pos.speed.clamp(1, 255) as u8;
                    for row in 0..rows {
                        if (0..nch).any(|c| pat.cell(row, c).fx == Fx::Speed) {
                            break;
                        }
                        if let Some(c) = (0..nch).find(|&c| pat.cell(row, c).fx == Fx::None) {
                            if let Some(x) = pat.cell_mut(row, c) {
                                set_fx(x, Fx::Speed, sp);
                            }
                            break;
                        }
                    }
                }
                patterns.push(Arc::new(pat));
                index
            };
            for _ in 0..loops_at(k) {
                orders.push(index);
            }
        }
    }

    let mut m = Module {
        message,
        format: "symmod".into(),
        format_name: "Symphonie".into(),
        tracker: if !pro {
            "Symphonie"
        } else if nins <= 128 {
            "Symphonie Pro"
        } else {
            "Symphonie Pro 256"
        }
        .into(),
        kind: Kind::It,
        flags: Flags { linear: true, ..Default::default() },
        speed: 6,
        tempo,
        global_volume: 128,
        mix_volume: (512 / nch.max(1)).clamp(16, 128) as u8,
        separation: 128,
        channels: (0..nch).map(|c| Channel { pan: if c % 2 == 1 { 255 } else { 0 }, ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Face The Music ----
//
// "FTMN", version 3, the sample count, the measures, a CIA tempo, the
// key, which channels play, the global volume, flags (1 samples in the
// file, 2 the LED filter), ticks a row and rows a measure, title and
// artist; 32 bytes a sample (its file name); the SEL effects (scripts
// of up to 512 lines); then each of the eight channels' events, two
// bytes each, a gap of empty rows before each: an instrument, a
// channel volume and instrument, a SEL effect, a pitch bend or a volume
// fade (lasting so many rows), or a loop; and the samples. Here the
// measures are patterns, the loops are written out in the order list,
// and the pitch bends and fades are written into the rows they last
// for (a slide to the note at the speed that gets there in time; the
// channel volume stepped down). The SEL scripts are kept as they are,
// started by `Fx::Sel`, and run as the song plays by neumetik-synth's
// script machine (`sel.rs`).

pub fn probe_ftm(d: &[u8]) -> bool {
    if d.len() < 82 || &d[..4] != b"FTMN" || d[4] != 3 || d[5] >= 64 {
        return false;
    }
    let tempo = u16::from_be_bytes([d[8], d[9]]);
    let (key, gvol, flags, tpr, rpm) = (d[10], d[12], d[13], d[14], d[15]);
    (0x1000..=0x4FFF).contains(&tempo)
        && key < 12
        && gvol < 64
        && flags & 0xFC == 0
        && (1..=24).contains(&tpr)
        && (4..=96).contains(&rpm)
        && u16::from(rpm) == 96 / u16::from(tpr)
        && d[80] <= 64
        && d[81] == 0
}

pub fn load_ftm(d: &[u8]) -> Result<Module> {
    let nsmp = usize::from(d[5]);
    // No more measures than the file could hold events for.
    let measures = usize::from(u16::from_be_bytes([d[6], d[7]])).min(d.len() / 2 + 1).min(4096);
    let cia = u16::from_be_bytes([d[8], d[9]]);
    let (mutes, gvol, flags, speed, rpm) = (d[11], d[12], d[13], d[14], usize::from(d[15]));
    let title = crate::io::text(&d[16..48]);
    let artist = crate::io::text(&d[48..80]);
    let neffects = usize::from(d[80]);
    let mut r = Reader::at(d, 82);
    let mut samples = Vec::with_capacity(nsmp);
    for _ in 0..nsmp {
        let h = r.bytes(32)?;
        let name = crate::io::text(&h[..30]);
        samples.push(Sample { name: name.rsplit(['/', ':']).next().unwrap_or("").to_string(), filename: name, c5: 8287, ..Sample::empty() });
    }
    let mut sel = neumetik_synth::sel::Program::new(rpm as u16);
    for _ in 0..neffects {
        let lines = usize::from(r.u16be()?);
        let index = r.u16be()?;
        if lines > 0x200 || index > 63 {
            return bad("a SEL effect is damaged");
        }
        let body = r.bytes(lines * 4)?;
        sel.scripts[usize::from(index)] = Some(body.chunks_exact(4).map(|c| neumetik_synth::sel::Line { op: c[0], data: [c[1], c[2], c[3]] }).collect());
    }

    const NCH: usize = 8;
    let mut patterns: Vec<Pattern> = (0..measures).map(|_| Pattern::new(rpm, NCH)).collect();
    // Loop marks: (measure, row, channel, count; 0 ends one).
    let mut loops: Vec<(usize, usize, usize, u8)> = Vec::new();
    for ch in 0..NCH {
        let default_gap = usize::from(r.u16be()?);
        let len = r.u32be()? as usize;
        let body = r.bytes_upto(len);
        let mut row_at = 0usize;
        let mut gap = default_gap;
        // What the channel's doing, to write out the lasting effects.
        let mut note_now = 0u8;
        let mut cv = 64u8;
        let mut porta: Option<(u8, usize)> = None;
        let mut fade: Option<(u8, usize, usize)> = None;
        let mut k = 0;
        while k + 2 <= body.len() {
            let (a, b) = (body[k], body[k + 1]);
            k += 2;
            if a & 0xF0 == 0xF0 {
                gap = usize::from(a & 0x0F) << 8 | usize::from(b);
                continue;
            }
            // The rows passed over carry on what's lasting.
            let target = row_at + gap;
            for at in row_at..target.min(measures * rpm) {
                ftm_carry(&mut patterns, rpm, at, ch, &mut porta, &mut fade, &mut cv);
            }
            if target / rpm >= measures {
                break;
            }
            let (pi, row) = (target / rpm, target % rpm);
            let param = (a & 0x0F) << 2 | b >> 6;
            let mut cell = Cell::default();
            let note = b & 0x3F;
            if note >= 35 {
                cell.note = NOTE_OFF;
            } else if note != 0 {
                cell.note = NOTE_MIDDLE_C - 13 + note;
            }
            match a & 0xF0 {
                0x00 => cell.instr = param,
                // A SEL script started (one that isn't there stops the channel's).
                0xB0 => (cell.fx, cell.param) = (Fx::Sel, if sel.scripts.get(usize::from(param)).is_some_and(|s| s.is_some()) { param } else { 64 }),
                0xC0 => {
                    if cell.note >= NOTE_MIN && cell.note <= NOTE_MAX && note_now != 0 {
                        let semis = u32::from(cell.note.abs_diff(note_now));
                        let ticks = u32::from(speed) * u32::from(param);
                        let p = if param == 0 { 0xFF } else { (semis * 16).div_ceil(ticks.max(1)).clamp(1, 0xFF) as u8 };
                        (cell.fx, cell.param) = (Fx::TonePorta, p);
                        porta = (param > 1).then(|| (p, usize::from(param) - 1));
                    }
                }
                0xD0 => {
                    fade = (param > 0).then_some((cv, usize::from(param), 0));
                    if param == 0 {
                        cv = 0;
                        (cell.fx, cell.param) = (Fx::ChannelVolume, 0);
                    }
                }
                0xE0 => loops.push((pi, row, ch, param)),
                hi => {
                    cv = (u32::from((hi >> 4) - 1) * 64 / 9) as u8;
                    fade = None;
                    (cell.fx, cell.param) = (Fx::ChannelVolume, cv);
                    cell.instr = param;
                }
            }
            if cell.note >= NOTE_MIN && cell.note <= NOTE_MAX {
                if cell.fx != Fx::TonePorta {
                    porta = None;
                }
                note_now = cell.note;
            }
            if let Some(c) = patterns[pi].cell_mut(row, ch) {
                *c = cell;
            }
            // This row's own fade step, after the cell's set.
            if matches!(a & 0xF0, 0xD0) {
                ftm_carry(&mut patterns, rpm, target, ch, &mut None, &mut fade, &mut cv);
            }
            row_at = target + 1;
            gap = default_gap;
        }
        for at in row_at..measures * rpm {
            if porta.is_none() && fade.is_none() {
                break;
            }
            ftm_carry(&mut patterns, rpm, at, ch, &mut porta, &mut fade, &mut cv);
        }
    }

    // The loops written out: each closing mark repeats the measures since
    // its opening one.
    let mut orders: Vec<u16> = (0..measures as u16).collect();
    loops.sort_by_key(|&(m, row, ch, _)| (m, row, ch));
    let mut open: Vec<(usize, u8)> = Vec::new();
    let mut inserted = 0usize;
    for &(m, _, _, count) in &loops {
        let at = m + inserted;
        if count != 0 && open.len() < 16 {
            open.push((at, count));
        } else if count == 0 {
            let Some((start, times)) = open.pop() else { continue };
            let copy: Vec<u16> = orders[start..=at.min(orders.len() - 1)].to_vec();
            for _ in 1..times {
                if orders.len() + copy.len() > 4000 {
                    break;
                }
                let pos = at + 1;
                orders.splice(pos.min(orders.len())..pos.min(orders.len()), copy.iter().copied());
                inserted += copy.len();
            }
        }
    }

    if flags & 1 != 0 {
        for s in samples.iter_mut() {
            if s.filename.is_empty() {
                continue;
            }
            let (Ok(ls), Ok(ll)) = (r.u16be(), r.u16be()) else { break };
            let (ls, ll) = (u32::from(ls) * 2, u32::from(ll) * 2);
            let n = (ls + ll) as usize;
            if n == 0 {
                continue;
            }
            s.wave = Arc::new(Wave::new(8, 1, decode(r.bytes_upto(n), Pcm::S8, n)));
            if ll > 0 {
                s.looped = Loop::forward(ls, ls + ll);
            }
        }
    }

    let mut m = Module {
        title,
        artist,
        format: "ftm".into(),
        format_name: "Face The Music".into(),
        tracker: "Face The Music".into(),
        kind: Kind::It,
        flags: Flags { linear: true, ..Default::default() },
        speed,
        tempo: (1_777_517.482 / f64::from(cia)).round().clamp(32.0, 999.0) as u16,
        global_volume: (u32::from(gvol) * 128 / 63) as u8,
        mix_volume: 48,
        separation: 128,
        time: TimeSig { rows_per_beat: if matches!(speed, 2 | 4 | 8 | 16) { rpm as u16 / 3 } else { rpm as u16 / 4 }.max(1), rows_per_measure: rpm as u16 },
        channels: (0..NCH).map(|c| Channel { pan: if !(2..=5).contains(&c) { 64 } else { 192 }, muted: mutes & (1 << c) == 0, ..Default::default() }).collect(),
        orders,
        patterns: patterns.into_iter().map(Arc::new).collect(),
        samples,
        sel: (!sel.is_empty()).then(|| Arc::new(sel)),
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

/// A row (counted through the whole song) of a channel where a pitch
/// bend or a fade lasts: the slide carried on, the volume stepped down.
fn ftm_carry(patterns: &mut [Pattern], rpm: usize, at: usize, ch: usize, porta: &mut Option<(u8, usize)>, fade: &mut Option<(u8, usize, usize)>, cv: &mut u8) {
    let Some(c) = patterns.get_mut(at / rpm).and_then(|p| p.cell_mut(at % rpm, ch)) else { return };
    if let Some((p, left)) = *porta {
        if c.fx == Fx::None {
            (c.fx, c.param) = (Fx::TonePorta, p);
        }
        *porta = (left > 1).then_some((p, left - 1));
    }
    if let Some((from, rows, done)) = *fade {
        let done = done + 1;
        let v = (u32::from(from) * (rows.saturating_sub(done)) as u32 / rows.max(1) as u32) as u8;
        // Where the effect column's taken, that row's step is missed.
        if c.fx == Fx::None {
            (c.fx, c.param) = (Fx::ChannelVolume, v);
        }
        *cv = v;
        *fade = (done < rows).then_some((from, rows, done));
    }
}

// ---- Fashion Tracker ----
//
// Not a module file but a program: Fashion Tracker saved a song as its
// replay routine (68000 code, at the address it was assembled for) with
// the song after it. Everything is found from the code: the replay's
// smallest JSR target is its first routine, so the base address; the
// pattern table's address follows its MULU #$400, the order list's a
// MOVEA.L #,A1 before the song position's ADDA, the song's length an
// immediate CMPI.L, the speed an immediate CMPI.B after the tick count's
// ADDQ; the sample routine names the instrument table (sample and
// volume each), and the sample starts, lengths and loop flags. Patterns
// are ProTracker's four-byte cells, four channels, 64 rows; the
// instrument is the cell's low nibble (16 of them, 0 repeating the
// channel's last). The first version plays no effects; the second only
// an arpeggio (command 1). A sample marked looped loops whole.

/// Where `pat` first appears in `d` (from `from`).
fn find(d: &[u8], pat: &[u8], from: usize) -> Option<usize> {
    d.get(from..)?.windows(pat.len()).position(|w| w == pat).map(|p| p + from)
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// What the replay routine says about its song.
struct FashionLayout {
    base: u32,
    patterns: usize,
    orders: usize,
    length: usize,
    speed: u8,
    instruments: usize,
    starts: usize,
    lengths: usize,
    loops: usize,
    arpeggio: bool,
}

fn fashion_layout(d: &[u8]) -> Option<FashionLayout> {
    if d.len() < 0x300 || d[..4] != [0x13, 0xFC, 0x00, 0x40] {
        return None;
    }
    // Every JSR's target; the smallest is the routine at the start.
    let code = &d[..d.len().min(0x600)];
    let mut base = u32::MAX;
    let mut at = 0;
    while let Some(p) = find(code, &[0x4E, 0xB9], at) {
        if let Some(t) = be32(code, p + 2) {
            base = base.min(t);
        }
        at = p + 2;
    }
    if base == u32::MAX {
        return None;
    }
    let addr = |v: u32| -> Option<usize> { v.checked_sub(base).map(|o| o as usize).filter(|&o| o < d.len()) };
    let p = find(code, &[0xC0, 0xFC, 0x04, 0x00, 0x23, 0xFC], 0)?;
    let patterns = addr(be32(code, p + 6)?)?;
    let p = find(code, &[0x06, 0xB9, 0x00, 0x00, 0x00, 0x01], 0)?;
    if code.get(p + 10..p + 12)? != [0x0C, 0xB9] {
        return None;
    }
    let length = be32(code, p + 12)? as usize;
    // The order list: the MOVEA.L #,A1 just before.
    let q = code[..p].windows(2).rposition(|w| w == [0x22, 0x7C])?;
    let orders = addr(be32(code, q + 2)?)?;
    let p = find(code, &[0x06, 0x39, 0x00, 0x01], 0)?;
    if code.get(p + 8..p + 10)? != [0x0C, 0x39] {
        return None;
    }
    let speed = *code.get(p + 11)?;
    // The sample routine: four tables, each a MOVEA.L #,A1; ADDA.L D0,A1.
    let mut tables = Vec::new();
    let mut at = find(code, &[0x02, 0x00, 0x00, 0xF0, 0x80, 0xFC, 0x00, 0x04], 0).or_else(|| find(code, &[0x02, 0x00, 0x00, 0xF0], 0))?;
    while tables.len() < 4 {
        let p = find(code, &[0x22, 0x7C], at)?;
        if code.get(p + 6..p + 8)? == [0xD3, 0xC0] {
            tables.push(addr(be32(code, p + 2)?)?);
        }
        at = p + 2;
    }
    let arpeggio = find(code, &[0x02, 0x00, 0x00, 0x0F, 0x0C, 0x00, 0x00, 0x01], 0).is_some();
    Some(FashionLayout { base, patterns, orders, length: length.clamp(1, 256), speed: speed.max(1), instruments: tables[0], starts: tables[1], lengths: tables[2], loops: tables[3], arpeggio })
}

pub fn probe_fashion(d: &[u8]) -> bool {
    fashion_layout(d).is_some_and(|l| l.length <= 128 && l.orders + l.length <= d.len() && l.lengths < l.starts && l.starts < l.loops && l.loops < l.instruments)
}

pub fn load_fashion(d: &[u8]) -> Result<Module> {
    let l = fashion_layout(d).ok_or_else(|| Error::Bad("its replay routine isn't one this reads".into()))?;
    let orders: Vec<u16> = d[l.orders..l.orders + l.length].iter().map(|&o| u16::from(o)).collect();
    let npat = usize::from(orders.iter().copied().max().unwrap_or(0)) + 1;
    let nsmp = ((l.starts - l.lengths) / 4).min(64);
    let waves: Vec<(Arc<Wave>, bool)> = (0..nsmp)
        .map(|i| {
            let start = be32(d, l.starts + i * 4).unwrap_or(0) as usize;
            let len = be32(d, l.lengths + i * 4).unwrap_or(0) as usize * 2;
            let looped = d.get(l.loops + i * 4 + 3).is_some_and(|&f| f == 1);
            let off = start.checked_sub(l.base as usize);
            let b = off.and_then(|o| d.get(o..(o + len).min(d.len()))).unwrap_or(&[]);
            (Arc::new(Wave::new(8, 1, decode(b, Pcm::S8, b.len()))), looped)
        })
        .collect();
    // An instrument each: a sample and its volume.
    let samples: Vec<Sample> = (0..16)
        .map(|i| {
            let e = d.get(l.instruments + i * 4..l.instruments + i * 4 + 4).unwrap_or(&[0; 4]);
            let (smp, vol) = (usize::from(u16::from_be_bytes([e[0], e[1]])), u16::from_be_bytes([e[2], e[3]]));
            match waves.get(smp) {
                Some((w, looped)) if i > 0 => {
                    Sample { wave: w.clone(), volume: vol.min(64) as u8, looped: if *looped { Loop::forward(0, w.frames() as u32) } else { Loop::default() }, c5: 8287, ..Sample::empty() }
                }
                _ => Sample::empty(),
            }
        })
        .collect();
    let mut patterns = Vec::with_capacity(npat);
    for p in 0..npat {
        let mut pat = Pattern::new(64, 4);
        for row in 0..64 {
            for ch in 0..4 {
                let at = l.patterns + p * 1024 + row * 16 + ch * 4;
                let Some(c) = d.get(at..at + 4) else { continue };
                let period = u16::from_be_bytes([c[0] & 0x0F, c[1]]);
                if period == 0 {
                    continue;
                }
                let mut cell = Cell { note: super::common::period_to_note(period).unwrap_or(NOTE_NONE), instr: c[2] >> 4, ..Default::default() };
                if l.arpeggio && c[2] & 0x0F == 1 && c[3] != 0 {
                    (cell.fx, cell.param) = (Fx::Arpeggio, c[3]);
                }
                if let Some(x) = pat.cell_mut(row, ch) {
                    *x = cell;
                }
            }
        }
        patterns.push(Arc::new(pat));
    }
    let mut m = Module {
        format: "ex".into(),
        format_name: "Fashion Tracker".into(),
        tracker: if l.arpeggio { "Fashion Tracker (with arpeggio)" } else { "Fashion Tracker" }.into(),
        kind: Kind::Mod,
        flags: Flags { amiga_limits: true, ..Default::default() },
        speed: l.speed,
        tempo: 125,
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        channels: (0..4).map(|c| Channel { pan: super::common::amiga_pan(c), ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Digital Sound Studio ----
//
// "MMU2", the tempo and speed, 31 sample headers (a name, where play
// starts in its data, the one-shot and loop lengths in words, the loop's
// start, finetune, volume, a rate), the orders (a count and 128), the
// patterns (four channels, 64 rows, four bytes a cell: the sample in
// five bits, an 11-bit Amiga period (7FF mutes), a command and its
// parameter), then each sample's data from its start. Its commands are
// its own (NostalgicPlayer's player, MIT, describes them): ProTracker's
// moved about, with fine slides, master volume and portamento
// commands of their own. A period with no sample plays nothing.

pub fn probe_dss(d: &[u8]) -> bool {
    if d.len() < 0x61E || &d[..4] != b"MMU2" {
        return false;
    }
    let n = usize::from(u16::from_be_bytes([d[0x59C], d[0x59D]]));
    n > 0 && n <= 128 && (usize::from(d[0x59E..0x59E + n].iter().copied().max().unwrap_or(0)) + 1) * 1024 + 0x61E <= d.len()
}

fn dss_effect(cmd: u8, p: u8) -> (Fx, u8) {
    match cmd {
        0x00 if p != 0 => (Fx::Arpeggio, p),
        0x01 if p != 0 => (Fx::PortaUp, p),
        0x02 if p != 0 => (Fx::PortaDown, p),
        0x03 => (Fx::Volume, p.min(64)),
        0x04 => (Fx::GlobalVolume, p.min(64) * 2),
        0x05 if p != 0 => (Fx::Speed, p),
        0x06 if p == 0 || p == 0xFF => (Fx::PatternBreak, 0),
        0x06 => (Fx::PositionJump, p - 1),
        0x07 => (Fx::ModEx, u8::from(p == 0)),
        0x08 if p != 0 => (Fx::FinePortaUp, p),
        0x09 if p != 0 => (Fx::FinePortaDown, p),
        0x0A | 0x1E => (Fx::ModEx, 0x30 | u8::from(p != 0)),
        0x0B if p >= 28 => (Fx::Tempo, p),
        0x0C if p != 0 => (Fx::FineVolSlideUp, p.min(64)),
        0x0D if p != 0 => (Fx::FineVolSlideDown, p.min(64)),
        0x0E if p != 0 => (Fx::VolSlide, p.min(15) << 4),
        0x0F if p != 0 => (Fx::VolSlide, p.min(15)),
        0x14 if p != 0 => (Fx::ModEx, 0x60),
        0x15 => (Fx::ModEx, 0x60 | p.min(15)),
        0x16 if p != 0 => (Fx::ModEx, 0x90 | p.min(15)),
        0x17 => (Fx::ModEx, 0xD0 | p.min(15)),
        0x18 => (Fx::ModEx, 0xC0 | p.min(15)),
        0x19 => (Fx::Offset, p),
        0x1A => (Fx::SetFinetune, p & 0x0F),
        0x1B => (Fx::TonePorta, p),
        0x1C => (Fx::TonePortaVol, p.min(15) << 4),
        0x1D => (Fx::TonePortaVol, p.min(15)),
        _ => (Fx::None, 0),
    }
}

pub fn load_dss(d: &[u8]) -> Result<Module> {
    let (tempo, speed) = (d[8], d[9]);
    let nord = usize::from(u16::from_be_bytes([d[0x59C], d[0x59D]])).min(128);
    let orders: Vec<u16> = d[0x59E..0x59E + nord].iter().map(|&o| u16::from(o)).collect();
    let npat = usize::from(orders.iter().copied().max().unwrap_or(0)) + 1;
    let mut patterns = Vec::with_capacity(npat);
    for p in 0..npat {
        let mut pat = Pattern::new(64, 4);
        for row in 0..64 {
            for ch in 0..4 {
                let at = 0x61E + p * 1024 + row * 16 + ch * 4;
                let c = &d[at..at + 4];
                let smp = c[0] >> 3;
                let period = u16::from(c[0] & 7) << 8 | u16::from(c[1]);
                let mut cell = Cell::default();
                if smp != 0 && period != 0 {
                    cell.instr = smp;
                    cell.note = if period == 0x7FF { NOTE_CUT } else { super::common::period_to_note(period).unwrap_or(NOTE_NONE) };
                } else if smp != 0 {
                    cell.instr = smp;
                }
                (cell.fx, cell.param) = dss_effect(c[2], c[3]);
                if let Some(x) = pat.cell_mut(row, ch) {
                    *x = cell;
                }
            }
        }
        patterns.push(Arc::new(pat));
    }
    let mut r = Reader::at(d, 0x61E + npat * 1024);
    let mut samples = Vec::with_capacity(31);
    for i in 0..31 {
        let h = &d[10 + i * 46..10 + i * 46 + 46];
        let w = |o: usize| u32::from(u16::from_be_bytes([h[o], h[o + 1]]));
        let l = |o: usize| u32::from_be_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]);
        let start = (l(30) & !1) as usize;
        let (len, loop_start, loop_len) = (w(34), l(36), w(40));
        let mut s = Sample { name: crate::io::text(&h[..30]), c5: super::common::finetune_c5(h[42]), volume: h[43].min(64), ..Sample::empty() };
        if len != 0 {
            let body = (len + if loop_len > 1 { loop_len } else { 0 }) as usize * 2;
            let b = r.bytes_upto(start + body);
            let b = b.get(start.min(b.len())..).unwrap_or(&[]);
            s.wave = Arc::new(Wave::new(8, 1, decode(b, Pcm::S8, b.len())));
            if loop_len > 1 {
                s.looped = Loop::forward(loop_start, loop_start + loop_len * 2);
            }
        }
        samples.push(s);
    }
    let mut m = Module {
        format: "dss".into(),
        format_name: "Digital Sound Studio".into(),
        tracker: "Digital Sound Studio".into(),
        kind: Kind::Mod,
        flags: Flags { amiga_limits: true, ..Default::default() },
        speed: speed.max(1),
        tempo: if tempo == 0 { 125 } else { u16::from(tempo) },
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        channels: (0..4).map(|c| Channel { pan: super::common::amiga_pan(c), ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- StarTrekker's AM instruments ----
//
// StarTrekker saved its AM synth's instruments beside the module, in a
// file named as the module with .NT after ("ST1.2 ModuleINFO",
// "ST1.3 ModuleINFO", or AudioSculpture's "AudioSculpture10"): 120 bytes
// a sample from 144, "AM" marking those that are synths. Each takes its
// sample's place: neumetik-synth's AM cycle looped (`am.rs`), its
// envelope the instrument's volume envelope, its vibrato the sample's
// auto-vibrato, its octave the sample's rate. The pitch fall isn't kept.

/// The AM instruments from the module's .NT file, when there is one.
pub fn startrekker_am(m: &mut Module, data: &[u8], name: &str, companion: super::Companion) {
    if !matches!(data.get(1080..1084), Some(b"FLT4" | b"FLT8" | b"EXO4" | b"EXO8")) {
        return;
    }
    let Some(nt) = companion(&format!("{name}.nt")).or_else(|| companion(&format!("{name}.as"))) else { return };
    let tracker = match nt.get(..16) {
        Some(b"ST1.2 ModuleINFO") => "StarTrekker 1.2",
        Some(b"ST1.3 ModuleINFO") => "StarTrekker 1.3",
        Some(b"AudioSculpture10") => "AudioSculpture 1.0",
        _ => return,
    };
    let mut any = false;
    let mut envelopes = vec![Envelope::default(); m.samples.len()];
    for (i, s) in m.samples.iter_mut().enumerate().take(31) {
        let at = 144 + i * 120;
        let Some(b) = nt.get(at..at + 36) else { break };
        if &b[..2] != b"AM" || u16::from_be_bytes([b[26], b[27]]) > 3 {
            continue;
        }
        let am = neumetik_synth::am::AmSound::from_nt(&b[6..36]);
        let wave = am.waveform();
        let n = wave.len() as u32;
        s.wave = Arc::new(Wave::new(8, 1, wave.iter().map(|&x| i16::from(x) << 8).collect()));
        s.looped = Loop::forward(0, n);
        s.c5 = (f64::from(s.c5) / 2f64.powi(i32::from(am.octave.min(8)))).round().max(1.0) as u32;
        s.volume = 64;
        s.vibrato = AutoVibrato { kind: 0, speed: am.vibrato_speed.min(255) as u8, depth: (am.vibrato_depth * 4).min(255) as u8, sweep: 0 };
        s.credit = format!("{} AM synth: {}", tracker, neumetik_synth::am::NAMES[usize::from(am.wave)]);
        let (pts, hold) = am.envelope();
        envelopes[i] = Envelope { enabled: true, points: pts.iter().map(|&(t, v)| EnvPoint { tick: t, value: v as i8 }).collect(), sustain: hold.map(|h| (h as u8, h as u8)), ..Default::default() };
        any = true;
    }
    if !any {
        return;
    }
    m.flags.instruments = true;
    m.instruments = m.samples.iter().enumerate().map(|(i, s)| Instrument { volume_env: envelopes[i].clone(), ..Instrument::for_sample(&s.name, i as u16 + 1) }).collect();
    m.tracker = format!("{tracker} (AM synth)");
}
