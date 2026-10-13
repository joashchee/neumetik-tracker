//! **Wavepack**: lossless packing made for sampled sound, the
//! repository's second way to keep a waveform (`repo::pack_wave` keeps
//! whichever of this and `lz` comes out smaller).
//!
//! Each channel is predicted point by point, and only what the
//! prediction missed is kept:
//!
//! 1. the first difference (each point less the one before);
//! 2. a cascade of two adaptive filters, sign-sign LMS of 64 and then
//!    16 taps: each guesses the next value from the values before it and
//!    moves each weight a step toward what would have guessed better
//!    (the adaptive filter of Widrow and Hoff, 1960, in its sign-sign
//!    form, as decades of lossless audio coders have used it);
//! 3. what's left, coded by the adaptive binary range coder `lz` uses:
//!    each residual's bit length under a context of how large the recent
//!    ones were, then its two top bits modeled and the rest as they are.
//!
//! Encoder and decoder run the same integer filters (wrapping
//! arithmetic, so every machine makes the same guesses), so it's exact.
//! On the test corpus (`examples/pack_study.rs`, 3,043 waveforms, 40 MB
//! at their own depth) it keeps 55.5% of the bytes where `lz` keeps
//! 59.4%; kept only where it wins (as the repository does), 51.9%, an
//! eighth less storage. It unpacks at about half `lz`'s speed (18 MB/s
//! on Apple silicon), paid when a song's samples load, never as it plays.
//! Longer filters gain little more (256 taps: 51.5%, slower still).
//!
//! A stream: a byte of flags (bit 0: the points are 8-bit, kept shifted
//! down by 8), then the coded residuals, a channel at a time.

use crate::lz::{Decoder, Encoder, ONE};

/// The filters: taps, and the shift that scales their sum.
const STAGES: [(usize, u32); 2] = [(64, 11), (16, 9)];

/// A sign-sign LMS filter.
struct Lms {
    n: usize,
    shift: u32,
    w: Vec<i32>,
    /// The inputs, newest first from `pos`, and their signs, kept twice
    /// over so the newest `n` are always one slice.
    h: Vec<i32>,
    sign: Vec<i32>,
    pos: usize,
}

impl Lms {
    fn new(n: usize, shift: u32) -> Lms {
        Lms { n, shift, w: vec![0; n], h: vec![0; n * 2], sign: vec![0; n * 2], pos: n }
    }

    fn guess(&self) -> i32 {
        dot(&self.w, &self.h[self.pos..self.pos + self.n]) >> self.shift
    }

    /// Learns from the error it made, then takes in its input.
    fn learn(&mut self, err: i32, input: i32) {
        let sign = &self.sign[self.pos..self.pos + self.n];
        if err > 0 {
            for (w, s) in self.w.iter_mut().zip(sign) {
                *w = w.wrapping_add(*s);
            }
        } else if err < 0 {
            for (w, s) in self.w.iter_mut().zip(sign) {
                *w = w.wrapping_sub(*s);
            }
        }
        if self.pos == 0 {
            let n = self.n;
            self.h.copy_within(0..n, n);
            self.sign.copy_within(0..n, n);
            self.pos = n;
        }
        self.pos -= 1;
        let v = input.clamp(-32768, 32767);
        self.h[self.pos] = v;
        self.sign[self.pos] = v.signum();
    }
}

/// The sum of `w` times `h`, wrapping: in eight lanes added at the end,
/// which wrapping makes the same sum in any order (and lets it run as
/// vector instructions).
fn dot(w: &[i32], h: &[i32]) -> i32 {
    let mut lanes = [0i32; 8];
    let (wc, hc) = (w.chunks_exact(8), h.chunks_exact(8));
    let (wr, hr) = (wc.remainder(), hc.remainder());
    for (a, b) in wc.zip(hc) {
        for j in 0..8 {
            lanes[j] = lanes[j].wrapping_add(a[j].wrapping_mul(b[j]));
        }
    }
    let mut s = lanes.iter().fold(0i32, |s, &l| s.wrapping_add(l));
    for (a, b) in wr.iter().zip(hr) {
        s = s.wrapping_add(a.wrapping_mul(*b));
    }
    s
}

struct Predictor {
    prev: i32,
    stages: Vec<Lms>,
}

impl Predictor {
    fn new() -> Predictor {
        Predictor { prev: 0, stages: STAGES.iter().map(|&(n, s)| Lms::new(n, s)).collect() }
    }

    /// The residual for `x`, learning from it.
    fn residual(&mut self, x: i32) -> i32 {
        let mut v = x.wrapping_sub(self.prev);
        self.prev = x;
        for st in &mut self.stages {
            let r = v.wrapping_sub(st.guess());
            st.learn(r, v);
            v = r;
        }
        v
    }

    /// The point a residual stands for, learning from it.
    fn point(&mut self, r: i32) -> i32 {
        let mut guesses = [0i32; STAGES.len()];
        for (g, st) in guesses.iter_mut().zip(&self.stages) {
            *g = st.guess();
        }
        // Undone last stage first: each stage's input is its residual plus its guess.
        let mut inputs = [0i32; STAGES.len()];
        let mut v = r;
        for i in (0..self.stages.len()).rev() {
            inputs[i] = v.wrapping_add(guesses[i]);
            v = inputs[i];
        }
        for (i, st) in self.stages.iter_mut().enumerate() {
            st.learn(inputs[i].wrapping_sub(guesses[i]), inputs[i]);
        }
        let x = v.wrapping_add(self.prev);
        self.prev = x;
        x
    }
}

/// How large the recent residuals were, in 24 steps.
const CTX: usize = 24;

struct Residuals {
    k: Vec<[u16; 32]>,
    top: [[u16; 4]; 32],
    avg: u32,
}

impl Residuals {
    fn new() -> Residuals {
        Residuals { k: vec![[ONE / 2; 32]; CTX], top: [[ONE / 2; 4]; 32], avg: 16 }
    }

    fn ctx(&self) -> usize {
        ((32 - self.avg.leading_zeros()) as usize).min(CTX - 1)
    }

    fn learn(&mut self, u: u32) {
        self.avg = (self.avg * 7 + (u.min(1 << 20) << 4)) / 8 + 1;
    }

    fn put(&mut self, e: &mut Encoder, r: i32) {
        let u = ((r << 1) ^ (r >> 31)) as u32;
        let k = 32 - u.leading_zeros();
        let c = self.ctx();
        let mut node = 1usize;
        for i in (0..5).rev() {
            let b = (k >> i) & 1;
            e.bit(&mut self.k[c][node], b);
            node = node * 2 + b as usize;
        }
        if k > 1 {
            let rest = k - 1;
            let hi = (u >> (rest - 1)) & 1;
            e.bit(&mut self.top[k as usize][1], hi);
            if rest >= 2 {
                e.bit(&mut self.top[k as usize][2 + hi as usize], (u >> (rest - 2)) & 1);
                e.direct(u & ((1 << (rest - 2)) - 1), rest - 2);
            }
        }
        self.learn(u);
    }

    fn get(&mut self, d: &mut Decoder) -> i32 {
        let c = self.ctx();
        let mut node = 1usize;
        for _ in 0..5 {
            node = node * 2 + d.bit(&mut self.k[c][node]) as usize;
        }
        let k = (node - 32) as u32;
        let u = match k {
            0 => 0,
            1 => 1,
            _ => {
                let rest = k - 1;
                let hi = d.bit(&mut self.top[k as usize][1]);
                let mut u = 1u32 << rest | hi << (rest - 1);
                if rest >= 2 {
                    u |= d.bit(&mut self.top[k as usize][2 + hi as usize]) << (rest - 2);
                    u |= d.direct(rest - 2);
                }
                u
            }
        };
        self.learn(u);
        (u >> 1) as i32 ^ -((u & 1) as i32)
    }
}

/// Packs interleaved points (`channels` of them a frame).
pub fn pack(pcm: &[i16], channels: usize) -> Vec<u8> {
    let channels = channels.max(1);
    let eight = pcm.iter().all(|&v| v & 0xFF == 0);
    let shift = if eight { 8 } else { 0 };
    let mut e = Encoder::new(pcm.len() / 2 + 16);
    for c in 0..channels {
        let mut p = Predictor::new();
        let mut r = Residuals::new();
        for &v in pcm.iter().skip(c).step_by(channels) {
            r.put(&mut e, p.residual(i32::from(v) >> shift));
        }
    }
    let mut out = vec![u8::from(eight)];
    out.extend_from_slice(&e.finish());
    out
}

/// Unpacks `frames` frames of `channels` points. A damaged stream gives
/// an error (or points that won't match their waveform's ID), never a
/// panic.
pub fn unpack(b: &[u8], channels: usize, frames: usize) -> Result<Vec<i16>, String> {
    let channels = channels.max(1);
    let (&flags, body) = b.split_first().ok_or("it's empty")?;
    let shift = if flags & 1 != 0 { 8 } else { 0 };
    let mut d = Decoder::new(body);
    let mut out = vec![0i16; frames.checked_mul(channels).ok_or("its length is out of range")?];
    for c in 0..channels {
        let mut p = Predictor::new();
        let mut r = Residuals::new();
        for i in 0..frames {
            let x = p.point(r.get(&mut d));
            out[i * channels + c] = (x.clamp(i32::from(i16::MIN) >> shift, i32::from(i16::MAX) >> shift) << shift) as i16;
            if d.overrun() {
                return Err("its points are cut short".into());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(pcm: &[i16], ch: usize) -> usize {
        let p = pack(pcm, ch);
        assert_eq!(unpack(&p, ch, pcm.len() / ch).unwrap(), pcm);
        p.len()
    }

    #[test]
    fn it_comes_back_exactly() {
        round(&[], 1);
        round(&[5], 1);
        let sine: Vec<i16> = (0..20_000).map(|i| ((i as f32 * 0.05).sin() * 20_000.0) as i16).collect();
        let n = round(&sine, 1);
        assert!(n < sine.len(), "a sine packs to under half its 16-bit bytes: {n}");
        let eight: Vec<i16> = sine.iter().map(|v| v & !0xFF).collect();
        round(&eight, 1);
        let mut noise = 1u32;
        let wild: Vec<i16> = (0..9_000)
            .map(|_| {
                noise ^= noise << 13;
                noise ^= noise >> 17;
                noise ^= noise << 5;
                noise as i16
            })
            .collect();
        round(&wild, 1);
        round(&wild, 2);
        round(&[i16::MIN, i16::MAX, i16::MIN, i16::MAX, 0, -1], 2);
    }

    #[test]
    fn damage_is_an_error_or_a_difference_never_a_panic() {
        let sine: Vec<i16> = (0..5_000).map(|i| ((i as f32 * 0.03).sin() * 9_000.0) as i16).collect();
        let p = pack(&sine, 1);
        for cut in [0, 1, 2, p.len() / 2] {
            let _ = unpack(&p[..cut], 1, sine.len());
        }
        let mut bad = p.clone();
        for i in (1..bad.len()).step_by(7) {
            bad[i] ^= 0x5A;
        }
        let _ = unpack(&bad, 1, sine.len());
    }
}
