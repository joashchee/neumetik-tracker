//! A song written as MOD, S3M, XM or IT, for the trackers that read
//! them, with what each format can't keep said: notes out of its range,
//! effects it doesn't have, more channels or samples than it takes,
//! UMF's own (effect slots, automation, phrases...). Each is written as
//! its tracker wrote it (ProTracker's M.K., Scream Tracker 3.20's,
//! FastTracker 2.04's, Impulse Tracker 2.14's), the effects moved
//! between the MOD family's commands and the S3M family's as their
//! rules have them.

use std::collections::BTreeMap;

use crate::model::*;

/// The formats written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Mod,
    S3m,
    Xm,
    It,
}

impl Format {
    pub fn from_name(s: &str) -> Option<Format> {
        Some(match s {
            "mod" => Format::Mod,
            "s3m" => Format::S3m,
            "xm" => Format::Xm,
            "it" => Format::It,
            _ => return None,
        })
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Mod => "mod",
            Format::S3m => "s3m",
            Format::Xm => "xm",
            Format::It => "it",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Mod => "ProTracker MOD",
            Format::S3m => "Scream Tracker 3",
            Format::Xm => "FastTracker 2",
            Format::It => "Impulse Tracker",
        }
    }
}

/// A file written, and what it couldn't keep.
#[derive(Clone, Debug, Default)]
pub struct Exported {
    pub bytes: Vec<u8>,
    /// Each thing lost, with how many times.
    pub lost: Vec<(String, usize)>,
}

/// What's lost, counted.
#[derive(Default)]
struct Lost(BTreeMap<String, usize>);

impl Lost {
    fn add(&mut self, what: &str) {
        *self.0.entry(what.to_string()).or_default() += 1;
    }

    fn many(&mut self, what: &str, n: usize) {
        if n > 0 {
            *self.0.entry(what.to_string()).or_default() += n;
        }
    }
}

pub fn export(m: &Module, to: Format) -> Exported {
    let mut lost = Lost::default();
    extras_lost(m, to, &mut lost);
    let bytes = match to {
        Format::Mod => write_mod(m, &mut lost),
        Format::S3m => write_s3m(m, &mut lost),
        Format::Xm => write_xm(m, &mut lost),
        Format::It => write_it(m, &mut lost),
    };
    Exported { bytes, lost: lost.0.into_iter().collect() }
}

/// UMF's own, which none of the four keep.
fn extras_lost(m: &Module, to: Format, lost: &mut Lost) {
    if m.all_sequences().len() > 1 {
        lost.add("only the order list being edited is kept");
    }
    lost.many("effect slots", m.effects.len());
    lost.many("automation lanes", m.patterns.iter().map(|p| p.automation.len()).sum());
    lost.many("phrases", m.instruments.iter().filter(|i| i.phrase.is_some()).count());
    lost.many("FM (AdLib) instruments (silent)", m.samples.iter().filter(|s| s.fm.is_some()).count());
    lost.many("synth sounds (a hybrid keeps its sample)", m.samples.iter().filter(|s| s.program.is_some()).count());
    lost.many("SEL scripts", m.sel.as_ref().map_or(0, |s| s.scripts.iter().flatten().count()));
    if m.swing > 0 || m.patterns.iter().any(|p| p.swing.is_some_and(|s| s > 0)) {
        lost.add("swing");
    }
    if m.tempo_mode != TempoMode::Classic {
        lost.add("the tempo mode (it plays at classic tempo)");
    }
    lost.many("muted matrix slots", m.slot_mutes.len());
    lost.many("bookmarks", m.bookmarks.len());
    if to != Format::It {
        lost.many("MIDI outputs", m.instruments.iter().filter(|i| i.midi.is_some()).count());
    }
    if m.macros.is_some() && to != Format::It {
        lost.add("MIDI macros");
    }
    if m.flags.linear && matches!(to, Format::Mod | Format::S3m) {
        lost.add("linear slides (made the Amiga's)");
    }
    // What the formats' headers have no room for (read back as their trackers' own).
    let amiga = |i: usize| crate::formats::common::amiga_pan(i);
    match to {
        Format::Mod => {
            if m.channels.iter().enumerate().any(|(i, c)| c.pan != amiga(i) || c.volume != 64 || c.surround) {
                lost.add("channel panning and volumes (MOD's are the Amiga's)");
            }
            if m.mix_volume != 32 || m.global_volume != 128 {
                lost.add("the mix level and global volume");
            }
        }
        Format::Xm => {
            if m.channels.iter().any(|c| c.pan != 128 || c.volume != 64 || c.surround) {
                lost.add("channel panning and volumes");
            }
            if m.mix_volume != 32 {
                lost.add("the mix level");
            }
        }
        Format::S3m => {
            if m.channels.iter().any(|c| c.volume != 64 || c.surround) {
                lost.add("channel volumes");
            }
        }
        Format::It => {}
    }
    let releases = m.instruments.iter().flat_map(|i| [&i.volume_env, &i.pan_env, &i.pitch_env]).filter(|e| e.release.is_some()).count();
    lost.many("envelope release nodes", releases);
}

// ---- Effects between the two families ----

/// MOD's (and XM's) command for a cell's effect, and its parameter:
/// 0 to F, and XM's letters as its file numbers them (G 16, H 17, K 20,
/// L 21, P 25, R 27, T 29, X 33).
fn mod_family(c: &Cell, from: Kind, xm: bool, lost: &mut Lost) -> (u8, u8) {
    let p = c.param;
    let s3m = from.s3m_like();
    let none = (0, 0);
    let fine = |up: bool, x: u8| (0xE, if up { 0x10 } else { 0x20 } | (x & 0x0F));
    let vol_slide = |p: u8, lost: &mut Lost| -> u8 {
        // S3M's fine slides in the parameter aren't there in MOD's 5, 6 and A.
        if s3m && ((p & 0x0F == 0x0F && p >> 4 != 0) || (p >> 4 == 0x0F && p & 0x0F != 0)) {
            lost.add("fine volume slides beside another effect");
            0
        } else {
            p
        }
    };
    match c.fx {
        Fx::None => none,
        Fx::Arpeggio => (0, p),
        Fx::PortaUp | Fx::PortaDown => {
            let up = c.fx == Fx::PortaUp;
            if s3m && p >= 0xF0 {
                fine(up, p)
            } else if s3m && p >= 0xE0 {
                if xm {
                    (0x21, if up { 0x10 } else { 0x20 } | (p & 0x0F))
                } else {
                    lost.add("extra-fine pitch slides (made fine)");
                    fine(up, ((p & 0x0F) + 2) / 4)
                }
            } else {
                (if up { 1 } else { 2 }, p)
            }
        }
        Fx::TonePorta => (3, p),
        Fx::Vibrato => (4, p),
        Fx::TonePortaVol => (5, vol_slide(p, lost)),
        Fx::VibratoVol => (6, vol_slide(p, lost)),
        Fx::Tremolo => (7, p),
        Fx::Panning => (8, p),
        Fx::Offset => (9, p),
        Fx::VolSlide => {
            if s3m && p & 0x0F == 0x0F && p >> 4 != 0 {
                (0xE, 0xA0 | p >> 4)
            } else if s3m && p >> 4 == 0x0F && p & 0x0F != 0 {
                (0xE, 0xB0 | (p & 0x0F))
            } else {
                (0xA, p)
            }
        }
        Fx::PositionJump => (0xB, p),
        Fx::Volume => (0xC, p.min(64)),
        Fx::PatternBreak => (0xD, ((p.min(63) / 10) << 4) | p.min(63) % 10),
        Fx::ModEx => (0xE, p),
        Fx::Speed => (0xF, p.clamp(1, 0x1F)),
        Fx::Tempo if p >= 0x20 => (0xF, p),
        Fx::Tempo => {
            lost.add("tempo slides (T0x and T1x)");
            none
        }
        Fx::S3mEx => {
            let (x, y) = (p >> 4, p & 0x0F);
            let e = match x {
                0x1 => Some(0x30),
                0x2 => Some(0x50),
                0x3 => Some(0x40),
                0x4 => Some(0x70),
                0x8 => Some(0x80),
                0xB => Some(0x60),
                0xC => Some(0xC0),
                0xD => Some(0xD0),
                0xE => Some(0xE0),
                _ => None,
            };
            match e {
                Some(e) => (0xE, e | y),
                None => {
                    lost.add("S3M and IT special commands (S6x, S7x, S9x, SAx, SFx)");
                    none
                }
            }
        }
        Fx::FinePortaUp => fine(true, p),
        Fx::FinePortaDown => fine(false, p),
        Fx::FineVolSlideUp => (0xE, 0xA0 | p.min(15)),
        Fx::FineVolSlideDown => (0xE, 0xB0 | p.min(15)),
        Fx::SetFinetune => (0xE, 0x50 | (p & 0x0F)),
        Fx::ExtraFinePorta if xm => (0x21, p),
        Fx::GlobalVolume if xm => (0x10, p.min(128) / 2),
        Fx::GlobalVolSlide if xm => (0x11, p),
        Fx::KeyOff if xm => (0x14, p),
        Fx::EnvelopePosition if xm => (0x15, p),
        // IT's P0x slides right, XM's Px0.
        Fx::PanningSlide if xm => (0x19, if s3m { p.rotate_left(4) } else { p }),
        Fx::Retrig if xm => (0x1B, p),
        Fx::Tremor if xm => (0x1D, p),
        other => {
            lost.add(&format!("{} effects", describe(other)));
            none
        }
    }
}

/// S3M's and IT's command letter (1 is A) for a cell's effect, and its
/// parameter.
fn s3m_family(c: &Cell, from: Kind, it: bool, lost: &mut Lost) -> (u8, u8) {
    let p = c.param;
    let mod_like = !from.s3m_like();
    let none = (0, 0);
    // MOD's 0 parameter means "nothing"; S3M's means "as before".
    let memory = |p: u8| mod_like && p == 0;
    match c.fx {
        Fx::None => none,
        Fx::Speed => (1, p.max(1)),
        Fx::PositionJump => (2, p),
        Fx::PatternBreak => (3, if it { p } else { ((p.min(63) / 10) << 4) | p.min(63) % 10 }),
        Fx::VolSlide if memory(p) => none,
        Fx::VolSlide => (4, if mod_like { if p >> 4 != 0 { p & 0xF0 } else { p & 0x0F } } else { p }),
        Fx::PortaDown if memory(p) => none,
        Fx::PortaDown => (5, if mod_like { p.min(0xDF) } else { p }),
        Fx::PortaUp if memory(p) => none,
        Fx::PortaUp => (6, if mod_like { p.min(0xDF) } else { p }),
        Fx::TonePorta => (7, p),
        Fx::Vibrato => (8, p),
        Fx::Tremor => (9, p),
        Fx::Arpeggio if memory(p) => none,
        Fx::Arpeggio => (10, p),
        Fx::VibratoVol => (11, if mod_like { if p >> 4 != 0 { p & 0xF0 } else { p & 0x0F } } else { p }),
        Fx::TonePortaVol => (12, if mod_like { if p >> 4 != 0 { p & 0xF0 } else { p & 0x0F } } else { p }),
        Fx::ChannelVolume if it => (13, p.min(64)),
        Fx::ChannelVolSlide if it => (14, p),
        Fx::Offset => (15, p),
        Fx::PanningSlide if it => (16, if mod_like { p.rotate_left(4) } else { p }),
        Fx::Retrig => (17, p),
        Fx::Tremolo => (18, p),
        Fx::S3mEx => (19, p),
        Fx::Tempo => (20, p),
        Fx::FineVibrato => (21, p),
        Fx::GlobalVolume => (22, if it { p.min(128) } else { p.min(128) / 2 }),
        Fx::GlobalVolSlide => (23, p),
        Fx::Panning => (24, if it { p } else { (u16::from(p) * 128 / 255) as u8 }),
        Fx::Panbrello if it => (25, p),
        Fx::Midi if it => (26, p),
        Fx::SmoothMidi if it => (27, p),
        Fx::ModEx => {
            let (x, y) = (p >> 4, p & 0x0F);
            match x {
                0x1 => (6, 0xF0 | y),
                0x2 => (5, 0xF0 | y),
                0x3 => (19, 0x10 | y),
                0x4 => (19, 0x30 | y),
                0x5 => (19, 0x20 | y),
                0x6 => (19, 0xB0 | y),
                0x7 => (19, 0x40 | y),
                0x8 => (19, 0x80 | y),
                0x9 => (17, y),
                0xA => (4, y << 4 | 0x0F),
                0xB => (4, 0xF0 | y),
                0xC => (19, 0xC0 | y),
                0xD => (19, 0xD0 | y),
                0xE => (19, 0xE0 | y),
                _ => {
                    lost.add("MOD's E0x (the Amiga's filter) and EFx");
                    none
                }
            }
        }
        Fx::FinePortaUp => (6, 0xF0 | (p & 0x0F)),
        Fx::FinePortaDown => (5, 0xF0 | (p & 0x0F)),
        Fx::FineVolSlideUp => (4, p.min(15) << 4 | 0x0F),
        Fx::FineVolSlideDown => (4, 0xF0 | p.min(15)),
        Fx::SetFinetune => (19, 0x20 | (p & 0x0F)),
        Fx::ExtraFinePorta => match p >> 4 {
            1 => (6, 0xE0 | (p & 0x0F)),
            2 => (5, 0xE0 | (p & 0x0F)),
            _ => none,
        },
        other => {
            lost.add(&format!("{} effects", describe(other)));
            none
        }
    }
}

fn describe(fx: Fx) -> String {
    let d = fx.describe();
    let short = d.split(':').next().unwrap_or(d).split(',').next().unwrap_or(d);
    format!("{} ({})", short, fx.letter())
}

/// A cell's volume column the MOD family's effect can carry, when the
/// effect's free: a volume as C.
fn volume_to_effect(c: &Cell) -> Option<(Fx, u8)> {
    (c.vol == Vol::Volume && c.fx == Fx::None).then_some((Fx::Volume, c.volp.min(64)))
}

/// The sample a cell plays in a format with no instruments: its
/// instrument's sample for its note (or for middle C), or the cell's own.
fn sample_slot(m: &Module, instr: u8, note: u8) -> u16 {
    if instr == 0 {
        return 0;
    }
    if m.flags.instruments {
        let n = if (NOTE_MIN..=NOTE_MAX).contains(&note) { note } else { NOTE_MIDDLE_C };
        m.instruments.get(usize::from(instr) - 1).map_or(0, |i| i.map(n).1)
    } else {
        u16::from(instr)
    }
}

/// The samples, as the formats without instruments have them: the
/// song's own, all of them.
fn mono(s: &Sample, bits: u8) -> Wave {
    crate::sample_edit::convert(&s.wave, 1, bits)
}

fn put_text(out: &mut Vec<u8>, s: &str, n: usize, pad: u8) {
    let b: Vec<u8> = s.chars().map(|c| if c.is_ascii() { c as u8 } else { b'?' }).take(n).collect();
    out.extend_from_slice(&b);
    out.extend(std::iter::repeat_n(pad, n - b.len()));
}

/// The orders a format without skips keeps, and the patterns they name.
fn played_orders(m: &Module, skips: bool, most: usize, lost: &mut Lost) -> Vec<u16> {
    let mut o: Vec<u16> = m.orders.iter().copied().filter(|&o| skips || o < ORDER_SKIP).collect();
    if o.len() > most {
        lost.many("orders past the format's most", o.len() - most);
        o.truncate(most);
    }
    o
}

/// Rows past the format's most cut, and short patterns broken off at
/// their end (for the formats whose patterns are all 64 rows).
fn fixed_rows(p: &Pattern, rows: usize, lost: &mut Lost) -> Pattern {
    if p.rows > rows {
        lost.many("pattern rows past 64", p.rows - rows);
    }
    let mut q = p.with_rows(rows);
    if p.rows < rows && p.rows > 0 {
        let last = p.rows - 1;
        match (0..q.channels).find(|&ch| q.cell(last, ch).fx == Fx::None) {
            Some(ch) => {
                let c = q.cell_mut(last, ch).expect("in the pattern");
                c.fx = Fx::PatternBreak;
                c.param = 0;
            }
            None => lost.add("a short pattern's end (no free effect for the break)"),
        }
    }
    q
}

// ---- MOD ----

/// ProTracker's periods for its three octaves (C-1 to B-3, here notes 49 to 84).
fn mod_period(note: u8) -> u16 {
    const TABLE: [u16; 36] = [856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453, 428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226, 214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113];
    if (49..=84).contains(&note) {
        TABLE[usize::from(note - 49)]
    } else {
        (428.0 * 2f64.powf(f64::from(61 - i32::from(note)) / 12.0)).round().clamp(1.0, 4095.0) as u16
    }
}

fn write_mod(m: &Module, lost: &mut Lost) -> Vec<u8> {
    let nch = m.channels.len().min(32);
    if m.channels.len() > 32 {
        lost.many("channels past 32", m.channels.len() - 32);
    }
    if m.flags.instruments {
        lost.add("instruments (played as their samples: no envelopes or keymaps)");
    }
    let mut out = Vec::new();
    put_text(&mut out, &m.title, 20, 0);
    if m.samples.len() > 31 {
        lost.many("samples past 31", m.samples.len() - 31);
    }
    let mut waves = Vec::new();
    for i in 0..31 {
        let Some(s) = m.samples.get(i) else {
            out.extend_from_slice(&[0; 22]);
            out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
            waves.push(Vec::new());
            continue;
        };
        if s.wave.bits > 8 {
            lost.add("16-bit samples (made 8-bit)");
        }
        if s.wave.channels > 1 {
            lost.add("stereo samples (mixed to mono)");
        }
        let mut w = mono(s, 8).pcm;
        if w.len() > 131_070 {
            lost.add("samples longer than 128 KB (cut)");
            w.truncate(131_070);
        }
        if w.len() % 2 == 1 {
            w.push(0);
        }
        let ft = (96.0 * (f64::from(s.c5.max(1)) / 8363.0).log2()).round();
        if !(-8.0..=7.0).contains(&ft) {
            lost.add("sample tunings past MOD's finetune (out of tune)");
        }
        if s.looped.kind == LoopKind::PingPong {
            lost.add("ping-pong loops (made forward)");
        }
        if s.sustain.on() {
            lost.add("sustain loops");
        }
        put_text(&mut out, &s.name, 22, 0);
        out.extend_from_slice(&((w.len() / 2) as u16).to_be_bytes());
        out.push((ft.clamp(-8.0, 7.0) as i8 as u8) & 0x0F);
        out.push(s.volume.min(64));
        let lp = s.looped.clamped(w.len() as u32);
        if lp.on() {
            out.extend_from_slice(&((lp.start / 2) as u16).to_be_bytes());
            out.extend_from_slice(&(((lp.end - lp.start) / 2).max(1) as u16).to_be_bytes());
        } else {
            out.extend_from_slice(&0u16.to_be_bytes());
            out.extend_from_slice(&1u16.to_be_bytes());
        }
        waves.push(w);
    }
    let orders = played_orders(m, false, 128, lost);
    // Patterns past what MOD numbers (64, or 100 tagged M!K!).
    let most_patterns = if nch == 4 { 100 } else { 128 };
    out.push(orders.len().max(1) as u8);
    out.push(if usize::from(m.restart) < orders.len() { m.restart as u8 } else { 127 });
    let mut table = [0u8; 128];
    for (i, &o) in orders.iter().enumerate() {
        table[i] = o.min(most_patterns as u16 - 1) as u8;
    }
    out.extend_from_slice(&table);
    let npat = (orders.iter().copied().max().unwrap_or(0) as usize + 1).min(most_patterns);
    let tag: Vec<u8> = match nch {
        4 if npat > 64 => b"M!K!".to_vec(),
        4 => b"M.K.".to_vec(),
        1..=9 => format!("{nch}CHN").into_bytes(),
        _ => format!("{nch:02}CH").into_bytes(),
    };
    out.extend_from_slice(&tag);
    let first = usize::from(orders.first().copied().unwrap_or(0));
    for pi in 0..npat {
        let mut src = m.patterns.get(pi).map(|p| fixed_rows(p, 64, lost)).unwrap_or_else(|| Pattern::new(64, nch));
        // Speed and tempo, when they aren't MOD's own, in the song's first row.
        if pi == first {
            let mut want: Vec<(Fx, u8)> = Vec::new();
            if m.speed != 6 {
                want.push((Fx::Speed, m.speed.min(31)));
            }
            if m.tempo != 125 {
                want.push((Fx::Tempo, m.tempo.clamp(32, 255) as u8));
            }
            for (fx, param) in want {
                match (0..nch).find(|&ch| src.cell(0, ch).fx == Fx::None && src.cell(0, ch).vol != Vol::Volume) {
                    Some(ch) => {
                        let c = src.cell_mut(0, ch).expect("in the pattern");
                        c.fx = fx;
                        c.param = param;
                    }
                    None => lost.add("the song's speed or tempo (no free effect in its first row)"),
                }
            }
        }
        for row in 0..64 {
            for ch in 0..nch {
                let mut c = src.cell(row, ch);
                if c.vol != Vol::None {
                    if let Some((fx, p)) = volume_to_effect(&c) {
                        c.fx = fx;
                        c.param = p;
                    } else {
                        lost.add("volume column commands");
                    }
                }
                let period = match c.note {
                    n @ NOTE_MIN..=NOTE_MAX => {
                        if !(37..=96).contains(&n) {
                            lost.add("notes out of MOD's range");
                        }
                        mod_period(n)
                    }
                    NOTE_NONE => 0,
                    _ => {
                        // A note off or cut as a volume of 0, when the effect's free.
                        if c.fx == Fx::None {
                            c.fx = Fx::Volume;
                            c.param = 0;
                        } else {
                            lost.add("note offs and cuts beside an effect");
                        }
                        0
                    }
                };
                let s = sample_slot(m, c.instr, c.note).min(31) as u8;
                let (cmd, param) = mod_family(&c, m.kind, false, lost);
                out.push((s & 0xF0) | (period >> 8) as u8);
                out.push(period as u8);
                out.push((s & 0x0F) << 4 | (cmd & 0x0F));
                out.push(param);
            }
        }
    }
    for w in waves {
        out.extend(w.iter().map(|&v| (v >> 8) as u8));
    }
    if m.patterns.len() > npat {
        lost.many("patterns no order plays, or past MOD's most", m.patterns.len() - npat);
    }
    out
}

// ---- S3M ----

fn pad16(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(16) {
        out.push(0);
    }
}

fn write_s3m(m: &Module, lost: &mut Lost) -> Vec<u8> {
    let nch = m.channels.len().min(16);
    if m.channels.len() > 16 {
        lost.many("channels past Scream Tracker's 16", m.channels.len() - 16);
    }
    if m.flags.instruments {
        lost.add("instruments (played as their samples: no envelopes or keymaps)");
    }
    let nsmp = m.samples.len().min(99);
    if m.samples.len() > 99 {
        lost.many("samples past 99", m.samples.len() - 99);
    }
    let orders = played_orders(m, true, 255, lost);
    let npat = m.patterns.len().min(100);
    if m.patterns.len() > 100 {
        lost.many("patterns past 100", m.patterns.len() - 100);
    }
    let mut out = Vec::new();
    put_text(&mut out, &m.title, 28, 0);
    out.extend_from_slice(&[0x1A, 16, 0, 0]);
    let ordnum = (orders.len() + 1).next_multiple_of(2);
    out.extend_from_slice(&(ordnum as u16).to_le_bytes());
    out.extend_from_slice(&(nsmp as u16).to_le_bytes());
    out.extend_from_slice(&(npat as u16).to_le_bytes());
    out.extend_from_slice(&(if m.flags.amiga_limits { 16u16 } else { 0 }).to_le_bytes());
    out.extend_from_slice(&0x1320u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(b"SCRM");
    out.push((m.global_volume / 2).min(64));
    out.push(m.speed);
    if m.tempo > 255 {
        lost.add("a tempo over 255");
    }
    out.push(m.tempo.clamp(33, 255) as u8);
    out.push(m.mix_volume.clamp(16, 127) | 0x80);
    out.push(0);
    out.push(252);
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&0u16.to_le_bytes());
    // Channels: left ones 0 to 7, right 8 to 15, as their panning has them.
    let (mut left, mut right) = (0u8, 8u8);
    let mut settings = [255u8; 32];
    for (i, c) in m.channels.iter().take(nch).enumerate() {
        let slot = if c.pan < 128 && left < 8 || right >= 16 {
            left += 1;
            left - 1
        } else {
            right += 1;
            right - 1
        };
        settings[i] = slot | if c.muted { 0x80 } else { 0 };
    }
    out.extend_from_slice(&settings);
    for &o in &orders {
        out.push(if o == ORDER_SKIP { 254 } else { o.min(99) as u8 });
    }
    out.extend(std::iter::repeat_n(255u8, ordnum - orders.len()));
    let ptrs_at = out.len();
    out.extend(std::iter::repeat_n(0u8, (nsmp + npat) * 2));
    for i in 0..32 {
        out.push(m.channels.get(i).map_or(0x08, |c| 0x20 | (c.pan / 17)));
    }
    pad16(&mut out);
    let mut sample_heads = Vec::new();
    for (i, s) in m.samples.iter().take(nsmp).enumerate() {
        let at = out.len();
        let mut h = Vec::with_capacity(80);
        let has = !s.wave.is_empty();
        h.push(if has { 1 } else { 0 });
        put_text(&mut h, &s.filename, 12, 0);
        h.extend_from_slice(&[0, 0, 0]);
        let w = mono(s, s.wave.bits);
        if s.wave.channels > 1 {
            lost.add("stereo samples (mixed to mono)");
        }
        let lp = s.looped.clamped(w.frames() as u32);
        if s.looped.kind == LoopKind::PingPong {
            lost.add("ping-pong loops (made forward)");
        }
        if s.sustain.on() {
            lost.add("sustain loops");
        }
        h.extend_from_slice(&(w.frames() as u32).to_le_bytes());
        h.extend_from_slice(&lp.start.to_le_bytes());
        h.extend_from_slice(&lp.end.to_le_bytes());
        h.push(s.volume.min(64));
        h.push(0);
        h.push(0);
        h.push(u8::from(lp.on()) | if w.bits == 16 { 4 } else { 0 });
        h.extend_from_slice(&s.c5.to_le_bytes());
        h.extend_from_slice(&[0; 12]);
        put_text(&mut h, &s.name, 28, 0);
        h.extend_from_slice(b"SCRS");
        out.extend_from_slice(&h);
        pad16(&mut out);
        out[ptrs_at + i * 2..ptrs_at + i * 2 + 2].copy_from_slice(&((at / 16) as u16).to_le_bytes());
        sample_heads.push((at, w));
    }
    for pi in 0..npat {
        let at = out.len();
        let p = fixed_rows(&m.patterns[pi], 64, lost);
        let mut body = Vec::new();
        for row in 0..64 {
            for ch in 0..nch {
                let mut c = p.cell(row, ch);
                let note = match c.note {
                    n @ 13..=108 => Some((((n - 13) / 12) << 4) | (n - 13) % 12),
                    NOTE_NONE => None,
                    NOTE_CUT | NOTE_OFF | NOTE_FADE => {
                        if c.note != NOTE_CUT {
                            lost.add("note offs and fades (made cuts)");
                        }
                        Some(254)
                    }
                    _ => {
                        lost.add("notes out of Scream Tracker's range");
                        None
                    }
                };
                let vol = match c.vol {
                    Vol::None => None,
                    Vol::Volume => Some(c.volp.min(64)),
                    _ => {
                        lost.add("volume column commands besides volume");
                        None
                    }
                };
                if c.fx == Fx::Volume {
                    // MOD's Cxx: S3M's volume column.
                    if vol.is_none() {
                        c.vol = Vol::Volume;
                        c.volp = c.param.min(64);
                        c.fx = Fx::None;
                    } else {
                        lost.add("a volume effect beside a volume");
                        c.fx = Fx::None;
                    }
                }
                let vol = vol.or((c.vol == Vol::Volume).then_some(c.volp));
                let (cmd, param) = s3m_family(&c, m.kind, false, lost);
                let instr = sample_slot(m, c.instr, c.note).min(99) as u8;
                let mut what = ch as u8;
                if note.is_some() || instr != 0 {
                    what |= 32;
                }
                if vol.is_some() {
                    what |= 64;
                }
                if cmd != 0 {
                    what |= 128;
                }
                if what & 0xE0 == 0 {
                    continue;
                }
                body.push(what);
                if what & 32 != 0 {
                    body.push(note.unwrap_or(255));
                    body.push(instr);
                }
                if let Some(v) = vol {
                    body.push(v);
                }
                if cmd != 0 {
                    body.push(cmd);
                    body.push(param);
                }
            }
            body.push(0);
        }
        out.extend_from_slice(&((body.len() + 2) as u16).to_le_bytes());
        out.extend_from_slice(&body);
        pad16(&mut out);
        let k = ptrs_at + (nsmp + pi) * 2;
        out[k..k + 2].copy_from_slice(&((at / 16) as u16).to_le_bytes());
    }
    for (head, w) in sample_heads {
        if w.is_empty() {
            continue;
        }
        let at = out.len();
        let para = at / 16;
        out[head + 13] = (para >> 16) as u8;
        out[head + 14..head + 16].copy_from_slice(&((para & 0xFFFF) as u16).to_le_bytes());
        // Unsigned, as Scream Tracker kept them.
        for &v in &w.pcm {
            if w.bits == 16 {
                out.extend_from_slice(&((v as u16) ^ 0x8000).to_le_bytes());
            } else {
                out.push(((v >> 8) as u8) ^ 0x80);
            }
        }
        pad16(&mut out);
    }
    out
}

// ---- XM ----

fn xm_volume(c: &Cell, from: Kind, lost: &mut Lost) -> u8 {
    let x = c.volp;
    match c.vol {
        Vol::None => 0,
        Vol::Volume => 0x10 + x.min(64),
        Vol::VolSlideDown => 0x60 | x.min(15),
        Vol::VolSlideUp => 0x70 | x.min(15),
        Vol::FineVolDown => 0x80 | x.min(15),
        Vol::FineVolUp => 0x90 | x.min(15),
        Vol::VibratoSpeed => 0xA0 | x.min(15),
        Vol::VibratoDepth => 0xB0 | x.min(15),
        Vol::Panning => 0xC0 | ((u16::from(x.min(64)) * 15 + 32) / 64) as u8,
        Vol::PanSlideLeft => 0xD0 | x.min(15),
        Vol::PanSlideRight => 0xE0 | x.min(15),
        // XM's speed is 16 times its digit; IT's is its table's value.
        Vol::TonePorta => 0xF0 | if from == Kind::Xm { x.min(15) } else { (x / 16).min(15) },
        _ => {
            lost.add("volume column pitch slides and offsets");
            0
        }
    }
}

fn xm_env(e: &Envelope, pan: bool, lost: &mut Lost) -> ([u8; 48], u8, u8, u8, u8, u8) {
    if e.points.len() > 12 {
        lost.add("envelope points past 12");
    }
    if e.sustain.is_some_and(|(a, b)| a != b) {
        lost.add("envelope sustain loops (held on their first point)");
    }
    let n = e.points.len().min(12) as u8;
    let ok = |v: u8| v.min(n.saturating_sub(1));
    let (ls, le) = e.looped.unwrap_or((0, 0));
    let flags = u8::from(e.enabled && n > 0) | u8::from(e.sustain.is_some()) << 1 | u8::from(e.looped.is_some()) << 2;
    let mut pts = [0u8; 48];
    for (i, p) in e.points.iter().take(12).enumerate() {
        let v = if pan { (i16::from(p.value) + 32).clamp(0, 64) as u16 } else { p.value.clamp(0, 64) as u16 };
        pts[i * 4..i * 4 + 2].copy_from_slice(&p.tick.to_le_bytes());
        pts[i * 4 + 2..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    (pts, n, ok(e.sustain.map_or(0, |s| s.0)), ok(ls), ok(le), flags)
}

fn relative_of(c5: u32) -> (i8, i8) {
    let semis = 12.0 * (f64::from(c5.max(1)) / 8363.0).log2();
    let rel = semis.floor();
    let ft = ((semis - rel) * 128.0).round();
    let (rel, ft) = if ft >= 128.0 { (rel + 1.0, ft - 128.0) } else { (rel, ft) };
    (rel.clamp(-96.0, 95.0) as i8, ft.clamp(-128.0, 127.0) as i8)
}

fn write_xm(m: &Module, lost: &mut Lost) -> Vec<u8> {
    let nch = m.channels.len().min(32);
    if m.channels.len() > 32 {
        lost.many("channels past 32", m.channels.len() - 32);
    }
    let orders = played_orders(m, false, 256, lost);
    let npat = m.patterns.len().min(256);
    // Instruments: the song's, or one a sample.
    let instruments: Vec<Instrument> = if m.flags.instruments { m.instruments.iter().take(128).cloned().collect() } else { m.samples.iter().take(128).enumerate().map(|(i, s)| Instrument::for_sample(&s.name, (i + 1) as u16)).collect() };
    let count = if m.flags.instruments { m.instruments.len() } else { m.samples.len() };
    if count > 128 {
        lost.many("instruments past 128", count - 128);
    }
    let mut out = b"Extended Module: ".to_vec();
    put_text(&mut out, &m.title, 20, 0);
    out.push(0x1A);
    put_text(&mut out, "Neumetik", 20, b' ');
    out.extend_from_slice(&0x0104u16.to_le_bytes());
    out.extend_from_slice(&276u32.to_le_bytes());
    out.extend_from_slice(&(orders.len().max(1) as u16).to_le_bytes());
    out.extend_from_slice(&(m.restart.min(orders.len().saturating_sub(1) as u16)).to_le_bytes());
    out.extend_from_slice(&(nch as u16).to_le_bytes());
    out.extend_from_slice(&(npat as u16).to_le_bytes());
    out.extend_from_slice(&(instruments.len() as u16).to_le_bytes());
    out.extend_from_slice(&u16::from(m.flags.linear).to_le_bytes());
    out.extend_from_slice(&u16::from(m.speed.max(1)).to_le_bytes());
    out.extend_from_slice(&m.tempo.clamp(32, 999).to_le_bytes());
    let mut table = [0u8; 256];
    for (i, &o) in orders.iter().enumerate() {
        table[i] = o.min(255) as u8;
    }
    out.extend_from_slice(&table);
    if m.global_volume < 128 {
        lost.add("the song's global volume (XM starts at full)");
    }
    for p in m.patterns.iter().take(npat) {
        let rows = p.rows.clamp(1, 256);
        if p.rows > 256 {
            lost.many("pattern rows past 256", p.rows - 256);
        }
        let mut body = Vec::new();
        for row in 0..rows {
            for ch in 0..nch {
                let c = p.cell(row, ch);
                let note = match c.note {
                    n @ 13..=108 => n - 12,
                    NOTE_NONE => 0,
                    NOTE_OFF => 97,
                    NOTE_CUT | NOTE_FADE => {
                        lost.add("note cuts and fades (made note offs)");
                        97
                    }
                    _ => {
                        lost.add("notes out of FastTracker 2's range");
                        0
                    }
                };
                let instr = if m.flags.instruments { c.instr.min(128) } else { sample_slot(m, c.instr, c.note).min(128) as u8 };
                let vol = xm_volume(&c, m.kind, lost);
                let (cmd, param) = mod_family(&c, m.kind, true, lost);
                let fields = [note, instr, vol, cmd, param];
                if fields.iter().all(|&f| f != 0) {
                    body.extend_from_slice(&fields);
                } else {
                    let mask = fields.iter().enumerate().fold(0x80u8, |a, (i, &f)| if f != 0 { a | 1 << i } else { a });
                    body.push(mask);
                    body.extend(fields.iter().filter(|&&f| f != 0));
                }
            }
        }
        out.extend_from_slice(&9u32.to_le_bytes());
        out.push(0);
        out.extend_from_slice(&(rows as u16).to_le_bytes());
        out.extend_from_slice(&(body.len() as u16).to_le_bytes());
        out.extend_from_slice(&body);
    }
    for ins in &instruments {
        // Its samples: the keymap's, in order, as its own (XM's instruments own theirs).
        let mut own: Vec<u16> = Vec::new();
        for &(_, s) in &ins.keymap {
            if s > 0 && usize::from(s) <= m.samples.len() && !own.contains(&s) {
                if own.len() < 16 {
                    own.push(s);
                } else {
                    lost.add("samples past an instrument's 16");
                }
            }
        }
        if ins.keymap.iter().enumerate().any(|(i, &(n, s))| s > 0 && usize::from(n) != i) {
            lost.add("keymaps that play other notes");
        }
        for e in [&ins.pitch_env] {
            if e.enabled {
                lost.add("pitch and filter envelopes");
            }
        }
        if ins.nna != Nna::Cut || ins.dct != 0 {
            lost.add("new note actions and duplicate checks");
        }
        let start = out.len();
        out.extend_from_slice(&(if own.is_empty() { 29u32 } else { 263 }).to_le_bytes());
        put_text(&mut out, &ins.name, 22, 0);
        out.push(0);
        out.extend_from_slice(&(own.len() as u16).to_le_bytes());
        if own.is_empty() {
            continue;
        }
        out.extend_from_slice(&40u32.to_le_bytes());
        for k in 12..108 {
            let s = ins.keymap.get(k).map_or(0, |e| e.1);
            out.push(own.iter().position(|&o| o == s).unwrap_or(0) as u8);
        }
        let (vp, vn, vs, vls, vle, vf) = xm_env(&ins.volume_env, false, lost);
        let (pp, pn, ps, pls, ple, pf) = xm_env(&ins.pan_env, true, lost);
        out.extend_from_slice(&vp);
        out.extend_from_slice(&pp);
        out.extend_from_slice(&[vn, pn, vs, vls, vle, ps, pls, ple, vf, pf]);
        let vib = m.samples[usize::from(own[0]) - 1].vibrato;
        out.extend_from_slice(&[vib.kind.min(3), vib.sweep, vib.depth, vib.speed]);
        out.extend_from_slice(&((ins.fadeout / 2).min(0xFFF) as u16).to_le_bytes());
        out.extend(std::iter::repeat_n(0u8, 263 - (out.len() - start)));
        let waves: Vec<Wave> = own.iter().map(|&s| mono(&m.samples[usize::from(s) - 1], m.samples[usize::from(s) - 1].wave.bits)).collect();
        for (k, &s) in own.iter().enumerate() {
            let smp = &m.samples[usize::from(s) - 1];
            let w = &waves[k];
            if smp.wave.channels > 1 {
                lost.add("stereo samples (mixed to mono)");
            }
            if smp.sustain.on() {
                lost.add("sustain loops");
            }
            let b = if w.bits == 8 { 1u32 } else { 2 };
            let lp = smp.looped.clamped(w.frames() as u32);
            out.extend_from_slice(&(w.frames() as u32 * b).to_le_bytes());
            out.extend_from_slice(&(lp.start * b).to_le_bytes());
            out.extend_from_slice(&((lp.end - lp.start) * b).to_le_bytes());
            out.push(smp.volume.min(64));
            let (rel, ft) = relative_of(smp.c5);
            out.push(ft as u8);
            let kind = match lp.kind {
                LoopKind::Off => 0,
                LoopKind::Forward => 1,
                LoopKind::PingPong => 2,
            };
            out.push(kind | if b == 2 { 0x10 } else { 0 });
            out.push(smp.pan.unwrap_or(128));
            out.push(rel as u8);
            out.push(0);
            put_text(&mut out, &smp.name, 22, 0);
        }
        for w in &waves {
            let mut last = 0i16;
            for &v in &w.pcm {
                if w.bits == 8 {
                    let x = i16::from((v >> 8) as i8);
                    out.push((x - last) as u8);
                    last = x;
                } else {
                    out.extend_from_slice(&v.wrapping_sub(last).to_le_bytes());
                    last = v;
                }
            }
        }
    }
    out
}

// ---- IT ----

fn it_volume(c: &Cell, from: Kind, lost: &mut Lost) -> u8 {
    const PORTA: [u8; 10] = [0, 1, 4, 8, 16, 32, 64, 96, 128, 255];
    let x = c.volp;
    match c.vol {
        Vol::None => 255,
        Vol::Volume => x.min(64),
        Vol::FineVolUp => 65 + x.min(9),
        Vol::FineVolDown => 75 + x.min(9),
        Vol::VolSlideUp => 85 + x.min(9),
        Vol::VolSlideDown => 95 + x.min(9),
        Vol::PortaDown => 105 + x.min(9),
        Vol::PortaUp => 115 + x.min(9),
        Vol::Panning => 128 + x.min(64),
        Vol::TonePorta => {
            let speed = if from == Kind::Xm { u16::from(x) * 16 } else { u16::from(x) };
            193 + PORTA.iter().enumerate().min_by_key(|(_, &p)| (i32::from(p) - i32::from(speed)).abs()).map_or(0, |(i, _)| i as u8)
        }
        Vol::VibratoDepth => 203 + x.min(9),
        _ => {
            lost.add("volume column vibrato speeds, panning slides and offsets");
            255
        }
    }
}

fn it_env(out: &mut Vec<u8>, e: &Envelope, lost: &mut Lost) {
    if e.points.len() > 25 {
        lost.add("envelope points past 25");
    }
    let n = e.points.len().min(25) as u8;
    let (lb, le) = e.looped.unwrap_or((0, 0));
    let (sb, se) = e.sustain.unwrap_or((0, 0));
    out.push(u8::from(e.enabled && n > 0) | u8::from(e.looped.is_some()) << 1 | u8::from(e.sustain.is_some()) << 2 | u8::from(e.carry) << 3 | if e.filter { 0x80 } else { 0 });
    out.extend_from_slice(&[n, lb.min(n), le.min(n), sb.min(n), se.min(n)]);
    for i in 0..25 {
        let p = e.points.get(i).copied().unwrap_or_default();
        out.push(p.value as u8);
        out.extend_from_slice(&p.tick.to_le_bytes());
    }
    out.push(0);
}

fn write_it(m: &Module, lost: &mut Lost) -> Vec<u8> {
    let nch = m.channels.len().min(64);
    let orders = played_orders(m, true, 255, lost);
    let nins = if m.flags.instruments { m.instruments.len().min(255) } else { 0 };
    let nsmp = m.samples.len().min(255);
    let npat = m.patterns.len().min(240);
    if m.samples.len() > 255 {
        lost.many("samples past 255", m.samples.len() - 255);
    }
    if m.patterns.len() > 240 {
        lost.many("patterns past 240", m.patterns.len() - 240);
    }
    let message: Vec<u8> = m.message.replace('\n', "\r").bytes().filter(u8::is_ascii).chain([0]).collect();
    let mut out = b"IMPM".to_vec();
    put_text(&mut out, &m.title, 26, 0);
    let t = m.time;
    out.push(t.rows_per_beat.min(255) as u8);
    out.push(t.rows_per_measure.min(255) as u8);
    out.extend_from_slice(&((orders.len() + 1) as u16).to_le_bytes());
    out.extend_from_slice(&(nins as u16).to_le_bytes());
    out.extend_from_slice(&(nsmp as u16).to_le_bytes());
    out.extend_from_slice(&(npat as u16).to_le_bytes());
    out.extend_from_slice(&0x0214u16.to_le_bytes());
    out.extend_from_slice(&0x0214u16.to_le_bytes());
    let f = m.flags;
    let flags = 1u16 | 2 | u16::from(f.instruments) << 2 | u16::from(f.linear) << 3 | u16::from(f.old_effects) << 4 | u16::from(f.compat_gxx) << 5;
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&(u16::from(message.len() > 1)).to_le_bytes());
    out.push(m.global_volume.min(128));
    out.push(m.mix_volume.min(128));
    out.push(m.speed.max(1));
    if m.tempo > 255 {
        lost.add("a tempo over 255");
    }
    out.push(m.tempo.clamp(32, 255) as u8);
    out.push(m.separation.min(128));
    out.push(0);
    out.extend_from_slice(&(message.len() as u16).to_le_bytes());
    let msg_ptr_at = out.len();
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&[0; 4]);
    for i in 0..64 {
        out.push(match m.channels.get(i) {
            Some(c) if c.surround => 100 | if c.muted { 0x80 } else { 0 },
            Some(c) => ((u16::from(c.pan) * 64 + 127) / 255) as u8 | if c.muted { 0x80 } else { 0 },
            None => 32 | 0x80,
        });
    }
    for i in 0..64 {
        out.push(m.channels.get(i).map_or(64, |c| c.volume.min(64)));
    }
    if m.channels.len() > 64 {
        lost.many("channels past 64", m.channels.len() - 64);
    }
    for &o in &orders {
        out.push(if o == ORDER_SKIP { 254 } else { o.min(239) as u8 });
    }
    out.push(255);
    let table_at = out.len();
    out.extend(std::iter::repeat_n(0u8, (nins + nsmp + npat) * 4));
    let set = |out: &mut Vec<u8>, k: usize, at: usize| out[table_at + k * 4..table_at + k * 4 + 4].copy_from_slice(&(at as u32).to_le_bytes());
    if message.len() > 1 {
        let at = out.len();
        out[msg_ptr_at..msg_ptr_at + 4].copy_from_slice(&(at as u32).to_le_bytes());
        out.extend_from_slice(&message);
    }
    for (i, ins) in m.instruments.iter().take(nins).enumerate() {
        let at = out.len();
        set(&mut out, i, at);
        out.extend_from_slice(b"IMPI");
        put_text(&mut out, &ins.filename, 12, 0);
        out.push(0);
        out.push(ins.nna as u8);
        out.push(ins.dct.min(3));
        out.push(ins.dca.min(2));
        out.extend_from_slice(&((ins.fadeout / 64).min(1024) as u16).to_le_bytes());
        out.push(ins.pitch_pan_separation as u8);
        out.push(ins.pitch_pan_center);
        out.push(ins.global_volume.min(128));
        out.push(ins.pan.map_or(32 | 0x80, |p| ((u16::from(p) * 64 + 127) / 255) as u8));
        out.push(ins.random_volume.min(100));
        out.push(ins.random_pan.min(64));
        out.extend_from_slice(&0x0214u16.to_le_bytes());
        out.push(0);
        out.push(0);
        put_text(&mut out, &ins.name, 26, 0);
        out.push(ins.cutoff.map_or(0, |c| c | 0x80));
        out.push(ins.resonance.map_or(0, |r| r | 0x80));
        match ins.midi {
            Some(o) => {
                out.push(o.channel + 1);
                out.push(o.program.unwrap_or(0xFF));
                out.extend_from_slice(&o.bank.map_or(0xFFFF, |b| b.min(16383)).to_le_bytes());
            }
            None => out.extend_from_slice(&[0, 0xFF, 0xFF, 0xFF]),
        }
        for k in 0..120 {
            let (n, s) = ins.keymap.get(k).copied().unwrap_or((k as u8, 0));
            out.push(n.min(119));
            out.push(s.min(255) as u8);
        }
        it_env(&mut out, &ins.volume_env, lost);
        it_env(&mut out, &ins.pan_env, lost);
        it_env(&mut out, &ins.pitch_env, lost);
        out.extend_from_slice(&[0; 4]);
    }
    let mut data_ptrs = Vec::new();
    for (i, s) in m.samples.iter().take(nsmp).enumerate() {
        let at = out.len();
        set(&mut out, nins + i, at);
        // Its header as an ITS file's, the data placed after.
        let (head, _) = crate::samplefile::write(s, "its");
        out.extend_from_slice(&head[..80]);
        data_ptrs.push((at + 72, head[80..].to_vec(), !s.wave.is_empty()));
    }
    for (pi, p) in m.patterns.iter().take(npat).enumerate() {
        let at = out.len();
        set(&mut out, nins + nsmp + pi, at);
        let rows = p.rows.clamp(1, 200);
        if p.rows > 200 {
            lost.many("pattern rows past 200", p.rows - 200);
        }
        let mut body = Vec::new();
        for row in 0..rows {
            for ch in 0..nch {
                let mut c = p.cell(row, ch);
                if c.fx == Fx::Volume {
                    // MOD's Cxx: the volume column.
                    if c.vol == Vol::None {
                        c.vol = Vol::Volume;
                        c.volp = c.param.min(64);
                        c.fx = Fx::None;
                    } else {
                        lost.add("a volume effect beside a volume");
                        c.fx = Fx::None;
                    }
                }
                if c.fx == Fx::KeyOff && c.note == NOTE_NONE && c.param == 0 {
                    c.note = NOTE_OFF;
                    c.fx = Fx::None;
                }
                let note = match c.note {
                    NOTE_NONE => None,
                    n @ NOTE_MIN..=NOTE_MAX => Some(n - 1),
                    NOTE_OFF => Some(255),
                    NOTE_CUT => Some(254),
                    _ => Some(246),
                };
                let vol = it_volume(&c, m.kind, lost);
                let (cmd, param) = s3m_family(&c, m.kind, true, lost);
                let mut mask = 0u8;
                if note.is_some() {
                    mask |= 1;
                }
                if c.instr != 0 {
                    mask |= 2;
                }
                if vol != 255 {
                    mask |= 4;
                }
                if cmd != 0 {
                    mask |= 8;
                }
                if mask == 0 {
                    continue;
                }
                body.push((ch as u8 + 1) | 0x80);
                body.push(mask);
                if let Some(n) = note {
                    body.push(n);
                }
                if c.instr != 0 {
                    body.push(c.instr);
                }
                if vol != 255 {
                    body.push(vol);
                }
                if cmd != 0 {
                    body.push(cmd);
                    body.push(param);
                }
            }
            body.push(0);
        }
        out.extend_from_slice(&(body.len() as u16).to_le_bytes());
        out.extend_from_slice(&(rows as u16).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&body);
    }
    for (ptr, data, has) in data_ptrs {
        if !has {
            continue;
        }
        let at = out.len();
        out[ptr..ptr + 4].copy_from_slice(&(at as u32).to_le_bytes());
        out.extend_from_slice(&data);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn song(kind: Kind, instruments: bool) -> Module {
        let mut m = Module::new(4);
        m.kind = kind;
        m.title = "export test".into();
        m.speed = 5;
        m.tempo = 130;
        let pcm: Vec<i16> = (0..2000).map(|i| (((i as f32) * 0.1).sin() * 16000.0) as i16).collect();
        m.samples = vec![Sample { name: "sine".into(), wave: Arc::new(Wave::new(16, 1, pcm.clone())), looped: Loop::forward(200, 1800), c5: 8363, volume: 50, ..Sample::empty() }, Sample { name: "eight".into(), wave: Arc::new(Wave::new(8, 1, pcm.iter().map(|v| v & !0xFF).collect())), c5: 16726, ..Sample::empty() }];
        if instruments {
            m.flags.instruments = true;
            m.instruments = vec![Instrument::for_sample("sine", 1), Instrument::for_sample("eight", 2)];
        }
        let mut p = Pattern::new(64, 4);
        let fx = if kind.s3m_like() { (Fx::VolSlide, 0x04) } else { (Fx::VolSlide, 0x04) };
        *p.cell_mut(0, 0).unwrap() = Cell { note: 61, instr: 1, fx: fx.0, param: fx.1, ..Default::default() };
        *p.cell_mut(4, 1).unwrap() = Cell { note: 65, instr: 2, vol: Vol::Volume, volp: 32, ..Default::default() };
        *p.cell_mut(8, 2).unwrap() = Cell { note: 68, instr: 1, fx: Fx::Vibrato, param: 0x46, ..Default::default() };
        *p.cell_mut(16, 0).unwrap() = Cell { note: NOTE_CUT, ..Default::default() };
        m.patterns = vec![Arc::new(p)];
        m.orders = vec![0, 0];
        m.tidy();
        m
    }

    /// The song read back plays as it did, frame for frame give or take
    /// the small things a format changes (8-bit samples, finetune).
    fn same_sound(a: &Module, b: &Module, tolerance: f32, what: Format) {
        let ra = crate::player::render(Arc::new(a.clone()), 22_050, 8.0, 0.0);
        let rb = crate::player::render(Arc::new(b.clone()), 22_050, 8.0, 0.0);
        let n = ra.len().min(rb.len());
        assert!((ra.len() as i64 - rb.len() as i64).abs() < 4096, "{} {}", ra.len(), rb.len());
        let err: f32 = (0..n).map(|i| (ra[i] - rb[i]).powi(2)).sum::<f32>() / n as f32;
        let pow: f32 = ra[..n].iter().map(|v| v * v).sum::<f32>() / n as f32;
        assert!(err < pow * tolerance, "{what:?}: {err} vs {pow}");
    }

    #[test]
    fn every_format_reads_back_and_plays_the_same() {
        for (to, kind, instruments) in [(Format::It, Kind::It, true), (Format::It, Kind::It, false), (Format::Xm, Kind::Xm, true), (Format::S3m, Kind::S3m, false), (Format::Mod, Kind::Mod, false)] {
            let mut m = song(kind, instruments);
            // As the format keeps them.
            if matches!(to, Format::Mod | Format::S3m) {
                m.flags.linear = false;
            }
            if to == Format::Mod {
                m.samples[1].c5 = 8363;
            }
            if matches!(to, Format::Mod | Format::Xm) {
                m.mix_volume = 32;
                for (i, c) in m.channels.iter_mut().enumerate() {
                    c.pan = if to == Format::Mod { crate::formats::common::amiga_pan(i) } else { 128 };
                }
            }
            if to == Format::S3m {
                for c in &mut m.channels {
                    c.pan = c.pan / 17 * 17;
                }
            }
            let e = export(&m, to);
            let back = crate::load(&e.bytes, &format!("x.{}", to.extension())).unwrap_or_else(|err| panic!("{to:?}: {err}"));
            assert_eq!(back.title, "export test", "{to:?}");
            assert_eq!(back.orders, vec![0, 0], "{to:?}");
            let p = &back.patterns[0];
            assert_eq!(p.cell(0, 0).note, 61, "{to:?}");
            assert_eq!(p.cell(8, 2).fx, Fx::Vibrato, "{to:?}");
            assert_eq!(p.cell(8, 2).param, 0x46, "{to:?}");
            if to != Format::Mod {
                assert_eq!((back.speed, back.tempo), (5, 130), "{to:?}");
            }
            same_sound(&m, &back, if to == Format::Mod { 0.05 } else { 0.01 }, to);
        }
    }

    #[test]
    fn what_can_t_be_kept_is_said() {
        let mut m = song(Kind::It, true);
        m.effects.push(EffectSlot::new(EffectKind::Reverb));
        let p = Arc::make_mut(&mut m.patterns[0]);
        *p.cell_mut(20, 3).unwrap() = Cell { fx: Fx::Panbrello, param: 0x44, ..Default::default() };
        let e = export(&m, Format::Mod);
        let lost: Vec<&str> = e.lost.iter().map(|(w, _)| w.as_str()).collect();
        assert!(lost.contains(&"effect slots"), "{lost:?}");
        assert!(lost.iter().any(|l| l.starts_with("Panbrello")), "{lost:?}");
        assert!(lost.iter().any(|l| l.starts_with("instruments")), "{lost:?}");
        // IT keeps it all but the effect slot.
        let e = export(&m, Format::It);
        assert_eq!(e.lost, vec![("effect slots".to_string(), 1)]);
    }
}
