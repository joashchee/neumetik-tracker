//! **StarTrekker's AM instruments**: a 32-point cycle (sine, ramp,
//! square, or noise) looped, its loudness drawn by a five-stage envelope
//! run a tick at a time (from a start level up to an attack level, on to
//! a second one, down to the sustain level, held for a while, then
//! released to silence), with a vibrato, a pitch fall and an octave.
//!
//! From StarTrekker 1.2's documentation of its AM synth (the parameters
//! and what each stage does) and the format as the community described
//! it (where the parameters sit in the .NT file); the cycles are made
//! here from their shapes: the sine is 127 × sin rounded, the ramp climbs
//! by 8 from -128, the square is half -128 and half 127.

/// The waveforms, by `AmSound::wave`.
pub const NAMES: [&str; 4] = ["Sine", "Ramp", "Square", "Noise"];

/// An AM instrument: its envelope's levels (0 to 256) and speeds (level
/// a tick), and the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct AmSound {
    /// The level a note starts at.
    pub start: u16,
    /// The first attack's target and speed.
    pub attack1: (u16, u16),
    /// The second attack's target and speed.
    pub attack2: (u16, u16),
    /// The sustain level, and the decay's speed down to it.
    pub sustain: u16,
    pub decay: u16,
    /// Ticks the sustain level is held for.
    pub hold: u16,
    /// The release's speed down to silence.
    pub release: u16,
    /// 0 sine, 1 ramp, 2 square, 3 noise.
    pub wave: u8,
    /// Added to the Amiga period each tick (a falling pitch).
    pub pitch_fall: i16,
    /// The vibrato's depth and speed.
    pub vibrato_depth: u16,
    pub vibrato_speed: u16,
    /// Octaves down from the note played.
    pub octave: u16,
}

impl AmSound {
    /// The instrument from its 30 bytes in a .NT file (from its seventh
    /// byte on, past "AM" and four more), big-endian words.
    pub fn from_nt(b: &[u8]) -> AmSound {
        let w = |i: usize| b.get(i * 2..i * 2 + 2).map_or(0, |x| u16::from_be_bytes([x[0], x[1]]));
        AmSound {
            start: w(0),
            attack1: (w(1), w(2)),
            attack2: (w(3), w(4)),
            sustain: w(5),
            decay: w(6),
            hold: w(7),
            release: w(9),
            wave: w(10).min(3) as u8,
            pitch_fall: w(11) as i16,
            vibrato_depth: w(12),
            vibrato_speed: w(13),
            octave: w(14),
        }
    }

    /// Its cycle, to loop whole: 32 points, or 1,024 of noise.
    pub fn waveform(&self) -> Vec<i8> {
        match self.wave {
            0 => (0..32).map(|i| (127.0 * (i as f64 * std::f64::consts::TAU / 32.0).sin()).round() as i8).collect(),
            1 => (0..32).map(|i| (-128 + 8 * i) as i8).collect(),
            2 => (0..32).map(|i| if i < 16 { -128 } else { 127 }).collect(),
            _ => {
                let mut seed = 0x1357_9bdfu32;
                (0..1024)
                    .map(|_| {
                        seed ^= seed << 13;
                        seed ^= seed >> 17;
                        seed ^= seed << 5;
                        (seed >> 24) as u8 as i8
                    })
                    .collect()
            }
        }
    }

    /// The envelope as points (tick, level 0 to 64) to join with straight
    /// lines, and the point it holds on when a stage never ends (a speed
    /// of 0 with somewhere left to go). Each stage moves its speed a
    /// tick until it reaches its level, so lasts the distance over the
    /// speed, rounded up, at least a tick; the first moves on the note's
    /// own tick. The hold lasts its time and a tick more.
    pub fn envelope(&self) -> (Vec<(u16, u8)>, Option<usize>) {
        let mut pts: Vec<(u16, u8)> = Vec::new();
        let lv = |v: u16| (v.min(256) / 4) as u8;
        let push = |pts: &mut Vec<(u16, u8)>, dur: u16, level: u16| {
            let t = pts.last().map_or(0, |p| p.0).saturating_add(dur);
            pts.push((t, lv(level)));
        };
        // How long a stage from `from` to `to` at `speed` takes, or None
        // when it never gets there.
        let stage = |from: u16, to: u16, speed: u16| -> Option<u16> {
            let dist = from.abs_diff(to);
            if speed == 0 && dist != 0 {
                return None;
            }
            Some(if speed > 0 && dist > 0 { dist.div_ceil(speed) } else { 1 })
        };
        let (a1, s1) = self.attack1;
        let first = if a1 > self.start { (self.start + s1).min(a1) } else { self.start.saturating_sub(s1).max(a1) };
        pts.push((0, lv(first)));
        if first != a1 {
            match stage(first, a1, s1) {
                Some(d) => push(&mut pts, d, a1),
                None => return (pts, Some(0)),
            }
        }
        let steps = [(a1, self.attack2.0, self.attack2.1), (self.attack2.0, self.sustain, self.decay)];
        for (from, to, speed) in steps {
            match stage(from, to, speed) {
                Some(d) => push(&mut pts, d, to),
                None => {
                    push(&mut pts, 1, from);
                    let at = pts.len() - 1;
                    return (pts, Some(at));
                }
            }
        }
        push(&mut pts, self.hold.saturating_add(1), self.sustain);
        match stage(self.sustain, 0, self.release) {
            Some(d) => push(&mut pts, d, 0),
            None => {
                push(&mut pts, 1, self.sustain);
                let at = pts.len() - 1;
                return (pts, Some(at));
            }
        }
        (pts, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cycles_have_their_shapes() {
        let sine = AmSound { wave: 0, ..Default::default() }.waveform();
        assert_eq!(&sine[..9], &[0, 25, 49, 71, 90, 106, 117, 125, 127]);
        assert_eq!(sine[24], -127);
        let ramp = AmSound { wave: 1, ..Default::default() }.waveform();
        assert_eq!((ramp[0], ramp[16], ramp[31]), (-128, 0, 120));
        assert_eq!(AmSound { wave: 3, ..Default::default() }.waveform().len(), 1024);
    }

    #[test]
    fn stages_last_their_distance_over_their_speed() {
        let s = AmSound { start: 0, attack1: (256, 64), attack2: (256, 1), sustain: 128, decay: 16, hold: 3, release: 32, ..Default::default() };
        let (pts, hold) = s.envelope();
        assert_eq!(hold, None);
        // 64 at once, then three ticks to 256; the second attack a tick; 128 down at 16 is 8; held 4; 128 at 32 is 4.
        assert_eq!(pts, vec![(0, 16), (3, 64), (4, 64), (12, 32), (16, 32), (20, 0)]);
        let held = AmSound { start: 0, attack1: (256, 64), attack2: (100, 0), ..Default::default() };
        assert_eq!(held.envelope().1, Some(2));
    }
}
