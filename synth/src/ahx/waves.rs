// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! The waveforms AHX plays from, made once: triangles and sawtooths of
//! 4 to 128 points, 32 squares of 128 (pulse widths 1 to 32 of 64) and
//! 1,920 points of noise; and each of those through 31 low-passes and
//! 31 high-passes, a state-variable filter whose state is first run round
//! the wave from rest. AHX's own replayer does that in 16.16 fixed point,
//! three times round first; HivelyTracker's in floating point, once.

use std::sync::OnceLock;

/// Where each length's wave starts in a set's triangles and sawtooths.
pub const OFFSETS: [usize; 6] = [0x00, 0x04, 0x0C, 0x1C, 0x3C, 0x7C];
const CYCLES: usize = 0xFC;
pub const NOISE: usize = 0x280 * 3;

/// One filter's waveforms.
pub struct Set {
    pub triangles: [i8; CYCLES],
    pub sawtooths: [i8; CYCLES],
    pub squares: [i8; 0x80 * 0x20],
    pub noise: [i8; NOISE],
}

impl Set {
    fn empty() -> Box<Set> {
        Box::new(Set { triangles: [0; CYCLES], sawtooths: [0; CYCLES], squares: [0; 0x80 * 0x20], noise: [0; NOISE] })
    }
}

/// The 63 sets: 0 to 30 low-passed (the lowest first), 31 as made,
/// 32 to 62 high-passed.
pub struct Waves {
    pub sets: Vec<Box<Set>>,
}

/// AHX's waves (`hively` false) or HivelyTracker's.
pub fn waves(hively: bool) -> &'static Waves {
    static AHX: OnceLock<Waves> = OnceLock::new();
    static HVL: OnceLock<Waves> = OnceLock::new();
    if hively {
        HVL.get_or_init(|| Waves::make(true))
    } else {
        AHX.get_or_init(|| Waves::make(false))
    }
}

fn sawtooth(out: &mut [i8]) {
    let add = 256 / (out.len() as i32 - 1);
    let mut v = -128i32;
    for o in out.iter_mut() {
        *o = v as i8;
        v += add;
    }
}

/// A triangle: a quarter up from 0 to the top, a quarter down, then the
/// first half again upside down.
fn triangle(out: &mut [i8]) {
    let len = out.len();
    let quarter = len >> 2;
    let step = 128 / quarter as i32;
    let mut at = 0;
    let mut v = 0i32;
    for _ in 0..quarter {
        out[at] = v as i8;
        at += 1;
        v += step;
    }
    out[at] = 0x7F;
    at += 1;
    if quarter != 1 {
        v = 128;
        for _ in 0..quarter - 1 {
            v -= step;
            out[at] = v as i8;
            at += 1;
        }
    }
    let mut from = at - (len >> 1);
    for _ in 0..quarter * 2 {
        let c = out[from];
        from += 1;
        out[at] = if c == 0x7F { -128 } else { c.wrapping_neg() };
        at += 1;
    }
}

/// Square `i` (1 to 32): (64 - i) × 2 points low, then i × 2 high.
fn squares(out: &mut [i8]) {
    let mut at = 0;
    for i in 1..=0x20 {
        for _ in 0..(0x40 - i) * 2 {
            out[at] = -128;
            at += 1;
        }
        for _ in 0..i * 2 {
            out[at] = 127;
            at += 1;
        }
    }
}

/// AHX's noise: a 32-bit register rotated and mixed a point at a time,
/// a point its low byte, or a full swing when bit 8 is set (down when
/// bit 15 is; HivelyTracker's test of that is never true, so its swings
/// all go up).
fn noise(out: &mut [i8], hively: bool) {
    let mut s: u32 = 0x4159_5321;
    for o in out.iter_mut() {
        let mut b = s as u8;
        if s & 0x100 != 0 {
            b = if s & 0x8000 != 0 && !hively { 0x80 } else { 0x7F };
        }
        *o = b as i8;
        s = s.rotate_right(5);
        s = (s & 0xFFFF_FF00) | ((s & 0xFF) ^ 0x9A);
        let mut bx = s as u16;
        s = s.rotate_left(2);
        let mut ax = s as u16;
        bx = bx.wrapping_add(ax);
        ax ^= bx;
        s = (s & 0xFFFF_0000) | u32::from(ax);
        s = s.rotate_right(3);
    }
}

/// A value of the filter's (16.16) held to a byte's range.
fn clip(x: i32) -> i32 {
    let top = (x >> 16) as i16;
    if top > 127 {
        127 << 16
    } else if top < -128 {
        -(128 << 16)
    } else {
        x
    }
}

/// One wave through AHX's filter at `freq` (in 256ths): its state run
/// round the wave three times from rest, then the fourth time out to
/// the low- and high-passed copies.
fn filter(input: &[i8], freq: i32, low_out: &mut [i8], high_out: &mut [i8]) {
    let (mut mid, mut low) = (0i32, 0i32);
    for pass in 0..4 {
        for (i, &x) in input.iter().enumerate() {
            let high = clip((i32::from(x) << 16) - mid - low);
            mid = clip(mid + (high >> 8) * freq);
            low = clip(low + (mid >> 8) * freq);
            if pass == 3 {
                high_out[i] = (high >> 16) as i8;
                low_out[i] = (low >> 16) as i8;
            }
        }
    }
}

/// One wave through HivelyTracker's filter at `fre` (a fraction a point):
/// once round from rest, then out to the copies.
fn filter_float(input: &[i8], fre: f64, low_out: &mut [i8], high_out: &mut [i8]) {
    let clip = |x: f64| x.clamp(-128.0, 127.0);
    let (mut mid, mut low) = (0.0f64, 0.0f64);
    for pass in 0..2 {
        for (i, &x) in input.iter().enumerate() {
            let high = clip(f64::from(x) - mid - low);
            mid = clip(mid + high * fre);
            low = clip(low + mid * fre);
            if pass == 1 {
                high_out[i] = high as i8;
                low_out[i] = low as i8;
            }
        }
    }
}

impl Waves {
    fn make(hively: bool) -> Waves {
        let mut plain = Set::empty();
        for (k, &at) in OFFSETS.iter().enumerate() {
            let len = 4 << k;
            sawtooth(&mut plain.sawtooths[at..at + len]);
            triangle(&mut plain.triangles[at..at + len]);
        }
        squares(&mut plain.squares);
        noise(&mut plain.noise, hively);
        let mut sets: Vec<Box<Set>> = (0..63).map(|_| Set::empty()).collect();
        for i in 0..31 {
            let freq = 25 + 9 * i as i32;
            let fre = (8.0 + 3.0 * i as f64) * 1.25 / 100.0;
            let filter = |input: &[i8], low_out: &mut [i8], high_out: &mut [i8]| {
                if hively {
                    filter_float(input, fre, low_out, high_out)
                } else {
                    filter(input, freq, low_out, high_out)
                }
            };
            let (lo, hi) = sets.split_at_mut(32);
            let (l, h) = (&mut lo[i], &mut hi[i]);
            for (k, &at) in OFFSETS.iter().enumerate() {
                let r = at..at + (4 << k);
                filter(&plain.triangles[r.clone()], &mut l.triangles[r.clone()], &mut h.triangles[r]);
            }
            for (k, &at) in OFFSETS.iter().enumerate() {
                let r = at..at + (4 << k);
                filter(&plain.sawtooths[r.clone()], &mut l.sawtooths[r.clone()], &mut h.sawtooths[r]);
            }
            for j in 0..32 {
                let r = j * 0x80..(j + 1) * 0x80;
                filter(&plain.squares[r.clone()], &mut l.squares[r.clone()], &mut h.squares[r]);
            }
            filter(&plain.noise, &mut l.noise, &mut h.noise);
        }
        sets[31] = plain;
        Waves { sets }
    }
}

/// AHX's periods: ProTracker's three octaves, and two more below them
/// (each twice the one above); index 0 is unused, 1 is C-1 and 60 B-5.
pub fn period(note: i32) -> i32 {
    const OCTAVE: [i32; 12] = [856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453];
    if note <= 0 {
        return 0;
    }
    let n = (note - 1).min(59);
    let (oct, k) = (n / 12, (n % 12) as usize);
    match oct {
        0 => OCTAVE[k] * 4,
        1 => OCTAVE[k] * 2,
        2 => OCTAVE[k],
        // ProTracker's own octaves 2 and 3.
        3 => [428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226][k],
        _ => [214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113][k],
    }
}

/// The vibrato's sine, 64 points, 255 at its peak.
pub fn vibrato(i: usize) -> i32 {
    (255.0 * (std::f64::consts::TAU * (i & 63) as f64 / 64.0).sin()) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods_run_from_c1_to_b5() {
        assert_eq!(period(1), 0xD60);
        assert_eq!(period(13), 0x6B0);
        assert_eq!(period(24), 0x38A);
        assert_eq!(period(60), 0x71);
        assert_eq!(period(45), 0x10D);
    }

    #[test]
    fn the_vibrato_is_a_sine() {
        let v: Vec<i32> = (0..18).map(vibrato).collect();
        assert_eq!(v, [0, 24, 49, 74, 97, 120, 141, 161, 180, 197, 212, 224, 235, 244, 250, 253, 255, 253]);
        assert_eq!(vibrato(48), -255);
        assert_eq!(vibrato(63), -24);
    }

    #[test]
    fn waves_have_their_shapes() {
        let w = waves(false);
        let p = &w.sets[31];
        assert_eq!(&p.sawtooths[0..4], &[-128, -43, 42, 127]);
        assert_eq!(&p.triangles[0..4], &[0, 127, 0, -128]);
        assert_eq!(&p.triangles[4..12], &[0, 64, 127, 64, 0, -64, -128, -64]);
        assert!(p.squares[..0x7E].iter().all(|&x| x == -128) && p.squares[0x7E..0x80].iter().all(|&x| x == 127));
        // A low-pass smooths, a high-pass keeps the edges.
        let rough = |s: &[i8]| s.windows(2).map(|w| (i32::from(w[1]) - i32::from(w[0])).abs()).sum::<i32>();
        assert!(rough(&w.sets[0].noise) < rough(&p.noise) / 2);
        assert!(rough(&w.sets[62].squares[..128]) > 0);
        let h = waves(true);
        assert_eq!(&h.sets[31].sawtooths[..], &p.sawtooths[..]);
        assert!(p.noise.contains(&-128) && !h.sets[31].noise.contains(&-128));
        assert!(rough(&h.sets[0].noise) < rough(&p.noise) / 2);
    }
}
