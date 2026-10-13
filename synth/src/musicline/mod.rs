// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! **Musicline Editor's replayer**: a song's tunes, each channel its own
//! sequence of parts (128 lines of a note, an instrument and five
//! effects) moving at its own speed and groove; instruments that play a
//! sample or a single-cycle wave under an envelope, vibrato, tremolo
//! and an arpeggio table, and whose waves are remade every tick by five
//! effects in turn (a morph through up to six waves, a phase squeeze,
//! a mix with another wave, a resonance and a filter) or whose loop
//! moves through the sample.
//!
//! The channels keep their own time, so the whole replayer is here
//! ([`Machine`]): each tick it reads the rows that are due, runs the
//! effects and remakes the waves, and says what each of the Amiga's
//! channels is told ([`Channel`]): a note started, the loop it goes on
//! to, its period and its volume. The player it runs in plays that.
//!
//! Written from the format as NostalgicPlayer's MIT-licensed player
//! reads and plays it, checked against the original replayer's source
//! (Musicline's own, LGPL, kept with UADE) where they differ; both read
//! for what each command and effect does, neither copied. The
//! replayer's tables are computed (`tables.rs`). Renders were compared
//! with UADE playing the original replayer.

mod engine;
pub mod tables;

pub use engine::{channels_of, Buf, Channel, Machine, Region, RowRead};

/// What the loop and arpeggio of a part line can be: a note's effects.
pub type Effect = (u8, u8);

/// A tune: the song's channels, each with its own sequence.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Tune {
    pub title: String,
    /// Beats a minute (the CIA's: a tick is 2.5 / tempo seconds).
    pub tempo: u16,
    pub speed: u8,
    /// The speed of every other row, when not 0.
    pub groove: u8,
    pub volume: u16,
    /// Eight channels mixed in pairs (the replayer's mixer mode).
    pub mixer: bool,
    /// Each channel's sequence, 256 words: a part (its number in the
    /// high byte and bits 6 and 7, a transpose in the low five bits plus
    /// 16) or, with bit 5, a command (end, jump, wait); past its end,
    /// part 0.
    pub sequences: Vec<Vec<u16>>,
}

/// A line of a part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Line {
    /// 1 to 60 a note, 61 the part's end; with bit 7, a jump back to the
    /// line in the low bits, `instr` times.
    pub note: u8,
    pub instr: u8,
    pub fx: [Effect; 5],
}

/// A part: 128 lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub lines: Vec<Line>,
}

impl Default for Part {
    fn default() -> Part {
        Part { lines: vec![Line::default(); 128] }
    }
}

/// A line of an arpeggio table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ArpLine {
    /// A note (1 to 60, absolute), below 0 semitones under the played
    /// note plus 61; 0 waits, 61 ends, 62 jumps to the line `sample` says.
    pub transpose: i8,
    pub sample: u8,
    pub fx: [Effect; 2],
}

/// An arpeggio table: 128 lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arpeggio {
    pub lines: Vec<ArpLine>,
}

impl Default for Arpeggio {
    fn default() -> Arpeggio {
        Arpeggio { lines: vec![ArpLine::default(); 128] }
    }
}

/// What a sample is: a sample, or a wave of 16 to 256 points (kept as
/// 256 points and their halvings, five octaves).
pub const SAMPLE: u8 = 0;

/// A sample.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Wave {
    pub name: String,
    pub kind: u8,
    /// In words.
    pub length: u16,
    /// Where its loop starts, in bytes, and how long it is, in words (0: none).
    pub repeat: u32,
    pub repeat_length: u16,
    /// 32nds of a semitone, and semitones.
    pub finetune: i16,
    pub semitone: i16,
    /// The points; a 256-point wave has its four halvings after it.
    pub data: Vec<i8>,
    /// How many points the file stored (256 for a wave, whose halvings
    /// are made from them).
    pub stored: u32,
}

impl Wave {
    /// A wave's octaves made from its first 256 points (the replayer
    /// keeps -128 off them), as it does when it loads one.
    pub fn octaves(points: &[i8]) -> Vec<i8> {
        let mut d: Vec<i8> = points.iter().take(256).map(|&x| x.max(-127)).collect();
        d.resize(256, 0);
        for i in 0..240 {
            d.push(d[i * 2]);
        }
        d
    }

    /// Its points as they were stored (a wave's first 256).
    pub fn points(&self) -> &[i8] {
        &self.data[..(self.stored as usize).min(self.data.len())]
    }

    /// Its points put back from what was stored (`points`, or less of
    /// it: what's missing is silence), the halvings made again.
    pub fn attach(&mut self, points: &[i8]) {
        let mut d = points.to_vec();
        d.resize(self.stored as usize, 0);
        self.data = if self.stored == 256 { Wave::octaves(&d) } else { d };
    }
}

/// The size of an instrument as stored.
pub const INSTRUMENT_BYTES: usize = 206;

/// An instrument, as stored (206 bytes), its fields read from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instrument {
    pub raw: Vec<u8>,
    /// Its sample was loaded after it (the replayer only gives an
    /// instrument a sample that comes later in the file).
    pub bound: bool,
}

impl Default for Instrument {
    fn default() -> Instrument {
        Instrument { raw: vec![0; INSTRUMENT_BYTES], bound: false }
    }
}

impl Instrument {
    pub fn new(raw: &[u8], bound: bool) -> Instrument {
        let mut raw = raw.to_vec();
        raw.resize(INSTRUMENT_BYTES, 0);
        Instrument { raw, bound }
    }
    fn b(&self, at: usize) -> u8 {
        self.raw[at]
    }
    fn w(&self, at: usize) -> u16 {
        u16::from_be_bytes([self.raw[at], self.raw[at + 1]])
    }
    pub fn name(&self) -> String {
        let end = self.raw[..32].iter().position(|&c| c == 0).unwrap_or(32);
        self.raw[..end].iter().map(|&c| c as char).collect::<String>().trim_end().to_string()
    }
    pub fn sample(&self) -> u8 {
        self.b(32)
    }
    pub fn kind(&self) -> u8 {
        self.b(33)
    }
    fn length(&self) -> u16 {
        self.w(38)
    }
    fn repeat_length(&self) -> u16 {
        self.w(44)
    }
    fn finetune(&self) -> i16 {
        self.w(46) as i16
    }
    fn semitone(&self) -> i16 {
        self.w(48) as i16
    }
    fn start(&self) -> u16 {
        self.w(50)
    }
    fn repeat_start(&self) -> u16 {
        self.w(54)
    }
    pub fn volume(&self) -> u16 {
        self.w(58)
    }
    fn transposable(&self) -> bool {
        self.b(60) != 0
    }
    fn slide_speed(&self) -> u8 {
        self.b(61)
    }
    fn effects1(&self) -> u8 {
        self.b(62)
    }
    fn effects2(&self) -> u8 {
        self.b(63)
    }
    /// The envelope's twelve words: four lengths, four speeds, four volumes.
    fn envelope(&self) -> [u16; 12] {
        std::array::from_fn(|i| self.w(64 + i * 2))
    }
    /// Vibrato (`at` 88) or tremolo (100): direction up, wave, speed,
    /// delay, attack speed, attack, depth.
    fn wobble(&self, at: usize) -> (bool, u8, u16, u16, u16, u16, u16) {
        (self.b(at) != 0, self.b(at + 1), self.w(at + 2), self.w(at + 4), self.w(at + 6), self.w(at + 8), self.w(at + 10))
    }
    fn arp_table(&self) -> u16 {
        self.w(112)
    }
    fn arp_speed(&self) -> u8 {
        self.b(114)
    }
    fn arp_groove(&self) -> u8 {
        self.b(115)
    }
    fn flags1(&self) -> u8 {
        self.b(116)
    }
    fn transform_waves(&self) -> [u8; 5] {
        std::array::from_fn(|i| self.b(117 + i))
    }
    /// One of the wave effects' counters: start, repeat, repeat end,
    /// speed, turns, delay.
    fn counter(&self, at: usize) -> [u16; 6] {
        std::array::from_fn(|i| self.w(at + i * 2))
    }
    fn transform(&self) -> [u16; 6] {
        self.counter(122)
    }
    fn phase(&self) -> [u16; 6] {
        self.counter(134)
    }
    fn phase_type(&self) -> u16 {
        self.w(146)
    }
    fn flags2(&self) -> u8 {
        self.b(148)
    }
    fn mix_wave(&self) -> u8 {
        self.b(149)
    }
    fn mix(&self) -> [u16; 6] {
        self.counter(150)
    }
    fn resonance(&self) -> [u16; 6] {
        self.counter(162)
    }
    fn boost(&self) -> u8 {
        self.b(174)
    }
    fn resonance_amp(&self) -> u8 {
        self.b(175)
    }
    fn filter(&self) -> [u16; 6] {
        self.counter(176)
    }
    fn filter_type(&self) -> u8 {
        self.b(189)
    }
    /// The moving loop: start, repeat, repeat end, length, step, wait,
    /// delay, turns.
    fn moving_loop(&self) -> [u16; 8] {
        std::array::from_fn(|i| self.w(190 + i * 2))
    }
}

/// A Musicline song.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Song {
    pub tunes: Vec<Tune>,
    /// By number, 1,024 of them.
    pub parts: Vec<Part>,
    /// By number, 256.
    pub arpeggios: Vec<Arpeggio>,
    /// From 1; 0 is none.
    pub instruments: Vec<Instrument>,
    /// From 1; 0 is none.
    pub samples: Vec<Wave>,
}

impl Song {
    pub fn new() -> Song {
        Song { tunes: Vec::new(), parts: vec![Part::default(); 1024], arpeggios: vec![Arpeggio::default(); 256], instruments: vec![Instrument::default()], samples: vec![Wave::default()] }
    }
}
