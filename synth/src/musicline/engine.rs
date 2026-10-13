// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! The replayer: [`Machine`] plays a tune a tick at a time.
//!
//! Its arithmetic is the replayer's 16-bit arithmetic, wrapping where
//! the replayer's wraps: volumes in 16ths (0 to 1,024), notes in 32nds
//! of a semitone (0 the lowest, 1,920 five octaves up), periods the
//! Amiga's.

use super::{tables, Effect, Instrument, Song, SAMPLE};

// Part effects.
const SLIDE_UP: u8 = 0x01;
const SLIDE_DOWN: u8 = 0x02;
const PORTAMENTO: u8 = 0x03;
const INIT_INST_PORTA: u8 = 0x04;
const PITCH_UP: u8 = 0x05;
const PITCH_DOWN: u8 = 0x06;
const VIBRATO_SPEED: u8 = 0x07;
const VIBRATO_UP: u8 = 0x08;
const VIBRATO_DOWN: u8 = 0x09;
const VIBRATO_WAVE: u8 = 0x0A;
const SET_FINETUNE: u8 = 0x0B;
const INST_VOLUME: u8 = 0x10;
const INST_VOL_UP: u8 = 0x11;
const INST_VOL_DOWN: u8 = 0x12;
const INST_VOL_SLIDE_TO_SET: u8 = 0x13;
const INST_VOL_SLIDE_TO: u8 = 0x14;
const INST_VOL_ADD: u8 = 0x15;
const INST_VOL_SUB: u8 = 0x16;
const TREMOLO_SPEED: u8 = 0x17;
const TREMOLO_UP: u8 = 0x18;
const TREMOLO_DOWN: u8 = 0x19;
const TREMOLO_WAVE: u8 = 0x1A;
const CHAN_VOLUME: u8 = 0x20;
const CHAN_VOL_UP: u8 = 0x21;
const CHAN_VOL_DOWN: u8 = 0x22;
const CHAN_VOL_SLIDE_TO_SET: u8 = 0x23;
const CHAN_VOL_SLIDE_TO: u8 = 0x24;
const CHAN_VOL_ADD: u8 = 0x25;
const CHAN_VOL_SUB: u8 = 0x26;
const ALL_CHAN_VOLUME: u8 = 0x27;
const MASTER_VOLUME: u8 = 0x30;
const MASTER_VOL_UP: u8 = 0x31;
const MASTER_VOL_DOWN: u8 = 0x32;
const MASTER_VOL_SLIDE_TO_SET: u8 = 0x33;
const MASTER_VOL_SLIDE_TO: u8 = 0x34;
const MASTER_VOL_ADD: u8 = 0x35;
const MASTER_VOL_SUB: u8 = 0x36;
const SPEED_ONE: u8 = 0x40;
const GROOVE_ONE: u8 = 0x41;
const SPEED_ALL: u8 = 0x42;
const GROOVE_ALL: u8 = 0x43;
const ARP_TABLE: u8 = 0x44;
const ARP_TABLE_ONCE: u8 = 0x45;
const HOLD_SUSTAIN_FX: u8 = 0x46;
const FILTER_FX: u8 = 0x47;
const SAMPLE_OFFSET: u8 = 0x48;
const RESTART_NO_VOLUME: u8 = 0x49;
const WAVE_SAMPLE: u8 = 0x4A;
const INIT_INSTRUMENT: u8 = 0x4B;
const PT_SLIDE_UP: u8 = 0xE1;
const PT_SLIDE_DOWN: u8 = 0xE2;
const PT_PORTAMENTO: u8 = 0xE3;
const PT_FINE_UP: u8 = 0xE4;
const PT_FINE_DOWN: u8 = 0xE5;
const PT_VOL_UP: u8 = 0xE6;
const PT_VOL_DOWN: u8 = 0xE7;
const PT_TREMOLO: u8 = 0xE8;
const PT_TREMOLO_WAVE: u8 = 0xE9;
const PT_VIBRATO: u8 = 0xEA;
const PT_VIBRATO_WAVE: u8 = 0xEB;

// Arpeggio table effects.
const ARP_SLIDE_UP: u8 = 1;
const ARP_SLIDE_DOWN: u8 = 2;
const ARP_SET_VOLUME: u8 = 3;
const ARP_VOL_UP: u8 = 4;
const ARP_VOL_DOWN: u8 = 5;
const ARP_RESTART: u8 = 6;

// The instrument's first effects.
const ENVELOPE: u8 = 1;
const VIBRATO: u8 = 2;
const TREMOLO: u8 = 4;
const ARPEGGIO: u8 = 8;
const LOOP: u8 = 16;
const LOOP_STOP: u8 = 32;
const HOLD_SUSTAIN: u8 = 64;
const WAVE_SAMPLE_LOOP: u8 = 128;
// Its wave effects.
const TRANSFORM: u8 = 1;
const PHASE: u8 = 2;
const MIX: u8 = 4;
const RESONANCE: u8 = 8;
const FILTER: u8 = 16;
// The first set's flags.
const ENV_HOLD: u8 = 1;
const TRANSFORM_INIT: u8 = 2;
const TRANSFORM_STEP: u8 = 4;
const PHASE_INIT: u8 = 8;
const PHASE_STEP: u8 = 16;
const PHASE_FILL: u8 = 32;
const FILTER_INIT: u8 = 64;
const FILTER_STEP: u8 = 128;
// The second set's.
const MIX_INIT: u8 = 1;
const MIX_STEP: u8 = 2;
const MIX_BUFF: u8 = 4;
const MIX_COUNTER: u8 = 8;
const RES_INIT: u8 = 16;
const RES_STEP: u8 = 32;
const LOOP_INIT: u8 = 64;
const LOOP_STEP: u8 = 128;
// Boosts.
const BOOST_FILTER: u8 = 1;
const BOOST_RES: u8 = 2;
const BOOST_MIX: u8 = 4;

// A voice's arpeggio.
const ARP_ON: u8 = 1;
const ARP_SETS_NOTE: u8 = 2;
const ARP_USE_TABLE: u8 = 4;
const ARP_USE_SAMPLE: u8 = 8;
const ARP_ONE_STEP: u8 = 16;
const ARP_ABSOLUTE: u8 = 32;
// What it's doing this tick.
const RETRIGGER: u8 = 1;
const FIRST_TICK: u8 = 2;
const KEEP_FINETUNE: u8 = 4;
// Why it restarts.
const FROM_PART: u8 = 1;
const FROM_ARP: u8 = 2;
const ARP_SET_VOL: u8 = 4;

/// Where a voice reads points from: one of the song's samples, one of
/// its own buffers (each wave effect's, and the wave it plays), or
/// silence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Buf {
    Sample(u8),
    Transform,
    Phase,
    Mix,
    Resonance,
    Filter,
    Wave,
    #[default]
    Zero,
}

impl Buf {
    /// One of a voice's own buffers, whose points change as it plays.
    pub fn live(self) -> bool {
        self.own().is_some()
    }

    fn own(self) -> Option<usize> {
        match self {
            Buf::Transform => Some(0),
            Buf::Phase => Some(1),
            Buf::Mix => Some(2),
            Buf::Resonance => Some(3),
            Buf::Filter => Some(4),
            Buf::Wave => Some(5),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct Ptr {
    buf: Buf,
    off: u32,
}

impl Ptr {
    fn at(buf: Buf, off: u32) -> Ptr {
        Ptr { buf, off }
    }
    fn plus(self, n: u32) -> Ptr {
        Ptr { buf: self.buf, off: self.off + n }
    }
}

/// Points to play: where, from what offset, how many (bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Region {
    pub buf: Buf,
    pub offset: u32,
    pub len: u32,
}

/// What a channel is told this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Channel {
    /// A note starts: `play` from its start, then `repeat` (or nothing).
    pub restart: bool,
    pub play: Region,
    /// The loop it goes on to, newly set (with or without a new note).
    pub repeat: Option<Region>,
    /// The loop ends in silence (the moving loop's stop).
    pub silence: bool,
    pub period: u16,
    /// 0 to 64.
    pub volume: u16,
    /// The sample playing, for the screen.
    pub sample: u8,
}

/// A row a channel read this tick, for the patterns that show the song.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct RowRead {
    /// The sequence's place.
    pub position: u8,
    pub part: u16,
    pub line: u8,
    /// The part's transpose in semitones.
    pub transpose: i8,
    /// A wait in the sequence: no line.
    pub wait: bool,
}

/// One of the wave effects' counters.
#[derive(Clone, Copy, Debug, Default)]
struct Counter {
    counter: u16,
    speed: i16,
    repeat: u16,
    repeat_end: u16,
    turns: i16,
    delay: u16,
    step: bool,
    save: u16,
}

impl Counter {
    /// Back and forth between the repeat and its end, `turns` times
    /// (0 for ever), after `delay` ticks.
    fn bounce(&mut self) {
        if !(self.step || self.delay == 0) {
            self.delay -= 1;
            return;
        }
        if self.turns < 0 {
            return;
        }
        let mut c = self.counter as i16;
        let speed = self.speed;
        c = c.wrapping_add(speed);
        let (lo, hi) = if self.repeat < self.repeat_end { (self.repeat, self.repeat_end) } else { (self.repeat_end, self.repeat) };
        let inside = if speed < 0 { i32::from(c) >= i32::from(lo) } else { i32::from(c) <= i32::from(hi) };
        if inside {
            self.counter = c as u16;
            return;
        }
        if self.turns != 0 {
            self.turns -= 1;
            if self.turns == 0 {
                self.turns = -1;
            }
        }
        self.counter = c.wrapping_sub(speed) as u16;
        self.speed = speed.wrapping_neg();
    }

    /// Round and round, 512 steps.
    fn around(&mut self) {
        if self.step || self.delay == 0 {
            self.counter = self.counter.wrapping_add(self.speed as u16) & 0x1FF;
        } else {
            self.delay -= 1;
        }
    }

    /// Starts or carries on, as the instrument's init and step flags say;
    /// true when it starts over.
    #[allow(clippy::too_many_arguments)]
    fn start(&mut self, speed_store: &mut u16, init_store: &mut u8, step: bool, keep: bool, instr: u8, p: [u16; 6]) -> bool {
        let [start, repeat, end, speed, turns, delay] = p;
        let (mut init, mut check) = (false, false);
        self.step = step;
        if step {
            *speed_store = turns;
            self.turns = 0;
            check = true;
        } else {
            self.turns = turns as i16;
            if !keep {
                *init_store = 0;
                init = true;
            } else {
                check = true;
            }
        }
        if check {
            let old = *init_store;
            *init_store = instr;
            if old == instr {
                self.delay = delay;
            } else {
                init = true;
            }
        }
        if init {
            self.counter = start;
            self.speed = speed as i16;
            self.repeat = repeat;
            self.repeat_end = end;
            if start > repeat {
                self.speed = self.speed.wrapping_neg();
            }
            self.delay = delay;
        }
        init
    }

    /// The tick's value: counted on (or, stepping, only on a new note),
    /// then nudged by the step speed.
    fn value(&mut self, step: bool, keep: bool, new_note: bool, around: bool, step_speed: u16, clamp: (i16, u16)) -> u16 {
        let count = if step {
            if keep {
                if new_note {
                    self.save = self.counter;
                }
                true
            } else if new_note {
                self.save = self.counter;
                true
            } else {
                false
            }
        } else {
            self.save = self.counter;
            true
        };
        if count {
            if around {
                self.around();
            } else {
                self.bounce();
            }
        }
        let value = self.save;
        if step {
            if self.delay != 0 {
                self.delay -= 1;
            } else {
                let speed = i16::from(step_speed as u8 as i8);
                let s = self.save.wrapping_add(speed as u16);
                self.save = if speed < 0 {
                    if (s as i16) < clamp.0 {
                        clamp.0 as u16
                    } else {
                        s
                    }
                } else if s > clamp.1 {
                    clamp.1
                } else {
                    s
                };
            }
        }
        value
    }
}

/// The source a note plays: an instrument's sample, or a sample on its own.
#[derive(Clone, Copy, Debug, Default)]
struct Src {
    ptr: Option<Ptr>,
    len: u16,
    rep: Option<Ptr>,
    rep_len: u16,
    finetune: i16,
    semitone: i16,
    kind: u8,
}

#[derive(Clone)]
struct Bufs([[i8; 256]; 6]);

impl Default for Bufs {
    fn default() -> Bufs {
        Bufs([[0; 256]; 6])
    }
}

/// A channel's state.
#[derive(Clone, Default)]
struct Voice {
    off: bool,
    speed: u8,
    groove: u8,
    speed_part: u8,
    groove_part: u8,
    speed_counter: u8,
    arp_speed_counter: u8,
    part_groove: bool,
    arp_groove: bool,
    wave_number: u8,
    wave_number_old: u8,
    instrument: Option<usize>,
    wave_sample: Option<u8>,
    part_note: u8,
    part_instr: u8,
    part_fx: [Effect; 5],
    arp: u8,
    arp_pos: u8,
    arp_table: u8,
    arp_wait: bool,
    arp_note: u8,
    arp_vol_slide: u8,
    arp_pitch_slide: u8,
    arp_pitch_down: bool,
    arp_calc_note: u16,
    tune_pos: u8,
    part_pos: u8,
    tune_jump_counter: u8,
    part_jump_counter: u8,
    rep_orig: Option<Ptr>,
    play_ptr: Option<Ptr>,
    play_len: u16,
    rep_ptr: Option<Ptr>,
    rep_len: u16,
    vol1: u16,
    vol2: u16,
    vol3: u16,
    note: u16,
    period: u16,
    transpose: i16,
    semitone: i16,
    finetune: i16,
    offset_on: bool,
    offset: u8,
    old_instr: u8,
    restart: u8,
    volume_add: u8,
    volume_slide: u8,
    chan_vol_slide: u8,
    master_vol_slide: u8,
    volume_set: u16,
    chan_volume: u16,
    volume_add_n: i16,
    chan_vol_add_n: i16,
    master_vol_add_n: i16,
    vol_slide_speed: u16,
    chan_vol_slide_speed: u16,
    master_vol_slide_speed: u16,
    vol_slide_to: u16,
    chan_vol_slide_to: u16,
    master_vol_slide_to: u16,
    vol_slide_down: bool,
    chan_vol_slide_down: bool,
    master_vol_slide_down: bool,
    vol_slide_done: bool,
    chan_vol_slide_done: bool,
    master_vol_slide_done: bool,
    volume: u8,
    inst_pitch_slide: u8,
    boost: u8,
    transpose_number: u8,
    part_number: u16,
    pitch_slide: u8,
    pitch_slide_down: bool,
    pitch_slide_speed: u16,
    pitch_slide_note: u16,
    pitch_slide_to: i16,
    pitch_add: i16,
    arp_vol_slide_speed: u16,
    arp_pitch_slide_speed: u16,
    arp_pitch_slide_to: i16,
    arp_pitch_slide_note: u16,
    pt_pitch_slide: u8,
    pt_pitch_slide_down: bool,
    pt_pitch_slide_speed: u16,
    pt_pitch_slide_speed2: u16,
    pt_pitch_slide_note: u16,
    pt_pitch_slide_to: i16,
    pt_pitch_add: i16,
    fx1: u8,
    fx1_arg: u8,
    fx2: u8,
    fx2_arg: u8,
    env_volume: u16,
    env: [u16; 12],
    play: u8,
    wave_or_sample: u8,
    phase_init: u8,
    filter_init: u8,
    transform_init: u8,
    tune_wait: u8,
    vibrato: u8,
    vib_down: bool,
    vib_wave: u8,
    part_vib_wave: u8,
    vib_counter: u16,
    vib_speed: u16,
    vib_depth_now: u16,
    vib_delay: u16,
    vib_attack_speed: u16,
    vib_attack_len: u16,
    vib_depth: u16,
    vib_note: u16,
    pt_trem_pos: i8,
    pt_trem_cmd: u8,
    pt_trem_wave: u8,
    pt_vib_pos: i8,
    pt_vib_cmd: u8,
    pt_vib_wave: u8,
    pt_vib_note: u16,
    tremolo: u8,
    trem_down: bool,
    trem_wave: u8,
    part_trem_wave: u8,
    trem_counter: u16,
    trem_speed: u16,
    trem_depth_now: u16,
    trem_delay: u16,
    trem_attack_speed: u16,
    trem_attack_len: u16,
    trem_depth: u16,
    filter_last: i8,
    res_last: i8,
    filter_last_init: bool,
    res_last_init: bool,
    res_amp: u8,
    res_init: u8,
    phase_type: u16,
    filter_type: u8,
    transform_c: Counter,
    transform_speed: u16,
    phase_c: Counter,
    phase_speed: u16,
    mix_c: Counter,
    mix_speed: u16,
    res_c: Counter,
    res_speed: u16,
    filter_c: Counter,
    filter_speed: u16,
    mix_wave: u8,
    mix_init: u8,
    transform_waves: [u8; 6],
    loop_init: u8,
    loop_repeat: u16,
    loop_repeat_end: u16,
    loop_length: u16,
    loop_step: i32,
    loop_wait: u16,
    loop_wait_counter: i16,
    loop_delay: i16,
    loop_turns: i16,
    loop_counter: u16,
    loop_counter_save: u16,
    loop_max: u16,
    loop_speed: i16,
    loop_sample: Option<Ptr>,
    bufs: Bufs,
    out: Channel,
    row: Option<RowRead>,
    ended: bool,
    looped: bool,
    /// Where its sequence's last run of part 0 starts: past its words a
    /// sequence goes round part 0 for ever, which doesn't hold the song's
    /// end back. Reaching it makes the channel idle.
    tail: usize,
    idle: bool,
}

/// The replayer, playing one of a song's tunes.
#[derive(Clone)]
pub struct Machine {
    voices: Vec<Voice>,
    tune: usize,
    tempo: u16,
    tune_speed: u8,
    tune_groove: u8,
    master: u16,
    playing: bool,
    filter: bool,
    ticks: u64,
    mixer: bool,
    /// A mixing tune's mix buffer, in points at period 126: its tick.
    mix_length: u32,
}

fn instrument(song: &Song, i: usize) -> &Instrument {
    song.instruments.get(i).unwrap_or(&song.instruments[0])
}

fn inst_src(song: &Song, i: usize) -> Src {
    let inst = instrument(song, i);
    let n = inst.sample();
    let bound = inst.bound && song.samples.get(usize::from(n)).is_some_and(|s| !s.data.is_empty());
    Src {
        ptr: bound.then(|| Ptr::at(Buf::Sample(n), u32::from(inst.start()) * 2)),
        len: inst.length(),
        rep: bound.then(|| Ptr::at(Buf::Sample(n), u32::from(inst.repeat_start()) * 2)),
        rep_len: inst.repeat_length(),
        finetune: inst.finetune(),
        semitone: inst.semitone(),
        kind: inst.kind(),
    }
}

fn sample_src(song: &Song, n: u8) -> Src {
    match song.samples.get(usize::from(n)).filter(|s| !s.data.is_empty() && n != 0) {
        Some(s) => Src {
            ptr: Some(Ptr::at(Buf::Sample(n), 0)),
            len: s.length,
            rep: Some(Ptr::at(Buf::Sample(n), s.repeat)),
            rep_len: s.repeat_length,
            finetune: s.finetune,
            semitone: s.semitone,
            kind: s.kind,
        },
        None => Src::default(),
    }
}

/// The mixer's buffer for a tempo: the points at period 126 a tick
/// takes, made even.
fn mix_length(tempo: u16) -> u32 {
    (3_546_895 / 126 * 125 / (u32::from(tempo.max(1)) * 50)) & !1
}

/// How many channels a song plays at most: eight in a tune that mixes
/// them, four in any other.
pub fn channels_of(song: &Song) -> usize {
    song.tunes.iter().map(|t| t.sequences.len().min(if t.mixer { 8 } else { 4 })).max().unwrap_or(4)
}

impl Machine {
    /// Ready to play `tune` from its start.
    pub fn new(song: &Song, tune: usize) -> Machine {
        let tune = tune.min(song.tunes.len().saturating_sub(1));
        let t = song.tunes.get(tune);
        // Eight channels only in a tune that mixes them; four in any other.
        let mixer = t.is_some_and(|t| t.mixer);
        let n = t.map_or(0, |t| t.sequences.len()).min(if mixer { 8 } else { 4 });
        let (speed, groove) = t.map_or((6, 0), |t| (t.speed, t.groove));
        let tail = |c: usize| t.map_or(0, |t| t.sequences[c].iter().rposition(|&w| w != 0x0010).map_or(0, |i| i + 1));
        let voices = (0..n).map(|c| Voice { tail: tail(c), idle: tail(c) == 0, speed, groove, speed_counter: 1, part_groove: groove != 0, chan_volume: 1024, pitch_slide_to: -1, ..Default::default() }).collect();
        Machine { voices, tune, tempo: t.map_or(125, |t| t.tempo), tune_speed: speed, tune_groove: groove, master: 1024, playing: !song.tunes.is_empty(), filter: false, ticks: 0, mixer, mix_length: mix_length(t.map_or(125, |t| t.tempo)) }
    }

    pub fn channels(&self) -> usize {
        self.voices.len()
    }

    /// The tune mixes its eight channels in pairs, each at half its
    /// level (the replayer's mixer: two to each of the Amiga's four).
    pub fn mixer(&self) -> bool {
        self.mixer
    }

    /// A tick's length in seconds, as the replayer times it on a PAL
    /// Amiga: the CIA timer at 1,773,448 / tempo (of 709,379 a second);
    /// a mixing tune by its mix buffer instead, which its tempo set at
    /// the start (a tempo command later doesn't change it).
    pub fn tick_seconds(&self) -> f64 {
        if self.mixer {
            f64::from(self.mix_length) * 126.0 / 3_546_895.0
        } else {
            // The CIA counts down through 0: a period is its value and one more.
            f64::from(1_773_448 / u32::from(self.tempo.max(1)) + 1) / 709_379.0
        }
    }

    /// Beats a minute now.
    pub fn tempo(&self) -> u16 {
        self.tempo
    }

    /// The Amiga's LED filter.
    pub fn filter(&self) -> bool {
        self.filter
    }

    /// Still playing (a part that ends on its first line stops it all).
    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// What each channel was told this tick.
    pub fn out(&self, ch: usize) -> Channel {
        self.voices.get(ch).map_or(Channel::default(), |v| v.out)
    }

    /// The row the channel read this tick, if it read one.
    pub fn row(&self, ch: usize) -> Option<RowRead> {
        self.voices.get(ch).and_then(|v| v.row)
    }

    /// The channel has ended (its sequence's end) or come round (a jump
    /// back that repeats for ever) since the start.
    pub fn reached_end(&self, ch: usize) -> bool {
        self.voices.get(ch).is_none_or(|v| v.ended || v.idle)
    }

    /// It came round, rather than stopping.
    pub fn looped(&self, ch: usize) -> bool {
        self.voices.get(ch).is_some_and(|v| v.looped)
    }

    /// Its sequence has ended: it reads nothing more (or it has none,
    /// and goes round part 0 for ever).
    pub fn stopped(&self, ch: usize) -> bool {
        self.voices.get(ch).is_none_or(|v| v.off || v.idle)
    }

    /// The tune's place in each channel's sequence.
    pub fn position(&self, ch: usize) -> u8 {
        self.voices.get(ch).map_or(0, |v| v.tune_pos)
    }

    /// A region's points as they are now (a voice's own buffers change
    /// every tick).
    pub fn bytes<'a>(&'a self, song: &'a Song, ch: usize, r: Region) -> &'a [i8] {
        let all: &[i8] = match (r.buf, r.buf.own()) {
            (_, Some(k)) => self.voices.get(ch).map_or(&[][..], |v| &v.bufs.0[k][..]),
            (Buf::Sample(n), _) => song.samples.get(usize::from(n)).map_or(&[][..], |s| &s.data[..]),
            _ => &[0, 0],
        };
        let a = (r.offset as usize).min(all.len());
        let b = (a + r.len as usize).min(all.len());
        &all[a..b]
    }

    /// A tick: the rows due, the effects, the periods and what each channel is told.
    pub fn tick(&mut self, song: &Song) {
        self.ticks += 1;
        for v in &mut self.voices {
            v.row = None;
            v.out.restart = false;
            v.out.repeat = None;
            v.out.silence = false;
        }
        if !self.playing || song.tunes.is_empty() {
            return;
        }
        for ch in 0..self.voices.len() {
            self.play_voice(song, ch);
        }
        for ch in 0..self.voices.len() {
            self.effects(song, ch);
        }
        for ch in 0..self.voices.len() {
            self.period(ch);
        }
        for ch in 0..self.voices.len() {
            self.dma(song, ch);
        }
    }

    // ---- The sequence and the parts ----

    fn end(&mut self, ch: usize) {
        self.voices[ch].ended = true;
    }

    fn stop_play(&mut self) {
        for v in &mut self.voices {
            v.wave_number_old = 0;
        }
        self.playing = false;
    }

    fn play_voice(&mut self, song: &Song, ch: usize) {
        let v = &mut self.voices[ch];
        if v.off {
            return;
        }
        v.speed_counter = v.speed_counter.wrapping_sub(1);
        if v.speed_counter > 0 {
            return;
        }
        let mut spd = v.speed;
        v.part_groove = !v.part_groove;
        if v.part_groove && v.groove != 0 {
            spd = v.groove;
        }
        v.speed_counter = spd;
        v.speed_part = 0;
        v.groove_part = 0;
        for _ in 0..255 {
            let Some(seq) = song.tunes[self.tune].sequences.get(ch) else { return };
            let data = seq.get(usize::from(self.voices[ch].tune_pos)).copied().unwrap_or(0x0010);
            let done = if data & 0x20 != 0 { self.control(song, ch, data) } else { self.part(song, ch, data) };
            if done {
                break;
            }
        }
    }

    fn control(&mut self, song: &Song, ch: usize, data: u16) -> bool {
        let arg = (data & 0x1F) as u8;
        let v = &mut self.voices[ch];
        match (data & 0xC0) >> 6 {
            1 => {
                v.off = true;
                self.end(ch);
                true
            }
            2 => {
                let target = (data >> 8) as u8;
                if v.tune_jump_counter != 0 {
                    v.tune_jump_counter -= 1;
                    v.tune_pos = if v.tune_jump_counter != 0 { target } else { v.tune_pos.wrapping_add(1) };
                    return false;
                }
                if v.tune_pos > target {
                    v.tune_jump_counter = arg;
                    v.tune_pos = target;
                    if arg == 0 {
                        v.looped = true;
                        self.end(ch);
                    }
                } else {
                    v.tune_pos = v.tune_pos.wrapping_add(1);
                }
                false
            }
            3 => {
                if v.tune_wait != 0 {
                    v.tune_wait -= 1;
                    if v.tune_wait != 0 {
                        v.row = Some(RowRead { position: v.tune_pos, wait: true, ..Default::default() });
                        return true;
                    }
                    v.tune_pos = v.tune_pos.wrapping_add(1);
                    return false;
                }
                let wait = (data >> 8) as u8;
                v.tune_wait = wait;
                if wait == 0 {
                    v.tune_pos = v.tune_pos.wrapping_add(1);
                    return false;
                }
                v.pt_pitch_slide = 0;
                v.pitch_slide = 0;
                v.volume_slide = 0;
                v.part_note = 0;
                v.part_instr = 0;
                if arg != 0 {
                    v.speed = arg;
                    v.speed_counter = arg;
                }
                v.row = Some(RowRead { position: v.tune_pos, wait: true, ..Default::default() });
                true
            }
            _ => self.part(song, ch, data),
        }
    }

    fn part(&mut self, song: &Song, ch: usize, data: u16) -> bool {
        let (tune_speed, tune_groove) = (self.tune_speed, self.tune_groove);
        let v = &mut self.voices[ch];
        v.part_number = ((data << 2) & 0x300) | (data >> 8);
        v.transpose_number = ((data & 0x1F) as u8).wrapping_sub(16);
        let part = &song.parts[usize::from(v.part_number) % song.parts.len().max(1)];
        // A part of nothing but jumps never gives a line: the guard ends it.
        for _ in 0..512 {
            let pos = v.part_pos;
            let position = v.tune_pos;
            v.part_pos = (v.part_pos + 1) & 0x7F;
            if v.part_pos == 0 {
                v.tune_pos = v.tune_pos.wrapping_add(1);
                v.speed = tune_speed;
                v.groove = tune_groove;
            }
            let line = part.lines[usize::from(pos)];
            v.part_note = line.note;
            v.part_instr = line.instr;
            v.part_fx = line.fx;
            if v.part_note == 61 {
                if pos == 0 {
                    self.stop_play();
                    self.end(ch);
                    return true;
                }
                v.part_pos = 0;
                v.speed = tune_speed;
                v.groove = tune_groove;
                v.speed_counter = v.speed;
                v.tune_pos = v.tune_pos.wrapping_add(1);
                return false;
            }
            let jump = v.part_note & 0x7F;
            if v.part_note & 0x80 != 0 {
                if v.part_jump_counter != 0 {
                    v.part_jump_counter -= 1;
                    if v.part_jump_counter != 0 {
                        v.part_pos = jump;
                    }
                } else if pos > jump {
                    v.part_jump_counter = v.part_instr;
                    v.part_pos = jump;
                    if v.part_jump_counter == 0 {
                        v.ended = true;
                    }
                }
                continue;
            }
            if jump != 0 {
                v.transpose = i16::from(v.transpose_number as i8) << 5;
            }
            v.row = Some(RowRead { position, part: v.part_number, line: pos, transpose: v.transpose_number as i8, wait: false });
            if usize::from(position) >= v.tail {
                v.idle = true;
            }
            self.check_instrument(song, ch);
            return true;
        }
        true
    }

    fn check_instrument(&mut self, song: &Song, ch: usize) {
        let v = &mut self.voices[ch];
        // An instrument the song doesn't have leaves the one playing.
        if v.part_instr != 0 && usize::from(v.part_instr) < song.instruments.len() {
            v.instrument = Some(usize::from(v.part_instr));
            if v.part_instr != v.old_instr {
                v.arp = 0;
                v.inst_pitch_slide = 0;
                v.old_instr = v.part_instr;
            }
        }
        self.part_fx(ch);
        self.start_arpeggio(song, ch);
        self.play_instrument(song, ch);
    }

    // ---- A line's effects ----

    fn part_fx(&mut self, ch: usize) {
        {
            let v = &mut self.voices[ch];
            v.play |= FIRST_TICK;
            v.fx1 &= !HOLD_SUSTAIN;
            v.vibrato = 0;
            v.vib_note = 0;
            v.pt_vib_note = 0;
            v.tremolo = 0;
            v.volume = 0;
            v.volume_add = 0;
            v.volume_slide = 0;
            v.chan_vol_slide = 0;
            v.master_vol_slide = 0;
            v.pitch_slide = 0;
            v.pt_pitch_slide = 0;
            v.offset = 0;
            v.restart = 0;
            v.arp &= !(ARP_SETS_NOTE | ARP_USE_SAMPLE);
            if v.arp & ARP_ONE_STEP != 0 {
                v.arp = 0;
                v.arp_vol_slide = 0;
                v.arp_pitch_slide = 0;
                v.arp_pitch_slide_note = 0;
            }
            if v.part_note != 0 {
                v.pitch_slide_to = -1;
                v.pt_pitch_slide_to = -1;
            }
        }
        for i in 0..5 {
            let (e, a) = self.voices[ch].part_fx[i];
            self.one_fx(ch, e, a);
        }
    }

    fn one_fx(&mut self, ch: usize, e: u8, a: u8) {
        let (tune_speed, master) = (&mut self.tune_speed, &mut self.master);
        let nvoices = self.voices.len();
        match e {
            SPEED_ALL => {
                if a != 0 {
                    if a < 32 {
                        *tune_speed = a;
                        for i in 0..nvoices {
                            let w = &mut self.voices[i];
                            if ch >= i {
                                if w.speed_part == 0 {
                                    w.speed = a;
                                    if w.groove == 0 || !w.part_groove {
                                        w.speed_counter = a;
                                    }
                                }
                            } else {
                                w.speed = a;
                            }
                        }
                    } else {
                        self.tempo = u16::from(a);
                    }
                }
                return;
            }
            GROOVE_ALL => {
                if a != 0 {
                    let g = a & 0x1F;
                    self.tune_groove = g;
                    for i in 0..nvoices {
                        let w = &mut self.voices[i];
                        if ch >= i {
                            if w.groove_part == 0 {
                                w.groove = g;
                                if g != 0 && !w.part_groove {
                                    w.speed_counter = g;
                                }
                            }
                        } else {
                            w.groove = g;
                        }
                    }
                }
                return;
            }
            ALL_CHAN_VOLUME => {
                for w in &mut self.voices {
                    w.chan_volume = u16::from(a) << 4;
                }
                return;
            }
            MASTER_VOLUME => {
                *master = u16::from(a) << 4;
                return;
            }
            FILTER_FX => {
                self.filter = a != 0;
                return;
            }
            _ => {}
        }
        let master_now = self.master;
        let v = &mut self.voices[ch];
        match e {
            SLIDE_UP | SLIDE_DOWN => {
                v.pitch_slide = e;
                v.pitch_slide_down = e == SLIDE_DOWN;
                if a != 0 {
                    v.pitch_slide_speed = u16::from(a);
                }
                v.pitch_slide_to = if e == SLIDE_UP { 59 * 32 + 32 } else { 0 };
            }
            PORTAMENTO => {
                v.pitch_slide = e;
                if a != 0 {
                    v.pitch_slide_speed = u16::from(a);
                }
                let note = v.note.wrapping_add(v.pitch_slide_note);
                if v.part_note != 0 {
                    let to = u16::from(v.part_note) << 5;
                    v.part_note = 0;
                    v.pitch_slide_to = to as i16;
                    if to != note {
                        v.pitch_slide_down = to < note;
                        return;
                    }
                    v.pitch_slide_to = -1;
                }
                if v.pitch_slide_to == -1 {
                    v.pitch_slide = 0;
                }
            }
            INIT_INST_PORTA => v.inst_pitch_slide = 0,
            PITCH_UP => v.pitch_add = v.pitch_add.wrapping_add(i16::from(a)),
            PITCH_DOWN => v.pitch_add = v.pitch_add.wrapping_sub(i16::from(a)),
            VIBRATO_SPEED => {
                if a != 0 {
                    v.vib_speed = u16::from(a);
                }
            }
            VIBRATO_UP | VIBRATO_DOWN => {
                v.vibrato = e;
                if v.part_note != 0 {
                    v.vib_down = e == VIBRATO_DOWN;
                    v.vib_counter = 0;
                    v.vib_depth_now = 0;
                    v.vib_delay = 0;
                    v.vib_attack_speed = 0;
                    v.vib_attack_len = 0;
                }
                if a != 0 {
                    v.vib_depth = u16::from(a) << 8;
                }
                v.vib_wave = v.part_vib_wave;
            }
            VIBRATO_WAVE => {
                if a <= 3 {
                    v.vib_wave = a;
                    v.part_vib_wave = a;
                }
            }
            SET_FINETUNE => {
                v.finetune = i16::from((a as i8).clamp(-31, 31));
                v.play |= KEEP_FINETUNE;
            }
            INST_VOLUME => {
                v.volume = e;
                v.volume_set = (u16::from(a) << 4).min(1024);
            }
            INST_VOL_UP | INST_VOL_DOWN | PT_VOL_UP | PT_VOL_DOWN => {
                v.volume_slide = e;
                if a != 0 {
                    v.vol_slide_speed = u16::from(a);
                }
            }
            INST_VOL_SLIDE_TO_SET => v.vol_slide_to = u16::from(a) << 4,
            INST_VOL_SLIDE_TO => {
                v.volume_slide = e;
                if a != 0 {
                    v.vol_slide_speed = u16::from(a);
                }
                if v.vol_slide_to != v.vol1 {
                    v.vol_slide_down = v.vol_slide_to < v.vol1;
                    v.vol_slide_done = false;
                } else {
                    v.vol_slide_done = true;
                }
            }
            INST_VOL_ADD | INST_VOL_SUB => {
                v.volume_add = e;
                if a != 0 {
                    let n = i16::from(a) << 4;
                    v.volume_add_n = if e == INST_VOL_SUB { -n } else { n };
                }
            }
            TREMOLO_SPEED => {
                if a != 0 {
                    v.trem_speed = u16::from(a);
                }
            }
            TREMOLO_UP | TREMOLO_DOWN => {
                v.tremolo = e;
                if v.part_note != 0 {
                    v.trem_down = e == TREMOLO_DOWN;
                    v.trem_counter = 0;
                    v.trem_depth_now = 0;
                    v.trem_delay = 0;
                    v.trem_attack_speed = 0;
                    v.trem_attack_len = 0;
                }
                if a != 0 {
                    v.trem_depth = u16::from(a) << 8;
                }
                v.trem_wave = v.part_trem_wave;
            }
            TREMOLO_WAVE => {
                if a <= 3 {
                    v.trem_wave = a;
                    v.part_trem_wave = a;
                }
            }
            CHAN_VOLUME => v.chan_volume = (u16::from(a) << 4).min(1024),
            CHAN_VOL_UP | CHAN_VOL_DOWN => {
                v.chan_vol_slide = e;
                if a != 0 {
                    v.chan_vol_slide_speed = u16::from(a);
                }
            }
            CHAN_VOL_SLIDE_TO_SET => v.chan_vol_slide_to = u16::from(a) << 4,
            CHAN_VOL_SLIDE_TO => {
                v.chan_vol_slide = e;
                if a != 0 {
                    v.chan_vol_slide_speed = u16::from(a);
                }
                if v.chan_vol_slide_to != v.chan_volume {
                    v.chan_vol_slide_down = v.chan_vol_slide_to < v.chan_volume;
                    v.chan_vol_slide_done = false;
                } else {
                    v.chan_vol_slide_done = true;
                }
            }
            CHAN_VOL_ADD => {
                if a != 0 {
                    v.chan_vol_add_n = i16::from(a) << 4;
                }
                v.chan_volume = (v.chan_volume as i16).wrapping_add(v.chan_vol_add_n).min(1024) as u16;
            }
            CHAN_VOL_SUB => {
                if a != 0 {
                    v.chan_vol_add_n = i16::from(a) << 4;
                }
                v.chan_volume = (v.chan_volume as i16).wrapping_sub(v.chan_vol_add_n).max(0) as u16;
            }
            MASTER_VOL_UP | MASTER_VOL_DOWN => {
                v.master_vol_slide = e;
                if a != 0 {
                    v.master_vol_slide_speed = u16::from(a);
                }
            }
            MASTER_VOL_SLIDE_TO_SET => v.master_vol_slide_to = u16::from(a) << 4,
            MASTER_VOL_SLIDE_TO => {
                v.master_vol_slide = e;
                if a != 0 {
                    v.master_vol_slide_speed = u16::from(a);
                }
                if v.master_vol_slide_to != master_now {
                    v.master_vol_slide_down = v.master_vol_slide_to < master_now;
                    v.master_vol_slide_done = false;
                } else {
                    v.master_vol_slide_done = true;
                }
            }
            MASTER_VOL_ADD => {
                if a != 0 {
                    v.master_vol_add_n = i16::from(a) << 4;
                }
                self.master = (master_now as i16).wrapping_add(v.master_vol_add_n).min(1024) as u16;
            }
            MASTER_VOL_SUB => {
                if a != 0 {
                    v.master_vol_add_n = i16::from(a) << 4;
                }
                self.master = (master_now as i16).wrapping_sub(v.master_vol_add_n).max(0) as u16;
            }
            SPEED_ONE => {
                if a != 0 {
                    let s = a.min(31);
                    v.speed_part = 1;
                    v.speed = s;
                    if v.groove == 0 || !v.part_groove {
                        v.speed_counter = s;
                    }
                }
            }
            GROOVE_ONE => {
                if a != 0 {
                    let g = a.min(31);
                    v.groove_part = 1;
                    v.groove = g;
                    if v.groove == 0 || v.part_groove {
                        v.speed_counter = g;
                    }
                }
            }
            ARP_TABLE => {
                v.arp |= ARP_USE_TABLE;
                v.arp_table = a;
            }
            ARP_TABLE_ONCE => {
                v.arp |= ARP_ONE_STEP | ARP_USE_TABLE;
                v.arp_table = a;
            }
            HOLD_SUSTAIN_FX => {
                v.fx1 |= HOLD_SUSTAIN;
                v.fx1_arg &= !ENV_HOLD;
                if a != 0 {
                    v.fx1_arg |= ENV_HOLD;
                }
            }
            SAMPLE_OFFSET => {
                v.offset_on = true;
                if a != 0 {
                    v.offset = a;
                }
            }
            RESTART_NO_VOLUME => {
                if v.part_instr == 0 {
                    v.restart = FROM_PART;
                }
            }
            WAVE_SAMPLE => {
                if a != 0 {
                    v.wave_sample = Some(a);
                    v.arp |= ARP_USE_SAMPLE;
                }
            }
            INIT_INSTRUMENT => {
                v.phase_init = 0;
                v.res_init = 0;
                v.filter_init = 0;
                v.transform_init = 0;
                v.mix_init = 0;
                v.loop_init = 0;
            }
            PT_SLIDE_UP | PT_SLIDE_DOWN => {
                v.pt_pitch_slide = e;
                v.pt_pitch_slide_down = e == PT_SLIDE_DOWN;
                if a != 0 {
                    v.pt_pitch_slide_speed = u16::from(a);
                }
                v.pt_pitch_slide_to = if e == PT_SLIDE_UP { 106 } else { 3591 };
            }
            PT_PORTAMENTO => {
                v.pitch_slide = PORTAMENTO;
                v.pt_pitch_slide = e;
                if a != 0 {
                    v.pt_pitch_slide_speed2 = u16::from(a);
                }
                let period = period_of(v, v.note).wrapping_add(v.pt_pitch_slide_note);
                if v.part_note != 0 {
                    let to = period_of(v, u16::from(v.part_note) << 5);
                    v.part_note = 0;
                    v.pt_pitch_slide_to = to as i16;
                    if period != to {
                        v.pt_pitch_slide_down = period < to;
                        return;
                    }
                    v.pt_pitch_slide_to = -1;
                }
                if v.pt_pitch_slide_to == -1 {
                    v.pt_pitch_slide = 0;
                    v.pitch_slide = 0;
                }
            }
            PT_FINE_UP => {
                if a != 0 {
                    v.pt_pitch_add = v.pt_pitch_add.wrapping_sub(i16::from(a & 0x0F));
                }
            }
            PT_FINE_DOWN => {
                if a != 0 {
                    v.pt_pitch_add = v.pt_pitch_add.wrapping_add(i16::from(a & 0x0F));
                }
            }
            PT_TREMOLO => {
                v.tremolo = e;
                v.pt_trem_cmd = nibbles(v.pt_trem_cmd, a);
            }
            PT_TREMOLO_WAVE => v.pt_trem_wave = a & 0x0F,
            PT_VIBRATO => {
                v.vibrato = e;
                v.pt_vib_cmd = nibbles(v.pt_vib_cmd, a);
            }
            PT_VIBRATO_WAVE => v.pt_vib_wave = a & 0x0F,
            _ => {}
        }
    }

    // ---- The arpeggio, the instrument ----

    fn start_arpeggio(&mut self, song: &Song, ch: usize) {
        let v = &mut self.voices[ch];
        let Some(ii) = v.instrument.filter(|_| v.part_note != 0) else { return };
        let inst = instrument(song, ii);
        v.arp_wait = false;
        if v.arp & (ARP_USE_TABLE | ARP_ON) == 0 {
            if inst.effects1() & ARPEGGIO == 0 {
                return;
            }
            v.arp |= ARP_ON;
        }
        if v.restart == 0 && v.part_instr == 0 {
            return;
        }
        v.arp |= ARP_SETS_NOTE;
        let num = if v.arp & ARP_USE_TABLE != 0 { usize::from(v.arp_table) } else { usize::from(inst.arp_table()) };
        let Some(arp) = song.arpeggios.get(num) else { return };
        v.arp_pos = 0;
        v.arp_wait = false;
        v.arp_vol_slide = 0;
        v.arp_pitch_slide = 0;
        v.arp_pitch_slide_note = 0;
        v.arp_speed_counter = inst.arp_speed();
        for _ in 0..128 {
            let p = v.arp_pos;
            v.arp_pos = (v.arp_pos + 1) & 0x7F;
            let line = arp.lines[usize::from(p)];
            v.arp_note = v.part_note;
            let mut t = line.transpose;
            if t == 0 {
                v.arp_wait = true;
                return;
            }
            if t == 61 {
                v.fx1 &= !ARPEGGIO;
                return;
            }
            if t == 62 {
                continue;
            }
            v.wave_number = if line.sample == 0 { inst.sample() } else { line.sample };
            for e in line.fx {
                arp_effect(v, e);
            }
            if t < 0 {
                t = t.wrapping_add((61 + v.arp_note) as i8);
            } else {
                v.arp |= ARP_ABSOLUTE;
            }
            v.note = (i32::from(t) << 5) as u16;
            v.arp_calc_note = v.note;
            arp_wait_start(v, line.sample);
            return;
        }
    }

    fn play_instrument(&mut self, song: &Song, ch: usize) {
        let v = &mut self.voices[ch];
        let Some(ii) = v.instrument else { return };
        if v.arp_wait {
            return;
        }
        let inst = instrument(song, ii);
        let mut src = inst_src(song, ii);
        let mut kind = inst.kind();
        let restarted = v.restart & (FROM_PART | FROM_ARP) != 0;
        let mut note_only = false;
        if restarted {
            v.wave_number = inst.sample();
        } else if v.part_instr == 0 {
            note_only = true;
        } else if v.pitch_slide == PORTAMENTO || v.part_note == 0 {
            v.vol1 = inst.volume().wrapping_mul(16);
            note_only = true;
        } else if v.arp & ARP_USE_SAMPLE != 0 {
            let Some(ws) = v.wave_sample else { return };
            src = sample_src(song, ws);
            if src.kind == SAMPLE {
                kind = SAMPLE;
            }
        } else {
            v.wave_number = inst.sample();
        }
        if !note_only {
            let Some(ptr) = src.ptr else { return };
            if !inst.transposable() {
                v.transpose = 0;
            }
            let mut flag = inst.flags1();
            if v.fx1 & HOLD_SUSTAIN != 0 {
                flag &= !ENV_HOLD;
                v.fx1_arg &= ENV_HOLD;
                flag |= v.fx1_arg;
            }
            v.fx1_arg = flag;
            v.fx2_arg = inst.flags2();
            v.fx1 = inst.effects1();
            v.fx2 = inst.effects2();
            v.wave_or_sample = kind;
            if kind != SAMPLE {
                fix_wave_length(v, inst, kind, ptr);
            } else {
                let (mut len, mut p) = (src.len, ptr);
                if v.offset_on {
                    let off = u16::from(v.offset) << 7;
                    if off < len {
                        len -= off;
                        p = p.plus(u32::from(off) * 2);
                    } else {
                        len = 1;
                    }
                }
                v.play_ptr = Some(p);
                v.play_len = len;
                let (mut rp, mut rl) = (src.rep, src.rep_len);
                if v.arp & ARP_USE_SAMPLE == 0 && inst.effects1() & WAVE_SAMPLE_LOOP == 0 {
                    rp = None;
                    rl = 0;
                }
                v.rep_ptr = rp;
                v.rep_orig = rp;
                v.rep_len = rl;
            }
            if v.restart == 0 {
                v.vol1 = inst.volume().wrapping_mul(16);
            }
        }
        // The note.
        let mut volume = if v.volume != 0 { v.volume_set as i16 } else { v.vol1 as i16 };
        if v.volume_add != 0 {
            volume = volume.wrapping_add(v.volume_add_n).clamp(0, 1024);
        }
        v.vol1 = volume as u16;
        v.vol2 = volume as u16;
        v.vol3 = volume as u16;
        if v.pitch_slide == PORTAMENTO {
            return;
        }
        if inst.slide_speed() != 0 && v.part_note != 0 {
            v.pitch_slide_speed = u16::from(inst.slide_speed());
            let new_note = v.note.wrapping_add(v.pitch_slide_note);
            v.pitch_slide_to = i16::from(v.part_note) << 5;
            v.pitch_slide_down = i32::from(v.pitch_slide_to) < i32::from(new_note);
            if v.inst_pitch_slide != 0 {
                self.inst_play(song, ch, ii);
                return;
            }
            v.inst_pitch_slide = PORTAMENTO;
        }
        let sets_note = v.arp & ARP_SETS_NOTE != 0;
        v.arp &= !ARP_SETS_NOTE;
        if !sets_note {
            if v.part_note == 0 {
                return;
            }
            v.note = u16::from(v.part_note) << 5;
            v.arp_vol_slide = 0;
            v.arp_pitch_slide = 0;
            v.arp_pitch_slide_note = 0;
        }
        v.semitone = src.semitone.wrapping_shl(5);
        if v.play & KEEP_FINETUNE == 0 {
            v.finetune = src.finetune;
        }
        v.pt_pitch_slide_note = 0;
        v.pitch_slide_note = 0;
        v.pt_vib_note = 0;
        v.pt_trem_pos = 0;
        v.pt_vib_pos = 0;
        v.vib_note = 0;
        v.pt_pitch_add = 0;
        v.pitch_add = 0;
        if v.part_instr != 0 || v.restart & (FROM_PART | FROM_ARP) != 0 {
            self.inst_play(song, ch, ii);
        }
    }

    /// A note's instrument starts: its vibrato, tremolo, envelope and
    /// wave effects from their beginnings (or carried on, as it says).
    fn inst_play(&mut self, song: &Song, ch: usize, ii: usize) {
        let inst = instrument(song, ii);
        let v = &mut self.voices[ch];
        if v.pitch_slide != PORTAMENTO && !v.arp_wait {
            v.play |= RETRIGGER;
            if v.vibrato == 0 && v.fx1 & VIBRATO != 0 {
                let (up, wave, speed, delay, attack_speed, attack, depth) = inst.wobble(88);
                v.vib_counter = 0;
                v.vib_depth_now = 0;
                v.vib_speed = speed;
                v.vib_delay = delay;
                v.vib_attack_speed = attack_speed;
                v.vib_attack_len = attack;
                v.vib_depth = depth;
                v.vib_wave = wave;
                v.vib_down = !up;
            } else if v.fx1 & VIBRATO == 0 {
                v.vib_speed = 0;
            }
            if v.tremolo == 0 && v.fx1 & TREMOLO != 0 {
                let (up, wave, speed, delay, attack_speed, attack, depth) = inst.wobble(100);
                v.trem_counter = 0;
                v.trem_depth_now = 0;
                v.trem_speed = speed;
                v.trem_delay = delay;
                v.trem_attack_speed = attack_speed;
                v.trem_attack_len = attack;
                v.trem_depth = depth;
                v.trem_wave = wave;
                v.trem_down = !up;
            }
            if v.fx1 & ENVELOPE != 0 {
                v.env_volume = 0;
                v.env = inst.envelope();
            }
            let instr = v.part_instr;
            if v.fx2 & PHASE != 0 {
                v.phase_type = inst.phase_type();
                v.phase_c.start(&mut v.phase_speed, &mut v.phase_init, v.fx1_arg & PHASE_STEP != 0, v.fx1_arg & PHASE_INIT != 0, instr, inst.phase());
            }
            v.boost = inst.boost();
            if v.fx2 & RESONANCE != 0 {
                v.res_amp = inst.resonance_amp();
                if v.res_c.start(&mut v.res_speed, &mut v.res_init, v.fx2_arg & RES_STEP != 0, v.fx2_arg & RES_INIT != 0, instr, inst.resonance()) {
                    v.res_last_init = true;
                }
            }
            if v.fx2 & FILTER != 0 {
                v.filter_type = inst.filter_type();
                if v.filter_c.start(&mut v.filter_speed, &mut v.filter_init, v.fx1_arg & FILTER_STEP != 0, v.fx1_arg & FILTER_INIT != 0, instr, inst.filter()) {
                    v.filter_last_init = true;
                }
            }
            if v.fx2 & MIX != 0 {
                v.mix_wave = inst.mix_wave();
                v.mix_c.start(&mut v.mix_speed, &mut v.mix_init, v.fx2_arg & MIX_STEP != 0, v.fx2_arg & MIX_INIT != 0, instr, inst.mix());
            }
            if v.fx2 & TRANSFORM != 0 {
                v.transform_waves[0] = inst.sample();
                v.transform_waves[1..].copy_from_slice(&inst.transform_waves());
                v.transform_c.start(&mut v.transform_speed, &mut v.transform_init, v.fx1_arg & TRANSFORM_STEP != 0, v.fx1_arg & TRANSFORM_INIT != 0, instr, inst.transform());
            }
            if v.fx1 & LOOP != 0 {
                let [start, repeat, repeat_end, length, step, wait, delay, turns] = inst.moving_loop();
                if inst.kind() != SAMPLE || length == 0 {
                    v.fx1 &= !LOOP;
                } else {
                    let (mut init, mut again, mut check) = (false, false, false);
                    if v.fx2_arg & LOOP_STEP != 0 {
                        v.loop_speed = turns as i16;
                        v.loop_turns = 0;
                        check = true;
                    } else {
                        v.loop_turns = turns as i16;
                        if v.fx2_arg & LOOP_INIT == 0 {
                            v.loop_init = 0;
                            init = true;
                        } else {
                            check = true;
                        }
                    }
                    if check {
                        let old = v.loop_init;
                        v.loop_init = instr;
                        if old == instr {
                            v.play &= !RETRIGGER;
                            again = true;
                        } else {
                            init = true;
                        }
                    }
                    if init {
                        let n = inst.sample();
                        let s = song.samples.get(usize::from(n)).filter(|s| !s.data.is_empty() && n != 0);
                        v.loop_sample = s.map(|_| Ptr::at(Buf::Sample(n), 0));
                        v.loop_counter = start;
                        v.loop_counter_save = start;
                        v.loop_max = s.map_or(0, |s| s.length).wrapping_sub(length);
                        v.loop_length = length;
                        v.loop_repeat_end = repeat_end;
                        v.loop_wait = wait;
                        v.loop_step = i32::from(step);
                        v.loop_repeat = repeat;
                        if start > repeat {
                            v.loop_step = -v.loop_step;
                        }
                    }
                    if init || again {
                        v.loop_delay = delay as i16;
                        v.loop_wait_counter = 0;
                        if let Some(ls) = v.loop_sample {
                            let p = ls.plus(u32::from(v.loop_counter_save) * 2);
                            v.play_ptr = Some(p);
                            v.rep_ptr = Some(p);
                            v.rep_orig = Some(p);
                            v.play_len = v.loop_length;
                            v.rep_len = v.loop_length;
                        }
                    }
                }
            }
        }
        // Only a wave of 16 to 256 points takes the wave effects.
        if !matches!(v.rep_len, 8 | 16 | 32 | 64 | 128) {
            v.fx2 = 0;
        }
    }

    // ---- The effects, every tick ----

    fn effects(&mut self, song: &Song, ch: usize) {
        let master = self.master;
        {
            let v = &mut self.voices[ch];
            v.rep_ptr = v.rep_orig;
        }
        if self.voices[ch].play & RETRIGGER == 0 {
            let v = &mut self.voices[ch];
            slide_volume(v);
            slide_channel_volume(v);
            self.master = slide_master_volume(v, master);
            let v = &mut self.voices[ch];
            slide_arp_volume(v);
            slide_note(v);
            slide_arp_note(v);
            self.arpeggio(song, ch);
            let v = &mut self.voices[ch];
            vibrato(v);
            tremolo(v);
        }
        let v = &mut self.voices[ch];
        envelope(v);
        move_loop(v);
        self.transform(song, ch);
        self.phase(song, ch);
        self.mix(song, ch);
        self.resonance(song, ch);
        self.filter_fx(song, ch);
        let v = &mut self.voices[ch];
        // What an effect made is played from the voice's own wave.
        if let Some(k) = v.rep_ptr.and_then(|p| p.buf.own()).filter(|&k| k != 5) {
            let n = (usize::from(v.rep_len) * 2).min(256);
            let src = v.bufs.0[k];
            v.bufs.0[5][..n].copy_from_slice(&src[..n]);
            let p = Ptr::at(Buf::Wave, 0);
            v.rep_ptr = Some(p);
            if v.wave_or_sample != SAMPLE || v.fx1 & LOOP != 0 {
                v.play_ptr = Some(p);
            }
        }
        if v.play & RETRIGGER == 0 && v.fx1 & LOOP != 0 {
            if let Some(p) = v.rep_ptr {
                v.out.repeat = Some(Region { buf: p.buf, offset: p.off, len: u32::from(v.loop_length) * 2 });
            }
            v.rep_len = v.loop_length;
        }
        if !v.arp_wait {
            v.part_note = 0;
        }
        v.play &= RETRIGGER;
    }

    fn arpeggio(&mut self, song: &Song, ch: usize) {
        let v = &mut self.voices[ch];
        if v.arp & (ARP_USE_TABLE | ARP_ON) == 0 {
            return;
        }
        v.arp_speed_counter = v.arp_speed_counter.wrapping_sub(1);
        if v.arp_speed_counter != 0 {
            return;
        }
        let Some(ii) = v.instrument else { return };
        let inst = instrument(song, ii);
        let mut spd = inst.arp_speed();
        v.arp_groove = !v.arp_groove;
        if v.arp_groove && inst.arp_groove() != 0 {
            spd = inst.arp_groove();
        }
        v.arp_speed_counter = spd;
        let num = if v.arp & ARP_USE_TABLE != 0 { usize::from(v.arp_table) } else { usize::from(inst.arp_table()) };
        let Some(arp) = song.arpeggios.get(num) else { return };
        for _ in 0..256 {
            let p = v.arp_pos;
            v.arp_pos = (v.arp_pos + 1) & 0x7F;
            let line = arp.lines[usize::from(p)];
            if v.arp_wait && line.transpose == 0 {
                return;
            }
            let mut t = line.transpose;
            if t == 61 {
                v.arp = 0;
                v.note = v.arp_calc_note;
                return;
            }
            if t == 62 {
                if line.sample != p {
                    v.arp_pos = line.sample & 0x7F;
                }
                continue;
            }
            v.wave_number = if line.sample == 0 { inst.sample() } else { line.sample };
            v.restart = 0;
            v.arp_pitch_slide = 0;
            v.arp_vol_slide = 0;
            for e in line.fx {
                if e.0 == ARP_RESTART {
                    arp_restart(v);
                } else {
                    arp_effect(v, e);
                }
            }
            v.arp &= !ARP_ABSOLUTE;
            if t == 0 {
                return;
            }
            if t < 0 {
                t = t.wrapping_add((61 + v.arp_note) as i8);
            } else {
                v.arp |= ARP_ABSOLUTE;
            }
            v.note = (i32::from(t) << 5) as u16;
            v.arp_calc_note = v.note;
            v.arp_pitch_slide_note = 0;
            if v.arp_wait {
                v.arp_wait = false;
                arp_wait_start(v, line.sample);
                self.play_instrument(song, ch);
                return;
            }
            if v.restart & FROM_ARP != 0 {
                v.arp |= ARP_SETS_NOTE;
                if line.sample != 0 {
                    v.wave_sample = Some(line.sample);
                    v.arp |= ARP_USE_SAMPLE;
                }
                self.play_instrument(song, ch);
                return;
            }
            if line.sample == 0 {
                v.semitone = inst.semitone().wrapping_shl(5);
                if v.play & KEEP_FINETUNE == 0 {
                    v.finetune = inst.finetune();
                }
                return;
            }
            v.play |= RETRIGGER;
            let s = sample_src(song, line.sample);
            v.semitone = s.semitone.wrapping_shl(5);
            if v.play & KEEP_FINETUNE == 0 {
                v.finetune = s.finetune;
            }
            v.wave_or_sample = s.kind;
            if v.wave_or_sample != SAMPLE {
                let kind = if inst.kind() == SAMPLE { 3 } else { inst.kind() };
                v.wave_or_sample = kind;
                fix_wave_length(v, inst, kind, Ptr::at(Buf::Sample(line.sample), 0));
            } else {
                v.play_ptr = s.ptr;
                v.play_len = s.len;
                let (rp, rl) = if s.rep_len == 0 { (None, 0) } else { (s.rep, s.rep_len) };
                v.rep_ptr = rp;
                v.rep_orig = rp;
                v.rep_len = rl;
            }
            v.fx2 = if matches!(v.rep_len, 8 | 16 | 32 | 64 | 128) { inst.effects2() } else { 0 };
            return;
        }
    }

    // ---- The wave effects ----

    /// `n` points from `p` (silence past the end of what's there).
    fn fetch(&self, song: &Song, ch: usize, p: Ptr, out: &mut [i8]) {
        let r = Region { buf: p.buf, offset: p.off, len: out.len() as u32 };
        let have = self.bytes(song, ch, r);
        out[..have.len()].copy_from_slice(have);
        out[have.len()..].fill(0);
    }

    fn transform(&mut self, song: &Song, ch: usize) {
        let v = &mut self.voices[ch];
        if v.fx2 & TRANSFORM == 0 {
            return;
        }
        let rl = usize::from(v.rep_len) * 2;
        let octave = octave_offset(rl);
        let (step, keep, new_note) = (v.fx1_arg & TRANSFORM_STEP != 0, v.fx1_arg & TRANSFORM_INIT != 0, v.part_note != 0);
        let mut c = v.transform_c.value(step, keep, new_note, false, v.transform_speed, (0, 510)) >> 1;
        // Past each 256, the next pair of waves.
        let mut idx = 0usize;
        let mut rem = c as i16;
        for _ in 0..5 {
            rem = rem.wrapping_sub(256);
            if rem <= 0 {
                break;
            }
            idx += 1;
            c = rem as u16;
        }
        let factor = i32::from(c);
        let n1 = v.transform_waves[idx.min(5)];
        let n2 = v.transform_waves.get(idx + 1).copied().unwrap_or(0);
        if n1 == 0 {
            return;
        }
        let rep_of = |n: u8| song.samples.get(usize::from(n)).filter(|s| n != 0 && !s.data.is_empty()).map(|s| (Ptr::at(Buf::Sample(n), s.repeat + octave), s.data.len()));
        let src1 = if idx == 0 {
            match v.rep_ptr {
                Some(p) => p,
                None => return,
            }
        } else {
            match rep_of(n1) {
                Some((p, _)) => p,
                None => return,
            }
        };
        let Some((src2, len2)) = rep_of(n2) else { return };
        if src2.off as usize >= len2 {
            return;
        }
        let (mut a, mut b) = ([0i8; 256], [0i8; 256]);
        self.fetch(song, ch, src1, &mut a);
        self.fetch(song, ch, src2, &mut b);
        let v = &mut self.voices[ch];
        let p = Ptr::at(Buf::Transform, 0);
        v.rep_ptr = Some(p);
        if v.fx1 & LOOP != 0 || v.wave_or_sample != SAMPLE {
            v.play_ptr = Some(p);
        }
        let out = &mut v.bufs.0[0];
        for i in 0..rl.min(256) {
            let (s1, s2) = (i32::from(a[i]), i32::from(b[i]));
            // The 68000 shifts the product's low word only: it wraps.
            out[i] = ((((s2 - s1) * factor) as i16 >> 8) as i32 + s1) as i8;
        }
    }

    fn phase(&mut self, song: &Song, ch: usize) {
        let v = &mut self.voices[ch];
        if v.fx2 & PHASE == 0 {
            return;
        }
        let (step, keep, new_note) = (v.fx1_arg & PHASE_STEP != 0, v.fx1_arg & PHASE_INIT != 0, v.part_note != 0);
        let mut c = phase_value(&mut v.phase_c, step, keep, new_note, v.phase_speed);
        let rl = usize::from(v.rep_len) * 2;
        let len = match rl {
            129.. => {
                c = c.wrapping_add(1) >> 1;
                256
            }
            65..=128 => {
                c = c.wrapping_add(3) >> 2;
                128
            }
            33..=64 => {
                c = c.wrapping_add(7) >> 3;
                64
            }
            17..=32 => {
                c = c.wrapping_add(15) >> 4;
                32
            }
            _ => {
                c = c.wrapping_add(31) >> 5;
                16
            }
        };
        let Some(rep) = v.rep_ptr else { return };
        // Past the three blends, the plain squeeze.
        let (fill, kind) = (v.fx1_arg & PHASE_FILL != 0, if v.phase_type > 3 { 0 } else { v.phase_type });
        let p = Ptr::at(Buf::Phase, 0);
        v.rep_ptr = Some(p);
        if v.fx1 & LOOP != 0 || v.wave_or_sample != SAMPLE {
            v.play_ptr = Some(p);
        }
        let mut src = [0i8; 256];
        self.fetch(song, ch, rep, &mut src);
        let out = &mut self.voices[ch].bufs.0[1];
        let rl = rl.min(256);
        let k = usize::from(c);
        if k >= rl || k == 0 {
            out[..rl].copy_from_slice(&src[..rl]);
            return;
        }
        let left = rl - k;
        // Where the squeezed cycle's points come from: row k of the
        // resampling table, read on past the row's end where the
        // replayer reads on.
        let first = k * (k - 1) / 2;
        let at = |o: usize| -> usize { sizer_at(len, o).min(255) };
        let mix = |s1: i16, s2: i16| -> i8 {
            match kind {
                1 => ((s1 + s2 + s1 * 2) >> 2) as i8,
                2 => ((s1 + s2) >> 1) as i8,
                3 => ((s1 + s2 + s2 * 2) >> 2) as i8,
                _ => s1 as i8,
            }
        };
        let mut last = 0usize;
        for i in 0..k {
            last = at(first + i);
            out[i] = if kind == 0 { src[last] } else { mix(i16::from(src[last]), i16::from(src[i])) };
        }
        if kind == 0 {
            if fill {
                for i in 0..left {
                    out[k + i] = out[i];
                }
            } else {
                for i in 0..left {
                    out[k + i] = src[last];
                }
            }
            return;
        }
        if fill {
            // Low and High read the table on; Med starts its row over each time.
            let mut o = first;
            let mut i = k;
            while i < rl {
                if kind == 2 {
                    o = first;
                }
                let n = k.min(rl - i);
                for _ in 0..n {
                    let idx = at(o);
                    o += 1;
                    out[i] = mix(i16::from(src[idx]), i16::from(src[i]));
                    i += 1;
                }
            }
        } else {
            let s1 = i16::from(src[last]);
            for i in k..rl {
                let s2 = i16::from(src[i]);
                out[i] = match kind {
                    1 => ((s1 * 3 + s2) >> 2) as i8,
                    2 => ((s1 + s2) >> 1) as i8,
                    _ => ((s1 + s2 + s2 * 2) >> 2) as i8,
                };
            }
        }
    }

    fn mix(&mut self, song: &Song, ch: usize) {
        let v = &mut self.voices[ch];
        if v.fx2 & MIX == 0 {
            return;
        }
        let (step, keep, new_note) = (v.fx2_arg & MIX_STEP != 0, v.fx2_arg & MIX_INIT != 0, v.part_note != 0);
        let around = v.fx2_arg & MIX_COUNTER != 0;
        let mut c = v.mix_c.value(step, keep, new_note, around, v.mix_speed, (0, 510));
        let rl = usize::from(v.rep_len) * 2;
        let shift = match rl {
            129.. => 1,
            65..=128 => 2,
            33..=64 => 3,
            17..=32 => 4,
            _ => 5,
        };
        c >>= shift;
        let octave = octave_offset(rl.max(1));
        let Some(rep) = v.rep_ptr else { return };
        let own = v.fx2_arg & MIX_BUFF != 0;
        let other = if own {
            None
        } else if v.mix_wave != 0 {
            match song.samples.get(usize::from(v.mix_wave)).filter(|s| !s.data.is_empty()) {
                Some(s) => Some(Ptr::at(Buf::Sample(v.mix_wave), s.repeat + octave)),
                None => return,
            }
        } else {
            Some(rep)
        };
        let p = Ptr::at(Buf::Mix, 0);
        v.rep_ptr = Some(p);
        if v.wave_or_sample != SAMPLE {
            v.play_ptr = Some(p);
        }
        let shift = if v.boost & BOOST_MIX != 0 { 0 } else { 1 };
        let rl = rl.min(256);
        let pc = usize::from(c).min(rl);
        let body = rl - pc;
        let mut a = [0i8; 256];
        self.fetch(song, ch, rep, &mut a);
        let mut b = [0i8; 512];
        if let Some(o) = other {
            self.fetch(song, ch, o, &mut b);
        }
        let out = &mut self.voices[ch].bufs.0[2];
        let sum = |x: i8, y: i8| ((i16::from(x) + i16::from(y)) >> shift) as i8;
        if own {
            // The mix buffer mixed into itself as it's written.
            for i in 0..body {
                out[i] = sum(a[i], out[pc + i]);
            }
            for i in 0..pc {
                out[body + i] = sum(a[body + i], out[i]);
            }
        } else {
            for i in 0..body {
                out[i] = sum(a[i], b[pc + i]);
            }
            for i in 0..pc {
                out[body + i] = sum(a[body + i], b[i]);
            }
        }
    }

    fn resonance(&mut self, song: &Song, ch: usize) {
        let v = &mut self.voices[ch];
        if v.fx2 & RESONANCE == 0 {
            return;
        }
        let (step, keep, new_note) = (v.fx2_arg & RES_STEP != 0, v.fx2_arg & RES_INIT != 0, v.part_note != 0);
        let c = v.res_c.value(step, keep, new_note, false, v.res_speed, (0, 510));
        let Some(rep) = v.rep_ptr else { return };
        let p = Ptr::at(Buf::Resonance, 0);
        v.rep_ptr = Some(p);
        if v.fx1 & LOOP != 0 || v.wave_or_sample != SAMPLE {
            v.play_ptr = Some(p);
        }
        let rl = (usize::from(v.rep_len) * 2).min(256);
        let mut src = [0i8; 256];
        self.fetch(song, ch, rep, &mut src);
        let v = &mut self.voices[ch];
        let mut last = v.res_last;
        if v.res_last_init {
            v.res_last_init = false;
            last = src[rl.max(1) - 1] >> 2;
        }
        let mut state = i16::from(last) << 7;
        let cutoff = i32::from(tables::resonance()[(usize::from(c & 0xFFFE) / 2).min(255)]);
        let mut damp: u16 = 0x8000 - tables::damping()[usize::from(v.res_amp)];
        damp = ((u32::from(damp) * 0xE666) >> 16) as u16;
        let shift = if v.boost & BOOST_RES != 0 { 6 } else { 7 };
        let mut vel: i16 = 0;
        let mut out = last;
        for i in 0..rl {
            // The 68000's word subtraction, and its divide, which leaves
            // the dividend where the quotient won't fit a word.
            let num = i32::from((i16::from(src[i]) << 5).wrapping_sub(state)) << 7;
            let q = num / cutoff;
            let acc = if i16::try_from(q).is_ok() { q } else { i32::from(num as i16) };
            vel = (i32::from(vel) + acc) as i16;
            state = (i32::from(state) + i32::from(vel)) as i16;
            out = (state >> shift) as i8;
            v.bufs.0[3][i] = out;
            vel = ((i32::from(vel) * i32::from(damp as i16)) >> 15) as i16;
        }
        v.res_last = out;
    }

    fn filter_fx(&mut self, song: &Song, ch: usize) {
        let v = &mut self.voices[ch];
        if v.fx2 & FILTER == 0 {
            return;
        }
        let (step, keep, new_note) = (v.fx1_arg & FILTER_STEP != 0, v.fx1_arg & FILTER_INIT != 0, v.part_note != 0);
        let c = v.filter_c.value(step, keep, new_note, false, v.filter_speed, (0, 510));
        let Some(rep) = v.rep_ptr else { return };
        let p = Ptr::at(Buf::Filter, 0);
        v.rep_ptr = Some(p);
        if v.fx1 & LOOP != 0 || v.wave_or_sample != SAMPLE {
            v.play_ptr = Some(p);
        }
        let rl = (usize::from(v.rep_len) * 2).min(256);
        let mut src = [0i8; 256];
        self.fetch(song, ch, rep, &mut src);
        let v = &mut self.voices[ch];
        let resonant = v.filter_type != 0;
        let mut last = v.filter_last;
        if v.filter_last_init {
            v.filter_last_init = false;
            let x = src[rl.max(1) - 1];
            last = if resonant { x >> 1 } else { x };
        }
        let mut state = i16::from(last) << 7;
        let idx = (usize::from(c & 0xFFFE) / 2).min(255);
        let (mut cutoff, damp, up) = if resonant {
            let cut = tables::resonant_filter()[idx];
            (cut, ((u32::from(0x8000 - cut) * 0xE666) >> 16) as u16, 6)
        } else {
            let cut = tables::filter()[idx];
            let d = 0x8000u16.wrapping_sub(cut);
            (cut, ((i32::from(d as i16) * i32::from(0xF000u16 as i16)) >> 16) as u16, 7)
        };
        cutoff >>= 1;
        let shift = if v.boost & BOOST_FILTER != 0 { 6 } else { 7 };
        let mut vel: i16 = 0;
        let mut out = last;
        for i in 0..rl {
            // The difference is a word on the 68000, wrapping.
            let acc = (i32::from((i16::from(src[i]) << up).wrapping_sub(state)) * i32::from(cutoff as i16)) >> 14;
            vel = (i32::from(vel) + acc) as i16;
            state = (i32::from(state) + i32::from(vel)) as i16;
            out = (state >> shift) as i8;
            v.bufs.0[4][i] = out;
            vel = ((i32::from(vel) * i32::from(damp as i16)) >> 15) as i16;
        }
        v.filter_last = out;
    }

    // ---- The periods, and the channels told ----

    fn period(&mut self, ch: usize) {
        let v = &mut self.voices[ch];
        let mut note = sum_note(v, v.note) as i32;
        if v.arp & ARP_ABSOLUTE == 0 && v.transpose != 0 {
            note = i32::from((note as i16).wrapping_add(v.transpose));
        }
        let note = note.clamp(-32, 5 * 12 * 32);
        let mut period = tables::period(note);
        period = period.wrapping_add(v.pt_pitch_slide_note.wrapping_add(v.pt_vib_note).wrapping_add(v.pt_pitch_add as u16));
        v.period = period.clamp(106, 3591);
    }

    fn dma(&mut self, song: &Song, ch: usize) {
        let master = u32::from(self.master);
        let v = &mut self.voices[ch];
        let volume = (((u32::from(v.vol3) * u32::from(v.chan_volume)) / 1024 * master) / 16384) as u16;
        v.out.period = v.period;
        v.out.volume = volume.min(64);
        if v.play & RETRIGGER == 0 {
            return;
        }
        let n = v.wave_number;
        let old = v.wave_number_old;
        v.wave_number_old = n;
        let retrig = v.wave_or_sample == SAMPLE || n != old;
        v.play &= !RETRIGGER;
        v.out.sample = n;
        if retrig {
            if let Some(p) = v.play_ptr {
                let all = data_len(song, p.buf);
                let len = (u32::from(v.play_len) * 2).min(all.saturating_sub(p.off));
                v.out.restart = true;
                v.out.play = Region { buf: p.buf, offset: p.off, len };
            }
        }
        if v.rep_len != 0 {
            if let Some(p) = v.rep_ptr {
                let all = data_len(song, p.buf);
                let len = (u32::from(v.rep_len) * 2).min(all.saturating_sub(p.off));
                v.out.repeat = Some(Region { buf: p.buf, offset: p.off, len });
            }
        }
    }
}

fn data_len(song: &Song, b: Buf) -> u32 {
    match (b, b.own()) {
        (_, Some(_)) => 256,
        (Buf::Sample(n), _) => song.samples.get(usize::from(n)).map_or(0, |s| s.data.len() as u32),
        _ => 2,
    }
}

/// Where a wave's octave of `rl` bytes starts among its halvings.
fn octave_offset(rl: usize) -> u32 {
    match rl {
        256 => 0,
        128 => 256,
        64 => 384,
        32 => 448,
        _ => 480,
    }
}

/// The phase effect's counter: like the others, but its step goes the
/// other way and is held between 2 and 512.
fn phase_value(c: &mut Counter, step: bool, keep: bool, new_note: bool, step_speed: u16) -> u16 {
    let count = if step {
        if keep {
            if new_note {
                c.save = c.counter;
            }
            true
        } else if new_note {
            c.save = c.counter;
            true
        } else {
            false
        }
    } else {
        c.save = c.counter;
        true
    };
    if count {
        c.bounce();
    }
    let value = c.save;
    if step {
        if c.delay != 0 {
            c.delay -= 1;
        } else {
            let speed = i16::from(step_speed as u8 as i8);
            let s = c.save.wrapping_sub(speed as u16);
            c.save = if speed < 0 {
                if (s as i16) > 512 {
                    512
                } else {
                    s
                }
            } else if (s as i16) < 2 {
                2
            } else {
                s
            };
        }
    }
    value
}

/// The resampling table read at offset `o`: its rows (k = 1, 2...) one
/// after another.
fn sizer_at(len: usize, o: usize) -> usize {
    let mut k = 1;
    let mut start = 0;
    while start + k <= o {
        start += k;
        k += 1;
    }
    tables::sizer(len, k, o - start + 1)
}

fn nibbles(old: u8, a: u8) -> u8 {
    let mut c = old;
    if a & 0x0F != 0 {
        c = (c & 0xF0) | (a & 0x0F);
    }
    if a & 0xF0 != 0 {
        c = (c & 0x0F) | (a & 0xF0);
    }
    c
}

fn sum_note(v: &Voice, note: u16) -> i16 {
    (i32::from(note) + i32::from(v.vib_note) + i32::from(v.pitch_slide_note) + i32::from(v.arp_pitch_slide_note) + i32::from(v.semitone) + i32::from(v.finetune) + i32::from(v.pitch_add)) as i16
}

/// The period a note gives, transposed (the ProTracker portamento's).
fn period_of(v: &Voice, note: u16) -> u16 {
    let mut n = sum_note(v, note);
    if v.transpose != 0 {
        n = n.wrapping_add(v.transpose);
    }
    tables::period(i32::from(n).clamp(-32, 5 * 12 * 32))
}

fn fix_wave_length(v: &mut Voice, inst: &Instrument, kind: u8, ptr: Ptr) {
    let mut len = inst.length();
    let mut p = ptr;
    match kind {
        1 => {
            p = p.plus(480);
            len >>= 4;
        }
        2 => {
            p = p.plus(448);
            len >>= 3;
        }
        3 => {
            p = p.plus(384);
            len >>= 2;
        }
        4 => {
            p = p.plus(256);
            len >>= 1;
        }
        _ => {}
    }
    v.play_ptr = Some(p);
    v.rep_ptr = Some(p);
    v.rep_orig = Some(p);
    v.play_len = len;
    v.rep_len = len;
}

fn arp_wait_start(v: &mut Voice, n: u8) {
    if n == 0 {
        return;
    }
    v.wave_sample = Some(n);
    v.arp |= ARP_USE_SAMPLE;
}

fn arp_effect(v: &mut Voice, (e, a): Effect) {
    match e {
        ARP_SLIDE_UP | ARP_SLIDE_DOWN => {
            v.arp_pitch_slide = e;
            v.arp_pitch_down = e == ARP_SLIDE_DOWN;
            if a != 0 {
                v.arp_pitch_slide_speed = u16::from(a);
                v.arp_pitch_slide_to = if e == ARP_SLIDE_UP { 59 * 32 + 32 } else { 0 };
            }
        }
        ARP_SET_VOLUME => {
            let vol = (u16::from(a) << 4).min(1024);
            v.restart |= ARP_SET_VOL;
            v.vol1 = vol;
            v.vol2 = vol;
            v.vol3 = vol;
        }
        ARP_VOL_UP | ARP_VOL_DOWN => {
            v.arp_vol_slide = e;
            if a != 0 {
                v.arp_vol_slide_speed = u16::from(a);
            }
        }
        _ => {}
    }
}

fn arp_restart(v: &mut Voice) {
    v.restart |= FROM_ARP;
    if v.fx1_arg & PHASE_INIT != 0 {
        v.phase_init = 0;
    }
    // The replayer tests the resonance's and the mix's flags in the
    // wrong set (the phase step's and the envelope hold's bits): kept,
    // as songs sound that way.
    if v.fx1_arg & PHASE_STEP != 0 {
        v.res_init = 0;
    }
    if v.fx1_arg & FILTER_INIT != 0 {
        v.filter_init = 0;
    }
    if v.fx1_arg & TRANSFORM_INIT != 0 {
        v.transform_init = 0;
    }
    if v.fx1_arg & ENV_HOLD != 0 {
        v.mix_init = 0;
    }
    if v.fx2_arg & LOOP_INIT != 0 {
        v.loop_init = 0;
    }
}

fn slide_volume(v: &mut Voice) {
    let first = v.play & FIRST_TICK != 0;
    let vol = match v.volume_slide {
        INST_VOL_SLIDE_TO => {
            if v.vol_slide_done {
                return;
            }
            let speed = v.vol_slide_speed as i16;
            if v.vol_slide_down {
                let x = (v.vol1 as i16).wrapping_sub(speed);
                if i32::from(x) < i32::from(v.vol_slide_to) {
                    v.vol_slide_done = true;
                    v.vol_slide_to as i16
                } else {
                    x
                }
            } else {
                let x = (v.vol1 as i16).wrapping_add(speed);
                if i32::from(x) > i32::from(v.vol_slide_to) {
                    v.vol_slide_done = true;
                    v.vol_slide_to as i16
                } else {
                    x
                }
            }
        }
        INST_VOL_UP => (v.vol1 as i16).wrapping_add(v.vol_slide_speed as i16).min(1024),
        PT_VOL_UP if !first => (v.vol1 as i16).wrapping_add(v.vol_slide_speed as i16).min(1024),
        INST_VOL_DOWN => (v.vol1 as i16).wrapping_sub(v.vol_slide_speed as i16).max(0),
        PT_VOL_DOWN if !first => (v.vol1 as i16).wrapping_sub(v.vol_slide_speed as i16).max(0),
        _ => return,
    };
    v.vol1 = vol as u16;
    v.vol2 = vol as u16;
    v.vol3 = vol as u16;
}

fn slide_channel_volume(v: &mut Voice) {
    let vol = match v.chan_vol_slide {
        CHAN_VOL_SLIDE_TO => {
            if v.chan_vol_slide_done {
                return;
            }
            let speed = v.chan_vol_slide_speed as i16;
            if v.chan_vol_slide_down {
                let x = (v.chan_volume as i16).wrapping_sub(speed);
                if i32::from(x) < i32::from(v.chan_vol_slide_to) {
                    v.chan_vol_slide_done = true;
                    v.chan_vol_slide_to as i16
                } else {
                    x
                }
            } else {
                let x = (v.chan_volume as i16).wrapping_add(speed);
                if i32::from(x) > i32::from(v.chan_vol_slide_to) {
                    v.chan_vol_slide_done = true;
                    v.chan_vol_slide_to as i16
                } else {
                    x
                }
            }
        }
        CHAN_VOL_UP => (v.chan_volume as i16).wrapping_add(v.chan_vol_slide_speed as i16).min(1024),
        CHAN_VOL_DOWN => (v.chan_volume as i16).wrapping_sub(v.chan_vol_slide_speed as i16).max(0),
        _ => return,
    };
    v.chan_volume = vol as u16;
}

fn slide_master_volume(v: &mut Voice, master: u16) -> u16 {
    let vol = match v.master_vol_slide {
        MASTER_VOL_SLIDE_TO => {
            if v.master_vol_slide_done {
                return master;
            }
            let speed = v.master_vol_slide_speed as i16;
            if v.master_vol_slide_down {
                let x = (master as i16).wrapping_sub(speed);
                if i32::from(x) < i32::from(v.master_vol_slide_to) {
                    v.master_vol_slide_done = true;
                    v.master_vol_slide_to as i16
                } else {
                    x
                }
            } else {
                let x = (master as i16).wrapping_add(speed);
                if i32::from(x) > i32::from(v.master_vol_slide_to) {
                    v.master_vol_slide_done = true;
                    v.master_vol_slide_to as i16
                } else {
                    x
                }
            }
        }
        MASTER_VOL_UP => (master as i16).wrapping_add(v.master_vol_slide_speed as i16).min(1024),
        MASTER_VOL_DOWN => (master as i16).wrapping_sub(v.master_vol_slide_speed as i16).max(0),
        _ => return master,
    };
    vol as u16
}

fn slide_arp_volume(v: &mut Voice) {
    let vol = match v.arp_vol_slide {
        ARP_VOL_UP => (v.vol1 as i16).wrapping_add(v.arp_vol_slide_speed as i16).min(1024),
        ARP_VOL_DOWN => (v.vol1 as i16).wrapping_sub(v.arp_vol_slide_speed as i16).max(0),
        _ => return,
    };
    v.vol1 = vol as u16;
    v.vol2 = vol as u16;
    v.vol3 = vol as u16;
}

fn slide_note(v: &mut Voice) {
    if v.pt_pitch_slide == 0 {
        if v.pitch_slide == 0 && v.inst_pitch_slide == 0 {
            return;
        }
        if v.pitch_slide_to < 0 {
            return;
        }
        if v.pitch_slide_down {
            v.pitch_slide_note = v.pitch_slide_note.wrapping_sub(v.pitch_slide_speed);
            let d = (i32::from(v.note) + i32::from(v.pitch_slide_note) - i32::from(v.pitch_slide_to)) as i16;
            if d <= 0 {
                v.pitch_slide_note = v.pitch_slide_note.wrapping_sub(d as u16);
                v.pitch_slide_to = -1;
            }
        } else {
            v.pitch_slide_note = v.pitch_slide_note.wrapping_add(v.pitch_slide_speed);
            let d = (i32::from(v.note) + i32::from(v.pitch_slide_note) - i32::from(v.pitch_slide_to)) as i16;
            if d >= 0 {
                v.pitch_slide_note = v.pitch_slide_note.wrapping_sub(d as u16);
                v.pitch_slide_to = -1;
            }
        }
    } else if v.play & FIRST_TICK == 0 && v.pt_pitch_slide_to >= 0 {
        let speed = if v.pt_pitch_slide == PT_PORTAMENTO { v.pt_pitch_slide_speed2 } else { v.pt_pitch_slide_speed };
        if v.pt_pitch_slide_down {
            v.pt_pitch_slide_note = v.pt_pitch_slide_note.wrapping_add(speed);
            let d = (i32::from(period_of(v, v.note)) + i32::from(v.pt_pitch_slide_note) - i32::from(v.pt_pitch_slide_to)) as i16;
            if d >= 0 {
                v.pt_pitch_slide_note = v.pt_pitch_slide_note.wrapping_sub(d as u16);
                v.pt_pitch_slide_to = -1;
            }
        } else {
            v.pt_pitch_slide_note = v.pt_pitch_slide_note.wrapping_sub(speed);
            let d = (i32::from(period_of(v, v.note)) + i32::from(v.pt_pitch_slide_note) - i32::from(v.pt_pitch_slide_to)) as i16;
            if d <= 0 {
                v.pt_pitch_slide_note = v.pt_pitch_slide_note.wrapping_sub(d as u16);
                v.pt_pitch_slide_to = -1;
            }
        }
    }
}

fn slide_arp_note(v: &mut Voice) {
    if v.arp_pitch_slide == 0 || v.arp_pitch_slide_to < 0 {
        return;
    }
    if v.arp_pitch_down {
        v.arp_pitch_slide_note = v.arp_pitch_slide_note.wrapping_sub(v.arp_pitch_slide_speed);
        let d = (i32::from(v.arp_calc_note) + i32::from(v.arp_pitch_slide_note) - i32::from(v.arp_pitch_slide_to)) as i16;
        if d <= 0 {
            v.arp_pitch_slide_note = v.arp_pitch_slide_note.wrapping_sub(d as u16);
            v.arp_pitch_slide_to = -1;
        }
    } else {
        v.arp_pitch_slide_note = v.arp_pitch_slide_note.wrapping_add(v.arp_pitch_slide_speed);
        let d = (i32::from(v.arp_calc_note) + i32::from(v.arp_pitch_slide_note) - i32::from(v.arp_pitch_slide_to)) as i16;
        if d >= 0 {
            v.arp_pitch_slide_note = v.arp_pitch_slide_note.wrapping_sub(d as u16);
            v.arp_pitch_slide_to = -1;
        }
    }
}

fn envelope(v: &mut Voice) {
    if v.fx1 & ENVELOPE == 0 {
        return;
    }
    let scale = |level: u16, v2: u16| ((u32::from(level) * u32::from(v2)) >> 6) as u16;
    // The first stage still running (attack, decay, sustain, release);
    // past the sustain, a held note stays at its level.
    let mut stage = 4;
    for k in 0..4 {
        if k == 3 && v.fx1_arg & ENV_HOLD != 0 {
            v.vol3 = scale(v.env[10], v.vol2).min(1024);
            return;
        }
        if v.env[k] != 0 {
            stage = k;
            break;
        }
    }
    if stage == 4 {
        v.vol3 = scale(v.env[11], v.vol2).min(1024);
        return;
    }
    let level = v.env_volume.wrapping_add(v.env[stage + 4]);
    v.env_volume = level;
    let mut level = level >> 8;
    v.env[stage] -= 1;
    if v.env[stage] == 0 {
        level = v.env[stage + 8];
    }
    v.vol3 = scale(level, v.vol2).min(1024);
}

fn vibrato(v: &mut Voice) {
    if v.vibrato == PT_VIBRATO {
        if v.play & FIRST_TICK != 0 {
            return;
        }
        let (amount, pos) = pt_wobble(v.pt_vib_pos, v.pt_vib_wave, v.pt_vib_cmd, 7);
        v.pt_vib_note = amount as u16;
        v.pt_vib_pos = pos;
        return;
    }
    if v.vibrato == 0 {
        if v.fx1 & VIBRATO == 0 {
            return;
        }
        if v.vib_delay != 0 {
            v.vib_delay -= 1;
            return;
        }
    }
    if v.vib_attack_len == 0 {
        v.vib_depth_now = v.vib_depth;
    } else {
        v.vib_depth_now = v.vib_depth_now.wrapping_add(v.vib_attack_speed);
        v.vib_attack_len -= 1;
        if v.vib_attack_len == 0 {
            v.vib_depth_now = v.vib_depth;
        }
    }
    let mut w = tables::shape(v.vib_wave, usize::from(v.vib_counter >> 2));
    if v.vib_down {
        w = w.wrapping_neg();
    }
    let mut n = ((i32::from(w) * i32::from(v.vib_depth_now >> 8)) >> 4) as i16;
    if n < 0 {
        n += 1;
    }
    v.vib_note = n as u16;
    v.vib_counter = v.vib_counter.wrapping_add(v.vib_speed) & 0x1FF;
}

/// ProTracker's vibrato and tremolo: the amount, and the place moved on.
fn pt_wobble(pos: i8, wave: u8, cmd: u8, shift: u32) -> (i32, i8) {
    let at = ((pos >> 2) & 0x1F) as usize;
    let w: i32 = match wave {
        0 => i32::from(tables::pt_sine(at)),
        1 => {
            let x = (at << 3) as i32;
            if pos < 0 {
                255 - x
            } else {
                x
            }
        }
        _ => 255,
    };
    let mut amount = (i32::from(cmd & 15) * w) >> shift;
    if pos < 0 {
        amount = -amount.abs();
    } else {
        amount = amount.abs();
    }
    (amount, pos.wrapping_add(((cmd >> 2) & 0x3C) as i8))
}

fn tremolo(v: &mut Voice) {
    // The ProTracker tremolo (NostalgicPlayer tests the vibrato here; the
    // original replayer, the tremolo).
    if v.tremolo == PT_TREMOLO {
        if v.play & FIRST_TICK != 0 {
            return;
        }
        let (amount, pos) = pt_wobble(v.pt_trem_pos, v.pt_trem_wave, v.pt_trem_cmd, 2);
        let mut vol = i32::from(v.vol1);
        if vol != 0 {
            vol = (vol + amount).clamp(0, 1024);
        }
        v.vol2 = vol as u16;
        v.vol3 = vol as u16;
        v.pt_trem_pos = pos;
        return;
    }
    if v.tremolo == 0 {
        if v.fx1 & TREMOLO == 0 {
            return;
        }
        if v.trem_delay != 0 {
            v.trem_delay -= 1;
            return;
        }
    }
    if v.trem_attack_len == 0 {
        v.trem_depth_now = v.trem_depth;
    } else {
        v.trem_depth_now = v.trem_depth_now.wrapping_add(v.trem_attack_speed);
        v.trem_attack_len -= 1;
        if v.trem_attack_len == 0 {
            v.trem_depth_now = v.trem_depth;
        }
    }
    let mut w = tables::shape(v.trem_wave, usize::from(v.trem_counter >> 2));
    if v.trem_down {
        w = w.wrapping_neg();
    }
    let mut t = ((i32::from(w) * i32::from(v.trem_depth_now >> 8)) >> 1) as i16;
    if t < 0 {
        t += 16;
    }
    let mut vol = i32::from(v.vol1);
    if v.vol1 != 0 {
        vol = (vol + i32::from(t)).clamp(0, 1024);
    }
    v.vol2 = vol as u16;
    v.vol3 = vol as u16;
    v.trem_counter = v.trem_counter.wrapping_add(v.trem_speed) & 0x1FF;
}

fn move_loop(v: &mut Voice) {
    if v.fx1 & LOOP == 0 {
        return;
    }
    let Some(base) = v.loop_sample else { return };
    let stepping = v.fx2_arg & LOOP_STEP != 0;
    if !stepping {
        v.loop_delay -= 1;
        if v.loop_delay >= 0 {
            let p = base.plus(u32::from(v.loop_counter_save) * 2);
            v.rep_ptr = Some(p);
            v.rep_orig = Some(p);
            return;
        }
        v.loop_delay = 0;
        if v.loop_wait != 0 {
            v.loop_wait_counter -= 1;
            if v.loop_wait_counter >= 0 {
                return;
            }
            v.loop_wait_counter = v.loop_wait as i16;
        }
        v.loop_counter_save = v.loop_counter;
        loop_count(v);
    } else if v.fx2_arg & LOOP_INIT == 0 {
        if v.part_note != 0 {
            v.loop_counter_save = v.loop_counter;
            loop_count(v);
        }
    } else {
        if v.part_note != 0 {
            v.loop_counter_save = v.loop_counter;
        }
        loop_count(v);
    }
    let previous = v.loop_counter_save;
    if stepping {
        v.loop_delay -= 1;
        if v.loop_delay < 0 {
            v.loop_delay = 0;
            let mut go = true;
            if v.loop_wait != 0 {
                v.loop_wait_counter -= 1;
                if v.loop_wait_counter >= 0 {
                    go = false;
                } else {
                    v.loop_wait_counter = v.loop_wait as i16;
                }
            }
            if go {
                let step = v.loop_step.abs();
                let speed = i32::from(v.loop_speed as i8);
                let c = i32::from(v.loop_counter_save) + speed * step;
                v.loop_counter_save = if speed < 0 {
                    c.max(0) as u16
                } else if c > i32::from(v.loop_max) {
                    v.loop_max
                } else {
                    c as u16
                };
            }
        }
    }
    let p = base.plus(u32::from(previous) * 2);
    v.rep_ptr = Some(p);
    v.rep_orig = Some(p);
    if v.loop_turns >= 0 || v.fx1 & LOOP_STOP == 0 {
        return;
    }
    v.fx1 &= !LOOP;
    v.out.silence = true;
}

fn loop_count(v: &mut Voice) {
    if v.loop_turns < 0 {
        return;
    }
    let c = i32::from(v.loop_counter) + v.loop_step;
    let (lo, hi) = if v.loop_repeat < v.loop_repeat_end { (v.loop_repeat, v.loop_repeat_end) } else { (v.loop_repeat_end, v.loop_repeat) };
    let inside = if v.loop_step < 0 { c >= i32::from(lo) } else { c <= i32::from(hi) };
    if inside {
        v.loop_counter = c as u16;
        return;
    }
    if v.loop_turns != 0 {
        v.loop_turns -= 1;
        if v.loop_turns == 0 {
            v.loop_turns = -1;
        }
    }
    v.loop_counter = (c - v.loop_step) as u16;
    v.loop_step = -v.loop_step;
}

#[cfg(test)]
mod tests {
    use super::super::{Line, Part, Tune, Wave};
    use super::*;

    /// A song of one 4-channel tune: channel 0 plays `lines` of part 0 at
    /// `speed`, the others nothing.
    fn song(lines: &[Line], speed: u8, inst: &[u8]) -> Song {
        let mut s = Song::new();
        let mut part = Part::default();
        part.lines[..lines.len()].copy_from_slice(lines);
        s.parts[0] = part;
        // Part 0, transpose 0 (16), then end.
        let mut seq = vec![0x0060u16; 256];
        seq[0] = 0x0010;
        let stop = vec![0x0060u16; 256];
        s.tunes.push(Tune { title: "t".into(), tempo: 125, speed, groove: 0, volume: 64, mixer: false, sequences: vec![seq, stop.clone(), stop.clone(), stop] });
        s.instruments.push(Instrument::new(inst, true));
        let data: Vec<i8> = (0..1000).map(|i| if i % 50 < 25 { 100 } else { -100 }).collect();
        s.samples.push(Wave { name: "s".into(), kind: 0, length: 500, repeat: 0, repeat_length: 0, finetune: 0, semitone: 0, data, stored: 1000 });
        s
    }

    fn inst(volume: u16) -> Vec<u8> {
        let mut b = vec![0u8; 206];
        b[32] = 1;
        b[38..40].copy_from_slice(&500u16.to_be_bytes());
        b[58..60].copy_from_slice(&volume.to_be_bytes());
        b[60] = 1;
        b
    }

    #[test]
    fn a_note_starts_its_sample_at_its_period_and_the_row_waits_its_speed() {
        let line = Line { note: 25, instr: 1, fx: [(0, 0); 5] };
        let s = song(&[line, Line::default(), Line { note: 37, instr: 1, fx: [(0, 0); 5] }], 3, &inst(64));
        let mut m = Machine::new(&s, 0);
        m.tick(&s);
        let out = m.out(0);
        assert!(out.restart);
        assert_eq!((out.play.buf, out.play.len, out.volume), (Buf::Sample(1), 1000, 64));
        // Notes are 32nds of a semitone up the table: note 25 at 800.
        assert_eq!(out.period, tables::period(25 * 32));
        assert_eq!(m.row(0).map(|r| r.line), Some(0));
        // Three ticks a row: the next row (empty) on tick 4, the note on tick 7.
        for t in 2..=7 {
            m.tick(&s);
            assert_eq!(m.row(0).is_some(), t == 4 || t == 7, "tick {t}");
        }
        assert_eq!(m.out(0).period, tables::period(37 * 32));
    }

    #[test]
    fn effects_slide_the_volume_and_a_part_end_moves_the_sequence_on() {
        let line = Line { note: 25, instr: 1, fx: [(INST_VOL_DOWN, 64), (0, 0), (0, 0), (0, 0), (0, 0)] };
        let end = Line { note: 61, ..Default::default() };
        let s = song(&[line, end], 2, &inst(64));
        let mut m = Machine::new(&s, 0);
        m.tick(&s);
        assert_eq!(m.out(0).volume, 64);
        m.tick(&s);
        assert_eq!(m.out(0).volume, 60);
        // Line 1 ends the part: the sequence's next entry ends the channel.
        m.tick(&s);
        assert!(m.stopped(0) && m.reached_end(0));
    }
}
