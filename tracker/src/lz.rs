//! Neumetik's own compression, for the repository's waveforms, UMF's
//! embedded samples and `soundpak.rep`: LZ77 (a hash-chain match finder,
//! one step of lazy matching, the last distance repeated cheaply) with
//! every decision coded by an adaptive binary range coder. The ideas are
//! decades old and public (Lempel and Ziv, 1977; range coding, Martin,
//! 1979); this is written from scratch.
//!
//! A stream is `NLZ1`, a method byte (0 stored, 1 coded), the length
//! unpacked (8 bytes, little-endian), then the bytes. Decoding never
//! panics and never makes more than the length it was told, whatever
//! the bytes.
//!
//! PCM compresses far better as differences, a byte plane at a time:
//! `delta_pack` and `delta_unpack` do that before and after.

const MAGIC: &[u8; 4] = b"NLZ1";
const BITS: u32 = 11;
pub(crate) const ONE: u16 = 1 << BITS;
const MOVE: u32 = 5;
const TOP: u32 = 1 << 24;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 273;
const WINDOW: usize = 1 << 22;
const HASH_BITS: u32 = 17;
const CHAIN: usize = 48;

pub(crate) struct Encoder {
    low: u64,
    range: u32,
    cache: u8,
    cache_size: u64,
    out: Vec<u8>,
}

impl Encoder {
    pub(crate) fn new(capacity: usize) -> Encoder {
        Encoder { low: 0, range: 0xFFFF_FFFF, cache: 0, cache_size: 1, out: Vec::with_capacity(capacity) }
    }

    fn shift_low(&mut self) {
        if self.low < 0xFF00_0000 || self.low > 0xFFFF_FFFF {
            let carry = (self.low >> 32) as u8;
            let mut temp = self.cache;
            loop {
                self.out.push(temp.wrapping_add(carry));
                temp = 0xFF;
                self.cache_size -= 1;
                if self.cache_size == 0 {
                    break;
                }
            }
            self.cache = ((self.low >> 24) & 0xFF) as u8;
        }
        self.cache_size += 1;
        self.low = (self.low & 0x00FF_FFFF) << 8;
    }

    pub(crate) fn bit(&mut self, p: &mut u16, bit: u32) {
        let bound = (self.range >> BITS) * u32::from(*p);
        if bit == 0 {
            self.range = bound;
            *p += (ONE - *p) >> MOVE;
        } else {
            self.low += u64::from(bound);
            self.range -= bound;
            *p -= *p >> MOVE;
        }
        while self.range < TOP {
            self.range <<= 8;
            self.shift_low();
        }
    }

    pub(crate) fn direct(&mut self, v: u32, n: u32) {
        for i in (0..n).rev() {
            self.range >>= 1;
            if (v >> i) & 1 != 0 {
                self.low += u64::from(self.range);
            }
            while self.range < TOP {
                self.range <<= 8;
                self.shift_low();
            }
        }
    }

    fn tree(&mut self, probs: &mut [u16], v: u32, n: u32) {
        let mut m = 1usize;
        for i in (0..n).rev() {
            let b = (v >> i) & 1;
            self.bit(&mut probs[m], b);
            m = (m << 1) | b as usize;
        }
    }

    fn reverse(&mut self, probs: &mut [u16], mut v: u32, n: u32) {
        let mut m = 1usize;
        for _ in 0..n {
            let b = v & 1;
            v >>= 1;
            self.bit(&mut probs[m], b);
            m = (m << 1) | b as usize;
        }
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        for _ in 0..5 {
            self.shift_low();
        }
        self.out
    }
}

pub(crate) struct Decoder<'a> {
    b: &'a [u8],
    pos: usize,
    code: u32,
    range: u32,
}

impl<'a> Decoder<'a> {
    pub(crate) fn new(b: &'a [u8]) -> Decoder<'a> {
        let mut d = Decoder { b, pos: 0, code: 0, range: 0xFFFF_FFFF };
        for _ in 0..5 {
            d.code = (d.code << 8) | u32::from(d.next());
        }
        d
    }

    fn next(&mut self) -> u8 {
        let v = self.b.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        v
    }

    pub(crate) fn bit(&mut self, p: &mut u16) -> u32 {
        let bound = (self.range >> BITS) * u32::from(*p);
        let bit = if self.code < bound {
            self.range = bound;
            *p += (ONE - *p) >> MOVE;
            0
        } else {
            self.code -= bound;
            self.range -= bound;
            *p -= *p >> MOVE;
            1
        };
        while self.range < TOP {
            self.range <<= 8;
            self.code = (self.code << 8) | u32::from(self.next());
        }
        bit
    }

    pub(crate) fn direct(&mut self, n: u32) -> u32 {
        let mut v = 0;
        for _ in 0..n {
            self.range >>= 1;
            let b = if self.code >= self.range {
                self.code -= self.range;
                1
            } else {
                0
            };
            v = (v << 1) | b;
            while self.range < TOP {
                self.range <<= 8;
                self.code = (self.code << 8) | u32::from(self.next());
            }
        }
        v
    }

    fn tree(&mut self, probs: &mut [u16], n: u32) -> u32 {
        let mut m = 1usize;
        for _ in 0..n {
            m = (m << 1) | self.bit(&mut probs[m]) as usize;
        }
        (m - (1 << n)) as u32
    }

    fn reverse(&mut self, probs: &mut [u16], n: u32) -> u32 {
        let mut m = 1usize;
        let mut v = 0;
        for i in 0..n {
            let b = self.bit(&mut probs[m]);
            m = (m << 1) | b as usize;
            v |= b << i;
        }
        v
    }

    /// Past the end by more than the coder's flush: the stream is bad.
    pub(crate) fn overrun(&self) -> bool {
        self.pos > self.b.len() + 8
    }
}

/// Lengths: 2 to 9 low, 10 to 17 middle, 18 up high.
struct Lengths {
    choice: u16,
    choice2: u16,
    low: [[u16; 8]; 4],
    mid: [[u16; 8]; 4],
    high: [u16; 256],
}

impl Lengths {
    fn new() -> Lengths {
        Lengths { choice: ONE / 2, choice2: ONE / 2, low: [[ONE / 2; 8]; 4], mid: [[ONE / 2; 8]; 4], high: [ONE / 2; 256] }
    }

    fn encode(&mut self, e: &mut Encoder, len: usize, ctx: usize) {
        let v = (len - 2) as u32;
        if v < 8 {
            e.bit(&mut self.choice, 0);
            e.tree(&mut self.low[ctx], v, 3);
        } else if v < 16 {
            e.bit(&mut self.choice, 1);
            e.bit(&mut self.choice2, 0);
            e.tree(&mut self.mid[ctx], v - 8, 3);
        } else {
            e.bit(&mut self.choice, 1);
            e.bit(&mut self.choice2, 1);
            e.tree(&mut self.high, v - 16, 8);
        }
    }

    fn decode(&mut self, d: &mut Decoder, ctx: usize) -> usize {
        let v = if d.bit(&mut self.choice) == 0 {
            d.tree(&mut self.low[ctx], 3)
        } else if d.bit(&mut self.choice2) == 0 {
            8 + d.tree(&mut self.mid[ctx], 3)
        } else {
            16 + d.tree(&mut self.high, 8)
        };
        v as usize + 2
    }
}

/// Every probability the coder adapts.
struct Model {
    is_match: [u16; 8],
    is_rep: [u16; 8],
    literal: Vec<u16>,
    lengths: Lengths,
    reps: Lengths,
    slot: [[u16; 64]; 4],
    spec: Vec<u16>,
    align: [u16; 16],
}

impl Model {
    fn new() -> Model {
        Model {
            is_match: [ONE / 2; 8],
            is_rep: [ONE / 2; 8],
            literal: vec![ONE / 2; 8 * 0x300],
            lengths: Lengths::new(),
            reps: Lengths::new(),
            slot: [[ONE / 2; 64]; 4],
            spec: vec![ONE / 2; 14 * 32],
            align: [ONE / 2; 16],
        }
    }
}

/// The literal coder: the previous byte's top bits as context; after a
/// match, the byte the match would have given steers it (LZMA's
/// "matched literal", which pays off on PCM's near-repeats).
fn literal_ctx(prev: u8) -> usize {
    usize::from(prev >> 5) * 0x300
}

fn encode_literal(e: &mut Encoder, probs: &mut [u16], byte: u8, matched: Option<u8>) {
    let mut m = 1usize;
    match matched {
        None => {
            for i in (0..8).rev() {
                let b = u32::from(byte >> i) & 1;
                e.bit(&mut probs[m], b);
                m = (m << 1) | b as usize;
            }
        }
        Some(mb) => {
            let mut same = true;
            for i in (0..8).rev() {
                let b = u32::from(byte >> i) & 1;
                if same {
                    let mbit = usize::from(mb >> i) & 1;
                    e.bit(&mut probs[0x100 + (mbit << 8) + m], b);
                    same = mbit == b as usize;
                } else {
                    e.bit(&mut probs[m], b);
                }
                m = (m << 1) | b as usize;
            }
        }
    }
}

fn decode_literal(d: &mut Decoder, probs: &mut [u16], matched: Option<u8>) -> u8 {
    let mut m = 1usize;
    match matched {
        None => {
            while m < 0x100 {
                m = (m << 1) | d.bit(&mut probs[m]) as usize;
            }
        }
        Some(mb) => {
            let mut same = true;
            for i in (0..8).rev() {
                let b = if same {
                    let mbit = usize::from(mb >> i) & 1;
                    let b = d.bit(&mut probs[0x100 + (mbit << 8) + m]) as usize;
                    same = mbit == b;
                    b
                } else {
                    d.bit(&mut probs[m]) as usize
                };
                m = (m << 1) | b;
            }
        }
    }
    (m & 0xFF) as u8
}

fn slot_of(dist: u32) -> u32 {
    if dist < 4 {
        dist
    } else {
        let top = 31 - dist.leading_zeros();
        (top << 1) | ((dist >> (top - 1)) & 1)
    }
}

fn encode_distance(e: &mut Encoder, m: &mut Model, dist: u32, len: usize) {
    let ctx = (len - 2).min(3);
    let slot = slot_of(dist);
    e.tree(&mut m.slot[ctx], slot, 6);
    if slot >= 4 {
        let n = (slot >> 1) - 1;
        let base = (2 | (slot & 1)) << n;
        let extra = dist - base;
        if slot < 14 {
            let at = slot as usize * 32;
            e.reverse(&mut m.spec[at..at + 32], extra, n);
        } else {
            e.direct(extra >> 4, n - 4);
            e.reverse(&mut m.align, extra & 15, 4);
        }
    }
}

fn decode_distance(d: &mut Decoder, m: &mut Model, len: usize) -> u32 {
    let ctx = (len - 2).min(3);
    let slot = d.tree(&mut m.slot[ctx], 6);
    if slot < 4 {
        return slot;
    }
    let n = (slot >> 1) - 1;
    let base = (2 | (slot & 1)) << n;
    if slot < 14 {
        let at = slot as usize * 32;
        base + d.reverse(&mut m.spec[at..at + 32], n)
    } else {
        base + (d.direct(n - 4) << 4) + d.reverse(&mut m.align, 4)
    }
}

/// How long the bytes at `a` and `b` agree, up to `most`.
fn agree(data: &[u8], a: usize, b: usize, most: usize) -> usize {
    let mut n = 0;
    while n < most && data[a + n] == data[b + n] {
        n += 1;
    }
    n
}

struct Finder {
    head: Vec<i64>,
    prev: Vec<i64>,
    /// Every position before this is in the chains.
    done: usize,
}

impl Finder {
    fn new(len: usize) -> Finder {
        Finder { head: vec![-1; 1 << HASH_BITS], prev: vec![-1; len.min(WINDOW).max(1)], done: 0 }
    }

    /// Puts every position up to `i` in the chains, each once.
    fn upto(&mut self, data: &[u8], i: usize) {
        while self.done <= i {
            self.insert(data, self.done);
            self.done += 1;
        }
    }

    fn hash(data: &[u8], i: usize) -> usize {
        let v = u32::from(data[i]) | u32::from(data[i + 1]) << 8 | u32::from(data[i + 2]) << 16;
        (v.wrapping_mul(2_654_435_761) >> (32 - HASH_BITS)) as usize
    }

    fn insert(&mut self, data: &[u8], i: usize) {
        if i + MIN_MATCH > data.len() {
            return;
        }
        let h = Finder::hash(data, i);
        let n = self.prev.len();
        self.prev[i % n] = self.head[h];
        self.head[h] = i as i64;
    }

    /// The longest match for the bytes at `i` (inserted already), as
    /// (length, distance - 1).
    fn best(&self, data: &[u8], i: usize) -> (usize, u32) {
        let most = (data.len() - i).min(MAX_MATCH);
        if most < MIN_MATCH {
            return (0, 0);
        }
        let n = self.prev.len();
        let mut cand = self.prev[i % n];
        let (mut len, mut dist) = (0, 0);
        for _ in 0..CHAIN {
            if cand < 0 {
                break;
            }
            let c = cand as usize;
            if i - c > WINDOW.min(n) - 1 {
                break;
            }
            if data[c + len.min(most - 1)] == data[i + len.min(most - 1)] {
                let l = agree(data, c, i, most);
                if l > len {
                    len = l;
                    dist = (i - c - 1) as u32;
                    if l == most {
                        break;
                    }
                }
            }
            let next = self.prev[c % n];
            if next >= cand {
                break;
            }
            cand = next;
        }
        if len >= MIN_MATCH {
            (len, dist)
        } else {
            (0, 0)
        }
    }
}

/// Packs `data`.
pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(13 + data.len() / 2);
    out.extend_from_slice(MAGIC);
    let coded = encode(data);
    if coded.len() >= data.len() {
        out.push(0);
        out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        out.extend_from_slice(data);
    } else {
        out.push(1);
        out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        out.extend_from_slice(&coded);
    }
    out
}

fn encode(data: &[u8]) -> Vec<u8> {
    let mut e = Encoder::new(data.len() / 2 + 16);
    let mut m = Model::new();
    let mut f = Finder::new(data.len());
    let mut state = 0usize;
    let mut rep0: u32 = 0;
    let mut i = 0;
    let mut pending: Option<(usize, u32)> = None;
    let mut after_match = false;
    while i < data.len() {
        f.upto(data, i);
        let (len, dist) = pending.take().unwrap_or_else(|| f.best(data, i));
        let most = (data.len() - i).min(MAX_MATCH);
        let rep_len = if i > rep0 as usize { agree(data, i - rep0 as usize - 1, i, most) } else { 0 };
        // One step of lazy matching: a longer match a byte on wins.
        let mut lazy = None;
        if len >= MIN_MATCH && len < 64 && i + 1 < data.len() {
            f.upto(data, i + 1);
            let next = f.best(data, i + 1);
            if next.0 > len + 1 {
                lazy = Some(next);
            }
        }
        let use_rep = rep_len >= 2 && rep_len + 1 >= len;
        if lazy.is_some() && !use_rep {
            // A literal now, the longer match next.
            e.bit(&mut m.is_match[state], 0);
            let ctx = literal_ctx(if i > 0 { data[i - 1] } else { 0 });
            let matched = (after_match && i > rep0 as usize).then(|| data[i - rep0 as usize - 1]);
            encode_literal(&mut e, &mut m.literal[ctx..ctx + 0x300], data[i], matched);
            state = (state << 1) & 6;
            after_match = false;
            pending = lazy;
            i += 1;
            continue;
        }
        if use_rep {
            e.bit(&mut m.is_match[state], 1);
            e.bit(&mut m.is_rep[state], 1);
            m.reps.encode(&mut e, rep_len, state & 3);
            i += rep_len;
            state = ((state << 1) | 1) & 7;
            after_match = true;
        } else if len >= MIN_MATCH {
            e.bit(&mut m.is_match[state], 1);
            e.bit(&mut m.is_rep[state], 0);
            m.lengths.encode(&mut e, len, state & 3);
            encode_distance(&mut e, &mut m, dist, len);
            rep0 = dist;
            i += len;
            state = ((state << 1) | 1) & 7;
            after_match = true;
        } else {
            e.bit(&mut m.is_match[state], 0);
            let ctx = literal_ctx(if i > 0 { data[i - 1] } else { 0 });
            let matched = (after_match && i > rep0 as usize).then(|| data[i - rep0 as usize - 1]);
            encode_literal(&mut e, &mut m.literal[ctx..ctx + 0x300], data[i], matched);
            state = (state << 1) & 6;
            after_match = false;
            i += 1;
        }
    }
    e.finish()
}

/// Why packed bytes couldn't be unpacked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Corrupt;

impl std::fmt::Display for Corrupt {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "the packed data is damaged")
    }
}

/// The length `packed` unpacks to, from its header.
pub fn unpacked_len(packed: &[u8]) -> Option<u64> {
    (packed.len() >= 13 && &packed[..4] == MAGIC).then(|| u64::from_le_bytes(packed[5..13].try_into().expect("8 bytes")))
}

/// Unpacks what `compress` packed; `most` caps what it may make (a
/// damaged length can't ask for the world).
pub fn decompress(packed: &[u8], most: usize) -> Result<Vec<u8>, Corrupt> {
    let len = unpacked_len(packed).ok_or(Corrupt)?;
    if len > most as u64 {
        return Err(Corrupt);
    }
    let len = len as usize;
    let body = &packed[13..];
    match packed[4] {
        0 => body.get(..len).map(|b| b.to_vec()).ok_or(Corrupt),
        1 => decode(body, len),
        _ => Err(Corrupt),
    }
}

fn decode(body: &[u8], len: usize) -> Result<Vec<u8>, Corrupt> {
    let mut out: Vec<u8> = Vec::with_capacity(len);
    let mut d = Decoder::new(body);
    let mut m = Model::new();
    let mut state = 0usize;
    let mut rep0 = 0usize;
    let mut after_match = false;
    while out.len() < len {
        if d.overrun() {
            return Err(Corrupt);
        }
        if d.bit(&mut m.is_match[state]) == 0 {
            let ctx = literal_ctx(out.last().copied().unwrap_or(0));
            let matched = (after_match && out.len() > rep0).then(|| out[out.len() - rep0 - 1]);
            let b = decode_literal(&mut d, &mut m.literal[ctx..ctx + 0x300], matched);
            out.push(b);
            state = (state << 1) & 6;
            after_match = false;
            continue;
        }
        let n = if d.bit(&mut m.is_rep[state]) == 1 {
            m.reps.decode(&mut d, state & 3)
        } else {
            let n = m.lengths.decode(&mut d, state & 3);
            rep0 = decode_distance(&mut d, &mut m, n) as usize;
            n
        };
        if rep0 >= out.len() || out.len() + n > len {
            return Err(Corrupt);
        }
        let from = out.len() - rep0 - 1;
        for k in 0..n {
            let b = out[from + k];
            out.push(b);
        }
        state = ((state << 1) | 1) & 7;
        after_match = true;
    }
    Ok(out)
}

/// 16-bit PCM, ready to pack: each channel's differences, the low bytes
/// then the high bytes (8-bit PCM: the differences only).
pub fn delta_pack(pcm: &[i16], channels: usize, bits: u8) -> Vec<u8> {
    let ch = channels.max(1);
    let mut prev = vec![0i16; ch];
    if bits <= 8 {
        return pcm
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let v8 = (v >> 8) as i8;
                let d = v8.wrapping_sub(prev[i % ch] as i8);
                prev[i % ch] = i16::from(v8);
                d as u8
            })
            .collect();
    }
    let mut lo = Vec::with_capacity(pcm.len());
    let mut hi = Vec::with_capacity(pcm.len());
    for (i, &v) in pcm.iter().enumerate() {
        let d = v.wrapping_sub(prev[i % ch]) as u16;
        prev[i % ch] = v;
        lo.push(d as u8);
        hi.push((d >> 8) as u8);
    }
    lo.extend_from_slice(&hi);
    lo
}

pub fn delta_unpack(b: &[u8], channels: usize, bits: u8) -> Vec<i16> {
    let ch = channels.max(1);
    let mut prev = vec![0i16; ch];
    if bits <= 8 {
        return b
            .iter()
            .enumerate()
            .map(|(i, &d)| {
                let v = (prev[i % ch] as i8).wrapping_add(d as i8);
                prev[i % ch] = i16::from(v);
                i16::from(v) << 8
            })
            .collect();
    }
    let n = b.len() / 2;
    (0..n)
        .map(|i| {
            let d = u16::from(b[i]) | u16::from(b[n + i]) << 8;
            let v = prev[i % ch].wrapping_add(d as i16);
            prev[i % ch] = v;
            v
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(data: &[u8]) -> usize {
        let p = compress(data);
        assert_eq!(decompress(&p, data.len()).unwrap(), data);
        p.len()
    }

    #[test]
    fn round_trips() {
        round(b"");
        round(b"a");
        round(b"abcabcabcabcabcabcabcabcabcabcabc");
        let text = "the quick brown fox jumps over the lazy dog. ".repeat(200);
        assert!(round(text.as_bytes()) < text.len() / 10);
        let mut x = 12345u32;
        let noise: Vec<u8> = (0..20000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        assert!(round(&noise) <= noise.len() + 13);
        let wave: Vec<i16> = (0..50_000).map(|i| ((i as f32 * 0.05).sin() * 12000.0) as i16).collect();
        let packed = delta_pack(&wave, 1, 16);
        let size = round(&packed);
        assert!(size < wave.len(), "{size}");
        assert_eq!(delta_unpack(&packed, 1, 16), wave);
        let wave8: Vec<i16> = wave.iter().map(|v| v & !0xFF).collect();
        assert_eq!(delta_unpack(&delta_pack(&wave8, 2, 8), 2, 8), wave8);
    }

    #[test]
    fn damage_is_an_error_never_a_panic() {
        let text = "tracker samples ".repeat(500);
        let p = compress(text.as_bytes());
        for cut in [13, 20, p.len() / 2] {
            let _ = decompress(&p[..cut], 1 << 20);
        }
        let mut bad = p.clone();
        for i in (14..bad.len()).step_by(7) {
            bad[i] ^= 0x5A;
        }
        let _ = decompress(&bad, 1 << 20);
        assert!(decompress(&p, 10).is_err());
    }
}
