//! A two-operator FM patch played as a voice on its own: what an AdLib
//! instrument in a tracker module is, played at any frequency and
//! volume without the chip's registers in between. It runs at the
//! chip's rate, as the chip does, and comes out at the rate it's asked
//! for.

use super::op::{Clock, Op, OpParams};
use super::tables::fnum_block;
use super::{Resampler, CHIP_RATE};

/// An FM instrument: a modulator and a carrier, the modulator's
/// feedback, and how they're joined.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Patch {
    /// The modulator, then the carrier.
    pub ops: [OpParams; 2],
    /// The modulator's self-feedback, 0 to 7.
    pub feedback: u8,
    /// The two heard side by side (additive) rather than the modulator
    /// bending the carrier's phase (FM).
    pub additive: bool,
}

impl Patch {
    /// From the eleven bytes Scream Tracker 3 keeps (an S3M AdLib
    /// instrument's D00 to D0A, the AdLib's own instrument order):
    /// 0x20, 0x40, 0x60, 0x80 and 0xE0 for the modulator and carrier in
    /// turn, then 0xC0.
    pub fn from_s3m(b: &[u8; 11]) -> Patch {
        let mut p = Patch::default();
        for (o, op) in p.ops.iter_mut().enumerate() {
            op.set20(b[o]);
            op.set40(b[2 + o]);
            op.set60(b[4 + o]);
            op.set80(b[6 + o]);
            op.wave = b[8 + o] & 7;
        }
        p.set_c0(b[10]);
        p
    }

    pub fn to_s3m(&self) -> [u8; 11] {
        let [m, c] = &self.ops;
        [m.reg20(), c.reg20(), m.reg40(), c.reg40(), m.reg60(), c.reg60(), m.reg80(), c.reg80(), m.wave, c.wave, self.c0()]
    }

    /// From Composer 670's eleven: 0xC0 first, then the modulator's
    /// 0x20, 0x40, 0x60, 0x80 and 0xE0, then the carrier's.
    pub fn from_c67(b: &[u8; 11]) -> Patch {
        let mut p = Patch::default();
        p.set_c0(b[0]);
        for (o, op) in p.ops.iter_mut().enumerate() {
            let at = 1 + o * 5;
            op.set20(b[at]);
            op.set40(b[at + 1]);
            op.set60(b[at + 2]);
            op.set80(b[at + 3]);
            op.wave = b[at + 4] & 7;
        }
        p
    }

    pub fn c0(&self) -> u8 {
        (self.feedback & 7) << 1 | u8::from(self.additive)
    }

    pub fn set_c0(&mut self, v: u8) {
        self.feedback = (v >> 1) & 7;
        self.additive = v & 1 != 0;
    }

    /// Nothing would be heard: no carrier (or, additive, no operator) can
    /// rise above silence.
    pub fn is_silent(&self) -> bool {
        let quiet = |o: &OpParams| o.attack == 0 || o.level >= 63;
        quiet(&self.ops[1]) && (!self.additive || quiet(&self.ops[0]))
    }
}

/// A patch sounding.
#[derive(Clone, Copy, Debug)]
pub struct FmVoice {
    pub patch: Patch,
    ops: [Op; 2],
    clock: Clock,
    fnum: u16,
    block: u8,
    hz: f64,
    /// 0 to 64, scaling the total level of what's heard (Scream
    /// Tracker 3's way).
    volume: u8,
    levels: [u8; 2],
    resample: Resampler,
}

impl FmVoice {
    pub fn new(patch: Patch, rate: u32) -> FmVoice {
        let mut v = FmVoice {
            patch,
            ops: [Op::default(); 2],
            clock: Clock { deep_am: true, deep_vib: true, ..Default::default() },
            fnum: 0,
            block: 0,
            hz: 0.0,
            volume: 64,
            levels: [0; 2],
            resample: Resampler::new(CHIP_RATE, f64::from(rate.max(1))),
        };
        v.ops[0].p = patch.ops[0];
        v.ops[1].p = patch.ops[1];
        v.set_volume(64);
        v
    }

    /// A patch changed as it plays (the app's editor).
    pub fn set_patch(&mut self, patch: Patch) {
        self.patch = patch;
        for o in 0..2 {
            self.ops[o].p = patch.ops[o];
            self.ops[o].update_ksl();
        }
        self.set_volume(self.volume);
    }

    /// The depths of the LFOs: 4.8 dB tremolo and 14 cent vibrato, or
    /// 1 dB and 7 (register 0xBD's top bits; trackers set both deep).
    pub fn set_depths(&mut self, deep_am: bool, deep_vib: bool) {
        self.clock.deep_am = deep_am;
        self.clock.deep_vib = deep_vib;
    }

    pub fn set_hz(&mut self, hz: f64) {
        if hz == self.hz {
            return;
        }
        self.hz = hz;
        let (f, b) = fnum_block(hz);
        self.fnum = f;
        self.block = b;
        for op in &mut self.ops {
            op.set_freq(f, b, false);
        }
    }

    /// 0 to 64: the carrier's total level (both, additive) taken
    /// toward silence as it falls.
    pub fn set_volume(&mut self, volume: u8) {
        self.volume = volume.min(64);
        let scale = |l: u8| 63 - ((63 - u32::from(l)) * u32::from(self.volume) / 64) as u8;
        let p = &self.patch;
        self.levels = [if p.additive { scale(p.ops[0].level) } else { p.ops[0].level }, scale(p.ops[1].level)];
    }

    pub fn key_on(&mut self, hz: f64) {
        self.set_hz(hz);
        for op in &mut self.ops {
            op.key_on(1);
        }
    }

    pub fn key_off(&mut self) {
        for op in &mut self.ops {
            op.key_off(1);
        }
    }

    pub fn held(&self) -> bool {
        self.ops[1].keyed()
    }

    /// Let go and faded to silence: nothing more will be heard.
    pub fn done(&self) -> bool {
        self.ops[1].silent() && (!self.patch.additive || self.ops[0].silent())
    }

    /// A chip sample: -1 to 1 for one operator at full level.
    fn chip_sample(&mut self) -> f32 {
        self.clock.step();
        let counter = self.clock.counter;
        let am = self.clock.am;
        let fb = self.patch.feedback;
        let [m, c] = &mut self.ops;
        for op in [&mut *m, &mut *c] {
            op.clock_env(counter);
            let f = if op.p.vibrato { (i32::from(self.fnum) + self.clock.vibrato(self.fnum)).clamp(0, 1023) as u16 } else { self.fnum };
            op.clock_phase(f);
        }
        let (mw, cw) = (m.p.wave, c.p.wave);
        let mo = m.compute(m.feedback(fb), self.levels[0], am, mw);
        let out = if self.patch.additive { mo + c.compute(0, self.levels[1], am, cw) } else { c.compute(mo, self.levels[1], am, cw) };
        out as f32 * (1.0 / 4096.0)
    }

    /// The next output frame.
    #[inline]
    pub fn next(&mut self) -> f32 {
        let mut r = self.resample;
        let v = r.next(|| {
            let s = self.chip_sample();
            (s, s)
        });
        self.resample = r;
        v.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn organ() -> Patch {
        // A plain sine carrier held while the key's down, a quiet modulator.
        Patch::from_s3m(&[0x21, 0x21, 0x3F, 0x00, 0xF0, 0xF0, 0x00, 0x0F, 0, 0, 0x00])
    }

    #[test]
    fn a_sine_patch_plays_its_note() {
        let mut v = FmVoice::new(organ(), 48_000);
        v.key_on(440.0);
        let x: Vec<f32> = (0..48_000).map(|_| v.next()).collect();
        let crossings = x.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((436..=444).contains(&crossings), "{crossings}");
        let peak = x.iter().fold(0f32, |a, &b| a.max(b.abs()));
        assert!(peak > 0.9 && peak <= 1.0, "{peak}");
    }

    #[test]
    fn it_releases_to_silence_and_volume_takes_it_down() {
        let mut v = FmVoice::new(organ(), 48_000);
        v.key_on(220.0);
        for _ in 0..4800 {
            v.next();
        }
        v.set_volume(32);
        let half: f32 = (0..4800).map(|_| v.next().abs()).fold(0.0, f32::max);
        // Halfway down 63 steps of 0.75 dB is about 24 dB.
        assert!(half > 0.03 && half < 0.12, "{half}");
        v.key_off();
        for _ in 0..48_000 {
            v.next();
        }
        assert!(v.done());
    }

    #[test]
    fn the_s3m_bytes_come_back() {
        let b = [0x21, 0x31, 0x4F, 0x00, 0xF2, 0xF2, 0x60, 0x72, 1, 2, 0x0B];
        assert_eq!(Patch::from_s3m(&b).to_s3m(), b);
        let p = Patch::from_c67(&[0x0B, 0x21, 0x4F, 0xF2, 0x60, 1, 0x31, 0x00, 0xF2, 0x72, 2]);
        assert_eq!(p, Patch::from_s3m(&b));
    }
}
