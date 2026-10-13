//! ProTrekkr's effects: each track's filter (24 kinds, from biquads to
//! ring modulation, slew limiting and power-law distortion), flanger,
//! three-band EQ and compressor; the mix's chorus (a feedback delay a
//! side, filtered), reverb (one-pole combs into allpasses and a
//! resonant low-pass) and compressor. All at 44.1 kHz, sample by sample.

use std::f32::consts::PI;

use super::{log2, pow, pow2};

const RATE: f32 = 44_100.0;

fn cutoff_hz(v: i32) -> f32 {
    pow((v as f32 + 5.0) / (127.0 + 5.0), 1.7) * 13_000.0 + 30.0
}

fn resonance_q(v: f32) -> f32 {
    pow(v / 127.0, 4.0) * 150.0 + 0.1
}

fn bandwidth(v: i32) -> f32 {
    pow(v as f32 / 127.0, 4.0) * 4.0 + 0.1
}

/// A biquad's five coefficients for a cutoff and resonance (0 to 127)
/// and a kind: 0 low-pass, 1 high-pass, 2 band-pass, 3 band-reject.
pub(crate) fn coefs(freq: i32, r: i32, t: i32) -> [f32; 5] {
    // Worked out in doubles, as the replayer's `PI` is one.
    let omega = (2.0 * std::f64::consts::PI * f64::from(cutoff_hz(freq)) / f64::from(RATE)) as f32;
    let (sn, cs) = (omega.sin(), omega.cos());
    let alpha = if t < 2 { sn / resonance_q(r as f32 * (freq as f32 + 70.0) / (127.0 + 70.0)) } else { sn * (bandwidth(r) * omega / sn).sinh() };
    let (b0, b1, b2) = match t {
        1 => ((1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0),
        2 => (alpha, 0.0, -alpha),
        3 => (1.0, -2.0 * cs, 1.0),
        _ => ((1.0 - cs) / 2.0, 1.0 - cs, (1.0 - cs) / 2.0),
    };
    let (a0, a1, a2) = (1.0 + alpha, -2.0 * cs, 1.0 - alpha);
    [b0 / a0, b1 / a0, b2 / a0, -a1 / a0, -a2 / a0]
}

/// The biquads' coefficients, made as they're asked for (the replayer
/// tabulates every cutoff, resonance and kind).
pub(crate) struct CoefCache {
    table: Vec<Option<[f32; 5]>>,
}

impl CoefCache {
    pub fn new() -> CoefCache {
        CoefCache { table: vec![None; 128 * 128 * 4] }
    }

    pub fn get(&mut self, freq: i32, r: i32, t: i32) -> [f32; 5] {
        let (f, r, t) = (freq.clamp(0, 127), r.clamp(0, 127), t.clamp(0, 3));
        let i = ((f * 128 + r) * 4 + t) as usize;
        *self.table[i].get_or_insert_with(|| coefs(f, r, t))
    }
}

/// A biquad's memory: (x1, x2, y1, y2).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Biquad {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    pub fn run(&mut self, c: &[f32; 5], x: f32) -> f32 {
        let y = c[0] * x + c[1] * self.x1 + c[2] * self.x2 + c[3] * self.y1 + c[4] * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        self.x2 = self.x1;
        self.x1 = x;
        y
    }
}

/// A track's filter memory, a side each.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TrackFilter {
    buf024: [f32; 2],
    buf124: [f32; 2],
    buf0: [f32; 2],
    buf1: [f32; 2],
    bq: [Biquad; 2],
    xi: [[f32; 3]; 2],
}

impl TrackFilter {
    /// The two-pole resonant low-pass on `buf0`/`buf1` (or the second
    /// pair, for the 24 dB kinds); `hp` takes it from its input.
    fn two_pole(a: &mut f32, b: &mut f32, input: f32, f: f32, q: f32, hp: bool) -> f32 {
        let f = f * 0.007_812_5;
        let q = q * 0.007_812_5;
        let fa = 1.0 - f;
        let fb = q * (1.0 + 1.0 / fa);
        *a = fa * *a + f * (input + fb * (*a - *b));
        *b = fa * *b + f * *a;
        if hp {
            input - *b
        } else {
            *b
        }
    }

    /// The bell-shaped EQ at `f` (a 127th of the way up from 320 Hz),
    /// width `q`, gain `g` dB-ish.
    fn bell(&mut self, s: usize, input: f32, f: f32, q: f32, g: f32) -> f32 {
        let input = input + 1.0;
        let q = (q * 0.007_874).max(0.01);
        let freq = (320.0 + f * 127.656_25).min(22_100.0);
        let gain = g / 6.6;
        let wn = 1.0 / (6.283_185_3 * freq);
        let wp = wn / (wn / 88_200.0).tan();
        let a = wn * wn * wp * wp;
        let t1 = wn * wp * q;
        let b = (3.0 + gain) * t1;
        let c = (3.0 - gain) * t1;
        let t1 = a + c + 1.0;
        let b2 = (1.0 - c + a) / t1;
        let a2 = (1.0 - b + a) / t1;
        let b1 = 2.0 * (1.0 - a) / t1;
        let a1 = b1;
        let a0 = (a + b + 1.0) / t1;
        let x = &mut self.xi[s];
        x[0] = input - b1 * x[1] - b2 * x[2];
        let out = a0 * x[0] + a1 * x[1] + a2 * x[2];
        x[2] = x[1];
        x[1] = x[0];
        out
    }

    /// Distortion through a two-pole filter: the filtered signal's
    /// magnitude raised to a power the resonance sets (always positive,
    /// as the replayer has it).
    fn dist(&mut self, s: usize, input: f32, f: f32, q: f32, q2: f32) -> f32 {
        let q = q * 0.078_740_1;
        let f = f * 0.007_812_5;
        let fa = 1.0 - f;
        let fb = q2 * (1.0 + 1.0 / fa);
        let (a, b) = (&mut self.buf0[s], &mut self.buf1[s]);
        *a = fa * *a + f * (input + fb * (*a - *b));
        *b = fa * *b + f * *a;
        32_767.0 * pow(b.abs() / 32_767.0, 1.0 - q / 11.0)
    }

    /// The track's mix through its filter: `kind` 0 to 23 (4 is none),
    /// `cut` 1 to 126 and `res` 0 to 127. `stereo` says the right side
    /// has its own signal; it can make it so (a stereo ring modulator).
    #[allow(clippy::too_many_arguments)]
    pub fn run(&mut self, coefs: &mut CoefCache, kind: i32, cut: f32, gco: i32, res: i32, l: &mut f32, r: &mut f32, stereo: &mut bool, sin: &[f32; 360]) {
        let rf = res as f32;
        let st = *stereo;
        match kind {
            0..=3 => {
                let c = coefs.get(gco, res, kind);
                *l = self.bq[0].run(&c, *l + 1.0);
                if st {
                    *r = self.bq[1].run(&c, *r + 1.0);
                }
            }
            5 => {
                *l = Self::two_pole(&mut self.buf0[0], &mut self.buf1[0], *l + 1.0, cut, rf, false);
                if st {
                    *r = Self::two_pole(&mut self.buf0[1], &mut self.buf1[1], *r + 1.0, cut, rf, false);
                }
            }
            6 => {
                *l = Self::two_pole(&mut self.buf0[0], &mut self.buf1[0], *l + 1.0, cut, rf, false);
                *l = Self::two_pole(&mut self.buf024[0], &mut self.buf124[0], *l + 1.0, cut, rf, false);
                if st {
                    *r = Self::two_pole(&mut self.buf0[1], &mut self.buf1[1], *r + 1.0, cut, rf, false);
                    *r = Self::two_pole(&mut self.buf024[1], &mut self.buf124[1], *r + 1.0, cut, rf, false);
                }
            }
            7 => {
                // Low-pass on the left, the second stage alone on the right.
                *l = Self::two_pole(&mut self.buf0[0], &mut self.buf1[0], *l + 1.0, cut, rf, false);
                if st {
                    *r = Self::two_pole(&mut self.buf024[1], &mut self.buf124[1], *r + 1.0, cut, rf, false);
                }
            }
            8 => {
                *l = self.ring(0, *l, cut, rf, sin);
                if st {
                    *r = self.ring(1, *r, cut, rf, sin);
                }
            }
            9 => {
                if !st {
                    *r = *l;
                    *stereo = true;
                }
                *l = self.ring(0, *l, cut, rf, sin);
                // Its cosine on the right, from the left's phase.
                *r *= (self.buf0[0] * 0.017_453_2).cos();
            }
            10 => {
                *l = Self::slew(&mut self.buf0[0], *l, cut);
                if st {
                    *r = Self::slew(&mut self.buf0[1], *r, cut);
                }
            }
            11 => {
                *l = Self::slew(&mut self.buf0[0], *l, cut);
                if st {
                    *r = Self::slew(&mut self.buf1[1], *r, cut);
                }
            }
            12..=15 => {
                let g = [-15.0, -6.0, 6.0, 15.0][(kind - 12) as usize];
                *l = self.bell(0, *l, cut, rf, g);
                if st {
                    *r = self.bell(1, *r, cut, rf, g);
                }
            }
            16 => {
                *l = self.delta(0, *l, cut, rf);
                if st {
                    *r = self.delta(1, *r, cut, rf);
                }
            }
            17..=20 => {
                let q2 = [0.25, 0.56, 0.78, 0.96][(kind - 17) as usize];
                *l = self.dist(0, *l, cut, rf, q2);
                if st {
                    *r = self.dist(1, *r, cut, rf, q2);
                }
            }
            21 => {
                *l = Self::two_pole(&mut self.buf0[0], &mut self.buf1[0], *l + 1.0, cut, rf, true);
                if st {
                    *r = Self::two_pole(&mut self.buf0[1], &mut self.buf1[1], *r + 1.0, cut, rf, true);
                }
            }
            22 => {
                *l = Self::two_pole(&mut self.buf0[0], &mut self.buf1[0], *l + 1.0, cut, rf, true);
                if st {
                    *r = Self::two_pole(&mut self.buf024[1], &mut self.buf124[1], *r + 1.0, cut, rf, true);
                }
            }
            23 => {
                *l = Self::two_pole(&mut self.buf0[0], &mut self.buf1[0], *l + 1.0, cut, rf, true);
                *l = Self::two_pole(&mut self.buf024[0], &mut self.buf124[0], *l + 1.0, cut, rf, true);
                if st {
                    *r = Self::two_pole(&mut self.buf0[1], &mut self.buf1[1], *r + 1.0, cut, rf, true);
                    *r = Self::two_pole(&mut self.buf024[1], &mut self.buf124[1], *r + 1.0, cut, rf, true);
                }
            }
            _ => {}
        }
    }

    /// Ring modulation by a sine the cutoff and resonance speed up.
    fn ring(&mut self, s: usize, input: f32, f: f32, q: f32, sin: &[f32; 360]) -> f32 {
        let q = q + 1.0;
        let f = f * 0.007_812_5;
        let p = &mut self.buf0[s];
        *p += f * (q * 0.125);
        if *p >= 360.0 {
            *p -= 360.0;
        }
        input * sin[(*p as usize).min(359)]
    }

    /// Slew limiting: the output moves toward the input by a share.
    fn slew(b: &mut f32, input: f32, f: f32) -> f32 {
        let f = 127.0 - f;
        let ad = input - *b;
        if !(-1.0..=1.0).contains(&ad) {
            *b += ad / f;
        }
        *b
    }

    /// Sample and hold, its held value decaying by the resonance.
    fn delta(&mut self, s: usize, input: f32, f: f32, q: f32) -> f32 {
        let f = 127.0 - f;
        let q = q * 0.007_874;
        let out = self.buf1[s];
        if !(-1.0..=1.0).contains(&self.buf1[s]) {
            self.buf1[s] *= q;
        }
        self.buf0[s] += 1.0;
        if self.buf0[s] >= f {
            self.buf0[s] = 0.0;
            self.buf1[s] = input;
            return input;
        }
        out
    }
}

/// The flanger's own resonant low-pass on what comes back.
#[derive(Clone, Copy, Debug, Default)]
struct FlangerLowpass {
    b0: f32,
    b1: f32,
}

impl FlangerLowpass {
    fn run(&mut self, input: f32) -> f32 {
        const CUT: f32 = 0.1;
        const RES: f32 = 0.4;
        let fa = 1.0 - CUT;
        let fb = RES * (1.0 + 1.0 / fa);
        self.b0 = fa * self.b0 + CUT * (input + fb * (self.b0 - self.b1));
        self.b1 = fa * self.b1 + CUT * self.b0;
        self.b1
    }
}

const FLANGE: usize = 16_400;

/// A track's flanger: a delay line read at two places a sine swings.
pub(crate) struct Flanger {
    left: Vec<f32>,
    right: Vec<f32>,
    pub offset: i32,
    pub offset1: f32,
    pub offset2: f32,
    pub phase: f32,
    old: [f32; 2],
    lp: [FlangerLowpass; 2],
}

impl Flanger {
    pub fn new() -> Flanger {
        Flanger { left: vec![0.0; FLANGE], right: vec![0.0; FLANGE], offset: 8192, offset1: 0.0, offset2: 0.0, phase: 0.0, old: [0.0; 2], lp: [FlangerLowpass::default(); 2] }
    }

    pub fn clear(&mut self, delay: i32) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        self.offset2 = (self.offset - delay) as f32;
        self.offset1 = (self.offset - delay) as f32;
        self.old = [0.0; 2];
        self.lp = [FlangerLowpass::default(); 2];
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run(&mut self, l: &mut f32, r: &mut f32, amount: f32, feedback: f32, dephase: f32, ampl: f32, rate: f32, delay: i32, sin: &[f32; 360]) {
        const TAU: f32 = 6.283_185;
        let at = self.offset.rem_euclid(FLANGE as i32) as usize;
        self.left[at] = *l * amount + self.old[0] * feedback;
        self.right[at] = *r * amount + self.old[1] * feedback;
        let gr = self.phase / TAU;
        let mut de = self.phase + dephase;
        if de >= TAU {
            de -= TAU;
        }
        let de = de / TAU;
        let idx = |v: f32| ((v * 359.0) as i32).clamp(0, 359) as usize;
        let step1 = pow2(sin[idx(gr)] * ampl);
        let step2 = pow2(sin[idx(de)] * ampl);
        self.offset2 += step1;
        self.offset1 += step2;
        for o in [&mut self.offset2, &mut self.offset1] {
            if *o >= 16_384.0 {
                *o -= 16_384.0;
            }
            if *o < 0.0 {
                *o += 16_384.0;
            }
        }
        self.old[0] = self.left[(self.offset2 as usize).min(FLANGE - 1)];
        self.old[1] = self.right[(self.offset1 as usize).min(FLANGE - 1)];
        self.old[0] += 1.0;
        self.old[1] += 1.0;
        *l += self.lp[0].run(self.old[0]);
        *r += self.lp[1].run(self.old[1]);
        self.offset += 1;
        if self.offset >= 16_384 {
            self.offset -= 16_384;
        }
        self.phase += rate;
        if self.phase >= TAU {
            self.phase -= TAU;
            self.offset2 = (self.offset - delay) as f32;
            self.offset1 = (self.offset - delay) as f32;
        }
    }
}

/// A track's three-band EQ (low under 880 Hz, high over 5 kHz).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Eq {
    f1: [[f32; 4]; 2],
    f2: [[f32; 4]; 2],
    sdm: [[f32; 3]; 2],
}

impl Eq {
    pub fn run(&mut self, gains: [f32; 3], x: f32, s: usize) -> f32 {
        const VSA: f32 = 1.0 / 4_294_967_295.0;
        let lf = 2.0 * (PI * (880.0 / RATE)).sin();
        let hf = 2.0 * (PI * (5000.0 / RATE)).sin();
        let f1 = &mut self.f1[s];
        f1[0] += lf * (x - f1[0]) + VSA;
        f1[1] += lf * (f1[0] - f1[1]);
        f1[2] += lf * (f1[1] - f1[2]);
        f1[3] += lf * (f1[2] - f1[3]);
        let low = f1[3];
        let f2 = &mut self.f2[s];
        f2[0] += hf * (x - f2[0]) + VSA;
        f2[1] += hf * (f2[0] - f2[1]);
        f2[2] += hf * (f2[1] - f2[2]);
        f2[3] += hf * (f2[2] - f2[3]);
        let sdm = &mut self.sdm[s];
        let high = sdm[2] - f2[3];
        let mid = sdm[2] - (high + low);
        sdm[2] = sdm[1];
        sdm[1] = sdm[0];
        sdm[0] = x;
        low * gains[0] + mid * gains[1] + high * gains[2]
    }
}

/// The compressors' RMS window, in frames (a tenth of a second).
pub(crate) const COMP_WINDOW: usize = 4410;

/// A compressor side: its RMS window and envelope.
pub(crate) struct Compressor {
    buf: Vec<f32>,
    sum: f32,
    env: f32,
}

impl Compressor {
    pub fn new() -> Compressor {
        Compressor { buf: vec![0.0; COMP_WINDOW], sum: 0.0, env: 0.0 }
    }

    pub fn clear(&mut self) {
        self.buf.fill(0.0);
        self.sum = 0.0;
        self.env = 0.0;
    }

    /// The gain for `x` at the window's place `pos`, for a threshold and
    /// ratio already scaled (threshold × 0.001, ratio × 0.01).
    pub fn gain(&mut self, x: f32, pos: usize, threshold: f32, ratio: f32) -> f32 {
        const THETA: f32 = 0.977_579;
        self.sum -= self.buf[pos];
        self.buf[pos] = x * x;
        self.sum += self.buf[pos];
        if self.sum < 0.0 {
            self.sum = 0.0;
        }
        let rms = (self.sum / COMP_WINDOW as f32).sqrt();
        self.env = (1.0 - THETA) * rms + THETA * self.env;
        if self.env > threshold {
            ((log2(threshold) - log2(self.env)) * ratio).exp()
        } else {
            1.0
        }
    }
}

const CHORUS: usize = 131_072;

/// The chorus: a feedback delay a side (left into right and right into
/// left, as the replayer crosses them), filtered.
pub(crate) struct Chorus {
    left: Vec<f32>,
    right: Vec<f32>,
    pub counters: [usize; 4],
    bq: [Biquad; 2],
}

impl Chorus {
    pub fn new() -> Chorus {
        Chorus { left: vec![0.0; CHORUS], right: vec![0.0; CHORUS], counters: [0; 4], bq: [Biquad::default(); 2] }
    }

    pub fn clear(&mut self, delays: [i32; 2]) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        let r = 44_100usize;
        self.counters = [r, r, (r as i32 - delays[0]).clamp(0, CHORUS as i32 - 1) as usize, (r as i32 - delays[1]).clamp(0, CHORUS as i32 - 1) as usize];
        self.bq = [Biquad::default(); 2];
    }

    /// Takes the sends in, gives what comes back: (left, right).
    pub fn run(&mut self, send: (f32, f32), feedback: [f32; 2], filter: Option<[f32; 5]>) -> (f32, f32) {
        const TOP: usize = 88_200;
        let [lc, rc, lc2, rc2] = &mut self.counters;
        *lc += 1;
        if *lc > TOP {
            *lc = 44_100;
        }
        *rc += 1;
        if *rc > TOP {
            *rc = 44_100;
        }
        self.left[*lc] = send.0 + self.left[*lc2] * feedback[0];
        self.right[*rc] = send.1 + self.right[*rc2] * feedback[1];
        *lc2 += 1;
        if *lc2 > TOP {
            *lc2 = 44_100;
        }
        *rc2 += 1;
        if *rc2 > TOP {
            *rc2 = 44_100;
        }
        let right = self.left[*lc2];
        let left = self.right[*rc2];
        match filter {
            Some(c) => (self.bq[0].run(&c, left + 1.0), self.bq[1].run(&c, right + 1.0)),
            None => (left, right),
        }
    }
}

const ALLPASS: usize = 5760;
const DELAYS: [f32; 10] = [44.1, 50.1, 60.1, 70.1, 73.1, 79.1, 64.0, 55.0, 20.0, 32.0];

/// The reverb: up to ten one-pole combs (their outputs summed, every
/// other one's input negated) into as many allpasses, then a resonant
/// low-pass. Each comb's delay line is read where it was written a frame
/// before, so it's a single value. The allpasses' lines are laid out as
/// the replayer's are (every left one, then every right one, end to
/// end): a wide stereo spread starts a right one's write before its own
/// line, into the one before it, as there.
pub(crate) struct Reverb {
    comb: [[f32; 2]; 10],
    all: Vec<f32>,
    current: i32,
    delayed: [[i32; 2]; 10],
    lp: [FlangerState; 2],
}

/// A resonant low-pass's memory (the reverb's last stage).
#[derive(Clone, Copy, Debug, Default)]
struct FlangerState {
    b0: f32,
    b1: f32,
}

impl Reverb {
    pub fn new() -> Reverb {
        Reverb { comb: [[0.0; 2]; 10], all: vec![0.0; 2 * 10 * ALLPASS], current: ALLPASS as i32 - 1, delayed: [[0; 2]; 10], lp: [FlangerState::default(); 2] }
    }

    /// Cleared, its allpasses' delays from the threshold and the stereo
    /// spread.
    pub fn clear(&mut self, threshold: i32, stereo: u8) {
        self.comb = [[0.0; 2]; 10];
        self.all.fill(0.0);
        self.current = ALLPASS as i32 - 1;
        let wrap = |mut v: i32| -> i32 {
            if v < 0 {
                v += ALLPASS as i32;
            }
            if v > ALLPASS as i32 - 1 {
                v -= ALLPASS as i32 - 1;
            }
            v
        };
        for (i, d) in self.delayed.iter_mut().enumerate() {
            d[0] = wrap(ALLPASS as i32 - 1 - (threshold as f32 * DELAYS[i]) as i32);
            d[1] = wrap(ALLPASS as i32 - 1 - (threshold as f32 * DELAYS[i] + f32::from(stereo) * 12.0) as i32);
        }
        self.lp = [FlangerState::default(); 2];
    }

    /// Where line `i` of side `s`'s point `at` lies in the lines laid end to end.
    fn at(s: usize, i: usize, at: i32) -> Option<usize> {
        usize::try_from(((s * 10 + i) * ALLPASS) as i64 + i64::from(at)).ok().filter(|&x| x < 2 * 10 * ALLPASS)
    }

    /// The reverb's output for its send.
    #[allow(clippy::too_many_arguments)]
    pub fn run(&mut self, send: (f32, f32), decays: &[f32], damp: f32, feedback: f32, cutoff: f32, resonance: f32) -> (f32, f32) {
        let n = decays.len().min(10);
        let mut out = [0.0f32; 2];
        for (i, &decay) in decays.iter().enumerate().take(n) {
            for (s, x) in [send.0, send.1].into_iter().enumerate() {
                let d = self.comb[i][s] * decay;
                let v = if i & 1 == 1 { d - x } else { d + x } * damp;
                self.comb[i][s] = v;
                out[s] += v;
            }
        }
        let cur = self.current;
        for i in 0..n {
            for (s, o) in out.iter_mut().enumerate() {
                let read = Reverb::at(s, i, cur).map_or(0.0, |x| self.all[x]);
                let y = -feedback * *o + read;
                if let Some(x) = Reverb::at(s, i, self.delayed[i][s]) {
                    self.all[x] = y * feedback + *o;
                }
                *o = y;
                let d = &mut self.delayed[i][s];
                *d += 1;
                if *d > ALLPASS as i32 - 1 {
                    *d -= ALLPASS as i32 - 1;
                }
            }
        }
        for (s, o) in out.iter_mut().enumerate() {
            let fa = 1.0 - cutoff;
            let fb = resonance * (1.0 + 1.0 / fa);
            let lp = &mut self.lp[s];
            lp.b0 = fa * lp.b0 + cutoff * (*o + fb * (lp.b0 - lp.b1));
            lp.b1 = fa * lp.b1 + cutoff * lp.b0;
            *o = lp.b1;
        }
        self.current += 1;
        if self.current > ALLPASS as i32 - 1 {
            self.current -= ALLPASS as i32 - 1;
        }
        (out[0], out[1])
    }
}
