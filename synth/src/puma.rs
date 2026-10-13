//! **Pumatracker's instruments**: two little scripts of four-byte
//! commands run side by side, one a tick. The volume script picks the
//! waveform (one of the song's ten samples or the 42 short cycles the
//! replayer has built in, or a run of those cycles stepped through back
//! and forth) and ramps the volume; the pitch script sets the note's
//! pitch in semitones or ramps its Amiga period.
//!
//! From the format as the community documented it (the commands, their
//! bytes, the built-in cycles), checked against OpenMPT's reading of it;
//! the choices it leaves open are said where they're made.

use crate::sequence::Step;

/// One of a script's commands: the command byte and its three.
pub type Command = [u8; 4];

/// A Pumatracker instrument.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PumaSound {
    /// C0 sets the waveform (a, and if a is a built-in cycle, steps
    /// of b through c more of them, back and forth, a tick each); A0
    /// ramps the volume from a to b over c ticks; B0 jumps to entry a / 4;
    /// E0 stops the voice.
    pub volume: Vec<Command>,
    /// D0 sets the pitch a / 2 semitones up for c ticks; A0 ramps the
    /// Amiga period's offset from a to b over c ticks; B0 jumps; E0 ends.
    pub pitch: Vec<Command>,
}

/// The song's own samples come first: waveform numbers from here are
/// the built-in cycles.
pub const SAMPLES: u8 = 10;

/// The cycles Pumatracker's replayer has built in, 32 signed points each.
pub fn built_in() -> Vec<[i8; 32]> {
    let mut out = Vec::with_capacity(42);
    // 0 to 15: a ramp down and up, its second half's start sloped further down.
    let rest = [0x3F, 0x37, 0x2F, 0x27, 0x1F, 0x17, 0x0F, 0x07, 0xFF, 0x07, 0x0F, 0x17, 0x1F, 0x27, 0x2F, 0x37u8];
    let slope = [0xC0, 0xB8, 0xB0, 0xA8, 0xA0, 0x98, 0x90, 0x88, 0x80, 0x88, 0x90, 0x98, 0xA0, 0xA8, 0xB0u8];
    let head = [0xC0, 0xC0, 0xD0, 0xD8, 0xE0, 0xE8, 0xF0, 0xF8, 0x00, 0xF8, 0xF0, 0xE8, 0xE0, 0xD8, 0xD0, 0xC8u8];
    for k in 0..16 {
        let mut w = [0u8; 32];
        w[..16].copy_from_slice(&head);
        for i in 0..16 {
            w[16 + i] = if i < k { slope[i] } else { rest[i] };
        }
        out.push(w);
    }
    // 16 to 31: pulses, low for 16 points and more.
    for k in 16..32 {
        let low = if k < 30 { 0x81 } else { 0x80 };
        out.push(std::array::from_fn(|i| if i < k { low } else { 0x7F }));
    }
    // 32 to 35: two pulses a cycle, narrowing.
    for (a, b) in [(8, 7), (6, 5), (4, 3), (2, 2)] {
        out.push(std::array::from_fn(|i| if (i < 16 && i < a) || (i >= 16 && i - 16 < b) { 0x80 } else { 0x7F }));
    }
    let saw: [u8; 32] = std::array::from_fn(|i| match i {
        0 | 1 => 0x80,
        31 => 0x7F,
        _ => (0x80 + i * 8) as u8,
    });
    let saw16: [u8; 16] = std::array::from_fn(|i| match i {
        0 | 1 => 0x80,
        _ => (0x80 + i * 16) as u8,
    });
    let bell = [0x45, 0x45, 0x79, 0x7D, 0x7A, 0x77, 0x70, 0x66u8];
    let tri = [0x00, 0x00, 0x40, 0x60, 0x7F, 0x60, 0x40, 0x20, 0x00, 0xE0, 0xC0, 0xA0, 0x80, 0xA0, 0xC0, 0xE0u8];
    let cat = |a: &[u8], b: &[u8]| -> [u8; 32] { std::array::from_fn(|i| if i < 16 { a[i] } else { b[i - 16] }) };
    // 36: a saw. 37 to 39: the replayer's three odd shapes.
    out.push(saw);
    out.push(cat(&saw16, &[&bell[..], &[0x61, 0x58, 0x53, 0x4D, 0x2C, 0x20, 0x18, 0x12]].concat()));
    out.push(cat(&[0x04, 0xDB, 0xD3, 0xCD, 0xC6, 0xBC, 0xB5, 0xAE, 0xA8, 0xA3, 0x9D, 0x99, 0x93, 0x8E, 0x8B, 0x8A], &[&bell[..], &[0x5B, 0x4B, 0x43, 0x37, 0x2C, 0x20, 0x18, 0x12]].concat()));
    out.push(cat(&[0x04, 0xF8, 0xE8, 0xDB, 0xCF, 0xC6, 0xBE, 0xB0, 0xA8, 0xA4, 0x9E, 0x9A, 0x95, 0x94, 0x8D, 0x83], &tri));
    // 40, 41: halves of the others, put together.
    out.push(cat(&tri, &saw[..16]));
    out.push(cat(&saw[16..], &saw16));
    out.into_iter().map(|w| w.map(|b| b as i8)).collect()
}

/// A script's place.
#[derive(Clone, Copy, Debug)]
struct Run {
    /// The command running, and the next to read (None once stopped).
    current: usize,
    next: Option<usize>,
    /// Ticks the running command has left.
    remain: u16,
}

impl Default for Run {
    fn default() -> Run {
        Run { current: 0, next: Some(0), remain: 0 }
    }
}

/// A Pumatracker instrument playing a note.
#[derive(Clone, Copy, Debug, Default)]
pub struct PumaState {
    vol: Run,
    pitch: Run,
    volume: i32,
    wave: u8,
    first_wave: u8,
    last_wave: u8,
    wave_step: i8,
    semitones: i8,
    period: i32,
}

/// a × b / c, rounded.
fn muldivr(a: i32, b: i32, c: i32) -> i32 {
    (f64::from(a) * f64::from(b) / f64::from(c)).round() as i32
}

/// More jumps than this in a tick is a loop with nothing in it.
const MOST_JUMPS: usize = 16;

impl PumaState {
    /// A note starts: both scripts from the top, at full volume.
    pub fn new() -> PumaState {
        PumaState { volume: 64, ..Default::default() }
    }

    /// A tick on.
    pub fn tick(&mut self, s: &PumaSound) -> Step {
        // A run of built-in cycles steps first, bouncing at its ends.
        if self.wave_step != 0 {
            let w = (i32::from(self.wave) + i32::from(self.wave_step)).clamp(i32::from(self.first_wave), i32::from(self.last_wave));
            self.wave = w as u8;
            if self.wave <= self.first_wave || self.wave >= self.last_wave {
                self.wave_step = -self.wave_step;
            }
        }
        self.step(s, true);
        self.step(s, false);
        Step { wave: self.wave, volume: self.volume.clamp(0, 64) as u8, period: self.period, arp: self.semitones }
    }

    fn step(&mut self, s: &PumaSound, volume: bool) {
        let script = if volume { &s.volume } else { &s.pitch };
        let mut run = if volume { self.vol } else { self.pitch };
        if run.remain > 0 {
            // A ramp carries on, from its start to its end.
            if let Some(&[c, a, b, n]) = script.get(run.current) {
                if c == 0xA0 && n > 0 {
                    let (a, b) = if volume { (i32::from(a.min(64)), i32::from(b.min(64))) } else { (i32::from(a as i8), i32::from(b as i8)) };
                    let v = b + muldivr(a - b, i32::from(run.remain), i32::from(n));
                    if volume {
                        self.volume = v;
                    } else {
                        self.period = v;
                    }
                }
            }
            run.remain -= 1;
        } else {
            let mut jumps = 0;
            while let Some(at) = run.next {
                let Some(&[c, a, b, n]) = script.get(at) else {
                    run.next = None;
                    break;
                };
                run.current = at;
                run.next = Some(at + 1);
                let took = match (c, volume) {
                    (0xC0, true) => {
                        self.wave = a;
                        self.first_wave = a;
                        // Only the built-in cycles step; the samples stay.
                        if a < SAMPLES {
                            self.wave_step = 0;
                        } else {
                            self.wave_step = b as i8;
                            self.last_wave = a.saturating_add(n);
                        }
                        false
                    }
                    (0xA0, true) => {
                        self.volume = i32::from(a.min(64));
                        run.remain = u16::from(n);
                        true
                    }
                    (0xA0, false) => {
                        self.semitones = 0;
                        self.period = i32::from(a as i8);
                        run.remain = u16::from(n.max(1) - 1);
                        true
                    }
                    (0xD0, false) => {
                        self.semitones = (a as i8) / 2;
                        self.period = 0;
                        run.remain = u16::from(n.max(1) - 1);
                        true
                    }
                    (0xB0, _) => {
                        run.next = Some(usize::from(a / 4));
                        jumps += 1;
                        if jumps > MOST_JUMPS {
                            run.next = None;
                        }
                        false
                    }
                    // E0: the voice stops (the volume script) or the script ends.
                    (0xE0, _) => {
                        if volume {
                            self.volume = 0;
                        }
                        run.next = None;
                        true
                    }
                    _ => {
                        run.next = None;
                        true
                    }
                };
                if took {
                    break;
                }
            }
        }
        if volume {
            self.vol = run;
        } else {
            self.pitch = run;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_cycles_are_as_the_replayer_has_them() {
        let w = built_in();
        assert_eq!(w.len(), 42);
        assert!(w.iter().all(|c| c.len() == 32));
        // A few points as Pumatracker lists them.
        assert_eq!(w[0][16] as u8, 0x3F);
        assert_eq!(w[1][16] as u8, 0xC0);
        assert_eq!(w[15][30] as u8, 0xB0);
        assert_eq!((w[16][15] as u8, w[16][16] as u8), (0x81, 0x7F));
        assert_eq!((w[31][30] as u8, w[31][31] as u8), (0x80, 0x7F));
        assert_eq!(w[33].iter().filter(|&&x| x as u8 == 0x80).count(), 11);
        assert_eq!((w[36][1] as u8, w[36][2] as u8, w[36][31] as u8), (0x80, 0x90, 0x7F));
        assert_eq!((w[37][2] as u8, w[37][16] as u8, w[37][31] as u8), (0xA0, 0x45, 0x12));
        assert_eq!((w[40][4] as u8, w[40][16] as u8), (0x7F, 0x80));
        assert_eq!((w[41][15] as u8, w[41][17] as u8), (0x7F, 0x80));
    }

    #[test]
    fn a_volume_ramp_runs_its_ticks_then_the_next_command() {
        let s = PumaSound { volume: vec![[0xC0, 3, 0, 0], [0xA0, 64, 0, 4], [0xE0, 0, 0, 0]], pitch: vec![] };
        let mut st = PumaState::new();
        let v: Vec<u8> = (0..7).map(|_| st.tick(&s).volume).collect();
        assert_eq!(v, [64, 64, 48, 32, 16, 0, 0]);
        assert_eq!(st.tick(&s).wave, 3);
    }

    #[test]
    fn cycles_bounce_and_pitch_sets_and_ramps() {
        let s = PumaSound { volume: vec![[0xC0, 10, 1, 2], [0xA0, 64, 64, 200]], pitch: vec![[0xD0, 24, 0, 2], [0xA0, 10, 0, 3], [0xE0, 0, 0, 0]] };
        let mut st = PumaState::new();
        let steps: Vec<Step> = (0..6).map(|_| st.tick(&s)).collect();
        let waves: Vec<u8> = steps.iter().map(|x| x.wave).collect();
        assert_eq!(waves, [10, 11, 12, 11, 10, 11]);
        let pitch: Vec<(i8, i32)> = steps.iter().map(|x| (x.arp, x.period)).collect();
        assert_eq!(pitch, [(12, 0), (12, 0), (0, 10), (0, 7), (0, 3), (0, 3)]);
    }

    #[test]
    fn a_jump_loop_with_nothing_in_it_ends() {
        let s = PumaSound { volume: vec![[0xB0, 0, 0, 0]], pitch: vec![[0xB0, 0, 0, 0]] };
        let mut st = PumaState::new();
        assert_eq!(st.tick(&s).volume, 64);
    }
}
