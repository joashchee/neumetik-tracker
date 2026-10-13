//! Modules often come packed: gzip (.gz, .mdgz), zip (.zip, .mdz, .s3z,
//! .xmz, .itz) and zlib (inside Jazz Jackrabbit 2's .j2b). Inflate
//! (RFC 1951, Deutsch, 1996) is written here from the specification,
//! in the manner of a canonical-Huffman decoder read a bit at a time.

/// Bits read lowest first.
struct Bits<'a> {
    b: &'a [u8],
    pos: usize,
    bit: u32,
    buf: u32,
}

impl<'a> Bits<'a> {
    fn new(b: &'a [u8]) -> Bits<'a> {
        Bits { b, pos: 0, bit: 0, buf: 0 }
    }

    fn need(&mut self, n: u32) -> Option<u32> {
        while self.bit < n {
            let byte = *self.b.get(self.pos)?;
            self.pos += 1;
            self.buf |= u32::from(byte) << self.bit;
            self.bit += 8;
        }
        let v = self.buf & ((1u64 << n) - 1) as u32;
        self.buf = if n == 32 { 0 } else { self.buf >> n };
        self.bit -= n;
        Some(v)
    }

    fn align(&mut self) {
        self.buf = 0;
        self.bit = 0;
    }
}

/// A canonical Huffman code: how many codes of each length, and the
/// symbols in code order.
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Option<Huffman> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[usize::from(l)] += 1;
        }
        counts[0] = 0;
        let mut offs = [0u16; 16];
        for i in 1..16 {
            offs[i] = offs[i - 1] + counts[i - 1];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[usize::from(offs[usize::from(l)])] = s as u16;
                offs[usize::from(l)] += 1;
            }
        }
        Some(Huffman { counts, symbols })
    }

    fn decode(&self, bits: &mut Bits) -> Option<u16> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= bits.need(1)? as i32;
            let count = i32::from(self.counts[len]);
            if code - count < first {
                return self.symbols.get((index + (code - first)) as usize).copied();
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        None
    }
}

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// Inflates a raw deflate stream, making at most `most` bytes.
pub fn inflate(data: &[u8], most: usize) -> Option<Vec<u8>> {
    let mut bits = Bits::new(data);
    let mut out: Vec<u8> = Vec::with_capacity(data.len() * 3);
    loop {
        let last = bits.need(1)?;
        match bits.need(2)? {
            0 => {
                bits.align();
                let at = bits.pos;
                let len = usize::from(u16::from_le_bytes([*data.get(at)?, *data.get(at + 1)?]));
                bits.pos = at + 4;
                out.extend_from_slice(data.get(bits.pos..bits.pos + len)?);
                bits.pos += len;
            }
            kind @ (1 | 2) => {
                let (lit, dist) = if kind == 1 {
                    let mut l = [8u8; 288];
                    l[144..256].fill(9);
                    l[256..280].fill(7);
                    (Huffman::new(&l)?, Huffman::new(&[5u8; 30])?)
                } else {
                    let hlit = bits.need(5)? as usize + 257;
                    let hdist = bits.need(5)? as usize + 1;
                    let hclen = bits.need(4)? as usize + 4;
                    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                    let mut cl = [0u8; 19];
                    for &i in ORDER.iter().take(hclen) {
                        cl[i] = bits.need(3)? as u8;
                    }
                    let clh = Huffman::new(&cl)?;
                    let mut lengths = vec![0u8; hlit + hdist];
                    let mut i = 0;
                    while i < hlit + hdist {
                        let sym = clh.decode(&mut bits)?;
                        let (val, rep) = match sym {
                            0..=15 => (sym as u8, 1),
                            16 => (*lengths.get(i.checked_sub(1)?)?, 3 + bits.need(2)? as usize),
                            17 => (0, 3 + bits.need(3)? as usize),
                            _ => (0, 11 + bits.need(7)? as usize),
                        };
                        for _ in 0..rep {
                            *lengths.get_mut(i)? = val;
                            i += 1;
                        }
                    }
                    (Huffman::new(&lengths[..hlit])?, Huffman::new(&lengths[hlit..])?)
                };
                loop {
                    let sym = lit.decode(&mut bits)?;
                    match sym {
                        0..=255 => out.push(sym as u8),
                        256 => break,
                        _ => {
                            let i = usize::from(sym - 257);
                            let len = usize::from(*LEN_BASE.get(i)?) + bits.need(u32::from(LEN_EXTRA[i]))? as usize;
                            let d = usize::from(dist.decode(&mut bits)?);
                            let back = usize::from(*DIST_BASE.get(d)?) + bits.need(u32::from(DIST_EXTRA[d]))? as usize;
                            let from = out.len().checked_sub(back)?;
                            for k in 0..len {
                                let b = out[from + k];
                                out.push(b);
                            }
                        }
                    }
                    if out.len() > most {
                        return None;
                    }
                }
            }
            _ => return None,
        }
        if out.len() > most {
            return None;
        }
        if last == 1 {
            return Some(out);
        }
    }
}

/// The most a packed module may unpack to.
const MOST: usize = 256 << 20;

pub fn is_gzip(d: &[u8]) -> bool {
    d.len() > 18 && d[0] == 0x1F && d[1] == 0x8B && d[2] == 8
}

pub fn gunzip(d: &[u8]) -> Option<Vec<u8>> {
    if !is_gzip(d) {
        return None;
    }
    let flags = d[3];
    let mut at = 10;
    if flags & 4 != 0 {
        at += 2 + usize::from(u16::from_le_bytes([*d.get(at)?, *d.get(at + 1)?]));
    }
    for bit in [8u8, 16] {
        if flags & bit != 0 {
            at += d.get(at..)?.iter().position(|&c| c == 0)? + 1;
        }
    }
    if flags & 2 != 0 {
        at += 2;
    }
    inflate(d.get(at..)?, MOST)
}

pub fn zlib(d: &[u8], most: usize) -> Option<Vec<u8>> {
    if d.len() < 2 || d[0] & 0x0F != 8 || (u16::from(d[0]) << 8 | u16::from(d[1])) % 31 != 0 {
        return None;
    }
    inflate(&d[2..], most)
}

pub fn is_zip(d: &[u8]) -> bool {
    d.len() > 30 && &d[..4] == b"PK\x03\x04"
}

/// Every file in a zip: its name and its bytes (those it can unpack).
pub fn unzip(d: &[u8]) -> Vec<(String, Vec<u8>)> {
    let u16le = |i: usize| d.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let u32le = |i: usize| d.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let mut out = Vec::new();
    // The central directory has the sizes even when the local headers don't.
    let Some(end) = (0..d.len().saturating_sub(21)).rev().take(70_000).find(|&i| &d[i..i + 4] == b"PK\x05\x06") else { return out };
    let (Some(count), Some(mut at)) = (u16le(end + 10), u32le(end + 16)) else { return out };
    for _ in 0..count {
        if d.get(at..at + 4) != Some(b"PK\x01\x02") {
            break;
        }
        let (Some(method), Some(csize), Some(usize_), Some(nlen), Some(xlen), Some(clen), Some(local)) =
            (u16le(at + 10), u32le(at + 20), u32le(at + 24), u16le(at + 28), u16le(at + 30), u16le(at + 32), u32le(at + 42))
        else {
            break;
        };
        let name = String::from_utf8_lossy(d.get(at + 46..at + 46 + nlen).unwrap_or(&[])).into_owned();
        at += 46 + nlen + xlen + clen;
        let (Some(lnlen), Some(lxlen)) = (u16le(local + 26), u16le(local + 28)) else { continue };
        let start = local + 30 + lnlen + lxlen;
        let Some(body) = d.get(start..start + csize) else { continue };
        let data = match method {
            0 => Some(body.to_vec()),
            8 => inflate(body, MOST.min(usize_.max(1) * 2 + 1024)),
            _ => None,
        };
        if let Some(data) = data {
            out.push((name, data));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflates_fixed_and_stored_blocks() {
        // "hello hello hello" deflated with fixed codes (by zlib, level 9).
        let packed = [0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x90, 0x00];
        assert_eq!(inflate(&packed, 1000).unwrap(), b"hello hello hello");
        // A stored block.
        let stored = [0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c'];
        assert_eq!(inflate(&stored, 1000).unwrap(), b"abc");
        assert!(inflate(&[0xff, 0xff], 1000).is_none());
    }
}
