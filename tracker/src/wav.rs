//! WAV files: interleaved stereo written as 16-bit PCM, and WAV files
//! read as a sample's waveform (for the editor's Load Sample).

use crate::io::{bad, decode, Pcm, Reader, Result};
use crate::model::Wave;

/// Interleaved stereo at `rate` as a 16-bit WAV file.
pub fn write(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

/// A waveform written as a sample's own WAV file (mono or stereo, at its
/// own depth), with its rate.
pub fn write_wave(wave: &Wave, rate: u32) -> Vec<u8> {
    let ch = u16::from(wave.channels.max(1));
    let bytes = if wave.bits == 8 { 1u16 } else { 2 };
    let data_len = (wave.pcm.len() * usize::from(bytes)) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len + (data_len & 1)).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&ch.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(ch * bytes)).to_le_bytes());
    out.extend_from_slice(&(ch * bytes).to_le_bytes());
    out.extend_from_slice(&(bytes * 8).to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &v in &wave.pcm {
        if bytes == 1 {
            out.push(((v >> 8) as u8) ^ 0x80);
        } else {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    if data_len & 1 == 1 {
        out.push(0);
    }
    out
}

/// A WAV file as a waveform and its rate: PCM of 8, 16, 24 or 32 bits,
/// or 32-bit float, mono or stereo (more channels keep the first two).
pub fn read(data: &[u8]) -> Result<(Wave, u32)> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return bad("it isn't a WAV file");
    }
    let mut r = Reader::at(data, 12);
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    while r.left() >= 8 {
        let id = r.tag()?;
        let len = r.u32le()? as usize;
        let start = r.pos;
        if &id == b"fmt " {
            let tag = r.u16le()?;
            let ch = r.u16le()?;
            let rate = r.u32le()?;
            r.skip(6)?;
            let bits = r.u16le()?;
            let tag = if tag == 0xFFFE && len >= 26 {
                r.skip(2 + 2 + 4)?;
                r.u16le()?
            } else {
                tag
            };
            fmt = Some((tag, ch.max(1), rate, bits));
        } else if &id == b"data" {
            let (tag, ch, rate, bits) = fmt.ok_or_else(|| crate::io::Error::Bad("its format comes after its data".into()))?;
            let body = &data[start..(start + len).min(data.len())];
            let chs = usize::from(ch);
            let pcm: Vec<i16> = match (tag, bits) {
                (1, 8) => decode(body, Pcm::U8, body.len()),
                (1, 16) => decode(body, Pcm::S16le, body.len() / 2),
                (1, 24) => body.chunks_exact(3).map(|c| i16::from_le_bytes([c[1], c[2]])).collect(),
                (1, 32) => body.chunks_exact(4).map(|c| i16::from_le_bytes([c[2], c[3]])).collect(),
                (3, 32) => body.chunks_exact(4).map(|c| (f32::from_le_bytes([c[0], c[1], c[2], c[3]]).clamp(-1.0, 1.0) * 32767.0) as i16).collect(),
                _ => return Err(crate::io::Error::Unsupported("that kind of WAV file isn't read".into())),
            };
            let keep = chs.min(2);
            let pcm: Vec<i16> = if chs <= 2 { pcm } else { pcm.chunks_exact(chs).flat_map(|f| f[..2].to_vec()).collect() };
            return Ok((Wave::new(if bits == 8 { 8 } else { 16 }, keep as u8, pcm), rate.max(1)));
        }
        r.seek((start + len + (len & 1)).min(data.len()))?;
    }
    bad("it has no sound in it")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wave_round_trips() {
        let w = Wave::new(16, 2, vec![0, 100, -100, 32767, -32768, 5]);
        let (back, rate) = read(&write_wave(&w, 22050)).unwrap();
        assert_eq!((back, rate), (w, 22050));
        let w8 = Wave::new(8, 1, vec![0, 256, -512]);
        assert_eq!(read(&write_wave(&w8, 8363)).unwrap().0, w8);
    }
}
