//! One operator: a phase generator, an envelope generator and a
//! waveform read through the log and exponential tables, run a chip
//! sample (1/49,716 s) at a time.

use super::tables::{level, tables, Tables, ENV_STEPS, KSL_EIGHTHS, MULT2};

/// Envelope units are 0.1875 dB: 0 loudest, 511 silent (96 dB down).
pub const ENV_MAX: u16 = 511;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Stage {
    Attack,
    Decay,
    Sustain,
    #[default]
    Release,
}

/// An operator's settings: its registers 0x20, 0x40, 0x60, 0x80 and 0xE0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct OpParams {
    /// Tremolo (AM) and vibrato from the chip's LFOs.
    pub tremolo: bool,
    pub vibrato: bool,
    /// Holds at the sustain level while the key's down (else decays on
    /// at the release rate: a percussive sound).
    pub sustain: bool,
    /// Key scale rate: higher notes' envelopes run faster.
    pub ksr: bool,
    /// The frequency multiplier, 0 to 15 (0 is a half).
    pub mult: u8,
    /// Key scale level, 0 to 3: none, 3, 1.5, 6 dB an octave.
    pub ksl: u8,
    /// Total level, 0 to 63: its attenuation, 0.75 dB a step.
    pub level: u8,
    /// Attack, decay and release rates, 0 to 15 (0 never moves).
    pub attack: u8,
    pub decay: u8,
    /// Sustain level, 0 to 15: 3 dB a step, 15 all the way down.
    pub sustain_level: u8,
    pub release: u8,
    /// Waveform, 0 to 7 (4 to 7 are the OPL3's).
    pub wave: u8,
}

impl OpParams {
    pub fn reg20(&self) -> u8 {
        u8::from(self.tremolo) << 7 | u8::from(self.vibrato) << 6 | u8::from(self.sustain) << 5 | u8::from(self.ksr) << 4 | (self.mult & 15)
    }
    pub fn reg40(&self) -> u8 {
        (self.ksl & 3) << 6 | (self.level & 63)
    }
    pub fn reg60(&self) -> u8 {
        (self.attack & 15) << 4 | (self.decay & 15)
    }
    pub fn reg80(&self) -> u8 {
        (self.sustain_level & 15) << 4 | (self.release & 15)
    }
    pub fn set20(&mut self, v: u8) {
        self.tremolo = v & 0x80 != 0;
        self.vibrato = v & 0x40 != 0;
        self.sustain = v & 0x20 != 0;
        self.ksr = v & 0x10 != 0;
        self.mult = v & 15;
    }
    pub fn set40(&mut self, v: u8) {
        self.ksl = v >> 6;
        self.level = v & 63;
    }
    pub fn set60(&mut self, v: u8) {
        self.attack = v >> 4;
        self.decay = v & 15;
    }
    pub fn set80(&mut self, v: u8) {
        self.sustain_level = v >> 4;
        self.release = v & 15;
    }
}

/// The LFOs and the envelope clock, shared by every operator of a chip
/// (or a voice), moved on a chip sample at a time.
#[derive(Clone, Copy, Debug, Default)]
pub struct Clock {
    pub counter: u32,
    /// Tremolo's depth now, in envelope units.
    pub am: u16,
    /// Vibrato's place, 0 to 7.
    pub vib: u8,
    /// 4.8 dB tremolo (else 1 dB), 14 cent vibrato (else 7).
    pub deep_am: bool,
    pub deep_vib: bool,
}

impl Clock {
    pub fn step(&mut self) {
        self.counter = self.counter.wrapping_add(1);
        // Tremolo: a triangle of 210 steps, one every 64 samples (3.7 Hz).
        let pos = (self.counter / 64) % 210;
        let tri = if pos < 105 { pos } else { 209 - pos } as u16;
        self.am = if self.deep_am { tri >> 2 } else { tri >> 4 };
        // Vibrato: eight places, one every 1,024 samples (6.1 Hz).
        self.vib = ((self.counter >> 10) & 7) as u8;
    }

    /// Vibrato's change to an F-number: up to its top three bits' worth
    /// (an eighth of it, 14 cents, deep; half that, 7 cents, else).
    pub fn vibrato(&self, fnum: u16) -> i32 {
        let range = i32::from((fnum >> 7) & 7);
        let v = match self.vib {
            0 | 4 => 0,
            2 => range,
            6 => -range,
            1 | 3 => range >> 1,
            _ => -(range >> 1),
        };
        if self.deep_vib {
            v
        } else {
            v >> 1
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Op {
    pub p: OpParams,
    phase: u32,
    pub env: u16,
    pub stage: Stage,
    /// Which keys hold it on (a bit each: the channel's, the rhythm's).
    keys: u8,
    /// Its last two outputs, for feedback.
    pub out: [i32; 2],
    fnum: u16,
    block: u8,
    ksl_att: u16,
    ksr_val: u8,
}

impl Default for Op {
    fn default() -> Op {
        Op { p: OpParams::default(), phase: 0, env: ENV_MAX, stage: Stage::Release, keys: 0, out: [0; 2], fnum: 0, block: 0, ksl_att: 0, ksr_val: 0 }
    }
}

impl Op {
    /// The note it plays: an F-number, a block, and (for key scaling)
    /// the note select bit (register 0x08's NTS).
    pub fn set_freq(&mut self, fnum: u16, block: u8, nts: bool) {
        self.fnum = fnum & 1023;
        self.block = block & 7;
        let bit = if nts { (fnum >> 8) & 1 } else { (fnum >> 9) & 1 };
        self.ksr_val = self.block << 1 | bit as u8;
        self.update_ksl();
    }

    pub fn update_ksl(&mut self) {
        let base = i32::from(KSL_EIGHTHS[usize::from(self.fnum >> 6)]) - 48 * (7 - i32::from(self.block));
        let base = base.max(0) as u32;
        // Eighths of a dB at 6 dB an octave, to envelope units (0.1875 dB),
        // at the rate chosen.
        let units = base * 16 / 3 / 8;
        self.ksl_att = match self.p.ksl {
            1 => units / 2,
            2 => units / 4,
            3 => units,
            _ => 0,
        } as u16;
    }

    pub fn key_on(&mut self, which: u8) {
        if self.keys == 0 {
            self.phase = 0;
            self.stage = Stage::Attack;
            if self.rate(self.p.attack) >= 60 {
                self.env = 0;
                self.stage = Stage::Decay;
            }
        }
        self.keys |= which;
    }

    pub fn key_off(&mut self, which: u8) {
        if self.keys != 0 {
            self.keys &= !which;
            if self.keys == 0 {
                self.stage = Stage::Release;
            }
        }
    }

    pub fn keyed(&self) -> bool {
        self.keys != 0
    }

    /// Done: let go and silent.
    pub fn silent(&self) -> bool {
        self.keys == 0 && self.env >= ENV_MAX - 7
    }

    fn rate(&self, r: u8) -> u32 {
        if r == 0 {
            0
        } else {
            let ksr = if self.p.ksr { self.ksr_val } else { self.ksr_val >> 2 };
            (u32::from(r) * 4 + u32::from(ksr)).min(63)
        }
    }

    /// The envelope, a chip sample on.
    pub fn clock_env(&mut self, counter: u32) {
        let r = match self.stage {
            Stage::Attack => self.p.attack,
            Stage::Decay => self.p.decay,
            Stage::Sustain => {
                if self.p.sustain {
                    0
                } else {
                    self.p.release
                }
            }
            Stage::Release => self.p.release,
        };
        let rate = self.rate(r);
        let mut inc = 0u32;
        if rate > 0 {
            let hi = rate >> 2;
            if hi < 12 {
                let shift = 12 - hi;
                if counter & ((1 << shift) - 1) == 0 {
                    inc = u32::from(ENV_STEPS[(rate & 3) as usize][((counter >> shift) & 7) as usize]);
                }
            } else {
                inc = u32::from(ENV_STEPS[(rate & 3) as usize][(counter & 7) as usize]) << (hi - 12);
            }
        }
        match self.stage {
            Stage::Attack => {
                if rate >= 60 {
                    self.env = 0;
                } else if inc > 0 {
                    let e = u32::from(self.env);
                    self.env = e.saturating_sub(((e + 1) * inc + 7) >> 3) as u16;
                }
                if self.env == 0 {
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                let sl = if self.p.sustain_level == 15 { 31 } else { u16::from(self.p.sustain_level) } << 4;
                self.env = (self.env + inc as u16).min(ENV_MAX);
                if self.env >= sl {
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain | Stage::Release => self.env = (self.env + inc as u16).min(ENV_MAX),
        }
    }

    /// The phase, a chip sample on, at the F-number (vibrato's change
    /// added by the caller).
    pub fn clock_phase(&mut self, fnum: u16) {
        let inc = (u32::from(fnum) << self.block) * MULT2[usize::from(self.p.mult & 15)] >> 1;
        self.phase = self.phase.wrapping_add(inc) & 0xF_FFFF;
    }

    pub fn fnum(&self) -> u16 {
        self.fnum
    }

    /// Its phase now, 10 bits.
    pub fn phase10(&self) -> u32 {
        self.phase >> 10
    }

    /// Its attenuation now, in envelope units: the envelope, the total
    /// level (or `level` in its place), key scaling and tremolo.
    pub fn attenuation(&self, level: u8, am: u16) -> u32 {
        let am = if self.p.tremolo { am } else { 0 };
        (u32::from(self.env) + (u32::from(level) << 2) + u32::from(self.ksl_att) + u32::from(am)).min(u32::from(ENV_MAX))
    }

    /// The output at a phase (10 bits, modulation added) and attenuation
    /// (envelope units): -4095 to 4095.
    #[inline]
    pub fn output(&self, phase: u32, att: u32, wave: u8) -> i32 {
        wave_out(tables(), wave, phase & 1023, att << 3)
    }

    /// Runs it a sample: its output, with `modulation` (in tenth-bit phase
    /// units, 1,024 a cycle) and `level` for its total level.
    #[inline]
    pub fn compute(&mut self, modulation: i32, level: u8, am: u16, wave: u8) -> i32 {
        let phase = (self.phase10() as i32 + modulation) as u32;
        let out = self.output(phase, self.attenuation(level, am), wave);
        self.out = [out, self.out[0]];
        out
    }

    /// Self-feedback: the modulation from its last two outputs, by the
    /// channel's feedback (0 none, 1 π/16 up to 7, 4π).
    #[inline]
    pub fn feedback(&self, fb: u8) -> i32 {
        if fb == 0 {
            0
        } else {
            (self.out[0] + self.out[1]) >> (9 - u32::from(fb.min(7)))
        }
    }
}

/// A waveform's output at a 10-bit phase, `att` log units down.
#[inline]
pub fn wave_out(t: &Tables, wave: u8, p: u32, att: u32) -> i32 {
    // The quarter sine's log at a phase, mirrored in each half.
    let sin = |p: u32| -> u32 {
        let i = if p & 256 != 0 { 255 - (p & 255) } else { p & 255 };
        u32::from(t.log_sin[i as usize])
    };
    let (log, negative) = match wave & 7 {
        0 => (sin(p), p & 512 != 0),
        1 => {
            if p & 512 != 0 {
                return 0;
            }
            (sin(p), false)
        }
        2 => (sin(p), false),
        3 => {
            if p & 256 != 0 {
                return 0;
            }
            (u32::from(t.log_sin[(p & 255) as usize]), false)
        }
        4 => {
            if p & 512 != 0 {
                return 0;
            }
            let q = (p << 1) & 1023;
            (sin(q), q & 512 != 0)
        }
        5 => {
            if p & 512 != 0 {
                return 0;
            }
            (sin((p << 1) & 1023), false)
        }
        6 => (0, p & 512 != 0),
        _ => {
            // The derived square: a log-linear ramp each half.
            let x = if p & 512 != 0 { 511 - (p & 511) } else { p & 511 };
            (x << 3, p & 512 != 0)
        }
    };
    let v = level(t, log + att);
    if negative {
        -v
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sine_peaks_where_it_should() {
        let t = tables();
        assert!(wave_out(t, 0, 256, 0) > 4000);
        assert!(wave_out(t, 0, 768, 0) < -4000);
        assert!(wave_out(t, 0, 0, 0).abs() < 30);
        // Half-sine and abs-sine.
        assert_eq!(wave_out(t, 1, 768, 0), 0);
        assert!(wave_out(t, 2, 768, 0) > 4000);
        // 6 dB down halves it.
        assert!((wave_out(t, 0, 256, 256) - 2047).abs() < 3);
    }

    #[test]
    fn decay_rate_one_takes_its_forty_seconds() {
        let mut op = Op { p: OpParams { attack: 15, decay: 1, sustain_level: 15, sustain: true, ..Default::default() }, ..Default::default() };
        op.key_on(1);
        let mut clock = Clock::default();
        let mut n = 0u32;
        while op.stage != Stage::Sustain && n < 49_716 * 120 {
            clock.step();
            op.clock_env(clock.counter);
            n += 1;
        }
        let secs = f64::from(n) / 49_716.0;
        // The datasheet: 39.3 s for decay rate 1, 0 to 96 dB (93 here).
        assert!((35.0..45.0).contains(&secs), "{secs}");
    }
}
