//! The OPL's tables, computed from what they stand for rather than
//! copied: its waveforms are read as a logarithm (a quarter sine's
//! log), attenuation is added in that domain, and an exponential table
//! turns the sum back into a level. That's how the chip avoids a
//! multiplier, and why its waveforms and envelopes sound the way they
//! do (`docs/synth-methods.md` has the sources).

use std::sync::OnceLock;

/// Log units an octave (6.02 dB): attenuation and the log-sine share them.
pub const OCTAVE: u32 = 256;

pub struct Tables {
    /// -log2(sin) over a quarter wave, 256 steps, in log units.
    pub log_sin: [u16; 256],
    /// 2^(-i/256) as a 12-bit level (4096 down to 2052).
    pub exp: [u16; 256],
}

pub fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let mut log_sin = [0u16; 256];
        let mut exp = [0u16; 256];
        for i in 0..256 {
            let angle = (i as f64 + 0.5) * std::f64::consts::PI / 512.0;
            log_sin[i] = (-angle.sin().log2() * f64::from(OCTAVE)).round().min(4095.0) as u16;
            exp[i] = (2f64.powf(-(i as f64) / 256.0) * 4096.0).round().min(4095.0) as u16;
        }
        Tables { log_sin, exp }
    })
}

/// A level from log units of attenuation: 4095 at none, halving every
/// octave, nothing past twelve.
#[inline]
pub fn level(t: &Tables, log: u32) -> i32 {
    let shift = log / OCTAVE;
    if shift >= 13 {
        0
    } else {
        i32::from(t.exp[(log % OCTAVE) as usize]) >> shift
    }
}

/// The multiplier's values, doubled (the first is a half): the
/// datasheet's 1/2, 1 to 10, 10, 12, 12, 15, 15.
pub const MULT2: [u32; 16] = [1, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 20, 24, 24, 30, 30];

/// Key scale level at block 7, by the F-number's top four bits, in
/// eighths of a decibel (the datasheet's table at 6 dB an octave: 0, 9,
/// 12, 13.875 ... 21 dB).
pub const KSL_EIGHTHS: [u16; 16] = [0, 72, 96, 111, 120, 129, 135, 141, 144, 150, 153, 156, 159, 162, 165, 168];

/// The envelope's step pattern over eight clocks, by a rate's low two
/// bits: rates in between the doublings step on more of the eight.
pub const ENV_STEPS: [[u8; 8]; 4] = [[0, 1, 0, 1, 0, 1, 0, 1], [0, 1, 0, 1, 1, 1, 0, 1], [0, 1, 1, 1, 0, 1, 1, 1], [0, 1, 1, 1, 1, 1, 1, 1]];

/// The F-number and block a frequency plays at (a multiplier of one):
/// the lowest block it fits in, for the finest steps.
pub fn fnum_block(hz: f64) -> (u16, u8) {
    let hz = hz.max(0.0);
    for block in 0..8u8 {
        let f = hz * f64::from(1u32 << (20 - u32::from(block))) / super::CHIP_RATE;
        if f < 1023.5 {
            return (f.round() as u16, block);
        }
    }
    (1023, 7)
}

/// The frequency an F-number and block play at (with a multiplier of one).
pub fn hz_of(fnum: u16, block: u8) -> f64 {
    f64::from(fnum) * super::CHIP_RATE / f64::from(1u32 << (20 - u32::from(block.min(7))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_meet_at_their_ends() {
        let t = tables();
        // The sine's peak is next to no attenuation; its start, a lot.
        assert!(t.log_sin[255] <= 1);
        assert!(t.log_sin[0] > 2000);
        assert_eq!(t.exp[0], 4095);
        assert!((2050..2060).contains(&t.exp[255]));
        assert_eq!(level(t, 0), 4095);
        assert_eq!(level(t, 256), 2047);
        assert_eq!(level(t, 13 * 256), 0);
    }

    #[test]
    fn middle_c_is_in_block_three() {
        let (f, b) = fnum_block(261.6256);
        assert_eq!(b, 3);
        assert!((hz_of(f, b) - 261.6256).abs() < 0.3);
        // A note an octave up halves nothing but moves the block.
        let (f2, b2) = fnum_block(523.25);
        assert_eq!((f2, b2), (f, b + 1));
    }
}
