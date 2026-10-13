//! Reading bytes: a cursor over a file that never panics, every read
//! checked, and the little helpers the loaders share (names in old code
//! pages, sample data in its many encodings).

use std::fmt;

/// Why a file couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// None of the formats knows it.
    Unknown,
    /// It's the format named, but it ends early or doesn't add up.
    Bad(String),
    /// It's the format named, but uses something not read yet.
    Unsupported(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Unknown => write!(f, "it isn't a tracker module Neumetik knows"),
            Error::Bad(why) => write!(f, "{why}"),
            Error::Unsupported(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub fn bad<T>(why: impl Into<String>) -> Result<T> {
    Err(Error::Bad(why.into()))
}

/// A cursor over bytes. Reads past the end fail with `Error::Bad`, never
/// a panic, so a damaged file is only ever an error.
#[derive(Clone)]
pub struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, pos: 0 }
    }

    pub fn at(data: &'a [u8], pos: usize) -> Reader<'a> {
        Reader { data, pos }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn left(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn eof(&self) -> bool {
        self.pos >= self.data.len()
    }

    pub fn seek(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            return bad("it ends before a part it points to");
        }
        self.pos = pos;
        Ok(())
    }

    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.seek(self.pos.checked_add(n).ok_or(Error::Bad("an offset is out of range".into()))?)
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len()).ok_or_else(|| Error::Bad("it ends early".into()))?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    /// Up to `n` bytes: what's left when there are fewer (sample data is
    /// often cut short in the files found).
    pub fn bytes_upto(&mut self, n: usize) -> &'a [u8] {
        let end = self.pos.saturating_add(n).min(self.data.len());
        let s = &self.data[self.pos.min(end)..end];
        self.pos = end;
        s
    }

    pub fn peek(&self, n: usize) -> Option<&'a [u8]> {
        self.data.get(self.pos..self.pos.checked_add(n)?)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    pub fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }

    pub fn u16le(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u16be(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn i16le(&mut self) -> Result<i16> {
        Ok(self.u16le()? as i16)
    }

    pub fn i16be(&mut self) -> Result<i16> {
        Ok(self.u16be()? as i16)
    }

    pub fn u24le(&mut self) -> Result<u32> {
        let b = self.bytes(3)?;
        Ok(u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16)
    }

    pub fn u24be(&mut self) -> Result<u32> {
        let b = self.bytes(3)?;
        Ok(u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]))
    }

    pub fn u32le(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u32be(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32le(&mut self) -> Result<i32> {
        Ok(self.u32le()? as i32)
    }

    pub fn f32le(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32le()?))
    }

    pub fn tag(&mut self) -> Result<[u8; 4]> {
        let b = self.bytes(4)?;
        Ok([b[0], b[1], b[2], b[3]])
    }

    /// A fixed-length text field, as text.
    pub fn text(&mut self, n: usize) -> Result<String> {
        Ok(text(self.bytes(n)?))
    }

    /// A Pascal string: a length byte, then the bytes.
    pub fn pstring(&mut self) -> Result<String> {
        let n = self.u8()? as usize;
        self.text(n)
    }
}

/// A name from a file: up to its first NUL, in code page 437 (the DOS
/// trackers', and close enough for the Amiga's ASCII), with control
/// characters as spaces and the ends trimmed.
pub fn text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    let s: String = b[..end].iter().map(|&c| cp437(c)).collect();
    s.trim_end().trim_start_matches(' ').to_string()
}

/// Text that may run on past NULs (song messages): NULs as spaces, line
/// ends made `\n`.
pub fn message(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\r' => {
                s.push('\n');
                if b.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
            }
            b'\n' => s.push('\n'),
            0 => s.push(' '),
            c => s.push(cp437(c)),
        }
        i += 1;
    }
    s.trim_end().to_string()
}

/// A byte in code page 437.
pub fn cp437(c: u8) -> char {
    const HIGH: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■ ";
    match c {
        0..=31 | 127 => ' ',
        32..=126 => c as char,
        _ => HIGH.chars().nth(usize::from(c - 128)).unwrap_or(' '),
    }
}

/// How a sample's bytes are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pcm {
    S8,
    U8,
    /// Each byte the difference from the one before (8-bit delta).
    D8,
    S16le,
    U16le,
    S16be,
    D16le,
    D16be,
}

impl Pcm {
    pub fn bytes(self) -> usize {
        match self {
            Pcm::S8 | Pcm::U8 | Pcm::D8 => 1,
            _ => 2,
        }
    }

    pub fn is_16(self) -> bool {
        self.bytes() == 2
    }
}

/// Decodes `frames * channels` sample points from `b`, interleaved as
/// they come (channels > 1 means interleaved frames), cut short where
/// the bytes end. 8-bit points come out shifted to 16-bit.
pub fn decode(b: &[u8], pcm: Pcm, points: usize) -> Vec<i16> {
    let n = points.min(b.len() / pcm.bytes());
    let mut out = Vec::with_capacity(n);
    match pcm {
        Pcm::S8 => out.extend(b[..n].iter().map(|&v| i16::from(v as i8) << 8)),
        Pcm::U8 => out.extend(b[..n].iter().map(|&v| i16::from((v ^ 0x80) as i8) << 8)),
        Pcm::D8 => {
            let mut acc = 0u8;
            out.extend(b[..n].iter().map(|&v| {
                acc = acc.wrapping_add(v);
                i16::from(acc as i8) << 8
            }));
        }
        Pcm::S16le => out.extend(b.chunks_exact(2).take(n).map(|c| i16::from_le_bytes([c[0], c[1]]))),
        Pcm::U16le => out.extend(b.chunks_exact(2).take(n).map(|c| (u16::from_le_bytes([c[0], c[1]]) ^ 0x8000) as i16)),
        Pcm::S16be => out.extend(b.chunks_exact(2).take(n).map(|c| i16::from_be_bytes([c[0], c[1]]))),
        Pcm::D16le | Pcm::D16be => {
            let mut acc = 0u16;
            out.extend(b.chunks_exact(2).take(n).map(|c| {
                let d = if pcm == Pcm::D16le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) };
                acc = acc.wrapping_add(d);
                acc as i16
            }));
        }
    }
    out
}

/// Two channels stored one after the other (left, then right) made
/// interleaved.
pub fn interleave(planar: &[i16]) -> Vec<i16> {
    let half = planar.len() / 2;
    let mut out = Vec::with_capacity(half * 2);
    for i in 0..half {
        out.push(planar[i]);
        out.push(planar[half + i]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_never_panic_past_the_end() {
        let mut r = Reader::new(&[1, 2, 3]);
        assert_eq!(r.u16le().unwrap(), 0x0201);
        assert!(r.u16le().is_err());
        assert_eq!(r.u8().unwrap(), 3);
        assert!(r.u8().is_err());
        assert!(r.seek(4).is_err());
    }

    #[test]
    fn names_stop_at_nul_and_trim() {
        assert_eq!(text(b"  kick drum \0garbage"), "kick drum");
        assert_eq!(text(&[0x82, b'!']), "é!");
    }

    #[test]
    fn delta_and_unsigned_decode() {
        assert_eq!(decode(&[1, 1, 0xFE], Pcm::D8, 3), vec![256, 512, 0]);
        assert_eq!(decode(&[0x80, 0xFF], Pcm::U8, 2), vec![0, 127 << 8]);
        assert_eq!(decode(&[0x00, 0x80], Pcm::U16le, 1), vec![0]);
    }
}
