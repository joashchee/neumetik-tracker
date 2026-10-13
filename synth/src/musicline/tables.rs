// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! The replayer's tables, computed: the periods in 32nds of a semitone,
//! the vibrato and tremolo shapes, the filters' and the resonance's
//! coefficients, and the phase effect's resampling steps. Each is the
//! replayer's data, which the songs depend on; the formulas here give
//! them to the value, and the coefficient lists' few entries that sit a
//! step off their curve are written as they are.

use std::sync::OnceLock;

/// The periods of the five octaves in 32nds of a semitone (index 0 is
/// the lowest note, period 3,591; 1,952 entries).
pub fn periods() -> &'static [u16] {
    static T: OnceLock<Vec<u16>> = OnceLock::new();
    T.get_or_init(|| (0..61 * 32).map(|i| (3590.822 * 2f64.powf(-f64::from(i) / 384.0) + 0.5).floor() as u16).collect())
}

/// The period for a note in 32nds of a semitone. Below the table (down
/// to -32) the replayer reads the word before it, 0x4040, which the
/// caller's range then holds at the lowest note.
pub fn period(note: i32) -> u16 {
    if note < 0 {
        0x4040
    } else {
        periods().get(note as usize).copied().unwrap_or(106)
    }
}

/// The vibrato's and tremolo's shapes, 128 steps a cycle: sine, ramp
/// down, sawtooth, square.
pub fn shape(kind: u8, i: usize) -> i8 {
    let i = i & 127;
    match kind & 3 {
        0 => (32.475 * (std::f64::consts::TAU * i as f64 / 128.0).sin()) as i8,
        1 => (i.div_ceil(2)) as i8,
        2 if i < 64 => i.div_ceil(2) as i8,
        2 => -(((i - 63) / 2) as i8),
        _ if i < 64 => 0,
        _ => 64,
    }
}

/// ProTracker's vibrato sine, half a cycle in 32 steps.
pub fn pt_sine(i: usize) -> u8 {
    (255.0 * (std::f64::consts::PI * (i & 31) as f64 / 32.0).sin()) as u8
}

fn curve(from: f64, to: f64, fixes: &[(usize, u16)]) -> Vec<u16> {
    let mut t: Vec<u16> = (0..256).map(|i| (from * (to / from).powf(i as f64 / 255.0)).round() as u16).collect();
    for &(i, v) in fixes {
        t[i] = v;
    }
    t
}

/// The normal filter's cutoff coefficients: 1 to 1/16, down a curve.
pub fn filter() -> &'static [u16] {
    static T: OnceLock<Vec<u16>> = OnceLock::new();
    T.get_or_init(|| curve(32768.0, 2048.0, &[(105, 10462)]))
}

/// The resonant filter's cutoff coefficients: 1 to 1/128.
pub fn resonant_filter() -> &'static [u16] {
    static T: OnceLock<Vec<u16>> = OnceLock::new();
    T.get_or_init(|| curve(32768.0, 256.0, &[(27, 19604), (33, 17489), (59, 10664), (89, 6026), (91, 5801)]))
}

/// The resonance effect's cutoff periods.
pub fn resonance() -> &'static [u16] {
    static T: OnceLock<Vec<u16>> = OnceLock::new();
    T.get_or_init(|| curve(3000.0, 55.0, &[(101, 615)]))
}

/// The resonance effect's damping, by its amount.
pub fn damping() -> &'static [u16] {
    static T: OnceLock<Vec<u16>> = OnceLock::new();
    T.get_or_init(|| curve(24080.0, 256.0, &[(22, 16270), (51, 9704), (60, 8266), (61, 8120), (111, 3331), (116, 3047), (132, 2291), (183, 923)]))
}

/// The phase effect's resampling: which of a cycle's `len` points the
/// `j`th of `k` squeezed points (1-based) reads.
pub fn sizer(len: usize, k: usize, j: usize) -> usize {
    (j * len / k.max(1)).saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_periods_run_from_the_lowest_note_to_the_highest() {
        let p = periods();
        assert_eq!(p.len(), 1952);
        assert_eq!((p[0], p[1], p[1951]), (3591, 3584, 106));
        // ProTracker's C-2, 428, at its place.
        assert_eq!(p[1179], 428);
    }

    #[test]
    fn the_shapes_and_curves_have_their_ends() {
        assert_eq!((shape(0, 32), shape(0, 96), shape(0, 1)), (32, -32, 1));
        assert_eq!((shape(1, 127), shape(2, 63), shape(2, 64), shape(2, 127), shape(3, 64)), (64, 32, 0, -32, 64));
        assert_eq!((pt_sine(1), pt_sine(16)), (24, 255));
        assert_eq!((filter()[0], filter()[255], resonant_filter()[255], resonance()[0], damping()[255]), (32768, 2048, 256, 3000, 256));
        assert_eq!((sizer(16, 3, 1), sizer(16, 3, 2), sizer(16, 3, 3)), (4, 9, 15));
    }
}
