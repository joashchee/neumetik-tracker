//! The module every format is read into, and UMF's: one model wide
//! enough for all of them. Patterns of cells (a note, an instrument, a
//! volume column and an effect), the order they play in, samples (the
//! waveform and how it plays: loops, tuning, volume) and, for the
//! formats that have them, instruments (a keymap from notes to samples,
//! and envelopes).
//!
//! Effects are kept in one command set (`Fx`), each format's mapped to
//! it as it's read, and `Kind` says whose rules they follow where the
//! trackers disagree (a volume slide's fine form, effect memory, how
//! far a porta goes), the way the format itself played them.

use std::sync::Arc;

/// No note in the cell.
pub const NOTE_NONE: u8 = 0;
/// The lowest note, C-0; notes run to `NOTE_MAX`, B-9. Middle C, C-5, is 61.
pub const NOTE_MIN: u8 = 1;
pub const NOTE_MAX: u8 = 120;
pub const NOTE_MIDDLE_C: u8 = 61;
/// The note fades out (IT's ~~~).
pub const NOTE_FADE: u8 = 253;
/// The note stops at once (^^^).
pub const NOTE_CUT: u8 = 254;
/// The note is released: its envelopes go past their sustain (===).
pub const NOTE_OFF: u8 = 255;

/// An order entry that's passed over (+++).
pub const ORDER_SKIP: u16 = 0xFFFE;
/// The song ends here (---).
pub const ORDER_END: u16 = 0xFFFF;

/// Whose rules the effects follow where the trackers disagree. Every
/// format plays as one of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Kind {
    /// ProTracker and the Amiga's: Amiga periods, no effect memory for
    /// slides, fine slides as E-commands.
    #[default]
    Mod,
    /// Scream Tracker 3: shared effect memory, fine slides in the
    /// parameter (DxF, EFx, EEx).
    S3m,
    /// FastTracker 2: MOD's commands with memory, instruments, envelopes.
    Xm,
    /// Impulse Tracker: S3M's commands, instruments with new note
    /// actions, envelopes, filters.
    It,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Mod => "mod",
            Kind::S3m => "s3m",
            Kind::Xm => "xm",
            Kind::It => "it",
        }
    }

    pub fn from_name(s: &str) -> Kind {
        match s {
            "s3m" => Kind::S3m,
            "xm" => Kind::Xm,
            "it" => Kind::It,
            _ => Kind::Mod,
        }
    }

    /// S3M and IT write fine slides into the parameter.
    pub fn s3m_like(self) -> bool {
        matches!(self, Kind::S3m | Kind::It)
    }
}

/// The volume column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum Vol {
    #[default]
    None = 0,
    /// 0 to 64.
    Volume,
    /// 0 to 64, left to right.
    Panning,
    VolSlideUp,
    VolSlideDown,
    FineVolUp,
    FineVolDown,
    VibratoSpeed,
    VibratoDepth,
    PanSlideLeft,
    PanSlideRight,
    TonePorta,
    PortaUp,
    PortaDown,
    /// The sample offset, in 256ths of... (OpenMPT's o: the offset's high part).
    Offset,
}

impl Vol {
    pub const ALL: [Vol; 15] = [
        Vol::None,
        Vol::Volume,
        Vol::Panning,
        Vol::VolSlideUp,
        Vol::VolSlideDown,
        Vol::FineVolUp,
        Vol::FineVolDown,
        Vol::VibratoSpeed,
        Vol::VibratoDepth,
        Vol::PanSlideLeft,
        Vol::PanSlideRight,
        Vol::TonePorta,
        Vol::PortaUp,
        Vol::PortaDown,
        Vol::Offset,
    ];

    pub fn from_u8(v: u8) -> Vol {
        Vol::ALL.get(usize::from(v)).copied().unwrap_or(Vol::None)
    }

    /// The letter it's shown with, before its value (v40, p32, c05...).
    pub fn letter(self) -> char {
        match self {
            Vol::None => '.',
            Vol::Volume => 'v',
            Vol::Panning => 'p',
            Vol::VolSlideUp => 'c',
            Vol::VolSlideDown => 'd',
            Vol::FineVolUp => 'a',
            Vol::FineVolDown => 'b',
            Vol::VibratoSpeed => 'u',
            Vol::VibratoDepth => 'h',
            Vol::PanSlideLeft => 'l',
            Vol::PanSlideRight => 'r',
            Vol::TonePorta => 'g',
            Vol::PortaUp => 'f',
            Vol::PortaDown => 'e',
            Vol::Offset => 'o',
        }
    }

    pub fn from_letter(c: char) -> Option<Vol> {
        Vol::ALL.iter().copied().find(|v| *v != Vol::None && v.letter() == c)
    }
}

/// The effect command set every format's effects are read into. The
/// parameter keeps the format's meaning, read with the module's `Kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum Fx {
    #[default]
    None = 0,
    Arpeggio,
    PortaUp,
    PortaDown,
    TonePorta,
    Vibrato,
    TonePortaVol,
    VibratoVol,
    Tremolo,
    /// 0 to 255, left to right.
    Panning,
    Offset,
    VolSlide,
    PositionJump,
    /// 0 to 64.
    Volume,
    /// The row to break to, as a number.
    PatternBreak,
    Retrig,
    Speed,
    Tempo,
    Tremor,
    /// MOD's and XM's Exy.
    ModEx,
    /// S3M's and IT's Sxy.
    S3mEx,
    ChannelVolume,
    ChannelVolSlide,
    /// 0 to 128.
    GlobalVolume,
    GlobalVolSlide,
    KeyOff,
    FineVibrato,
    Panbrello,
    /// XM's X1x (up) and X2x (down): the high nibble says which.
    ExtraFinePorta,
    PanningSlide,
    EnvelopePosition,
    Midi,
    SmoothMidi,
    /// A note delayed x ticks and cut y ticks after (OpenMPT's :xy).
    DelayCut,
    /// More of the effect before's parameter (OpenMPT's #xx).
    XParam,
    /// x semitones up after y ticks (Imago Orpheus, PolyTracker).
    NoteSlideUp,
    NoteSlideDown,
    /// Pitch slides once on the first tick (formats with no fine forms
    /// in their parameter).
    FinePortaUp,
    FinePortaDown,
    FineVolSlideUp,
    FineVolSlideDown,
    /// The sample's finetune, -8 to 7 in its low nibble (MOD's E5x).
    SetFinetune,
    /// Starts the song's SEL script xx on the channel (Face The Music):
    /// `Module::sel`. 40 and over stops it.
    Sel,
    /// JamCracker's phase: the channel's AM wave mixed with itself, the
    /// mix moving x (1 to F, the low nibble) a tick from the start; FF
    /// sets it back.
    Phase,
}

impl Fx {
    pub const ALL: [Fx; 44] = [
        Fx::None,
        Fx::Arpeggio,
        Fx::PortaUp,
        Fx::PortaDown,
        Fx::TonePorta,
        Fx::Vibrato,
        Fx::TonePortaVol,
        Fx::VibratoVol,
        Fx::Tremolo,
        Fx::Panning,
        Fx::Offset,
        Fx::VolSlide,
        Fx::PositionJump,
        Fx::Volume,
        Fx::PatternBreak,
        Fx::Retrig,
        Fx::Speed,
        Fx::Tempo,
        Fx::Tremor,
        Fx::ModEx,
        Fx::S3mEx,
        Fx::ChannelVolume,
        Fx::ChannelVolSlide,
        Fx::GlobalVolume,
        Fx::GlobalVolSlide,
        Fx::KeyOff,
        Fx::FineVibrato,
        Fx::Panbrello,
        Fx::ExtraFinePorta,
        Fx::PanningSlide,
        Fx::EnvelopePosition,
        Fx::Midi,
        Fx::SmoothMidi,
        Fx::DelayCut,
        Fx::XParam,
        Fx::NoteSlideUp,
        Fx::NoteSlideDown,
        Fx::FinePortaUp,
        Fx::FinePortaDown,
        Fx::FineVolSlideUp,
        Fx::FineVolSlideDown,
        Fx::SetFinetune,
        Fx::Sel,
        Fx::Phase,
    ];

    pub fn from_u8(v: u8) -> Fx {
        Fx::ALL.get(usize::from(v)).copied().unwrap_or(Fx::None)
    }

    /// UMF's letter for it, as the editor shows it: IT's where IT has
    /// the command, the rest after.
    pub fn letter(self) -> char {
        match self {
            Fx::None => '.',
            Fx::Speed => 'A',
            Fx::PositionJump => 'B',
            Fx::PatternBreak => 'C',
            Fx::VolSlide => 'D',
            Fx::PortaDown => 'E',
            Fx::PortaUp => 'F',
            Fx::TonePorta => 'G',
            Fx::Vibrato => 'H',
            Fx::Tremor => 'I',
            Fx::Arpeggio => 'J',
            Fx::VibratoVol => 'K',
            Fx::TonePortaVol => 'L',
            Fx::ChannelVolume => 'M',
            Fx::ChannelVolSlide => 'N',
            Fx::Offset => 'O',
            Fx::PanningSlide => 'P',
            Fx::Retrig => 'Q',
            Fx::Tremolo => 'R',
            Fx::S3mEx => 'S',
            Fx::Tempo => 'T',
            Fx::FineVibrato => 'U',
            Fx::GlobalVolume => 'V',
            Fx::GlobalVolSlide => 'W',
            Fx::Panning => 'X',
            Fx::Panbrello => 'Y',
            Fx::Midi => 'Z',
            Fx::SmoothMidi => '\\',
            Fx::DelayCut => ':',
            Fx::XParam => '#',
            Fx::Volume => 'c',
            Fx::ModEx => 'e',
            Fx::KeyOff => 'k',
            Fx::EnvelopePosition => 'l',
            Fx::ExtraFinePorta => 'x',
            Fx::NoteSlideUp => 'u',
            Fx::NoteSlideDown => 'n',
            Fx::FinePortaUp => 'f',
            Fx::FinePortaDown => 'd',
            Fx::FineVolSlideUp => 'a',
            Fx::FineVolSlideDown => 'b',
            Fx::SetFinetune => 't',
            Fx::Sel => 's',
            Fx::Phase => 'p',
        }
    }

    pub fn from_letter(c: char) -> Option<Fx> {
        Fx::ALL.iter().copied().find(|f| *f != Fx::None && f.letter() == c)
    }

    /// What it does, for the editor's tips.
    pub fn describe(self) -> &'static str {
        match self {
            Fx::None => "No effect",
            Fx::Arpeggio => "Arpeggio: the note, then x and y semitones above it, in turn",
            Fx::PortaUp => "Pitch slide up",
            Fx::PortaDown => "Pitch slide down",
            Fx::TonePorta => "Slide to the note",
            Fx::Vibrato => "Vibrato: x its speed, y its depth",
            Fx::TonePortaVol => "Slide to the note, and a volume slide",
            Fx::VibratoVol => "Vibrato, and a volume slide",
            Fx::Tremolo => "Tremolo: x its speed, y its depth",
            Fx::Panning => "Panning: 00 left, 80 the middle, FF right",
            Fx::Offset => "Start the sample from xx × 256 frames in",
            Fx::VolSlide => "Volume slide: x up, y down (xF and Fy once, finely)",
            Fx::PositionJump => "Jump to the order xx",
            Fx::Volume => "Volume: 00 to 40",
            Fx::PatternBreak => "Break to row xx of the next pattern",
            Fx::Retrig => "Play the note again every y ticks, the volume changed by x",
            Fx::Speed => "Ticks a row",
            Fx::Tempo => "Tempo, in beats a minute",
            Fx::Tremor => "Tremor: on x ticks, off y",
            Fx::ModEx => "Extended command",
            Fx::S3mEx => "Special command",
            Fx::ChannelVolume => "Channel volume: 00 to 40",
            Fx::ChannelVolSlide => "Channel volume slide",
            Fx::GlobalVolume => "Global volume: 00 to 80",
            Fx::GlobalVolSlide => "Global volume slide",
            Fx::KeyOff => "Release the note after xx ticks",
            Fx::FineVibrato => "Fine vibrato",
            Fx::Panbrello => "Panbrello: panning vibrato",
            Fx::ExtraFinePorta => "Extra-fine pitch slide (1x up, 2x down)",
            Fx::PanningSlide => "Panning slide",
            Fx::EnvelopePosition => "Set the envelopes' position",
            Fx::Midi => "MIDI macro (the filter by default)",
            Fx::SmoothMidi => "Smooth MIDI macro",
            Fx::DelayCut => "Delay the note x ticks, cut it after y",
            Fx::XParam => "More of the effect above's parameter",
            Fx::NoteSlideUp => "Slide up y semitones, x the speed",
            Fx::NoteSlideDown => "Slide down y semitones, x the speed",
            Fx::FinePortaUp => "Fine pitch slide up, once",
            Fx::FinePortaDown => "Fine pitch slide down, once",
            Fx::FineVolSlideUp => "Fine volume slide up, once",
            Fx::FineVolSlideDown => "Fine volume slide down, once",
            Fx::SetFinetune => "Set the sample's finetune",
            Fx::Sel => "Start the song's SEL script xx (Face The Music); 40 and up stops it",
            Fx::Phase => "Phase: the AM wave mixed with itself, moving x a tick (JamCracker); FF sets it back",
        }
    }
}

/// A cell of a pattern: one row of one channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Cell {
    /// `NOTE_NONE`, a note (`NOTE_MIN`..=`NOTE_MAX`), `NOTE_OFF`, `NOTE_CUT` or `NOTE_FADE`.
    pub note: u8,
    /// 1-based instrument (or sample, without instruments); 0 for none.
    pub instr: u8,
    pub vol: Vol,
    pub volp: u8,
    pub fx: Fx,
    pub param: u8,
}

impl Cell {
    pub fn is_empty(&self) -> bool {
        *self == Cell::default()
    }
}

/// Rows a beat and a measure: how the editor highlights them, and the
/// beat Modern tempo and swing count in (OpenMPT's).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TimeSig {
    pub rows_per_beat: u16,
    pub rows_per_measure: u16,
}

impl Default for TimeSig {
    fn default() -> TimeSig {
        TimeSig { rows_per_beat: 4, rows_per_measure: 16 }
    }
}

/// What an automation lane moves (Renoise's automation, OpenMPT's
/// parameter control events).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// The song's global volume, 0 to 128.
    GlobalVolume,
    /// Tempo, 32 to 512.
    Tempo,
    /// A channel's volume, 0 to 64.
    ChannelVolume(u8),
    /// A channel's panning, left to right.
    ChannelPan(u8),
    /// An effect slot's parameter.
    Effect { slot: u8, param: u8 },
}

/// An automation lane: values (0 to 1) at rows, joined by straight lines
/// tick by tick; before the first point and after the last, nothing's moved.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Lane {
    pub target: Option<Target>,
    /// (row, value), in row order.
    pub points: Vec<(u16, f32)>,
}

impl Lane {
    /// The value at a row and the fraction of it played, when the lane covers it.
    pub fn at(&self, row: f32) -> Option<f32> {
        let p = &self.points;
        let first = p.first()?;
        let last = p.last()?;
        if row < f32::from(first.0) || row > f32::from(last.0) {
            return None;
        }
        let i = p.iter().position(|&(r, _)| f32::from(r) > row).unwrap_or(p.len());
        if i == 0 || i == p.len() {
            return Some(if i == 0 { first.1 } else { last.1 });
        }
        let (a, b) = (p[i - 1], p[i]);
        let span = f32::from(b.0 - a.0).max(1.0);
        Some(a.1 + (b.1 - a.1) * (row - f32::from(a.0)) / span)
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Pattern {
    pub rows: usize,
    pub channels: usize,
    pub name: String,
    /// Row by row, `channels` cells a row.
    pub cells: Vec<Cell>,
    /// Its own rows a beat and a measure, over the song's.
    pub time: Option<TimeSig>,
    /// Its own swing, over the song's.
    pub swing: Option<u8>,
    pub automation: Vec<Lane>,
}

impl Pattern {
    pub fn new(rows: usize, channels: usize) -> Pattern {
        Pattern { rows, channels, cells: vec![Cell::default(); rows * channels], ..Default::default() }
    }

    /// The same pattern's settings (name, time, swing, automation) over
    /// other cells.
    fn shaped(&self, rows: usize, channels: usize) -> Pattern {
        Pattern { rows, channels, cells: vec![Cell::default(); rows * channels], name: self.name.clone(), time: self.time, swing: self.swing, automation: self.automation.clone() }
    }

    pub fn cell(&self, row: usize, ch: usize) -> Cell {
        if row < self.rows && ch < self.channels {
            self.cells[row * self.channels + ch]
        } else {
            Cell::default()
        }
    }

    pub fn cell_mut(&mut self, row: usize, ch: usize) -> Option<&mut Cell> {
        if row < self.rows && ch < self.channels {
            self.cells.get_mut(row * self.channels + ch)
        } else {
            None
        }
    }

    /// The same notes over `channels` channels, cut or padded.
    pub fn with_channels(&self, channels: usize) -> Pattern {
        let mut p = self.shaped(self.rows, channels);
        for r in 0..self.rows {
            for c in 0..channels.min(self.channels) {
                p.cells[r * channels + c] = self.cell(r, c);
            }
        }
        p
    }

    /// The same notes over `rows` rows, cut or padded.
    pub fn with_rows(&self, rows: usize) -> Pattern {
        let mut p = self.shaped(rows, self.channels);
        let n = rows.min(self.rows) * self.channels;
        p.cells[..n].copy_from_slice(&self.cells[..n]);
        p
    }
}

/// A sample's waveform: 16-bit points (8-bit ones shifted up, `bits`
/// says which they were), interleaved when there are two channels.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Wave {
    pub bits: u8,
    pub channels: u8,
    pub pcm: Vec<i16>,
}

impl Wave {
    pub fn new(bits: u8, channels: u8, pcm: Vec<i16>) -> Wave {
        let channels = channels.clamp(1, 2);
        let mut pcm = pcm;
        pcm.truncate(pcm.len() / usize::from(channels) * usize::from(channels));
        Wave { bits: if bits <= 8 { 8 } else { 16 }, channels, pcm }
    }

    pub fn frames(&self) -> usize {
        self.pcm.len() / usize::from(self.channels.max(1))
    }

    pub fn is_empty(&self) -> bool {
        self.pcm.is_empty()
    }

    /// Bytes it takes at its own depth.
    pub fn bytes(&self) -> usize {
        self.pcm.len() * if self.bits == 8 { 1 } else { 2 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum LoopKind {
    #[default]
    Off,
    Forward,
    PingPong,
}

impl LoopKind {
    pub fn from_u8(v: u8) -> LoopKind {
        match v {
            1 => LoopKind::Forward,
            2 => LoopKind::PingPong,
            _ => LoopKind::Off,
        }
    }
}

/// A loop, in frames: `end` is just past its last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Loop {
    pub kind: LoopKind,
    pub start: u32,
    pub end: u32,
}

impl Loop {
    pub fn on(&self) -> bool {
        self.kind != LoopKind::Off && self.end > self.start
    }

    pub fn forward(start: u32, end: u32) -> Loop {
        Loop { kind: LoopKind::Forward, start, end }
    }

    /// Held inside `frames`, off when nothing's left of it.
    pub fn clamped(self, frames: u32) -> Loop {
        let end = self.end.min(frames);
        if self.kind == LoopKind::Off || self.start >= end || end - self.start < 1 {
            Loop::default()
        } else {
            Loop { kind: self.kind, start: self.start, end }
        }
    }
}

/// The waveforms auto-vibrato follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct AutoVibrato {
    /// 0 sine, 1 ramp down, 2 square, 3 random, 4 ramp up.
    pub kind: u8,
    pub speed: u8,
    pub depth: u8,
    /// How quickly it reaches full depth (XM's sweep, IT's rate).
    pub sweep: u8,
}

/// A sample: its waveform, and how it plays.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Sample {
    pub name: String,
    /// The file it was loaded from, when the format keeps one (S3M, IT, XM...).
    pub filename: String,
    pub wave: Arc<Wave>,
    /// The rate it plays at for middle C (C-5), in hertz.
    pub c5: u32,
    /// 0 to 64.
    pub volume: u8,
    /// IT's sample global volume, 0 to 64.
    pub global_volume: u8,
    /// Its own panning, 0 to 255, when it sets one.
    pub pan: Option<u8>,
    pub looped: Loop,
    /// IT's sustain loop: played while the note's held.
    pub sustain: Loop,
    pub vibrato: AutoVibrato,
    /// Who made it, or where it's from.
    pub credit: String,
    /// Where it's sliced, in frames (Renoise's slices): each slice can
    /// become a sample of its own, a key each.
    pub slices: Vec<u32>,
    /// The synth it was made by, when it was: its waveform comes from
    /// these settings and can be made again from them.
    pub synth: Option<Synth>,
    /// An FM (OPL) instrument, when it's one: S3M's and Composer 670's
    /// AdLib instruments. It has no waveform; neumetik-synth plays it.
    pub fm: Option<FmPatch>,
    /// A synth sound, when it's one: OctaMED's synth and hybrid
    /// instruments, their waveforms stepped through by its sequences
    /// (a hybrid's waveform 0 is `wave`).
    pub program: Option<Arc<Program>>,
}

pub use neumetik_synth::opl::Patch as FmPatch;
pub use neumetik_synth::puma::PumaSound;
pub use neumetik_synth::sel::{Line as SelLine, Program as SelProgram};
pub use neumetik_synth::sequence::SynthSound;
pub use neumetik_synth::musicline::Song as MlSong;
pub use neumetik_synth::ptk::Song as PtkSong;

/// A song that plays by its own replayer (neumetik-synth's `replay`): the
/// synth and chip trackers' (AHX, HivelyTracker...). Two are the same
/// when they're the same format and file.
#[derive(Clone)]
pub struct Tune(pub Arc<dyn neumetik_synth::replay::Tune>);

impl PartialEq for Tune {
    fn eq(&self, other: &Tune) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || (self.0.format() == other.0.format() && self.0.file() == other.0.file())
    }
}

impl std::fmt::Debug for Tune {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", &*self.0)
    }
}

/// The synth sound a sample plays: OctaMED's, Pumatracker's, or
/// JamCracker's AM wave.
#[derive(Clone, Debug, PartialEq)]
pub enum Sound {
    Med(SynthSound),
    Puma(PumaSound),
    /// A JamCracker AM instrument: the sample holds its bytes (the cycle
    /// twice, then each channel's copy); its waves are the copies, then
    /// the cycle's mixes, chosen by the channel's phase (`Fx::Phase`).
    Jam,
}

/// A synth sound and its waveforms, made ready to play.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub sound: Sound,
    pub waves: Vec<Arc<Wave>>,
    /// Each waveform's loop, where they aren't looped cycles
    /// (Pumatracker's samples play once); none: each loops whole.
    pub loops: Vec<Loop>,
    /// The song's samples (0-based) its first waveforms are, when they
    /// are (Pumatracker's ten).
    pub sources: Vec<u16>,
}

impl Program {
    /// An OctaMED sound's waveforms made into looped waves; a hybrid's
    /// first is `sample`'s.
    pub fn new(sound: SynthSound, sample: &Arc<Wave>) -> Program {
        let waves = sound
            .waveforms
            .iter()
            .enumerate()
            .map(|(i, w)| if i == 0 && sound.hybrid { sample.clone() } else { Arc::new(Wave::new(8, 1, w.iter().map(|&x| i16::from(x) << 8).collect())) })
            .collect();
        Program { sound: Sound::Med(sound), waves, loops: Vec::new(), sources: Vec::new() }
    }

    /// A Pumatracker instrument: its first waveforms the song's samples
    /// `sources` names (taken from `samples`), the replayer's built-in
    /// cycles after them.
    pub fn puma(sound: PumaSound, sources: Vec<u16>, samples: &[Sample]) -> Program {
        let cycles = neumetik_synth::puma::built_in();
        let mut p = Program { sound: Sound::Puma(sound), waves: Vec::with_capacity(sources.len() + cycles.len()), loops: Vec::new(), sources };
        p.waves.resize(p.sources.len(), Arc::default());
        p.loops.resize(p.sources.len(), Loop::default());
        for c in cycles {
            p.waves.push(Arc::new(Wave::new(8, 1, c.iter().map(|&x| i16::from(x) << 8).collect())));
            p.loops.push(Loop::forward(0, 32));
        }
        p.attach(samples);
        p
    }

    /// A JamCracker AM instrument from its sample's bytes: each
    /// channel's copy, then the cycle mixed with itself at each shift,
    /// every one looped whole.
    pub fn jam(sample: &Wave) -> Program {
        let bytes: Vec<i8> = sample.pcm.iter().map(|&x| (x >> 8) as i8).collect();
        let waves = neumetik_synth::jam::waves(&bytes).into_iter().map(|w| Arc::new(Wave::new(8, 1, w.iter().map(|&x| i16::from(x) << 8).collect()))).collect();
        Program { sound: Sound::Jam, waves, loops: Vec::new(), sources: Vec::new() }
    }

    /// Its waveforms that are the song's samples, taken from them again
    /// (once they're read).
    pub fn attach(&mut self, samples: &[Sample]) {
        for (k, &s) in self.sources.iter().enumerate() {
            if let (Some(smp), Some(w)) = (samples.get(usize::from(s)), self.waves.get_mut(k)) {
                *w = smp.wave.clone();
                self.loops[k] = smp.looped;
            }
        }
    }

    /// Its waveform 0 is the sample's own (an OctaMED hybrid).
    pub fn hybrid(&self) -> bool {
        matches!(&self.sound, Sound::Med(s) if s.hybrid)
    }
}

/// A synth's settings: a sample made from oscillators instead of
/// recorded (neumetik-synth's single-cycle synths).
pub use neumetik_synth::cycle::Cycle as Synth;

impl Sample {
    pub fn empty() -> Sample {
        Sample { c5: 8363, volume: 64, global_volume: 64, ..Default::default() }
    }

    pub fn frames(&self) -> usize {
        self.wave.frames()
    }

    /// It plays something: a waveform, an FM instrument or a synth sound.
    pub fn sounds(&self) -> bool {
        !self.wave.is_empty() || self.fm.is_some() || self.program.as_ref().is_some_and(|p| p.waves.iter().any(|w| !w.is_empty()))
    }
}

/// An envelope point: a tick, and a value (volume 0 to 64; panning and
/// pitch -32 to 32).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct EnvPoint {
    pub tick: u16,
    pub value: i8,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Envelope {
    pub enabled: bool,
    pub points: Vec<EnvPoint>,
    /// Loop from point to point.
    pub looped: Option<(u8, u8)>,
    /// Sustain from point to point (the same point to hold on one).
    pub sustain: Option<(u8, u8)>,
    /// Carries on from the note before (IT).
    pub carry: bool,
    /// The pitch envelope drives the filter instead (IT).
    pub filter: bool,
    /// OpenMPT's release node: once the note's let go, the envelope goes
    /// on from this point, its values from there relative to where it was.
    pub release: Option<u8>,
}

/// What happens to a note playing when another comes (IT).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Nna {
    #[default]
    Cut,
    Continue,
    Off,
    Fade,
}

impl Nna {
    pub fn from_u8(v: u8) -> Nna {
        match v {
            1 => Nna::Continue,
            2 => Nna::Off,
            3 => Nna::Fade,
            _ => Nna::Cut,
        }
    }
}

/// An instrument: which sample each note plays, and its envelopes.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Instrument {
    pub name: String,
    pub filename: String,
    /// For each note (index 0 is C-0): the note played and the 1-based
    /// sample (0 none).
    pub keymap: Vec<(u8, u16)>,
    pub volume_env: Envelope,
    pub pan_env: Envelope,
    pub pitch_env: Envelope,
    /// Taken off a fading note's volume each tick, of 65,536.
    pub fadeout: u32,
    /// 0 to 128.
    pub global_volume: u8,
    pub pan: Option<u8>,
    pub nna: Nna,
    /// Duplicate check: 0 off, 1 note, 2 sample, 3 instrument.
    pub dct: u8,
    /// Duplicate action: 0 cut, 1 off, 2 fade.
    pub dca: u8,
    pub pitch_pan_separation: i8,
    pub pitch_pan_center: u8,
    /// Random variation, in percent of volume and of panning.
    pub random_volume: u8,
    pub random_pan: u8,
    /// The resonant filter's cutoff and resonance (0 to 127), when set.
    pub cutoff: Option<u8>,
    pub resonance: Option<u8>,
    /// Its notes sent to a MIDI output too (IT's and OpenMPT's MIDI
    /// instruments).
    pub midi: Option<MidiOut>,
    /// Its notes go through this effect slot (OpenMPT's instrument plugin).
    pub effect: Option<u8>,
    /// A phrase its notes play (Renoise's): a column of cells played a
    /// row a row from the note, transposed to it.
    pub phrase: Option<Phrase>,
}

/// Where an instrument's notes go over MIDI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct MidiOut {
    /// 0 to 15.
    pub channel: u8,
    /// A program change before its first note, when set (0 to 127).
    pub program: Option<u8>,
    /// Bank select (0 to 16383), when set.
    pub bank: Option<u16>,
}

/// A phrase: cells (one column) an instrument's note plays in turn, a
/// row each, as if written under it, transposed by how far the note is
/// from `base`. Notes written in the pattern take over from it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Phrase {
    pub cells: Vec<Cell>,
    /// Back to its start at its end (or it stops there).
    pub looped: bool,
    /// The note it plays as written.
    pub base: u8,
}

impl Instrument {
    /// An instrument whose every note plays `sample` unchanged.
    pub fn for_sample(name: &str, sample: u16) -> Instrument {
        Instrument {
            name: name.to_string(),
            keymap: (0..120).map(|n| (n as u8, sample)).collect(),
            global_volume: 128,
            ..Default::default()
        }
    }

    /// The (note, sample) a note (`NOTE_MIN`..) plays.
    pub fn map(&self, note: u8) -> (u8, u16) {
        let i = usize::from(note.saturating_sub(1));
        self.keymap.get(i).map(|&(n, s)| (n + 1, s)).unwrap_or((note, 0))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Channel {
    /// 0 to 255, left to right.
    pub pan: u8,
    /// 0 to 64.
    pub volume: u8,
    /// Heard the same in both ears, the far side inverted (IT's surround).
    pub surround: bool,
    pub muted: bool,
    pub name: String,
    /// Its sound goes through this effect slot (OpenMPT's channel plugin).
    pub effect: Option<u8>,
}

impl Default for Channel {
    fn default() -> Channel {
        Channel { pan: 128, volume: 64, surround: false, muted: false, name: String::new(), effect: None }
    }
}

/// How long a tick is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TempoMode {
    /// 2.5 / tempo seconds: every tracker's.
    #[default]
    Classic,
    /// 1 / tempo seconds: tempo is ticks a second (OpenMPT's alternative).
    Alternative,
    /// Tempo is beats a minute whatever the speed: a row is 60 / (tempo ×
    /// rows a beat) seconds (OpenMPT's modern tempo).
    Modern,
}

impl TempoMode {
    pub fn from_u8(v: u8) -> TempoMode {
        match v {
            1 => TempoMode::Alternative,
            2 => TempoMode::Modern,
            _ => TempoMode::Classic,
        }
    }
}

/// An order list of its own: a song of the module's (OpenMPT's
/// sequences, MED's sections).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Sequence {
    pub name: String,
    pub orders: Vec<u16>,
    pub restart: u16,
}

/// A place marked to go back to: an order, or a pattern's row.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Bookmark {
    pub name: String,
    /// The order it marks (an order list bookmark), or none.
    pub order: Option<u16>,
    /// The pattern it marks (a pattern bookmark), and the row.
    pub pattern: Option<u16>,
    pub row: u16,
}

/// The effects a slot can be: built in, so they play anywhere
/// (OpenMPT's DirectX Media Objects, Renoise's DSP effects).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum EffectKind {
    #[default]
    Reverb,
    Delay,
    Chorus,
    Flanger,
    Distortion,
    Compressor,
    Equalizer,
    Filter,
    Gargle,
}

impl EffectKind {
    pub const ALL: [EffectKind; 9] = [
        EffectKind::Reverb,
        EffectKind::Delay,
        EffectKind::Chorus,
        EffectKind::Flanger,
        EffectKind::Distortion,
        EffectKind::Compressor,
        EffectKind::Equalizer,
        EffectKind::Filter,
        EffectKind::Gargle,
    ];

    pub fn from_u8(v: u8) -> EffectKind {
        EffectKind::ALL.get(usize::from(v)).copied().unwrap_or_default()
    }

    pub fn name(self) -> &'static str {
        match self {
            EffectKind::Reverb => "Reverb",
            EffectKind::Delay => "Delay",
            EffectKind::Chorus => "Chorus",
            EffectKind::Flanger => "Flanger",
            EffectKind::Distortion => "Distortion",
            EffectKind::Compressor => "Compressor",
            EffectKind::Equalizer => "Equalizer",
            EffectKind::Filter => "Filter",
            EffectKind::Gargle => "Gargle",
        }
    }

    /// Its parameters' names and defaults (each 0 to 1).
    pub fn params(self) -> &'static [(&'static str, f32)] {
        match self {
            EffectKind::Reverb => &[("Room size", 0.6), ("Damping", 0.4), ("Width", 1.0), ("Pre-delay", 0.1)],
            EffectKind::Delay => &[("Left time", 0.3), ("Right time", 0.4), ("Feedback", 0.4), ("Ping-pong", 0.0), ("Damping", 0.3)],
            EffectKind::Chorus => &[("Rate", 0.2), ("Depth", 0.5), ("Delay", 0.4), ("Feedback", 0.1)],
            EffectKind::Flanger => &[("Rate", 0.15), ("Depth", 0.7), ("Delay", 0.1), ("Feedback", 0.6)],
            EffectKind::Distortion => &[("Drive", 0.5), ("Tone", 0.6), ("Level", 0.6)],
            EffectKind::Compressor => &[("Threshold", 0.6), ("Ratio", 0.3), ("Attack", 0.1), ("Release", 0.3), ("Make-up", 0.2)],
            EffectKind::Equalizer => &[("Low", 0.5), ("Mid", 0.5), ("Mid frequency", 0.5), ("High", 0.5)],
            EffectKind::Filter => &[("Cutoff", 0.7), ("Resonance", 0.2), ("High-pass", 0.0)],
            EffectKind::Gargle => &[("Rate", 0.3), ("Square", 0.0)],
        }
    }
}

/// An effect slot: an effect, its settings, and where its sound goes.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct EffectSlot {
    pub kind: EffectKind,
    pub name: String,
    /// Each 0 to 1, as `kind.params()` names them.
    pub params: Vec<f32>,
    /// How much of the effect is heard, 0 (none) to 1 (only it).
    pub mix: f32,
    pub bypass: bool,
    /// Into another slot after it (a chain), or the mix.
    pub output: Option<u8>,
}

impl EffectSlot {
    pub fn new(kind: EffectKind) -> EffectSlot {
        EffectSlot { kind, name: String::new(), params: kind.params().iter().map(|p| p.1).collect(), mix: if matches!(kind, EffectKind::Reverb | EffectKind::Delay | EffectKind::Chorus) { 0.35 } else { 1.0 }, bypass: false, output: None }
    }

    pub fn param(&self, i: usize) -> f32 {
        self.params.get(i).copied().unwrap_or_else(|| self.kind.params().get(i).map_or(0.0, |p| p.1)).clamp(0.0, 1.0)
    }
}

/// IT's and OpenMPT's MIDI macros: what Zxx and \xx send. SFx picks
/// which of the 16 parametered macros Z00 to Z7F use; Z80 to ZFF each
/// have their own. A macro is hex bytes and letters: z the value, n the
/// note, v the velocity, u the volume, x the panning, c the channel.
/// F0 F0 0n v is Neumetik's (and OpenMPT's) own: n 0 the filter's
/// cutoff, 1 its resonance, 2 its mode (v 0 low-pass, 10 high-pass);
/// n 80 and up an effect slot's parameter n - 80.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MidiMacros {
    /// 16 of them.
    pub parametered: Vec<String>,
    /// 128 of them, Z80 to ZFF.
    pub fixed: Vec<String>,
}

impl Default for MidiMacros {
    /// Impulse Tracker's: SF0 the cutoff; Z80 to Z8F the resonance.
    fn default() -> MidiMacros {
        let mut parametered = vec![String::new(); 16];
        parametered[0] = "F0F000z".into();
        let fixed = (0..128).map(|i| if i < 16 { format!("F0F001{:02X}", i * 8) } else { String::new() }).collect();
        MidiMacros { parametered, fixed }
    }
}

/// How a module plays, besides its effects' `Kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Flags {
    /// Pitch slides in steps of a semitone's 64th, not the Amiga's periods.
    pub linear: bool,
    /// Pitch kept within the Amiga's three octaves (ProTracker).
    pub amiga_limits: bool,
    /// Notes play instruments, not samples.
    pub instruments: bool,
    /// IT's old effects (vibrato, offset past the end...).
    pub old_effects: bool,
    /// IT's G shares its memory with E and F.
    pub compat_gxx: bool,
    /// Scream Tracker 3.00's volume slides, on every tick.
    pub fast_slides: bool,
    /// Ticks a row run at the speed of the old Amiga trackers' vertical
    /// blank, not by tempo (the 15-sample Soundtrackers).
    pub vblank: bool,
}

/// A whole module.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Module {
    pub title: String,
    pub artist: String,
    /// The song message, comments, or the samples' names read as one.
    pub message: String,
    /// The format's short name ("mod", "xm", "umf"...).
    pub format: String,
    /// The format in words ("ProTracker", "FastTracker 2"...).
    pub format_name: String,
    /// The tracker it was made with, when the file says ("Impulse Tracker 2.14").
    pub tracker: String,
    /// The file's SAUCE record, when it had one (`sauce.rs`).
    pub sauce: Option<crate::sauce::Sauce>,
    pub kind: Kind,
    pub flags: Flags,
    /// Ticks a row, and beats a minute (a tick is 2.5 / tempo seconds).
    pub speed: u8,
    pub tempo: u16,
    /// 0 to 128.
    pub global_volume: u8,
    /// The mix's level, 0 to 128 (S3M's and IT's master volume).
    pub mix_volume: u8,
    /// Stereo separation, 0 (mono) to 128.
    pub separation: u8,
    /// The order the song goes back to at its end.
    pub restart: u16,
    pub channels: Vec<Channel>,
    pub orders: Vec<u16>,
    pub patterns: Vec<Arc<Pattern>>,
    pub samples: Vec<Sample>,
    pub instruments: Vec<Instrument>,
    pub tempo_mode: TempoMode,
    /// Rows a beat and a measure, for the patterns that don't say.
    pub time: TimeSig,
    /// How far each beat's first row of a pair is held over its second,
    /// 0 (even) to 100 (all of it) percent.
    pub swing: u8,
    /// Every order list, when there's more than one; `orders` and
    /// `restart` are the one `sequence` names, as it's edited.
    pub sequences: Vec<Sequence>,
    pub sequence: usize,
    pub bookmarks: Vec<Bookmark>,
    /// (order, channel) slots of the pattern matrix that don't play
    /// (Renoise's muted matrix slots).
    pub slot_mutes: Vec<(u16, u16)>,
    pub effects: Vec<EffectSlot>,
    /// What Zxx and \xx send, when the song has its own (else IT's).
    pub macros: Option<MidiMacros>,
    /// Face The Music's SEL scripts, started by `Fx::Sel` and played by
    /// neumetik-synth's script machine.
    pub sel: Option<Arc<SelProgram>>,
    /// A Musicline Editor song, which plays by its own replayer
    /// (neumetik-synth's) rather than its patterns, which show it.
    pub musicline: Option<Arc<MlSong>>,
    /// A ProTrekkr or NoiseTrekker song, which plays by its own replayer
    /// (neumetik-synth's): its tracks' filters and effects, its synths and
    /// TB-303s; its patterns show it.
    pub ptk: Option<Arc<PtkSong>>,
    /// A synth or chip song (AHX, HivelyTracker...), which plays by its
    /// own replayer; its patterns show it, and each of its subsongs is
    /// one of `sequences`.
    pub tune: Option<Tune>,
    /// The song's cover: an ANSI art file (SAUCE and all, as Stylus opens
    /// it), kept in a UMF's COVR chunk. None until one's painted or chosen;
    /// a UMF saved without one gets one painted ([`Module::cover_file`]).
    pub cover: Option<Arc<Vec<u8>>>,
    /// How the song likes to be seen as it plays: a neumetik-visual
    /// settings line (`bars;palette=fire;...`), empty for the player's
    /// own choice. Kept in a UMF's VISL chunk.
    pub visual: String,
    /// Colors the song keeps (OctaMED's screen colors), as RGB: clues for
    /// its cover's palette.
    pub palette: Vec<[u8; 3]>,
}

impl Module {
    /// A new song: `channels` channels, one empty pattern of 64 rows.
    pub fn new(channels: usize) -> Module {
        let channels = channels.clamp(1, 64);
        Module {
            title: String::new(),
            format: "umf".into(),
            format_name: "Universal Mod Format".into(),
            tracker: "Neumetik".into(),
            kind: Kind::It,
            flags: Flags { linear: true, ..Default::default() },
            speed: 6,
            tempo: 125,
            global_volume: 128,
            mix_volume: 48,
            separation: 128,
            channels: (0..channels).map(|i| Channel { pan: if i % 4 == 0 || i % 4 == 3 { 64 } else { 192 }, ..Default::default() }).collect(),
            orders: vec![0],
            patterns: vec![Arc::new(Pattern::new(64, channels))],
            samples: Vec::new(),
            instruments: Vec::new(),
            ..Default::default()
        }
    }

    /// Beats a minute, near enough: the tempo and speed as the tempo mode
    /// reads them, over the song's rows a beat.
    pub fn bpm(&self) -> f32 {
        let (speed, tempo, rpb) = (f32::from(self.speed.max(1)), f32::from(self.tempo.max(1)), f32::from(self.time.rows_per_beat.max(1)));
        match self.tempo_mode {
            TempoMode::Classic => 24.0 * tempo / (speed * rpb),
            TempoMode::Alternative => 60.0 * tempo / (speed * rpb),
            TempoMode::Modern => tempo,
        }
    }


    pub fn pattern(&self, i: usize) -> Option<&Pattern> {
        self.patterns.get(i).map(|p| p.as_ref())
    }

    /// Every pattern, cut or padded to the module's channels; empty
    /// patterns for orders that point past the end; orders that point
    /// nowhere dropped at the end. Loaders call this when they're done.
    pub fn tidy(&mut self) {
        let channels = self.channels.len().max(1);
        if self.channels.is_empty() {
            self.channels.push(Channel::default());
        }
        for p in &mut self.patterns {
            if p.channels != channels {
                *p = Arc::new(p.with_channels(channels));
            }
        }
        let most = self.orders.iter().filter(|&&o| o < ORDER_SKIP).max().copied();
        if let Some(most) = most {
            while self.patterns.len() <= usize::from(most) {
                self.patterns.push(Arc::new(Pattern::new(64, channels)));
            }
        }
        if self.patterns.is_empty() {
            self.patterns.push(Arc::new(Pattern::new(64, channels)));
        }
        while self.orders.last() == Some(&ORDER_END) {
            self.orders.pop();
        }
        if self.orders.is_empty() {
            self.orders.push(0);
        }
        if self.speed == 0 {
            self.speed = 6;
        }
        if self.tempo < 32 {
            self.tempo = 125;
        }
        if self.global_volume == 0 {
            self.global_volume = 128;
        }
        if self.mix_volume == 0 {
            self.mix_volume = 48;
        }
        for s in &mut self.samples {
            let frames = s.wave.frames() as u32;
            s.looped = s.looped.clamped(frames);
            s.sustain = s.sustain.clamped(frames);
            s.volume = s.volume.min(64);
            s.global_volume = s.global_volume.min(64);
            if s.c5 == 0 {
                s.c5 = 8363;
            }
        }
        for i in &mut self.instruments {
            i.keymap.resize(120, (0, 0));
            for (n, k) in i.keymap.iter_mut().enumerate() {
                if k.0 >= 120 {
                    k.0 = n as u8;
                }
            }
        }
        let title = self.title.trim().to_string();
        self.title = title;
    }

    /// The orders that play, in order (skips and the end left out).
    pub fn played_orders(&self) -> impl Iterator<Item = (usize, u16)> + '_ {
        self.orders.iter().copied().enumerate().take_while(|&(_, o)| o != ORDER_END).filter(|&(_, o)| o != ORDER_SKIP)
    }

    /// The 1-based instruments a cell can name: instruments when there
    /// are, samples when not.
    pub fn slots(&self) -> usize {
        if self.flags.instruments {
            self.instruments.len()
        } else {
            self.samples.len()
        }
    }

    /// Rows a beat and a measure in a pattern.
    pub fn time_of(&self, pattern: usize) -> TimeSig {
        self.patterns.get(pattern).and_then(|p| p.time).unwrap_or(self.time)
    }

    /// The order lists, the one being edited as it is now.
    pub fn all_sequences(&self) -> Vec<Sequence> {
        let mut v = self.sequences.clone();
        let now = Sequence { name: v.get(self.sequence).map(|s| s.name.clone()).unwrap_or_default(), orders: self.orders.clone(), restart: self.restart };
        if v.is_empty() {
            v.push(now);
        } else if let Some(s) = v.get_mut(self.sequence) {
            *s = now;
        }
        v
    }

    /// Another order list to play and edit; the one edited is kept.
    pub fn switch_sequence(&mut self, to: usize) {
        let all = self.all_sequences();
        if let Some(s) = all.get(to) {
            self.orders = s.orders.clone();
            self.restart = s.restart;
            self.sequence = to;
        }
        self.sequences = all;
    }

    /// A slot of the pattern matrix that doesn't play.
    pub fn slot_muted(&self, order: usize, channel: usize) -> bool {
        !self.slot_mutes.is_empty() && self.slot_mutes.contains(&(order as u16, channel as u16))
    }
}

/// A note as trackers write it: C-5, C#5, === (off), ^^^ (cut), ~~~ (fade).
pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"];
    match note {
        NOTE_NONE => "...".into(),
        NOTE_OFF => "===".into(),
        NOTE_CUT => "^^^".into(),
        NOTE_FADE => "~~~".into(),
        NOTE_MIN..=NOTE_MAX => {
            let n = note - 1;
            format!("{}{}", NAMES[usize::from(n % 12)], n / 12)
        }
        _ => "???".into(),
    }
}

/// The 1-based note `name` (as `note_name` writes it) stands for.
pub fn parse_note(name: &str) -> Option<u8> {
    match name {
        "..." => return Some(NOTE_NONE),
        "===" => return Some(NOTE_OFF),
        "^^^" => return Some(NOTE_CUT),
        "~~~" => return Some(NOTE_FADE),
        _ => {}
    }
    let b = name.as_bytes();
    if b.len() != 3 {
        return None;
    }
    let base = match b[0].to_ascii_uppercase() {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    let sharp = match b[1] {
        b'#' => 1,
        b'-' => 0,
        _ => return None,
    };
    let octave = (b[2] as char).to_digit(10)? as u8;
    Some(octave * 12 + base + sharp + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_round_trip_through_their_names() {
        for n in NOTE_MIN..=NOTE_MAX {
            assert_eq!(parse_note(&note_name(n)), Some(n));
        }
        assert_eq!(note_name(NOTE_MIDDLE_C), "C-5");
        assert_eq!(parse_note("==="), Some(NOTE_OFF));
    }

    #[test]
    fn letters_round_trip() {
        for f in Fx::ALL.iter().copied().filter(|&f| f != Fx::None) {
            assert_eq!(Fx::from_letter(f.letter()), Some(f), "{f:?}");
            assert_eq!(Fx::from_u8(f as u8), f);
        }
        for v in Vol::ALL.iter().copied().filter(|&v| v != Vol::None) {
            assert_eq!(Vol::from_letter(v.letter()), Some(v));
        }
    }

    #[test]
    fn tidy_pads_patterns_and_orders() {
        let mut m = Module::new(4);
        m.orders = vec![0, 3, ORDER_END, ORDER_END];
        m.channels.push(Channel::default());
        m.tidy();
        assert_eq!(m.patterns.len(), 4);
        assert_eq!(m.patterns[0].channels, 5);
        assert_eq!(m.orders, vec![0, 3]);
    }
}
