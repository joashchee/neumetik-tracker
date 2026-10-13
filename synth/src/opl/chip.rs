//! The chip, by its registers: an OPL2's nine two-operator channels, or
//! an OPL3's eighteen in two banks with four-operator channels, stereo
//! and four more waveforms; and the rhythm mode's five drums.

use super::op::{Clock, Op};
use super::{Resampler, CHIP_RATE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipKind {
    /// YM3812: nine channels, four waveforms (when enabled), mono.
    Opl2,
    /// YMF262: eighteen channels, eight waveforms, four-operator
    /// channels and stereo once its NEW bit is set (an OPL2 till then).
    Opl3,
}

/// The first operator slot of each channel in a bank (its carrier is
/// three slots on).
const SLOT: [usize; 9] = [0, 1, 2, 6, 7, 8, 12, 13, 14];

/// The slot a register's low five bits address (None for the gaps).
fn slot_of(off: u8) -> Option<usize> {
    match off {
        0x00..=0x05 => Some(usize::from(off)),
        0x08..=0x0D => Some(usize::from(off) - 2),
        0x10..=0x15 => Some(usize::from(off) - 4),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Channel {
    fnum: u16,
    block: u8,
    key: bool,
    feedback: u8,
    additive: bool,
    left: bool,
    right: bool,
}

#[derive(Clone, Debug)]
pub struct Chip {
    kind: ChipKind,
    ops: [Op; 36],
    chans: [Channel; 18],
    clock: Clock,
    /// Register 0x01's waveform select enable (OPL2).
    wse: bool,
    /// Register 0x08's note select (which F-number bit key scaling reads).
    nts: bool,
    /// Register 0xBD: rhythm mode and its drums' keys.
    rhythm: bool,
    drums: u8,
    /// OPL3 mode (register 0x105's NEW bit), and 0x104's four-operator pairs.
    new: bool,
    four_op: u8,
    noise: u32,
    rate: u32,
    resample: Resampler,
    /// The output's level: a channel at full level is `gain` (OPL3s add
    /// up to eighteen of them).
    pub gain: f32,
}

impl Chip {
    pub fn new(kind: ChipKind, rate: u32) -> Chip {
        Chip {
            kind,
            ops: [Op::default(); 36],
            chans: [Channel { left: true, right: true, ..Default::default() }; 18],
            clock: Clock::default(),
            wse: false,
            nts: false,
            rhythm: false,
            drums: 0,
            new: false,
            four_op: 0,
            noise: 1,
            rate,
            resample: Resampler::new(CHIP_RATE, f64::from(rate.max(1))),
            gain: 0.25,
        }
    }

    pub fn kind(&self) -> ChipKind {
        self.kind
    }

    /// Every register back to zero and every note stopped.
    pub fn reset(&mut self) {
        let gain = self.gain;
        *self = Chip::new(self.kind, self.rate);
        self.gain = gain;
    }

    /// Writes a register: 0x000 to 0x0FF (the first bank), 0x100 to
    /// 0x1FF (the OPL3's second).
    pub fn write(&mut self, reg: u16, v: u8) {
        let bank = usize::from(reg >> 8 & 1);
        if bank == 1 && self.kind == ChipKind::Opl2 {
            return;
        }
        let r = (reg & 0xFF) as u8;
        match r {
            0x01 if bank == 0 => self.wse = v & 0x20 != 0,
            0x04 if bank == 1 => self.four_op = v & 0x3F,
            0x05 if bank == 1 => self.new = v & 1 != 0,
            0x08 if bank == 0 => self.nts = v & 0x40 != 0,
            0x20..=0x95 | 0xE0..=0xF5 => {
                let Some(s) = slot_of(r & 0x1F) else { return };
                let op = &mut self.ops[bank * 18 + s];
                match r & 0xE0 {
                    0x20 => op.p.set20(v),
                    0x40 => {
                        op.p.set40(v);
                        op.update_ksl();
                    }
                    0x60 => op.p.set60(v),
                    0x80 => op.p.set80(v),
                    _ => op.p.wave = v & 7,
                }
            }
            0xA0..=0xA8 | 0xB0..=0xB8 => {
                let c = bank * 9 + usize::from(r & 15);
                let ch = &mut self.chans[c];
                if r < 0xB0 {
                    ch.fnum = ch.fnum & 0x300 | u16::from(v);
                } else {
                    ch.fnum = ch.fnum & 0xFF | u16::from(v & 3) << 8;
                    ch.block = (v >> 2) & 7;
                    let key = v & 0x20 != 0;
                    if key != ch.key {
                        ch.key = key;
                        self.key(c, key);
                    }
                }
                self.update_freq(c);
            }
            0xBD if bank == 0 => {
                self.clock.deep_am = v & 0x80 != 0;
                self.clock.deep_vib = v & 0x40 != 0;
                let rhythm = v & 0x20 != 0;
                if self.rhythm && !rhythm {
                    for s in [12, 15, 13, 16, 14, 17] {
                        self.ops[s].key_off(2);
                    }
                }
                self.rhythm = rhythm;
                let drums = if rhythm { v & 0x1F } else { 0 };
                // Their slots, by bit: hi-hat, cymbal, tom, snare, bass drum.
                let slots: [&[usize]; 5] = [&[13], &[17], &[14], &[16], &[12, 15]];
                for (bit, slots) in slots.iter().enumerate() {
                    let on = drums & (1 << bit) != 0;
                    if on != (self.drums & (1 << bit) != 0) {
                        for &s in *slots {
                            if on {
                                self.ops[s].key_on(2);
                            } else {
                                self.ops[s].key_off(2);
                            }
                        }
                    }
                }
                self.drums = drums;
            }
            0xC0..=0xC8 => {
                let ch = &mut self.chans[bank * 9 + usize::from(r & 15)];
                ch.additive = v & 1 != 0;
                ch.feedback = (v >> 1) & 7;
                ch.left = v & 0x10 != 0;
                ch.right = v & 0x20 != 0;
            }
            _ => {}
        }
    }

    /// Whether channel `c` is the first of a four-operator pair, or the
    /// second (which plays as part of the first).
    fn pair(&self, c: usize) -> (bool, bool) {
        if !self.new || c % 9 >= 6 {
            return (false, false);
        }
        let i = c % 9 % 3 + 3 * (c / 9);
        let on = self.four_op & (1 << i) != 0;
        (on && c % 9 < 3, on && c % 9 >= 3)
    }

    fn ops_of(c: usize) -> [usize; 2] {
        let s = (c / 9) * 18 + SLOT[c % 9];
        [s, s + 3]
    }

    fn key(&mut self, c: usize, on: bool) {
        let (first, second) = self.pair(c);
        if second {
            return;
        }
        let mut slots = Self::ops_of(c).to_vec();
        if first {
            slots.extend(Self::ops_of(c + 3));
        }
        for s in slots {
            if on {
                self.ops[s].key_on(1);
            } else {
                self.ops[s].key_off(1);
            }
        }
    }

    fn update_freq(&mut self, c: usize) {
        let (first, second) = self.pair(c);
        let src = if second { c - 3 } else { c };
        let (f, b) = (self.chans[src].fnum, self.chans[src].block);
        let nts = self.nts;
        for s in Self::ops_of(c) {
            self.ops[s].set_freq(f, b, nts);
        }
        if first {
            for s in Self::ops_of(c + 3) {
                self.ops[s].set_freq(f, b, nts);
            }
        }
    }

    fn wave(&self, w: u8) -> u8 {
        if self.new {
            w & 7
        } else if self.wse {
            w & 3
        } else {
            0
        }
    }

    /// One operator, run with its own wave and level.
    #[inline]
    fn run(&mut self, s: usize, modulation: i32) -> i32 {
        let w = self.wave(self.ops[s].p.wave);
        let am = self.clock.am;
        let op = &mut self.ops[s];
        op.compute(modulation, op.p.level, am, w)
    }

    /// A chip sample, left and right, a channel at full level being 1.
    fn chip_sample(&mut self) -> (f32, f32) {
        self.clock.step();
        let counter = self.clock.counter;
        let banks = if self.new { 2 } else { 1 };
        for s in 0..18 * banks {
            let op = &mut self.ops[s];
            op.clock_env(counter);
            let fnum = op.fnum();
            let f = if op.p.vibrato { (i32::from(fnum) + self.clock.vibrato(fnum)).clamp(0, 1023) as u16 } else { fnum };
            op.clock_phase(f);
        }
        // The noise generator: a 23-bit shift register.
        let bit = (self.noise >> 22 ^ self.noise >> 8 ^ self.noise >> 7 ^ self.noise) & 1;
        self.noise = (self.noise << 1 | bit) & 0x7F_FFFF;
        let (mut l, mut r) = (0i32, 0i32);
        for c in 0..9 * banks {
            let (first, second) = self.pair(c);
            if second || (self.rhythm && (6..9).contains(&c)) {
                continue;
            }
            let ch = self.chans[c];
            let [a, b] = Self::ops_of(c);
            let fb = self.ops[a].feedback(ch.feedback);
            let out = if first {
                let [cc, d] = Self::ops_of(c + 3);
                let add2 = self.chans[c + 3].additive;
                match (ch.additive, add2) {
                    (false, false) => {
                        let x = self.run(a, fb);
                        let x = self.run(b, x);
                        let x = self.run(cc, x);
                        self.run(d, x)
                    }
                    (true, false) => {
                        let x = self.run(a, fb);
                        let y = self.run(b, 0);
                        let y = self.run(cc, y);
                        x + self.run(d, y)
                    }
                    (false, true) => {
                        let x = self.run(a, fb);
                        let x = self.run(b, x);
                        let y = self.run(cc, 0);
                        x + self.run(d, y)
                    }
                    (true, true) => {
                        let x = self.run(a, fb);
                        let y = self.run(b, 0);
                        let y = self.run(cc, y);
                        x + y + self.run(d, 0)
                    }
                }
            } else {
                let m = self.run(a, fb);
                if ch.additive {
                    m + self.run(b, 0)
                } else {
                    self.run(b, m)
                }
            };
            let (left, right) = if self.new { (ch.left, ch.right) } else { (true, true) };
            if left {
                l += out;
            }
            if right {
                r += out;
            }
        }
        if self.rhythm {
            let out = self.drums_sample();
            let ch = self.chans[6];
            let (left, right) = if self.new { (ch.left, ch.right) } else { (true, true) };
            if left {
                l += out;
            }
            if right {
                r += out;
            }
        }
        let k = self.gain / 4096.0;
        (l as f32 * k, r as f32 * k)
    }

    /// The rhythm mode's five drums, each twice a channel's level: the
    /// bass drum is channel 6's pair; the tom an operator's plain sine;
    /// the hi-hat, snare and cymbal take their phases from bits of
    /// channel 7's and 8's oscillators and the noise.
    fn drums_sample(&mut self) -> i32 {
        let ch6 = self.chans[6];
        let fb = self.ops[12].feedback(ch6.feedback);
        let m = self.run(12, fb);
        let bd = if ch6.additive { self.run(15, 0) } else { self.run(15, m) };
        let tom = self.run(14, 0);
        let p13 = self.ops[13].phase10();
        let p17 = self.ops[17].phase10();
        let noise = self.noise & 1;
        let ring = (((p13 >> 2) ^ (p13 >> 7)) | (p13 >> 3) | ((p17 >> 3) ^ (p17 >> 5))) & 1;
        let hh_phase = ring << 9 | if ring ^ noise != 0 { 0xD0 } else { 0x34 };
        let b8 = (p13 >> 8) & 1;
        let sd_phase = b8 << 9 | (b8 ^ noise) << 8;
        let cy_phase = ring << 9 | 0x100;
        let am = self.clock.am;
        let fixed = |s: usize, phase: u32| -> i32 {
            let w = self.wave(self.ops[s].p.wave);
            let op = &self.ops[s];
            op.output(phase, op.attenuation(op.p.level, am), w)
        };
        let hh = fixed(13, hh_phase);
        let sd = fixed(16, sd_phase);
        let cy = fixed(17, cy_phase);
        2 * (bd + tom + hh + sd + cy)
    }

    /// Adds the next `left.len()` frames into `left` and `right`.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let mut rs = self.resample;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let (a, b) = rs.next(|| self.chip_sample());
            *l += a;
            *r += b;
        }
        self.resample = rs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(chip: &mut Chip, c: u16, fnum: u16, block: u8) {
        // A plain sine on the channel's carrier, the modulator silent.
        let s = SLOT[usize::from(c % 9)] as u16;
        let off = |slot: u16| (slot / 6) * 8 + slot % 6 + (c / 9) * 0x100;
        let (m, k) = (off(s), off(s + 3));
        for (reg, v) in [(0x20, 0x01), (0x40, 0x3F), (0x60, 0xF0), (0x80, 0x0F)] {
            chip.write(reg + m, v);
        }
        for (reg, v) in [(0x20, 0x21), (0x40, 0x00), (0x60, 0xF0), (0x80, 0x0F)] {
            chip.write(reg + k, v);
        }
        let base = (c / 9) * 0x100 + c % 9;
        chip.write(0xC0 + base, 0x30);
        chip.write(0xA0 + base, (fnum & 0xFF) as u8);
        chip.write(0xB0 + base, 0x20 | block << 2 | (fnum >> 8) as u8);
    }

    #[test]
    fn a_channel_keyed_on_sounds_at_its_frequency() {
        let mut chip = Chip::new(ChipKind::Opl2, 48_000);
        // A440: block 4, F-number 580.
        tone(&mut chip, 0, 580, 4);
        let (mut l, mut r) = (vec![0.0; 48_000], vec![0.0; 48_000]);
        chip.render(&mut l, &mut r);
        let crossings = l.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((436..=444).contains(&crossings), "{crossings}");
        assert_eq!(l, r);
        // Keyed off: it releases to nothing.
        chip.write(0xB0, 0x10 | 2);
        let (mut l, mut r) = (vec![0.0; 48_000], vec![0.0; 48_000]);
        chip.render(&mut l, &mut r);
        assert!(l[40_000..].iter().all(|x| x.abs() < 1e-3));
    }

    #[test]
    fn opl3_pans_and_its_second_bank_plays() {
        let mut chip = Chip::new(ChipKind::Opl3, 48_000);
        chip.write(0x105, 1);
        tone(&mut chip, 9, 580, 4);
        chip.write(0x1C0, 0x10); // left only
        let (mut l, mut r) = (vec![0.0; 4800], vec![0.0; 4800]);
        chip.render(&mut l, &mut r);
        assert!(l.iter().any(|x| x.abs() > 0.1));
        assert!(r.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn the_drums_sound_in_rhythm_mode() {
        let mut chip = Chip::new(ChipKind::Opl2, 48_000);
        for s in [12u16, 13, 14, 15, 16, 17] {
            let off = (s / 6) * 8 + s % 6;
            chip.write(0x60 + off, 0xF4);
            chip.write(0x80 + off, 0x0F);
        }
        chip.write(0xA6, 0x80);
        chip.write(0xB6, 0x08);
        chip.write(0xA7, 0x80);
        chip.write(0xB7, 0x08);
        chip.write(0xBD, 0x20 | 0x01); // the hi-hat
        let (mut l, mut r) = (vec![0.0; 4800], vec![0.0; 4800]);
        chip.render(&mut l, &mut r);
        assert!(l.iter().any(|x| x.abs() > 0.05));
    }
}
