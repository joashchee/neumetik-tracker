//! MOD's relatives: Images Music System (.ims), Chiptracker (KRIS),
//! Game Music Creator (.gmc), Quadra Composer (.emod) on the Amiga, and
//! Digital Tracker (.dtm) on the Atari Falcon. Each keeps MOD's samples
//! and most of its effects, and lays the patterns out its own way.

use std::sync::Arc;

use super::common::{amiga_pan, finetune_c5, mod_effect, period_to_note, plausible_text};
use crate::io::{bad, decode, message, Pcm, Reader, Result};
use crate::model::*;

/// A sample from MOD-style fields (lengths in bytes), its PCM taken from
/// `at` on (cut short where the file ends).
fn amiga_sample(d: &[u8], at: usize, name: String, len: usize, finetune: u8, volume: u8, ls: usize, ll: usize) -> Sample {
    let bytes = d.get(at..).map_or(&[][..], |rest| &rest[..len.min(rest.len())]);
    Sample {
        name,
        wave: Arc::new(Wave::new(8, 1, decode(bytes, Pcm::S8, len))),
        c5: finetune_c5(finetune),
        volume: volume.min(64),
        looped: if ll > 2 { Loop::forward(ls as u32, (ls + ll) as u32).clamped(len as u32) } else { Loop::default() },
        ..Sample::empty()
    }
}

/// A four-channel Amiga module's settings, its patterns and samples in.
fn amiga_module(title: String, format: &str, format_name: &str, orders: Vec<u16>, patterns: Vec<Arc<Pattern>>, samples: Vec<Sample>) -> Module {
    let mut m = Module {
        title,
        format: format.into(),
        format_name: format_name.into(),
        tracker: format_name.into(),
        kind: Kind::Mod,
        flags: Flags { amiga_limits: true, ..Default::default() },
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        channels: (0..4).map(|c| Channel { pan: amiga_pan(c), ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.message = m.samples.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join("\n").trim_end().to_string();
    m
}

// ---- Images Music System ----
//
// A MOD with three-byte cells (a note number, not a period) and, where
// MOD has its tag, where the samples start.

pub fn probe_ims(d: &[u8]) -> bool {
    if d.len() < 1084 || !plausible_text(&d[..20]) {
        return false;
    }
    let mut any = false;
    for i in 0..31 {
        let h = &d[20 + i * 30..50 + i * 30];
        let len = u16::from_be_bytes([h[22], h[23]]);
        if !plausible_text(&h[..22]) || len > 0x8000 || h[24] != 0 || h[25] > 64 {
            return false;
        }
        any |= len > 0;
    }
    let (songlen, restart) = (d[950], d[951]);
    let offset = u32::from_be_bytes([d[1080], d[1081], d[1082], d[1083]]) as usize;
    if !any || songlen == 0 || songlen > 128 || restart > songlen || offset <= 1084 || !(offset - 1084).is_multiple_of(768) || offset > d.len() {
        return false;
    }
    let npat = (offset - 1084) / 768;
    npat <= 128 && d[952..1080].iter().all(|&o| usize::from(o) < npat)
}

pub fn load_ims(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    let title = r.text(20)?;
    let mut heads = Vec::new();
    for _ in 0..31 {
        let name = r.text(22)?;
        let len = usize::from(r.u16be()?) * 2;
        let _finetune = r.u8()?;
        let vol = r.u8()?;
        let ls = usize::from(r.u16be()?) * 2;
        let ll = usize::from(r.u16be()?) * 2;
        heads.push((name, len, vol, ls, ll));
    }
    let songlen = usize::from(r.u8()?).clamp(1, 128);
    let restart = r.u8()?;
    let orders: Vec<u16> = r.bytes(128)?[..songlen].iter().map(|&o| u16::from(o)).collect();
    let offset = r.u32be()? as usize;
    let npat = offset.saturating_sub(1084) / 768;
    let mut patterns = Vec::with_capacity(npat);
    for _ in 0..npat {
        let data = r.bytes(768)?;
        let mut p = Pattern::new(64, 4);
        for (i, b) in data.chunks_exact(3).enumerate() {
            // Notes count from ProTracker's C-1.
            let n = b[0] & 0x3F;
            let (fx, param) = mod_effect(b[1] & 0x0F, b[2], false);
            p.cells[i] = Cell { note: if n < 48 { n + 37 } else { 0 }, instr: (b[0] & 0xC0) >> 2 | b[1] >> 4, fx, param, ..Default::default() };
        }
        patterns.push(Arc::new(p));
    }
    let mut at = offset;
    let mut samples = Vec::with_capacity(31);
    for (name, len, vol, ls, ll) in heads {
        samples.push(amiga_sample(d, at, name, len, 0, vol, ls, ll));
        at += len;
    }
    let mut m = amiga_module(title, "ims", "Images Music System", orders, patterns, samples);
    m.restart = u16::from(restart);
    m.tidy();
    Ok(m)
}

// ---- Chiptracker ----
//
// MOD's header (a 22-byte title) tagged KRIS at 952, then four tracks an
// order (each transposed), tracks of 64 four-byte cells.

pub fn probe_kris(d: &[u8]) -> bool {
    d.len() >= 1984 && &d[952..956] == b"KRIS" && d[956] <= 128 && d[957] <= 127 && plausible_text(&d[..22])
}

pub fn load_kris(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    let title = r.text(22)?;
    let mut heads = Vec::new();
    let mut waveforms = 0usize;
    for _ in 0..31 {
        let raw = r.bytes(22)?;
        let len = usize::from(r.u16be()?) * 2;
        let ft = r.u8()?;
        let vol = r.u8()?;
        let ls = usize::from(r.u16be()?) * 2;
        let ll = usize::from(r.u16be()?) * 2;
        if raw[0] == 0 {
            // A synth sound: unfinished in Chiptracker, nothing to play;
            // its waveforms sit before the tracks.
            let most = [raw[1], raw[5], raw[10], raw[19]].into_iter().max().unwrap_or(0);
            if most > 0 {
                waveforms = waveforms.max(usize::from(most) + 1);
            }
            heads.push((String::new(), 0, 0, 0, 0, 0));
        } else {
            heads.push((crate::io::text(raw), len, ft, vol, ls, ll));
        }
    }
    r.seek(956)?;
    let nord = usize::from(r.u8()?);
    let restart = r.u8()?;
    let table = r.bytes(128 * 4 * 2)?;
    let tracks_at = 1984 + waveforms * 64;
    let mut patterns = Vec::with_capacity(nord);
    let mut most = 0usize;
    for o in 0..nord {
        let mut p = Pattern::new(64, 4);
        for c in 0..4 {
            let track = usize::from(table[(o * 4 + c) * 2]);
            let transpose = table[(o * 4 + c) * 2 + 1] as i8;
            most = most.max(track);
            let mut tr = Reader::at(d, tracks_at + track * 256);
            for row in 0..64 {
                let b = tr.bytes(4)?;
                let note = if (0x18..=0x9E).contains(&b[0]) && b[0] & 1 == 0 {
                    // 0x48 is ProTracker's C-1; transposed within its three octaves.
                    (25 + i16::from((b[0] - 0x18) / 2) + i16::from(transpose)).clamp(49, 84) as u8
                } else {
                    0
                };
                let (fx, param) = mod_effect(b[2] & 0x0F, b[3], false);
                p.cells[row * 4 + c] = Cell { note, instr: b[1], fx, param, ..Default::default() };
            }
        }
        patterns.push(Arc::new(p));
    }
    let mut at = tracks_at + (most + 1) * 256;
    let mut samples = Vec::with_capacity(31);
    for (name, len, ft, vol, ls, ll) in heads {
        samples.push(amiga_sample(d, at, name, len, ft, vol, ls, ll));
        at += len;
    }
    let mut m = amiga_module(title, "kris", "Chiptracker", (0..nord as u16).collect(), patterns, samples);
    m.restart = if usize::from(restart) < nord { u16::from(restart) } else { 0 };
    m.tidy();
    Ok(m)
}

// ---- Game Music Creator ----
//
// No title and no tag: 15 sample headers of Amiga addresses, an order
// list of pattern offsets, MOD's cells with effects of its own.

pub fn probe_gmc(d: &[u8]) -> bool {
    if d.len() < 444 + 1024 {
        return false;
    }
    let be16 = |i: usize| u16::from_be_bytes([d[i], d[i + 1]]);
    let be32 = |i: usize| u32::from_be_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
    let mut any = false;
    for s in 0..15 {
        let h = s * 16;
        let (offset, len, address, ll, start) = (be32(h), be16(h + 4), be32(h + 8), be16(h + 12), be16(h + 14));
        if offset > 0x1F_FFFF || offset & 1 != 0 || address > 0x1F_FFFF || address & 1 != 0 || len > 0x7FFF || start > 0x7FFF || start & 1 != 0 {
            return false;
        }
        if (ll > 2 && ll > len) || d[h + 7] > 64 || d[h + 6] != 0 {
            return false;
        }
        any |= len > 0;
    }
    let n = usize::from(d[243]);
    any && d[240..243] == [0, 0, 0] && (1..=100).contains(&n) && (0..100).all(|i| be16(244 + i * 2) % 1024 == 0)
}

pub fn load_gmc(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    let mut heads = Vec::new();
    for _ in 0..15 {
        r.skip(4)?;
        let len = usize::from(r.u16be()?) * 2;
        r.skip(1)?;
        let vol = r.u8()?;
        r.skip(4)?;
        let ll = usize::from(r.u16be()?) * 2;
        r.skip(2)?;
        heads.push((len, vol, ll));
    }
    r.skip(3)?;
    let nord = usize::from(r.u8()?).clamp(1, 100);
    let mut orders = Vec::with_capacity(nord);
    for _ in 0..100 {
        orders.push(r.u16be()? / 1024);
    }
    orders.truncate(nord);
    // Pattern 63 in an order is a slip of GMC's exporter, not a stored one.
    let npat = orders.iter().filter(|&&o| o != 63).max().map_or(0, |&o| usize::from(o) + 1);
    let mut patterns = Vec::with_capacity(npat);
    for _ in 0..npat {
        let data = r.bytes(1024)?;
        let mut p = Pattern::new(64, 4);
        for (i, b) in data.chunks_exact(4).enumerate() {
            let cut = b[0] == 0xFF && b[1] == 0xFE;
            let period = if cut { 0 } else { u16::from(b[0] & 0x0F) << 8 | u16::from(b[1]) };
            let param = b[3];
            let (fx, param) = match b[2] & 0x0F {
                1 => (Fx::PortaUp, param),
                2 => (Fx::PortaDown, param),
                3 => (Fx::Volume, (param & 0x7F).min(64)),
                4 => mod_effect(0xD, param, false),
                5 => (Fx::PositionJump, param),
                // The LED filter on and off.
                6 => (Fx::ModEx, 0x00),
                7 => (Fx::ModEx, 0x01),
                8 if param > 0 => (Fx::Speed, param),
                _ => (Fx::None, 0),
            };
            p.cells[i] = Cell {
                note: if cut { NOTE_CUT } else { period_to_note(period).unwrap_or(0) },
                instr: if cut { 0 } else { (b[0] & 0x10) | b[2] >> 4 },
                fx,
                param,
                ..Default::default()
            };
        }
        patterns.push(Arc::new(p));
    }
    let mut at = r.pos;
    let mut samples = Vec::with_capacity(15);
    for (len, vol, ll) in heads {
        // The loop is the sample's last `ll` bytes.
        samples.push(amiga_sample(d, at, String::new(), len, 0, vol, len.saturating_sub(ll), ll));
        at += len;
    }
    let mut m = amiga_module(String::new(), "gmc", "Game Music Creator", orders, patterns, samples);
    m.tidy();
    Ok(m)
}

// ---- Quadra Composer ----
//
// IFF: FORM EMOD, an EMIC chunk (title, composer, samples, the patterns'
// rows, the orders), PATT (the cells) and 8SMP (the samples).

fn iff_chunks(d: &[u8], from: usize) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut at = from;
    while at + 8 <= d.len() {
        let id: [u8; 4] = d[at..at + 4].try_into().unwrap_or_default();
        let len = u32::from_be_bytes([d[at + 4], d[at + 5], d[at + 6], d[at + 7]]) as usize;
        let body = &d[at + 8..(at + 8).saturating_add(len).min(d.len())];
        out.push((id, body));
        at = at.saturating_add(8 + len + (len & 1));
    }
    out
}

pub fn probe_emod(d: &[u8]) -> bool {
    d.len() >= 16 && &d[..4] == b"FORM" && &d[8..12] == b"EMOD" && &d[12..16] == b"EMIC"
}

pub fn load_emod(d: &[u8]) -> Result<Module> {
    let chunks = iff_chunks(d, 12);
    let find = |id: &[u8; 4]| chunks.iter().find(|c| &c.0 == id).map(|c| c.1);
    let Some(emic) = find(b"EMIC") else { return bad("it has no EMIC chunk") };
    let mut r = Reader::new(emic);
    let version = r.u16be()?;
    let title = r.text(20)?;
    let artist = r.text(20)?;
    let tempo = r.u8()?;
    let nsmp = usize::from(r.u8()?);
    let mut heads = Vec::with_capacity(nsmp);
    let mut numbers = Vec::with_capacity(nsmp);
    for _ in 0..nsmp {
        numbers.push(r.u8()?);
        let vol = r.u8()?;
        let len = usize::from(r.u16be()?) * 2;
        let name = r.text(20)?;
        let flags = r.u8()?;
        let ft = r.u8()?;
        let ls = usize::from(r.u16be()?) * 2;
        let ll = usize::from(r.u16be()?) * 2;
        r.skip(4)?;
        heads.push((name, len, ft, vol, ls, if flags & 1 != 0 { ll } else { 0 }));
    }
    r.skip(1)?;
    let npat = usize::from(r.u8()?);
    let mut layout = Vec::with_capacity(npat);
    for _ in 0..npat {
        let number = r.u8()?;
        let rows = usize::from(r.u8()?) + 1;
        let name = r.text(20)?;
        r.skip(4)?;
        layout.push((number, rows, name));
    }
    let nord = usize::from(r.u8()?);
    let raw_orders = r.bytes_upto(nord).to_vec();
    // Patterns and samples are named by numbers of their own.
    let pattern_index = |n: u8| layout.iter().position(|l| l.0 == n).map_or(ORDER_SKIP, |i| i as u16);
    let sample_index = |n: u8| numbers.iter().position(|&x| x == n).map_or(n, |i| i as u8 + 1);

    let mut cells = Reader::new(find(b"PATT").unwrap_or(&[]));
    let mut patterns = Vec::with_capacity(npat);
    for (_, rows, name) in &layout {
        let mut p = Pattern::new(*rows, 4);
        p.name = name.clone();
        for i in 0..rows * 4 {
            let Ok(b) = cells.bytes(4) else { break };
            let param = b[3];
            let (fx, param) = match b[2] & 0x0F {
                // The vibrato's depth is half MOD's.
                4 => (Fx::Vibrato, (param & 0xF0) | ((param << 1) & 0x0F)),
                9 => (Fx::None, 0),
                cmd => mod_effect(cmd, param, false),
            };
            p.cells[i] = Cell {
                note: if b[1] == 0xFF { 0 } else { b[1].saturating_add(49).min(NOTE_MAX) },
                instr: if b[0] == 0 { 0 } else { sample_index(b[0]) },
                fx,
                param,
                ..Default::default()
            };
        }
        patterns.push(Arc::new(p));
    }
    let pcm = find(b"8SMP").unwrap_or(&[]);
    let mut at = 0;
    let mut samples = Vec::with_capacity(nsmp);
    for (name, len, ft, vol, ls, ll) in heads {
        samples.push(amiga_sample(pcm, at, name, len, ft, vol, ls, ll));
        at += len;
    }
    let orders = raw_orders.iter().map(|&o| pattern_index(o)).collect();
    let mut m = amiga_module(title, "emod", "Quadra Composer", orders, patterns, samples);
    m.artist = artist;
    m.tracker = format!("Quadra Composer (EMOD {version})");
    if tempo >= 32 {
        m.tempo = u16::from(tempo);
    }
    m.tidy();
    Ok(m)
}

// ---- Digital Tracker ----
//
// The Atari Falcon's: "D.T." and the title, then IFF-style chunks: PATT
// (channels and the pattern format), S.Q. (orders), SV19 (panning and
// volumes), TEXT, INST (samples, and Digital Home Studio's
// instruments), DAPT (a pattern each), DAIT (a sample each).

pub fn probe_dtm(d: &[u8]) -> bool {
    if d.len() < 22 || &d[..4] != b"D.T." {
        return false;
    }
    let size = u32::from_be_bytes([d[4], d[5], d[6], d[7]]);
    (14..=256).contains(&size) && d[8] == 0 && d[9] == 0
}

pub fn load_dtm(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    r.skip(4)?;
    let hsize = r.u32be()? as usize;
    r.skip(2)?;
    let stereo = r.u8()?;
    r.skip(3)?;
    let speed = r.u16be()?;
    let tempo = r.u16be()?;
    let forced_rate = r.u32be()?;
    let title = r.text(hsize - 14)?;
    let chunks = iff_chunks(d, 8 + hsize);
    let find = |id: &[u8; 4]| chunks.iter().find(|c| &c.0 == id).map(|c| c.1);

    let Some(patt) = find(b"PATT") else { return bad("it has no PATT chunk") };
    let mut pr = Reader::new(patt);
    let nch = usize::from(pr.u16be()?);
    let _stored = pr.u16be()?;
    let layout = pr.tag()?;
    if nch == 0 || nch > 32 {
        return bad(format!("it says it has {nch} channels"));
    }
    let (pt, v204, v206) = (layout == [0; 4], &layout == b"2.04", &layout == b"2.06");
    if !(pt || v204 || v206) {
        return Err(crate::io::Error::Unsupported(format!("Digital Tracker's {} patterns aren't read", crate::io::text(&layout))));
    }
    let speed = if speed == 0 { 6 } else { speed.min(255) as u8 };

    let Some(sq) = find(b"S.Q.") else { return bad("it has no order list") };
    let mut sr = Reader::new(sq);
    let nord = usize::from(sr.u16be()?);
    let restart = sr.u16be()?;
    sr.skip(4)?;
    let orders: Vec<u16> = sr.bytes_upto(nord).iter().map(|&o| u16::from(o)).collect();

    let mut channels: Vec<Channel> = (0..nch).map(|c| Channel { pan: if stereo == 0 { amiga_pan(c) } else { 128 }, ..Default::default() }).collect();
    let mut global_volume = 128;
    let mut tempo = if tempo == 0 { 125 } else { tempo };
    if let Some(sv) = find(b"SV19") {
        let mut v = Reader::new(sv);
        v.skip(2)?;
        // A fraction of a beat a minute, to the nearest.
        if v.u32be()? >= 0x8000_0000 {
            tempo += 1;
        }
        for c in 0..32 {
            // 0 to 180, 90 the middle.
            let p = i32::from(v.u16be()?.min(180));
            if let Some(c) = channels.get_mut(c) {
                c.pan = (128 + (p - 90) * 128 / 90).clamp(0, 255) as u8;
            }
        }
        v.skip(16)?;
        if let Ok(g) = v.u16be() {
            global_volume = (g.min(256) / 2) as u8;
        }
        if v.skip(128).is_ok() {
            for c in channels.iter_mut() {
                if let Ok(vol) = v.u16be() {
                    c.volume = (vol.min(128) / 2) as u8;
                }
            }
        }
    }
    let mut msg = String::new();
    if let Some(text) = find(b"TEXT") {
        if text.len() >= 12 {
            let odd = u16::from_be_bytes([text[10], text[11]]) == 0xFFFF;
            msg = message(&text[12 + usize::from(odd).min(text.len() - 12)..]);
        }
    }

    // Samples: a header each, their PCM in DAIT chunks.
    let mut samples: Vec<Sample> = Vec::new();
    let mut formats: Vec<(bool, bool)> = Vec::new();
    let mut instruments = Vec::new();
    if let Some(inst) = find(b"INST") {
        let mut ir = Reader::new(inst);
        let n = ir.u16be()?;
        let numbered = n >= 0x8000;
        for i in 0..usize::from(n & 0x7FFF) {
            let slot = if numbered { usize::from(ir.u16be()?) } else { i };
            ir.skip(4)?;
            let _len = ir.u32be()?;
            let ft = ir.u8()?;
            let vol = ir.u8()?;
            let ls = ir.u32be()?;
            let ll = ir.u32be()?;
            let name = ir.text(22)?;
            let st = ir.u8()? & 1 != 0;
            let bits = ir.u8()?;
            let transpose = ir.u16be()?;
            ir.skip(2)?;
            let rate = ir.u32be()?;
            if slot >= 255 {
                continue;
            }
            let b16 = bits > 8;
            let div = (if st { 2 } else { 1 }) * (if b16 { 2 } else { 1 });
            let base = if pt && forced_rate > 0 { forced_rate } else { rate };
            // The finetune in eighths of a semitone, and Digital Home
            // Studio's transpose (48 is none).
            let mut semis = f64::from(((ft & 0x0F) as i8) << 4 >> 4) / 8.0;
            if v206 && transpose > 0 && transpose != 48 {
                semis += 48.0 - f64::from(transpose);
            }
            let c5 = (f64::from(base.max(1)) * 2f64.powf(semis / 12.0)).round().clamp(1.0, 2_000_000.0) as u32;
            let (ls, le) = (ls / div, (ls + ll) / div);
            while samples.len() <= slot {
                samples.push(Sample::empty());
                formats.push((false, false));
            }
            samples[slot] = Sample {
                name,
                wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, if st { 2 } else { 1 }, Vec::new())),
                c5,
                volume: vol.min(64),
                looped: if le > ls + 1 { Loop::forward(ls, le) } else { Loop::default() },
                ..Sample::empty()
            };
            formats[slot] = (b16, st);
        }
        // Digital Home Studio's instruments: an envelope each, held at its end.
        if ir.u16be().ok() == Some(4) {
            let envs = find(b"IENV").unwrap_or(&[]);
            let mut per: Vec<Option<Instrument>> = vec![None; samples.len()];
            while let Ok(b) = ir.bytes(15) {
                let n = usize::from(u16::from_be_bytes([b[0], b[1]]));
                if n >= per.len() {
                    continue;
                }
                let mut ins = Instrument::for_sample(&samples[n].name, n as u16 + 1);
                ins.fadeout = u32::from(u16::from_be_bytes([b[5], b[6]]).min(0xFFF)) * 2;
                samples[n].vibrato = AutoVibrato { kind: 0, speed: b[7], depth: b[8], sweep: 0 };
                let at = 2 + 34 * usize::from(b[3]);
                if b[3] != 0xFF && at + 34 <= envs.len() {
                    let e = &envs[at..at + 34];
                    let count = usize::from(u16::from_be_bytes([e[0], e[1]])).min(16);
                    let points: Vec<EnvPoint> = (0..count).map(|i| EnvPoint { tick: u16::from(e[3 + i * 2]), value: e[2 + i * 2].min(64) as i8 }).collect();
                    let last = points.len().saturating_sub(1) as u8;
                    ins.volume_env = Envelope {
                        enabled: !points.is_empty(),
                        looped: (!points.is_empty()).then_some((last, last)),
                        sustain: (b[4] != 0xFF && usize::from(b[4]) < points.len()).then_some((b[4], b[4])),
                        points,
                        ..Default::default()
                    };
                }
                per[n] = Some(ins);
            }
            instruments = per.into_iter().enumerate().map(|(i, ins)| ins.unwrap_or_else(|| Instrument::for_sample(&samples[i].name, i as u16 + 1))).collect();
        }
    }
    for (id, body) in &chunks {
        if id != b"DAIT" || body.len() < 2 {
            continue;
        }
        let slot = usize::from(u16::from_be_bytes([body[0], body[1]]));
        let Some(s) = samples.get_mut(slot) else { continue };
        let (b16, st) = formats[slot];
        let pcm = &body[2..];
        let points = pcm.len() / if b16 { 2 } else { 1 };
        let wave = decode(pcm, if b16 { Pcm::S16be } else { Pcm::S8 }, points - points % if st { 2 } else { 1 });
        s.wave = Arc::new(Wave::new(if b16 { 16 } else { 8 }, if st { 2 } else { 1 }, wave));
        s.looped = s.looped.clamped(s.frames() as u32);
    }

    let mut patterns: Vec<Arc<Pattern>> = Vec::new();
    for (id, body) in &chunks {
        if id != b"DAPT" || body.len() < 8 {
            continue;
        }
        let mut pr = Reader::new(body);
        pr.skip(4)?;
        let num = usize::from(pr.u16be()?);
        let mut rows = usize::from(pr.u16be()?);
        if num > 255 {
            continue;
        }
        if v206 {
            // Stored by ticks, not rows.
            rows /= usize::from(speed);
        }
        let rows = rows.clamp(1, 1024);
        let mut p = Pattern::new(rows, nch);
        if v206 {
            pr.skip(4)?;
            for c in 0..nch {
                let Ok(len) = pr.u16be() else { break };
                let len = usize::from(len) + usize::from(len) % 2;
                let mut er = Reader::new(pr.bytes_upto(len));
                let mut tick = 0usize;
                while er.left() >= 6 && tick / usize::from(speed) < rows {
                    let (row, rem) = (tick / usize::from(speed), (tick % usize::from(speed)).min(15) as u8);
                    let b = er.bytes(6)?;
                    let cell = &mut p.cells[row * nch + c];
                    if (1..=96).contains(&b[0]) {
                        cell.note = b[0] + 13;
                        if rem > 0 {
                            (cell.fx, cell.param) = (Fx::ModEx, 0xD0 | rem);
                        }
                    } else if b[0] & 0x80 != 0 {
                        if rem > 0 {
                            (cell.fx, cell.param) = (Fx::ModEx, 0xC0 | rem);
                        } else {
                            cell.note = NOTE_CUT;
                        }
                    }
                    if b[1] != 0 {
                        (cell.vol, cell.volp) = (Vol::Volume, b[1].min(64));
                    }
                    if b[2] != 0 {
                        cell.instr = b[2];
                    }
                    if b[3] != 0 || b[4] != 0 {
                        (cell.fx, cell.param) = mod_effect(b[3] & 0x0F, b[4], false);
                    }
                    let delay = b[5];
                    tick += if delay & 0x80 != 0 { usize::from(delay & 0x7F) * 256 + usize::from(er.u8().unwrap_or(0)) } else { usize::from(delay) };
                }
            }
        } else {
            for i in 0..rows * nch {
                let Ok(b) = pr.bytes(4) else { break };
                let mut cell = Cell::default();
                let (cmd, param);
                if v204 {
                    if b[0] > 0 && b[0] < 0x80 {
                        cell.note = ((b[0] >> 4) * 12 + (b[0] & 0x0F) + 12).min(NOTE_MAX);
                    }
                    let vol = b[1] >> 2;
                    if vol > 0 {
                        (cell.vol, cell.volp) = (Vol::Volume, (vol - 1).min(64));
                    }
                    cell.instr = (b[1] & 3) << 4 | b[2] >> 4;
                    (cmd, param) = (b[2] & 0x0F, b[3]);
                } else {
                    let period = u16::from(b[0] & 0x0F) << 8 | u16::from(b[1]);
                    cell.note = period_to_note(period).unwrap_or(0);
                    cell.instr = (b[0] & 0x30) | b[2] >> 4;
                    (cmd, param) = (b[2] & 0x0F, b[3]);
                }
                (cell.fx, cell.param) = mod_effect(cmd, param, false);
                // No memory for these: a zero does nothing; a slide up wins.
                match cell.fx {
                    Fx::PortaUp | Fx::PortaDown if cell.param == 0 => cell.fx = Fx::None,
                    Fx::VolSlide | Fx::TonePortaVol | Fx::VibratoVol if cell.param & 0xF0 != 0 => cell.param &= 0xF0,
                    Fx::VolSlide if cell.param == 0 => cell.fx = Fx::None,
                    _ => {}
                }
                p.cells[i] = cell;
            }
        }
        while patterns.len() <= num {
            patterns.push(Arc::new(Pattern::new(64, nch)));
        }
        patterns[num] = Arc::new(p);
    }

    let tracker = if v206 {
        "Digital Home Studio".to_string()
    } else if pt {
        "Digital Tracker 2.3".to_string()
    } else if let Some(v) = find(b"VERS").filter(|v| v.len() >= 4) {
        let n = u32::from_be_bytes([v[0], v[1], v[2], v[3]]);
        format!("Digital Tracker {}.{}", n >> 4, n & 0x0F)
    } else {
        "Digital Tracker".to_string()
    };
    if let Some(names) = find(b"PATN") {
        for (p, name) in names.split(|&c| c == 0).zip(patterns.iter_mut()) {
            Arc::make_mut(name).name = crate::io::text(p);
        }
    }
    if let Some(names) = find(b"TRKN") {
        for (n, c) in names.split(|&c| c == 0).zip(channels.iter_mut()) {
            c.name = crate::io::text(n);
        }
    }
    let has_instruments = !instruments.is_empty();
    let mut m = Module {
        title,
        message: msg,
        format: "dtm".into(),
        format_name: "Digital Tracker".into(),
        tracker,
        kind: Kind::Mod,
        flags: Flags { instruments: has_instruments, ..Default::default() },
        speed,
        tempo: tempo.clamp(32, 999),
        global_volume,
        mix_volume: 32,
        separation: 128,
        restart: if usize::from(restart) < orders.len() { restart } else { 0 },
        channels,
        orders,
        patterns,
        samples,
        instruments,
        ..Default::default()
    };
    if m.message.is_empty() {
        m.message = m.samples.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join("\n").trim_end().to_string();
    }
    m.tidy();
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ims_notes_count_from_protracker_c1() {
        let mut d = vec![0u8; 1084 + 768 + 2];
        d[..4].copy_from_slice(b"song");
        d[20..21].copy_from_slice(b"s");
        d[42..44].copy_from_slice(&1u16.to_be_bytes());
        d[45] = 64;
        d[950] = 1;
        d[1080..1084].copy_from_slice(&(1084u32 + 768).to_be_bytes());
        for c in 0..256 {
            d[1084 + c * 3] = 0x3F;
        }
        // C-1, sample 1, speed 6.
        d[1084..1087].copy_from_slice(&[12, 0x1F, 6]);
        assert!(probe_ims(&d));
        let m = load_ims(&d).unwrap();
        let c = m.patterns[0].cell(0, 0);
        assert_eq!((c.note, c.instr, c.fx, c.param), (49, 1, Fx::Speed, 6));
        assert_eq!(m.patterns[0].cell(1, 0).note, 0);
        assert_eq!(m.samples[0].frames(), 2);
    }

    #[test]
    fn gmc_loops_end_its_samples_and_3_sets_volume() {
        let mut d = vec![0u8; 444 + 1024 + 8];
        d[4..6].copy_from_slice(&4u16.to_be_bytes());
        d[7] = 64;
        d[12..14].copy_from_slice(&3u16.to_be_bytes());
        d[243] = 1;
        // Middle C, sample 1, volume 32.
        d[444..448].copy_from_slice(&[0x01, 0xAC, 0x13, 0x20]);
        assert!(probe_gmc(&d));
        let m = load_gmc(&d).unwrap();
        let c = m.patterns[0].cell(0, 0);
        assert_eq!((c.note, c.instr, c.fx, c.param), (61, 1, Fx::Volume, 32));
        assert_eq!((m.samples[0].looped.start, m.samples[0].looped.end), (2, 8));
    }
}
