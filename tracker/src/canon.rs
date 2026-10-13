//! What makes two samples the same, and their IDs.
//!
//! - A **waveform**'s ID is the SHA-256 of its canonical form: the part
//!   that can be heard (nothing past a loop's end that's never played,
//!   no silence trailing a one-shot), at its own depth and channels.
//!   The same sound saved by two trackers, padded differently, is one
//!   waveform.
//! - A **sample**'s ID adds how it plays: its loop and sustain loop. The
//!   same waveform looped two ways is two samples sharing one waveform,
//!   stored once.
//! - An **instrument**'s ID adds its keymap (which sample, by ID, each
//!   note plays, and the note it plays it at) and its envelopes, fade,
//!   new note actions and filter.
//! - A **module**'s ID is the SHA-256 of the file as it came.
//!
//! IDs come from content alone, so the same sample has the same ID in
//! every repository, and repositories merge without two things ever
//! claiming one ID.

use std::fmt;
use std::str::FromStr;

use crate::hash::{hex, Sha256};
use crate::model::{Envelope, Instrument, Loop, LoopKind, Sample, Wave};

/// A content ID: 32 bytes of SHA-256, written as 64 hex digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Id(pub [u8; 32]);

impl Id {
    /// The first 12 digits, as people read it.
    pub fn short(&self) -> String {
        hex(&self.0[..6])
    }

    pub fn is_zero(&self) -> bool {
        self.0 == [0; 32]
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&hex(&self.0))
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Id({})", self.short())
    }
}

impl FromStr for Id {
    type Err = ();

    fn from_str(s: &str) -> Result<Id, ()> {
        let s = s.trim();
        if s.len() != 64 {
            return Err(());
        }
        let mut out = [0u8; 32];
        for (i, o) in out.iter_mut().enumerate() {
            *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2).ok_or(())?, 16).map_err(|_| ())?;
        }
        Ok(Id(out))
    }
}

/// Where a sample's audible part ends: at its loop's end when the loop
/// holds it there forever, else at its last frame that isn't silent.
pub fn audible_end(s: &Sample) -> usize {
    let frames = s.wave.frames();
    let lp = s.looped.clamped(frames as u32);
    let sus = s.sustain.clamped(frames as u32);
    if lp.on() && (!sus.on() || sus.end <= lp.end) {
        return lp.end as usize;
    }
    if sus.on() {
        return frames;
    }
    let ch = usize::from(s.wave.channels.max(1));
    let mut end = frames;
    while end > 0 && s.wave.pcm[(end - 1) * ch..end * ch].iter().all(|&v| v == 0) {
        end -= 1;
    }
    end
}

/// The sample's waveform cut to what's heard.
pub fn canonical_wave(s: &Sample) -> Wave {
    let end = audible_end(s);
    let ch = usize::from(s.wave.channels.max(1));
    Wave { bits: s.wave.bits, channels: s.wave.channels.max(1), pcm: s.wave.pcm[..end * ch].to_vec() }
}

/// A waveform's bytes as stored and hashed: 8-bit points as bytes,
/// 16-bit as little-endian pairs.
pub fn wave_bytes(w: &Wave) -> Vec<u8> {
    if w.bits <= 8 {
        w.pcm.iter().map(|&v| (v >> 8) as u8).collect()
    } else {
        w.pcm.iter().flat_map(|v| v.to_le_bytes()).collect()
    }
}

pub fn wave_id(w: &Wave) -> Id {
    let mut h = Sha256::new();
    h.update(b"umf-wave-1");
    h.update(&[w.bits, w.channels.max(1)]);
    h.update(&(w.frames() as u64).to_le_bytes());
    h.update(&wave_bytes(w));
    Id(h.finish())
}

fn put_loop(h: &mut Sha256, l: &Loop) {
    let kind = match l.kind {
        LoopKind::Off => 0u8,
        LoopKind::Forward => 1,
        LoopKind::PingPong => 2,
    };
    if kind == 0 || l.end <= l.start {
        h.update(&[0]);
    } else {
        h.update(&[kind]);
        h.update(&l.start.to_le_bytes());
        h.update(&l.end.to_le_bytes());
    }
}

/// A sample's ID: its waveform's, and its loops, on the canonical frames.
pub fn sample_id(wave: Id, s: &Sample) -> Id {
    let frames = audible_end(s) as u32;
    let mut h = Sha256::new();
    h.update(b"umf-sample-1");
    h.update(&wave.0);
    put_loop(&mut h, &s.looped.clamped(frames));
    put_loop(&mut h, &s.sustain.clamped(frames));
    Id(h.finish())
}

fn put_env(h: &mut Sha256, e: &Envelope) {
    if !e.enabled || e.points.is_empty() {
        h.update(&[0]);
        return;
    }
    h.update(&[1, e.points.len() as u8]);
    for p in &e.points {
        h.update(&p.tick.to_le_bytes());
        h.update(&[p.value as u8]);
    }
    let pair = |p: Option<(u8, u8)>| p.map_or([0xFF, 0xFF], |(a, b)| [a, b]);
    h.update(&pair(e.looped));
    h.update(&pair(e.sustain));
    h.update(&[u8::from(e.carry), u8::from(e.filter)]);
}

/// An instrument's ID: its keymap (by the samples' IDs) and its
/// envelopes, fade, actions and filter. `samples` gives each 1-based
/// sample slot's ID.
pub fn instrument_id(ins: &Instrument, samples: &dyn Fn(u16) -> Option<Id>) -> Id {
    let mut h = Sha256::new();
    h.update(b"umf-instrument-1");
    for (n, &(note, slot)) in ins.keymap.iter().enumerate() {
        match samples(slot) {
            Some(id) => {
                h.update(&[n as u8, note]);
                h.update(&id.0);
            }
            None => {
                h.update(&[n as u8, 0xFF]);
            }
        }
    }
    put_env(&mut h, &ins.volume_env);
    put_env(&mut h, &ins.pan_env);
    put_env(&mut h, &ins.pitch_env);
    h.update(&ins.fadeout.to_le_bytes());
    h.update(&[ins.global_volume, ins.pan.unwrap_or(0xFF), ins.nna as u8, ins.dct, ins.dca, ins.pitch_pan_separation as u8, ins.pitch_pan_center, ins.random_volume, ins.random_pan]);
    h.update(&[ins.cutoff.unwrap_or(0xFF), ins.resonance.unwrap_or(0xFF)]);
    Id(h.finish())
}

pub fn module_id(file: &[u8]) -> Id {
    let mut h = Sha256::new();
    h.update(file);
    Id(h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn sample(pcm: Vec<i16>, looped: Loop) -> Sample {
        Sample { wave: Arc::new(Wave::new(16, 1, pcm)), looped, ..Sample::empty() }
    }

    #[test]
    fn padding_doesnt_make_another_waveform() {
        let a = sample(vec![1, 2, 3, 0, 0], Loop::default());
        let b = sample(vec![1, 2, 3], Loop::default());
        assert_eq!(wave_id(&canonical_wave(&a)), wave_id(&canonical_wave(&b)));
        // Past a loop's end is never heard.
        let c = sample(vec![1, 2, 3, 9, 9], Loop::forward(0, 3));
        let d = sample(vec![1, 2, 3], Loop::forward(0, 3));
        assert_eq!(wave_id(&canonical_wave(&c)), wave_id(&canonical_wave(&d)));
        assert_eq!(sample_id(wave_id(&canonical_wave(&c)), &c), sample_id(wave_id(&canonical_wave(&d)), &d));
    }

    #[test]
    fn another_loop_is_another_sample_of_the_same_waveform() {
        let a = sample(vec![5, 6, 7, 8], Loop::forward(0, 4));
        let b = sample(vec![5, 6, 7, 8], Loop::forward(2, 4));
        let (wa, wb) = (wave_id(&canonical_wave(&a)), wave_id(&canonical_wave(&b)));
        assert_eq!(wa, wb);
        assert_ne!(sample_id(wa, &a), sample_id(wb, &b));
        // An 8-bit copy is another waveform.
        let mut c = a.clone();
        c.wave = Arc::new(Wave::new(8, 1, vec![5, 6, 7, 8]));
        assert_ne!(wave_id(&canonical_wave(&c)), wa);
    }

    #[test]
    fn ids_read_back_from_their_text() {
        let id = module_id(b"x");
        assert_eq!(id.to_string().parse::<Id>(), Ok(id));
        assert!("xyz".parse::<Id>().is_err());
    }
}
