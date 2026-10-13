//! **Synth sounds, OctaMED's way**: a handful of tiny single-cycle
//! waveforms, played one after another by a waveform sequence while a
//! volume sequence shapes the level, each a little program of values
//! and commands stepped once a tick. A hybrid sound is the same with a
//! sample as its waveform 0.
//!
//! From the MMD file format's description (Teijo Kinnunen, 1992) and
//! OctaMED's manual for what the commands do; where the two leave a
//! detail open (vibrato's depth scale, a command's place in a tick) the
//! choice is said where it's made.

/// A synth sound: its waveforms and its two sequences.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SynthSound {
    /// The waveforms, each one cycle of signed 8-bit points (a hybrid's
    /// first is a placeholder: its sample plays in its place).
    pub waveforms: Vec<Vec<i8>>,
    /// The volume sequence: 0 to 64 sets the volume; 0xF0 and up are
    /// commands (`VOL_*`).
    pub volumes: Vec<u8>,
    /// The waveform sequence: 0 to 63 picks a waveform; 0xF0 and up are
    /// commands (`WAVE_*`).
    pub waves: Vec<u8>,
    /// Ticks between steps of each sequence (1 every tick).
    pub vol_speed: u8,
    pub wave_speed: u8,
    /// Its waveform 0 is the instrument's sample.
    pub hybrid: bool,
}

// The volume sequence's commands.
pub const VOL_SPD: u8 = 0xF0;
pub const VOL_WAI: u8 = 0xF1;
pub const VOL_CHD: u8 = 0xF2;
pub const VOL_CHU: u8 = 0xF3;
/// A waveform as the volume's envelope, once (EN1) or looping (EN2).
pub const VOL_EN1: u8 = 0xF4;
pub const VOL_EN2: u8 = 0xF5;
pub const VOL_JWS: u8 = 0xFA;
pub const VOL_HLT: u8 = 0xFB;
pub const VOL_JMP: u8 = 0xFE;
pub const VOL_END: u8 = 0xFF;

// The waveform sequence's.
pub const WAVE_SPD: u8 = 0xF0;
pub const WAVE_WAI: u8 = 0xF1;
/// The pitch going down (the period up) or up, by the value a tick.
pub const WAVE_CHD: u8 = 0xF2;
pub const WAVE_CHU: u8 = 0xF3;
pub const WAVE_VBD: u8 = 0xF4;
pub const WAVE_VBS: u8 = 0xF5;
/// The pitch's changes undone.
pub const WAVE_RES: u8 = 0xF6;
/// A waveform as vibrato's shape.
pub const WAVE_VWF: u8 = 0xF7;
pub const WAVE_JVS: u8 = 0xFA;
pub const WAVE_HLT: u8 = 0xFB;
/// An arpeggio: the semitones after it, up to ARE, one a tick, round and round.
pub const WAVE_ARP: u8 = 0xFC;
pub const WAVE_ARE: u8 = 0xFD;
pub const WAVE_JMP: u8 = 0xFE;
pub const WAVE_END: u8 = 0xFF;

/// What a sound is doing this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Step {
    /// The waveform playing.
    pub wave: u8,
    /// 0 to 64, over the note's own volume.
    pub volume: u8,
    /// Added to the note's Amiga period (positive is lower).
    pub period: i32,
    /// Semitones over the note (the arpeggio's).
    pub arp: i8,
}

/// A sequence's place.
#[derive(Clone, Copy, Debug, Default)]
struct Run {
    pos: usize,
    speed: u8,
    count: u8,
    wait: u8,
    halted: bool,
}

impl Run {
    fn new(speed: u8) -> Run {
        Run { speed: speed.max(1), count: 1, ..Default::default() }
    }

    /// Whether it steps this tick.
    fn due(&mut self) -> bool {
        if self.halted {
            return false;
        }
        if self.wait > 0 {
            self.wait -= 1;
            return false;
        }
        self.count = self.count.saturating_sub(1);
        self.count == 0
    }
}

/// A synth sound playing a note.
#[derive(Clone, Copy, Debug, Default)]
pub struct SoundState {
    vol: Run,
    wave_run: Run,
    volume: i32,
    vol_change: i32,
    /// An envelope from a waveform: (waveform, looping, place).
    env: Option<(u8, bool, usize)>,
    wave: u8,
    period: i32,
    period_change: i32,
    vib_depth: u8,
    vib_speed: u8,
    vib_wave: Option<u8>,
    vib_pos: u8,
    /// The arpeggio's first entry, and how far through it is.
    arp: Option<(usize, usize)>,
    arp_now: i8,
}

/// More commands than this in one step is a loop with nothing in it.
const MOST_COMMANDS: usize = 256;

impl SoundState {
    /// A note starts: both sequences from the top.
    pub fn new(s: &SynthSound) -> SoundState {
        SoundState { vol: Run::new(s.vol_speed), wave_run: Run::new(s.wave_speed), volume: 64, ..Default::default() }
    }

    /// A tick on.
    pub fn tick(&mut self, s: &SynthSound) -> Step {
        // A step's commands may set the speed: the count starts after them.
        if self.vol.due() {
            self.run_volume(s);
            self.vol.count = self.vol.speed;
        }
        if self.wave_run.due() {
            self.run_waves(s);
            self.wave_run.count = self.wave_run.speed;
        }
        // What runs every tick: the slides, the envelope, vibrato, the arpeggio.
        self.volume = (self.volume + self.vol_change).clamp(0, 64);
        if let Some((w, looped, at)) = self.env {
            match s.waveforms.get(usize::from(w)).filter(|w| !w.is_empty()) {
                Some(wf) if at < wf.len() => {
                    self.volume = (i32::from(wf[at]) + 128) >> 2;
                    let next = at + 1;
                    self.env = if next < wf.len() { Some((w, looped, next)) } else if looped { Some((w, looped, 0)) } else { None };
                }
                _ => self.env = None,
            }
        }
        self.period += self.period_change;
        let vib = if self.vib_depth > 0 {
            self.vib_pos = self.vib_pos.wrapping_add(self.vib_speed);
            // The shape's point (-128 to 127) times the depth over 64:
            // a depth of 64 swings the period by a whole 128.
            let shape = match self.vib_wave.and_then(|w| s.waveforms.get(usize::from(w))).filter(|w| !w.is_empty()) {
                Some(wf) => i32::from(wf[usize::from(self.vib_pos) * wf.len() / 256]),
                None => ((f64::from(self.vib_pos) / 256.0 * std::f64::consts::TAU).sin() * 127.0) as i32,
            };
            shape * i32::from(self.vib_depth) / 64
        } else {
            0
        };
        if let Some((start, i)) = self.arp {
            match s.waves.get(start + i) {
                Some(&v) if v != WAVE_ARE && start + i < s.waves.len() => {
                    self.arp_now = v as i8;
                    self.arp = Some((start, i + 1));
                }
                _ => {
                    self.arp_now = s.waves.get(start).filter(|&&v| v != WAVE_ARE).map_or(0, |&v| v as i8);
                    self.arp = Some((start, 1));
                }
            }
        }
        Step { wave: self.wave, volume: self.volume as u8, period: self.period + vib, arp: self.arp_now }
    }

    fn run_volume(&mut self, s: &SynthSound) {
        let t = &s.volumes;
        let arg = |p: usize| t.get(p + 1).copied().unwrap_or(0);
        for _ in 0..MOST_COMMANDS {
            let p = self.vol.pos;
            let Some(&v) = t.get(p) else {
                self.vol.halted = true;
                return;
            };
            match v {
                0..=64 => {
                    self.volume = i32::from(v);
                    self.vol_change = 0;
                    self.vol.pos += 1;
                    return;
                }
                VOL_SPD => {
                    self.vol.speed = arg(p).max(1);
                    self.vol.pos += 2;
                }
                VOL_WAI => {
                    self.vol.wait = arg(p);
                    self.vol.pos += 2;
                    return;
                }
                VOL_CHD => {
                    self.vol_change = -i32::from(arg(p));
                    self.vol.pos += 2;
                }
                VOL_CHU => {
                    self.vol_change = i32::from(arg(p));
                    self.vol.pos += 2;
                }
                VOL_EN1 | VOL_EN2 => {
                    self.env = Some((arg(p), v == VOL_EN2, 0));
                    self.vol.pos += 2;
                }
                VOL_JWS => {
                    self.wave_run.pos = usize::from(arg(p));
                    self.wave_run.halted = false;
                    self.vol.pos += 2;
                }
                VOL_JMP => self.vol.pos = usize::from(arg(p)),
                // HLT, END and anything unknown: it stops here.
                _ => {
                    self.vol.halted = true;
                    return;
                }
            }
        }
        self.vol.halted = true;
    }

    fn run_waves(&mut self, s: &SynthSound) {
        let t = &s.waves;
        let arg = |p: usize| t.get(p + 1).copied().unwrap_or(0);
        for _ in 0..MOST_COMMANDS {
            let p = self.wave_run.pos;
            let Some(&v) = t.get(p) else {
                self.wave_run.halted = true;
                return;
            };
            match v {
                0..=0x3F => {
                    if usize::from(v) < s.waveforms.len() {
                        self.wave = v;
                    }
                    self.wave_run.pos += 1;
                    return;
                }
                WAVE_SPD => {
                    self.wave_run.speed = arg(p).max(1);
                    self.wave_run.pos += 2;
                }
                WAVE_WAI => {
                    self.wave_run.wait = arg(p);
                    self.wave_run.pos += 2;
                    return;
                }
                WAVE_CHD => {
                    self.period_change = i32::from(arg(p));
                    self.wave_run.pos += 2;
                }
                WAVE_CHU => {
                    self.period_change = -i32::from(arg(p));
                    self.wave_run.pos += 2;
                }
                WAVE_VBD => {
                    self.vib_depth = arg(p);
                    self.wave_run.pos += 2;
                }
                WAVE_VBS => {
                    self.vib_speed = arg(p);
                    self.wave_run.pos += 2;
                }
                WAVE_RES => {
                    self.period = 0;
                    self.period_change = 0;
                    self.wave_run.pos += 1;
                }
                WAVE_VWF => {
                    self.vib_wave = Some(arg(p));
                    self.wave_run.pos += 2;
                }
                WAVE_JVS => {
                    self.vol.pos = usize::from(arg(p));
                    self.vol.halted = false;
                    self.wave_run.pos += 2;
                }
                WAVE_ARP => {
                    // The notes follow, to ARE; the sequence goes on after it.
                    let start = p + 1;
                    let end = t[start.min(t.len())..].iter().position(|&x| x == WAVE_ARE).map_or(t.len(), |i| start + i);
                    self.arp = Some((start, 0));
                    self.wave_run.pos = end + 1;
                }
                WAVE_ARE => self.wave_run.pos += 1,
                WAVE_JMP => self.wave_run.pos = usize::from(arg(p)),
                _ => {
                    self.wave_run.halted = true;
                    return;
                }
            }
        }
        self.wave_run.halted = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sound(volumes: &[u8], waves: &[u8]) -> SynthSound {
        SynthSound { waveforms: vec![vec![0; 32]; 4], volumes: volumes.to_vec(), waves: waves.to_vec(), vol_speed: 1, wave_speed: 1, hybrid: false }
    }

    #[test]
    fn the_sequences_step_their_values_and_halt() {
        let s = sound(&[64, 48, 32, VOL_HLT], &[0, 1, 2, WAVE_END]);
        let mut st = SoundState::new(&s);
        let steps: Vec<(u8, u8)> = (0..5).map(|_| st.tick(&s)).map(|x| (x.volume, x.wave)).collect();
        assert_eq!(steps, [(64, 0), (48, 1), (32, 2), (32, 2), (32, 2)]);
    }

    #[test]
    fn waits_slides_jumps_and_speed() {
        // Down by 8 a tick from 64, after two ticks' wait.
        let s = sound(&[64, VOL_WAI, 2, VOL_CHD, 8, VOL_HLT], &[VOL_SPD, 2, 0, 1, WAVE_JMP, 2]);
        let mut st = SoundState::new(&s);
        let v: Vec<u8> = (0..6).map(|_| st.tick(&s).volume).collect();
        assert_eq!(v, [64, 64, 64, 64, 56, 48]);
        let s2 = sound(&[64], &[WAVE_SPD, 2, 0, 1, WAVE_JMP, 2]);
        let mut st = SoundState::new(&s2);
        let w: Vec<u8> = (0..6).map(|_| st.tick(&s2).wave).collect();
        assert_eq!(w, [0, 0, 1, 1, 0, 0]);
    }

    #[test]
    fn an_arpeggio_goes_round_while_the_sequence_goes_on() {
        let s = sound(&[64], &[WAVE_ARP, 0, 4, 7, WAVE_ARE, 3, WAVE_HLT]);
        let mut st = SoundState::new(&s);
        let a: Vec<(i8, u8)> = (0..5).map(|_| st.tick(&s)).map(|x| (x.arp, x.wave)).collect();
        assert_eq!(a, [(0, 3), (4, 3), (7, 3), (0, 3), (4, 3)]);
    }

    #[test]
    fn a_loop_of_commands_halts_rather_than_hangs() {
        let s = sound(&[VOL_JMP, 0], &[WAVE_JMP, 0]);
        let mut st = SoundState::new(&s);
        assert_eq!(st.tick(&s).volume, 64);
    }
}
