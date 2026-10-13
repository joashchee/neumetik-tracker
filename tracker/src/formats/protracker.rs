//! The Amiga's MOD and the trackers that kept it: Ultimate SoundTracker
//! and the 15-sample SoundTrackers, NoiseTracker, ProTracker (M.K.,
//! M!K!), StarTrekker (FLT4, FLT8), FastTracker and TakeTracker's
//! channel counts (6CHN, 12CH, 32CN...), Oktalyser on the Atari (CD81,
//! OKTA, OCTA), Digital Tracker (FA04 to FA08), TakeTracker's TDZx,
//! and Mod's Grave (.wow).
//!
//! 20 bytes of title, 31 (or 15) sample headers of 30 bytes, the order
//! list, a tag at 1080 naming the variant, patterns of 64 rows of 4-byte
//! cells (a period, a sample, an effect), then 8-bit signed samples.

use std::sync::Arc;

use super::common::{amiga_pan, finetune_c5, mod_effect, period_to_note, plausible_text};
use crate::io::{bad, decode, Pcm, Reader, Result};
use crate::model::{Cell, Channel, Flags, Kind, Loop, Module, Pattern, Sample, Wave};

/// The variant a tag names: channels, and the tracker.
fn variant(tag: &[u8; 4]) -> Option<(usize, &'static str)> {
    let t = tag;
    Some(match t {
        // PATT: ProTracker 3.6's, inside its IFF.
        b"M.K." | b"M!K!" | b"M&K!" | b"PATT" => (4, "ProTracker"),
        b"N.T." => (4, "NoiseTracker"),
        b"FEST" | b"LARD" | b"NSMS" => (4, "NoiseTracker"),
        b"FLT4" | b"EXO4" => (4, "StarTrekker"),
        b"FLT8" | b"EXO8" => (8, "StarTrekker"),
        b"CD81" | b"OKTA" | b"OCTA" => (8, "Oktalyser"),
        b"CD61" => (6, "Oktalyser"),
        _ => {
            let d = |c: u8| c.is_ascii_digit().then(|| usize::from(c - b'0'));
            if &t[1..] == b"CHN" {
                return d(t[0]).filter(|&n| n > 0).map(|n| (n, "FastTracker"));
            }
            if &t[2..] == b"CH" || &t[2..] == b"CN" {
                return Some((d(t[0])? * 10 + d(t[1])?, if &t[2..] == b"CN" { "TakeTracker" } else { "FastTracker" })).filter(|&(n, _)| (1..=99).contains(&n));
            }
            if &t[..3] == b"TDZ" {
                return d(t[3]).filter(|&n| n > 0).map(|n| (n, "TakeTracker"));
            }
            if &t[..2] == b"FA" && t[2] == b'0' {
                return d(t[3]).filter(|&n| n > 0).map(|n| (n, "Digital Tracker"));
            }
            return None;
        }
    })
}

/// A 31-sample MOD (any tag above), or a 15-sample SoundTracker when
/// `fifteen` and it passes the checks that stand in for a tag.
pub fn probe(data: &[u8]) -> bool {
    data.len() >= 1084 && data[1080..1084].try_into().ok().and_then(|t: [u8; 4]| variant(&t)).is_some()
}

pub fn probe_15(data: &[u8]) -> bool {
    let mut r = Reader::new(data);
    if data.len() < 600 + 1024 || !plausible_text(&data[..20]) {
        return false;
    }
    r.pos = 20;
    let mut total = 0usize;
    let mut any = false;
    for _ in 0..15 {
        let Ok(h) = r.bytes(30) else { return false };
        if !plausible_text(&h[..22]) {
            return false;
        }
        let len = usize::from(u16::from_be_bytes([h[22], h[23]])) * 2;
        if h[25] > 64 || len > 65_535 * 2 {
            return false;
        }
        if h[24] & 0xF0 != 0 {
            return false;
        }
        total += len;
        any |= len > 0;
    }
    let (Ok(songlen), Ok(_)) = (r.u8(), r.u8()) else { return false };
    let Ok(orders) = r.bytes(128) else { return false };
    if songlen == 0 || songlen > 128 || !any {
        return false;
    }
    let most = orders.iter().copied().max().unwrap_or(0);
    if most >= 64 || orders[usize::from(songlen)..].iter().any(|&o| o > most) {
        return false;
    }
    let patterns = usize::from(most) + 1;
    // The file holds the patterns, and most of the samples.
    let need = 600 + patterns * 1024;
    data.len() >= need && data.len() + 1024 >= need + total / 2
}

pub fn load(data: &[u8], wow: bool) -> Result<Module> {
    let fifteen = !probe(data);
    let mut r = Reader::new(data);
    let title = r.text(20)?;
    let count = if fifteen { 15 } else { 31 };
    let mut headers = Vec::with_capacity(count);
    for _ in 0..count {
        let name = r.text(22)?;
        let len = usize::from(r.u16be()?) * 2;
        let finetune = r.u8()?;
        let volume = r.u8()?;
        let ls = usize::from(r.u16be()?) * 2;
        let ll = usize::from(r.u16be()?) * 2;
        headers.push((name, len, finetune, volume, ls, ll));
    }
    let songlen = usize::from(r.u8()?).clamp(1, 128);
    let restart = r.u8()?;
    let orders_raw = r.bytes(128)?.to_vec();
    let (mut channels, mut tracker) = if fifteen {
        (4, "SoundTracker")
    } else {
        let tag: [u8; 4] = r.tag()?;
        variant(&tag).ok_or(crate::io::Error::Unknown)?
    };
    if wow {
        channels = 8;
        tracker = "Mod's Grave";
    }
    let flt8 = !fifteen && &data[1080..1084] == b"FLT8";
    let header_end = r.pos;
    let pattern_bytes = 64 * channels * 4;
    let samples_bytes: usize = headers.iter().map(|h| h.1).sum();

    // Patterns: as many as the orders name; all 128 entries count when
    // the file has room for them (ProTracker counts them all).
    let in_song = orders_raw[..songlen].iter().copied().max().unwrap_or(0);
    let in_all = orders_raw.iter().copied().max().unwrap_or(0);
    let fits = |most: u8| {
        let n = usize::from(most) + 1;
        let n = if flt8 { n.div_ceil(2) * 2 } else { n };
        header_end + n * if flt8 { 64 * 4 * 4 } else { pattern_bytes } <= data.len()
    };
    let most = if in_all > in_song && fits(in_all) && data.len() >= header_end + (usize::from(in_all) + 1) * pattern_bytes + samples_bytes / 2 { in_all } else { in_song };
    let mut stored = usize::from(most) + 1;
    if flt8 {
        stored = stored.div_ceil(2) * 2;
    }
    let stored_bytes = if flt8 { 64 * 4 * 4 } else { pattern_bytes };
    if header_end + stored * stored_bytes > data.len() {
        return bad("its patterns are cut short");
    }

    let read_cell = |b: &[u8]| -> Cell {
        let period = u16::from(b[0] & 0x0F) << 8 | u16::from(b[1]);
        let instr = (b[0] & 0xF0) | (b[2] >> 4);
        let (fx, param) = mod_effect(b[2] & 0x0F, b[3], false);
        Cell { note: period_to_note(period).unwrap_or(0), instr, fx, param, ..Default::default() }
    };

    let mut patterns = Vec::new();
    if flt8 {
        // StarTrekker's eight channels: each pattern is two of four
        // channels, the left four then the right, the orders counting
        // them in twos.
        for p in 0..stored / 2 {
            let mut pat = Pattern::new(64, 8);
            for half in 0..2 {
                let base = header_end + (p * 2 + half) * stored_bytes;
                for row in 0..64 {
                    for c in 0..4 {
                        let at = base + (row * 4 + c) * 4;
                        pat.cells[row * 8 + half * 4 + c] = read_cell(&data[at..at + 4]);
                    }
                }
            }
            patterns.push(Arc::new(pat));
        }
    } else {
        for p in 0..stored {
            let mut pat = Pattern::new(64, channels);
            let base = header_end + p * pattern_bytes;
            for i in 0..64 * channels {
                let at = base + i * 4;
                pat.cells[i] = read_cell(&data[at..at + 4]);
            }
            patterns.push(Arc::new(pat));
        }
    }
    let orders: Vec<u16> = orders_raw[..songlen].iter().map(|&o| if flt8 { u16::from(o / 2) } else { u16::from(o) }).collect();

    let mut at = header_end + stored * stored_bytes;
    let mut samples = Vec::with_capacity(count);
    for (name, len, finetune, volume, mut ls, ll) in headers {
        let bytes = r.data.get(at..).map_or(&[][..], |rest| &rest[..len.min(rest.len())]);
        at += len;
        let pcm = decode(bytes, Pcm::S8, len);
        // Some SoundTrackers wrote the loop start in bytes.
        if fifteen && ls + ll > len && ls / 2 + ll <= len {
            ls /= 2;
        }
        let looped = if ll > 2 { Loop::forward(ls as u32, (ls + ll) as u32) } else { Loop::default() };
        samples.push(Sample {
            name,
            wave: Arc::new(Wave::new(8, 1, pcm)),
            c5: finetune_c5(finetune),
            volume: volume.min(64),
            global_volume: 64,
            looped,
            ..Sample::empty()
        });
    }

    let mut m = Module {
        title,
        format: if wow { "wow" } else if fifteen { "stk" } else { "mod" }.into(),
        format_name: if fifteen { "SoundTracker (15 samples)".into() } else if wow { "Mod's Grave".into() } else { "MOD".into() },
        tracker: tracker.into(),
        kind: Kind::Mod,
        flags: Flags { amiga_limits: channels == 4 && !wow, vblank: fifteen, ..Default::default() },
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        restart: if usize::from(restart) < songlen { u16::from(restart) } else { 0 },
        channels: (0..channels).map(|c| Channel { pan: amiga_pan(c), ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.message = m.samples.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join("\n").trim_end().to_string();
    m.tidy();
    Ok(m)
}

/// Mod's Grave: a MOD tagged M.K. with eight channels, the file's length
/// saying so.
pub fn probe_wow(data: &[u8], name: &str) -> bool {
    if data.len() < 1084 || &data[1080..1084] != b"M.K." || !name.to_ascii_lowercase().ends_with(".wow") {
        return false;
    }
    let songlen = usize::from(data[950]).clamp(1, 128);
    let most = data[952..952 + 128][..songlen].iter().copied().max().unwrap_or(0);
    let samples: usize = (0..31).map(|i| usize::from(u16::from_be_bytes([data[20 + i * 30 + 22], data[20 + i * 30 + 23]])) * 2).sum();
    1084 + (usize::from(most) + 1) * 64 * 8 * 4 + samples == data.len()
}
