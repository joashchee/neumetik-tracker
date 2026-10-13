//! Packed ProTracker modules: the MODs that games and demos shipped
//! rearranged (and shrunk) by a ripper's or a demo group's packer, their
//! patterns stored as tracks, deltas or references, their samples raw.
//! Each is turned back into the ProTracker MOD it was made from, as
//! ProWizard does, and read by the MOD reader.
//!
//! Written from the packers' public descriptions: ProWizard's (Sylvain
//! "Asle" Chipaux) as libxmp keeps them, and NostalgicPlayer's notes on
//! The Player's signatures and packed samples. Each packer is a probe
//! (checks of its header that stand in for a mark, most having none) and
//! an unpacker; they're tried in the order ProWizard tries them, the
//! marked ones first.

use super::protracker;
use crate::io::{Error, Result};
use crate::model::Module;

/// A packer: its short name (the Amiga prefix its files carry), its name
/// in words, and how it's found and unpacked.
pub struct Packer {
    pub id: &'static str,
    pub name: &'static str,
    probe: fn(&[u8]) -> Option<()>,
    unpack: fn(&[u8]) -> Option<Pt>,
}

/// The packers, in the order they're tried: the ones with a mark of their
/// own first, then the ones found by their headers alone.
pub const PACKERS: &[Packer] = &[
    Packer { id: "ac1d", name: "AC1D Packer", probe: probe_ac1d, unpack: unpack_ac1d },
    Packer { id: "fuchs", name: "Fuchs Tracker", probe: probe_fuchs, unpack: unpack_fuchs },
    Packer { id: "fcm", name: "FC-M Packer", probe: probe_fcm, unpack: unpack_fcm },
    Packer { id: "fuz", name: "Fuzzac Packer", probe: probe_fuzzac, unpack: unpack_fuzzac },
    Packer { id: "hrt", name: "Hornet Packer", probe: probe_hrt, unpack: unpack_hrt },
    Packer { id: "ksm", name: "Kefrens Sound Machine", probe: probe_ksm, unpack: unpack_ksm },
    Packer { id: "mp", name: "Module Protector", probe: probe_mp_id, unpack: unpack_mp },
    Packer { id: "ntp", name: "NovoTrade Packer", probe: probe_ntp, unpack: unpack_ntp },
    Packer { id: "pm18a", name: "Promizer 1.8a", probe: probe_pm18a, unpack: unpack_pm18a },
    Packer { id: "pm10c", name: "Promizer 1.0c", probe: probe_pm10c, unpack: unpack_pm10c },
    Packer { id: "pru1", name: "ProRunner 1", probe: probe_pru1, unpack: unpack_pru1 },
    Packer { id: "pru2", name: "ProRunner 2", probe: probe_pru2, unpack: unpack_pru2 },
    Packer { id: "pha", name: "Pha Packer", probe: probe_pha, unpack: unpack_pha },
    Packer { id: "wn", name: "Wanton Packer", probe: probe_wn, unpack: unpack_wn },
    Packer { id: "unic", name: "Unic Tracker", probe: probe_unic_id, unpack: unpack_unic },
    Packer { id: "tp3", name: "Tracker Packer 3", probe: probe_tp3, unpack: unpack_tp3 },
    Packer { id: "tp2", name: "Tracker Packer 2", probe: probe_tp2, unpack: unpack_tp2 },
    Packer { id: "tp1", name: "Tracker Packer 1", probe: probe_tp1, unpack: unpack_tp1 },
    Packer { id: "skyt", name: "SKYT Packer", probe: probe_skyt, unpack: unpack_skyt },
    Packer { id: "stim", name: "STIM (Slam Tilt)", probe: probe_stim, unpack: unpack_stim },
    // No marks: their headers' checks stand in for one.
    Packer { id: "xann", name: "Xann Packer", probe: probe_xann, unpack: unpack_xann },
    Packer { id: "di", name: "Digital Illusions", probe: probe_di, unpack: unpack_di },
    Packer { id: "eu", name: "Eureka Packer", probe: probe_eureka, unpack: unpack_eureka },
    Packer { id: "p4x", name: "The Player 4", probe: probe_p4x, unpack: unpack_p4x },
    Packer { id: "pp21", name: "ProPacker 2.1", probe: probe_pp21, unpack: unpack_pp21 },
    Packer { id: "pp30", name: "ProPacker 3.0", probe: probe_pp30, unpack: unpack_pp30 },
    Packer { id: "pp10", name: "ProPacker 1.0", probe: probe_pp10, unpack: unpack_pp10 },
    Packer { id: "p50a", name: "The Player 5.0A", probe: probe_p50a, unpack: unpack_p50a },
    Packer { id: "p60a", name: "The Player 6.0A", probe: probe_p60a, unpack: unpack_p60a },
    Packer { id: "p61a", name: "The Player 6.1A", probe: probe_p61a, unpack: unpack_p61a },
    Packer { id: "mp", name: "Module Protector", probe: probe_mp_noid, unpack: unpack_mp },
    Packer { id: "nru", name: "NoiseRunner", probe: probe_nru, unpack: unpack_nru },
    Packer { id: "np2", name: "NoisePacker 2", probe: probe_np2, unpack: unpack_np2 },
    Packer { id: "np1", name: "NoisePacker 1", probe: probe_np1, unpack: unpack_np1 },
    Packer { id: "np3", name: "NoisePacker 3", probe: probe_np3, unpack: unpack_np3 },
    Packer { id: "zen", name: "Zen Packer", probe: probe_zen, unpack: unpack_zen },
    Packer { id: "unic", name: "Unic Tracker", probe: probe_unic_emptyid, unpack: unpack_unic },
    Packer { id: "unic", name: "Unic Tracker", probe: probe_unic_noid, unpack: unpack_unic },
    Packer { id: "unic2", name: "Unic Tracker 2", probe: probe_unic2, unpack: unpack_unic2 },
    Packer { id: "crb", name: "Heatseeker mc1.0", probe: probe_crb, unpack: unpack_crb },
    Packer { id: "tdd", name: "The Dark Demon", probe: probe_tdd, unpack: unpack_tdd },
    Packer { id: "stpk", name: "StarTrekker Packer", probe: probe_starpack, unpack: unpack_starpack },
    Packer { id: "tits", name: "Titanics Player", probe: probe_titanics, unpack: unpack_titanics },
];

/// The prefixes (and extensions) packed modules carry, as UADE and
/// Modland name them: for a file picker.
pub const PREFIXES: &[&str] = &[
    "ac1d", "fuchs", "fcm", "fc-m", "fuz", "fuzz", "hrt", "ksm", "mp", "mpro", "ntp", "pm18a", "pm10c", "pm1", "pm10", "pm18", "pru1", "pru2", "pha", "wn", "unic", "tp1", "tp2", "tp3", "skyt",
    "xann", "xan", "di", "eu", "p40a", "p40b", "p41a", "p4x", "pp10", "pp21", "pp30", "p50a", "p60a", "p61a", "p60", "p61", "p5x", "p6x", "nru", "np1", "np2", "np3", "zen", "unic2", "crb", "tdd",
    "stpk", "star", "tits", "stim",
];

/// The packer a file was packed with, when one's probe takes it.
pub fn find(d: &[u8]) -> Option<&'static Packer> {
    PACKERS.iter().find(|p| (p.probe)(d).is_some())
}

/// The MOD a packed module was made from.
pub fn unpack(d: &[u8], p: &Packer) -> Result<Vec<u8>> {
    (p.unpack)(d).map(|pt| pt.to_mod()).ok_or_else(|| Error::Bad(format!("its {} packing is damaged", p.name)))
}

/// Reads a packed module as the MOD it was made from.
pub fn load(d: &[u8], p: &Packer) -> Result<Module> {
    let mut m = protracker::load(&unpack(d, p)?, false)?;
    m.format = "packed".into();
    m.format_name = p.name.into();
    m.tracker = p.name.into();
    Ok(m)
}

// --- Reading -----------------------------------------------------------

fn ok(c: bool) -> Option<()> {
    c.then_some(())
}

fn b(d: &[u8], i: usize) -> Option<u8> {
    d.get(i).copied()
}

fn w(d: &[u8], i: usize) -> Option<usize> {
    Some(usize::from(u16::from_be_bytes([*d.get(i)?, *d.get(i + 1)?])))
}

fn l(d: &[u8], i: usize) -> Option<usize> {
    Some(u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?) as usize)
}

/// A place in the file, read forwards.
struct Cur<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Cur<'a> {
    fn at(d: &'a [u8], p: usize) -> Self {
        Cur { d, p }
    }
    fn u8(&mut self) -> Option<u8> {
        let v = b(self.d, self.p)?;
        self.p += 1;
        Some(v)
    }
    fn w(&mut self) -> Option<usize> {
        let v = w(self.d, self.p)?;
        self.p += 2;
        Some(v)
    }
    fn l(&mut self) -> Option<usize> {
        let v = l(self.d, self.p)?;
        self.p += 4;
        Some(v)
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let v = self.d.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(v)
    }
}

/// Up to `n` bytes from `at`: what's there of a sample cut short.
fn upto(d: &[u8], at: usize, n: usize) -> &[u8] {
    d.get(at.min(d.len())..at.saturating_add(n).min(d.len())).unwrap_or(&[])
}

// --- The MOD made ------------------------------------------------------

/// ProTracker's periods, C-1 to B-3: note 1 to 36 (0 is none).
const PERIODS: [u16; 36] =
    [856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453, 428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226, 214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113];

/// A note's period as a cell's first two bytes (its sample's high bit
/// apart); note 0 is none. None past B-3.
fn period(note: usize) -> Option<[u8; 2]> {
    match note {
        0 => Some([0, 0]),
        1..=36 => Some(PERIODS[note - 1].to_be_bytes()),
        _ => None,
    }
}

/// A cell: a note (0 none, 1 to 36), a sample (0 to 31), an effect.
fn cell(note: usize, ins: u8, fx: u8, fp: u8) -> Option<[u8; 4]> {
    let [hi, lo] = period(note)?;
    Some([(ins & 0xf0) | hi, lo, (ins << 4) | (fx & 0x0f), fp])
}

/// `cell`, a note past B-3 left out (its period empty).
fn cell_or_none(note: usize, ins: u8, fx: u8, fp: u8) -> [u8; 4] {
    cell(note, ins, fx, fp).unwrap_or([ins & 0xf0, 0, (ins << 4) | (fx & 0x0f), fp])
}

/// The ProTracker module a packer's file is turned back into.
pub struct Pt {
    title: Vec<u8>,
    /// The samples' names, when the packer kept them.
    names: Vec<Vec<u8>>,
    /// Sample headers: length, finetune, volume, loop start, loop length
    /// (lengths in words), as a MOD keeps them.
    heads: Vec<[u8; 8]>,
    len: usize,
    restart: u8,
    orders: [u8; 128],
    patterns: Vec<[u8; 1024]>,
    samples: Vec<u8>,
}

impl Pt {
    fn new() -> Pt {
        Pt { title: Vec::new(), names: Vec::new(), heads: Vec::new(), len: 0, restart: 0x7f, orders: [0; 128], patterns: Vec::new(), samples: Vec::new() }
    }

    /// A sample header, its numbers in words.
    fn head(&mut self, len: usize, finetune: u8, volume: u8, loop_start: usize, loop_len: usize) {
        let [a, b] = (len as u16).to_be_bytes();
        let [c, d] = (loop_start as u16).to_be_bytes();
        let [e, f] = (loop_len as u16).to_be_bytes();
        self.heads.push([a, b, finetune, volume, c, d, e, f]);
    }

    /// The sample names of a MOD's header (31 of 30 bytes from `at`), the
    /// first `n` bytes of each.
    fn names_at(&mut self, d: &[u8], at: usize, n: usize) {
        self.names = (0..31).map(|i| upto(d, at + i * 30, n).to_vec()).collect();
    }

    /// A sample header as a MOD has it.
    fn raw_head(&mut self, h: &[u8]) {
        self.heads.push(h[..8].try_into().unwrap_or([0; 8]));
    }

    /// The sum of the samples' lengths, in bytes.
    fn sample_bytes(&self) -> usize {
        self.heads.iter().map(|h| usize::from(u16::from_be_bytes([h[0], h[1]])) * 2).sum()
    }

    fn put(p: &mut [u8; 1024], row: usize, ch: usize, c: [u8; 4]) {
        if row < 64 && ch < 4 {
            p[row * 16 + ch * 4..][..4].copy_from_slice(&c);
        }
    }

    fn to_mod(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1084 + self.patterns.len() * 1024 + self.samples.len());
        let mut title = self.title.clone();
        title.resize(20, 0);
        out.extend_from_slice(&title);
        for i in 0..31 {
            let mut name = self.names.get(i).cloned().unwrap_or_default();
            name.resize(22, 0);
            out.extend_from_slice(&name);
            out.extend_from_slice(self.heads.get(i).unwrap_or(&[0, 0, 0, 0, 0, 0, 0, 1]));
        }
        // The song as long as the order list allows, every pattern it names there.
        let len = self.len.min(128);
        let mut orders = self.orders;
        for o in &mut orders[len..] {
            if usize::from(*o) >= self.patterns.len() {
                *o = 0;
            }
        }
        out.push(len as u8);
        out.push(self.restart);
        out.extend_from_slice(&orders);
        out.extend_from_slice(b"M.K.");
        let most = orders.iter().map(|&o| usize::from(o) + 1).max().unwrap_or(1);
        for i in 0..most.max(self.patterns.len()) {
            out.extend_from_slice(self.patterns.get(i).unwrap_or(&[0; 1024]));
        }
        out.extend_from_slice(&self.samples);
        out
    }
}

/// Positions numbered as patterns by the first position that has each
/// key (the same tracks, the same address): the orders, and for each
/// pattern the position it was first at.
fn number_by_first<K: PartialEq>(keys: &[K]) -> (Vec<u8>, Vec<usize>) {
    let mut orders = Vec::with_capacity(keys.len());
    let mut first: Vec<usize> = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        match first.iter().position(|&f| keys[f] == *k) {
            Some(n) => orders.push(n as u8),
            None => {
                orders.push(first.len() as u8);
                first.push(i);
            }
        }
    }
    (orders, first)
}

/// ProTracker's slides kept as a signed byte by the packers (5, 6 and A):
/// a negative one is a slide down, a positive one up.
fn signed_slide(fx: u8, fp: u8) -> u8 {
    if matches!(fx, 5 | 6 | 0xa) && fp > 0x7f {
        fp.wrapping_neg() << 4
    } else {
        fp
    }
}

// --- AC1D Packer ---------------------------------------------------------

fn probe_ac1d(d: &[u8]) -> Option<()> {
    ok(d.len() >= 896 && d[2] == 0xac && d[3] == 0x1d && d[0] <= 0x7f)?;
    ok((0..31).all(|i| d[10 + 8 * i] <= 0x0f) && d[768..896].iter().all(|&o| o <= 0x7f))
}

fn unpack_ac1d(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    pt.len = usize::from(d[0]);
    pt.restart = d[1];
    let saddr = l(d, 4)?;
    for i in 0..31 {
        pt.raw_head(d.get(8 + i * 8..16 + i * 8)?);
    }
    let mut paddr = Vec::new();
    for i in 0..128 {
        let a = l(d, 256 + i * 4)?;
        if a == 0 {
            break;
        }
        paddr.push(a);
    }
    ok(!paddr.is_empty())?;
    pt.orders.copy_from_slice(d.get(0x300..0x380)?);
    for &a in &paddr[..paddr.len() - 1] {
        let mut p = [0; 1024];
        let mut c = Cur::at(d, a + 12);
        for ch in 0..4 {
            let mut row = 0;
            while row < 64 {
                let c1 = c.u8()?;
                if c1 & 0x80 != 0 {
                    row += usize::from(c1 & 0x7f);
                    continue;
                }
                let c2 = c.u8()?;
                let ins = ((c1 & 0xc0) >> 2) | (c2 >> 4);
                // Its notes start eleven up; 0x3f is none.
                let note = match c1 & 0x3f {
                    0x3f => None,
                    0 => Some(1),
                    n => Some(usize::from(n.wrapping_sub(0x0b)).max(1)),
                };
                let fx = c2 & 0x0f;
                let fp = if fx == 7 { None } else { Some(c.u8()?) };
                let [hi, lo] = note.and_then(period).unwrap_or([0, 0]);
                Pt::put(&mut p, row, ch, [(ins & 0xf0) | hi, lo, (ins << 4) | fp.map_or(0, |_| fx), fp.unwrap_or(0)]);
                row += 1;
            }
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, saddr, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- Fuchs Tracker -------------------------------------------------------

fn probe_fuchs(d: &[u8]) -> Option<()> {
    ok(d.get(192..196)? == b"SONG")?;
    let total = l(d, 10)?;
    ok(total > 2 && total < 65535 * 16)?;
    let mut sum = 0;
    for i in 0..16 {
        let len = w(d, 14 + i * 2)?;
        ok(b(d, 46 + i * 2)? <= 0x40 && len >= w(d, 78 + i * 2)?)?;
        sum += len;
    }
    ok(sum > 2 && sum <= total && (0..40).all(|i| d.get(113 + i * 2).is_some_and(|&p| p <= 40)))
}

fn unpack_fuchs(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    pt.title = d.get(..10)?.to_vec();
    let mut lens = [0; 16];
    for i in 0..16 {
        let (len, vol, ls) = (w(d, 14 + i * 2)?, w(d, 46 + i * 2)?, w(d, 78 + i * 2)?);
        lens[i] = len;
        let ll = if ls == 0 || len == ls { 1 } else { (len - ls) / 2 };
        pt.head(len / 2, 0, vol as u8, ls / 2, ll);
    }
    pt.len = w(d, 110)? & 0xff;
    for i in 0..40 {
        pt.orders[i] = w(d, 112 + i * 2)? as u8;
    }
    let size = l(d, 196)?;
    ok(size > 0 && size <= 0x20000 && size % 4 == 0)?;
    let raw = d.get(200..200 + size)?;
    for chunk in raw.chunks(1024) {
        let mut p = [0; 1024];
        p[..chunk.len()].copy_from_slice(chunk);
        // Its volumes are decimal.
        for c in p.chunks_mut(4) {
            if c[2] & 0x0f == 0x0c {
                c[3] = 10 * (c[3] >> 4) + (c[3] & 0x0f);
            }
        }
        pt.patterns.push(p);
    }
    let mut at = 200 + size + 4;
    for len in lens {
        pt.samples.extend_from_slice(upto(d, at, len));
        at += len;
    }
    Some(pt)
}

// --- FC-M Packer ---------------------------------------------------------

fn probe_fcm(d: &[u8]) -> Option<()> {
    ok(d.get(..6)? == b"FC-M\x01\x00" && (0..31).all(|j| d.get(37 + 8 * j).is_some_and(|&v| v <= 0x40)))
}

fn unpack_fcm(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    pt.title = d.get(10..30)?.to_vec();
    for i in 0..31 {
        let h = d.get(34 + i * 8..42 + i * 8)?;
        let ll = w(h, 6)?.max(1);
        pt.head(w(h, 0)?, h[2], h[3], w(h, 4)?, ll);
    }
    pt.len = usize::from(b(d, 286)?);
    pt.restart = b(d, 287)?;
    let orders = d.get(292..292 + pt.len)?;
    pt.orders[..orders.len()].copy_from_slice(orders);
    let count = usize::from(orders.iter().copied().max().unwrap_or(0)) + 1;
    let at = 292 + pt.len + 4;
    for i in 0..count {
        pt.patterns.push(d.get(at + i * 1024..at + (i + 1) * 1024)?.try_into().ok()?);
    }
    pt.samples = upto(d, at + count * 1024 + 4, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- Fuzzac Packer -------------------------------------------------------

fn probe_fuzzac(d: &[u8]) -> Option<()> {
    ok(d.get(..4)? == b"M1.0" && b(d, 2114)? != 0)?;
    ok((0..31).all(|i| d[72 + i * 68] <= 0x0f && d[73 + i * 68] <= 0x40 && w(d, 66 + i * 68).is_some_and(|n| n <= 0x8000)))
}

fn unpack_fuzzac(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    for i in 0..31 {
        let h = d.get(6 + i * 68..6 + (i + 1) * 68)?;
        pt.names.push(h[..22].to_vec());
        pt.head(w(h, 60)?, h[66], h[67], w(h, 62)?, w(h, 64)?.max(1));
    }
    pt.len = usize::from(b(d, 2114)?);
    let tracks = usize::from(b(d, 2115)?);
    // Each position's four tracks; positions with the same four share a pattern.
    let mut keys = Vec::with_capacity(pt.len);
    for pos in 0..pt.len {
        let mut k = [0u8; 4];
        for (ch, t) in k.iter_mut().enumerate() {
            *t = b(d, 2118 + ch * pt.len * 4 + pos * 4)?;
        }
        keys.push(k);
    }
    let (orders, first) = number_by_first(&keys);
    pt.orders[..orders.len()].copy_from_slice(&orders);
    let base = 2118 + pt.len * 16;
    for &pos in &first {
        let mut p = [0; 1024];
        for (ch, &t) in keys[pos].iter().enumerate() {
            let track = upto(d, base + usize::from(t) * 256, 256);
            for (row, c) in track.chunks_exact(4).enumerate() {
                Pt::put(&mut p, row, ch, c.try_into().ok()?);
            }
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, base + tracks * 256 + 4, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- Hornet Packer -------------------------------------------------------

fn probe_hrt(d: &[u8]) -> Option<()> {
    ok(d.get(1080..1084)? == b"HRT!" && (0..31).all(|i| d[44 + i * 30] <= 0x0f && d[45 + i * 30] <= 0x40))
}

fn unpack_hrt(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    pt.title = d.get(..20)?.to_vec();
    // The last four bytes of each name hold the sample's address.
    pt.names_at(d, 20, 18);
    for i in 0..31 {
        pt.raw_head(d.get(42 + i * 30..50 + i * 30)?);
    }
    pt.len = usize::from(d[950]);
    pt.restart = d[951];
    pt.orders.copy_from_slice(d.get(952..1080)?);
    let count = usize::from(pt.orders.iter().copied().max().unwrap_or(0)) + 1;
    let mut c = Cur::at(d, 1084);
    for _ in 0..count {
        let mut p = [0; 1024];
        for i in 0..256 {
            let x = c.take(4)?;
            let ins = x[0] / 2;
            let [hi, lo] = if x[1] == 0 { [0, 0] } else { period(usize::from(x[1] / 2)).unwrap_or([0, 0]) };
            p[i * 4..][..4].copy_from_slice(&[(ins & 0xf0) | hi, lo, (ins << 4) | x[2], x[3]]);
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, c.p, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- Kefrens Sound Machine -----------------------------------------------

fn probe_ksm(d: &[u8]) -> Option<()> {
    ok(d.get(..2)? == b"M." && b(d, 15)? == b'a' && (0..15).all(|i| d.get(54 + i * 32).is_some_and(|&v| v <= 0x40)))?;
    let mut most = 0;
    let mut i = 0;
    loop {
        ok(i < 1024)?;
        let t = b(d, 512 + i)?;
        if t == 0xff {
            break;
        }
        most = most.max(usize::from(t));
        i += 1;
    }
    ok(most > 0)?;
    ok((0..=most).all(|t| (0..64).all(|j| d.get(1536 + t * 192 + j * 3).is_some_and(|&n| n <= 0x24))))
}

fn unpack_ksm(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    pt.title = d.get(2..15)?.to_vec();
    let mut total = 0;
    for i in 0..15 {
        let h = d.get(32 + i * 32..64 + i * 32)?;
        let (len, ls) = (w(h, 20)?, w(h, 24)?);
        total += len;
        let ll = if ls == 0 { 1 } else { len.saturating_sub(ls) / 2 };
        pt.head(len / 2, 0, h[22], ls / 2, ll);
    }
    let mut keys = Vec::new();
    let mut most = 0;
    while keys.len() < 128 {
        let k: [u8; 4] = d.get(512 + keys.len() * 4..516 + keys.len() * 4)?.try_into().ok()?;
        if k[0] == 0xff {
            break;
        }
        most = most.max(k.iter().copied().max().unwrap_or(0));
        keys.push(k);
    }
    pt.len = keys.len();
    let (orders, first) = number_by_first(&keys);
    pt.orders[..orders.len()].copy_from_slice(&orders);
    for &pos in &first {
        let mut p = [0; 1024];
        for (ch, &t) in keys[pos].iter().enumerate() {
            let track = d.get(1536 + usize::from(t) * 192..1536 + (usize::from(t) + 1) * 192)?;
            for (row, x) in track.chunks_exact(3).enumerate() {
                // Its D is ProTracker's A.
                let fx = if x[1] & 0x0f == 0x0d { x[1] - 3 } else { x[1] };
                let [hi, lo] = period(usize::from(x[0]))?;
                Pt::put(&mut p, row, ch, [hi, lo, fx, x[2]]);
            }
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, 1536 + 192 * (usize::from(most) + 1), total).to_vec();
    Some(pt)
}

// --- Module Protector ----------------------------------------------------

fn probe_mp_id(d: &[u8]) -> Option<()> {
    ok(d.get(..4)? == b"TRK1" && d.len() >= 382 && (0..31).all(|i| d[6 + 8 * i] <= 0x0f))?;
    ok((1..=0x7f).contains(&d[252]) && d[254..382].iter().all(|&o| o <= 0x7f))?;
    let cells = (usize::from(d[254..382].iter().copied().max().unwrap_or(0)) + 1) * 256;
    ok((0..cells).all(|i| d.get(382 + i * 4).is_some_and(|&x| x <= 19)))
}

fn probe_mp_noid(d: &[u8]) -> Option<()> {
    ok(d.len() >= 378)?;
    let mut total = 0;
    for i in 0..31 {
        let h = &d[i * 8..i * 8 + 8];
        let (size, start, lsize) = (w(h, 0)? * 2, w(h, 4)? * 2, w(h, 6)? * 2);
        total += size;
        ok(h[2] <= 0x0f && (lsize == 2 || start + lsize <= size) && lsize <= size + 2 && (start == 0 || lsize > 2) && (size == 0 || lsize != 0))?;
    }
    ok(total > 2 && (1..=0x7f).contains(&d[248]))?;
    let len = usize::from(d[248]);
    ok(d[250..378].iter().enumerate().all(|(i, &o)| o <= 0x7f && (i <= len + 3 || o == 0)))?;
    let cells = (usize::from(d[250..378].iter().copied().max().unwrap_or(0)) + 1) * 256;
    for i in 0..cells {
        let x = d.get(378 + i * 4..380 + i * 4)?;
        let per = w(x, 0)? & 0x0fff;
        ok((x[0] <= 19 || x[0] == 74) && (per == 0 || per >= 0x71))?;
    }
    Some(())
}

fn unpack_mp(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    let mut c = Cur::at(d, if d.get(..4)? == b"TRK1" { 4 } else { 0 });
    for _ in 0..31 {
        pt.raw_head(c.take(8)?);
    }
    pt.len = usize::from(c.u8()?);
    pt.restart = c.u8()?;
    pt.orders.copy_from_slice(c.take(128)?);
    if l(d, c.p)? == 0 {
        c.p += 4;
    }
    let count = usize::from(pt.orders.iter().copied().max().unwrap_or(0)) + 1;
    for _ in 0..count {
        pt.patterns.push(c.take(1024)?.try_into().ok()?);
    }
    pt.samples = upto(d, c.p, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- NovoTrade Packer ----------------------------------------------------

fn probe_ntp(d: &[u8]) -> Option<()> {
    let body = w(d, 20)? + 4;
    let samp = w(d, 28)? + body + 4;
    ok(d.get(..4)? == b"MODU" && d.get(body..body + 4)? == b"BODY" && d.get(samp..samp + 4)? == b"SAMP")
}

fn unpack_ntp(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    pt.title = d.get(4..20)?.to_vec();
    let body = w(d, 20)? + 4;
    let (count, len, patterns) = (w(d, 22)?, w(d, 24)?, w(d, 26)?);
    let samp = w(d, 28)? + body + 4;
    ok(patterns <= 128 && len <= 128)?;
    let mut heads = [[0u8, 0, 0, 0, 0, 0, 0, 0]; 31];
    let mut c = Cur::at(d, 30);
    for _ in 0..count {
        let x = c.take(8)?;
        if let Some(h) = heads.get_mut(usize::from(x[0])) {
            *h = [x[2], x[3], 0, x[1], x[4], x[5], x[6], x[7]];
        }
    }
    for h in heads {
        pt.raw_head(&h);
    }
    pt.len = len;
    for i in 0..len {
        let n = c.w()?;
        ok(n < patterns)?;
        pt.orders[i] = n as u8;
    }
    let addrs: Vec<usize> = (0..patterns).map(|_| c.w()).collect::<Option<_>>()?;
    for a in addrs {
        let mut p = [0; 1024];
        let mut c = Cur::at(d, body + 4 + a);
        for row in 0..64 {
            let mask = c.w()?;
            for ch in 0..4 {
                if mask & (1 << ch) != 0 {
                    Pt::put(&mut p, row, ch, c.take(4)?.try_into().ok()?);
                }
            }
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, samp, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- Promizer 1.0c and 1.8a -----------------------------------------------

/// The 68000 code both start with (their own replayer).
const PROMIZER: [u8; 16] = [0x60, 0x38, 0x60, 0x00, 0x00, 0xa0, 0x60, 0x00, 0x01, 0x3e, 0x60, 0x00, 0x01, 0x0c, 0x48, 0xe7];

fn probe_pm10c(d: &[u8]) -> Option<()> {
    ok(d.get(..16)? == PROMIZER && b(d, 21)? == 0xce && w(d, 4712)? & 3 == 0 && d[36] == 0x10 && d[37] == 0xfc)
}

fn probe_pm18a(d: &[u8]) -> Option<()> {
    ok(d.get(..16)? == PROMIZER && b(d, 21)? == 0xd2 && w(d, 4712)? & 3 == 0 && d[36] == 0x11 && d[37] == 0x00)
}

/// A period played with a finetune, as the note it is (ProTracker's
/// table, from its tuning: an eighth of a semitone a step).
fn untune(per: usize, finetune: u8) -> Option<usize> {
    let ft = f64::from(if finetune > 7 { i32::from(finetune) - 16 } else { i32::from(finetune) });
    (0..36).find(|&n| (856.0 * 2f64.powf(-(n as f64 * 8.0 + ft) / 96.0) - per as f64).abs() < 1.5).map(|n| n + 1)
}

/// Promizer's patterns: each cell a reference to a 4-byte cell in a
/// table after them, its periods kept with the sample's finetune.
fn unpack_promizer(d: &[u8], heads_at: usize, refs_at: usize, refs_len: usize, samples_at: usize) -> Option<Pt> {
    let mut pt = Pt::new();
    let mut finetunes = [0u8; 31];
    let mut c = Cur::at(d, heads_at);
    for ft in &mut finetunes {
        let h = c.take(8)?;
        *ft = h[2];
        pt.raw_head(h);
    }
    pt.len = c.w()? / 4;
    ok(pt.len <= 128)?;
    let addrs: Vec<usize> = (0..128).map(|_| c.l()).collect::<Option<_>>()?;
    // Patterns numbered in the order they're stored.
    let mut stored = addrs[..pt.len].to_vec();
    stored.sort_unstable();
    stored.dedup();
    for (o, a) in pt.orders.iter_mut().zip(&addrs[..pt.len]) {
        *o = stored.iter().position(|s| s == a)? as u8;
    }
    let refs = d.get(refs_at..refs_at.checked_add(refs_len)?)?;
    let most = refs.chunks_exact(2).map(|r| usize::from(u16::from_be_bytes([r[0], r[1]]))).max()?;
    let table = d.get(refs_at + refs_len..refs_at + refs_len + (most + 1) * 4)?;
    for &at in &stored {
        let mut p = [0; 1024];
        let mut c = Cur::at(d, refs_at + at);
        let mut last = [0u8; 4];
        'rows: for row in 0..64 {
            let mut ends = false;
            for (ch, last) in last.iter_mut().enumerate() {
                let x = c.w()? * 4;
                let mut cell: [u8; 4] = table.get(x..x + 4)?.try_into().ok()?;
                let ins = (cell[0] & 0xf0) | (cell[2] >> 4);
                if ins != 0 {
                    *last = ins;
                }
                let per = (usize::from(cell[0] & 0x0f) << 8) | usize::from(cell[1]);
                let ft = if (1..32).contains(last) { finetunes[usize::from(*last) - 1] } else { 0 };
                ok(ft < 16)?;
                if per != 0 && *last != 0 && ft != 0 {
                    if let Some([hi, lo]) = untune(per, ft).and_then(period) {
                        cell[0] = (cell[0] & 0xf0) | hi;
                        cell[1] = lo;
                    }
                }
                ends |= matches!(cell[2] & 0x0f, 0x0b | 0x0d);
                Pt::put(&mut p, row, ch, cell);
            }
            if ends {
                break 'rows;
            }
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, samples_at, pt.sample_bytes()).to_vec();
    Some(pt)
}

fn unpack_pm10c(d: &[u8]) -> Option<Pt> {
    unpack_promizer(d, 4460, 5222, l(d, 4456)?, 4456 + l(d, 4452)?)
}

fn unpack_pm18a(d: &[u8]) -> Option<Pt> {
    unpack_promizer(d, 4464, 5226, l(d, 4460)?, 4460 + l(d, 4456)?)
}

// --- ProRunner 1 and 2 ---------------------------------------------------

fn probe_pru1(d: &[u8]) -> Option<()> {
    ok(d.get(1080..1084)? == b"SNT." && d[951] == 0x7f && d[950] <= 0x7f)
}

fn unpack_pru1(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    pt.title = d.get(..20)?.to_vec();
    pt.names_at(d, 20, 22);
    for i in 0..31 {
        pt.raw_head(d.get(42 + i * 30..50 + i * 30)?);
    }
    pt.len = usize::from(d[950]);
    pt.restart = d[951];
    pt.orders.copy_from_slice(d.get(952..1080)?);
    let count = usize::from(pt.orders.iter().copied().max().unwrap_or(0)) + 1;
    let mut c = Cur::at(d, 1084);
    for _ in 0..count {
        let mut p = [0; 1024];
        for i in 0..256 {
            let x = c.take(4)?;
            let [hi, lo] = period(usize::from(x[1]))?;
            p[i * 4..][..4].copy_from_slice(&[(x[0] & 0xf0) | hi, lo, (x[0] << 4) | x[2], x[3]]);
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, c.p, pt.sample_bytes()).to_vec();
    Some(pt)
}

fn probe_pru2(d: &[u8]) -> Option<()> {
    ok(d.get(..4)? == b"SNT!" && (0..31).all(|k| d.get(10 + k * 8).is_some_and(|&f| f <= 0x0f) && d.get(11 + k * 8).is_some_and(|&v| v <= 0x40)))
}

fn unpack_pru2(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    for i in 0..31 {
        pt.raw_head(d.get(8 + i * 8..16 + i * 8)?);
    }
    pt.len = usize::from(b(d, 256)?);
    pt.restart = b(d, 257)?;
    pt.orders.copy_from_slice(d.get(258..386)?);
    let count = usize::from(pt.orders.iter().copied().max().unwrap_or(0)) + 1;
    let mut c = Cur::at(d, 770);
    let mut last = [[0u8; 4]; 4];
    for _ in 0..count {
        let mut p = [0; 1024];
        for i in 0..256 {
            let x = c.u8()?;
            let v = &mut last[i & 3];
            let cell = match x {
                0x80 => [0; 4],
                0xc0 => *v,
                _ => {
                    let (y, z) = (c.u8()?, c.u8()?);
                    let [hi, lo] = period(usize::from(x >> 1))?;
                    *v = [((y & 0x80) >> 3) | hi, lo, ((y & 0x70) << 1) | ((x & 1) << 4) | (y & 0x0f), z];
                    *v
                }
            };
            p[i * 4..][..4].copy_from_slice(&cell);
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, c.p, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- Pha Packer ----------------------------------------------------------

fn probe_pha(d: &[u8]) -> Option<()> {
    ok(b(d, 10)? == 0x03 && b(d, 11)? == 0xc0 && d.len() >= 960)?;
    let mut total = 0;
    for i in 0..31 {
        let h = &d[i * 14..i * 14 + 14];
        total += w(h, 0)? * 2;
        ok(h[3] <= 0x40 && w(h, 4)? * 2 <= total && l(h, 8)? >= 0x3c0)?;
    }
    ok(total > 2 && total <= 31 * 65535 && (0..128).all(|i| l(d, 448 + i * 4).is_some_and(|a| a + 2 >= total + 960)))
}

fn unpack_pha(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    for i in 0..31 {
        let h = d.get(i * 14..i * 14 + 14)?;
        let ft = if h[13] == 0 { 0 } else { h[13].wrapping_add(11) };
        pt.head(w(h, 0)?, ft, h[3], w(h, 4)?, w(h, 6)?);
    }
    let addrs: Vec<usize> = (0..128).map(|i| l(d, 448 + i * 4)).collect::<Option<_>>()?;
    let orders = numbered_with_gaps(&addrs, 128, 120);
    // The song ends at the last position that isn't the first pattern.
    pt.len = (1..=128).rev().find(|&n| orders[n - 1] != 0).unwrap_or(0);
    pt.orders.copy_from_slice(&orders);
    let count = usize::from(orders[..pt.len].iter().copied().max().unwrap_or(0)) + 1;
    let start = addrs.iter().copied().min()?;
    let packed = upto(d, start, count * 1024);
    // One stream for all the patterns: a cell repeats its channel's last
    // while the count a 0xff byte set for it runs.
    let mut cells = vec![0u8; count * 1024];
    let (mut repeat, mut last) = ([0usize; 4], [[0u8; 4]; 4]);
    let (mut i, mut k) = (0, 0);
    while i < packed.len() && k < count * 256 {
        if packed[i] == 0xff {
            repeat[(k + 3) % 4] = usize::from(0xff - *packed.get(i + 1)?);
            i += 2;
            continue;
        }
        let x = if repeat[k % 4] != 0 {
            repeat[k % 4] -= 1;
            last[k % 4]
        } else {
            let x: [u8; 4] = packed.get(i..i + 4)?.try_into().ok()?;
            i += 4;
            last[k % 4] = x;
            x
        };
        cells[k * 4..][..4].copy_from_slice(&cell_or_none(usize::from(x[1] / 2), x[0], x[2], x[3]));
        k += 1;
    }
    for p in cells.chunks_exact(1024) {
        pt.patterns.push(p.try_into().ok()?);
    }
    pt.samples = upto(d, 960, pt.sample_bytes()).to_vec();
    Some(pt)
}

/// Positions numbered as patterns by their addresses (the order the
/// patterns are stored in), a pattern no position names counted where
/// two stored ones are further apart than one pattern (1024 bytes).
fn numbered_with_gaps(addrs: &[usize], len: usize, scan: usize) -> [u8; 128] {
    let mut sorted: Vec<usize> = addrs[..len].to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut stored: Vec<usize> = Vec::new();
    for (i, &a) in sorted.iter().enumerate().take(scan) {
        stored.push(a);
        if sorted.get(i + 1).is_some_and(|&n| n - a > 1024) {
            stored.push(a + 1024);
        }
    }
    stored.truncate(128);
    let mut orders = [0u8; 128];
    for (o, a) in orders.iter_mut().zip(addrs) {
        *o = stored.iter().position(|s| s == a).unwrap_or(0) as u8;
    }
    orders
}

// --- Wanton Packer -------------------------------------------------------

fn probe_wn(d: &[u8]) -> Option<()> {
    ok(d.get(1080..1082)? == b"WN" && d[951] == 0x7f && d[950] <= 0x7f)
}

fn unpack_wn(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    pt.title = d.get(..20)?.to_vec();
    pt.names_at(d, 20, 22);
    for i in 0..31 {
        pt.raw_head(d.get(42 + i * 30..50 + i * 30)?);
    }
    pt.len = usize::from(d[950]);
    pt.restart = d[951];
    pt.orders.copy_from_slice(d.get(952..1080)?);
    let count = usize::from(pt.orders.iter().copied().max().unwrap_or(0)) + 1;
    let mut c = Cur::at(d, 1084);
    for _ in 0..count {
        let mut p = [0; 1024];
        for i in 0..256 {
            let x = c.take(4)?;
            let [hi, lo] = period(usize::from(x[0] / 2))?;
            p[i * 4..][..4].copy_from_slice(&[(x[1] & 0xf0) | hi, lo, (x[1] << 4) | x[2], x[3]]);
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, c.p, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- Unic Tracker 1 and 2 ------------------------------------------------

/// Unic's sample headers: a 20-byte name, a finetune as a signed word,
/// and a loop start that's in bytes when it fits that way.
fn unic_heads(pt: &mut Pt, d: &[u8], at: usize) -> Option<()> {
    for i in 0..31 {
        let h = d.get(at + i * 30..at + i * 30 + 30)?;
        pt.names.push(h[..20].to_vec());
        let fine = match w(h, 20)? {
            0 => 0,
            j if j < 256 => 0x10u8.wrapping_sub(h[21]),
            _ => 0u8.wrapping_sub(h[21]),
        };
        let len = w(h, 22)?;
        let (mut ls, ll) = (w(h, 26)?, w(h, 28)?);
        if ls * 2 + ll <= len && ls != 0 {
            ls <<= 1;
        }
        pt.head(len, fine, h[25], ls, ll);
    }
    Some(())
}

/// Unic's 3-byte cells.
fn unic_patterns(pt: &mut Pt, d: &[u8], at: usize) -> Option<usize> {
    let count = usize::from(pt.orders.iter().copied().max().unwrap_or(0)) + 1;
    let mut c = Cur::at(d, at);
    for _ in 0..count {
        let mut p = [0; 1024];
        for i in 0..256 {
            let x = c.take(3)?;
            let ins = ((x[0] >> 2) & 0x10) | (x[1] >> 4);
            let fx = x[1] & 0x0f;
            // Its pattern breaks are binary; ProTracker's are decimal.
            let fp = if fx == 0x0d { (x[2] / 10) * 16 + x[2] % 10 } else { x[2] };
            p[i * 4..][..4].copy_from_slice(&cell(usize::from(x[0] & 0x3f), ins, fx, fp)?);
        }
        pt.patterns.push(p);
    }
    Some(c.p)
}

fn unic_instruments(d: &[u8]) -> Option<usize> {
    let (mut total, mut most) = (0, 0);
    for i in 0..31 {
        let h = d.get(20 + i * 30..50 + i * 30)?;
        let (len, start, lsize) = (w(h, 22)? * 2, w(h, 26)? * 2, w(h, 28)? * 2);
        total += len;
        let fine = w(h, 20)?;
        ok((lsize == 0 || len + 2 >= start + lsize) && h[25] <= 0x40 && !(fine != 0 && len == 0) && !(fine > 8 && fine < 247))?;
        ok((start == 0 || lsize > 2) && !(h[25] != 0 && len == 0))?;
        if len != 0 {
            most = i + 1;
        }
    }
    ok(total > 2)?;
    Some(most)
}

fn unic_orders(d: &[u8], at: usize) -> Option<usize> {
    let len = usize::from(b(d, at)?);
    let orders = d.get(at + 2..at + 130)?;
    ok((1..=127).contains(&len) && orders[..len].iter().all(|&o| o <= 127) && orders[len..].iter().all(|&o| o == 0))?;
    Some((usize::from(orders.iter().copied().max().unwrap_or(0)) + 1) * 256)
}

fn unic_cells_plausible(d: &[u8], cells: usize, most: usize, at: usize, ins_from_fp: bool) -> Option<()> {
    for i in 0..cells {
        let x = d.get(at + i * 3..at + i * 3 + 3)?;
        let ins = ((x[0] >> 2) & 0x30) | (if ins_from_fp { x[2] } else { x[1] } >> 4);
        let (fx, fp) = (x[1] & 0x0f, x[2]);
        ok(x[0] <= 0x74 && x[0] & 0x3f <= 0x24 && !(fx == 0x0c && fp > 0x40) && !(fx == 0x0b && fp > 0x7f) && !(fx == 0x0d && fp > 0x40) && usize::from(ins) <= most)?;
    }
    Some(())
}

/// Whether `cells` 4-byte cells from `at` read as a MOD's: a note
/// somewhere, and every period one ProTracker plays. Packers that keep a
/// MOD's tag aren't taken for one.
fn mod_like(d: &[u8], at: usize, cells: usize) -> bool {
    let mut note = false;
    for x in upto(d, at, cells * 4).chunks_exact(4) {
        note |= x != [0; 4];
        let per = (usize::from(x[0] & 0x0f) << 8) | usize::from(x[1]);
        if per != 0 && !(0x71..=0x358).contains(&per) {
            return false;
        }
    }
    note
}

fn probe_unic_id(d: &[u8]) -> Option<()> {
    ok(d.get(1080..1084)? == b"M.K.")?;
    let mut total = 0;
    for i in 0..31 {
        let h = &d[20 + i * 30..50 + i * 30];
        let size = w(h, 22)? * 2;
        total += size;
        ok(size + 2 >= (w(h, 26)? + w(h, 28)?) * 2 && (-8..=7).contains(&(h[20] as i8)) && h[24] == 0 && h[25] <= 0x40)?;
    }
    ok(total > 2)?;
    let cells = unic_orders(d, 950)?;
    ok((0..cells).all(|i| d.get(1084 + i * 3).is_some_and(|&x| x <= 0x74)))?;
    ok(!mod_like(d, 1084, cells / 2))
}

fn probe_unic_emptyid(d: &[u8]) -> Option<()> {
    ok(l(d, 1080)? == 0)?;
    let most = unic_instruments(d)?;
    unic_cells_plausible(d, unic_orders(d, 950)?, most, 1084, false)
}

fn probe_unic_noid(d: &[u8]) -> Option<()> {
    ok(l(d, 1080)? != 0)?;
    let most = unic_instruments(d)?;
    unic_cells_plausible(d, unic_orders(d, 950)?, most, 1080, false)?;
    ok(d[..20].iter().all(|&c| c == 0 || (32..=180).contains(&c)))
}

fn unpack_unic(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    pt.title = d.get(..20)?.to_vec();
    unic_heads(&mut pt, d, 20)?;
    pt.len = usize::from(b(d, 950)?);
    pt.orders.copy_from_slice(d.get(952..1080)?);
    let id = d.get(1080..1084)?;
    let at = if id == [0; 4] || id == b"M.K." || id == b"UNIC" { 1084 } else { 1080 };
    let end = unic_patterns(&mut pt, d, at)?;
    pt.samples = upto(d, end, pt.sample_bytes()).to_vec();
    Some(pt)
}

fn probe_unic2(d: &[u8]) -> Option<()> {
    ok(l(d, 1080)? != 0)?;
    let (mut total, mut most) = (0, 0);
    for i in 0..31 {
        let h = &d[i * 30..i * 30 + 30];
        let (size, start, lsize) = (w(h, 22)? * 2, w(h, 26)? * 2, w(h, 28)? * 2);
        total += size;
        ok(size + 2 >= start + lsize && h[25] <= 0x40 && !(w(h, 20)? != 0 && size == 0) && !(h[25] != 0 && size == 0))?;
        if size != 0 {
            most = i + 1;
        }
    }
    ok(total > 2)?;
    let len = usize::from(d[930]);
    let orders = d.get(932..1060)?;
    ok((1..=127).contains(&len) && orders[..len].iter().all(|&o| o <= 127) && orders.get(len + 2..).map_or(true, |r| r.iter().all(|&o| o == 0)))?;
    let cells = (usize::from(orders.iter().copied().max().unwrap_or(0)) + 1) * 256;
    unic_cells_plausible(d, cells, most, 1060, true)
}

fn unpack_unic2(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    // Its headers are where a MOD's title and headers end: no title.
    let mut heads = Pt::new();
    unic_heads(&mut heads, &[&[0u8; 20][..], d.get(..930)?].concat(), 20)?;
    pt.heads = heads.heads;
    pt.len = usize::from(b(d, 930)?);
    pt.orders.copy_from_slice(d.get(932..1060)?);
    let end = unic_patterns(&mut pt, d, 1060)?;
    pt.samples = upto(d, end, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- Tracker Packer 1 to 3 -----------------------------------------------

fn probe_tp1(d: &[u8]) -> Option<()> {
    ok(d.get(..4)? == b"MEXX")?;
    let size = l(d, 4)?;
    let smp = l(d, 28)?;
    ok((794..=2129178).contains(&size) && smp != 0 && smp <= size)?;
    for i in 0..31 {
        let h = d.get(32 + i * 8..40 + i * 8)?;
        let (sz, start, lsize) = (w(h, 2)? * 2, w(h, 4)? * 2, w(h, 6)? * 2);
        ok(h[0] <= 0x0f && h[1] <= 0x40 && start + lsize <= sz + 2 && !(start != 0 && lsize == 0))?;
    }
    ok(w(d, 280)? < 128)
}

fn unpack_tp1(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    pt.title = d.get(8..28)?.to_vec();
    let smp = l(d, 28)?;
    for i in 0..31 {
        let h = d.get(32 + i * 8..40 + i * 8)?;
        pt.head(w(h, 2)?, h[0], h[1], w(h, 4)?, w(h, 6)?);
    }
    pt.len = w(d, 280)? + 1;
    let addrs: Vec<usize> = (0..pt.len).map(|i| l(d, 282 + i * 4)).collect::<Option<_>>()?;
    let lowest = addrs.iter().copied().min()?;
    let (orders, first) = number_by_first(&addrs);
    pt.orders[..orders.len()].copy_from_slice(&orders);
    for &pos in &first {
        let mut p = [0; 1024];
        let mut c = Cur::at(d, 794 + addrs[pos] - lowest);
        for i in 0..256 {
            let c1 = c.u8()?;
            let x = if c1 == 0xc0 {
                continue;
            } else if c1 & 0xc0 == 0x80 {
                [0, 0, (c1 >> 2) & 0x0f, c.u8()?]
            } else {
                let (c2, c3) = (c.u8()?, c.u8()?);
                cell(usize::from(c1 >> 1), (c2 >> 4) | ((c1 << 4) & 0x10), c2 & 0x0f, c3)?
            };
            p[i * 4..][..4].copy_from_slice(&x);
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, smp, pt.sample_bytes()).to_vec();
    Some(pt)
}

fn probe_tp23(d: &[u8], magic: &[u8]) -> Option<()> {
    ok(d.get(..8)? == magic)?;
    let n = w(d, 28)?;
    ok(n != 0 && n % 8 == 0 && n / 8 <= 31)?;
    let mut total = 0;
    for i in 0..n / 8 {
        let h = d.get(30 + i * 8..38 + i * 8)?;
        let (len, start, lsize) = (w(h, 2)? * 2, w(h, 4)? * 2, w(h, 6)? * 2);
        ok(h[0] <= 0x0f && h[1] <= 0x40 && start + lsize <= len + 2 && !(start != 0 && lsize == 0))?;
        total += len;
    }
    ok(total > 4 && (1..=128).contains(&b(d, n + 31)?))
}

fn probe_tp2(d: &[u8]) -> Option<()> {
    probe_tp23(d, b"MEXX_TP2")
}

fn probe_tp3(d: &[u8]) -> Option<()> {
    probe_tp23(d, b"CPLX_TP3")
}

fn unpack_tp2(d: &[u8]) -> Option<Pt> {
    unpack_tp23(d, 2)
}

fn unpack_tp3(d: &[u8]) -> Option<Pt> {
    unpack_tp23(d, 3)
}

fn unpack_tp23(d: &[u8], version: u8) -> Option<Pt> {
    let mut pt = Pt::new();
    pt.title = d.get(8..28)?.to_vec();
    let n = w(d, 28)? / 8;
    let mut c = Cur::at(d, 30);
    for _ in 0..n {
        let h = c.take(8)?;
        pt.head(w(h, 2)?, h[0], h[1], w(h, 4)?, w(h, 6)?);
    }
    c.p += 1;
    pt.len = usize::from(c.u8()?);
    ok(pt.len < 128)?;
    let mut most = 0;
    for i in 0..pt.len {
        let o = (c.w()? / 8) as u8;
        pt.orders[i] = o;
        most = most.max(o);
    }
    ok(most < 128)?;
    let mut tracks = Vec::new();
    for _ in 0..=most {
        tracks.push([c.w()?, c.w()?, c.w()?, c.w()?]);
    }
    let base = c.p + 2;
    let mut end = 0;
    for t in tracks {
        let mut p = [0; 1024];
        for (ch, &off) in t.iter().enumerate() {
            let mut c = Cur::at(d, base + off);
            let mut row = 0;
            while row < 64 {
                let c1 = c.u8()?;
                if c1 & 0xc0 == 0xc0 {
                    row += usize::from(0u8.wrapping_sub(c1));
                    continue;
                }
                let fx_of = |fx: u8, fp: u8| -> (u8, u8) {
                    let fp = if matches!(fx, 5 | 6 | 0x0a) {
                        if fp > 0x80 {
                            fp.wrapping_neg()
                        } else {
                            fp << 4
                        }
                    } else {
                        fp
                    };
                    (if fx == 8 { 0 } else { fx }, fp)
                };
                if c1 & 0xc0 == 0x80 {
                    let fx = if version == 2 { (c1 >> 2) & 0x0f } else { (c1 >> 1) & 0x0f };
                    let (fx, fp) = fx_of(fx, c.u8()?);
                    Pt::put(&mut p, row, ch, [0, 0, fx, fp]);
                    row += 1;
                    continue;
                }
                let c2 = c.u8()?;
                let ins = (c2 >> 4) | ((c1 >> 2) & 0x10);
                let note = if version == 2 {
                    usize::from(c1 >> 1)
                } else if c1 & 0x40 != 0 {
                    usize::from(0x7f - c1)
                } else {
                    usize::from(c1 & 0x3f)
                };
                let fx = c2 & 0x0f;
                let x = if fx == 0 {
                    cell(note, ins, 0, 0)?
                } else {
                    let (fx, fp) = fx_of(fx, c.u8()?);
                    cell(note, ins, fx, fp)?
                };
                Pt::put(&mut p, row, ch, x);
                row += 1;
            }
            end = end.max(c.p);
        }
        pt.patterns.push(p);
    }
    if version > 2 && end % 2 == 1 {
        end += 1;
    }
    pt.samples = upto(d, end, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- SKYT Packer ---------------------------------------------------------

fn probe_skyt(d: &[u8]) -> Option<()> {
    ok(d.get(256..260)? == b"SKYT" && (0..31).all(|i| d[8 * i + 4] <= 0x40))
}

fn unpack_skyt(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    for i in 0..31 {
        pt.raw_head(d.get(i * 8..i * 8 + 8)?);
    }
    pt.len = usize::from(b(d, 260)?) + 1;
    ok(pt.len < 128)?;
    let mut c = Cur::at(d, 261);
    let mut tracks = Vec::new();
    let mut most = 0;
    for _ in 0..pt.len {
        let t = [c.w()?, c.w()?, c.w()?, c.w()?];
        most = most.max(t.iter().copied().max().unwrap_or(0));
        tracks.push(t);
    }
    for (i, o) in pt.orders.iter_mut().enumerate().take(pt.len) {
        *o = i as u8;
    }
    let base = c.p + 1;
    for t in tracks {
        let mut p = [0; 1024];
        for (ch, &n) in t.iter().enumerate() {
            if n == 0 {
                continue;
            }
            let track = d.get(base + (n - 1) * 256..base + n * 256)?;
            for (row, x) in track.chunks_exact(4).enumerate() {
                let [hi, lo] = period(usize::from(x[0]))?;
                Pt::put(&mut p, row, ch, [(x[1] & 0xf0) | hi, lo, (x[1] << 4) | x[2], x[3]]);
            }
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, base + most * 256, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- Xann Packer ---------------------------------------------------------

fn probe_xann(d: &[u8]) -> Option<()> {
    ok(b(d, 3)? == 0x3c && d.len() >= 1006)?;
    ok((0..128).all(|i| l(d, i * 4).is_some_and(|a| a % 4 == 0 && a <= 132156)))?;
    ok((0..64).all(|i| d[3 + i * 4] == 0x3c || d[3 + i * 4] == 0))?;
    ok((0..31).all(|i| d[519 + 16 * i] <= 0x40))?;
    for i in 0..30 {
        let (a, b) = (l(d, 526 + 16 * i)?, l(d, 520 + 16 * (i + 1))?);
        ok(a >= 2108 && b >= 2108 && a <= b)?;
    }
    Some(())
}

fn unpack_xann(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    for i in 0..31 {
        let h = d.get(0x206 + i * 16..0x206 + (i + 1) * 16)?;
        let (loop_at, ll, at, len) = (l(h, 2)?, w(h, 6)?, l(h, 8)?, w(h, 12)?);
        pt.head(len, h[0], h[1], loop_at.wrapping_sub(at) / 2, ll);
    }
    let mut most = 0;
    while pt.len < 128 {
        let a = l(d, pt.len * 4)?;
        if a == 0 {
            break;
        }
        let o = (a.wrapping_sub(0x3c) / 1024).wrapping_sub(1) as u8;
        pt.orders[pt.len] = o;
        most = most.max(o);
        pt.len += 1;
    }
    let mut c = Cur::at(d, 0x43c);
    for _ in 0..=most {
        let mut p = [0; 1024];
        for i in 0..256 {
            let x = c.take(4)?;
            let ins = (x[0] >> 3) & 0x1f;
            let swap = |fp: u8| fp.rotate_left(4);
            // Its own effect numbers, as ProTracker's.
            let (fx, fp) = match x[2] {
                0x00 | 0x04 => (0, x[3]),
                0x08 => (1, x[3]),
                0x0c => (2, x[3]),
                0x10 | 0x14 => (3, x[3]),
                0x18 | 0x1c => (4, x[3]),
                0x24 => (5, x[3]),
                0x28 => (6, swap(x[3])),
                0x2c => (6, x[3]),
                0x38 => (9, x[3]),
                0x3c => (0x0a, swap(x[3])),
                0x40 => (0x0a, x[3]),
                0x44 => (0x0b, x[3]),
                0x48 => (0x0c, x[3]),
                0x4c => (0x0d, x[3]),
                0x50 => (0x0f, x[3]),
                0x58 => (0x0e, 0x01),
                0x5c => (0x0e, x[3] | 0x10),
                0x60 => (0x0e, x[3] | 0x20),
                0x84 => (0x0e, x[3] | 0x90),
                0x88 => (0x0e, x[3] | 0xa0),
                0x8c => (0x0e, x[3] | 0xb0),
                0x94 => (0x0e, x[3] | 0xd0),
                0x98 => (0x0e, x[3] | 0xe0),
                _ => (0, 0),
            };
            p[i * 4..][..4].copy_from_slice(&cell(usize::from(x[1] >> 1), ins, fx, fp)?);
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, c.p, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- Digital Illusions ---------------------------------------------------

fn probe_di(d: &[u8]) -> Option<()> {
    let n = w(d, 0)?;
    ok(n <= 31)?;
    let mut total = 0;
    for i in 0..n {
        let h = d.get(14 + i * 8..22 + i * 8)?;
        let (len, start, lsize) = (w(h, 0)? * 2, w(h, 4)? * 2, w(h, 6)? * 2);
        ok(start + lsize <= len && h[2] <= 0x0f && h[3] <= 0x40)?;
        total += len;
    }
    ok(total > 2)?;
    let (seq, pat, smp) = (l(d, 2)?, l(d, 6)?, l(d, 10)?);
    ok(seq >= n * 8 + 2 && pat > seq && smp > seq && smp > pat && pat - seq <= 128 && smp <= 65535)?;
    ok(d.get(seq..pat - 1)?.iter().all(|&o| o <= 0x80) && b(d, pat - 1)? == 0xff)
}

fn unpack_di(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    let n = w(d, 0)?;
    ok(n <= 31)?;
    let (seq, smp) = (l(d, 2)?, l(d, 10)?);
    for i in 0..n {
        pt.raw_head(d.get(14 + i * 8..22 + i * 8)?);
    }
    while pt.heads.len() < 31 {
        pt.heads.push([0; 8]);
    }
    let mut most = 0;
    loop {
        let o = b(d, seq + pt.len)?;
        if o == 0xff {
            break;
        }
        ok(pt.len < 128)?;
        pt.orders[pt.len] = o;
        most = most.max(o);
        pt.len += 1;
    }
    ok(most < 128)?;
    let at = 14 + n * 8;
    for i in 0..=usize::from(most) {
        let mut p = [0; 1024];
        let mut c = Cur::at(d, w(d, at + i * 2)?);
        for k in 0..256 {
            let c1 = c.u8()?;
            if c1 == 0xff {
                continue;
            }
            let c2 = c.u8()?;
            let fp = if c1 & 0x80 != 0 { c.u8()? } else { 0 };
            let note = usize::from(((c1 << 4) & 0x30) | (c2 >> 4));
            let x = cell(note, (c1 >> 2) & 0x1f, c2 & 0x0f, fp).unwrap_or([0; 4]);
            p[k * 4..][..4].copy_from_slice(&x);
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, smp, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- Eureka Packer -------------------------------------------------------

fn probe_eureka(d: &[u8]) -> Option<()> {
    let len = usize::from(b(d, 950)?);
    ok((1..=127).contains(&len) && d.len() >= 1084)?;
    for i in 0..31 {
        let h = &d[42 + i * 30..50 + i * 30];
        let (size, start, lsize) = (w(h, 0)? * 2, w(h, 4)? * 2, w(h, 6)? * 2);
        ok(start + lsize <= size + 2 && h[2] <= 0x0f && h[3] <= 0x40)?;
    }
    let smp = l(d, 1080)?;
    ok(smp >= 1084)?;
    let orders = &d[952..1080];
    ok(orders[..len].iter().all(|&o| o <= 127) && orders[len..].iter().all(|&o| o == 0))?;
    let count = usize::from(orders.iter().copied().max().unwrap_or(0)) + 1;
    let (mut lo, mut hi) = (usize::MAX, 0);
    for i in 0..count * 4 {
        let t = w(d, 1084 + i * 2)?;
        ok((1084..=smp).contains(&t))?;
        (lo, hi) = (lo.min(t), hi.max(t));
    }
    let mut i = lo;
    while i < hi {
        let x = b(d, i)?;
        match x & 0xc0 {
            0xc0 => i += 1,
            0x80 => i += 3,
            0x40 => {
                ok(!(x & 0x3f == 0 && b(d, i + 1)? == 0))?;
                i += 2;
            }
            _ => {
                ok(x <= 0x13)?;
                i += 4;
            }
        }
    }
    Some(())
}

fn unpack_eureka(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    pt.title = d.get(..20)?.to_vec();
    pt.names_at(d, 20, 22);
    for i in 0..31 {
        pt.raw_head(d.get(42 + i * 30..50 + i * 30)?);
    }
    pt.len = usize::from(d[950]);
    pt.restart = d[951];
    pt.orders.copy_from_slice(d.get(952..1080)?);
    let count = usize::from(pt.orders.iter().copied().max().unwrap_or(0)) + 1;
    let smp = l(d, 1080)?;
    for i in 0..count {
        let mut p = [0; 1024];
        for ch in 0..4 {
            let mut c = Cur::at(d, w(d, 1084 + (i * 4 + ch) * 2)?);
            let mut row = 0;
            while row < 64 {
                let x = c.u8()?;
                let cell = match x & 0xc0 {
                    0x00 => [x, c.u8()?, c.u8()?, c.u8()?],
                    0xc0 => {
                        row += usize::from(x & 0x3f) + 1;
                        continue;
                    }
                    0x40 => [0, 0, x & 0x0f, c.u8()?],
                    _ => [c.u8()?, c.u8()?, x << 4, 0],
                };
                Pt::put(&mut p, row, ch, cell);
                row += 1;
            }
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, smp, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- The Player 4.0A, 4.0B, 4.1A ---------------------------------------

fn probe_p4x(d: &[u8]) -> Option<()> {
    ok(matches!(d.get(..4)?, b"P40A" | b"P40B" | b"P41A") && d.len() >= 8)
}

/// The Player 4's cells: a note (doubled, the sample's high bit below
/// it), the rest of the sample and the effect, its parameter.
fn p4x_cell(c1: u8, c2: u8, c3: u8) -> [u8; 4] {
    let Some([hi, lo]) = period(usize::from((c1 & 0x7f) / 2)) else { return [0; 4] };
    let fx = c2 & 0x0f;
    let fp = if matches!(fx, 5 | 6 | 0x0a) && c3 > 0x7f { c3 << 4 } else { c3 };
    [((c1 << 4) & 0x10) | hi, lo, if fx == 8 { c2 & 0xf0 } else { c2 }, fp]
}

fn unpack_p4x(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    let id = d.get(..4)?;
    pt.len = usize::from(b(d, 5)?);
    let n = usize::from(b(d, 6)?);
    ok(pt.len < 128 && n <= 31)?;
    let (tracks_at, table_at, samples_at) = (l(d, 8)? + 4, l(d, 12)? + 4, l(d, 16)? + 4);
    ok(tracks_at < d.len() && table_at < d.len() && samples_at < d.len())?;
    let mut c = Cur::at(d, 20);
    let mut starts = Vec::new();
    for _ in 0..n {
        let (at, len, loop_at, ll) = (c.l()?, c.w()?, c.l()?, c.w()?);
        let fine = if id != b"P41A" { c.w()? } else { 0 };
        c.p += 1;
        let vol = c.u8()?;
        let fine = if id == b"P41A" { c.w()? } else { fine };
        ok(loop_at >= at && at <= d.len() - samples_at)?;
        pt.head(len, (fine as i16 / 74) as u8, vol, (loop_at - at) / 2, ll);
        starts.push((at, len * 2));
    }
    for (i, o) in pt.orders.iter_mut().enumerate().take(pt.len) {
        *o = i as u8;
    }
    let mut c = Cur::at(d, table_at);
    let mut addrs = Vec::new();
    for _ in 0..pt.len {
        addrs.push([c.w()? + tracks_at, c.w()? + tracks_at, c.w()? + tracks_at, c.w()? + tracks_at]);
    }
    for a in addrs {
        let mut p = [0; 1024];
        for (ch, &at) in a.iter().enumerate() {
            let mut c = Cur::at(d, at);
            let mut row = 0;
            // One event, and the rows after it: c4 under 128 skips that
            // many, over it repeats the event 256 - c4 times.
            let event = |p: &mut [u8; 1024], row: &mut usize, x: &[u8]| {
                let cell = p4x_cell(x[0], x[1], x[2]);
                Pt::put(p, *row, ch, cell);
                *row += 1;
                match x[3] {
                    0 => {}
                    n if n < 0x80 => *row += usize::from(n),
                    n => {
                        for _ in 0..256 - usize::from(n) {
                            Pt::put(p, *row, ch, cell);
                            *row += 1;
                        }
                    }
                }
            };
            while row < 64 {
                let x = c.take(4)?;
                if x[0] != 0x80 {
                    event(&mut p, &mut row, x);
                    continue;
                }
                // Earlier events again: x[1] + 1 of them, from the track data's start plus x[2..4].
                let mut r = Cur::at(d, ((usize::from(x[2]) << 8) | usize::from(x[3])) + tracks_at);
                for _ in 0..=x[1] {
                    if row >= 64 {
                        break;
                    }
                    event(&mut p, &mut row, r.take(4)?);
                }
            }
        }
        pt.patterns.push(p);
    }
    for (at, len) in starts {
        pt.samples.extend_from_slice(upto(d, samples_at + at, len));
    }
    Some(pt)
}

// --- ProPacker 1.0, 2.1, 3.0 ---------------------------------------------

fn probe_pp10(d: &[u8]) -> Option<()> {
    ok(d.len() >= 1024 && d[249] <= 0x7f && (1..=127).contains(&d[248]))?;
    let mut total = 0;
    for i in 0..31 {
        let h = &d[i * 8..i * 8 + 8];
        let (size, start, lsize) = (w(h, 0)? * 2, w(h, 4)? * 2, w(h, 6)? * 2);
        total += size;
        ok(lsize != 0 && !(start != 0 && lsize <= 2) && start + lsize <= size + 2 && h[2] <= 0x0f && h[3] <= 0x40 && start <= size)?;
    }
    ok(total > 2)?;
    let tracks = usize::from(d[250..762].iter().copied().max().unwrap_or(0)) + 1;
    ok((0..tracks * 64).all(|i| d.get(762 + i * 4).is_some_and(|&x| x <= 0x13)))
}

fn unpack_pp10(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    for i in 0..31 {
        pt.raw_head(d.get(i * 8..i * 8 + 8)?);
    }
    pt.len = usize::from(d[248]);
    pt.restart = d[249];
    let tracks = d.get(250..762)?;
    let most = usize::from(tracks.iter().copied().max().unwrap_or(0));
    for i in 0..pt.len {
        pt.orders[i] = i as u8;
        let mut p = [0; 1024];
        for ch in 0..4 {
            let at = 762 + usize::from(tracks[ch * 128 + i]) * 256;
            for row in 0..64 {
                Pt::put(&mut p, row, ch, d.get(at + row * 4..at + row * 4 + 4)?.try_into().ok()?);
            }
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, 762 + (most + 1) * 256, pt.sample_bytes()).to_vec();
    Some(pt)
}

fn pp2x_heads_plausible(d: &[u8]) -> Option<usize> {
    let mut total = 0;
    for i in 0..31 {
        let h = d.get(i * 8..i * 8 + 8)?;
        let (len, start) = (w(h, 0)? * 2, w(h, 4)? * 2);
        total += len;
        ok(h[2] <= 0x0f && h[3] <= 0x40 && start <= len)?;
    }
    ok(total > 2 && (1..=127).contains(&b(d, 248)?))?;
    Some((usize::from(d.get(250..762)?.iter().copied().max().unwrap_or(0)) + 1) * 64)
}

fn probe_pp21(d: &[u8]) -> Option<()> {
    let refs = pp2x_heads_plausible(d)?;
    let mut most = 0;
    for i in 0..refs {
        let r = w(d, 762 + i * 2)?;
        ok(r <= 0x4000)?;
        most = most.max(r);
    }
    ok(l(d, 762 + refs * 2)? == (most + 1) * 4)
}

fn probe_pp30(d: &[u8]) -> Option<()> {
    let refs = pp2x_heads_plausible(d)?;
    let mut most = 0;
    for i in 0..refs {
        let r = w(d, 762 + i * 2)?;
        ok(r & 3 == 0)?;
        most = most.max(r);
    }
    let size = l(d, 762 + refs * 2)?;
    ok(size <= 0xffff && size == ((most >> 2) + 1) * 4)?;
    for i in 0..size / 4 {
        let x = d.get(766 + refs * 2 + i * 4..770 + refs * 2 + i * 4)?;
        let (fx, fp) = (x[2] & 0x0f, x[3]);
        ok(!(fx == 0x0c && fp > 0x41) && !(fx == 0x0d && (fp > 0x64 || fp & 0x0f > 9)) && !(fx == 0x0b && fp > 0x7f) && x[0] & 0xf0 <= 0x10)?;
    }
    Some(())
}

fn unpack_pp21(d: &[u8]) -> Option<Pt> {
    unpack_pp2x(d, 1)
}

fn unpack_pp30(d: &[u8]) -> Option<Pt> {
    unpack_pp2x(d, 4)
}

/// ProPacker 2.1 and 3.0: tracks of references to a table of cells (3.0's
/// references in bytes, 2.1's in cells).
fn unpack_pp2x(d: &[u8], unit: usize) -> Option<Pt> {
    let mut pt = Pt::new();
    for i in 0..31 {
        pt.raw_head(d.get(i * 8..i * 8 + 8)?);
    }
    pt.len = usize::from(b(d, 248)?);
    ok(pt.len <= 128)?;
    pt.restart = d[249];
    let tracks = d.get(250..762)?;
    let count = usize::from(tracks.iter().copied().max().unwrap_or(0)) + 1;
    let refs_at = 762;
    let size = l(d, refs_at + count * 128)?;
    ok(size > 0)?;
    let table = d.get(refs_at + count * 128 + 4..refs_at + count * 128 + 4 + size)?;
    for i in 0..pt.len {
        pt.orders[i] = i as u8;
        let mut p = [0; 1024];
        for ch in 0..4 {
            let t = usize::from(tracks[ch * 128 + i]);
            for row in 0..64 {
                let r = w(d, refs_at + t * 128 + row * 2)? / unit * 4;
                Pt::put(&mut p, row, ch, table.get(r..r + 4)?.try_into().ok()?);
            }
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, refs_at + count * 128 + 4 + size, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- The Player 5.0A, 6.0A, 6.1A -----------------------------------------

/// The Player's header (after its optional "P50A", "P60A" or "P61A"):
/// where the samples are, the patterns, the samples' count (with the
/// flags for delta and packed samples).
struct Player<'a> {
    d: &'a [u8],
    samples_at: usize,
    patterns: usize,
    count: usize,
    delta: bool,
    packed: bool,
    heads_at: usize,
}

fn player(d: &[u8]) -> Option<Player<'_>> {
    let d = if matches!(d.get(..4)?, b"P50A" | b"P60A" | b"P61A") { &d[4..] } else { d };
    let flags = b(d, 3)?;
    let packed = flags & 0x40 != 0;
    Some(Player { d, samples_at: w(d, 0)?, patterns: usize::from(b(d, 2)?), count: usize::from(flags & 0x3f), delta: flags & 0x80 != 0, packed, heads_at: if packed { 8 } else { 4 } })
}

/// The checks The Player's header passes, each version alike: its sample
/// headers, its track table, its order list (its length returned).
fn player_plausible(p: &Player, halved: bool) -> Option<usize> {
    let d = p.d;
    ok((1..=0x7f).contains(&p.patterns) && (1..=0x1f).contains(&p.count))?;
    for i in 0..p.count {
        let h = d.get(p.heads_at + i * 6..p.heads_at + i * 6 + 6)?;
        let size = w(h, 0)?;
        ok(h[3] <= 0x40 && h[2] & 0x3f <= 0x0f && !((0x8001..=0xffdf).contains(&size) || size == 0))?;
        let start = w(h, 4)?;
        ok((start == 0xffff || start < size) && (size <= 0xffdf || 0xffff - size <= p.count))?;
    }
    let tracks_at = p.heads_at + p.count * 6;
    let orders_at = tracks_at + p.patterns * 8;
    ok(p.samples_at >= orders_at)?;
    ok((0..p.patterns * 4).all(|i| w(d, tracks_at + i * 2).is_some_and(|t| t + orders_at <= p.samples_at)))?;
    let mut len = 0;
    loop {
        ok(len < 128 && orders_at + len < p.samples_at)?;
        let o = usize::from(b(d, orders_at + len)?);
        if o == 0xff {
            break;
        }
        ok(if halved { o % 2 == 0 && o <= p.patterns * 2 } else { o < p.patterns })?;
        len += 1;
    }
    ok(len > 0)?;
    Some(len)
}

fn probe_p50a(d: &[u8]) -> Option<()> {
    let p = player(d)?;
    ok(!p.packed)?;
    let len = player_plausible(&p, true)?;
    let d = p.d;
    let mut i = p.heads_at + p.count * 6 + p.patterns * 8 + len + 1;
    while i < p.samples_at {
        let x = b(d, i)?;
        if x & 0x80 == 0 {
            ok(x <= 0x49 && (((x << 4) & 0x10) | (b(d, i + 1)? >> 4)) as usize <= p.count)?;
            i += 3;
        } else {
            i += 4;
        }
    }
    Some(())
}

fn probe_p60a(d: &[u8]) -> Option<()> {
    let p = player(d)?;
    let len = player_plausible(&p, false)?;
    // 6.0A's notes, as 5.0A's.
    let d = p.d;
    let mut i = p.heads_at + p.count * 6 + p.patterns * 8 + len + 1;
    while i < p.samples_at {
        let x = b(d, i)?;
        if x & 0x80 == 0 {
            ok(x <= 0x49 && (((x << 4) & 0x10) | (b(d, i + 1)? >> 4)) as usize <= p.count)?;
            i += 3;
        } else {
            i += 4;
        }
    }
    Some(())
}

fn probe_p61a(d: &[u8]) -> Option<()> {
    let p = player(d)?;
    let len = player_plausible(&p, false)?;
    let d = p.d;
    let mut n = p.heads_at + p.count * 6 + p.patterns * 8 + len + 1;
    while n + 1 < p.samples_at {
        let (x, y) = (b(d, n)?, b(d, n + 1)?);
        let ins = || usize::from(((x << 4) & 0x10) | (y >> 4));
        n += match x {
            0xff => match y & 0xc0 {
                0x00 => 2,
                0x40 => 3,
                0xc0 => 4,
                _ => 1,
            },
            0x7f => 1,
            _ if x & 0xf0 == 0xf0 => {
                ok(usize::from(y & 0x1f) <= p.count)?;
                3
            }
            _ if x & 0xf0 == 0x70 => {
                ok(usize::from(y & 0x1f) <= p.count)?;
                2
            }
            _ if x & 0xf0 == 0xe0 => 3,
            _ if x & 0xf0 == 0x60 => 2,
            _ if x & 0x80 != 0 => {
                ok(ins() <= p.count)?;
                4
            }
            _ => {
                ok(ins() <= p.count)?;
                3
            }
        };
    }
    Some(())
}

/// The Player's sample headers and samples: a length past 0xffdf names
/// an earlier sample whose data it shares; a sample's finetune has its
/// packing in the top bit (4-bit deltas through a table); delta samples
/// are stored as the differences from one byte to the next.
fn player_samples(pt: &mut Pt, p: &Player) -> Option<()> {
    let d = p.d;
    let mut at = Vec::with_capacity(p.count);
    let mut sizes: Vec<(usize, bool)> = Vec::with_capacity(p.count);
    let mut next = 0;
    for i in 0..p.count {
        let h = d.get(p.heads_at + i * 6..p.heads_at + i * 6 + 6)?;
        let size = w(h, 0)?;
        let packed = p.packed && h[2] & 0x80 != 0;
        let (start, len, packed) = if size > 0xff00 {
            let j = 0xffff - size;
            ok(j < i)?;
            (at[j], sizes[j].0, sizes[j].1)
        } else {
            let s = next;
            next += if packed { size } else { size * 2 };
            (s, size, packed)
        };
        at.push(start);
        sizes.push((len, packed));
        let ls = w(h, 4)?;
        let (ls, ll) = if ls == 0xffff { (0, 1) } else { (ls, len.saturating_sub(ls)) };
        pt.head(len, h[2] & 0x0f, h[3], ls, ll);
    }
    const NIBBLES: [i8; 16] = [0, 1, 2, 4, 8, 16, 32, 64, -128, -64, -32, -16, -8, -4, -2, -1];
    for (i, &(len, packed)) in sizes.iter().enumerate() {
        let from = p.samples_at + at[i];
        if packed {
            let mut s = 0i8;
            for &x in upto(d, from, len) {
                for n in [x >> 4, x & 0x0f] {
                    s = s.wrapping_sub(NIBBLES[usize::from(n)]);
                    pt.samples.push(s as u8);
                }
            }
        } else if p.delta {
            let raw = upto(d, from, len * 2);
            let mut s = 0u8;
            for (j, &x) in raw.iter().enumerate() {
                s = if j == 0 { x } else { s.wrapping_sub(x) };
                pt.samples.push(s);
            }
        } else {
            pt.samples.extend_from_slice(upto(d, from, len * 2));
        }
    }
    Some(())
}

/// The Player's note byte (doubled, the sample's high bit below it) and
/// effect bytes, as a cell.
fn player_cell(note: u8, c2: u8, c3: u8) -> [u8; 4] {
    let [hi, lo] = period(usize::from(note / 2)).unwrap_or([0, 0]);
    let fx = c2 & 0x0f;
    [((note << 4) & 0x10) | hi, lo, if fx == 8 { c2 - 8 } else { c2 }, signed_slide(fx, c3)]
}

fn player_orders(pt: &mut Pt, p: &Player, halved: bool) -> Option<(usize, Vec<[usize; 4]>)> {
    let tracks_at = p.heads_at + p.count * 6;
    let mut tracks = Vec::with_capacity(p.patterns);
    for i in 0..p.patterns {
        let at = tracks_at + i * 8;
        tracks.push([w(p.d, at)?, w(p.d, at + 2)?, w(p.d, at + 4)?, w(p.d, at + 6)?]);
    }
    let orders_at = tracks_at + p.patterns * 8;
    loop {
        let o = b(p.d, orders_at + pt.len)?;
        if o == 0xff || pt.len == 128 {
            break;
        }
        pt.orders[pt.len] = if halved { o / 2 } else { o };
        pt.len += 1;
    }
    Some((orders_at + pt.len + 1, tracks))
}

fn unpack_p50a(d: &[u8]) -> Option<Pt> {
    unpack_player5(d, true)
}

fn unpack_p60a(d: &[u8]) -> Option<Pt> {
    unpack_player5(d, false)
}

/// 5.0A and 6.0A: an event of 3 bytes (a note over 0x80 counted down from
/// 0xff, then the rows after it to skip or repeat), or 0x80 and a count
/// of earlier events to play again, and how far back they are.
fn unpack_player5(d: &[u8], halved: bool) -> Option<Pt> {
    let p = player(d)?;
    ok(p.patterns <= 128 && p.count <= 31)?;
    let mut pt = Pt::new();
    player_samples(&mut pt, &p)?;
    let (base, tracks) = player_orders(&mut pt, &p, halved)?;
    let d = p.d;
    for t in tracks {
        let mut pat = [0; 1024];
        // A break or a jump ends the pattern: the channels after it are
        // read no further than its row.
        let mut rows = 64;
        for (ch, &off) in t.iter().enumerate() {
            let mut c = Cur::at(d, base + off);
            let mut row = 0;
            // One event; false when it ends the track (a break or a jump).
            let event = |c: &mut Cur, pat: &mut [u8; 1024], row: &mut usize| -> Option<bool> {
                let (c1, c2, c3) = (c.u8()?, c.u8()?, c.u8()?);
                if c1 & 0x80 != 0 && c1 != 0x80 {
                    let c4 = c.u8()?;
                    let cell = player_cell(0xff - c1, c2, c3);
                    Pt::put(pat, *row, ch, cell);
                    *row += 1;
                    if matches!(c2 & 0x0f, 0x0b | 0x0d) {
                        return Some(false);
                    }
                    if c4 < 0x80 {
                        *row += usize::from(c4);
                    } else {
                        for _ in 0..0x100 - usize::from(c4) {
                            Pt::put(pat, *row, ch, cell);
                            *row += 1;
                        }
                    }
                    return Some(true);
                }
                Pt::put(pat, *row, ch, player_cell(c1, c2, c3));
                *row += 1;
                Some(!matches!(c2 & 0x0f, 0x0b | 0x0d))
            };
            'track: while row < rows {
                if b(d, c.p)? != 0x80 {
                    if !event(&mut c, &mut pat, &mut row)? {
                        rows = row;
                        break;
                    }
                    continue;
                }
                let (lines, back) = (b(d, c.p + 1)?, w(d, c.p + 2)?);
                c.p += 4;
                let mut r = Cur::at(d, c.p.checked_sub(back)?);
                for _ in 0..=lines {
                    if row >= 64 {
                        break;
                    }
                    if !event(&mut r, &mut pat, &mut row)? {
                        rows = row;
                        break 'track;
                    }
                }
            }
        }
        pt.patterns.push(pat);
    }
    Some(pt)
}

/// 6.1A: an event of 1 to 4 bytes by its top bits (a note and sample
/// only, an effect only, all three, an empty row), most followed by the
/// rows after it to skip or repeat; or 0xff and a count of earlier events
/// to play again, and how far back they are.
fn unpack_p61a(d: &[u8]) -> Option<Pt> {
    let p = player(d)?;
    ok(p.patterns < 128 && p.count <= 31)?;
    let mut pt = Pt::new();
    player_samples(&mut pt, &p)?;
    let (base, tracks) = player_orders(&mut pt, &p, false)?;
    let d = p.d;
    for t in tracks {
        let mut pat = [0; 1024];
        // A break or a jump ends the pattern, as in 5.0A.
        let mut rows = 64;
        for (ch, &off) in t.iter().enumerate() {
            let mut c = Cur::at(d, base + off);
            let mut row = 0;
            // One event; Some(false) when it ends the track. `nested`: no
            // copies within a copy.
            fn event(c: &mut Cur, pat: &mut [u8; 1024], row: &mut usize, ch: usize, nested: bool) -> Option<bool> {
                let c1 = c.u8()?;
                // Repeats the cell `n` more times, or skips n rows.
                let follow = |c: &mut Cur, pat: &mut [u8; 1024], row: &mut usize, cell: [u8; 4], keep: [bool; 4]| -> Option<()> {
                    let n = c.u8()?;
                    if n < 0x80 {
                        *row += usize::from(n);
                    } else {
                        for _ in 0..n - 0x80 {
                            if *row >= 64 {
                                break;
                            }
                            let mut x = if *row < 64 { pat[*row * 16 + ch * 4..][..4].try_into().unwrap_or([0; 4]) } else { [0; 4] };
                            for i in 0..4 {
                                if keep[i] {
                                    x[i] = cell[i];
                                }
                            }
                            Pt::put(pat, *row, ch, x);
                            *row += 1;
                        }
                    }
                    Some(())
                };
                if c1 & 0x70 == 0x70 && c1 != 0xff && c1 != 0x7f {
                    // A note and sample, no effect.
                    let c2 = c.u8()?;
                    let n = ((c1 << 4) & 0xf0) | ((c2 >> 4) & 0x0e);
                    let [hi, lo] = period(usize::from(n / 2))?;
                    let cell = [(c2 & 0x10) | hi, lo, c2 << 4, 0];
                    Pt::put(pat, *row, ch, cell);
                    *row += 1;
                    if c1 & 0x80 != 0 {
                        follow(c, pat, row, cell, [true, true, true, false])?;
                    }
                    return Some(true);
                }
                if c1 & 0x70 == 0x60 && c1 != 0xff {
                    // An effect only.
                    let c2 = c.u8()?;
                    let fx = c1 & 0x0f;
                    let cell = [0, 0, if fx == 8 { 0 } else { fx }, signed_slide(fx, c2)];
                    Pt::put(pat, *row, ch, cell);
                    *row += 1;
                    if matches!(fx, 0x0b | 0x0d) {
                        return Some(false);
                    }
                    if c1 & 0x80 != 0 {
                        follow(c, pat, row, cell, [false, false, true, true])?;
                    }
                    return Some(true);
                }
                if c1 & 0x80 != 0 && c1 != 0xff {
                    // Everything, and the rows after it.
                    let (c2, c3) = (c.u8()?, c.u8()?);
                    let note = c1 & 0x7f;
                    period(usize::from(note / 2))?;
                    let cell = player_cell(note, c2, c3);
                    Pt::put(pat, *row, ch, cell);
                    *row += 1;
                    if matches!(c2 & 0x0f, 0x0b | 0x0d) {
                        c.u8()?;
                        return Some(false);
                    }
                    follow(c, pat, row, cell, [true; 4])?;
                    return Some(true);
                }
                if c1 & 0x7f == 0x7f {
                    if c1 & 0x80 == 0 {
                        // One empty row.
                        *row += 1;
                        return Some(true);
                    }
                    let c2 = c.u8()?;
                    if c2 < 0x40 {
                        // Empty rows.
                        *row += usize::from(c2) + 1;
                        return Some(true);
                    }
                    if nested {
                        *row += 1;
                        return Some(true);
                    }
                    // Earlier events again.
                    let count = c2 & 0x3f;
                    let back = if c2 & 0x80 != 0 { w(c.d, c.p)? } else { usize::from(b(c.d, c.p)?) };
                    let size = if c2 & 0x80 != 0 { 2 } else { 1 };
                    c.p += size;
                    let mut r = Cur::at(c.d, c.p.checked_sub(back)?);
                    for _ in 0..=count {
                        if *row >= 64 {
                            break;
                        }
                        if !event(&mut r, pat, row, ch, true)? {
                            return Some(false);
                        }
                    }
                    return Some(true);
                }
                // A note, sample and effect: 3 bytes.
                let (c2, c3) = (c.u8()?, c.u8()?);
                period(usize::from(c1 / 2))?;
                let cell = player_cell(c1, c2, c3);
                Pt::put(pat, *row, ch, cell);
                *row += 1;
                Some(!matches!(c2 & 0x0f, 0x0b | 0x0d))
            }
            while row < rows {
                if !event(&mut c, &mut pat, &mut row, ch, false)? {
                    rows = row;
                    break;
                }
            }
        }
        pt.patterns.push(pat);
    }
    Some(pt)
}

// --- NoiseRunner ---------------------------------------------------------

fn probe_nru(d: &[u8]) -> Option<()> {
    ok(d.get(1080..1084)? == b"M.K.")?;
    let total: usize = (0..31).map(|i| w(d, 6 + i * 16).unwrap_or(0) * 2).sum();
    ok(total != 0)?;
    ok((0..31).all(|i| d[1 + i * 16] <= 0x40 && l(d, 8 + i * 16) >= l(d, 2 + i * 16)))?;
    // Each sample's record starts with a zero, and its finetune is one of its table's.
    ok((0..31).all(|i| d[i * 16] == 0 && w(d, 14 + i * 16).is_some_and(|f| NRU_FINES.contains(&(f as u16)))))?;
    let len = usize::from(d[950]);
    let orders = &d[952..1080];
    ok((1..=127).contains(&len) && orders[..len].iter().all(|&o| o <= 127) && orders[len..].iter().all(|&o| o == 0))?;
    let cells = (usize::from(orders.iter().copied().max().unwrap_or(0)) + 1) * 256;
    for i in 0..cells {
        let x = d.get(1084 + i * 4..1088 + i * 4)?;
        ok(x[2] <= 0x48 && x[3] & 0x07 == 0 && x[0] & 0x03 == 0)?;
    }
    ok(!mod_like(d, 1084, cells))
}

/// NoiseRunner's finetunes, as the period steps they make.
const NRU_FINES: [u16; 16] = [0x0000, 0xffb8, 0xff70, 0xff28, 0xfee0, 0xfe98, 0xfe50, 0xfe08, 0xfdc0, 0xfd78, 0xfd30, 0xfce8, 0xfca0, 0xfc58, 0xfc10, 0xfbc8];

fn unpack_nru(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    for i in 0..31 {
        let h = d.get(i * 16..i * 16 + 16)?;
        let (at, len, loop_at, ll, fine) = (l(h, 2)?, w(h, 6)?, l(h, 8)?, w(h, 12)?, w(h, 14)? as u16);
        let ft = NRU_FINES.iter().position(|&f| f == fine).unwrap_or(0) as u8;
        pt.head(len, ft, h[1], loop_at.wrapping_sub(at) / 2, ll);
    }
    pt.len = usize::from(d[950]);
    pt.restart = d[951];
    pt.orders.copy_from_slice(d.get(952..1080)?);
    let count = usize::from(pt.orders.iter().copied().max().unwrap_or(0)) + 1;
    let mut c = Cur::at(d, 1084);
    for _ in 0..count {
        let mut p = [0; 1024];
        for i in 0..256 {
            let x = c.take(4)?;
            let fx = match x[0] {
                0x00 => 3,
                0x0c => 0,
                f => f >> 2,
            };
            p[i * 4..][..4].copy_from_slice(&cell_or_none(usize::from(x[2] / 2), (x[3] >> 3) & 0x1f, fx, x[1]));
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, c.p, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- NoisePacker 1 to 3 --------------------------------------------------

/// NoisePacker's header: the samples' count, the order list's size, the
/// track data's; its sample headers' layout differs by version.
fn np_plausible(d: &[u8], version: u8) -> Option<()> {
    let table = w(d, 2)?;
    ok(table != 0 && table % 2 == 0 && table <= 0xff && b(d, 1)? & 0x0f == 0x0c)?;
    let n = usize::from(((d[0] << 4) & 0xf0) | (d[1] >> 4));
    ok((1..=0x1f).contains(&n))?;
    let vol_at = if version == 3 { 9 } else { 15 };
    ok((0..n).all(|i| d.get(vol_at + i * 16).is_some_and(|&v| v <= 0x40)))?;
    let mut total = 0;
    for i in 0..n {
        let h = d.get(i * 16..i * 16 + 24)?;
        let len = w(h, if version == 3 { 14 } else { 12 })? * 2;
        let start = w(h, 20)? * 2;
        let lsize = if version == 1 { w(h, 22)? } else { w(h, 22)? * 2 };
        ok(start + lsize <= len + 2 && !(start == 0 && lsize != 0))?;
        total += len;
    }
    ok(total > 4)?;
    let mut at = n * 16 + 12;
    let mut most = 0;
    for i in (0..table).step_by(2) {
        let p = w(d, at + i)?;
        ok(p % 8 == 0 && (if version == 2 { p <= 0x400 } else { p < 0x400 }))?;
        most = most.max(p);
    }
    at += table + most + 8;
    let tracks = w(d, 6)?;
    if version == 3 {
        ok(tracks > 63)?;
        let mut errors = 0;
        let mut i = 0;
        while i < tracks {
            let x = d.get(at + i..at + i + 3)?;
            if x[0] & 0x80 != 0 {
                i += 1;
                continue;
            }
            errors += usize::from(x[0] > 0x49 || x[1] & 0x0f == 0x0a);
            errors += usize::from(x[1] & 0x0f == 0x0d && x[2] > 0x40);
            errors += usize::from(usize::from(((x[0] << 4) & 0x10) | (x[1] >> 4)) > n);
            errors += usize::from(x == [0, 0, 0] && i + 3 < tracks);
            ok(errors <= 1)?;
            i += 3;
        }
        return Some(());
    }
    ok(tracks >= 192 && tracks % 64 == 0)?;
    for i in (0..tracks).step_by(3) {
        let x = d.get(at + i..at + i + 3)?;
        ok(x[0] <= 0x49)?;
        if version == 2 {
            ok(usize::from(((x[0] << 4) & 0x10) | (x[1] >> 4)) <= n && !(x[1] & 0x0f == 0 && x[2] != 0))?;
        }
    }
    Some(())
}

fn probe_np1(d: &[u8]) -> Option<()> {
    np_plausible(d, 1)
}

fn probe_np2(d: &[u8]) -> Option<()> {
    np_plausible(d, 2)
}

fn probe_np3(d: &[u8]) -> Option<()> {
    np_plausible(d, 3)
}

fn unpack_np1(d: &[u8]) -> Option<Pt> {
    unpack_np(d, 1)
}

fn unpack_np2(d: &[u8]) -> Option<Pt> {
    unpack_np(d, 2)
}

fn unpack_np3(d: &[u8]) -> Option<Pt> {
    unpack_np(d, 3)
}

fn unpack_np(d: &[u8], version: u8) -> Option<Pt> {
    let mut pt = Pt::new();
    let n = usize::from(((b(d, 0)? << 4) & 0xf0) | (b(d, 1)? >> 4));
    pt.len = w(d, 2)? / 2;
    ok(pt.len <= 128)?;
    for i in 0..n {
        let h = d.get(8 + i * 16..24 + i * 16)?;
        if version == 3 {
            pt.head(w(h, 6)?, h[0], h[1], w(h, 14)?, w(h, 12)?);
        } else {
            let ls = if version == 1 { w(h, 14)? / 2 } else { w(h, 14)? };
            pt.head(w(h, 4)?, h[6], h[7], ls, w(h, 12)?);
        }
    }
    let mut c = Cur::at(d, 8 + n * 16 + 4);
    let mut most = 0;
    for i in 0..pt.len {
        let o = (c.w()? / 8) as u8;
        pt.orders[i] = o;
        most = most.max(o);
    }
    let count = usize::from(most) + 1;
    ok(count <= 128)?;
    let mut tracks = Vec::with_capacity(count);
    let mut highest = 0;
    for _ in 0..count {
        let t = [c.w()?, c.w()?, c.w()?, c.w()?];
        highest = highest.max(t.iter().copied().max().unwrap_or(0));
        tracks.push(t);
    }
    let base = c.p;
    let mut end = 0;
    for t in tracks {
        let mut p = [0; 1024];
        for ch in 0..4 {
            // Its channels are stored last first.
            let mut c = Cur::at(d, base + t[3 - ch]);
            let mut row = 0;
            while row < 64 {
                let c1 = c.u8()?;
                if version == 3 && c1 >= 0x80 {
                    row += 0x100 - usize::from(c1);
                    continue;
                }
                let (mut c2, mut c3) = (c.u8()?, c.u8()?);
                let [hi, lo] = period(usize::from((c1 & 0xfe) / 2))?;
                match c2 & 0x0f {
                    0x08 => c2 &= 0xf0,
                    0x07 | 0x06 | 0x05 => {
                        if c2 & 0x0f == 0x07 {
                            c2 = (c2 & 0xf0) | 0x0a;
                        }
                        c3 = if c3 > 0x80 { c3.wrapping_neg() } else { c3 << 4 };
                    }
                    0x0e if version == 2 => c3 = c3.wrapping_sub(1),
                    0x0e if version == 3 => c3 = 1,
                    0x0b => c3 = ((u16::from(c3) + 4) / 2) as u8,
                    _ => {}
                }
                Pt::put(&mut p, row, ch, [((c1 << 4) & 0x10) | hi, lo, c2, c3]);
                row += 1;
                if version == 3 && c2 & 0x0f == 0x0d {
                    break;
                }
            }
            end = end.max(c.p);
        }
        pt.patterns.push(p);
    }
    let samples_at = if version == 3 { end + end % 2 } else { base + highest + 192 };
    pt.samples = upto(d, samples_at, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- Zen Packer ----------------------------------------------------------

fn probe_zen(d: &[u8]) -> Option<()> {
    let at = l(d, 0)?;
    ok((502..=2163190).contains(&at) && d.len() >= 6 + 31 * 16)?;
    for i in 0..31 {
        let h = &d[6 + 16 * i..22 + 16 * i];
        ok(h[3] <= 0x40 && w(h, 0)? % 72 == 0 && l(h, 8)? >= at)?;
    }
    let len = usize::from(d[5]);
    ok((1..=0x7f).contains(&len) && l(d, at + len * 4)? == 0xffff_ffff)
}

fn unpack_zen(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    let table = l(d, 0)?;
    let most = usize::from(b(d, 4)?);
    pt.len = usize::from(b(d, 5)?);
    ok(pt.len < 128 && most < 128)?;
    let mut samples_at = usize::MAX;
    for i in 0..31 {
        let h = d.get(6 + i * 16..22 + i * 16)?;
        let (start, loop_at) = (l(h, 8)?, l(h, 12)?);
        samples_at = samples_at.min(start);
        pt.head(w(h, 4)?, (w(h, 0)? / 0x48) as u8, h[3], loop_at.wrapping_sub(start) / 2, w(h, 6)?);
    }
    let addrs: Vec<usize> = (0..pt.len).map(|i| l(d, table + i * 4)).collect::<Option<_>>()?;
    let (orders, first) = number_by_first(&addrs);
    pt.orders[..orders.len()].copy_from_slice(&orders);
    for i in 0..=most {
        let mut p = [0; 1024];
        // A pattern no position names is where the first one is.
        let mut c = Cur::at(d, first.get(i).map_or(addrs[0], |&f| addrs[f]));
        // Each cell with its place in the pattern; the last is cell 255.
        loop {
            let x = c.take(4)?;
            let ins = ((x[1] << 4) & 0x10) | (x[2] >> 4);
            p[usize::from(x[0]) * 4..][..4].copy_from_slice(&cell(usize::from((x[1] & 0x7f) / 2), ins, x[2] & 0x0f, x[3])?);
            if x[0] == 0xff {
                break;
            }
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, samples_at, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- Heatseeker mc1.0 ----------------------------------------------------

fn probe_crb(d: &[u8]) -> Option<()> {
    ok(d.len() >= 378 && (1..=0x7f).contains(&d[248]) && d[249] == 0x7f)?;
    let mut total = 0;
    for i in 0..31 {
        let h = &d[i * 8..i * 8 + 8];
        let (len, start, lsize) = (w(h, 0)? * 2, w(h, 4)? * 2, w(h, 6)? * 2);
        ok(h[2] <= 0x0f && h[3] <= 0x40 && !(lsize != 0 && lsize != 2 && start + lsize > len) && !(start != 0 && lsize <= 2))?;
        total += len;
    }
    ok(total > 4 && d[250..378].iter().all(|&o| o <= 0x7f))?;
    let count = usize::from(d[250..378].iter().copied().max().unwrap_or(0)) + 1;
    let mut at = 378;
    for _ in 0..count * 4 {
        let mut row = 0;
        while row < 64 {
            let x = d.get(at..at + 4)?;
            match x[0] & 0xc0 {
                0x00 => ok(x[0] & 0x0f <= 0x03)?,
                0x80 => {
                    ok(x[1] == 0)?;
                    row += usize::from(x[3]);
                }
                0xc0 => {
                    ok(x[1] == 0)?;
                    row = 64;
                }
                // Not a cell Heatseeker writes: looked at again for the next row.
                _ => {
                    row += 1;
                    continue;
                }
            }
            at += 4;
            row += 1;
        }
    }
    Some(())
}

fn unpack_crb(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    for i in 0..31 {
        let h = d.get(i * 8..i * 8 + 8)?;
        pt.head(w(h, 0)?, h[2], h[3], w(h, 4)?, w(h, 6)?.max(1));
    }
    pt.len = usize::from(d[248]);
    pt.restart = d[249];
    pt.orders.copy_from_slice(d.get(250..378)?);
    let count = usize::from(pt.orders.iter().copied().max().unwrap_or(0)) + 1;
    let mut c = Cur::at(d, 378);
    let mut starts = Vec::with_capacity(count * 4);
    let u24 = |c: &mut Cur| -> Option<usize> { Some((usize::from(c.u8()?) << 16) | c.w()?) };
    for _ in 0..count {
        let mut p = [0; 1024];
        for ch in 0..4 {
            starts.push(c.p);
            let mut row = 0;
            while row < 64 {
                let x = c.u8()?;
                if x == 0x80 {
                    row += u24(&mut c)? + 1;
                    continue;
                }
                if x == 0xc0 {
                    // An earlier track again, its number (times four) after.
                    let t = u24(&mut c)?;
                    let mut r = Cur::at(d, *starts.get(t >> 2)?);
                    let mut row = 0;
                    while row < 64 {
                        let x = r.u8()?;
                        if x == 0x80 {
                            row += u24(&mut r)? + 1;
                            continue;
                        }
                        Pt::put(&mut p, row, ch, [x, r.u8()?, r.u8()?, r.u8()?]);
                        row += 1;
                    }
                    break;
                }
                Pt::put(&mut p, row, ch, [x, c.u8()?, c.u8()?, c.u8()?]);
                row += 1;
            }
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, c.p, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- The Dark Demon ------------------------------------------------------

fn probe_tdd(d: &[u8]) -> Option<()> {
    ok(d.len() >= 564)?;
    let mut total = 0;
    for i in 0..31 {
        let h = &d[130 + i * 14..144 + i * 14];
        let (at, size, loop_at, ll) = (l(h, 0)?, w(h, 4)? * 2, l(h, 8)?, w(h, 12)?);
        ok(h[7] <= 0x40 && loop_at >= at && at >= 564 && loop_at - at <= size && loop_at - at + ll <= size + 2)?;
        total += size;
    }
    ok(total > 2 && total <= 31 * 65535 && (1..=0x7f).contains(&d[0]))?;
    let orders = &d[2..130];
    ok(orders.iter().all(|&o| o <= 0x7f) && orders[usize::from(d[0])..].iter().all(|&o| o == 0))?;
    let cells = (usize::from(orders.iter().copied().max().unwrap_or(0)) + 1) * 256;
    for i in 0..cells {
        let x = d.get(564 + total + i * 4..568 + total + i * 4)?;
        let (fx, fp) = (x[2] & 0x0f, x[3]);
        ok(x[0] <= 0x1f && x[1] <= 0x48 && x[1] & 1 == 0 && !(fx == 0x0c && fp > 0x40) && !(fx == 0x0d && fp > 0x40) && fx != 0x0b)?;
    }
    Some(())
}

fn unpack_tdd(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    pt.len = usize::from(b(d, 0)?);
    pt.restart = b(d, 1)?;
    pt.orders.copy_from_slice(d.get(2..130)?);
    let mut places = Vec::with_capacity(31);
    for i in 0..31 {
        let h = d.get(130 + i * 14..144 + i * 14)?;
        let (at, len, loop_at) = (l(h, 0)?, w(h, 4)?, l(h, 8)?);
        pt.head(len, h[6], h[7], loop_at.wrapping_sub(at) / 2, w(h, 12)?);
        places.push((at, len * 2));
    }
    let count = usize::from(pt.orders.iter().copied().max().unwrap_or(0)) + 1;
    let mut c = Cur::at(d, 564 + pt.sample_bytes());
    for _ in 0..count {
        let mut p = [0; 1024];
        for i in 0..256 {
            let x = c.take(4)?;
            let [hi, lo] = period(usize::from(x[1] / 2)).unwrap_or([0, 0]);
            p[i * 4..][..4].copy_from_slice(&[(x[0] & 0xf0) | hi, lo, (x[2] & 0x0f) | (x[0] << 4), x[3]]);
        }
        pt.patterns.push(p);
    }
    for (at, len) in places {
        if len != 0 {
            pt.samples.extend_from_slice(upto(d, at, len));
        }
    }
    Some(pt)
}

// --- StarTrekker Packer --------------------------------------------------

fn probe_starpack(d: &[u8]) -> Option<()> {
    let size = w(d, 268)?;
    ok(size % 4 == 0 && (1..=127).contains(&(size / 4)) && b(d, 784)? == 0)?;
    let len = size / 4;
    for i in 0..31 {
        let h = &d[20 + i * 8..28 + i * 8];
        ok((w(h, 4)? + w(h, 6)?) * 2 <= w(h, 0)? * 2 + 2 && h[2] <= 0x0f && h[3] <= 0x40)?;
    }
    let samples = l(d, 784)?;
    ok(samples >= 788)?;
    ok((0..len).all(|i| l(d, 272 + i * 4).is_some_and(|a| a <= samples)) && (len..128).all(|i| l(d, 272 + i * 4) == Some(0)))?;
    let mut at = 788;
    while at < samples + 4 {
        let x = d.get(at..at + 4)?;
        if x[0] == 0x80 {
            at += 1;
            continue;
        }
        ok(x[0] < 0x80 && x != [0; 4])?;
        at += 4;
    }
    Some(())
}

fn unpack_starpack(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    pt.title = d.get(..20)?.to_vec();
    for i in 0..31 {
        pt.raw_head(d.get(20 + i * 8..28 + i * 8)?);
    }
    pt.len = w(d, 268)? / 4;
    ok(pt.len < 128)?;
    let addrs: Vec<usize> = (0..128).map(|i| l(d, 272 + i * 4)).collect::<Option<_>>()?;
    let orders = numbered_with_gaps(&addrs, pt.len, 128);
    pt.orders[..pt.len].copy_from_slice(&orders[..pt.len]);
    let count = usize::from(orders[..pt.len].iter().copied().max().unwrap_or(0)) + 1;
    let samples = l(d, 0x310)? + 0x314;
    let mut c = Cur::at(d, 788);
    for _ in 0..count {
        let mut p = [0; 1024];
        for i in 0..256 {
            let c1 = c.u8()?;
            if c1 == 0x80 {
                continue;
            }
            let (c2, c3, c4) = (c.u8()?, c.u8()?, c.u8()?);
            // Its sample numbers are times four.
            let ins = ((c1 & 0xf0) | (c3 >> 4)) >> 2;
            p[i * 4..][..4].copy_from_slice(&[(c1 & 0x0f) | (ins & 0xf0), c2, (c3 & 0x0f) | (ins << 4), c4]);
        }
        pt.patterns.push(p);
    }
    pt.samples = upto(d, samples, pt.sample_bytes()).to_vec();
    Some(pt)
}

// --- STIM (Slam Tilt) ----------------------------------------------------

// "STIM", the samples' table's address, the sample count, the song's
// length and the patterns' count, 128 orders, 64 patterns' addresses
// (from 12): each pattern four tracks' offsets, then the tracks, a cell
// three bytes (sample, then the effect's high bits and the note, then
// its value) or a byte with bit 7 for empty rows. The samples' table
// holds their offsets, each sample an 8-byte MOD header and its data.

fn probe_stim(d: &[u8]) -> Option<()> {
    ok(d.get(..4)? == b"STIM" && d.len() >= 406 && l(d, 4)? >= 406)?;
    let (nos, len, npat) = (w(d, 16)?, w(d, 18)?, w(d, 20)?);
    ok(nos <= 31 && len <= 128 && (1..=64).contains(&npat) && d[22..150].iter().all(|&o| usize::from(o) <= npat))
}

fn unpack_stim(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    let table = l(d, 4)?;
    let (nos, len, npat) = (w(d, 16)?, w(d, 18)?, w(d, 20)?);
    pt.len = len;
    pt.orders.copy_from_slice(d.get(22..150)?);
    for i in 0..npat {
        let at = l(d, 150 + i * 4)? + 12;
        let mut p = [0; 1024];
        for ch in 0..4 {
            let mut c = Cur::at(d, at + w(d, at + ch * 2)?);
            let mut row = 0;
            while row < 64 {
                let c1 = c.u8()?;
                if c1 & 0x80 != 0 {
                    row += usize::from(c1 & 0x7f) + 1;
                    continue;
                }
                let (c2, c3) = (c.u8()?, c.u8()?);
                // Note 1 is none; 2 is C-1.
                let note = usize::from(c2 & 0x3f).saturating_sub(1);
                Pt::put(&mut p, row, ch, cell_or_none(note, c1 & 0x1f, (c2 >> 4) & 0x0c | (c1 >> 5) & 0x03, c3));
                row += 1;
            }
        }
        pt.patterns.push(p);
    }
    for i in 0..nos {
        let at = table + l(d, table + i * 4)?;
        let h = d.get(at..at + 8)?;
        pt.raw_head(h);
        pt.samples.extend_from_slice(upto(d, at + 8, w(h, 0)? * 2));
        // A sample cut short is filled out, for the ones after it.
        pt.samples.resize(pt.sample_bytes(), 0);
    }
    Some(pt)
}

// --- Titanics Player -----------------------------------------------------

fn probe_titanics(d: &[u8]) -> Option<()> {
    let mut total = 0;
    for i in 0..15 {
        let h = d.get(i * 12..i * 12 + 12)?;
        let at = l(h, 0)?;
        let (len, start, lsize) = (w(h, 4)?, w(h, 8)?, w(h, 10)?);
        ok(h[7] <= 0x40 && h[6] == 0 && (at == 0 || at >= 180))?;
        ok(start <= len && lsize <= len + 1 && len <= 32768 && lsize != 0 && !(len == 0 && (start != 0 || lsize != 1)))?;
        total += len;
    }
    ok(total >= 2)?;
    for i in 0..128 {
        let a = w(d, 180 + i * 2)?;
        if a == 0xffff {
            return Some(());
        }
        ok(a >= 180)?;
    }
    None
}

fn unpack_titanics(d: &[u8]) -> Option<Pt> {
    let mut pt = Pt::new();
    let mut places = Vec::with_capacity(15);
    for i in 0..15 {
        let h = d.get(i * 12..i * 12 + 12)?;
        places.push((l(h, 0)?, w(h, 4)? * 2));
        pt.raw_head(&h[4..12]);
    }
    for _ in 15..31 {
        pt.head(0, 0, 0x40, 0, 1);
    }
    let mut addrs = Vec::new();
    while addrs.len() < 128 {
        let at = 180 + addrs.len() * 2;
        if b(d, at)? == 0xff {
            break;
        }
        addrs.push(w(d, at)?);
    }
    pt.len = addrs.len();
    let mut stored = addrs.clone();
    stored.sort_unstable();
    stored.dedup();
    for (o, a) in pt.orders.iter_mut().zip(&addrs) {
        *o = stored.iter().position(|s| s == a)? as u8;
    }
    for &a in &stored {
        let mut p = [0; 1024];
        let mut c = Cur::at(d, a);
        let mut x = c.u8()?;
        let mut row = 0;
        // Events: a channel and note, the sample and effect; a byte
        // before each next one gives the rows to it (and its top bit ends
        // the pattern).
        while row < 64 {
            let y = c.u8()?;
            let [hi, lo] = period(usize::from(y & 0x3f)).unwrap_or([0, 0]);
            Pt::put(&mut p, row, usize::from(y >> 6), [hi, lo, c.u8()?, c.u8()?]);
            if x & 0x80 != 0 {
                break;
            }
            x = c.u8()?;
            row += usize::from(x & 0x7f);
        }
        pt.patterns.push(p);
    }
    for (at, len) in places {
        if at != 0 {
            pt.samples.extend_from_slice(upto(d, at, len));
        }
    }
    Some(pt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Fx;

    #[test]
    fn periods_are_protrackers() {
        assert_eq!(period(1), Some(856u16.to_be_bytes()));
        // F#-2 is 339 in ProTracker's table, not the 340 its tuning gives.
        assert_eq!(period(17), Some(339u16.to_be_bytes()));
        assert_eq!(period(37), None);
        assert_eq!(untune(850, 1), Some(1));
        assert_eq!(untune(907, 8), Some(1));
    }

    #[test]
    fn player_61a_repeats_copies_and_breaks() {
        let mut d = vec![0u8; 33 + 4];
        d[..2].copy_from_slice(&33u16.to_be_bytes());
        d[2] = 1;
        d[3] = 1;
        // One sample of 2 words, volume 64, no loop.
        d[4..10].copy_from_slice(&[0, 2, 0, 0x40, 0xff, 0xff]);
        for (i, off) in [0u16, 6, 11, 11].iter().enumerate() {
            d[10 + i * 2..12 + i * 2].copy_from_slice(&off.to_be_bytes());
        }
        d[18..20].copy_from_slice(&[0, 0xff]);
        // C-2, sample 1, volume 32, two more times; then 61 empty rows.
        d[20..26].copy_from_slice(&[0x80 | 26, 0x1c, 0x20, 0x82, 0xff, 0x3c]);
        // That event again (from 9 bytes back), then 61 empty rows.
        d[26..31].copy_from_slice(&[0xff, 0x40, 9, 0xff, 0x3c]);
        // A pattern break alone.
        d[31..33].copy_from_slice(&[0x6d, 0]);
        d[33..].copy_from_slice(&[1, 2, 3, 4]);
        let p = find(&d).unwrap();
        assert_eq!(p.id, "p61a");
        let m = load(&d, p).unwrap();
        assert_eq!((m.format.as_str(), m.format_name.as_str()), ("packed", "The Player 6.1A"));
        let pat = &m.patterns[0];
        for ch in 0..2 {
            for row in 0..3 {
                let c = pat.cell(row, ch);
                assert_eq!((c.note, c.instr, c.fx, c.param), (61, 1, Fx::Volume, 32), "row {row} channel {ch}");
            }
            assert_eq!(pat.cell(3, ch).note, 0);
        }
        assert_eq!(pat.cell(0, 2).fx, Fx::PatternBreak);
        assert_eq!(m.samples[0].frames(), 4);
    }

    #[test]
    fn propacker_21_tracks_reference_a_table_of_cells() {
        let mut d = vec![0u8; 762 + 128 + 4 + 8 + 4];
        d[..8].copy_from_slice(&[0, 2, 0, 0x40, 0, 0, 0, 1]);
        d[248] = 1;
        d[249] = 0x7f;
        // Every channel plays track 0; its first row is the table's cell 1.
        d[762..764].copy_from_slice(&1u16.to_be_bytes());
        d[890..894].copy_from_slice(&8u32.to_be_bytes());
        d[898..902].copy_from_slice(&[0x01, 0xac, 0x1c, 0x20]);
        let p = find(&d).unwrap();
        assert_eq!(p.id, "pp21");
        let m = load(&d, p).unwrap();
        for ch in 0..4 {
            let c = m.patterns[0].cell(0, ch);
            assert_eq!((c.note, c.instr, c.fx, c.param), (61, 1, Fx::Volume, 32));
            assert_eq!(m.patterns[0].cell(1, ch).note, 0);
        }
        assert_eq!(m.samples[0].frames(), 4);
    }
}
