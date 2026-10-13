// Copyright 2026 ansiapps. Neumetik Tracker: MIT licensed, see the LICENSE at the repository root.

//! Ogg Vorbis decoded, for the formats that keep their samples so (MO3,
//! Skale Tracker). Written from the Vorbis I specification (Xiph.Org)
//! and the Ogg bitstream's (RFC 3533): the pages joined into packets,
//! the three headers read (identification, comments passed over, setup:
//! codebooks, floors 0 and 1, residues 0 to 2, mappings and modes), then
//! each audio packet: floors, residues, inverse coupling, the inverse
//! MDCT (a DCT-IV made of a complex FFT a quarter of the block's size),
//! windowed and overlapped.
//!
//! Damaged packets are passed over (they decode to silence); a stream
//! cut short gives what came before the cut.

/// A stream decoded: its channels, rate and points (interleaved when
/// there are two or more channels).
pub struct Decoded {
    pub channels: u8,
    pub rate: u32,
    pub pcm: Vec<i16>,
}

/// Decodes an Ogg Vorbis stream: the first logical stream's packets,
/// whatever its serial number (MO3's shared headers are another
/// stream's). None when it isn't Vorbis or its headers are damaged.
pub fn decode_ogg(data: &[u8]) -> Option<Decoded> {
    let (packets, last_granule) = ogg_packets(data);
    let mut it = packets.into_iter();
    let mut dec = Decoder::new(&it.next()?, &it.next()?, &it.next()?)?;
    let mut pcm = Vec::new();
    for p in it {
        dec.decode(&p, &mut pcm);
    }
    let ch = usize::from(dec.channels);
    // The last page's granule position is the stream's length.
    if let Some(g) = last_granule {
        let frames = usize::try_from(g).unwrap_or(usize::MAX);
        if frames.saturating_mul(ch) < pcm.len() {
            pcm.truncate(frames * ch);
        }
    }
    Some(Decoded { channels: dec.channels, rate: dec.rate, pcm })
}

/// The packets of the Ogg pages in `d` (pages found by their capture
/// pattern, so junk between them is passed over), and the last page's
/// granule position.
pub fn ogg_packets(d: &[u8]) -> (Vec<Vec<u8>>, Option<u64>) {
    let mut packets = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut granule = None;
    let mut at = 0;
    while at + 27 <= d.len() {
        if &d[at..at + 4] != b"OggS" {
            at += 1;
            continue;
        }
        let segs = usize::from(d[at + 26]);
        let body = at + 27 + segs;
        if body > d.len() {
            break;
        }
        let lacing = &d[at + 27..body];
        let total: usize = lacing.iter().map(|&l| usize::from(l)).sum();
        // A continued packet whose start was lost is dropped.
        if d[at + 5] & 1 == 0 && !cur.is_empty() {
            packets.push(std::mem::take(&mut cur));
        }
        let g = u64::from_le_bytes(d[at + 6..at + 14].try_into().unwrap());
        if g != u64::MAX {
            granule = Some(g);
        }
        let mut p = body;
        for &l in lacing {
            let end = (p + usize::from(l)).min(d.len());
            cur.extend_from_slice(&d[p..end]);
            p = end;
            if l < 255 {
                packets.push(std::mem::take(&mut cur));
            }
        }
        at = body + total;
    }
    if !cur.is_empty() {
        packets.push(cur);
    }
    (packets, granule)
}

// ---- Bits ----

/// Bits read least significant first, as Vorbis packs them. Reading past
/// the end gives `None` (the packet's end, which the floors and residues
/// treat as the spec says).
struct Bits<'a> {
    d: &'a [u8],
    bit: usize,
}

impl<'a> Bits<'a> {
    fn new(d: &'a [u8]) -> Bits<'a> {
        Bits { d, bit: 0 }
    }

    fn read(&mut self, n: u32) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        if self.bit + n as usize > self.d.len() * 8 {
            self.bit = self.d.len() * 8 + 1;
            return None;
        }
        let mut v: u64 = 0;
        let mut got = 0u32;
        while got < n {
            let byte = self.d[self.bit >> 3] as u64;
            let off = (self.bit & 7) as u32;
            let take = (8 - off).min(n - got);
            v |= ((byte >> off) & ((1 << take) - 1)) << got;
            got += take;
            self.bit += take as usize;
        }
        Some(v as u32)
    }

    fn flag(&mut self) -> Option<bool> {
        self.read(1).map(|b| b == 1)
    }

    /// Up to 32 bits ahead without moving, zeros past the end; and how
    /// many of them are really there.
    fn peek32(&self) -> (u32, usize) {
        let mut v: u64 = 0;
        let byte = self.bit >> 3;
        for i in 0..5 {
            if let Some(&b) = self.d.get(byte + i) {
                v |= (b as u64) << (8 * i);
            }
        }
        let have = (self.d.len() * 8).saturating_sub(self.bit).min(32);
        ((v >> (self.bit & 7)) as u32, have)
    }

    fn eaten(&mut self, n: usize) {
        self.bit += n;
    }
}

fn ilog(v: u32) -> u32 {
    32 - v.leading_zeros()
}

fn float32_unpack(x: u32) -> f32 {
    let mantissa = (x & 0x1f_ffff) as f64;
    let exponent = ((x & 0x7fe0_0000) >> 21) as i32;
    let v = mantissa * 2f64.powi(exponent - 788);
    (if x & 0x8000_0000 != 0 { -v } else { v }) as f32
}

/// The largest r with r^dims at most `entries`.
fn lookup1_values(entries: u32, dims: u32) -> u32 {
    let mut r = (entries as f64).powf(1.0 / dims as f64).floor() as u32;
    let pow = |r: u32| (0..dims).try_fold(1u64, |a, _| a.checked_mul(r as u64)).unwrap_or(u64::MAX);
    while pow(r + 1) <= entries as u64 {
        r += 1;
    }
    while r > 0 && pow(r) > entries as u64 {
        r -= 1;
    }
    r
}

// ---- Codebooks ----

const FAST: u32 = 10;

struct Codebook {
    dims: usize,
    /// Its codewords as a tree: each node's two children, a leaf as
    /// `!entry`.
    tree: Vec<[i32; 2]>,
    /// The next FAST bits looked up at once: (entry, length), length 0
    /// when the codeword is longer.
    fast: Vec<(u32, u8)>,
    /// The entries' vectors, `dims` each, when it has them.
    vq: Vec<f32>,
}

impl Codebook {
    fn read(b: &mut Bits) -> Option<Codebook> {
        if b.read(24)? != 0x56_4342 {
            return None;
        }
        let dims = b.read(16)? as usize;
        let entries = b.read(24)? as usize;
        if dims == 0 && entries > 0 {
            return None;
        }
        let mut lengths = vec![0u8; entries];
        if !b.flag()? {
            let sparse = b.flag()?;
            for l in lengths.iter_mut() {
                if !sparse || b.flag()? {
                    *l = b.read(5)? as u8 + 1;
                }
            }
        } else {
            let mut at = 0usize;
            let mut len = b.read(5)? + 1;
            while at < entries {
                let n = b.read(ilog((entries - at) as u32))? as usize;
                if at + n > entries || len > 32 {
                    return None;
                }
                lengths[at..at + n].fill(len as u8);
                at += n;
                len += 1;
            }
        }
        let lookup = b.read(4)?;
        let mut vq = Vec::new();
        match lookup {
            0 => {}
            1 | 2 => {
                let min = float32_unpack(b.read(32)?);
                let delta = float32_unpack(b.read(32)?);
                let bits = b.read(4)? + 1;
                let seq = b.flag()?;
                let values = if lookup == 1 { lookup1_values(entries as u32, dims as u32) as usize } else { entries.checked_mul(dims)? };
                if values > 1 << 24 || entries.checked_mul(dims)? > 1 << 24 {
                    return None;
                }
                let mut mult = Vec::with_capacity(values);
                for _ in 0..values {
                    mult.push(b.read(bits)? as f32);
                }
                vq = vec![0f32; entries * dims];
                for e in 0..entries {
                    let mut last = 0f32;
                    let mut div = 1usize;
                    for i in 0..dims {
                        let off = if lookup == 1 { (e / div) % values.max(1) } else { e * dims + i };
                        let v = mult.get(off).copied().unwrap_or(0.0) * delta + min + last;
                        if seq {
                            last = v;
                        }
                        vq[e * dims + i] = v;
                        if lookup == 1 {
                            div = div.saturating_mul(values);
                        }
                    }
                }
            }
            _ => return None,
        }
        let (tree, fast) = build_tree(&lengths)?;
        Some(Codebook { dims, tree, fast, vq })
    }

    /// The next entry, or None at the packet's end (or a codeword not in
    /// the book).
    fn decode(&self, b: &mut Bits) -> Option<u32> {
        let (bits, have) = b.peek32();
        let (e, len) = self.fast[(bits & ((1 << FAST) - 1)) as usize];
        if len > 0 && usize::from(len) <= have {
            b.eaten(usize::from(len));
            return Some(e);
        }
        let mut node = 0usize;
        loop {
            let bit = b.read(1)? as usize;
            let next = *self.tree.get(node)?.get(bit)?;
            if next < 0 {
                return Some(!next as u32);
            }
            if next == 0 {
                return None;
            }
            node = next as usize;
        }
    }

    fn vector(&self, b: &mut Bits) -> Option<&[f32]> {
        let e = self.decode(b)? as usize;
        self.vq.get(e * self.dims..(e + 1) * self.dims)
    }
}

/// The codewords the spec assigns to `lengths` (each the lowest free one
/// of its length, in entry order), as a tree and a lookup table.
fn build_tree(lengths: &[u8]) -> Option<(Vec<[i32; 2]>, Vec<(u32, u8)>)> {
    let mut tree = vec![[0i32; 2]];
    let mut fast = vec![(0u32, 0u8); 1 << FAST];
    let used: Vec<usize> = (0..lengths.len()).filter(|&i| lengths[i] > 0).collect();
    // One entry: its codeword is all zeros (and a single bit when longer
    // ones would be wasted).
    if used.len() == 1 {
        let e = used[0];
        tree[0] = [!(e as i32), !(e as i32)];
        for f in fast.iter_mut() {
            *f = (e as u32, 1);
        }
        return Some((tree, fast));
    }
    let mut marker = [0u32; 33];
    for &e in &used {
        let len = usize::from(lengths[e]);
        let mut entry = marker[len];
        if len < 32 && entry >> len != 0 {
            return None;
        }
        let code = entry;
        for j in (1..=len).rev() {
            if marker[j] & 1 != 0 {
                if j == 1 {
                    marker[1] += 1;
                } else {
                    marker[j] = marker[j - 1] << 1;
                }
                break;
            }
            marker[j] += 1;
        }
        for j in len + 1..33 {
            if marker[j] >> 1 == entry {
                entry = marker[j];
                marker[j] = marker[j - 1] << 1;
            } else {
                break;
            }
        }
        // Into the tree, its first bit the codeword's top one.
        let mut node = 0usize;
        for i in (0..len).rev() {
            let bit = ((code >> i) & 1) as usize;
            if i == 0 {
                tree[node][bit] = !(e as i32);
            } else {
                let next = tree[node][bit];
                if next < 0 {
                    return None;
                }
                if next == 0 {
                    tree.push([0, 0]);
                    let n = (tree.len() - 1) as i32;
                    tree[node][bit] = n;
                    node = n as usize;
                } else {
                    node = next as usize;
                }
            }
        }
        if len as u32 <= FAST {
            // The stream's bits come in the codeword's order, top first.
            let mut rev = 0u32;
            for i in 0..len {
                rev |= ((code >> (len - 1 - i)) & 1) << i;
            }
            let mut i = rev as usize;
            while i < 1 << FAST {
                fast[i] = (e as u32, len as u8);
                i += 1 << len;
            }
        }
    }
    Some((tree, fast))
}

// ---- Floors ----

enum Floor {
    Zero { order: usize, rate: u32, bark_map_size: u32, amplitude_bits: u32, amplitude_offset: u32, books: Vec<usize> },
    One(Floor1),
}

struct Floor1 {
    partition_class: Vec<usize>,
    class_dims: Vec<usize>,
    class_subclasses: Vec<u32>,
    class_masterbook: Vec<usize>,
    subclass_books: Vec<Vec<i32>>,
    multiplier: u32,
    xs: Vec<u32>,
    /// The points in order of their x: (index, low neighbour, high neighbour) for each from 2.
    neighbours: Vec<(usize, usize)>,
    sorted: Vec<usize>,
}

fn read_floor(b: &mut Bits, books: usize) -> Option<Floor> {
    match b.read(16)? {
        0 => {
            let order = b.read(8)? as usize;
            let rate = b.read(16)?;
            let bark_map_size = b.read(16)?;
            let amplitude_bits = b.read(6)?;
            let amplitude_offset = b.read(8)?;
            let n = b.read(4)? as usize + 1;
            let mut list = Vec::with_capacity(n);
            for _ in 0..n {
                let book = b.read(8)? as usize;
                if book >= books {
                    return None;
                }
                list.push(book);
            }
            if order == 0 || bark_map_size == 0 {
                return None;
            }
            Some(Floor::Zero { order, rate, bark_map_size, amplitude_bits, amplitude_offset, books: list })
        }
        1 => {
            let partitions = b.read(5)? as usize;
            let mut partition_class = Vec::with_capacity(partitions);
            for _ in 0..partitions {
                partition_class.push(b.read(4)? as usize);
            }
            let classes = partition_class.iter().max().map_or(0, |&m| m + 1);
            let mut class_dims = Vec::new();
            let mut class_subclasses = Vec::new();
            let mut class_masterbook = Vec::new();
            let mut subclass_books = Vec::new();
            for _ in 0..classes {
                class_dims.push(b.read(3)? as usize + 1);
                let sub = b.read(2)?;
                class_subclasses.push(sub);
                class_masterbook.push(if sub != 0 { b.read(8)? as usize } else { 0 });
                if sub != 0 && *class_masterbook.last()? >= books {
                    return None;
                }
                let mut list = Vec::new();
                for _ in 0..1 << sub {
                    let book = b.read(8)? as i32 - 1;
                    if book >= books as i32 {
                        return None;
                    }
                    list.push(book);
                }
                subclass_books.push(list);
            }
            let multiplier = b.read(2)? + 1;
            let rangebits = b.read(4)?;
            let mut xs = vec![0, 1 << rangebits];
            for &c in &partition_class {
                for _ in 0..class_dims[c] {
                    xs.push(b.read(rangebits)?);
                }
            }
            if xs.len() > 65 {
                return None;
            }
            let mut neighbours = vec![(0, 0); xs.len()];
            for i in 2..xs.len() {
                let (mut lo, mut hi) = (0usize, 1usize);
                for j in 0..i {
                    if xs[j] < xs[i] && xs[j] > xs[lo] {
                        lo = j;
                    }
                    if xs[j] > xs[i] && xs[j] < xs[hi] {
                        hi = j;
                    }
                }
                neighbours[i] = (lo, hi);
            }
            let mut sorted: Vec<usize> = (0..xs.len()).collect();
            sorted.sort_by_key(|&i| xs[i]);
            Some(Floor::One(Floor1 { partition_class, class_dims, class_subclasses, class_masterbook, subclass_books, multiplier, xs, neighbours, sorted }))
        }
        _ => None,
    }
}

/// A floor read from a packet, ready to be drawn when the residue's in.
enum FloorData {
    Unused,
    Zero { amplitude: u32, coefficients: Vec<f32> },
    One(Vec<i32>),
}

fn decode_floor(f: &Floor, books: &[Codebook], b: &mut Bits) -> FloorData {
    match f {
        Floor::Zero { order, amplitude_bits, books: list, .. } => {
            let Some(amplitude) = b.read(*amplitude_bits) else { return FloorData::Unused };
            if amplitude == 0 {
                return FloorData::Unused;
            }
            let Some(n) = b.read(ilog(list.len() as u32)) else { return FloorData::Unused };
            let Some(&book) = list.get(n as usize) else { return FloorData::Unused };
            let cb = &books[book];
            if cb.vq.is_empty() {
                return FloorData::Unused;
            }
            let mut coefficients = Vec::with_capacity(*order + cb.dims);
            let mut last = 0f32;
            while coefficients.len() < *order {
                let Some(v) = cb.vector(b) else { return FloorData::Unused };
                for &x in v {
                    coefficients.push(x + last);
                }
                last = *coefficients.last().unwrap_or(&0.0);
            }
            coefficients.truncate(*order);
            FloorData::Zero { amplitude, coefficients }
        }
        Floor::One(f) => {
            let read = |b: &mut Bits| -> Option<Vec<i32>> {
                if !b.flag()? {
                    return None;
                }
                let range = [256, 128, 86, 64][(f.multiplier - 1) as usize];
                let bits = ilog(range - 1);
                let mut ys = vec![b.read(bits)? as i32, b.read(bits)? as i32];
                for &c in &f.partition_class {
                    let cbits = f.class_subclasses[c];
                    let csub = (1 << cbits) - 1;
                    let mut cval = if cbits > 0 { books[f.class_masterbook[c]].decode(b)? } else { 0 };
                    for _ in 0..f.class_dims[c] {
                        let book = f.subclass_books[c][(cval & csub) as usize];
                        cval >>= cbits;
                        ys.push(if book >= 0 { books[book as usize].decode(b)? as i32 } else { 0 });
                    }
                }
                Some(ys)
            };
            match read(b) {
                Some(ys) => FloorData::One(ys),
                None => FloorData::Unused,
            }
        }
    }
}

fn render_point(x0: i32, y0: i32, x1: i32, y1: i32, x: i32) -> i32 {
    let dy = y1 - y0;
    let adx = x1 - x0;
    let err = dy.abs() * (x - x0);
    let off = if adx != 0 { err / adx } else { 0 };
    if dy < 0 {
        y0 - off
    } else {
        y0 + off
    }
}

fn render_line(x0: i32, y0: i32, x1: i32, y1: i32, out: &mut [f32]) {
    let n = out.len() as i32;
    let dy = y1 - y0;
    let adx = x1 - x0;
    if adx <= 0 {
        return;
    }
    let base = dy / adx;
    let sy = if dy < 0 { base - 1 } else { base + 1 };
    let ady = dy.abs() - base.abs() * adx;
    let mut y = y0;
    let mut err = 0;
    if x0 < n && x0 >= 0 {
        out[x0 as usize] = db(y);
    }
    for x in x0 + 1..x1.min(n) {
        err += ady;
        if err >= adx {
            err -= adx;
            y += sy;
        } else {
            y += base;
        }
        if x >= 0 {
            out[x as usize] = db(y);
        }
    }
}

/// Floor 1's amplitudes: 140 dB in 256 steps.
fn db(y: i32) -> f32 {
    DB.with(|t| t[y.clamp(0, 255) as usize])
}

thread_local! {
    static DB: [f32; 256] = {
        let mut t = [0f32; 256];
        for (i, v) in t.iter_mut().enumerate() {
            *v = 10f64.powf((i as f64 - 255.0) * 0.546875 / 20.0) as f32;
        }
        t
    };
}

/// Draws a floor into `out` (half the block): false when it's unused.
fn draw_floor(f: &Floor, data: &FloorData, out: &mut [f32]) -> bool {
    let n = out.len();
    match (f, data) {
        (_, FloorData::Unused) => false,
        (Floor::Zero { order, rate, bark_map_size, amplitude_bits, amplitude_offset, .. }, FloorData::Zero { amplitude, coefficients }) => {
            let bark = |x: f64| 13.1 * (0.00074 * x).atan() + 2.24 * (0.0000000185 * x * x).atan() + 0.0001 * x;
            let nyq = bark(0.5 * *rate as f64);
            let map: Vec<i64> = (0..n).map(|i| ((bark(*rate as f64 * i as f64 / (2.0 * n as f64)) * *bark_map_size as f64 / nyq).floor() as i64).min(*bark_map_size as i64 - 1)).collect();
            let cos: Vec<f64> = coefficients.iter().map(|&c| (c as f64).cos()).collect();
            let max_amp = ((1u64 << amplitude_bits) - 1) as f64;
            let mut i = 0;
            while i < n {
                let w = std::f64::consts::PI * map[i] as f64 / *bark_map_size as f64;
                let cw = w.cos();
                let (mut p, mut q);
                if order % 2 == 1 {
                    p = 1.0 - cw * cw;
                    q = 0.25;
                    for j in 0..(order - 1) / 2 {
                        q *= (cos[2 * j] - cw).powi(2) * 4.0;
                    }
                    for j in 0..(order - 1) / 2 + 1 {
                        p *= if 2 * j + 1 < cos.len() { (cos[2 * j + 1] - cw).powi(2) * 4.0 } else { 1.0 };
                    }
                    // The odd order's last even coefficient joins q.
                    if order - 1 < cos.len() && (order - 1) % 2 == 0 {
                        q *= (cos[order - 1] - cw).powi(2) * 4.0;
                    }
                } else {
                    p = (1.0 - cw) / 2.0;
                    q = (1.0 + cw) / 2.0;
                    for j in 0..order / 2 {
                        p *= (cos[2 * j + 1] - cw).powi(2) * 4.0;
                        q *= (cos[2 * j] - cw).powi(2) * 4.0;
                    }
                }
                let l = (0.11512925 * (*amplitude as f64 * *amplitude_offset as f64 / (max_amp * (p + q).sqrt()) - *amplitude_offset as f64)).exp();
                let m = map[i];
                while i < n && map[i] == m {
                    out[i] = l as f32;
                    i += 1;
                }
            }
            true
        }
        (Floor::One(f), FloorData::One(ys)) => {
            let range = [256, 128, 86, 64][(f.multiplier - 1) as usize];
            let count = f.xs.len().min(ys.len());
            let mut fy = vec![0i32; count];
            let mut step2 = vec![false; count];
            fy[0] = ys[0];
            fy[1] = ys[1];
            step2[0] = true;
            step2[1] = true;
            for i in 2..count {
                let (lo, hi) = f.neighbours[i];
                let predicted = render_point(f.xs[lo] as i32, fy[lo], f.xs[hi] as i32, fy[hi], f.xs[i] as i32);
                let val = ys[i];
                let highroom = range - predicted;
                let lowroom = predicted;
                let room = if highroom < lowroom { highroom * 2 } else { lowroom * 2 };
                if val != 0 {
                    step2[lo] = true;
                    step2[hi] = true;
                    step2[i] = true;
                    fy[i] = if val >= room {
                        if highroom > lowroom {
                            val - lowroom + predicted
                        } else {
                            predicted - val + highroom - 1
                        }
                    } else if val % 2 == 1 {
                        predicted - (val + 1) / 2
                    } else {
                        predicted + val / 2
                    };
                } else {
                    fy[i] = predicted;
                }
            }
            let mult = f.multiplier as i32;
            let (mut lx, mut ly) = (0i32, fy[f.sorted[0]] * mult);
            let mut hx = 0i32;
            let mut hy = ly;
            for &i in &f.sorted[1..] {
                if i < count && step2[i] {
                    hy = fy[i] * mult;
                    hx = f.xs[i] as i32;
                    render_line(lx, ly, hx, hy, out);
                    lx = hx;
                    ly = hy;
                }
            }
            if (hx as usize) < n {
                render_line(hx, hy, n as i32, hy, out);
            }
            true
        }
        _ => false,
    }
}

// ---- Residues ----

struct Residue {
    kind: u16,
    begin: usize,
    end: usize,
    partition_size: usize,
    classifications: usize,
    classbook: usize,
    books: Vec<[i32; 8]>,
}

fn read_residue(b: &mut Bits, books: &[Codebook]) -> Option<Residue> {
    let kind = b.read(16)? as u16;
    if kind > 2 {
        return None;
    }
    let begin = b.read(24)? as usize;
    let end = b.read(24)? as usize;
    let partition_size = b.read(24)? as usize + 1;
    let classifications = b.read(6)? as usize + 1;
    let classbook = b.read(8)? as usize;
    if classbook >= books.len() {
        return None;
    }
    let mut cascade = Vec::with_capacity(classifications);
    for _ in 0..classifications {
        let low = b.read(3)?;
        let high = if b.flag()? { b.read(5)? } else { 0 };
        cascade.push(high << 3 | low);
    }
    let mut list = Vec::with_capacity(classifications);
    for c in cascade {
        let mut row = [-1i32; 8];
        for (j, r) in row.iter_mut().enumerate() {
            if c & (1 << j) != 0 {
                let book = b.read(8)? as usize;
                if book >= books.len() || books[book].vq.is_empty() {
                    return None;
                }
                *r = book as i32;
            }
        }
        list.push(row);
    }
    Some(Residue { kind, begin, end, partition_size, classifications, classbook, books: list })
}

/// Decodes a residue into `vectors` (each `n` long), those with `skip`
/// set left alone.
fn decode_residue(r: &Residue, books: &[Codebook], b: &mut Bits, vectors: &mut [Vec<f32>], skip: &[bool], n: usize) {
    let ch = vectors.len();
    if r.kind == 2 {
        if skip.iter().all(|&s| s) {
            return;
        }
        let mut big = vec![vec![0f32; n * ch]];
        decode_partitions(r, books, b, &mut big, &[false], n * ch, 1);
        for (i, &v) in big[0].iter().enumerate() {
            vectors[i % ch][i / ch] += v;
        }
        return;
    }
    decode_partitions(r, books, b, vectors, skip, n, r.kind);
}

fn decode_partitions(r: &Residue, books: &[Codebook], b: &mut Bits, vectors: &mut [Vec<f32>], skip: &[bool], size: usize, kind: u16) {
    let begin = r.begin.min(size);
    let end = r.end.min(size);
    let psize = r.partition_size;
    let parts = (end.saturating_sub(begin)) / psize;
    if parts == 0 {
        return;
    }
    let classbook = &books[r.classbook];
    let per_word = classbook.dims.max(1);
    let ch = vectors.len();
    let mut classes = vec![vec![0usize; parts + per_word]; ch];
    for pass in 0..8 {
        let mut p = 0;
        while p < parts {
            if pass == 0 {
                for j in 0..ch {
                    if skip[j] {
                        continue;
                    }
                    let Some(mut temp) = classbook.decode(b) else { return };
                    for i in (0..per_word).rev() {
                        classes[j][i + p] = temp as usize % r.classifications;
                        temp /= r.classifications as u32;
                    }
                }
            }
            let mut i = 0;
            while i < per_word && p < parts {
                for j in 0..ch {
                    if skip[j] {
                        continue;
                    }
                    let book = r.books[classes[j][p]][pass];
                    if book < 0 {
                        continue;
                    }
                    let cb = &books[book as usize];
                    let off = begin + p * psize;
                    let v = &mut vectors[j];
                    if kind == 0 {
                        let step = psize / cb.dims;
                        for s in 0..step {
                            let Some(t) = cb.vector(b) else { return };
                            for (d, &x) in t.iter().enumerate() {
                                if let Some(o) = v.get_mut(off + s + d * step) {
                                    *o += x;
                                }
                            }
                        }
                    } else {
                        let mut k = 0;
                        while k < psize {
                            let Some(t) = cb.vector(b) else { return };
                            for &x in t {
                                if k < psize {
                                    if let Some(o) = v.get_mut(off + k) {
                                        *o += x;
                                    }
                                }
                                k += 1;
                            }
                        }
                    }
                }
                i += 1;
                p += 1;
            }
        }
    }
}

// ---- Mappings, modes ----

struct Mapping {
    coupling: Vec<(usize, usize)>,
    mux: Vec<usize>,
    /// Each submap's (floor, residue).
    submaps: Vec<(usize, usize)>,
}

struct Mode {
    long: bool,
    mapping: usize,
}

// ---- The inverse MDCT ----

/// The inverse MDCT of one block size: `n` points from `n / 2`
/// coefficients, y[i] = Σ X[k] cos(2π/n (i + 1/2 + n/4)(k + 1/2)), the
/// unscaled transform Vorbis uses.
struct Imdct {
    n: usize,
    pre: Vec<(f32, f32)>,
    post: Vec<(f32, f32)>,
    fft: Vec<(f32, f32)>,
    rev: Vec<usize>,
}

impl Imdct {
    fn new(n: usize) -> Imdct {
        let m = n / 2;
        let h = m / 2;
        let tw = |a: f64| ((a.cos()) as f32, (a.sin()) as f32);
        let pre = (0..h).map(|k| tw(-std::f64::consts::PI * (k as f64 + 0.25) / m as f64)).collect();
        let post = (0..h).map(|k| tw(-std::f64::consts::PI * k as f64 / m as f64)).collect();
        let fft = (0..h / 2).map(|k| tw(-2.0 * std::f64::consts::PI * k as f64 / h as f64)).collect();
        let bits = h.trailing_zeros();
        let rev = (0..h).map(|i| if bits == 0 { 0 } else { i.reverse_bits() >> (usize::BITS - bits) }).collect();
        Imdct { n, pre, post, fft, rev }
    }

    fn run(&self, x: &[f32], y: &mut [f32]) {
        let m = self.n / 2;
        let h = m / 2;
        // The DCT-IV of x, by a complex FFT of half its size.
        let mut z = vec![(0f32, 0f32); h];
        for k in 0..h {
            let (a, b) = (x[2 * k], x[m - 1 - 2 * k]);
            let (c, s) = self.pre[k];
            z[self.rev[k]] = (a * c - b * s, a * s + b * c);
        }
        let mut len = 2;
        while len <= h {
            let step = h / len;
            for start in (0..h).step_by(len) {
                for j in 0..len / 2 {
                    let (c, s) = self.fft[j * step];
                    let (ur, ui) = z[start + j];
                    let (vr, vi) = z[start + j + len / 2];
                    let (tr, ti) = (vr * c - vi * s, vr * s + vi * c);
                    z[start + j] = (ur + tr, ui + ti);
                    z[start + j + len / 2] = (ur - tr, ui - ti);
                }
            }
            len <<= 1;
        }
        let mut u = vec![0f32; m];
        for k in 0..h {
            let (r, i) = z[k];
            let (c, s) = self.post[k];
            u[2 * k] = r * c - i * s;
            u[m - 1 - 2 * k] = -(r * s + i * c);
        }
        // Unfolded to the block.
        for i in 0..m / 2 {
            y[i] = u[i + m / 2];
        }
        for i in m / 2..3 * m / 2 {
            y[i] = -u[3 * m / 2 - 1 - i];
        }
        for i in 3 * m / 2..2 * m {
            y[i] = -u[i - 3 * m / 2];
        }
    }
}

/// Vorbis's window slope over `n` points, rising.
fn slope(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let s = ((i as f64 + 0.5) / n as f64 * std::f64::consts::FRAC_PI_2).sin();
            (std::f64::consts::FRAC_PI_2 * s * s).sin() as f32
        })
        .collect()
}

// ---- The decoder ----

pub struct Decoder {
    pub channels: u8,
    pub rate: u32,
    blocksize: [usize; 2],
    books: Vec<Codebook>,
    floors: Vec<Floor>,
    residues: Vec<Residue>,
    mappings: Vec<Mapping>,
    modes: Vec<Mode>,
    imdct: [Imdct; 2],
    slopes: [Vec<f32>; 2],
    /// The last block, windowed, each channel's; empty before the first.
    prev: Vec<Vec<f32>>,
}

impl Decoder {
    /// A decoder from the stream's three header packets.
    pub fn new(ident: &[u8], _comments: &[u8], setup: &[u8]) -> Option<Decoder> {
        if ident.len() < 30 || &ident[..7] != b"\x01vorbis" || setup.len() < 7 || &setup[..7] != b"\x05vorbis" {
            return None;
        }
        let channels = ident[11];
        let rate = u32::from_le_bytes(ident[12..16].try_into().ok()?);
        let b0 = 1usize << (ident[28] & 15);
        let b1 = 1usize << (ident[28] >> 4);
        if channels == 0 || rate == 0 || b0 < 64 || b1 > 8192 || b0 > b1 {
            return None;
        }
        let mut b = Bits::new(&setup[7..]);
        let mut books = Vec::new();
        for _ in 0..b.read(8)? + 1 {
            books.push(Codebook::read(&mut b)?);
        }
        for _ in 0..b.read(6)? + 1 {
            if b.read(16)? != 0 {
                return None;
            }
        }
        let mut floors = Vec::new();
        for _ in 0..b.read(6)? + 1 {
            floors.push(read_floor(&mut b, books.len())?);
        }
        let mut residues = Vec::new();
        for _ in 0..b.read(6)? + 1 {
            residues.push(read_residue(&mut b, &books)?);
        }
        let mut mappings = Vec::new();
        let ch = u32::from(channels);
        for _ in 0..b.read(6)? + 1 {
            if b.read(16)? != 0 {
                return None;
            }
            let submaps = if b.flag()? { b.read(4)? as usize + 1 } else { 1 };
            let mut coupling = Vec::new();
            if b.flag()? {
                for _ in 0..b.read(8)? + 1 {
                    let mag = b.read(ilog(ch - 1))? as usize;
                    let ang = b.read(ilog(ch - 1))? as usize;
                    if mag == ang || mag >= ch as usize || ang >= ch as usize {
                        return None;
                    }
                    coupling.push((mag, ang));
                }
            }
            if b.read(2)? != 0 {
                return None;
            }
            let mut mux = vec![0usize; ch as usize];
            if submaps > 1 {
                for m in mux.iter_mut() {
                    *m = b.read(4)? as usize;
                    if *m >= submaps {
                        return None;
                    }
                }
            }
            let mut maps = Vec::new();
            for _ in 0..submaps {
                b.read(8)?;
                let floor = b.read(8)? as usize;
                let residue = b.read(8)? as usize;
                if floor >= floors.len() || residue >= residues.len() {
                    return None;
                }
                maps.push((floor, residue));
            }
            mappings.push(Mapping { coupling, mux, submaps: maps });
        }
        let mut modes = Vec::new();
        for _ in 0..b.read(6)? + 1 {
            let long = b.flag()?;
            let (w, t) = (b.read(16)?, b.read(16)?);
            let mapping = b.read(8)? as usize;
            if w != 0 || t != 0 || mapping >= mappings.len() {
                return None;
            }
            modes.push(Mode { long, mapping });
        }
        if !b.flag()? {
            return None;
        }
        Some(Decoder {
            channels,
            rate,
            blocksize: [b0, b1],
            books,
            floors,
            residues,
            mappings,
            modes,
            imdct: [Imdct::new(b0), Imdct::new(b1)],
            slopes: [slope(b0 / 2), slope(b1 / 2)],
            prev: Vec::new(),
        })
    }

    /// Decodes an audio packet, adding what it finishes to `out`
    /// (interleaved, 16-bit). A damaged packet adds silence.
    pub fn decode(&mut self, packet: &[u8], out: &mut Vec<i16>) {
        let ch = usize::from(self.channels);
        let mut b = Bits::new(packet);
        if b.read(1) != Some(0) {
            return;
        }
        let Some(mode_no) = b.read(ilog(self.modes.len() as u32 - 1)) else { return };
        let Some(mode) = self.modes.get(mode_no as usize) else { return };
        let long = mode.long;
        let n = self.blocksize[long as usize];
        let (prev_long, next_long) = if long {
            match (b.flag(), b.flag()) {
                (Some(p), Some(q)) => (p, q),
                _ => return,
            }
        } else {
            (false, false)
        };
        let map = &self.mappings[mode.mapping];
        let half = n / 2;
        // Floors.
        let floors: Vec<FloorData> = (0..ch).map(|c| decode_floor(&self.floors[map.submaps[map.mux[c]].0], &self.books, &mut b)).collect();
        let mut unused: Vec<bool> = floors.iter().map(|f| matches!(f, FloorData::Unused)).collect();
        let no_residue = unused.clone();
        for &(m, a) in &map.coupling {
            if !no_residue[m] || !no_residue[a] {
                unused[m] = false;
                unused[a] = false;
            }
        }
        // Residues, a submap at a time.
        let mut vectors = vec![vec![0f32; half]; ch];
        for (s, &(_, residue)) in map.submaps.iter().enumerate() {
            let chans: Vec<usize> = (0..ch).filter(|&c| map.mux[c] == s).collect();
            let mut vs: Vec<Vec<f32>> = chans.iter().map(|_| vec![0f32; half]).collect();
            let skip: Vec<bool> = chans.iter().map(|&c| unused[c]).collect();
            decode_residue(&self.residues[residue], &self.books, &mut b, &mut vs, &skip, half);
            for (v, &c) in vs.into_iter().zip(&chans) {
                vectors[c] = v;
            }
        }
        // Inverse coupling.
        for &(m, a) in map.coupling.iter().rev() {
            for j in 0..half {
                let (mv, av) = (vectors[m][j], vectors[a][j]);
                let (nm, na) = if mv > 0.0 {
                    if av > 0.0 {
                        (mv, mv - av)
                    } else {
                        (mv + av, mv)
                    }
                } else if av > 0.0 {
                    (mv, mv + av)
                } else {
                    (mv - av, mv)
                };
                vectors[m][j] = nm;
                vectors[a][j] = na;
            }
        }
        // The floor curves times the residues, then each channel's block.
        let mut curve = vec![0f32; half];
        let mut blocks = vec![vec![0f32; n]; ch];
        for c in 0..ch {
            let floor = &self.floors[map.submaps[map.mux[c]].0];
            if !draw_floor(floor, &floors[c], &mut curve) {
                continue;
            }
            for (v, &f) in vectors[c].iter_mut().zip(&curve) {
                *v *= f;
            }
            self.imdct[long as usize].run(&vectors[c], &mut blocks[c]);
        }
        // The window.
        let b0 = self.blocksize[0];
        let (ls, ln) = if long && !prev_long { (n / 4 - b0 / 4, b0 / 2) } else { (0, half) };
        let (rs, rn) = if long && !next_long { (n * 3 / 4 - b0 / 4, b0 / 2) } else { (half, half) };
        let lslope = &self.slopes[(ln != b0 / 2) as usize];
        let rslope = &self.slopes[(rn != b0 / 2) as usize];
        for blk in blocks.iter_mut() {
            for (i, s) in blk.iter_mut().enumerate() {
                let w = if i < ls {
                    0.0
                } else if i < ls + ln {
                    lslope[i - ls]
                } else if i < rs {
                    1.0
                } else if i < rs + rn {
                    rslope[rn - 1 - (i - rs)]
                } else {
                    0.0
                };
                *s *= w;
            }
        }
        // Overlapped with the last block: from its centre to this one's.
        if let Some(pn) = self.prev.first().map(|p| p.len()) {
            let count = pn / 4 + n / 4;
            out.reserve(count * ch);
            for t in 0..count {
                for c in 0..ch {
                    let mut v = self.prev[c].get(pn / 2 + t).copied().unwrap_or(0.0);
                    let ci = (t + n / 4) as isize - (pn / 4) as isize;
                    if ci >= 0 {
                        v += blocks[c].get(ci as usize).copied().unwrap_or(0.0);
                    }
                    out.push((v * 32768.0).round().clamp(-32768.0, 32767.0) as i16);
                }
            }
        }
        self.prev = blocks;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imdct_matches_its_definition() {
        for n in [64usize, 256, 2048] {
            let x: Vec<f32> = (0..n / 2).map(|i| ((i * 7919) % 101) as f32 / 50.0 - 1.0).collect();
            let mut y = vec![0f32; n];
            Imdct::new(n).run(&x, &mut y);
            for i in 0..n {
                let want: f64 = (0..n / 2).map(|k| x[k] as f64 * (2.0 * std::f64::consts::PI / n as f64 * (i as f64 + 0.5 + n as f64 / 4.0) * (k as f64 + 0.5)).cos()).sum();
                assert!((want - y[i] as f64).abs() < 1e-2, "n {n} i {i}: {want} vs {}", y[i]);
            }
        }
    }

    #[test]
    fn codewords_are_the_specs() {
        // The spec's example: lengths 2 4 4 4 4 2 3 3.
        let (tree, _) = build_tree(&[2, 4, 4, 4, 4, 2, 3, 3]).unwrap();
        let mut b = Bits::new(&[0b0000_0000]);
        let walk = |bits: &[u8]| {
            let mut node = 0usize;
            for &bit in bits {
                let next = tree[node][bit as usize];
                if next < 0 {
                    return !next;
                }
                node = next as usize;
            }
            -1
        };
        assert_eq!(walk(&[0, 0]), 0);
        assert_eq!(walk(&[0, 1, 0, 0]), 1);
        assert_eq!(walk(&[0, 1, 1, 1]), 4);
        assert_eq!(walk(&[1, 0]), 5);
        assert_eq!(walk(&[1, 1, 0]), 6);
        assert_eq!(walk(&[1, 1, 1]), 7);
        let _ = b.read(1);
    }
}
