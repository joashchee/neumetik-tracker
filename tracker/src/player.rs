//! Plays a module: the song read a row at a time, each row's effects
//! worked tick by tick the way the module's tracker did (`Kind`), and
//! every channel's sample mixed with its loops, envelopes and fades.
//!
//! - Pitch is a period: the Amiga's (four times finer, as Scream
//!   Tracker and FastTracker kept it) or a linear one in 64ths of a
//!   semitone. A slide's step is the same in both: `param * 4`, fine
//!   slides `x * 4`, extra-fine `x`.
//! - Notes a channel leaves still sounding (IT's new note actions, and
//!   every cut, faded over a few milliseconds so it never clicks) move
//!   to background voices.
//! - `render` adds into the buffers it's given and never allocates once
//!   the player is made, so it can run in an audio callback.

use std::sync::Arc;

use neumetik_synth::opl::FmVoice;
use neumetik_synth::puma::PumaState;
use neumetik_synth::musicline::{Machine, Region};
use neumetik_synth::ptk::Machine as PtkMachine;
use neumetik_synth::replay::Machine as TuneMachine;
use neumetik_synth::sel::{self, RowEvents, Sel};
use neumetik_synth::sequence::{SoundState, Step};

use crate::dsp::{Rack, BLOCK};
use crate::model::*;

/// Middle C's frequency: the note an FM instrument plays at its C5 rate.
const MIDDLE_C: f64 = 261.625_565;
/// An FM note's level beside a sample's: a full-scale operator would
/// drown a mix of full-scale samples, and the chip's nine channels sat
/// under a Sound Blaster's PCM.
const FM_LEVEL: f32 = 0.4;

/// How a sample's points are read between them: the trackers' own
/// choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Interp {
    /// The nearest point, as the Amiga's Paula and the first trackers.
    None,
    /// A straight line between two points (FastTracker 2's).
    Linear,
    /// A curve through four (Impulse Tracker's cubic, Catmull-Rom).
    #[default]
    Cubic,
    /// A windowed sinc through eight (OpenMPT's).
    Sinc,
}

impl Interp {
    pub fn from_u8(v: u8) -> Interp {
        match v {
            0 => Interp::None,
            1 => Interp::Linear,
            3 => Interp::Sinc,
            _ => Interp::Cubic,
        }
    }
}

/// A MIDI message a song sends (an instrument's notes, a macro), up to
/// three bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MidiEvent {
    pub bytes: [u8; 3],
    pub len: u8,
}

/// MIDI messages kept between `take_midi`s; more are dropped.
const MIDI_QUEUE: usize = 256;
/// Phases of the sinc table, and its taps.
const SINC_PHASES: usize = 256;
const SINC_TAPS: usize = 8;

/// A phrase playing on a channel: the instrument's, where it is, and
/// how far it's transposed.
#[derive(Clone, Copy, Default)]
struct PhraseAt {
    instr: u8,
    row: usize,
    transpose: i16,
}

/// The Amiga's period of middle C, four times finer, times its rate.
const AMIGA_CLOCK: f64 = 1712.0 * 8363.0;
/// A linear period's middle C.
const LINEAR_C5: f64 = 3840.0;
/// Background voices a module may leave sounding.
const BACKGROUND: usize = 96;
/// Seconds a volume change is ramped over, so nothing clicks.
const RAMP_SECONDS: f32 = 0.0015;

/// A sine through 64 steps, -64 to 64.
fn sine(pos: u8) -> i32 {
    const T: [i8; 64] = [
        0, 6, 12, 19, 24, 30, 36, 41, 45, 49, 53, 56, 59, 61, 63, 64, 64, 64, 63, 61, 59, 56, 53, 49, 45, 41, 36, 30, 24, 19, 12, 6, 0, -6, -12, -19, -24, -30, -36, -41, -45, -49, -53, -56, -59, -61, -63, -64,
        -64, -64, -63, -61, -59, -56, -53, -49, -45, -41, -36, -30, -24, -19, -12, -6,
    ];
    i32::from(T[usize::from(pos & 63)])
}

/// A waveform's value at `pos` (64 steps), -64 to 64: 0 sine, 1 ramp
/// down, 2 square, 3 random, 4 ramp up.
fn wave(kind: u8, pos: u8, random: &mut u32) -> i32 {
    let p = i32::from(pos & 63);
    match kind & 7 {
        1 => 64 - p * 2,
        2 => {
            if p < 32 {
                64
            } else {
                -64
            }
        }
        3 => {
            *random = random.wrapping_mul(1_103_515_245).wrapping_add(12345);
            ((*random >> 16) % 129) as i32 - 64
        }
        4 => p * 2 - 64,
        _ => sine(pos),
    }
}

/// An envelope's place and value as a note plays.
#[derive(Clone, Copy, Default)]
struct EnvState {
    tick: u32,
    on: bool,
    ended: bool,
    /// Gone to the release node, and what's added from there.
    released: bool,
    offset: i32,
}

/// An envelope's value at a tick.
fn env_value(pts: &[EnvPoint], t: u32) -> i32 {
    match pts.iter().position(|p| u32::from(p.tick) > t) {
        None => i32::from(pts[pts.len() - 1].value),
        Some(0) => i32::from(pts[0].value),
        Some(i) => {
            let (a, b) = (pts[i - 1], pts[i]);
            let span = (i32::from(b.tick) - i32::from(a.tick)).max(1);
            i32::from(a.value) + (i32::from(b.value) - i32::from(a.value)) * (t as i32 - i32::from(a.tick)) / span
        }
    }
}

impl EnvState {
    fn reset(&mut self, env: &Envelope) {
        self.tick = 0;
        self.on = env.enabled;
        self.ended = false;
        self.released = false;
        self.offset = 0;
    }

    /// The value now (volume 0 to 64, the others -32 to 32), then a tick on.
    fn step(&mut self, env: &Envelope, released: bool, it: bool) -> Option<i32> {
        if !self.on || env.points.is_empty() {
            return None;
        }
        let pts = &env.points;
        // Let go: on from the release node, from where it was.
        if released && !self.released {
            self.released = true;
            if let Some(node) = env.release.and_then(|n| pts.get(usize::from(n))) {
                let now = env_value(pts, self.tick);
                self.tick = u32::from(node.tick);
                self.offset = now - i32::from(node.value);
            }
        }
        let t = self.tick;
        let value = env_value(pts, t) + self.offset;
        // A tick on, through the sustain while held, then the loop.
        let mut next = t + 1;
        let point = |i: u8| u32::from(pts.get(usize::from(i)).map_or(0, |p| p.tick));
        if let Some((s, e)) = env.sustain.filter(|_| !released) {
            if next > point(e) || (it && t == point(e) && s == e) {
                next = point(s);
            }
        } else if let Some((s, e)) = env.looped {
            if next > point(e) {
                next = point(s);
            }
        }
        if next > u32::from(pts[pts.len() - 1].tick) && env.looped.is_none() && (released || env.sustain.is_none()) {
            self.ended = true;
            next = u32::from(pts[pts.len() - 1].tick);
        }
        self.tick = next;
        Some(value)
    }
}

/// A sample sounding: a channel's own, or one left in the background.
#[derive(Clone, Default)]
struct Voice {
    active: bool,
    wave: Arc<Wave>,
    sample: usize,
    instrument: Option<usize>,
    /// The channel it belongs to, and the note it plays.
    channel: usize,
    note: u8,
    pos: f64,
    backwards: bool,
    looped: Loop,
    sustain: Loop,
    /// Frames a frame of output moves on.
    step: f64,
    released: bool,
    fading: bool,
    /// Of 65,536.
    fade: i32,
    vol_env: EnvState,
    pan_env: EnvState,
    pitch_env: EnvState,
    auto_pos: u8,
    /// Auto-vibrato's depth so far (it sweeps in), of 256ths.
    auto_level: i32,
    /// The gains the channel gives it, before its envelope and fade (a
    /// background voice keeps the last it had).
    base: (f32, f32),
    /// The gains now, and where they're ramping to.
    gain: (f32, f32),
    target: (f32, f32),
    ramp: u32,
    /// Fading out to silence over the ramp, then done (a cut, declicked).
    dying: bool,
    /// Played to its sample's end: silent, but still the channel's note
    /// (as it would be playing on through silence), until another comes.
    ended: bool,
    /// The resonant filter (IT): its coefficients and memory, and
    /// whether it's the high-pass (what the low-pass takes away).
    filter: Option<(f32, f32, f32)>,
    fmem: [(f32, f32); 2],
    highpass: bool,
    /// The effect slot it plays into, or the mix.
    route: Option<u8>,
    /// ProTracker's instrument swap: the sample it goes on with at its
    /// loop's end.
    swap: Option<usize>,
    /// An FM instrument sounding in place of a waveform, and the hertz
    /// of its note for each frame `step` moves on (its C5 rate's).
    fm: Option<FmVoice>,
    fm_hz: f64,
    /// A synth sound's sequences, and where they are this tick.
    program: Option<(Arc<Program>, ProgramState)>,
    program_step: Step,
    /// A Musicline channel's wave: the frames its note starts with
    /// before its loop, and where its loop and start come from (redrawn
    /// each tick when they're the replayer's own changing buffers).
    ml_play: usize,
    ml_loop: Option<Region>,
    ml_start: Option<Region>,
    /// The loop the Amiga goes on to when this pass of the loop ends
    /// (its pointer and length set while it plays), when one is waiting.
    ml_next: Vec<i16>,
    ml_next_set: bool,
    ml_next_region: Option<Region>,
}

impl Voice {
    /// The point at frame `i` on its channel `c`: inside the loop when
    /// it's past the loop's end (reflected, for a ping-pong loop), and
    /// silence past the sample's end. Nothing past a loop's end is ever
    /// read, so cutting it off (as the repository does) changes nothing.
    #[inline]
    fn point(&self, i: usize, c: usize, frames: usize) -> f32 {
        let ch = usize::from(self.wave.channels.max(1));
        let lp = self.live_loop();
        let i = if lp.on() && i >= lp.end as usize {
            let (s, e) = (lp.start as usize, lp.end as usize);
            let len = e - s;
            if lp.kind == LoopKind::PingPong {
                let k = (i - e) % (2 * len);
                if k < len {
                    e - 1 - k
                } else {
                    s + (k - len)
                }
            } else {
                s + (i - e) % len
            }
        } else if i >= frames {
            return 0.0;
        } else {
            i
        };
        f32::from(self.wave.pcm[i * ch + c.min(ch - 1)]) * (1.0 / 32768.0)
    }

    /// The loop playing: the sustain loop while held, then the loop.
    fn live_loop(&self) -> Loop {
        if !self.released && self.sustain.on() {
            self.sustain
        } else {
            self.looped
        }
    }
}

/// What a channel remembers from row to row.
#[derive(Clone, Default)]
struct Chan {
    note: u8,
    /// The 1-based instrument (or sample) last named.
    instr: u8,
    sample: Option<usize>,
    c5: f64,
    period: f64,
    target: f64,
    volume: i32,
    chan_volume: i32,
    pan: i32,
    surround: bool,
    muted: bool,
    // Effect memory.
    porta_up: u8,
    porta_down: u8,
    porta_speed: u8,
    fine_up: u8,
    fine_down: u8,
    xfine: u8,
    vol_slide: u8,
    fine_vol_up: u8,
    fine_vol_down: u8,
    chan_vol_slide: u8,
    global_slide: u8,
    pan_slide: u8,
    vib_speed: u8,
    vib_depth: u8,
    vib_pos: u8,
    vib_wave: u8,
    vib_fine: bool,
    trem_speed: u8,
    trem_depth: u8,
    trem_pos: u8,
    trem_wave: u8,
    pbr_speed: u8,
    pbr_depth: u8,
    pbr_pos: u8,
    pbr_wave: u8,
    arpeggio: u8,
    offset: u8,
    high_offset: u32,
    retrig: u8,
    retrig_count: u8,
    tremor: u8,
    tremor_count: u8,
    tremor_on: bool,
    s3m_last: u8,
    tempo_slide: u8,
    // This row.
    cell: Cell,
    delayed: Option<Cell>,
    delay: u8,
    cut_at: Option<u8>,
    off_at: Option<u8>,
    vib_now: i32,
    trem_now: i32,
    pbr_now: i32,
    arp_now: u8,
    vol_col_slide: Option<(Vol, u8)>,
    loop_row: usize,
    loop_count: u8,
    nna: Option<Nna>,
    vol_env_on: Option<bool>,
    pan_env_on: Option<bool>,
    pitch_env_on: Option<bool>,
    note_slide: Option<(i8, u8, u8)>,
    cutoff: Option<u8>,
    resonance: Option<u8>,
    /// The filter's high-pass mode (a macro's F0F002).
    highpass: bool,
    /// The macro Z00 to Z7F send (SFx).
    sfx: u8,
    /// A smooth macro (\xx) gliding a value: what it sets, from and to.
    smooth: Option<(u8, f32, f32)>,
    /// A phrase playing, from an instrument's note.
    phrase: Option<PhraseAt>,
    /// The matrix slot this row is in doesn't play.
    slot_muted: bool,
    /// The MIDI note it has sounding: (MIDI channel, note).
    midi_note: Option<(u8, u8)>,
    /// The voice this channel plays (index into `voices`).
    voice: usize,
    /// JamCracker's phase, and the wave each AM instrument's copy for
    /// this channel was last mixed to (by sample; 0 none yet, else the
    /// wave and one).
    am_phase: neumetik_synth::jam::Phase,
    am_waves: Vec<u8>,
}

/// Where the song is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Position {
    pub order: usize,
    pub pattern: usize,
    pub row: usize,
    /// How many times the song has come round to where it had been.
    pub looped: u32,
}

pub struct Player {
    module: Arc<Module>,
    rate: u32,
    chans: Vec<Chan>,
    voices: Vec<Voice>,
    order: usize,
    row: usize,
    tick: u32,
    speed: u32,
    tempo: u32,
    global_volume: i32,
    /// Output frames left in this tick.
    left: f64,
    pattern_delay: u32,
    fine_delay: u32,
    /// Where this row sends the song: an order (B), a row in the next
    /// (C), a row back in this pattern (a pattern loop).
    jump: Option<usize>,
    brk: Option<usize>,
    loop_to: Option<usize>,
    visited: Vec<Vec<u64>>,
    looped: u32,
    ended: bool,
    /// Play the song round again at its end (or stop).
    pub repeat: bool,
    /// Stay in one pattern (the editor's Play Pattern).
    pub hold_pattern: bool,
    /// How wide the stereo is, 0 to 1, over the module's own.
    pub separation: f32,
    /// The mix's level.
    pub gain: f32,
    /// How samples are read between their points.
    pub interp: Interp,
    /// The Amiga 500's filters over a MOD: its fixed low-pass, and the
    /// LED filter E00 and E01 turn on and off.
    pub amiga: bool,
    led: bool,
    amiga_mem: [[f32; 3]; 2],
    ramp_len: u32,
    random: u32,
    playing: bool,
    /// The effect slots, ready.
    rack: Option<Box<Rack>>,
    /// The song's own mix, before it's added to what it's given.
    mix_l: Vec<f32>,
    mix_r: Vec<f32>,
    /// Each channel's fader as it plays (the info screen's): a gain and
    /// a panning (0 to 1) over the song's.
    faders: Vec<(f32, Option<f32>)>,
    /// Face The Music's SEL scripts running, a track a channel.
    sel: Sel,
    /// A Musicline song's replayer, and the song and tune it plays.
    ml: Option<Machine>,
    ml_key: (usize, usize),
    /// A ProTrekkr song's replayer (it makes the song's whole sound), the
    /// song it plays, and whether it's fading out after a stop.
    ptk: Option<Box<PtkMachine>>,
    ptk_key: usize,
    ptk_fading: bool,
    /// A synth or chip song's replayer (AHX, HivelyTracker...: `Module::tune`),
    /// the song and subsong it plays, and whether it's fading after a stop.
    tune: Option<Box<TuneMachine>>,
    tune_key: (usize, usize),
    tune_fading: bool,
    /// Rows left to play before it stops (Play Row), when counting.
    rows_left: Option<u32>,
    midi: Vec<MidiEvent>,
    midi_programs: [Option<(u8, Option<u16>)>; 16],
    sinc: Vec<f32>,
}

impl Player {
    pub fn new(module: Arc<Module>, rate: u32) -> Player {
        let rate = rate.max(8000);
        let nch = module.channels.len();
        let mut p = Player {
            chans: Vec::new(),
            voices: Vec::new(),
            order: 0,
            row: 0,
            tick: 0,
            speed: 6,
            tempo: 125,
            global_volume: 128,
            left: 0.0,
            pattern_delay: 0,
            fine_delay: 0,
            jump: None,
            brk: None,
            loop_to: None,
            visited: Vec::new(),
            looped: 0,
            ended: false,
            repeat: true,
            hold_pattern: false,
            separation: 1.0,
            gain: 1.0,
            interp: Interp::Cubic,
            amiga: false,
            led: false,
            amiga_mem: [[0.0; 3]; 2],
            ramp_len: ((rate as f32 * RAMP_SECONDS) as u32).max(8),
            random: 0x1234_5678,
            playing: true,
            rack: (!module.effects.is_empty()).then(|| Box::new(Rack::new(&module.effects, rate))),
            mix_l: vec![0.0; BLOCK],
            mix_r: vec![0.0; BLOCK],
            faders: vec![(1.0, None); nch + 1],
            sel: Sel::new(65),
            ml: None,
            ml_key: (0, 0),
            ptk: None,
            ptk_key: 0,
            ptk_fading: false,
            tune: None,
            tune_key: (0, 0),
            tune_fading: false,
            rows_left: None,
            midi: Vec::with_capacity(MIDI_QUEUE),
            midi_programs: [None; 16],
            sinc: sinc_table(),
            rate,
            module,
        };
        p.chans = vec![Chan::default(); nch + 1];
        p.voices = vec![Voice::default(); nch + 1 + BACKGROUND];
        p.reset();
        p
    }

    /// The effect slots a module needs, made ready (to hand to
    /// `set_module_with` from outside the audio thread, since it allocates).
    pub fn rack_for(module: &Module, rate: u32) -> Option<Box<Rack>> {
        (!module.effects.is_empty()).then(|| Box::new(Rack::new(&module.effects, rate.max(8000))))
    }

    /// Whether the slots it has fit a module's (else a new rack's needed).
    pub fn rack_fits(&self, module: &Module) -> bool {
        match &self.rack {
            Some(r) => r.fits(&module.effects),
            None => module.effects.is_empty(),
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// A player that isn't playing the song: only notes played on it
    /// (`jam`) sound, until `play_from`.
    pub fn idle(module: Arc<Module>, rate: u32) -> Player {
        let mut p = Player::new(module, rate);
        p.playing = false;
        p
    }

    /// Plays the song from `order`, `row` (in `hold_pattern`, that
    /// order's pattern over and over).
    pub fn play_from(&mut self, order: usize, row: usize, hold_pattern: bool) {
        let m = self.module.clone();
        // Speed, tempo and global volume as the song sets them; a play
        // from the middle doesn't know what came before.
        self.speed = u32::from(m.speed.max(1));
        self.tempo = u32::from(m.tempo.max(32));
        self.global_volume = i32::from(m.global_volume);
        self.hold_pattern = hold_pattern;
        self.looped = 0;
        self.seek(order, row);
    }

    /// The song stops, its notes fading; jammed notes go on.
    pub fn stop(&mut self) {
        self.playing = false;
        self.ended = false;
        self.rows_left = None;
        self.ptk_fade();
        self.tune_fade();
        let jam = self.chans.len() - 1;
        for (i, v) in self.voices.iter_mut().enumerate() {
            if v.active && i != jam && v.channel != jam {
                v.dying = true;
                v.target = (0.0, 0.0);
                v.ramp = v.ramp.max(1);
            }
        }
        for ch in 0..jam {
            self.midi_off(ch);
        }
    }

    /// Plays one row from `order`, `row`, its notes let go after it
    /// (Impulse Tracker's 8 key).
    pub fn play_row(&mut self, order: usize, row: usize) {
        self.silence();
        self.play_from(order, row, true);
        self.rows_left = Some(1);
    }

    /// A channel's fader as the song plays: its gain (1 as the song has
    /// it) and its panning (0 to 1, or the song's).
    pub fn set_fader(&mut self, channel: usize, gain: f32, pan: Option<f32>) {
        if let Some(f) = self.faders.get_mut(channel) {
            *f = (gain.clamp(0.0, 4.0), pan.map(|p| p.clamp(0.0, 1.0)));
        }
        if let (Some(t), Some(p)) = (self.ptk_track_of(channel), self.ptk.as_mut()) {
            p.set_gain(t, gain.clamp(0.0, 4.0));
        }
        if let Some(t) = self.tune.as_mut() {
            t.set_gain(channel, gain.clamp(0.0, 4.0));
        }
    }

    /// The MIDI messages sent since the last time, each handed to `send`.
    pub fn take_midi(&mut self, send: &mut dyn FnMut(&[u8])) {
        for e in &self.midi {
            send(&e.bytes[..usize::from(e.len)]);
        }
        self.midi.clear();
    }

    fn send_midi(&mut self, bytes: &[u8]) {
        if self.midi.len() < MIDI_QUEUE && !bytes.is_empty() {
            let mut b = [0u8; 3];
            let n = bytes.len().min(3);
            b[..n].copy_from_slice(&bytes[..n]);
            self.midi.push(MidiEvent { bytes: b, len: n as u8 });
        }
    }

    /// A channel's MIDI note, if it has one sounding, let go.
    fn midi_off(&mut self, ch: usize) {
        if let Some((mc, note)) = self.chans.get_mut(ch).and_then(|c| c.midi_note.take()) {
            self.send_midi(&[0x80 | mc, note, 0]);
        }
    }

    /// Every channel's notes let go (as at a row's end, with Play Row).
    fn release_all(&mut self) {
        for ch in 0..self.chans.len() - 1 {
            self.key_off(ch);
            self.midi_off(ch);
        }
    }

    /// The song is playing.
    pub fn playing(&self) -> bool {
        self.playing
    }

    /// The note played with `jam` is let go.
    pub fn jam_off(&mut self) {
        let ch = self.chans.len() - 1;
        self.key_off(ch);
        if !self.module.flags.instruments {
            self.cut(ch);
        }
    }

    pub fn module(&self) -> &Arc<Module> {
        &self.module
    }

    /// A changed module (an edit), played on from where it is; notes
    /// sounding go on.
    pub fn set_module(&mut self, module: Arc<Module>) {
        self.set_module_with(module, None);
    }

    /// `set_module`, with its effect slots made ready elsewhere
    /// (`rack_for`) when they've changed, so nothing's allocated here.
    pub fn set_module_with(&mut self, module: Arc<Module>, rack: Option<Option<Box<Rack>>>) {
        let nch = module.channels.len();
        let old = self.chans.len().saturating_sub(1);
        match rack {
            Some(r) => self.rack = r,
            None if self.rack_fits(&module) => {
                if let Some(r) = &mut self.rack {
                    r.update(&module.effects);
                }
            }
            None => self.rack = Player::rack_for(&module, self.rate),
        }
        self.module = module;
        if self.ml_key != Player::ml_key_of(&self.module) {
            self.ml_restart();
        }
        if self.ptk_key != Player::ptk_key_of(&self.module) {
            self.ptk_restart();
            if let Some(p) = self.ptk.as_mut() {
                p.seek(self.order, self.row);
            }
        }
        if self.tune_key != Player::tune_key_of(&self.module) {
            self.tune_restart();
            if let Some(t) = self.tune.as_mut() {
                t.seek(self.order, self.row);
            }
        }
        if nch != old {
            self.chans.resize(nch + 1, Chan::default());
            self.voices.resize(nch + 1 + BACKGROUND, Voice::default());
            self.faders.resize(nch + 1, (1.0, None));
            for (i, c) in self.chans.iter_mut().enumerate() {
                c.voice = i;
            }
        }
        for (i, c) in self.module.channels.iter().enumerate() {
            self.chans[i].muted = c.muted;
        }
        self.ptk_mutes();
        self.tune_mutes();
        self.mark_visited_reset();
        let order = self.order.min(self.module.orders.len().saturating_sub(1));
        self.order = order;
        if let Some(p) = self.current_pattern() {
            if self.row >= p.rows {
                self.row = 0;
            }
        }
    }

    /// Back to the start, every channel as the module sets it.
    pub fn reset(&mut self) {
        let m = self.module.clone();
        for (i, c) in self.chans.iter_mut().enumerate() {
            let def = m.channels.get(i).cloned().unwrap_or_default();
            *c = Chan { pan: i32::from(def.pan), chan_volume: i32::from(def.volume), surround: def.surround, muted: def.muted, c5: 8363.0, voice: i, ..Default::default() };
        }
        for v in &mut self.voices {
            v.active = false;
        }
        self.sel.reset();
        self.ml_restart();
        self.ptk_restart();
        self.tune_restart();
        self.speed = u32::from(m.speed.max(1));
        self.tempo = u32::from(m.tempo.max(32));
        self.global_volume = i32::from(m.global_volume);
        self.order = 0;
        self.row = 0;
        self.tick = 0;
        self.left = 0.0;
        self.pattern_delay = 0;
        self.fine_delay = 0;
        self.jump = None;
        self.brk = None;
        self.loop_to = None;
        self.looped = 0;
        self.ended = false;
        self.playing = true;
        self.led = false;
        self.rows_left = None;
        if let Some(r) = self.rack.as_deref_mut() {
            r.clear();
        }
        self.mark_visited_reset();
        self.skip_to_playable();
    }

    fn mark_visited_reset(&mut self) {
        let m = &self.module;
        let rows: Vec<usize> = m.orders.iter().map(|&o| m.patterns.get(usize::from(o)).map_or(1, |p| p.rows)).collect();
        if self.visited.len() != rows.len() {
            self.visited = rows.iter().map(|&r| vec![0u64; r.div_ceil(64)]).collect();
        } else {
            for (v, &r) in self.visited.iter_mut().zip(&rows) {
                v.clear();
                v.resize(r.div_ceil(64), 0);
            }
        }
    }

    /// Plays from `order`, `row` on.
    pub fn seek(&mut self, order: usize, row: usize) {
        self.order = order.min(self.module.orders.len().saturating_sub(1));
        self.row = row;
        self.tick = 0;
        self.left = 0.0;
        self.pattern_delay = 0;
        self.jump = None;
        self.brk = None;
        self.loop_to = None;
        self.ended = false;
        self.playing = true;
        self.mark_visited_reset();
        self.skip_to_playable();
        if let Some(p) = self.current_pattern() {
            if self.row >= p.rows {
                self.row = 0;
            }
        }
        for c in &mut self.chans {
            c.loop_count = 0;
            c.loop_row = 0;
        }
        self.ml_seek();
        if let Some(p) = self.ptk.as_mut() {
            p.seek(self.order, self.row);
            self.ptk_fading = false;
        }
        if let Some(t) = self.tune.as_mut() {
            t.seek(self.order, self.row);
            self.tune_fading = false;
        }
    }

    /// Every note stops (faded in a moment).
    pub fn silence(&mut self) {
        self.ptk_fade();
        self.tune_fade();
        for v in &mut self.voices {
            if v.active {
                v.dying = true;
                v.target = (0.0, 0.0);
                v.ramp = v.ramp.max(1);
            }
        }
    }

    pub fn position(&self) -> Position {
        Position { order: self.order, pattern: self.module.orders.get(self.order).map_or(0, |&o| usize::from(o)), row: self.row, looped: self.looped }
    }

    pub fn tempo(&self) -> (u32, u32) {
        if let Some(t) = &self.tune {
            return t.speed();
        }
        if let Some(p) = &self.ptk {
            return (self.speed, p.tempo().0.max(1) as u32);
        }
        (self.speed, self.tempo)
    }

    /// The song has played to its end (only when not repeating).
    pub fn ended(&self) -> bool {
        self.ended
    }

    pub fn set_muted(&mut self, channel: usize, muted: bool) {
        if let Some(c) = self.chans.get_mut(channel) {
            c.muted = muted;
        }
        self.ptk_mutes();
        self.tune_mutes();
    }

    /// Notes sounding now, for meters: each channel's volume, 0 to 1.
    pub fn levels(&self, out: &mut [f32]) {
        if let Some(t) = &self.tune {
            out.fill(0.0);
            t.levels(out);
            return;
        }
        if let Some(p) = &self.ptk {
            let mut tracks = [0.0f32; neumetik_synth::ptk::TRACKS];
            p.levels(&mut tracks);
            for (i, o) in out.iter_mut().enumerate() {
                *o = self.ptk_track_of(i).map_or(0.0, |t| tracks[t]);
            }
            return;
        }
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.voices.get(self.chans.get(i).map_or(usize::MAX, |c| c.voice)).filter(|v| v.active && !v.dying).map_or(0.0, |v| (v.target.0 + v.target.1).min(1.0));
        }
    }

    /// The editor's keyboard: a note played on its own channel (past the
    /// song's), with the instrument (or sample) given; `NOTE_OFF` lets
    /// it go.
    pub fn jam(&mut self, note: u8, instr: u8) {
        let ch = self.chans.len() - 1;
        let cell = Cell { note, instr, ..Default::default() };
        self.chans[ch].cell = cell;
        self.chans[ch].volume = 64;
        self.chans[ch].chan_volume = 64;
        self.chans[ch].pan = 128;
        self.start_row_cell(ch, cell);
        self.update_voice(ch);
    }

    fn current_pattern(&self) -> Option<&Pattern> {
        let o = *self.module.orders.get(self.order)?;
        self.module.patterns.get(usize::from(o)).map(|p| p.as_ref())
    }

    /// Moves past skips and orders that name no pattern; at the end, back
    /// to the restart (or stop).
    fn skip_to_playable(&mut self) {
        let n = self.module.orders.len();
        for _ in 0..=n {
            match self.module.orders.get(self.order) {
                Some(&o) if o < ORDER_SKIP && usize::from(o) < self.module.patterns.len() => return,
                Some(&o) if o == ORDER_SKIP => self.order += 1,
                Some(_) | None => {
                    self.song_end();
                    self.order = usize::from(self.module.restart).min(n.saturating_sub(1));
                    self.row = 0;
                    if !self.playing {
                        return;
                    }
                }
            }
        }
    }

    fn song_end(&mut self) {
        self.looped += 1;
        // A Musicline song that stopped starts again; one that comes round plays on.
        if self.ml.as_ref().is_some_and(|m| !m.playing() || (0..m.channels()).all(|c| m.stopped(c))) {
            self.ml_restart();
        }
        if !self.repeat {
            self.ended = true;
            self.playing = false;
        }
        self.mark_visited_reset();
    }

    /// A tick's length in seconds, as the tempo mode has it, swung.
    fn tick_seconds(&self) -> f64 {
        // A Musicline song: as its replayer times it.
        if let Some(ml) = &self.ml {
            return ml.tick_seconds();
        }
        // A ProTrekkr song keeps its own time, as does a synth or chip
        // song; ticks only move the jammed notes on.
        if self.ptk.is_some() || self.tune.is_some() {
            return 0.02;
        }
        let m = &self.module;
        let tempo = f64::from(self.tempo.max(1));
        let pattern = m.orders.get(self.order).map_or(0, |&o| usize::from(o));
        let time = m.time_of(pattern);
        let base = if m.flags.vblank {
            0.02
        } else {
            match m.tempo_mode {
                TempoMode::Classic => 2.5 / tempo,
                TempoMode::Alternative => 1.0 / tempo,
                TempoMode::Modern => 60.0 / (tempo * f64::from(time.rows_per_beat.max(1)) * f64::from(self.speed.max(1))),
            }
        };
        // Swing: a pair of rows' first held longer, its second shorter, the pair as long.
        let swing = m.patterns.get(pattern).and_then(|p| p.swing).unwrap_or(m.swing).min(100);
        if swing > 0 && time.rows_per_beat >= 2 {
            let s = f64::from(swing) / 200.0;
            base * if self.row.is_multiple_of(2) { 1.0 + s } else { 1.0 - s }
        } else {
            base
        }
    }

    fn samples_per_tick(&self) -> f64 {
        f64::from(self.rate) * self.tick_seconds()
    }

    /// Adds the next `left.len()` frames into `left` and `right`.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        let mut at = 0;
        while at < frames {
            if self.left <= 0.0 {
                self.process_tick();
                self.left += self.samples_per_tick();
            }
            let n = (self.left.ceil() as usize).clamp(1, (frames - at).min(BLOCK));
            self.mix_block(n);
            for i in 0..n {
                left[at + i] += self.mix_l[i];
                right[at + i] += self.mix_r[i];
            }
            self.left -= n as f64;
            at += n;
        }
    }

    /// The song's next `n` frames (at most `BLOCK`) into its own mix:
    /// the voices, through the effect slots they go to, then the Amiga's
    /// filters when they're on.
    fn mix_block(&mut self, n: usize) {
        let mut l = std::mem::take(&mut self.mix_l);
        let mut r = std::mem::take(&mut self.mix_r);
        l[..n].fill(0.0);
        r[..n].fill(0.0);
        self.mix(&mut l[..n], &mut r[..n]);
        self.ptk_render(&mut l[..n], &mut r[..n]);
        self.tune_render(&mut l[..n], &mut r[..n]);
        if let Some(rack) = self.rack.as_deref_mut() {
            rack.process(n, &mut l[..n], &mut r[..n]);
        }
        if self.amiga && self.module.kind == Kind::Mod {
            self.amiga_filter(&mut l[..n], &mut r[..n]);
        }
        self.mix_l = l;
        self.mix_r = r;
    }

    /// The Amiga 500's output: an RC low-pass at about 4.9 kHz always,
    /// and the "LED" filter (a two-pole Butterworth at 3.3 kHz) when the
    /// song turns it on.
    fn amiga_filter(&mut self, l: &mut [f32], r: &mut [f32]) {
        let fs = self.rate as f32;
        let a = 1.0 - (-2.0 * std::f32::consts::PI * 4900.0 / fs).exp();
        let led = self.led;
        // The LED filter as a state-variable low-pass, Q of a Butterworth.
        let g = (std::f32::consts::PI * (3300.0 / fs).min(0.49)).tan();
        let k = std::f32::consts::SQRT_2;
        let a1 = 1.0 / (1.0 + g * (g + k));
        for (c, buf) in [l, r].into_iter().enumerate() {
            let m = &mut self.amiga_mem[c];
            for v in buf.iter_mut() {
                m[0] += (*v - m[0]) * a;
                let mut y = m[0];
                if led {
                    let v1 = a1 * m[1] + g * a1 * (y - m[2]);
                    let v2 = m[2] + g * v1;
                    m[1] = 2.0 * v1 - m[1];
                    m[2] = 2.0 * v2 - m[2];
                    y = v2;
                }
                *v = y;
            }
        }
    }

    // ---- The song ----

    fn process_tick(&mut self) {
        if !self.playing || self.ptk.is_some() || self.tune.is_some() {
            for i in 0..self.chans.len() {
                self.update_voice(i);
            }
            return;
        }
        if self.ml.is_some() {
            self.musicline_tick();
            return;
        }
        if self.tick == 0 && self.pattern_delay == 0 {
            self.start_row();
        } else {
            for ch in 0..self.chans.len() - 1 {
                self.tick_effects(ch);
            }
        }
        self.run_sel();
        self.automate();
        // The jam channel's notes run their envelopes too.
        for ch in 0..self.chans.len() {
            self.update_voice(ch);
        }
        self.tick += 1;
        if self.tick >= self.speed + self.fine_delay {
            self.tick = 0;
            self.fine_delay = 0;
            if self.pattern_delay > 0 {
                self.pattern_delay -= 1;
                if self.pattern_delay > 0 {
                    return;
                }
            }
            self.next_row();
        }
    }

    fn ml_key_of(m: &Module) -> (usize, usize) {
        m.musicline.as_ref().map_or((0, 0), |s| (Arc::as_ptr(s) as usize, m.sequence))
    }

    /// The Musicline replayer from the tune's start (none for other songs).
    fn ml_restart(&mut self) {
        self.ml_key = Player::ml_key_of(&self.module);
        self.ml = self.module.musicline.as_ref().map(|s| Machine::new(s, self.module.sequence));
    }

    fn ptk_key_of(m: &Module) -> usize {
        m.ptk.as_ref().map_or(0, |s| Arc::as_ptr(s) as usize)
    }

    /// The ProTrekkr replayer from the song's start (none for other songs).
    fn ptk_restart(&mut self) {
        self.ptk_key = Player::ptk_key_of(&self.module);
        self.ptk = self.module.ptk.as_ref().map(|s| Box::new(PtkMachine::new(s.clone())));
        self.ptk_fading = false;
        self.ptk_mutes();
    }

    /// The ProTrekkr track a channel shows (each track's notes are a
    /// channel each).
    fn ptk_track_of(&self, channel: usize) -> Option<usize> {
        let song = self.module.ptk.as_ref()?;
        let mut first = 0;
        for (t, tr) in song.tracks.iter().enumerate().take(song.song_tracks) {
            let n = usize::from(tr.notes).clamp(1, neumetik_synth::ptk::POLYPHONY);
            if channel < first + n {
                return Some(t);
            }
            first += n;
        }
        None
    }

    /// The tracks muted as their channels are (a track by its first).
    fn ptk_mutes(&mut self) {
        let Some(song) = self.module.ptk.clone() else { return };
        let mut first = 0;
        for (t, tr) in song.tracks.iter().enumerate().take(song.song_tracks) {
            let muted = self.chans.get(first).is_some_and(|c| c.muted);
            if let Some(p) = self.ptk.as_mut() {
                p.set_muted(t, muted);
            }
            first += usize::from(tr.notes).clamp(1, neumetik_synth::ptk::POLYPHONY);
        }
    }

    fn ptk_fade(&mut self) {
        if let Some(p) = self.ptk.as_mut() {
            p.fade_out();
            self.ptk_fading = true;
        }
    }

    /// A ProTrekkr song's next frames added in; where it is now, a row at
    /// a time (Play Row's count, Play Pattern's hold, its coming round).
    fn ptk_render(&mut self, l: &mut [f32], r: &mut [f32]) {
        let rate = self.rate;
        let gain = self.gain;
        let Some(p) = self.ptk.as_mut() else { return };
        if !self.playing && !self.ptk_fading {
            return;
        }
        p.render(rate, gain, l, r);
        if self.ptk_fading {
            if p.faded() {
                self.ptk_fading = false;
            }
            return;
        }
        let (order, row) = p.position();
        let moved = (order, row) != (self.order, self.row);
        let came_round = p.take_done();
        if self.hold_pattern && order != self.order && !came_round {
            p.seek(self.order, 0);
            self.row = 0;
        } else if moved {
            self.order = order;
            self.row = row;
        }
        if moved {
            if let Some(n) = self.rows_left.as_mut() {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    self.rows_left = None;
                    self.playing = false;
                    p.fade_out();
                    self.ptk_fading = true;
                    return;
                }
            }
        }
        if came_round || !p.playing() {
            self.looped += 1;
            if !self.repeat || !p.playing() {
                self.ended = true;
                self.playing = false;
                p.fade_out();
                self.ptk_fading = true;
            }
        }
    }

    fn tune_key_of(m: &Module) -> (usize, usize) {
        m.tune.as_ref().map_or((0, 0), |t| (Arc::as_ptr(&t.0) as *const () as usize, m.sequence))
    }

    /// A synth or chip song's replayer from its start (none for other
    /// songs): the subsong the order list shows.
    fn tune_restart(&mut self) {
        self.tune_key = Player::tune_key_of(&self.module);
        self.tune = self.module.tune.as_ref().map(|t| Box::new(TuneMachine::new(t.0.clone(), self.module.sequence)));
        self.tune_fading = false;
        self.tune_mutes();
    }

    /// The replayer's channels muted as the song's are.
    fn tune_mutes(&mut self) {
        let Some(t) = self.tune.as_mut() else { return };
        for (c, ch) in self.chans.iter().enumerate().take(t.channels()) {
            t.set_muted(c, ch.muted);
        }
    }

    fn tune_fade(&mut self) {
        if let Some(t) = self.tune.as_mut() {
            t.fade_out();
            self.tune_fading = true;
        }
    }

    /// A synth or chip song's next frames added in; where it is, a row at
    /// a time (Play Row's count, Play Pattern's hold, its coming round),
    /// as with ProTrekkr's.
    fn tune_render(&mut self, l: &mut [f32], r: &mut [f32]) {
        let rate = self.rate;
        let gain = self.gain * f32::from(self.module.mix_volume) / 32.0;
        let sep = f32::from(self.module.separation) / 128.0 * self.separation;
        let Some(t) = self.tune.as_mut() else { return };
        if !self.playing && !self.tune_fading {
            return;
        }
        t.render(rate, gain, sep, l, r);
        if self.tune_fading {
            if t.faded() {
                self.tune_fading = false;
            }
            return;
        }
        let (order, row) = t.position();
        let moved = (order, row) != (self.order, self.row);
        let came_round = t.take_done();
        if self.hold_pattern && order != self.order && !came_round {
            t.seek(self.order, 0);
            self.row = 0;
        } else if moved {
            self.order = order;
            self.row = row;
        }
        if moved {
            if let Some(n) = self.rows_left.as_mut() {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    self.rows_left = None;
                    self.playing = false;
                    t.fade_out();
                    self.tune_fading = true;
                    return;
                }
            }
        }
        if came_round || !t.playing() {
            self.looped += 1;
            if !self.repeat || !t.playing() {
                self.ended = true;
                self.playing = false;
                t.fade_out();
                self.tune_fading = true;
            }
        }
    }

    /// The speed a row says (the patterns of a Musicline song say every change).
    fn row_speed(&self, order: usize, row: usize) -> Option<u32> {
        let p = self.module.orders.get(order).and_then(|&o| self.module.patterns.get(usize::from(o)))?;
        (0..p.channels).map(|c| p.cell(row, c)).find(|c| c.fx == Fx::Speed && c.param > 0).map(|c| u32::from(c.param))
    }

    /// Played silently from the start to where the song now is.
    fn ml_seek(&mut self) {
        let Some(song) = self.module.musicline.clone() else { return };
        let mut ticks = 0u64;
        let mut speed = u32::from(self.module.speed.max(1));
        for o in 0..=self.order.min(self.module.orders.len().saturating_sub(1)) {
            let Some(p) = self.module.orders.get(o).and_then(|&n| self.module.patterns.get(usize::from(n))) else { continue };
            let rows = if o == self.order { self.row.min(p.rows) } else { p.rows };
            for r in 0..rows {
                speed = self.row_speed(o, r).unwrap_or(speed);
                ticks += u64::from(speed);
            }
        }
        let mut m = Machine::new(&song, self.module.sequence);
        for _ in 0..ticks {
            m.tick(&song);
        }
        self.tempo = u32::from(m.tempo().max(1));
        self.ml_key = Player::ml_key_of(&self.module);
        self.ml = Some(m);
        self.speed = speed;
    }

    /// A tick of a Musicline song: its replayer runs and the channels
    /// play what it tells them; the rows go by at the patterns' speeds,
    /// for the screen.
    fn musicline_tick(&mut self) {
        if self.tick == 0 {
            let (o, r) = (self.order, self.row);
            if let Some(w) = self.visited.get_mut(o).and_then(|v| v.get_mut(r / 64)) {
                *w |= 1 << (r % 64);
            }
            if let Some(s) = self.row_speed(o, r) {
                self.speed = s;
            }
        }
        self.run_musicline();
        for ch in 0..self.chans.len() {
            self.update_voice(ch);
        }
        self.tick += 1;
        if self.tick >= self.speed {
            self.tick = 0;
            self.next_row();
        }
    }

    fn run_musicline(&mut self) {
        let Some(song) = self.module.musicline.clone() else { return };
        let Some(mut mach) = self.ml.take() else { return };
        mach.tick(&song);
        self.tempo = u32::from(mach.tempo().max(1));
        self.led = mach.filter();
        let n = mach.channels().min(self.chans.len() - 1);
        let nsmp = self.module.samples.len();
        for ch in 0..n {
            let out = mach.out(ch);
            let vi = self.chans[ch].voice;
            let v = &mut self.voices[vi];
            if out.restart {
                // Its start, then its loop (once, when they're the same points).
                let same = out.repeat == Some(out.play);
                let play: &[i8] = if same { &[] } else { mach.bytes(&song, ch, out.play) };
                let rep: &[i8] = out.repeat.map_or(&[], |r| mach.bytes(&song, ch, r));
                let w = Arc::make_mut(&mut v.wave);
                w.bits = 8;
                w.channels = 1;
                w.pcm.clear();
                w.pcm.extend(play.iter().chain(rep).map(|&x| i16::from(x) << 8));
                v.ml_play = play.len();
                v.looped = if rep.is_empty() { Loop::default() } else { Loop::forward(play.len() as u32, (play.len() + rep.len()) as u32) };
                v.ml_loop = out.repeat;
                v.ml_start = (!same && out.play.buf.live()).then_some(out.play);
                v.active = true;
                v.pos = 0.0;
                v.backwards = false;
                v.ended = false;
                v.dying = false;
                v.released = false;
                v.fading = false;
                v.fade = 65536;
                v.sample = usize::from(out.sample).saturating_sub(1).min(nsmp.saturating_sub(1));
                v.instrument = None;
                v.channel = ch;
                v.sustain = Loop::default();
                v.swap = None;
                v.fm = None;
                v.program = None;
                v.filter = None;
                v.route = None;
                self.chans[ch].sample = Some(v.sample);
            } else if let Some(r) = out.repeat.filter(|_| v.active) {
                queue_ml_loop(v, mach.bytes(&song, ch, r), Some(r));
            }
            if out.silence && v.active {
                queue_ml_loop(v, &[0, 0], None);
            }
            if out.restart {
                v.ml_next_set = false;
            }
            // The replayer's own buffers change as they play.
            if let Some(r) = v.ml_loop.filter(|r| r.buf.live()) {
                let b = mach.bytes(&song, ch, r);
                let at = v.ml_play;
                for (d, &x) in Arc::make_mut(&mut v.wave).pcm.iter_mut().skip(at).zip(b) {
                    *d = i16::from(x) << 8;
                }
            }
            if let Some(r) = v.ml_next_region.filter(|r| v.ml_next_set && r.buf.live()) {
                for (d, &x) in v.ml_next.iter_mut().zip(mach.bytes(&song, ch, r)) {
                    *d = i16::from(x) << 8;
                }
            }
            if let Some(r) = v.ml_start {
                let b = mach.bytes(&song, ch, r);
                for (d, &x) in Arc::make_mut(&mut v.wave).pcm.iter_mut().zip(b) {
                    *d = i16::from(x) << 8;
                }
            }
            let c = &mut self.chans[ch];
            // The PAL Amiga's clock, in the player's periods.
            c.period = f64::from(out.period) * AMIGA_CLOCK / 3_546_895.0;
            c.target = c.period;
            c.volume = i32::from(out.volume);
            c.chan_volume = if mach.mixer() { 32 } else { 64 };
            c.vib_now = 0;
            c.trem_now = 0;
            c.arp_now = 0;
        }
        self.ml = Some(mach);
    }

    /// Face The Music's SEL scripts, a tick on: those the row starts
    /// started, then each run against the channels (`SelHost`).
    fn run_sel(&mut self) {
        let Some(prog) = self.module.sel.clone() else { return };
        let n = self.module.channels.len().min(self.chans.len() - 1).min(64);
        let mut rows = [RowEvents::default(); 64];
        for (ch, r) in rows.iter_mut().enumerate().take(n) {
            let c = self.chans[ch].cell;
            if self.tick == 0 && c.fx == Fx::Sel {
                self.sel.start(&prog, ch, c.param);
            }
            let note = (NOTE_MIN..=NOTE_MAX).contains(&c.note);
            // A volume comes with its sample in Face The Music; a fade's steps come alone.
            *r = RowEvents {
                note,
                volume: c.fx == Fx::ChannelVolume && c.instr != 0,
                sample: c.instr != 0,
                release: c.note == NOTE_OFF,
                bend: c.fx == Fx::TonePorta && note,
                fade: c.fx == Fx::ChannelVolume && c.instr == 0,
            };
        }
        let mut sel = std::mem::take(&mut self.sel);
        sel.tick(&prog, &mut SelHost { p: self, n }, &rows[..n]);
        self.sel = sel;
    }

    /// The pattern's automation, where the song is now.
    fn automate(&mut self) {
        let module = self.module.clone();
        let Some(p) = self.current_pattern_index().and_then(|i| module.patterns.get(i)) else { return };
        if p.automation.is_empty() {
            return;
        }
        let at = self.row as f32 + self.tick as f32 / self.speed.max(1) as f32;
        for lane in &p.automation {
            let (Some(t), Some(v)) = (lane.target, lane.at(at)) else { continue };
            match t {
                Target::GlobalVolume => self.global_volume = (v * 128.0).round() as i32,
                Target::Tempo => self.tempo = (32.0 + v * 480.0).round() as u32,
                Target::ChannelVolume(c) => {
                    if let Some(c) = self.chans.get_mut(usize::from(c)).filter(|_| usize::from(c) < module.channels.len()) {
                        c.chan_volume = (v * 64.0).round() as i32;
                    }
                }
                Target::ChannelPan(c) => {
                    if let Some(c) = self.chans.get_mut(usize::from(c)).filter(|_| usize::from(c) < module.channels.len()) {
                        c.pan = (v * 255.0).round() as i32;
                        c.surround = false;
                    }
                }
                Target::Effect { slot, param } => {
                    if let Some(r) = self.rack.as_deref_mut() {
                        r.set_param(usize::from(slot), usize::from(param), v);
                    }
                }
            }
        }
    }

    fn current_pattern_index(&self) -> Option<usize> {
        self.module.orders.get(self.order).map(|&o| usize::from(o)).filter(|&o| o < self.module.patterns.len())
    }

    fn next_row(&mut self) {
        // Play Row: the row's played, its notes let go.
        if let Some(n) = self.rows_left {
            if n <= 1 {
                self.rows_left = None;
                self.playing = false;
                self.release_all();
                return;
            }
            self.rows_left = Some(n - 1);
        }
        let rows = self.current_pattern().map_or(64, |p| p.rows);
        let (jump, brk, loop_to) = (self.jump.take(), self.brk.take(), self.loop_to.take());
        let (order, row) = if let Some(r) = loop_to {
            (self.order, r)
        } else if jump.is_some() || brk.is_some() {
            let o = if self.hold_pattern { self.order } else { jump.unwrap_or(self.order + 1) };
            (o, brk.unwrap_or(0))
        } else if self.row + 1 >= rows {
            (if self.hold_pattern { self.order } else { self.order + 1 }, 0)
        } else {
            (self.order, self.row + 1)
        };
        let looping = self.chans.iter().any(|c| c.loop_count > 0);
        let jumped_back = order < self.order || (order == self.order && row <= self.row);
        self.order = order;
        self.row = row;
        if self.hold_pattern {
            self.order = self.order.min(self.module.orders.len().saturating_sub(1));
        }
        self.skip_to_playable();
        if let Some(p) = self.current_pattern() {
            if self.row >= p.rows {
                self.row = 0;
            }
        }
        // Back where the song had been: it has come round.
        if !looping && !self.hold_pattern {
            let (o, r) = (self.order, self.row);
            let seen = self.visited.get(o).and_then(|v| v.get(r / 64)).is_some_and(|w| w & (1 << (r % 64)) != 0);
            if seen && jumped_back {
                self.song_end();
                if self.playing {
                    self.order = o;
                    self.row = r;
                }
            }
        }
    }

    fn start_row(&mut self) {
        let (o, r) = (self.order, self.row);
        if let Some(w) = self.visited.get_mut(o).and_then(|v| v.get_mut(r / 64)) {
            *w |= 1 << (r % 64);
        }
        let module = self.module.clone();
        let Some(pat) = module.orders.get(o).and_then(|&p| module.patterns.get(usize::from(p))) else { return };
        for ch in 0..self.chans.len() - 1 {
            // A muted matrix slot plays nothing, and its notes are silent.
            let muted = module.slot_muted(o, ch);
            self.chans[ch].slot_muted = muted;
            let mut cell = if muted { Cell::default() } else { pat.cell(r, ch) };
            cell = self.phrase_cell(ch, cell);
            // Scream Tracker 3's effects share one memory: a 0 parameter is the last any of them had.
            if module.kind == Kind::S3m && cell.param == 0 && st3_shared(cell.fx) {
                cell.param = self.chans[ch].s3m_last;
            }
            self.chans[ch].cell = cell;
            self.row_effects_reset(ch);
            // A delayed note waits for its tick.
            let delay = match (cell.fx, cell.param >> 4, module.kind) {
                (Fx::ModEx, 0xD, Kind::Mod | Kind::Xm) => cell.param & 0x0F,
                (Fx::S3mEx, 0xD, Kind::S3m | Kind::It) => cell.param & 0x0F,
                (Fx::DelayCut, _, _) => cell.param >> 4,
                _ => 0,
            };
            if delay > 0 && u32::from(delay) < self.speed {
                self.chans[ch].delayed = Some(cell);
                self.chans[ch].delay = delay;
                self.first_tick_effects(ch, Cell { note: 0, instr: 0, vol: Vol::None, volp: 0, ..cell });
            } else if delay > 0 {
                // Delayed past the row: never played.
                self.first_tick_effects(ch, Cell { note: 0, instr: 0, vol: Vol::None, volp: 0, ..cell });
            } else {
                self.start_row_cell(ch, cell);
            }
        }
    }

    /// A row's cell with the channel's phrase in it: a note on an
    /// instrument with a phrase starts it (its first row played instead,
    /// transposed); a row with no note goes on with it; any other note
    /// (or a note off or cut) ends it.
    fn phrase_cell(&mut self, ch: usize, cell: Cell) -> Cell {
        let m = self.module.clone();
        if !m.flags.instruments {
            self.chans[ch].phrase = None;
            return cell;
        }
        let instr = if cell.instr > 0 { cell.instr } else { self.chans[ch].instr };
        let phrase_of = |i: u8| m.instruments.get(usize::from(i).wrapping_sub(1)).and_then(|x| x.phrase.as_ref()).filter(|p| !p.cells.is_empty());
        if (NOTE_MIN..=NOTE_MAX).contains(&cell.note) {
            match phrase_of(instr) {
                Some(p) => {
                    let at = PhraseAt { instr, row: 0, transpose: i16::from(cell.note) - i16::from(p.base.clamp(NOTE_MIN, NOTE_MAX)) };
                    self.chans[ch].phrase = Some(at);
                }
                None => {
                    self.chans[ch].phrase = None;
                    return cell;
                }
            }
        } else if cell.note != NOTE_NONE {
            self.chans[ch].phrase = None;
            return cell;
        }
        let Some(mut at) = self.chans[ch].phrase else { return cell };
        let Some(p) = phrase_of(at.instr) else {
            self.chans[ch].phrase = None;
            return cell;
        };
        let row = if p.looped { at.row % p.cells.len() } else { at.row };
        let Some(&pc) = p.cells.get(row) else {
            self.chans[ch].phrase = None;
            return cell;
        };
        at.row += 1;
        self.chans[ch].phrase = Some(at);
        let note = if (NOTE_MIN..=NOTE_MAX).contains(&pc.note) { (i16::from(pc.note) + at.transpose).clamp(i16::from(NOTE_MIN), i16::from(NOTE_MAX)) as u8 } else { pc.note };
        Cell {
            note,
            instr: if note != NOTE_NONE && (NOTE_MIN..=NOTE_MAX).contains(&note) { at.instr } else { cell.instr },
            vol: if cell.vol != Vol::None { cell.vol } else { pc.vol },
            volp: if cell.vol != Vol::None { cell.volp } else { pc.volp },
            fx: if cell.fx != Fx::None { cell.fx } else { pc.fx },
            param: if cell.fx != Fx::None { cell.param } else { pc.param },
        }
    }

    fn row_effects_reset(&mut self, ch: usize) {
        let c = &mut self.chans[ch];
        c.delayed = None;
        c.cut_at = None;
        c.off_at = None;
        c.vib_now = 0;
        c.trem_now = 0;
        c.arp_now = 0;
        c.vol_col_slide = None;
        c.note_slide = None;
        c.smooth = None;
    }

    /// A cell's note, instrument and volume, then its effect's first tick.
    fn start_row_cell(&mut self, ch: usize, cell: Cell) {
        let m = self.module.clone();
        let porta = matches!(cell.fx, Fx::TonePorta | Fx::TonePortaVol) || cell.vol == Vol::TonePorta;
        // The instrument.
        if cell.instr > 0 {
            self.chans[ch].instr = cell.instr;
            if let Some(s) = self.sample_for(cell.instr, if cell.note >= NOTE_MIN && cell.note <= NOTE_MAX { cell.note } else { self.chans[ch].note.max(NOTE_MIDDLE_C) }) {
                let smp = &m.samples[s];
                // Its default volume, and its panning when it has one.
                self.chans[ch].volume = i32::from(smp.volume);
                if let Some(p) = smp.pan {
                    self.chans[ch].pan = i32::from(p);
                }
                if let Some(ins) = m.flags.instruments.then(|| m.instruments.get(usize::from(cell.instr) - 1)).flatten() {
                    if let Some(p) = ins.pan {
                        if smp.pan.is_none() || m.kind == Kind::It {
                            self.chans[ch].pan = i32::from(p);
                        }
                    }
                }
                if !(cell.note >= NOTE_MIN && cell.note <= NOTE_MAX) && m.kind == Kind::Mod {
                    // ProTracker's instrument swap: a new sample named alone
                    // takes over at the playing one's loop end.
                    let v = self.chans[ch].voice;
                    if self.voices[v].active && self.voices[v].sample != s {
                        self.voices[v].swap = Some(s);
                    }
                }
                if !(cell.note >= NOTE_MIN && cell.note <= NOTE_MAX) {
                    // An instrument alone: the volume resets, and (XM) the envelopes.
                    if m.kind == Kind::Xm {
                        let v = self.chans[ch].voice;
                        if self.voices[v].active {
                            self.voices[v].released = false;
                            self.voices[v].fading = false;
                            self.voices[v].fade = 65536;
                            if let Some(ins) = self.voices[v].instrument.and_then(|i| m.instruments.get(i)) {
                                self.voices[v].vol_env.reset(&ins.volume_env);
                                self.voices[v].pan_env.reset(&ins.pan_env);
                            }
                        }
                    }
                }
            }
        }
        // The note.
        match cell.note {
            NOTE_OFF => self.key_off(ch),
            NOTE_CUT => self.cut(ch),
            NOTE_FADE => {
                let v = self.chans[ch].voice;
                self.voices[v].fading = true;
            }
            n @ NOTE_MIN..=NOTE_MAX => {
                let instr = self.chans[ch].instr;
                let v = self.chans[ch].voice;
                // A slide to a note doesn't start it again, even when the
                // last note's sample has run out (only IT plays it then).
                let active = self.voices[v].active && !self.voices[v].dying || (m.kind != Kind::It && self.chans[ch].sample.is_some());
                if porta && active {
                    if let Some(s) = self.sample_for(instr, n) {
                        let (note, _) = self.mapped(instr, n);
                        self.chans[ch].target = self.note_period(note, m.samples[s].c5 as f64);
                        self.chans[ch].note = n;
                        if m.kind == Kind::Xm && cell.instr > 0 {
                            self.voices[v].released = false;
                            self.voices[v].fade = 65536;
                            self.voices[v].fading = false;
                        }
                    }
                } else {
                    self.trigger(ch, n, &cell);
                }
            }
            _ => {}
        }
        // The volume column.
        self.volume_column(ch, cell, true);
        self.first_tick_effects(ch, cell);
    }

    /// The sample a note of an instrument (or a sample, without
    /// instruments) plays, 0-based.
    fn sample_for(&self, instr: u8, note: u8) -> Option<usize> {
        let m = &self.module;
        if instr == 0 {
            return None;
        }
        let s = if m.flags.instruments {
            let ins = m.instruments.get(usize::from(instr) - 1)?;
            usize::from(ins.map(note).1).checked_sub(1)?
        } else {
            usize::from(instr) - 1
        };
        (s < m.samples.len()).then_some(s)
    }

    fn mapped(&self, instr: u8, note: u8) -> (u8, Option<usize>) {
        let m = &self.module;
        if m.flags.instruments && instr > 0 {
            if let Some(ins) = m.instruments.get(usize::from(instr) - 1) {
                let (n, _) = ins.map(note);
                return (n.clamp(NOTE_MIN, NOTE_MAX), Some(usize::from(instr) - 1));
            }
        }
        (note, None)
    }

    fn note_period(&self, note: u8, c5: f64) -> f64 {
        if self.module.flags.linear {
            7680.0 - f64::from(note.saturating_sub(1)) * 64.0
        } else {
            AMIGA_CLOCK / c5.max(1.0) * 2f64.powf(f64::from(61 - i32::from(note)) / 12.0)
        }
    }

    fn frequency(&self, period: f64, c5: f64) -> f64 {
        if self.module.flags.linear {
            c5 * 2f64.powf((LINEAR_C5 - period) / 768.0)
        } else {
            AMIGA_CLOCK / period.max(1.0)
        }
    }

    /// Starts a note on a channel: the voice it had goes to the
    /// background, as the instrument's new note action says.
    fn trigger(&mut self, ch: usize, note: u8, cell: &Cell) {
        let m = self.module.clone();
        let instr = self.chans[ch].instr;
        let Some(s) = self.sample_for(instr, note) else {
            // A note with nothing to play stops what was playing (IT plays on).
            if m.kind != Kind::It {
                self.cut(ch);
            }
            return;
        };
        let (mapped, ins_index) = self.mapped(instr, note);
        let smp = &m.samples[s];
        if !smp.sounds() {
            self.cut(ch);
            return;
        }
        let ins = ins_index.and_then(|i| m.instruments.get(i));
        // The note before: to the background, or faded out.
        let old = self.chans[ch].voice;
        // IT's envelope carry: the same instrument's envelopes go on from where they were.
        let carried = (self.voices[old].active && self.voices[old].instrument == ins_index && m.kind == Kind::It).then(|| (self.voices[old].vol_env, self.voices[old].pan_env, self.voices[old].pitch_env));
        if self.voices[old].active {
            let nna = self.chans[ch].nna.or(ins.map(|i| i.nna)).unwrap_or(Nna::Cut);
            let keep = m.kind == Kind::It && m.flags.instruments && nna != Nna::Cut;
            if let Some(slot) = self.free_background() {
                self.voices[slot] = self.voices[old].clone();
                let bg = &mut self.voices[slot];
                if keep {
                    match nna {
                        Nna::Off => bg.released = true,
                        Nna::Fade => bg.fading = true,
                        _ => {}
                    }
                } else {
                    bg.dying = true;
                    bg.target = (0.0, 0.0);
                    bg.ramp = self.ramp_len;
                }
            }
            self.voices[old].active = false;
        }
        // Duplicate checks (IT): the same note, sample or instrument
        // left in the background goes.
        if let Some(ins) = ins.filter(|i| i.dct > 0) {
            for v in self.voices.iter_mut().skip(self.chans.len()) {
                if !v.active || v.channel != ch || v.dying {
                    continue;
                }
                // Only this instrument's own notes are checked (IT's way).
                let dup = v.instrument == ins_index
                    && match ins.dct {
                        1 => v.note == note,
                        2 => v.sample == s,
                        _ => true,
                    };
                if dup {
                    match ins.dca {
                        1 => v.released = true,
                        2 => v.fading = true,
                        _ => {
                            v.dying = true;
                            v.target = (0.0, 0.0);
                            v.ramp = v.ramp.max(1);
                        }
                    }
                }
            }
        }
        let c = &mut self.chans[ch];
        c.note = note;
        c.sample = Some(s);
        c.c5 = f64::from(smp.c5);
        c.nna = None;
        c.vol_env_on = None;
        c.pan_env_on = None;
        c.pitch_env_on = None;
        if c.vib_wave & 4 == 0 {
            c.vib_pos = 0;
        }
        if c.trem_wave & 4 == 0 {
            c.trem_pos = 0;
        }
        c.tremor_count = 0;
        c.tremor_on = true;
        let period = self.note_period(mapped, f64::from(smp.c5));
        let c = &mut self.chans[ch];
        c.period = period;
        c.target = period;
        let fm_hz = f64::from(self.rate) * MIDDLE_C / f64::from(smp.c5.max(1));
        let start_hz = self.frequency(period, f64::from(smp.c5)) * MIDDLE_C / f64::from(smp.c5.max(1));
        let rate = self.rate;
        let v = &mut self.voices[old];
        *v = Voice {
            active: true,
            wave: smp.wave.clone(),
            sample: s,
            instrument: ins_index,
            channel: ch,
            note,
            looped: smp.looped,
            sustain: smp.sustain,
            fade: 65536,
            ramp: 0,
            ..Default::default()
        };
        if let Some(patch) = smp.fm {
            let mut fm = FmVoice::new(patch, rate);
            fm.key_on(start_hz);
            v.fm = Some(fm);
            v.fm_hz = fm_hz;
        } else if let Some(prog) = &smp.program {
            // Its first step is taken with the tick's update, which follows.
            v.program = Some((prog.clone(), ProgramState::new(&prog.sound)));
            set_program_wave(v, smp);
        }
        if let Some(ins) = ins {
            v.vol_env.reset(&ins.volume_env);
            v.pan_env.reset(&ins.pan_env);
            v.pitch_env.reset(&ins.pitch_env);
            if let Some((ve, pe, fe)) = carried {
                if ins.volume_env.carry {
                    v.vol_env = EnvState { released: false, offset: 0, ..ve };
                }
                if ins.pan_env.carry {
                    v.pan_env = EnvState { released: false, offset: 0, ..pe };
                }
                if ins.pitch_env.carry {
                    v.pitch_env = EnvState { released: false, offset: 0, ..fe };
                }
            }
            // Random volume and panning swing.
            if ins.random_volume > 0 || ins.random_pan > 0 {
                self.random = self.random.wrapping_mul(1_103_515_245).wrapping_add(12345);
                let r = ((self.random >> 16) % 201) as i32 - 100;
                let c = &mut self.chans[ch];
                c.volume = (c.volume + c.volume * r * i32::from(ins.random_volume) / 10000).clamp(0, 64);
                c.pan = (c.pan + r * i32::from(ins.random_pan) * 128 / 6400).clamp(0, 255);
            }
            // Pitch-pan separation.
            if ins.pitch_pan_separation != 0 {
                let c = &mut self.chans[ch];
                let d = (i32::from(note) - i32::from(ins.pitch_pan_center) - 1) * i32::from(ins.pitch_pan_separation) / 8;
                c.pan = (c.pan + d).clamp(0, 255);
            }
            self.chans[ch].cutoff = ins.cutoff;
            self.chans[ch].resonance = ins.resonance;
        }
        // The sample offset.
        let c = &self.chans[ch];
        let (fx, param) = (cell.fx, cell.param);
        if fx == Fx::Offset || cell.vol == Vol::Offset {
            let p = if fx == Fx::Offset { if param == 0 { c.offset } else { param } } else { cell.volp };
            let off = (u32::from(p) * 256 + c.high_offset) as f64;
            let frames = smp.wave.frames() as f64;
            let v = &mut self.voices[old];
            // Past a loop's end (where nothing's ever heard) is the loop's
            // start, so a sample cut at its loop plays the same.
            if smp.looped.on() && off >= f64::from(smp.looped.end) {
                v.pos = f64::from(smp.looped.start);
            } else if off < frames {
                v.pos = off;
            } else {
                v.pos = frames;
            }
            if fx == Fx::Offset && param != 0 {
                self.chans[ch].offset = param;
            }
        }
        if let Some(i) = ins_index.filter(|&i| m.instruments.get(i).is_some_and(|x| x.midi.is_some())) {
            self.midi_note_on(ch, note, i);
        }
    }

    /// An instrument's note sent over MIDI too, when it has an output:
    /// the channel's last note let go first, and the program (and bank)
    /// changed when it isn't already.
    fn midi_note_on(&mut self, ch: usize, note: u8, ins: usize) {
        self.midi_off(ch);
        let Some(out) = self.module.instruments.get(ins).and_then(|i| i.midi) else { return };
        let mc = out.channel & 15;
        if let Some(program) = out.program {
            if self.midi_programs[usize::from(mc)] != Some((program, out.bank)) {
                self.midi_programs[usize::from(mc)] = Some((program, out.bank));
                if let Some(bank) = out.bank {
                    self.send_midi(&[0xB0 | mc, 0, ((bank >> 7) & 127) as u8]);
                    self.send_midi(&[0xB0 | mc, 32, (bank & 127) as u8]);
                }
                self.send_midi(&[0xC0 | mc, program & 127]);
            }
        }
        let velocity = (self.chans[ch].volume.clamp(1, 64) * 127 / 64) as u8;
        let n = note.saturating_sub(1).min(127);
        self.send_midi(&[0x90 | mc, n, velocity.max(1)]);
        self.chans[ch].midi_note = Some((mc, n));
    }

    fn free_background(&mut self) -> Option<usize> {
        let first = self.chans.len();
        if let Some(i) = (first..self.voices.len()).find(|&i| !self.voices[i].active) {
            return Some(i);
        }
        // All in use: the quietest goes.
        (first..self.voices.len()).min_by(|&a, &b| {
            let va = self.voices[a].gain.0 + self.voices[a].gain.1;
            let vb = self.voices[b].gain.0 + self.voices[b].gain.1;
            va.partial_cmp(&vb).unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    fn key_off(&mut self, ch: usize) {
        self.midi_off(ch);
        let m = self.module.clone();
        let v = self.chans[ch].voice;
        let voice = &mut self.voices[v];
        if !voice.active {
            return;
        }
        voice.released = true;
        let env = voice.instrument.and_then(|i| m.instruments.get(i)).map(|i| &i.volume_env);
        let has_env = env.is_some_and(|e| e.enabled) && self.chans[ch].vol_env_on != Some(false);
        match m.kind {
            Kind::Xm => {
                if has_env {
                    voice.fading = true;
                } else {
                    self.chans[ch].volume = 0;
                }
            }
            Kind::It if m.flags.instruments => {
                if !has_env || env.is_some_and(|e| e.looped.is_some()) {
                    voice.fading = true;
                }
            }
            _ => {
                // No instruments: the note stops.
                self.cut(ch);
            }
        }
    }

    fn cut(&mut self, ch: usize) {
        self.midi_off(ch);
        let v = self.chans[ch].voice;
        let voice = &mut self.voices[v];
        // An FM note is let go, as the chip's key off: it rings out by
        // its own release.
        if let Some(fm) = voice.fm.as_mut().filter(|_| voice.active && !voice.dying) {
            fm.key_off();
            voice.released = true;
            return;
        }
        if voice.active {
            voice.dying = true;
            voice.target = (0.0, 0.0);
            voice.ramp = self.ramp_len;
        }
    }

    fn volume_column(&mut self, ch: usize, cell: Cell, first: bool) {
        let kind = self.module.kind;
        let (vol, x) = (cell.vol, cell.volp);
        let c = &mut self.chans[ch];
        if first {
            match vol {
                Vol::Volume => c.volume = i32::from(x.min(64)),
                Vol::Panning => {
                    c.pan = (i32::from(x.min(64)) * 255 / 64).clamp(0, 255);
                    c.surround = false;
                }
                Vol::FineVolUp => {
                    let x = if x == 0 { c.fine_vol_up } else { x };
                    c.fine_vol_up = x;
                    c.volume = (c.volume + i32::from(x)).min(64);
                }
                Vol::FineVolDown => {
                    let x = if x == 0 { c.fine_vol_down } else { x };
                    c.fine_vol_down = x;
                    c.volume = (c.volume - i32::from(x)).max(0);
                }
                Vol::VibratoSpeed => {
                    if x > 0 {
                        c.vib_speed = x;
                    }
                }
                Vol::VibratoDepth => {
                    if x > 0 {
                        c.vib_depth = x;
                    }
                    if kind == Kind::It {
                        c.vol_col_slide = Some((Vol::VibratoDepth, x));
                    }
                }
                Vol::TonePorta => {
                    if x > 0 {
                        c.porta_speed = if kind == Kind::Xm { x << 4 } else { x };
                    }
                    c.vol_col_slide = Some((Vol::TonePorta, x));
                }
                Vol::VolSlideUp | Vol::VolSlideDown | Vol::PanSlideLeft | Vol::PanSlideRight | Vol::PortaUp | Vol::PortaDown => c.vol_col_slide = Some((vol, x)),
                _ => {}
            }
        } else if let Some((vol, x)) = c.vol_col_slide {
            match vol {
                Vol::VolSlideUp => c.volume = (c.volume + i32::from(x)).min(64),
                Vol::VolSlideDown => c.volume = (c.volume - i32::from(x)).max(0),
                Vol::PanSlideLeft => c.pan = (c.pan - i32::from(x) * 4).max(0),
                Vol::PanSlideRight => c.pan = (c.pan + i32::from(x) * 4).min(255),
                Vol::PortaUp => {
                    c.period -= f64::from(x) * 16.0;
                }
                Vol::PortaDown => {
                    c.period += f64::from(x) * 16.0;
                }
                Vol::TonePorta => self.tone_porta(ch),
                Vol::VibratoDepth => self.vibrato(ch, false),
                _ => {}
            }
        }
    }

    // ---- Effects ----

    fn first_tick_effects(&mut self, ch: usize, cell: Cell) {
        let m = self.module.clone();
        let kind = m.kind;
        let (fx, param) = (cell.fx, cell.param);
        let s3m = kind.s3m_like();
        let c = &mut self.chans[ch];
        if s3m && param != 0 {
            c.s3m_last = param;
        }
        match fx {
            Fx::Speed => {
                if param > 0 {
                    self.speed = u32::from(param);
                }
            }
            Fx::Tempo => {
                if param >= 0x20 {
                    self.tempo = u32::from(param);
                } else if s3m {
                    // T0x and T1x slide the tempo, tick by tick.
                    if param != 0 {
                        c.tempo_slide = param;
                    }
                }
            }
            Fx::PositionJump => self.jump = Some(usize::from(param)),
            Fx::PatternBreak => self.brk = Some(usize::from(param)),
            Fx::Volume => c.volume = i32::from(param.min(64)),
            Fx::Panning => {
                c.pan = i32::from(param);
                c.surround = false;
            }
            Fx::ChannelVolume => c.chan_volume = i32::from(param.min(64)),
            Fx::GlobalVolume => self.global_volume = i32::from(param.min(128)),
            Fx::Offset => {
                if param > 0 {
                    c.offset = param;
                }
            }
            Fx::Arpeggio => {
                if param > 0 {
                    c.arpeggio = param;
                }
            }
            Fx::PortaUp | Fx::PortaDown => {
                let up = fx == Fx::PortaUp;
                let p = if s3m {
                    // E and F share their memory (and G, with compatible Gxx off).
                    let p = if param == 0 { c.porta_up } else { param };
                    c.porta_up = p;
                    c.porta_down = p;
                    p
                } else if kind == Kind::Xm {
                    let mem = if up { &mut c.porta_up } else { &mut c.porta_down };
                    if param != 0 {
                        *mem = param;
                    }
                    *mem
                } else {
                    param
                };
                if s3m && p >= 0xE0 {
                    // Fine (Fx) and extra-fine (Ex), on this tick only.
                    let amount = if p >= 0xF0 { f64::from(p & 0x0F) * 4.0 } else { f64::from(p & 0x0F) };
                    self.slide(ch, if up { -amount } else { amount });
                }
                if kind == Kind::Xm {
                    let c = &mut self.chans[ch];
                    if up {
                        c.porta_up = p;
                    } else {
                        c.porta_down = p;
                    }
                }
            }
            Fx::FinePortaUp | Fx::FinePortaDown => {
                self.slide(ch, if fx == Fx::FinePortaUp { -4.0 } else { 4.0 } * f64::from(param & 0x0F));
            }
            Fx::ExtraFinePorta => {
                let p = if param & 0x0F == 0 { c.xfine } else { param };
                c.xfine = p;
                match p >> 4 {
                    1 => self.slide(ch, -f64::from(p & 0x0F)),
                    2 => self.slide(ch, f64::from(p & 0x0F)),
                    _ => {}
                }
            }
            Fx::TonePorta | Fx::TonePortaVol => {
                if fx == Fx::TonePorta && param > 0 {
                    c.porta_speed = param;
                    if s3m && !m.flags.compat_gxx {
                        c.porta_up = param;
                        c.porta_down = param;
                    }
                }
                if fx == Fx::TonePortaVol {
                    self.vol_slide_memory(ch, param);
                    self.fine_vol_slides(ch);
                }
            }
            Fx::Vibrato | Fx::FineVibrato | Fx::VibratoVol => {
                if fx != Fx::VibratoVol {
                    if param >> 4 != 0 {
                        c.vib_speed = param >> 4;
                    }
                    if param & 0x0F != 0 {
                        c.vib_depth = param & 0x0F;
                    }
                    c.vib_fine = fx == Fx::FineVibrato;
                } else {
                    self.vol_slide_memory(ch, param);
                    self.fine_vol_slides(ch);
                }
                if kind == Kind::It && !m.flags.old_effects {
                    self.vibrato(ch, true);
                }
            }
            Fx::Tremolo => {
                if param >> 4 != 0 {
                    c.trem_speed = param >> 4;
                }
                if param & 0x0F != 0 {
                    c.trem_depth = param & 0x0F;
                }
            }
            Fx::Panbrello => {
                if param >> 4 != 0 {
                    c.pbr_speed = param >> 4;
                }
                if param & 0x0F != 0 {
                    c.pbr_depth = param & 0x0F;
                }
            }
            Fx::VolSlide => {
                self.vol_slide_memory(ch, param);
                self.fine_vol_slides(ch);
                if m.flags.fast_slides {
                    self.vol_slide(ch);
                }
            }
            Fx::FineVolSlideUp => c.volume = (c.volume + i32::from(param)).min(64),
            Fx::FineVolSlideDown => c.volume = (c.volume - i32::from(param)).max(0),
            Fx::ChannelVolSlide => {
                if param != 0 {
                    c.chan_vol_slide = param;
                }
                let p = c.chan_vol_slide;
                if let Some(d) = fine_amount(p) {
                    c.chan_volume = (c.chan_volume + d).clamp(0, 64);
                }
            }
            Fx::GlobalVolSlide => {
                if param != 0 {
                    c.global_slide = param;
                }
                let p = c.global_slide;
                if s3m {
                    if let Some(d) = fine_amount(p) {
                        self.global_volume = (self.global_volume + d * 2).clamp(0, 128);
                    }
                }
            }
            Fx::PanningSlide => {
                if param != 0 {
                    c.pan_slide = param;
                }
                let p = c.pan_slide;
                if s3m {
                    // IT: P0x right, Px0 left, PFx and PxF finely.
                    if p & 0xF0 == 0xF0 && p & 0x0F != 0 {
                        c.pan = (c.pan + i32::from(p & 0x0F) * 4).min(255);
                    } else if p & 0x0F == 0x0F && p & 0xF0 != 0 {
                        c.pan = (c.pan - i32::from(p >> 4) * 4).max(0);
                    }
                }
            }
            Fx::Retrig => {
                if param & 0x0F != 0 {
                    c.retrig = param;
                } else if param != 0 {
                    c.retrig = (param & 0xF0) | (c.retrig & 0x0F);
                }
                if cell.note >= NOTE_MIN && cell.note <= NOTE_MAX && kind != Kind::It {
                    c.retrig_count = 0;
                }
            }
            Fx::Tremor => {
                if param != 0 {
                    c.tremor = param;
                }
            }
            Fx::KeyOff => {
                if param == 0 {
                    self.key_off(ch);
                } else {
                    c.off_at = Some(param);
                }
            }
            Fx::EnvelopePosition => {
                let v = c.voice;
                self.voices[v].vol_env.tick = u32::from(param);
                // FastTracker 2 moves the panning envelope too only when the
                // volume envelope has its sustain on.
                let pan_too = kind != Kind::Xm || self.voices[v].instrument.and_then(|i| m.instruments.get(i)).is_some_and(|i| i.volume_env.sustain.is_some());
                if pan_too {
                    self.voices[v].pan_env.tick = u32::from(param);
                }
            }
            Fx::DelayCut => {
                c.cut_at = Some((param >> 4) + (param & 0x0F));
            }
            Fx::NoteSlideUp | Fx::NoteSlideDown => {
                let up = fx == Fx::NoteSlideUp;
                c.note_slide = Some((if up { 1 } else { -1 }, param >> 4, param & 0x0F));
            }
            Fx::SetFinetune => {
                let ft = ((param & 0x0F) as i8) << 4 >> 4;
                c.c5 = 8363.0 * 2f64.powf(f64::from(ft) / 96.0);
            }
            Fx::Phase => c.am_phase.command(param),
            Fx::ModEx => self.mod_ex(ch, param, cell),
            Fx::S3mEx => self.s3m_ex(ch, param, cell),
            Fx::Midi => self.midi_macro(ch, param, false),
            Fx::SmoothMidi => self.midi_macro(ch, param, true),
            _ => {}
        }
        // Arpeggio, vibrato and tremolo show on the first tick too.
        if fx == Fx::Arpeggio {
            self.chans[ch].arp_now = 0;
        }
        if fx == Fx::Tremor {
            self.tremor(ch);
        }
    }

    fn vol_slide_memory(&mut self, ch: usize, param: u8) {
        let kind = self.module.kind;
        let c = &mut self.chans[ch];
        if param != 0 || kind == Kind::Mod {
            c.vol_slide = param;
        }
    }

    /// S3M's and IT's fine volume slides (DxF, DFy): on the first tick.
    fn fine_vol_slides(&mut self, ch: usize) {
        if !self.module.kind.s3m_like() {
            return;
        }
        let c = &mut self.chans[ch];
        if let Some(d) = fine_amount(c.vol_slide) {
            c.volume = (c.volume + d).clamp(0, 64);
        }
    }

    fn mod_ex(&mut self, ch: usize, param: u8, cell: Cell) {
        let kind = self.module.kind;
        let (x, y) = (param >> 4, param & 0x0F);
        let c = &mut self.chans[ch];
        match x {
            // ProTracker's E0x: the Amiga's LED filter on (0) or off.
            0x0 if kind == Kind::Mod => self.led = y == 0,
            0x1 => {
                let y = if kind == Kind::Xm && y == 0 { c.fine_up } else { y };
                c.fine_up = y;
                self.slide(ch, -f64::from(y) * 4.0);
            }
            0x2 => {
                let y = if kind == Kind::Xm && y == 0 { c.fine_down } else { y };
                c.fine_down = y;
                self.slide(ch, f64::from(y) * 4.0);
            }
            0x4 => c.vib_wave = y & 7,
            0x5 => {
                let ft = ((y as i8) << 4) >> 4;
                c.c5 = 8363.0 * 2f64.powf(f64::from(ft) / 96.0);
                if cell.note >= NOTE_MIN && cell.note <= NOTE_MAX && !self.module.flags.linear {
                    let n = c.note;
                    let c5 = c.c5;
                    let p = self.note_period(n, c5);
                    self.chans[ch].period = p;
                    self.chans[ch].target = p;
                }
            }
            0x6 => self.pattern_loop(ch, y),
            0x7 => c.trem_wave = y & 7,
            0x8 => c.pan = i32::from(y) * 17,
            0x9 => {
                c.retrig = y;
                c.retrig_count = 0;
            }
            0xA => {
                let y = if kind == Kind::Xm && y == 0 { c.fine_vol_up } else { y };
                c.fine_vol_up = y;
                c.volume = (c.volume + i32::from(y)).min(64);
            }
            0xB => {
                let y = if kind == Kind::Xm && y == 0 { c.fine_vol_down } else { y };
                c.fine_vol_down = y;
                c.volume = (c.volume - i32::from(y)).max(0);
            }
            0xC => {
                if y == 0 && kind == Kind::Mod {
                    c.volume = 0;
                } else {
                    c.cut_at = Some(y);
                }
            }
            0xE => {
                if self.pattern_delay == 0 {
                    self.pattern_delay = u32::from(y) + 1;
                }
            }
            _ => {}
        }
    }

    fn s3m_ex(&mut self, ch: usize, param: u8, _cell: Cell) {
        let (x, y) = (param >> 4, param & 0x0F);
        let c = &mut self.chans[ch];
        match x {
            0x2 => {
                let ft = ((y as i8) << 4) >> 4;
                c.c5 = 8363.0 * 2f64.powf(f64::from(ft) / 96.0);
            }
            0x3 => c.vib_wave = y & 7,
            0x4 => c.trem_wave = y & 7,
            0x5 => c.pbr_wave = y & 7,
            0x6 => self.fine_delay += u32::from(y),
            0x7 => self.past_notes(ch, y),
            0x8 => {
                c.pan = i32::from(y) * 17;
                c.surround = false;
            }
            0x9 => {
                if y == 1 {
                    c.surround = true;
                } else if y == 0 {
                    c.surround = false;
                } else if y == 0xF || y == 0xE {
                    let v = c.voice;
                    self.voices[v].backwards = y == 0xF;
                }
            }
            0xA => c.high_offset = u32::from(y) << 16,
            0xB => self.pattern_loop(ch, y),
            0xC => c.cut_at = Some(if y == 0 { 1 } else { y }),
            0xE => {
                if self.pattern_delay == 0 {
                    self.pattern_delay = u32::from(y) + 1;
                }
            }
            0xF => c.sfx = y,
            _ => {}
        }
    }

    /// Zxx (or \xx, `smooth`): the MIDI macro it names, worked out. Neumetik's
    /// own messages (F0 F0 n v) set the filter or an effect slot's
    /// parameter; any other goes out over MIDI, when the channel's
    /// instrument has an output.
    fn midi_macro(&mut self, ch: usize, param: u8, smooth: bool) {
        let m = self.module.clone();
        let defaults;
        let macros = match &m.macros {
            Some(x) => x,
            None => {
                defaults = MidiMacros::default();
                &defaults
            }
        };
        let c = &self.chans[ch];
        let text = if param < 0x80 { macros.parametered.get(usize::from(c.sfx)) } else { macros.fixed.get(usize::from(param - 0x80)) };
        let Some(text) = text else { return };
        let ins = (c.instr > 0).then(|| m.instruments.get(usize::from(c.instr) - 1)).flatten();
        let midi_ch = ins.and_then(|i| i.midi).map_or(0, |o| o.channel & 15);
        let vars = MacroVars { z: param & 0x7F, note: c.note.saturating_sub(1).min(127), velocity: (c.volume.clamp(0, 64) * 2).min(127) as u8, volume: (c.chan_volume.clamp(0, 64) * 2).min(127) as u8, pan: (c.pan / 2).clamp(0, 127) as u8, channel: midi_ch };
        let mut bytes = [0u8; 16];
        let n = eval_macro(text, &vars, &mut bytes);
        let b = &bytes[..n];
        if n >= 4 && b[0] == 0xF0 && b[1] == 0xF0 {
            let (what, v) = (b[2], b[3] & 0x7F);
            let from = match what {
                0 => self.chans[ch].cutoff.map_or(127.0, f32::from),
                1 => self.chans[ch].resonance.map_or(0.0, f32::from),
                w if w >= 0x80 => self.slot_param(ch, usize::from(w - 0x80)).map_or(0.0, |p| p * 127.0),
                _ => 0.0,
            };
            if smooth && what != 2 {
                self.chans[ch].smooth = Some((what, from, f32::from(v)));
            } else {
                self.internal_macro(ch, what, f32::from(v));
            }
        } else if n > 0 && ins.is_some_and(|i| i.midi.is_some()) {
            // Sent as messages of up to three bytes.
            for chunk in b.chunks(3) {
                self.send_midi(chunk);
            }
        }
    }

    /// The effect slot a channel's sound goes through (its instrument's,
    /// or its own), and a parameter's value there.
    fn slot_param(&self, ch: usize, param: usize) -> Option<f32> {
        let slot = self.route_of(ch)?;
        self.rack.as_deref()?.param(usize::from(slot), param)
    }

    fn route_of(&self, ch: usize) -> Option<u8> {
        let m = &self.module;
        let c = &self.chans[ch];
        let ins = (c.instr > 0 && m.flags.instruments).then(|| m.instruments.get(usize::from(c.instr) - 1)).flatten();
        ins.and_then(|i| i.effect).or(m.channels.get(ch).and_then(|c| c.effect)).filter(|&e| usize::from(e) < m.effects.len())
    }

    /// F0 F0 `what` `v` (0 to 127): 0 the cutoff, 1 the resonance, 2 the
    /// filter's mode, 80 and up an effect slot's parameter.
    fn internal_macro(&mut self, ch: usize, what: u8, v: f32) {
        let v7 = v.round().clamp(0.0, 127.0) as u8;
        match what {
            0 => self.chans[ch].cutoff = Some(v7),
            1 => self.chans[ch].resonance = Some(v7),
            2 => self.chans[ch].highpass = v7 >= 0x10,
            w if w >= 0x80 => {
                if let (Some(slot), Some(rack)) = (self.route_of(ch), self.rack.as_deref_mut()) {
                    rack.set_param(usize::from(slot), usize::from(w - 0x80), v / 127.0);
                }
            }
            _ => {}
        }
    }

    /// IT's S7x: what's left in the background, and this note's actions
    /// and envelopes.
    fn past_notes(&mut self, ch: usize, y: u8) {
        let first = self.chans.len();
        match y {
            0..=2 => {
                for v in self.voices.iter_mut().skip(first) {
                    if v.active && v.channel == ch {
                        match y {
                            0 => {
                                v.dying = true;
                                v.target = (0.0, 0.0);
                                v.ramp = v.ramp.max(1);
                            }
                            1 => v.released = true,
                            _ => v.fading = true,
                        }
                    }
                }
            }
            3..=6 => self.chans[ch].nna = Some(Nna::from_u8(y - 3)),
            7 | 8 => self.chans[ch].vol_env_on = Some(y == 8),
            9 | 10 => self.chans[ch].pan_env_on = Some(y == 10),
            11 | 12 => self.chans[ch].pitch_env_on = Some(y == 12),
            _ => {}
        }
    }

    fn pattern_loop(&mut self, ch: usize, y: u8) {
        let row = self.row;
        let c = &mut self.chans[ch];
        if y == 0 {
            c.loop_row = row;
        } else if c.loop_count == 0 {
            c.loop_count = y;
            self.loop_to = Some(c.loop_row);
        } else {
            c.loop_count -= 1;
            if c.loop_count > 0 {
                self.loop_to = Some(c.loop_row);
            } else if self.module.kind == Kind::It {
                c.loop_row = row + 1;
            }
        }
    }

    fn tick_effects(&mut self, ch: usize) {
        let m = self.module.clone();
        let kind = m.kind;
        let tick = self.tick as u8;
        // A delayed note, on its tick.
        if let Some(cell) = self.chans[ch].delayed {
            if tick == self.chans[ch].delay {
                self.chans[ch].delayed = None;
                let row_cell = Cell { fx: Fx::None, param: 0, ..cell };
                self.start_row_cell(ch, row_cell);
            }
        }
        let cell = self.chans[ch].cell;
        self.volume_column(ch, cell, false);
        let (fx, param) = (cell.fx, cell.param);
        match fx {
            Fx::Arpeggio => {
                let p = if param == 0 && kind.s3m_like() { self.chans[ch].arpeggio } else { param };
                // FastTracker 2 counts its ticks down, so its arpeggio runs
                // backwards from the row's end (and, past 16 ticks, sticks).
                let step = if kind == Kind::Xm {
                    let left = self.speed.saturating_sub(self.tick);
                    if left > 16 {
                        2
                    } else if left == 16 {
                        0
                    } else {
                        left % 3
                    }
                } else {
                    self.tick % 3
                };
                self.chans[ch].arp_now = match step {
                    0 => 0,
                    1 => p >> 4,
                    _ => p & 0x0F,
                };
            }
            Fx::SmoothMidi => {
                if let Some((what, from, to)) = self.chans[ch].smooth {
                    let k = self.tick as f32 / self.speed.max(1) as f32;
                    self.internal_macro(ch, what, from + (to - from) * k);
                }
            }
            Fx::PortaUp | Fx::PortaDown => {
                let up = fx == Fx::PortaUp;
                let c = &self.chans[ch];
                let p = match kind {
                    Kind::Mod => param,
                    Kind::Xm => {
                        if up {
                            c.porta_up
                        } else {
                            c.porta_down
                        }
                    }
                    _ => c.porta_up,
                };
                if !(kind.s3m_like() && p >= 0xE0) {
                    self.slide(ch, if up { -1.0 } else { 1.0 } * f64::from(p) * 4.0);
                }
            }
            Fx::TonePorta => self.tone_porta(ch),
            Fx::TonePortaVol => {
                self.tone_porta(ch);
                self.vol_slide(ch);
            }
            Fx::Vibrato | Fx::FineVibrato => self.vibrato(ch, false),
            Fx::VibratoVol => {
                self.vibrato(ch, false);
                self.vol_slide(ch);
            }
            Fx::Tremolo => self.tremolo(ch),
            Fx::Panbrello => self.panbrello(ch),
            Fx::VolSlide => self.vol_slide(ch),
            Fx::ChannelVolSlide => {
                let c = &mut self.chans[ch];
                if let Some(d) = coarse_amount(c.chan_vol_slide) {
                    c.chan_volume = (c.chan_volume + d).clamp(0, 64);
                }
            }
            Fx::GlobalVolSlide => {
                let p = self.chans[ch].global_slide;
                if kind.s3m_like() {
                    if let Some(d) = coarse_amount(p) {
                        self.global_volume = (self.global_volume + d * if kind == Kind::S3m { 2 } else { 1 }).clamp(0, 128);
                    }
                } else {
                    let d = if p >> 4 != 0 { i32::from(p >> 4) } else { -i32::from(p & 0x0F) };
                    self.global_volume = (self.global_volume + d * 2).clamp(0, 128);
                }
            }
            Fx::PanningSlide => {
                let c = &mut self.chans[ch];
                let p = c.pan_slide;
                if kind.s3m_like() {
                    if p & 0xF0 == 0 {
                        c.pan = (c.pan + i32::from(p & 0x0F) * 4).min(255);
                    } else if p & 0x0F == 0 {
                        c.pan = (c.pan - i32::from(p >> 4) * 4).max(0);
                    }
                } else if p >> 4 != 0 {
                    c.pan = (c.pan + i32::from(p >> 4)).min(255);
                } else {
                    c.pan = (c.pan - i32::from(p & 0x0F)).max(0);
                }
            }
            Fx::Retrig => self.retrig(ch),
            Fx::Tremor => self.tremor(ch),
            Fx::Tempo => {
                let s = self.chans[ch].tempo_slide;
                if param < 0x20 && kind.s3m_like() {
                    match s >> 4 {
                        0 => self.tempo = self.tempo.saturating_sub(u32::from(s & 0x0F)).max(32),
                        1 => self.tempo = (self.tempo + u32::from(s & 0x0F)).min(255),
                        _ => {}
                    }
                }
            }
            Fx::ModEx => match param >> 4 {
                0x9 => self.retrig(ch),
                _ => {}
            },
            Fx::NoteSlideUp | Fx::NoteSlideDown => {
                if let Some((dir, speed, semis)) = self.chans[ch].note_slide {
                    if speed > 0 && self.tick % u32::from(speed) == 0 {
                        let units = f64::from(semis) * 64.0;
                        if m.flags.linear {
                            self.slide(ch, -f64::from(dir) * units);
                        } else {
                            let c = &mut self.chans[ch];
                            c.period *= 2f64.powf(-f64::from(dir) * f64::from(semis) / 12.0);
                        }
                    }
                }
            }
            _ => {}
        }
        let c = &mut self.chans[ch];
        if c.cut_at == Some(tick) {
            c.cut_at = None;
            c.volume = 0;
            if kind == Kind::It || kind == Kind::S3m {
                self.cut(ch);
            }
        }
        let c = &mut self.chans[ch];
        if c.off_at == Some(tick) {
            c.off_at = None;
            self.key_off(ch);
        }
    }

    /// A pitch slide, in period units (negative is up).
    fn slide(&mut self, ch: usize, by: f64) {
        let m = &self.module;
        let c = &mut self.chans[ch];
        c.period += by;
        if m.flags.amiga_limits && !m.flags.linear {
            let scale = 8363.0 / c.c5.max(1.0);
            c.period = c.period.clamp(113.0 * 4.0 * scale, 856.0 * 4.0 * scale);
        } else {
            c.period = c.period.clamp(1.0, 1_000_000.0);
        }
    }

    fn tone_porta(&mut self, ch: usize) {
        let c = &mut self.chans[ch];
        let speed = f64::from(c.porta_speed) * 4.0;
        if c.target <= 0.0 {
            return;
        }
        if c.period < c.target {
            c.period = (c.period + speed).min(c.target);
        } else if c.period > c.target {
            c.period = (c.period - speed).max(c.target);
        }
    }

    fn vibrato(&mut self, ch: usize, first: bool) {
        let old = self.module.flags.old_effects;
        let c = &mut self.chans[ch];
        let w = wave(c.vib_wave, c.vib_pos, &mut self.random);
        let depth = i32::from(c.vib_depth);
        let mut d = w * depth;
        d = if c.vib_fine { d / 32 } else { d / 8 };
        if old {
            d *= 2;
            d = -d;
        }
        c.vib_now = d;
        if !first || !old {
            c.vib_pos = c.vib_pos.wrapping_add(c.vib_speed) & 63;
        }
    }

    fn tremolo(&mut self, ch: usize) {
        let c = &mut self.chans[ch];
        let w = wave(c.trem_wave, c.trem_pos, &mut self.random);
        c.trem_now = w * i32::from(c.trem_depth) / 16;
        c.trem_pos = c.trem_pos.wrapping_add(c.trem_speed) & 63;
    }

    fn panbrello(&mut self, ch: usize) {
        let c = &mut self.chans[ch];
        let w = wave(c.pbr_wave, c.pbr_pos, &mut self.random);
        c.pbr_now = w * i32::from(c.pbr_depth) / 8;
        c.pbr_pos = c.pbr_pos.wrapping_add(c.pbr_speed) & 63;
    }

    fn vol_slide(&mut self, ch: usize) {
        let kind = self.module.kind;
        let c = &mut self.chans[ch];
        let p = c.vol_slide;
        let d = if kind.s3m_like() {
            coarse_amount(p).unwrap_or(0)
        } else if p >> 4 != 0 {
            i32::from(p >> 4)
        } else {
            -i32::from(p & 0x0F)
        };
        c.volume = (c.volume + d).clamp(0, 64);
    }

    fn retrig(&mut self, ch: usize) {
        let kind = self.module.kind;
        let c = &mut self.chans[ch];
        let every = c.retrig & 0x0F;
        if every == 0 {
            return;
        }
        c.retrig_count += 1;
        if c.retrig_count < every {
            return;
        }
        c.retrig_count = 0;
        // MOD's E9x keeps the volume; Q's x changes it.
        if !(kind == Kind::Mod && c.cell.fx == Fx::ModEx) {
            let v = c.volume;
            c.volume = match c.retrig >> 4 {
                1 => v - 1,
                2 => v - 2,
                3 => v - 4,
                4 => v - 8,
                5 => v - 16,
                6 => v * 2 / 3,
                7 => v / 2,
                9 => v + 1,
                0xA => v + 2,
                0xB => v + 4,
                0xC => v + 8,
                0xD => v + 16,
                0xE => v * 3 / 2,
                0xF => v * 2,
                _ => v,
            }
            .clamp(0, 64);
        }
        let v = c.voice;
        let voice = &mut self.voices[v];
        if voice.active || c.sample.is_some() {
            voice.pos = 0.0;
            voice.ended = false;
            voice.backwards = false;
            voice.active = voice.active || !voice.wave.is_empty() || voice.fm.is_some();
            voice.dying = false;
            if let Some(fm) = voice.fm.as_mut() {
                fm.key_off();
                fm.key_on(voice.step * voice.fm_hz);
                voice.released = false;
            }
        }
    }

    fn tremor(&mut self, ch: usize) {
        let old = self.module.flags.old_effects;
        let c = &mut self.chans[ch];
        let (on, off) = (c.tremor >> 4, c.tremor & 0x0F);
        let (on, off) = if old { (on.max(1), off.max(1)) } else { (on + 1, off + 1) };
        c.tremor_count = (c.tremor_count + 1) % (on + off);
        c.tremor_on = c.tremor_count < on;
    }

    // ---- The voices ----

    /// A channel's sound after this tick: its voice's rate, volume and
    /// panning, and the background voices' envelopes.
    fn update_voice(&mut self, ch: usize) {
        let last = ch + 1 == self.chans.len();
        let v = self.chans[ch].voice;
        self.update_one(v, Some(ch));
        // The background is moved on once a tick, with the jam channel.
        if last {
            for i in self.chans.len()..self.voices.len() {
                self.update_one(i, None);
            }
        }
    }

    fn update_one(&mut self, vi: usize, ch: Option<usize>) {
        if !self.voices[vi].active {
            return;
        }
        let m = self.module.clone();
        let it = m.kind == Kind::It;
        let owner = self.voices[vi].channel.min(self.chans.len() - 1);
        let (vol_on, pan_on, pitch_on, muted) = {
            let c = &self.chans[owner];
            (c.vol_env_on != Some(false), c.pan_env_on != Some(false), c.pitch_env_on != Some(false), c.muted || c.slot_muted)
        };
        let ins = self.voices[vi].instrument.and_then(|i| m.instruments.get(i));
        let filter_env = ins.is_some_and(|i| i.pitch_env.filter && i.pitch_env.enabled);
        // Envelopes, a tick on.
        let (vol_env, pan_env, pitch_env) = {
            let v = &mut self.voices[vi];
            let r = v.released;
            let ve = ins.filter(|_| vol_on).and_then(|i| v.vol_env.step(&i.volume_env, r, it));
            let pe = ins.filter(|_| pan_on).and_then(|i| v.pan_env.step(&i.pan_env, r, it));
            let fe = ins.filter(|_| pitch_on).and_then(|i| v.pitch_env.step(&i.pitch_env, r, it));
            (ve, pe, fe)
        };
        // The fade, once a note's let go (or its envelope has ended).
        {
            let v = &mut self.voices[vi];
            if let Some(i) = ins {
                if it && v.vol_env.ended && vol_env == Some(0) {
                    v.dying = true;
                }
                if it && v.vol_env.ended {
                    v.fading = true;
                }
                if v.fading {
                    v.fade -= i.fadeout.min(65536) as i32;
                    if v.fade <= 0 {
                        v.fade = 0;
                        v.dying = true;
                    }
                }
            } else if v.fading {
                v.dying = true;
            }
        }
        // A synth sound's sequences, a tick on.
        {
            let v = &mut self.voices[vi];
            // A JamCracker AM wave: its copy for the channel, mixed as the phase runs.
            if let (Some(ch), Some((_, ProgramState::Jam(w)))) = (ch, v.program.as_mut()) {
                let c = &mut self.chans[ch];
                if c.am_waves.len() <= v.sample {
                    c.am_waves.resize(v.sample + 1, 0);
                }
                if let Some(k) = c.am_phase.tick() {
                    c.am_waves[v.sample] = k as u8 + 1;
                }
                *w = c.am_waves[v.sample].checked_sub(1).unwrap_or((ch % neumetik_synth::jam::COPIES) as u8);
            }
            if let Some((prog, state)) = v.program.as_mut() {
                v.program_step = state.tick(&prog.sound);
                if let Some(smp) = m.samples.get(v.sample) {
                    set_program_wave(v, smp);
                }
            }
        }
        if let Some(ch) = ch {
            let c = &self.chans[ch];
            let smp = m.samples.get(self.voices[vi].sample);
            let sample_gv = smp.map_or(64, |s| i32::from(s.global_volume)) as f32 / 64.0;
            let ins_gv = ins.map_or(128, |i| i32::from(i.global_volume)) as f32 / 128.0;
            let tremor = if c.cell.fx == Fx::Tremor && !c.tremor_on { 0 } else { 1 };
            let (fader, fader_pan) = self.faders.get(ch).copied().unwrap_or((1.0, None));
            let units = (c.volume + c.trem_now).clamp(0, 64) * tremor;
            // An FM note's volume is its operators' level (on the chip's
            // decibel steps), not a gain.
            let is_fm = self.voices[vi].fm.is_some();
            let vol = if is_fm { 1.0 } else { units as f32 / 64.0 } * c.chan_volume as f32 / 64.0 * sample_gv * ins_gv * fader;
            let mut pan = c.pan + c.pbr_now;
            if let Some(e) = pan_env {
                pan += e * (128 - (pan - 128).abs()) / 32;
            }
            let pan = match fader_pan {
                Some(f) => (f * 255.0) as i32,
                None => pan.clamp(0, 255),
            };
            let sep = f32::from(m.separation) / 128.0 * self.separation;
            let p = 0.5 + (pan as f32 / 255.0 - 0.5) * sep;
            let gain = (f32::from(m.mix_volume) / 128.0) * 1.6 * self.gain;
            let surround = c.surround;
            // The pitch: the period, vibrato, auto-vibrato, arpeggio, the pitch envelope.
            let mut period = c.period + f64::from(c.vib_now);
            let (c5, arp, cutoff, resonance, highpass) = (c.c5, f64::from(c.arp_now), c.cutoff, c.resonance, c.highpass);
            let n_fx = m.effects.len();
            let route = ins.and_then(|i| i.effect).or(m.channels.get(ch).and_then(|c| c.effect)).filter(|&e| usize::from(e) < n_fx);
            if let Some(av) = smp.map(|s| s.vibrato).filter(|a| a.depth > 0) {
                let kind = match av.kind {
                    1 => 4,
                    2 => 2,
                    3 => 3,
                    4 => 1,
                    _ => 0,
                };
                let pos = self.voices[vi].auto_pos;
                let w = wave(kind, pos, &mut self.random);
                let v = &mut self.voices[vi];
                let full = i32::from(av.depth) * 256;
                let step = if av.sweep == 0 { full } else { (full / i32::from(av.sweep)).max(1) };
                v.auto_level = (v.auto_level + step).min(full);
                v.auto_pos = pos.wrapping_add((av.speed / 4).max(1)) & 63;
                period += f64::from(w) * f64::from(v.auto_level) / 256.0 / 64.0;
            }
            let mut freq = if m.flags.linear { self.frequency(period - arp * 64.0, c5) } else { self.frequency(period * 2f64.powf(-arp / 12.0), c5) };
            // A SEL script's LFOs and detune on the channel.
            let mods = if m.sel.is_some() { self.sel.mods(ch) } else { sel::Mods::default() };
            if mods.slide != 0 || mods.detune != 0.0 {
                freq *= 2f64.powf(f64::from(mods.slide) / 768.0 + f64::from(mods.detune) / 12.0);
            }
            let vol = match mods.volume_add {
                Some(add) => (vol + add as f32 / 16384.0).clamp(0.0, 1.0),
                None => vol,
            };
            let (gl, mut gr) = (vol * (1.0 - p) * gain, vol * p * gain);
            if surround {
                gr = -gr;
            }
            if let Some(e) = pitch_env.filter(|_| !filter_env) {
                freq *= 2f64.powf(f64::from(e) / 24.0);
            }
            // A synth sound's own pitch: its slides and vibrato on the
            // Amiga's period (four of ours), its arpeggio in semitones.
            let ps = self.voices[vi].program_step;
            if self.voices[vi].program.is_some() && freq > 0.0 {
                if ps.period != 0 {
                    freq = AMIGA_CLOCK / (AMIGA_CLOCK / freq + 4.0 * f64::from(ps.period)).max(1.0);
                }
                freq *= 2f64.powf(f64::from(ps.arp) / 12.0);
            }
            let cutoff = if filter_env {
                let e = pitch_env.unwrap_or(0) + 32;
                Some((i32::from(cutoff.unwrap_or(127)) * e / 64).clamp(0, 127) as u8)
            } else {
                cutoff
            };
            let rate = self.rate;
            let v = &mut self.voices[vi];
            v.base = (gl, gr);
            v.step = (freq / f64::from(rate)).clamp(0.0, 64.0);
            if let Some(fm) = v.fm.as_mut() {
                fm.set_volume(units as u8);
                fm.set_hz(v.step * v.fm_hz);
            }
            v.filter = filter(cutoff, resonance, rate);
            v.highpass = highpass;
            v.route = route;
        }
        let env = vol_env.map_or(1.0, |e| e.clamp(0, 64) as f32 / 64.0);
        let global = self.global_volume as f32 / 128.0;
        let ramp = self.ramp_len;
        let v = &mut self.voices[vi];
        let fade = v.fade as f32 / 65536.0;
        let program = if v.program.is_some() { f32::from(v.program_step.volume) / 64.0 } else { 1.0 };
        let k = env * fade * global * program;
        v.target = if muted || v.dying { (0.0, 0.0) } else { (v.base.0 * k, v.base.1 * k) };
        v.ramp = ramp;
    }

    fn mix(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len();
        let ramp_len = self.ramp_len;
        let interp = self.interp;
        let foreground = self.chans.len();
        let module = self.module.clone();
        let sinc = &self.sinc;
        let mut rack = self.rack.as_deref_mut();
        for (vi, voice) in self.voices.iter_mut().enumerate() {
            if !voice.active {
                continue;
            }
            if voice.ended {
                // Silent: a channel's own stays its note; one in the
                // background, or cut, is done.
                if vi >= foreground || voice.dying {
                    voice.active = false;
                }
                continue;
            }
            let mut frames = voice.wave.frames();
            if frames == 0 && voice.fm.is_none() {
                voice.active = false;
                continue;
            }
            // Into its effect slot, or the mix.
            let (dl, dr): (&mut [f32], &mut [f32]) = match (voice.route, rack.as_deref_mut()) {
                (Some(slot), Some(rk)) => match rk.input(usize::from(slot), n) {
                    Some(io) => io,
                    None => (&mut *left, &mut *right),
                },
                _ => (&mut *left, &mut *right),
            };
            if let Some(fm) = voice.fm.as_mut() {
                let (mut gl, mut gr) = voice.gain;
                let (tl, tr) = voice.target;
                let steps = voice.ramp.min(ramp_len).max(1);
                let (dgl, dgr) = ((tl - gl) / steps as f32, (tr - gr) / steps as f32);
                let mut ramp = voice.ramp;
                for i in 0..n {
                    if ramp > 0 {
                        gl += dgl;
                        gr += dgr;
                        ramp -= 1;
                        if ramp == 0 {
                            gl = tl;
                            gr = tr;
                        }
                    }
                    let x = fm.next() * FM_LEVEL;
                    dl[i] += x * gl;
                    dr[i] += x * gr;
                }
                voice.gain = (gl, gr);
                voice.ramp = ramp;
                if (voice.dying && ramp == 0) || fm.done() {
                    if vi < foreground && !voice.dying {
                        voice.ended = true;
                    } else {
                        voice.active = false;
                    }
                }
                continue;
            }
            let mut stereo = voice.wave.channels == 2;
            let (mut gl, mut gr) = voice.gain;
            let (tl, tr) = voice.target;
            let steps = voice.ramp.min(ramp_len).max(1);
            let (dgl, dgr) = ((tl - gl) / steps as f32, (tr - gr) / steps as f32);
            let mut ramp = voice.ramp;
            for i in 0..n {
                if ramp > 0 {
                    gl += dgl;
                    gr += dgr;
                    ramp -= 1;
                    if ramp == 0 {
                        gl = tl;
                        gr = tr;
                    }
                }
                // Where it is, held to its loop.
                let lp = voice.live_loop();
                if lp.on() {
                    let (s, e) = (f64::from(lp.start), f64::from(lp.end));
                    if lp.kind == LoopKind::PingPong {
                        if !voice.backwards && voice.pos >= e {
                            voice.pos = (e - (voice.pos - e)).max(s);
                            voice.backwards = true;
                        } else if voice.backwards && voice.pos < s {
                            voice.pos = (s + (s - voice.pos)).min(e - 0.001);
                            voice.backwards = false;
                        }
                    } else if voice.pos >= e {
                        // A Musicline channel's next loop, set while this one played.
                        if voice.ml_next_set {
                            voice.ml_next_set = false;
                            let next = std::mem::take(&mut voice.ml_next);
                            set_ml_loop(voice, &[], voice.ml_next_region);
                            Arc::make_mut(&mut voice.wave).pcm.extend_from_slice(&next);
                            let at = voice.ml_play.min(frames) as u32;
                            voice.looped = if next.is_empty() { Loop::default() } else { Loop::forward(at, at + next.len() as u32) };
                            voice.ml_next = next;
                            frames = voice.wave.frames();
                            if !voice.looped.on() {
                                voice.ended = true;
                                break;
                            }
                            voice.pos = f64::from(voice.looped.start) + (voice.pos - e).min(f64::from(voice.looped.end - voice.looped.start) - 0.001);
                        } else if let Some(ns) = voice.swap.take().and_then(|k| module.samples.get(k).map(|s| (k, s))) {
                            let (k, smp) = ns;
                            voice.wave = smp.wave.clone();
                            voice.sample = k;
                            voice.looped = smp.looped;
                            voice.sustain = smp.sustain;
                            frames = voice.wave.frames();
                            stereo = voice.wave.channels == 2;
                            if smp.looped.on() && frames > 0 {
                                voice.pos = f64::from(smp.looped.start);
                            } else {
                                voice.ended = true;
                                break;
                            }
                        } else {
                            voice.pos = s + (voice.pos - e) % (e - s);
                        }
                    } else if voice.backwards && voice.pos < s {
                        voice.pos = e - (s - voice.pos) % (e - s);
                    }
                }
                // Interpolation still hears the last frame a frame past the end.
                let tail = if interp == Interp::None { 0.0 } else { 1.0 };
                // A filter rings on after its sample ends, to silence: so
                // where a sample's trailing silence ends isn't heard.
                let ringing = voice.filter.is_some() && voice.pos < frames as f64 + 65_536.0 && voice.fmem.iter().any(|m| m.0.abs() > 1e-6 || m.1.abs() > 1e-6);
                if (voice.pos >= frames as f64 + tail && !ringing) || voice.pos < 0.0 {
                    if vi < foreground {
                        voice.ended = true;
                    } else {
                        voice.active = false;
                    }
                    break;
                }
                let ip = voice.pos as usize;
                let frac = (voice.pos - ip as f64) as f32;
                let channel = |c: usize| -> f32 {
                    match interp {
                        Interp::None => voice.point(ip, c, frames),
                        Interp::Linear => {
                            let s1 = voice.point(ip, c, frames);
                            let s2 = voice.point(ip + 1, c, frames);
                            s1 + (s2 - s1) * frac
                        }
                        Interp::Cubic => {
                            let s1 = voice.point(ip, c, frames);
                            let s2 = voice.point(ip + 1, c, frames);
                            let s0 = if ip > 0 { voice.point(ip - 1, c, frames) } else { s1 };
                            let s3 = voice.point(ip + 2, c, frames);
                            // Catmull-Rom.
                            let a = -0.5 * s0 + 1.5 * s1 - 1.5 * s2 + 0.5 * s3;
                            let b = s0 - 2.5 * s1 + 2.0 * s2 - 0.5 * s3;
                            let cc = -0.5 * s0 + 0.5 * s2;
                            ((a * frac + b) * frac + cc) * frac + s1
                        }
                        Interp::Sinc => {
                            let phase = ((frac * SINC_PHASES as f32) as usize).min(SINC_PHASES);
                            let taps = &sinc[phase * SINC_TAPS..phase * SINC_TAPS + SINC_TAPS];
                            let mut acc = 0.0;
                            for (k, w) in taps.iter().enumerate() {
                                let at = ip as isize + k as isize - (SINC_TAPS as isize / 2 - 1);
                                if at >= 0 {
                                    acc += w * voice.point(at as usize, c, frames);
                                }
                            }
                            acc
                        }
                    }
                };
                let (mut l, mut r) = if stereo {
                    (channel(0), channel(1))
                } else {
                    let v = channel(0);
                    (v, v)
                };
                if let Some((a0, b1, b2)) = voice.filter {
                    let hp = voice.highpass;
                    let [ref mut fl, ref mut fr] = voice.fmem;
                    let yl = a0 * l + b1 * fl.0 + b2 * fl.1;
                    *fl = (yl.clamp(-4.0, 4.0), fl.0);
                    l = if hp { l - yl } else { yl };
                    let yr = a0 * r + b1 * fr.0 + b2 * fr.1;
                    *fr = (yr.clamp(-4.0, 4.0), fr.0);
                    r = if hp { r - yr } else { yr };
                }
                dl[i] += l * gl;
                dr[i] += r * gr;
                if voice.backwards {
                    voice.pos -= voice.step;
                } else {
                    voice.pos += voice.step;
                }
            }
            voice.gain = (gl, gr);
            voice.ramp = ramp;
            if voice.dying && ramp == 0 {
                voice.active = false;
            }
        }
    }
}

/// A synth sound playing: where its sequences or scripts are.
#[derive(Clone, Copy, Debug)]
enum ProgramState {
    Med(SoundState),
    Puma(PumaState),
    /// A JamCracker AM wave: the wave its channel has it play.
    Jam(u8),
}

impl ProgramState {
    fn new(sound: &Sound) -> ProgramState {
        match sound {
            Sound::Med(s) => ProgramState::Med(SoundState::new(s)),
            Sound::Puma(_) => ProgramState::Puma(PumaState::new()),
            Sound::Jam => ProgramState::Jam(0),
        }
    }

    fn tick(&mut self, sound: &Sound) -> Step {
        match (self, sound) {
            (ProgramState::Med(st), Sound::Med(s)) => st.tick(s),
            (ProgramState::Puma(st), Sound::Puma(s)) => st.tick(s),
            (ProgramState::Jam(w), Sound::Jam) => Step { wave: *w, volume: 64, ..Step::default() },
            _ => Step::default(),
        }
    }
}

/// A synth sound's waveform for this tick on its voice: a looped cycle
/// (a hybrid's sample, waveform 0, with the sample's own loop).
fn set_program_wave(v: &mut Voice, smp: &Sample) {
    let Some((prog, _)) = &v.program else { return };
    let w = usize::from(v.program_step.wave);
    let Some(wave) = prog.waves.get(w) else { return };
    if Arc::ptr_eq(wave, &v.wave) {
        return;
    }
    let hybrid_sample = w == 0 && prog.hybrid();
    v.looped = if hybrid_sample { smp.looped } else { prog.loops.get(w).copied().unwrap_or(Loop::forward(0, wave.frames() as u32)) };
    v.wave = wave.clone();
    if !hybrid_sample || v.pos >= wave.frames() as f64 {
        v.pos %= wave.frames().max(1) as f64;
    }
}

/// A Musicline channel's next loop: waiting for this pass of the loop
/// to end, as the Amiga's channel takes a new pointer and length; at
/// once when it isn't looping.
fn queue_ml_loop(v: &mut Voice, b: &[i8], r: Option<Region>) {
    if !v.looped.on() {
        set_ml_loop(v, b, r);
        return;
    }
    v.ml_next.clear();
    v.ml_next.extend(b.iter().map(|&x| i16::from(x) << 8));
    v.ml_next_set = true;
    v.ml_next_region = r;
}

/// A Musicline channel's loop put in place of the one it had (the
/// replayer's moving loop, or silence when it stops).
fn set_ml_loop(v: &mut Voice, b: &[i8], r: Option<Region>) {
    let at = v.ml_play.min(v.wave.frames());
    let w = Arc::make_mut(&mut v.wave);
    w.pcm.truncate(at);
    w.pcm.extend(b.iter().map(|&x| i16::from(x) << 8));
    v.looped = if b.is_empty() { Loop::default() } else { Loop::forward(at as u32, (at + b.len()) as u32) };
    v.ml_loop = r;
}

/// The windowed sinc's taps (a Blackman window over eight points), for
/// `SINC_PHASES + 1` places between two points.
fn sinc_table() -> Vec<f32> {
    let mut t = Vec::with_capacity((SINC_PHASES + 1) * SINC_TAPS);
    let half = SINC_TAPS as f64 / 2.0;
    for p in 0..=SINC_PHASES {
        let frac = p as f64 / SINC_PHASES as f64;
        let mut row = [0f64; SINC_TAPS];
        for (k, w) in row.iter_mut().enumerate() {
            // The tap's distance from the point read.
            let x = k as f64 - (half - 1.0) - frac;
            let sinc = if x.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * x * 0.97).sin() / (std::f64::consts::PI * x) };
            let n = (x + half) / SINC_TAPS as f64;
            let win = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * n).cos() + 0.08 * (4.0 * std::f64::consts::PI * n).cos();
            *w = sinc * win.max(0.0);
        }
        let sum: f64 = row.iter().sum();
        t.extend(row.iter().map(|w| (w / sum) as f32));
    }
    t
}

/// IT's resonant low-pass: cutoff and resonance 0 to 127, as its
/// coefficients (a0, b1, b2); None when it's open.
fn filter(cutoff: Option<u8>, resonance: Option<u8>, rate: u32) -> Option<(f32, f32, f32)> {
    let cut = cutoff?;
    let res = resonance.unwrap_or(0);
    if cut >= 127 && res == 0 {
        return None;
    }
    // Frequency from 110 Hz up, doubling every 24 steps (IT's curve).
    let fc = (110.0 * 2f32.powf(0.25 + f32::from(cut) / 24.0)).min(rate as f32 / 2.0 - 1.0);
    let damp = 10f32.powf(-(f32::from(res) * 24.0 / 128.0) / 20.0);
    let w = 2.0 * std::f32::consts::PI * fc / rate as f32;
    let d = (1.0 - 2.0 * damp) * w;
    let d = d.clamp(0.0, 2.0);
    let e = 1.0 / (w * w);
    let a0 = 1.0 / (1.0 + d + e);
    let b1 = (d + 2.0 * e) * a0;
    let b2 = -e * a0;
    Some((a0, b1, b2))
}

/// What a macro's letters stand for.
struct MacroVars {
    z: u8,
    note: u8,
    velocity: u8,
    volume: u8,
    pan: u8,
    channel: u8,
}

/// A macro's text as bytes: hex pairs (0-9, A-F), and lowercase letters
/// for values (z the parameter, n the note, v the velocity, u the
/// volume, x the panning; c the channel, added into the status byte
/// written just before). Anything else is passed over. Returns how
/// many bytes it made.
fn eval_macro(text: &str, v: &MacroVars, out: &mut [u8; 16]) -> usize {
    let mut n = 0;
    let mut half: Option<u8> = None;
    for ch in text.bytes() {
        if n >= out.len() {
            break;
        }
        let value = match ch {
            b'0'..=b'9' | b'A'..=b'F' => {
                let d = (ch as char).to_digit(16).unwrap_or(0) as u8;
                match half.take() {
                    Some(h) => Some(h << 4 | d),
                    None => {
                        half = Some(d);
                        None
                    }
                }
            }
            b'z' => Some(v.z),
            b'n' => Some(v.note),
            b'v' => Some(v.velocity),
            b'u' => Some(v.volume),
            b'x' => Some(v.pan),
            b'c' => {
                if n > 0 {
                    out[n - 1] = (out[n - 1] & 0xF0) | v.channel;
                }
                None
            }
            _ => None,
        };
        if let Some(b) = value {
            out[n] = b;
            n += 1;
        }
    }
    n
}

/// The effects whose memory Scream Tracker 3 shares: all but the slide
/// to a note's and the ones that set a value.
fn st3_shared(fx: Fx) -> bool {
    matches!(fx, Fx::VolSlide | Fx::PortaDown | Fx::PortaUp | Fx::Vibrato | Fx::FineVibrato | Fx::Tremor | Fx::Arpeggio | Fx::VibratoVol | Fx::TonePortaVol | Fx::Retrig | Fx::Tremolo | Fx::S3mEx)
}

/// S3M's and IT's slide parameter on the first tick: DxF up x, DFy down
/// y finely (DFF is up 15).
fn fine_amount(p: u8) -> Option<i32> {
    let (x, y) = (p >> 4, p & 0x0F);
    if y == 0x0F && x != 0 {
        Some(i32::from(x))
    } else if x == 0x0F && y != 0 {
        Some(-i32::from(y))
    } else {
        None
    }
}

/// S3M's and IT's slide parameter on the other ticks: Dx0 up, D0y down.
fn coarse_amount(p: u8) -> Option<i32> {
    let (x, y) = (p >> 4, p & 0x0F);
    if y == 0 && x != 0 {
        Some(i32::from(x))
    } else if x == 0 && y != 0 {
        Some(-i32::from(y))
    } else if x == 0x0F || y == 0x0F {
        None
    } else {
        // Both set: FastTracker's way, up wins; ST3 slides down.
        Some(-i32::from(y))
    }
}

/// A whole song played once, as interleaved stereo at `rate`, at most
/// `most` seconds, with `tail` seconds for the last notes to ring.
pub fn render(module: Arc<Module>, rate: u32, most: f64, tail: f64) -> Vec<f32> {
    let mut p = Player::new(module, rate);
    p.repeat = false;
    let block = 1024;
    let (mut l, mut r) = (vec![0f32; block], vec![0f32; block]);
    let mut out = Vec::new();
    let limit = (most * f64::from(rate)) as usize;
    let mut after = None;
    while out.len() / 2 < limit {
        l.fill(0.0);
        r.fill(0.0);
        p.render(&mut l, &mut r);
        for i in 0..block {
            out.push(l[i]);
            out.push(r[i]);
        }
        if p.ended() {
            let a = after.get_or_insert(out.len() / 2);
            if out.len() / 2 >= *a + (tail * f64::from(rate)) as usize {
                break;
            }
        }
    }
    out
}

/// Each channel played on its own (OpenMPT's stems): a render a channel,
/// as `render` makes them, with the others muted. A muted channel's
/// stem is left empty.
pub fn render_stems(module: Arc<Module>, rate: u32, most: f64, tail: f64, progress: &mut dyn FnMut(usize)) -> Vec<Vec<f32>> {
    (0..module.channels.len())
        .map(|ch| {
            progress(ch);
            if module.channels[ch].muted {
                return Vec::new();
            }
            let mut m = (*module).clone();
            for (i, c) in m.channels.iter_mut().enumerate() {
                c.muted = i != ch;
            }
            render(Arc::new(m), rate, most, tail)
        })
        .collect()
}

/// How long the song plays once through, in seconds (at most `most`),
/// worked out tick by tick without making its sound.
pub fn length(module: Arc<Module>, most: f64) -> f64 {
    if let Some(song) = &module.ptk {
        return PtkMachine::length(song, most);
    }
    if let Some(t) = &module.tune {
        return TuneMachine::length(&t.0, module.sequence, most);
    }
    let rate = 44_100;
    let mut p = Player::new(module, rate);
    p.repeat = false;
    let mut t = 0.0;
    while !p.ended() && t < most {
        p.process_tick();
        t += p.tick_seconds();
    }
    t.min(most)
}

/// The player as SEL's host: its channels read and set by a script.
struct SelHost<'a> {
    p: &'a mut Player,
    n: usize,
}

impl SelHost<'_> {
    fn voice(&self, t: usize) -> Option<&Voice> {
        let v = &self.p.voices[self.p.chans[t].voice];
        v.active.then_some(v)
    }
}

impl sel::Host for SelHost<'_> {
    fn tracks(&self) -> usize {
        self.n
    }
    fn pitch(&self, t: usize) -> i32 {
        // Eighths of a semitone above C-4, from the linear period (64ths of a semitone below C-5's).
        (((LINEAR_C5 - self.p.chans[t].period) / 64.0 + 12.0) * 8.0).round() as i32
    }
    fn set_pitch(&mut self, t: usize, pitch: i32) {
        let period = LINEAR_C5 - (f64::from(pitch) / 8.0 - 12.0) * 64.0;
        let c = &mut self.p.chans[t];
        c.period = period;
        c.target = period;
    }
    fn volume(&self, t: usize) -> u8 {
        self.p.chans[t].chan_volume.clamp(0, 64) as u8
    }
    fn set_volume(&mut self, t: usize, v: u8) {
        self.p.chans[t].chan_volume = i32::from(v.min(64));
    }
    fn set_sample(&mut self, t: usize, s: u8) {
        self.p.chans[t].instr = s;
        let vi = self.p.chans[t].voice;
        let k = usize::from(s).wrapping_sub(1);
        if self.p.voices[vi].active && k < self.p.module.samples.len() {
            self.p.voices[vi].swap = Some(k);
        }
    }
    fn play(&mut self, t: usize, from: u32) {
        let c = &self.p.chans[t];
        let (period, note) = (c.period, if c.note == 0 { NOTE_MIDDLE_C } else { c.note });
        self.p.trigger(t, note, &Cell::default());
        let m = self.p.module.clone();
        let c = &mut self.p.chans[t];
        if let Some(smp) = c.sample.and_then(|s| m.samples.get(s)) {
            c.volume = i32::from(smp.volume);
        }
        if period > 0.0 {
            c.period = period;
            c.target = period;
        }
        let vi = c.voice;
        self.p.voices[vi].pos = f64::from(from);
    }
    fn quiet(&mut self, t: usize) {
        self.p.cut(t);
    }
    fn position(&self, t: usize) -> u32 {
        self.voice(t).map_or(0, |v| v.pos as u32)
    }
    fn set_position(&mut self, t: usize, frames: u32) {
        let vi = self.p.chans[t].voice;
        let v = &mut self.p.voices[vi];
        if v.active {
            v.pos = f64::from(frames.min(v.wave.frames() as u32));
        }
    }
    fn loop_points(&self, t: usize) -> (u32, u32, u32) {
        self.voice(t).map_or((0, 0, 0), |v| (v.looped.start, v.looped.end, v.wave.frames() as u32))
    }
    fn set_loop(&mut self, t: usize, start: u32, end: u32) {
        let vi = self.p.chans[t].voice;
        let v = &mut self.p.voices[vi];
        if v.active {
            v.looped = if end > start { Loop::forward(start, end) } else { Loop::default() };
        }
    }
    fn copy(&mut self, t: usize, from: usize, what: u8) {
        if t == from || from >= self.n {
            return;
        }
        let all = what & 8 != 0;
        if what & 1 != 0 || all {
            let p = self.p.chans[from].period;
            self.p.chans[t].period = p;
            self.p.chans[t].target = p;
        }
        if what & 2 != 0 || all {
            self.p.chans[t].chan_volume = self.p.chans[from].chan_volume;
        }
        if what & 4 != 0 || all {
            let src = self.p.chans[from].clone();
            let c = &mut self.p.chans[t];
            (c.instr, c.sample, c.c5) = (src.instr, src.sample, src.c5);
            let (fv, tv) = (src.voice, self.p.chans[t].voice);
            if self.p.voices[fv].active {
                let mut v = self.p.voices[fv].clone();
                v.channel = t;
                self.p.voices[tv] = v;
            }
        }
    }
    fn set_global_volume(&mut self, v: u8) {
        self.p.global_volume = i32::from(v.min(64)) * 2;
    }
    fn set_tempo(&mut self, cia: u16) {
        self.p.tempo = (1_777_517.482 / f64::from(cia.max(1))).round().clamp(32.0, 999.0) as u32;
    }
    fn set_speed(&mut self, ticks: u8) {
        self.p.speed = u32::from(ticks.max(1));
    }
    fn jump(&mut self, measure: u16, row: u16) {
        if let Some(o) = self.p.module.orders.iter().position(|&p| p == measure) {
            self.p.jump = Some(o);
            self.p.brk = Some(usize::from(row));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-channel song: a square wave 64 frames long, looped, and the
    /// cells given from row 0.
    fn song(kind: Kind, cells: &[(usize, Cell)], rows: usize) -> Module {
        let mut m = Module::new(1);
        m.kind = kind;
        m.flags.linear = kind == Kind::Xm || kind == Kind::It;
        m.mix_volume = 128;
        let pcm: Vec<i16> = (0..64).map(|i| if i < 32 { 16000 } else { -16000 }).collect();
        m.samples = vec![Sample { wave: Arc::new(Wave::new(16, 1, pcm)), looped: Loop::forward(0, 64), ..Sample::empty() }];
        let mut p = Pattern::new(rows, 1);
        for &(row, c) in cells {
            p.cells[row] = c;
        }
        m.patterns = vec![Arc::new(p)];
        m.channels[0].pan = 128;
        m
    }

    fn note(n: u8) -> Cell {
        Cell { note: n, instr: 1, ..Default::default() }
    }

    /// Frames of left channel, `seconds` long, from the start.
    fn play(m: Module, seconds: f64) -> Vec<f32> {
        let mut p = Player::new(Arc::new(m), 44_100);
        p.repeat = false;
        p.interp = Interp::Linear;
        let n = (seconds * 44_100.0) as usize;
        let (mut l, mut r) = (vec![0f32; n], vec![0f32; n]);
        p.render(&mut l, &mut r);
        l
    }

    /// The pitch heard in a stretch: cycles between its first and last
    /// upward zero crossings, each placed between frames.
    fn hz(x: &[f32]) -> f64 {
        let times: Vec<f64> = x.windows(2).enumerate().filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0).map(|(i, w)| i as f64 + f64::from(-w[0] / (w[1] - w[0]))).collect();
        match (times.first(), times.last()) {
            (Some(a), Some(b)) if times.len() > 1 => (times.len() - 1) as f64 / ((b - a) / 44_100.0),
            _ => 0.0,
        }
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn middle_c_plays_at_the_samples_rate() {
        for kind in [Kind::Mod, Kind::Xm] {
            let out = play(song(kind, &[(0, note(NOTE_MIDDLE_C))], 64), 1.0);
            let f = hz(&out[4410..]);
            assert!((f - 8363.0 / 64.0).abs() < 2.0, "{kind:?}: {f}");
            // An octave up doubles it.
            let out = play(song(kind, &[(0, note(NOTE_MIDDLE_C + 12))], 64), 1.0);
            assert!((hz(&out[4410..]) - 2.0 * 8363.0 / 64.0).abs() < 3.0);
        }
    }

    #[test]
    fn an_fm_instrument_plays_its_note_and_rings_out_when_cut() {
        // A sine carrier held while keyed, released in a few tenths.
        let patch = FmPatch::from_s3m(&[0x21, 0x21, 0x3F, 0x00, 0xF0, 0xF0, 0x00, 0x05, 0, 0, 0x00]);
        let mut m = song(Kind::S3m, &[(0, note(NOTE_MIDDLE_C)), (16, Cell { note: NOTE_CUT, ..Default::default() })], 64);
        m.samples[0] = Sample { fm: Some(patch), ..Sample::empty() };
        let out = play(m.clone(), 3.0);
        let f = hz(&out[4410..44_100]);
        assert!((f - 261.63).abs() < 1.5, "{f}");
        // Cut on row 16 (0.32 s in a tempo-125 speed-6 row of 0.12 s... 1.92 s): it fades by its release, not at once.
        let cut = (16.0 * 0.12 * 44_100.0) as usize;
        assert!(rms(&out[cut + 200..cut + 1200]) > 0.01);
        assert!(rms(&out[out.len() - 2000..]) < 1e-3);
        // Volume takes its level down in the chip's steps: 32 is about 24 dB.
        let mut half = m;
        half.patterns = vec![Arc::new({
            let mut p = Pattern::new(64, 1);
            p.cells[0] = Cell { vol: Vol::Volume, volp: 32, ..note(NOTE_MIDDLE_C) };
            p
        })];
        let ratio = rms(&play(half, 1.0)[4410..]) / rms(&out[4410..44_100]);
        assert!((0.04..0.1).contains(&ratio), "{ratio}");
    }

    #[test]
    fn a_synth_sound_steps_its_waveforms_and_volumes() {
        // Two waveforms, a square then silence, a tick each, looping;
        // the volume halving after the first tick.
        let square: Vec<i8> = (0..32).map(|i| if i < 16 { 100 } else { -100 }).collect();
        let sound = SynthSound { waveforms: vec![square, vec![0; 32]], volumes: vec![64, 32, 0xFB], waves: vec![0, 1, 0xFE, 0], vol_speed: 1, wave_speed: 1, hybrid: false };
        let mut m = song(Kind::Mod, &[(0, note(NOTE_MIDDLE_C))], 64);
        m.samples[0] = Sample { program: Some(Arc::new(Program::new(sound, &Arc::default()))), ..Sample::empty() };
        let out = play(m, 0.2);
        let tick = (0.02 * 44_100.0) as usize;
        let first = rms(&out[100..tick - 100]);
        let second = rms(&out[tick + 100..2 * tick - 100]);
        let third = rms(&out[2 * tick + 100..3 * tick - 100]);
        assert!(first > 0.1, "{first}");
        assert!(second < 1e-3, "{second}");
        // The square again, at half the volume.
        assert!((third / first - 0.5).abs() < 0.1, "{third} {first}");
    }

    #[test]
    fn a_pattern_lasts_its_rows_at_speed_and_tempo() {
        // 64 rows at speed 6, tempo 125: 64 * 6 ticks of 20 ms.
        let audio = render(Arc::new(song(Kind::Mod, &[(0, note(61))], 64)), 44_100, 60.0, 0.0);
        let secs = audio.len() as f64 / 2.0 / 44_100.0;
        assert!((secs - 7.68).abs() < 0.05, "{secs}");
        // A break on row 15 ends it at 16 rows; speed 3 halves that.
        let mut cells = vec![(0, Cell { fx: Fx::Speed, param: 3, ..note(61) })];
        cells.push((15, Cell { fx: Fx::PatternBreak, param: 0, ..Default::default() }));
        let audio = render(Arc::new(song(Kind::Mod, &cells, 64)), 44_100, 60.0, 0.0);
        let secs = audio.len() as f64 / 2.0 / 44_100.0;
        assert!((secs - 16.0 * 3.0 * 0.02).abs() < 0.05, "{secs}");
    }

    #[test]
    fn volume_slides_and_cuts() {
        // A00-style slide down by 4 a tick for one row (5 ticks): 64 to 44.
        let out = play(song(Kind::Mod, &[(0, Cell { fx: Fx::VolSlide, param: 0x04, ..note(61) })], 64), 0.3);
        let early = rms(&out[100..800]);
        let late = rms(&out[6000..12000]);
        assert!(late < early * 0.8 && late > early * 0.5, "{early} {late}");
        // EC2: cut on tick 2.
        let out = play(song(Kind::Mod, &[(0, Cell { fx: Fx::ModEx, param: 0xC2, ..note(61) })], 64), 0.3);
        assert!(rms(&out[200..1500]) > 0.1);
        assert!(rms(&out[3000..10000]) < 1e-4);
        // A note off with no instruments stops the note.
        let out = play(song(Kind::Xm, &[(0, note(61)), (1, Cell { note: NOTE_OFF, ..Default::default() })], 64), 0.5);
        assert!(rms(&out[10000..20000]) < 1e-4);
    }

    #[test]
    fn arpeggio_plays_three_notes_in_turn() {
        // Slow ticks (tempo 32: 78 ms) and a high note, to count cycles in.
        let mut m = song(Kind::Mod, &[(0, Cell { fx: Fx::Arpeggio, param: 0x7C, ..note(73) })], 64);
        m.tempo = 32;
        let out = play(m, 0.5);
        let tick = (44_100.0 * 2.5 / 32.0) as usize;
        let at = |k: usize| hz(&out[k * tick + tick / 8..(k + 1) * tick - tick / 8]);
        let (base, fifth, octave) = (at(0), at(1), at(2));
        assert!(fifth / base > 1.42 && fifth / base < 1.58, "{base} {fifth}");
        assert!(octave / base > 1.9 && octave / base < 2.1, "{base} {octave}");
        assert!((at(3) - base).abs() < base * 0.05);
    }

    #[test]
    fn portamento_moves_the_pitch_and_tone_porta_reaches_its_note() {
        for kind in [Kind::Mod, Kind::Xm] {
            let out = play(song(kind, &[(0, Cell { fx: Fx::PortaUp, param: 0x04, ..note(73) })], 64), 0.12);
            let (start, end) = (hz(&out[100..880]), hz(&out[4410..5290]));
            assert!(end > start * 1.05, "{kind:?}: {start} {end}");
            let cells = [(0, note(61)), (1, Cell { note: 73, instr: 1, fx: Fx::TonePorta, param: 0xFF, ..Default::default() })];
            let out = play(song(kind, &cells, 64), 1.0);
            let f = hz(&out[15000..44000]);
            assert!((f - 2.0 * 8363.0 / 64.0).abs() < 4.0, "{kind:?}: {f}");
        }
    }

    #[test]
    fn the_song_comes_round_when_it_jumps_back() {
        let cells = [(0, note(61)), (7, Cell { fx: Fx::PositionJump, param: 0, ..Default::default() })];
        let mut p = Player::new(Arc::new(song(Kind::Mod, &cells, 64)), 44_100);
        let (mut l, mut r) = (vec![0f32; 44_100], vec![0f32; 44_100]);
        p.render(&mut l, &mut r);
        // 8 rows of 120 ms: it's come round once in a second.
        assert_eq!(p.position().looped, 1);
        assert!(!p.ended());
        p.repeat = false;
        p.render(&mut l, &mut r);
        assert!(p.ended());
    }

    #[test]
    fn a_jammed_note_sounds_while_the_song_is_stopped() {
        let mut p = Player::idle(Arc::new(song(Kind::It, &[], 64)), 44_100);
        p.jam(61, 1);
        let (mut l, mut r) = (vec![0f32; 4410], vec![0f32; 4410]);
        p.render(&mut l, &mut r);
        assert!(rms(&l[1000..]) > 0.05);
        assert!(!p.playing());
        p.jam_off();
        let (mut l, mut r) = (vec![0f32; 4410], vec![0f32; 4410]);
        p.render(&mut l, &mut r);
        assert!(rms(&l[2000..]) < 1e-4);
    }

    #[test]
    fn tempo_modes_and_swing_set_the_ticks_length() {
        let base = song(Kind::It, &[(0, note(61))], 16);
        let secs = |m: Module| length(Arc::new(m), 60.0);
        // Classic: 16 rows × 6 ticks × 20 ms.
        assert!((secs(base.clone()) - 1.92).abs() < 0.01);
        // Modern: 125 beats a minute, 4 rows a beat: 16 rows are 4 beats, 1.92 s, whatever the speed.
        let mut m = base.clone();
        m.tempo_mode = TempoMode::Modern;
        m.speed = 3;
        assert!((secs(m) - 1.92).abs() < 0.01);
        // Alternative: 125 ticks a second.
        let mut m = base.clone();
        m.tempo_mode = TempoMode::Alternative;
        assert!((secs(m) - 16.0 * 6.0 / 125.0).abs() < 0.01);
        // Swing keeps each pair of rows as long.
        let mut m = base;
        m.swing = 60;
        assert!((secs(m) - 1.92).abs() < 0.01);
    }

    #[test]
    fn a_muted_matrix_slot_is_silent_and_automation_moves_the_volume() {
        let mut m = song(Kind::It, &[(0, note(61))], 64);
        m.slot_mutes = vec![(0, 0)];
        let out = play(m, 0.3);
        assert!(rms(&out[2000..]) < 1e-4);
        // Global volume automated from full to nothing over the first 32 rows.
        let mut m = song(Kind::It, &[(0, note(61))], 64);
        let p = Arc::make_mut(&mut m.patterns[0]);
        p.automation.push(Lane { target: Some(Target::GlobalVolume), points: vec![(0, 1.0), (32, 0.0)] });
        let out = play(m, 5.0);
        let row = 44_100 * 6 / 50;
        let early = rms(&out[row..row * 2]);
        let mid = rms(&out[row * 16..row * 17]);
        let late = rms(&out[row * 33..row * 34]);
        assert!(early > mid * 1.5 && mid > late && late < 1e-3, "{early} {mid} {late}");
    }

    #[test]
    fn a_phrase_plays_its_rows_transposed_to_the_note() {
        let mut m = song(Kind::It, &[(0, note(61 + 12))], 64);
        m.flags.instruments = true;
        let mut ins = Instrument::for_sample("lead", 1);
        // C-5, nothing, G-5: as written from C-5, so a C-6 plays them an octave up.
        ins.phrase = Some(Phrase { cells: vec![Cell { note: 61, ..Default::default() }, Cell::default(), Cell { note: 68, ..Default::default() }], looped: false, base: 61 });
        m.instruments = vec![ins];
        m.tempo = 32;
        let out = play(m, 1.6);
        let row = (44_100.0 * 2.5 / 32.0) as usize * 6;
        let f0 = hz(&out[row / 4..row - row / 4]);
        let f2 = hz(&out[2 * row + row / 4..3 * row - row / 4]);
        assert!((f0 - 2.0 * 8363.0 / 64.0).abs() < 4.0, "{f0}");
        assert!((f2 / f0 - 2f64.powf(7.0 / 12.0)).abs() < 0.03, "{f0} {f2}");
    }

    #[test]
    fn interpolations_keep_the_pitch_and_the_none_one_steps() {
        for interp in [Interp::None, Interp::Linear, Interp::Cubic, Interp::Sinc] {
            let mut p = Player::new(Arc::new(song(Kind::Xm, &[(0, note(61))], 64)), 44_100);
            p.interp = interp;
            let (mut l, mut r) = (vec![0f32; 44_100], vec![0f32; 44_100]);
            p.render(&mut l, &mut r);
            let f = hz(&l[4410..]);
            assert!((f - 8363.0 / 64.0).abs() < 2.0, "{interp:?}: {f}");
            assert!(l.iter().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn a_macro_closes_the_filter_and_an_effect_slot_takes_the_channel() {
        // Z00: the cutoff all the way down, by IT's default macro.
        let loud = rms(&play(song(Kind::It, &[(0, note(85))], 64), 0.3)[2000..]);
        let shut = rms(&play(song(Kind::It, &[(0, Cell { fx: Fx::Midi, param: 0x00, ..note(85) })], 64), 0.3)[2000..]);
        assert!(shut < loud * 0.3, "{loud} {shut}");
        assert_eq!(eval_macro("F0F000z", &MacroVars { z: 0x40, note: 0, velocity: 0, volume: 0, pan: 0, channel: 0 }, &mut [0; 16]), 4);
        // A channel through a delay with only its echo heard is silent at first.
        let mut m = song(Kind::It, &[(0, note(61))], 64);
        let mut slot = EffectSlot::new(EffectKind::Delay);
        slot.mix = 1.0;
        slot.params[0] = 1.0;
        slot.params[1] = 1.0;
        m.effects = vec![slot];
        m.channels[0].effect = Some(0);
        let out = play(m, 0.5);
        assert!(rms(&out[..10_000]) < 1e-4);
    }

    #[test]
    fn play_row_lets_go_and_midi_instruments_send_their_notes() {
        let mut m = song(Kind::It, &[(0, note(61)), (1, note(73))], 64);
        m.flags.instruments = true;
        let mut ins = Instrument::for_sample("lead", 1);
        ins.midi = Some(MidiOut { channel: 2, program: Some(5), bank: None });
        m.instruments = vec![ins];
        let mut p = Player::idle(Arc::new(m), 44_100);
        p.play_row(0, 0);
        let (mut l, mut r) = (vec![0f32; 44_100], vec![0f32; 44_100]);
        p.render(&mut l, &mut r);
        assert!(!p.playing());
        let mut sent = Vec::new();
        p.take_midi(&mut |b| sent.push(b.to_vec()));
        assert_eq!(sent, vec![vec![0xC2, 5], vec![0x92, 60, 127], vec![0x82, 60, 0]]);
    }

    #[test]
    fn a_release_node_goes_on_from_where_the_envelope_was() {
        let env = Envelope {
            enabled: true,
            points: vec![EnvPoint { tick: 0, value: 64 }, EnvPoint { tick: 10, value: 32 }, EnvPoint { tick: 20, value: 32 }, EnvPoint { tick: 30, value: 0 }],
            release: Some(2),
            ..Default::default()
        };
        let mut st = EnvState::default();
        st.reset(&env);
        for _ in 0..5 {
            st.step(&env, false, true);
        }
        // Let go at tick 5 (value 48): on from point 2 (32), 16 over it.
        assert_eq!(st.step(&env, true, true), Some(48));
        assert_eq!(st.tick, 21);
    }

    #[test]
    fn stems_add_up_to_the_song() {
        let mut m = song(Kind::It, &[(0, note(61))], 16);
        m.channels.push(Channel::default());
        let p = Arc::make_mut(&mut m.patterns[0]);
        *p = p.with_channels(2);
        *p.cell_mut(4, 1).unwrap() = note(68);
        let m = Arc::new(m);
        let whole = render(m.clone(), 22_050, 10.0, 0.0);
        let stems = render_stems(m, 22_050, 10.0, 0.0, &mut |_| {});
        assert_eq!(stems.len(), 2);
        let n = whole.len().min(stems[0].len()).min(stems[1].len());
        let worst = (0..n).map(|i| (whole[i] - stems[0][i] - stems[1][i]).abs()).fold(0f32, f32::max);
        assert!(worst < 1e-4, "{worst}");
    }
}
