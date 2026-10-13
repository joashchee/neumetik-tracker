//! A patch played polyphonically from notes, as a keyboard plays it: nine
//! voices (the OPL2's nine channels), the oldest taken when all are in
//! use. Made once; `note_on`, `note_off` and `render` never allocate, so
//! it runs in an audio callback.

use super::voice::{FmVoice, Patch};

/// Voices at once.
pub const POLYPHONY: usize = 9;

#[derive(Clone, Debug)]
pub struct Poly {
    patch: Patch,
    voices: [FmVoice; POLYPHONY],
    /// Each voice's note (MIDI numbers) while it's held or ringing, and
    /// when it started.
    notes: [Option<u8>; POLYPHONY],
    ages: [u64; POLYPHONY],
    started: u64,
    /// The output's level for a voice at full volume.
    pub gain: f32,
}

impl Poly {
    pub fn new(patch: Patch, rate: u32) -> Poly {
        Poly { patch, voices: [FmVoice::new(patch, rate); POLYPHONY], notes: [None; POLYPHONY], ages: [0; POLYPHONY], started: 0, gain: 0.3 }
    }

    /// A new patch, heard at once on the notes sounding too.
    pub fn set_patch(&mut self, patch: Patch) {
        self.patch = patch;
        for v in &mut self.voices {
            v.set_patch(patch);
        }
    }

    pub fn patch(&self) -> Patch {
        self.patch
    }

    pub fn set_depths(&mut self, deep_am: bool, deep_vib: bool) {
        for v in &mut self.voices {
            v.set_depths(deep_am, deep_vib);
        }
    }

    /// A note (MIDI's numbers, 69 the A at 440 Hz) at a volume, 0 to 64.
    pub fn note_on(&mut self, note: u8, volume: u8) {
        // The same note again takes its own voice; else a silent one; else the oldest.
        let i = self
            .notes
            .iter()
            .position(|&n| n == Some(note))
            .or_else(|| (0..POLYPHONY).find(|&i| self.notes[i].is_none()))
            .unwrap_or_else(|| (0..POLYPHONY).min_by_key(|&i| self.ages[i]).unwrap_or(0));
        let v = &mut self.voices[i];
        v.key_off();
        v.set_volume(volume);
        v.key_on(440.0 * 2f64.powf((f64::from(note) - 69.0) / 12.0));
        self.notes[i] = Some(note);
        self.started += 1;
        self.ages[i] = self.started;
    }

    pub fn note_off(&mut self, note: u8) {
        for (v, n) in self.voices.iter_mut().zip(&self.notes) {
            if *n == Some(note) && v.held() {
                v.key_off();
            }
        }
    }

    /// Every note let go.
    pub fn all_off(&mut self) {
        self.voices.iter_mut().for_each(FmVoice::key_off);
    }

    /// The notes held down now.
    pub fn held(&self) -> impl Iterator<Item = u8> + '_ {
        self.voices.iter().zip(&self.notes).filter(|(v, _)| v.held()).filter_map(|(_, n)| *n)
    }

    /// Adds the next `left.len()` frames into `left` and `right`.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (v, n) in self.voices.iter_mut().zip(self.notes.iter_mut()) {
            if n.is_none() {
                continue;
            }
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                let x = v.next() * self.gain;
                *l += x;
                *r += x;
            }
            if v.done() {
                *n = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::presets::PRESETS;
    use super::*;

    #[test]
    fn notes_sound_ring_out_and_steal_the_oldest() {
        let mut p = Poly::new(PRESETS[0].1, 48_000);
        let (mut l, mut r) = (vec![0.0; 4800], vec![0.0; 4800]);
        p.note_on(60, 64);
        p.render(&mut l, &mut r);
        assert!(l.iter().any(|x| x.abs() > 0.05));
        assert_eq!(p.held().collect::<Vec<_>>(), [60]);
        for n in 0..12 {
            p.note_on(48 + n, 64);
        }
        assert_eq!(p.held().count(), POLYPHONY);
        assert!(!p.held().any(|n| n == 48), "the oldest went");
        p.all_off();
        assert_eq!(p.held().count(), 0);
        for _ in 0..40 {
            p.render(&mut l, &mut r);
        }
        assert!(p.notes.iter().all(Option::is_none));
    }
}
