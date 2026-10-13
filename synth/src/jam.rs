//! **JamCracker's AM instruments**: a 64-point cycle looped, remade
//! every tick while its channel's phase runs. Each point becomes the
//! average of the cycle's point and the one a quarter of the phase
//! further on (the cycle is stored twice over, so the shift wraps), a
//! comb whose teeth move as the phase steps (by 1 to 15 a tick, through
//! 256). The phase belongs to the channel, not the note: a new note
//! leaves it where it is. Each channel plays its own copy of the cycle
//! (the instrument keeps four after the cycle), and a copy keeps the
//! last mix made into it; one never mixed into plays as the file left
//! it.
//!
//! From JamCracker's replayer (Martin Gemmel's, kept with UADE) and
//! NostalgicPlayer's MIT-licensed reading of it, both read for what the
//! phase does, neither copied.

/// Points in a cycle.
pub const POINTS: usize = 64;

/// The channels' own copies, which come before the mixes in `waves`.
pub const COPIES: usize = 4;

/// Bytes an AM instrument holds: the cycle twice, then a copy a channel.
pub const BYTES: usize = 2 * POINTS + COPIES * POINTS;

/// The cycle (its 128 stored points) mixed with itself `shift` points on.
pub fn mix(cycle: &[i8], shift: usize) -> [i8; POINTS] {
    let at = |i: usize| i16::from(cycle.get(i).copied().unwrap_or(0));
    std::array::from_fn(|i| ((at(i) + at(i + shift)) >> 1) as i8)
}

/// The waves an AM instrument plays, from its bytes: the four channels'
/// copies as stored, then the cycle mixed at each shift (0 to 63).
pub fn waves(data: &[i8]) -> Vec<[i8; POINTS]> {
    let copy = |c: usize| std::array::from_fn(|i| data.get(2 * POINTS + c * POINTS + i).copied().unwrap_or(0));
    (0..COPIES).map(copy).chain((0..POINTS).map(|s| mix(data, s))).collect()
}

/// A channel's phase: where it is (0 to 255), and its step a tick (0
/// still; -1 a last mix at 0, then still).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Phase {
    pub at: u16,
    pub step: i16,
}

impl Phase {
    /// A row's phase command: FF sets it back to 0, 1 to 15 (the low
    /// nibble) start it from 0 at that step; 0 leaves it.
    pub fn command(&mut self, cmd: u8) {
        match cmd {
            0 => {}
            0xFF => *self = Phase { at: 0, step: -1 },
            c => *self = Phase { at: 0, step: i16::from(c & 15) },
        }
    }

    /// A tick on an AM instrument: the wave (in `waves`) its copy is
    /// mixed to now, when the phase runs.
    pub fn tick(&mut self) -> Option<usize> {
        if self.step == 0 {
            return None;
        }
        self.step = self.step.max(0);
        let wave = COPIES + usize::from(self.at >> 2);
        self.at = (self.at + self.step as u16) & 0xFF;
        Some(wave)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixes_average_the_cycle_with_itself_shifted() {
        let cycle: Vec<i8> = (0..128).map(|i| if i % 64 < 32 { 100 } else { -100 }).collect();
        assert_eq!(mix(&cycle, 0)[..], cycle[..64]);
        // Half a cycle on, the square cancels itself.
        assert!(mix(&cycle, 32).iter().all(|&x| x == 0));
        // Odd sums round down, as an arithmetic shift does.
        assert_eq!(mix(&[-1, 0], 1)[0], -1);
    }

    #[test]
    fn the_phase_steps_and_stops() {
        let mut p = Phase::default();
        assert_eq!(p.tick(), None);
        p.command(0x34);
        assert_eq!((p.tick(), p.tick(), p.at), (Some(COPIES), Some(COPIES + 1), 8));
        p.at = 254;
        assert_eq!((p.tick(), p.at), (Some(COPIES + 63), 2));
        // FF mixes once at 0, then holds.
        p.command(0xFF);
        assert_eq!((p.tick(), p.tick()), (Some(COPIES), None));
    }

    #[test]
    fn copies_come_first() {
        let data: Vec<i8> = (0..BYTES).map(|i| (i / 64) as i8).collect();
        let w = waves(&data);
        assert_eq!(w.len(), COPIES + POINTS);
        assert!(w[0].iter().all(|&x| x == 2) && w[3].iter().all(|&x| x == 5));
    }
}
