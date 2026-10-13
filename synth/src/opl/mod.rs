//! **FM, the OPL's way**: Yamaha's YM3812 (OPL2, the AdLib and Sound
//! Blaster) and YMF262 (OPL3), written from their public descriptions.
//!
//! - [`Patch`] and [`FmVoice`]: a two-operator instrument played on its
//!   own at any pitch and volume, as tracker modules' AdLib instruments
//!   (Scream Tracker 3's, Composer 670's) are.
//! - [`Poly`]: a patch played from notes, nine at once, as a keyboard
//!   plays it (the app's Synth tab), and [`PRESETS`] to start from.
//! - [`Chip`]: the chip itself, its registers written as a program would
//!   write them: nine channels (OPL2) or eighteen (OPL3) with four-operator
//!   channels and stereo, and the rhythm mode's five drums. For the formats
//!   that log or drive the chip (RAD, A2M, IMF, D00 and the rest).
//!
//! Each runs at the chip's own clock, 49,716 samples a second, so envelopes
//! and LFOs keep the chip's timing and steps, and resample to the rate
//! asked for.

mod chip;
pub mod op;
mod poly;
pub mod presets;
pub mod tables;
mod voice;

pub use chip::{Chip, ChipKind};
pub use op::OpParams;
pub use poly::{Poly, POLYPHONY};
pub use presets::PRESETS;
pub use voice::{FmVoice, Patch};

/// The chip's sample rate: its 14.318 MHz crystal over 288.
pub const CHIP_RATE: f64 = 14_318_180.0 / 288.0;

/// From the chip's rate to another, linearly between its samples.
#[derive(Clone, Copy, Debug)]
pub struct Resampler {
    step: f64,
    frac: f64,
    prev: (f32, f32),
    cur: (f32, f32),
    primed: bool,
}

impl Resampler {
    pub fn new(from: f64, to: f64) -> Resampler {
        Resampler { step: from / to.max(1.0), frac: 0.0, prev: (0.0, 0.0), cur: (0.0, 0.0), primed: false }
    }

    /// The next output frame, pulling chip frames from `source` as needed.
    #[inline]
    pub fn next(&mut self, mut source: impl FnMut() -> (f32, f32)) -> (f32, f32) {
        if !self.primed {
            self.cur = source();
            self.primed = true;
        }
        self.frac += self.step;
        while self.frac >= 1.0 {
            self.frac -= 1.0;
            self.prev = self.cur;
            self.cur = source();
        }
        let t = self.frac as f32;
        (self.prev.0 + (self.cur.0 - self.prev.0) * t, self.prev.1 + (self.cur.1 - self.prev.1) * t)
    }
}
