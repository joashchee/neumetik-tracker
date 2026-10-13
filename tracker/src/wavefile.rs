// Copyright 2026 ansiapps. Neumetik Tracker: MIT licensed, see the LICENSE at the repository root.

//! A waveform packed as one file (`UMFW`): how the repository keeps
//! each waveform, and how a UMF carries the ones it bundles.

use crate::canon::{wave_id, Id};
use crate::lz;
use crate::model::Wave;

pub const MAGIC: &[u8; 4] = b"UMFW";

/// A waveform as a repository file: `UMFW`, a version, its depth and
/// channels, how it's packed, its frames, and its points packed, by
/// whichever of the two ways comes out smaller: as differences by `lz`
/// (version 1, as every Neumetik reads), or predicted by `wavepack`
/// (version 2).
pub fn pack_wave(w: &Wave) -> Vec<u8> {
    let ch = usize::from(w.channels.max(1));
    let lz = lz::compress(&lz::delta_pack(&w.pcm, ch, w.bits));
    let predicted = crate::wavepack::pack(&w.pcm, ch);
    let (version, body) = if predicted.len() < lz.len() { (2, predicted) } else { (1, lz) };
    let mut out = Vec::with_capacity(16 + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&[version, w.bits, w.channels.max(1), 0]);
    out.extend_from_slice(&(w.frames() as u64).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// A repository file's waveform; when `expect` is given, it must hash to it.
pub fn unpack_wave(b: &[u8], expect: Option<Id>) -> Result<Wave, String> {
    if b.len() < 16 || &b[..4] != MAGIC {
        return Err("it isn't a waveform file".into());
    }
    let version = b[4];
    if version > 2 {
        return Err("its waveform was packed by a newer Neumetik".into());
    }
    let (bits, ch) = (b[5], b[6].clamp(1, 2));
    let frames = u64::from_le_bytes(b[8..16].try_into().expect("8 bytes")) as usize;
    let points = frames.checked_mul(usize::from(ch)).filter(|&p| p <= 1 << 30).ok_or("its length is out of range")?;
    let pcm = if version == 2 {
        crate::wavepack::unpack(&b[16..], usize::from(ch), frames)?
    } else {
        let bytes = points * if bits <= 8 { 1 } else { 2 };
        let raw = lz::decompress(&b[16..], bytes).map_err(|e| e.to_string())?;
        lz::delta_unpack(&raw, usize::from(ch), bits)
    };
    if pcm.len() != points {
        return Err("its waveform is cut short".into());
    }
    let w = Wave::new(bits, ch, pcm);
    if let Some(id) = expect {
        if wave_id(&w) != id {
            return Err("its waveform doesn't match its ID".into());
        }
    }
    Ok(w)
}
