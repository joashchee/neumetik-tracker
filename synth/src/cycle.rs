//! **Single-cycle synths**: a waveform made from oscillators instead of
//! recorded, a cycle (or a few) long, to loop as a sample so it plays at
//! any note: the trackers' "chip" instruments, and Psycle's and
//! Renoise's generators made into samples. Sine, saw, pulse, triangle,
//! noise and a two-operator FM pair, optionally beating against a
//! second copy a cycle faster, and softened by a one-pole low-pass run
//! round the loop. The Tracker's sample editor makes its synths here.

/// The frames in one cycle: it plays middle C at 256 × 261.63 Hz.
pub const CYCLE: usize = 256;

/// The waveforms' names, by `Cycle::wave`.
pub const NAMES: [&str; 6] = ["Sine", "Saw", "Square", "Triangle", "Noise", "FM"];

/// A synth's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Cycle {
    /// 0 sine, 1 saw, 2 square, 3 triangle, 4 noise, 5 FM.
    pub wave: u8,
    /// The square's pulse width, 1 to 99 (percent).
    pub width: u8,
    /// FM: the modulator's ratio to the carrier (1 to 16), and its depth (0 to 100).
    pub ratio: u8,
    pub depth: u8,
    /// A second oscillator a cycle in `beat` faster over the sample: a
    /// slow beating (0 none).
    pub beat: u8,
    /// Low-passed (0 none to 100).
    pub soften: u8,
}

/// The waveform: a cycle (or, beating, `beat` × 4 cycles; noise,
/// 32,768 points), its peak at 1, to loop whole.
pub fn render(y: &Cycle) -> Vec<f32> {
    let cycles = if y.beat > 0 { usize::from(y.beat.min(64)) * 4 } else { 1 };
    let len = if y.wave == 4 { 32_768 } else { CYCLE * cycles };
    let mut seed = 0x2468_ace1u32;
    let osc = |kind: u8, ph: f32, seed: &mut u32| -> f32 {
        let ph = ph.rem_euclid(1.0);
        match kind {
            1 => 2.0 * ph - 1.0,
            2 => {
                if ph < f32::from(y.width.clamp(1, 99)) / 100.0 {
                    1.0
                } else {
                    -1.0
                }
            }
            3 => 1.0 - 4.0 * (ph - 0.5).abs(),
            4 => {
                *seed ^= *seed << 13;
                *seed ^= *seed >> 17;
                *seed ^= *seed << 5;
                (*seed as f32 / u32::MAX as f32) * 2.0 - 1.0
            }
            5 => {
                let mod_ph = ph * f32::from(y.ratio.clamp(1, 16));
                let index = f32::from(y.depth.min(100)) / 100.0 * 8.0;
                (2.0 * std::f32::consts::PI * ph + index * (2.0 * std::f32::consts::PI * mod_ph).sin()).sin()
            }
            _ => (2.0 * std::f32::consts::PI * ph).sin(),
        }
    };
    let mut pts: Vec<f32> = (0..len)
        .map(|i| {
            let ph = i as f32 / CYCLE as f32;
            let a = osc(y.wave, ph, &mut seed);
            if y.beat > 0 && y.wave != 4 {
                // A second voice a cycle ahead over the whole: one slow beat.
                let ph2 = ph * (cycles as f32 + 1.0) / cycles as f32;
                (a + osc(y.wave, ph2, &mut seed)) * 0.5
            } else {
                a
            }
        })
        .collect();
    // Softened: a one-pole low-pass run twice round, so the loop's ends meet.
    if y.soften > 0 {
        let k = 1.0 - f32::from(y.soften.min(100)) / 101.0;
        let mut mem = 0.0;
        for _ in 0..2 {
            for v in pts.iter_mut() {
                mem += (*v - mem) * k;
                *v = mem;
            }
        }
    }
    let peak = pts.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
    pts.iter().map(|v| v / peak).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cycle_loops_at_its_length_and_peaks_at_one() {
        for wave in 0..6 {
            let x = render(&Cycle { wave, width: 50, ratio: 2, depth: 40, ..Default::default() });
            assert_eq!(x.len(), if wave == 4 { 32_768 } else { CYCLE });
            let peak = x.iter().fold(0f32, |m, v| m.max(v.abs()));
            assert!((peak - 1.0).abs() < 1e-6, "{wave}");
        }
        assert_eq!(render(&Cycle { wave: 1, beat: 3, ..Default::default() }).len(), CYCLE * 12);
    }
}
