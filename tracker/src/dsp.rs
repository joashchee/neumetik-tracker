//! The built-in effects a song's effect slots play through (`EffectSlot`
//! in the model): reverb, delay, chorus, flanger, distortion, a
//! compressor, an equalizer, a filter and gargle, each from its public,
//! decades-old design (Schroeder and Moorer's reverb as Freeverb tunes
//! it, RBJ's biquads, a state-variable filter). They're what OpenMPT's
//! DirectX Media Objects and Renoise's DSP effects do, built in so a
//! song plays the same anywhere.
//!
//! A `Rack` is a song's slots made ready to play: their delay lines and
//! buffers allocated once, when it's made, so `process` never allocates
//! and can run in an audio callback. A slot's sound goes into a later
//! slot (a chain) or the mix.

use std::f32::consts::PI;

use crate::model::{EffectKind, EffectSlot};

/// The most frames a rack takes in one go.
pub const BLOCK: usize = 512;

/// A delay line of a fixed length, read at any distance behind.
#[derive(Clone)]
struct Line {
    buf: Vec<f32>,
    at: usize,
}

impl Line {
    fn new(len: usize) -> Line {
        Line { buf: vec![0.0; len.max(2)], at: 0 }
    }

    fn push(&mut self, v: f32) {
        self.at = (self.at + 1) % self.buf.len();
        self.buf[self.at] = v;
    }

    /// The value `d` frames ago (0 the last pushed), between frames.
    fn tap(&self, d: f32) -> f32 {
        let n = self.buf.len();
        let d = d.clamp(0.0, (n - 2) as f32);
        let i = d as usize;
        let f = d - i as f32;
        let a = self.buf[(self.at + n - i) % n];
        let b = self.buf[(self.at + n - i - 1) % n];
        a + (b - a) * f
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

/// Freeverb's comb filter: a delay with a damped feedback.
#[derive(Clone)]
struct Comb {
    buf: Vec<f32>,
    at: usize,
    store: f32,
}

impl Comb {
    fn run(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let y = self.buf[self.at];
        self.store = y * (1.0 - damp) + self.store * damp;
        self.buf[self.at] = x + self.store * feedback;
        self.at = (self.at + 1) % self.buf.len();
        y
    }
}

#[derive(Clone)]
struct Allpass {
    buf: Vec<f32>,
    at: usize,
}

impl Allpass {
    fn run(&mut self, x: f32) -> f32 {
        let b = self.buf[self.at];
        let y = -x + b;
        self.buf[self.at] = x + b * 0.5;
        self.at = (self.at + 1) % self.buf.len();
        y
    }
}

/// Freeverb's tunings, in frames at 44.1 kHz; the right side is 23 longer.
const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const SPREAD: usize = 23;

#[derive(Clone)]
struct Reverb {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
    pre: [Line; 2],
}

impl Reverb {
    fn new(rate: u32) -> Reverb {
        let k = rate as f32 / 44_100.0;
        let side = |s: usize| {
            (
                COMBS.iter().map(|&n| Comb { buf: vec![0.0; ((n + s) as f32 * k) as usize + 1], at: 0, store: 0.0 }).collect(),
                ALLPASSES.iter().map(|&n| Allpass { buf: vec![0.0; ((n + s) as f32 * k) as usize + 1], at: 0 }).collect(),
            )
        };
        let (cl, al) = side(0);
        let (cr, ar) = side(SPREAD);
        let pre = Line::new((rate as f32 * 0.1) as usize + 2);
        Reverb { combs: [cl, cr], allpasses: [al, ar], pre: [pre.clone(), pre] }
    }
}

/// RBJ's biquad, direct form I.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    x: [[f32; 2]; 2],
    y: [[f32; 2]; 2],
}

impl Biquad {
    fn set(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        self.b = [b0 / a0, b1 / a0, b2 / a0];
        self.a = [a1 / a0, a2 / a0];
    }

    /// Peaking (kind 0), low shelf (1) or high shelf (2), at `f` hertz.
    fn shape(&mut self, kind: u8, f: f32, gain_db: f32, q: f32, rate: u32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w = 2.0 * PI * (f / rate as f32).min(0.49);
        let (sn, cs) = w.sin_cos();
        let alpha = sn / (2.0 * q);
        match kind {
            0 => self.set(1.0 + alpha * a, -2.0 * cs, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cs, 1.0 - alpha / a),
            1 => {
                let r = 2.0 * a.sqrt() * alpha;
                self.set(a * ((a + 1.0) - (a - 1.0) * cs + r), 2.0 * a * ((a - 1.0) - (a + 1.0) * cs), a * ((a + 1.0) - (a - 1.0) * cs - r), (a + 1.0) + (a - 1.0) * cs + r, -2.0 * ((a - 1.0) + (a + 1.0) * cs), (a + 1.0) + (a - 1.0) * cs - r)
            }
            _ => {
                let r = 2.0 * a.sqrt() * alpha;
                self.set(a * ((a + 1.0) + (a - 1.0) * cs + r), -2.0 * a * ((a - 1.0) + (a + 1.0) * cs), a * ((a + 1.0) + (a - 1.0) * cs - r), (a + 1.0) - (a - 1.0) * cs + r, 2.0 * ((a - 1.0) - (a + 1.0) * cs), (a + 1.0) - (a - 1.0) * cs - r)
            }
        }
    }

    fn run(&mut self, c: usize, v: f32) -> f32 {
        let [x1, x2] = self.x[c];
        let [y1, y2] = self.y[c];
        let y = self.b[0] * v + self.b[1] * x1 + self.b[2] * x2 - self.a[0] * y1 - self.a[1] * y2;
        self.x[c] = [v, x1];
        self.y[c] = [y, y1];
        y
    }
}

/// A state-variable filter (Simper's), low-pass or high-pass.
#[derive(Clone, Copy, Default)]
struct Svf {
    g: f32,
    k: f32,
    s: [[f32; 2]; 2],
}

impl Svf {
    fn set(&mut self, f: f32, q: f32, rate: u32) {
        self.g = (PI * (f / rate as f32).min(0.49)).tan();
        self.k = 1.0 / q.max(0.3);
    }

    fn run(&mut self, c: usize, v: f32, high: bool) -> f32 {
        let [ic1, ic2] = self.s[c];
        let (g, k) = (self.g, self.k);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let v1 = a1 * ic1 + g * a1 * (v - ic2);
        let v2 = ic2 + g * v1;
        self.s[c] = [2.0 * v1 - ic1, 2.0 * v2 - ic2];
        if high {
            v - k * v1 - v2
        } else {
            v2
        }
    }
}

#[derive(Clone)]
enum Unit {
    Reverb(Box<Reverb>),
    /// Delay: two lines and their damping's memory.
    Delay([Line; 2], [f32; 2]),
    /// Chorus and flanger: a line each side, and the LFO's phase.
    Modulated([Line; 2], f32),
    /// Distortion: the tone filter's memory.
    Distortion([f32; 2]),
    /// Compressor: the envelope follower.
    Compressor(f32),
    Equalizer([Biquad; 3]),
    Filter(Svf),
    /// Gargle: the modulator's phase.
    Gargle(f32),
}

impl Unit {
    fn new(kind: EffectKind, rate: u32) -> Unit {
        let secs = |s: f32| (rate as f32 * s) as usize + 4;
        match kind {
            EffectKind::Reverb => Unit::Reverb(Box::new(Reverb::new(rate))),
            EffectKind::Delay => Unit::Delay([Line::new(secs(2.0)), Line::new(secs(2.0))], [0.0; 2]),
            EffectKind::Chorus => Unit::Modulated([Line::new(secs(0.05)), Line::new(secs(0.05))], 0.0),
            EffectKind::Flanger => Unit::Modulated([Line::new(secs(0.02)), Line::new(secs(0.02))], 0.0),
            EffectKind::Distortion => Unit::Distortion([0.0; 2]),
            EffectKind::Compressor => Unit::Compressor(0.0),
            EffectKind::Equalizer => Unit::Equalizer([Biquad::default(); 3]),
            EffectKind::Filter => Unit::Filter(Svf::default()),
            EffectKind::Gargle => Unit::Gargle(0.0),
        }
    }
}

/// A slot ready to play.
struct Slot {
    kind: EffectKind,
    unit: Unit,
    params: [f32; 8],
    mix: f32,
    bypass: bool,
    output: Option<usize>,
    /// What's come in this block.
    input: [Vec<f32>; 2],
    /// It's been given sound since it last went quiet (an idle slot is passed over).
    awake: u32,
}

/// A song's effect slots, ready to play.
pub struct Rack {
    slots: Vec<Slot>,
    rate: u32,
}

impl Rack {
    /// The slots made ready: everything they'll need allocated now.
    pub fn new(slots: &[EffectSlot], rate: u32) -> Rack {
        let mut r = Rack {
            slots: slots
                .iter()
                .map(|s| Slot { kind: s.kind, unit: Unit::new(s.kind, rate), params: [0.0; 8], mix: 1.0, bypass: false, output: None, input: [vec![0.0; BLOCK], vec![0.0; BLOCK]], awake: 0 })
                .collect(),
            rate: rate.max(8000),
        };
        r.update(slots);
        r
    }

    /// Made for these slots: the same effects in the same places.
    pub fn fits(&self, slots: &[EffectSlot]) -> bool {
        self.slots.len() == slots.len() && self.slots.iter().zip(slots).all(|(a, b)| a.kind == b.kind)
    }

    /// New settings for the same slots (`fits`).
    pub fn update(&mut self, slots: &[EffectSlot]) {
        let n = self.slots.len();
        for (i, (s, e)) in self.slots.iter_mut().zip(slots).enumerate() {
            for (k, p) in s.params.iter_mut().enumerate() {
                *p = e.param(k);
            }
            s.mix = e.mix.clamp(0.0, 1.0);
            s.bypass = e.bypass;
            // Only into a later slot, so a chain never comes round.
            s.output = e.output.map(usize::from).filter(|&o| o > i && o < n);
        }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// One parameter moved (automation, a MIDI macro), 0 to 1.
    pub fn set_param(&mut self, slot: usize, param: usize, v: f32) {
        if let Some(p) = self.slots.get_mut(slot).and_then(|s| s.params.get_mut(param)) {
            *p = v.clamp(0.0, 1.0);
        }
    }

    pub fn param(&self, slot: usize, param: usize) -> Option<f32> {
        self.slots.get(slot).and_then(|s| s.params.get(param).copied())
    }

    /// A slot's input, `n` frames, for voices to add into.
    pub fn input(&mut self, slot: usize, n: usize) -> Option<(&mut [f32], &mut [f32])> {
        let s = self.slots.get_mut(slot)?;
        s.awake = 4;
        let [l, r] = &mut s.input;
        Some((&mut l[..n], &mut r[..n]))
    }

    /// Every slot quiet, its tails gone.
    pub fn clear(&mut self) {
        for s in &mut self.slots {
            s.input[0].fill(0.0);
            s.input[1].fill(0.0);
            s.awake = 0;
            match &mut s.unit {
                Unit::Reverb(rv) => {
                    for c in 0..2 {
                        for comb in &mut rv.combs[c] {
                            comb.buf.fill(0.0);
                            comb.store = 0.0;
                        }
                        for ap in &mut rv.allpasses[c] {
                            ap.buf.fill(0.0);
                        }
                        rv.pre[c].clear();
                    }
                }
                Unit::Delay(lines, lp) => {
                    lines.iter_mut().for_each(Line::clear);
                    *lp = [0.0; 2];
                }
                Unit::Modulated(lines, _) => lines.iter_mut().for_each(Line::clear),
                Unit::Distortion(t) => *t = [0.0; 2],
                Unit::Compressor(e) => *e = 0.0,
                Unit::Equalizer(bands) => {
                    for b in bands.iter_mut() {
                        b.x = [[0.0; 2]; 2];
                        b.y = [[0.0; 2]; 2];
                    }
                }
                Unit::Filter(f) => f.s = [[0.0; 2]; 2],
                Unit::Gargle(_) => {}
            }
        }
    }

    /// Plays every slot over the `n` frames its input has, in order, each
    /// into the slot after it or `out`, and empties the inputs.
    pub fn process(&mut self, n: usize, out_l: &mut [f32], out_r: &mut [f32]) {
        let n = n.min(BLOCK).min(out_l.len()).min(out_r.len());
        let rate = self.rate;
        for i in 0..self.slots.len() {
            let (head, tail) = self.slots.split_at_mut(i + 1);
            let s = &mut head[i];
            let loud = s.input[0][..n].iter().chain(&s.input[1][..n]).any(|v| v.abs() > 1e-7);
            if loud {
                // Tails ring on for a while after the sound stops.
                s.awake = (rate * 10 / n.max(1) as u32).max(4);
            } else if s.awake == 0 {
                continue;
            } else {
                s.awake -= 1;
            }
            let [il, ir] = &mut s.input;
            if !s.bypass {
                run(s.kind, &mut s.unit, &s.params, s.mix, rate, &mut il[..n], &mut ir[..n]);
            }
            let (dl, dr): (&mut [f32], &mut [f32]) = match s.output.and_then(|o| tail.get_mut(o - i - 1)) {
                Some(next) => {
                    next.awake = next.awake.max(4);
                    let [a, b] = &mut next.input;
                    (&mut a[..n], &mut b[..n])
                }
                None => (&mut out_l[..n], &mut out_r[..n]),
            };
            for k in 0..n {
                dl[k] += il[k];
                dr[k] += ir[k];
            }
            il[..n].fill(0.0);
            ir[..n].fill(0.0);
        }
    }
}

/// Seconds from a 0-to-1 knob, between `lo` and `hi` on a log scale.
fn log_range(v: f32, lo: f32, hi: f32) -> f32 {
    lo * (hi / lo).powf(v.clamp(0.0, 1.0))
}

/// One effect over a block, in place, mixed with what came in.
fn run(kind: EffectKind, unit: &mut Unit, p: &[f32; 8], mix: f32, rate: u32, l: &mut [f32], r: &mut [f32]) {
    let fs = rate as f32;
    let dry = 1.0 - mix;
    match unit {
        Unit::Reverb(rv) => {
            let feedback = 0.7 + p[0] * 0.28;
            let damp = p[1] * 0.4;
            let width = p[2];
            let pre = p[3] * 0.1 * fs;
            let (w1, w2) = ((1.0 + width) / 2.0, (1.0 - width) / 2.0);
            for k in 0..l.len() {
                let x = (l[k] + r[k]) * 0.015;
                let mut out = [0.0f32; 2];
                for (c, o) in out.iter_mut().enumerate() {
                    rv.pre[c].push(x);
                    let x = rv.pre[c].tap(pre);
                    let mut acc = 0.0;
                    for comb in &mut rv.combs[c] {
                        acc += comb.run(x, feedback, damp);
                    }
                    for ap in &mut rv.allpasses[c] {
                        acc = ap.run(acc);
                    }
                    *o = acc;
                }
                let (wl, wr) = (out[0] * w1 + out[1] * w2, out[1] * w1 + out[0] * w2);
                l[k] = l[k] * dry + wl * mix * 3.0;
                r[k] = r[k] * dry + wr * mix * 3.0;
            }
        }
        Unit::Delay(lines, lp) => {
            let tl = (log_range(p[0], 0.001, 2.0) * fs).max(1.0);
            let tr = (log_range(p[1], 0.001, 2.0) * fs).max(1.0);
            let fb = p[2] * 0.95;
            let ping = p[3] >= 0.5;
            let damp = p[4] * 0.9;
            for k in 0..l.len() {
                let (yl, yr) = (lines[0].tap(tl - 1.0), lines[1].tap(tr - 1.0));
                lp[0] = yl * (1.0 - damp) + lp[0] * damp;
                lp[1] = yr * (1.0 - damp) + lp[1] * damp;
                let (fl, fr) = if ping { (lp[1], lp[0]) } else { (lp[0], lp[1]) };
                let (xl, xr) = if ping { ((l[k] + r[k]) * 0.5, 0.0) } else { (l[k], r[k]) };
                lines[0].push(xl + fl * fb);
                lines[1].push(xr + fr * fb);
                l[k] = l[k] * dry + yl * mix;
                r[k] = r[k] * dry + yr * mix;
            }
        }
        Unit::Modulated(lines, phase) => {
            let flanger = kind == EffectKind::Flanger;
            let rate_hz = if flanger { log_range(p[0], 0.05, 2.0) } else { log_range(p[0], 0.05, 5.0) };
            let (base, depth) = if flanger { (0.0005 + p[2] * 0.005, p[1] * 0.004) } else { (0.005 + p[2] * 0.025, p[1] * 0.01) };
            let fb = p[3] * if flanger { 0.95 } else { 0.6 };
            let step = rate_hz / fs;
            for k in 0..l.len() {
                *phase = (*phase + step) % 1.0;
                let lfo = |off: f32| ((*phase + off) * 2.0 * PI).sin() * 0.5 + 0.5;
                let dl = (base + depth * lfo(0.0)) * fs;
                let dr = (base + depth * lfo(0.25)) * fs;
                let (yl, yr) = (lines[0].tap(dl), lines[1].tap(dr));
                lines[0].push(l[k] + yl * fb);
                lines[1].push(r[k] + yr * fb);
                l[k] = l[k] * dry + yl * mix;
                r[k] = r[k] * dry + yr * mix;
            }
        }
        Unit::Distortion(tone) => {
            let drive = 1.0 + p[0] * p[0] * 49.0;
            let cut = log_range(p[1], 500.0, 16_000.0);
            let a = 1.0 - (-2.0 * PI * cut / fs).exp();
            let level = p[2] * 1.5;
            for k in 0..l.len() {
                for (c, v) in [&mut l[k], &mut r[k]].into_iter().enumerate() {
                    let y = (*v * drive).tanh();
                    tone[c] += (y - tone[c]) * a;
                    *v = *v * dry + tone[c] * level * mix;
                }
            }
        }
        Unit::Compressor(env) => {
            let threshold = 10f32.powf((-60.0 + p[0] * 60.0) / 20.0);
            let ratio = 1.0 + p[1] * 19.0;
            let attack = (-1.0 / (log_range(p[2], 0.0001, 0.1) * fs)).exp();
            let release = (-1.0 / (log_range(p[3], 0.01, 1.0) * fs)).exp();
            let makeup = 10f32.powf(p[4] * 24.0 / 20.0);
            for k in 0..l.len() {
                let peak = l[k].abs().max(r[k].abs());
                let coef = if peak > *env { attack } else { release };
                *env = peak + (*env - peak) * coef;
                let gain = if *env > threshold { (threshold / *env).powf(1.0 - 1.0 / ratio) } else { 1.0 } * makeup;
                l[k] = l[k] * dry + l[k] * gain * mix;
                r[k] = r[k] * dry + r[k] * gain * mix;
            }
        }
        Unit::Equalizer(bands) => {
            let db = |v: f32| (v - 0.5) * 30.0;
            bands[0].shape(1, 120.0, db(p[0]), 0.707, rate);
            bands[1].shape(0, log_range(p[2], 200.0, 5000.0), db(p[1]), 1.0, rate);
            bands[2].shape(2, 8000.0, db(p[3]), 0.707, rate);
            for k in 0..l.len() {
                for (c, v) in [&mut l[k], &mut r[k]].into_iter().enumerate() {
                    let mut y = *v;
                    for b in bands.iter_mut() {
                        y = b.run(c, y);
                    }
                    *v = *v * dry + y * mix;
                }
            }
        }
        Unit::Filter(f) => {
            f.set(log_range(p[0], 20.0, 20_000.0), 0.5 + p[1] * p[1] * 19.5, rate);
            let high = p[2] >= 0.5;
            for k in 0..l.len() {
                for (c, v) in [&mut l[k], &mut r[k]].into_iter().enumerate() {
                    let y = f.run(c, *v, high);
                    *v = *v * dry + y * mix;
                }
            }
        }
        Unit::Gargle(phase) => {
            let hz = log_range(p[0], 20.0, 1000.0);
            let square = p[1] >= 0.5;
            for k in 0..l.len() {
                *phase = (*phase + hz / fs) % 1.0;
                let m = if square {
                    if *phase < 0.5 {
                        1.0
                    } else {
                        0.0
                    }
                } else {
                    1.0 - (2.0 * *phase - 1.0).abs()
                };
                l[k] = l[k] * dry + l[k] * m * mix;
                r[k] = r[k] * dry + r[k] * m * mix;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse(kind: EffectKind, mix: f32) -> Vec<f32> {
        let mut slot = EffectSlot::new(kind);
        slot.mix = mix;
        let mut rack = Rack::new(&[slot], 44_100);
        let mut out = Vec::new();
        for b in 0..40 {
            let (il, ir) = rack.input(0, BLOCK).unwrap();
            if b == 0 {
                il[0] = 1.0;
                ir[0] = 1.0;
            }
            let (mut l, mut r) = (vec![0f32; BLOCK], vec![0f32; BLOCK]);
            rack.process(BLOCK, &mut l, &mut r);
            out.extend_from_slice(&l);
        }
        out
    }

    #[test]
    fn every_effect_stays_finite_and_the_echoes_come_late() {
        for kind in EffectKind::ALL {
            let out = impulse(kind, 1.0);
            assert!(out.iter().all(|v| v.is_finite() && v.abs() < 50.0), "{kind:?}");
        }
        // A delay of 0.3 on the knob is log-spaced: about 10 ms.
        let out = impulse(EffectKind::Delay, 1.0);
        let first = out.iter().position(|v| v.abs() > 0.1).unwrap();
        let expect = log_range(0.3, 0.001, 2.0) * 44_100.0;
        assert!((first as f32 - expect).abs() < 3.0, "{first} {expect}");
        // Reverb rings on long after.
        let out = impulse(EffectKind::Reverb, 1.0);
        assert!(out[10_000..15_000].iter().any(|v| v.abs() > 1e-4));
    }

    #[test]
    fn a_chain_runs_one_slot_into_the_next_and_mix_zero_is_dry() {
        let out = impulse(EffectKind::Reverb, 0.0);
        assert_eq!(out[0], 1.0);
        assert!(out[1..].iter().all(|v| v.abs() < 1e-6));
        let mut a = EffectSlot::new(EffectKind::Gargle);
        a.output = Some(1);
        let b = EffectSlot::new(EffectKind::Filter);
        let mut rack = Rack::new(&[a, b], 44_100);
        rack.input(0, 4).unwrap().0.fill(0.5);
        let (mut l, mut r) = (vec![0f32; 4], vec![0f32; 4]);
        rack.process(4, &mut l, &mut r);
        assert!(l.iter().any(|v| *v != 0.0));
        assert!(rack.fits(&[EffectSlot::new(EffectKind::Gargle), EffectSlot::new(EffectKind::Filter)]));
        assert!(!rack.fits(&[EffectSlot::new(EffectKind::Filter)]));
    }
}
