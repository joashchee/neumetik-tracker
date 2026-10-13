// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! **Neumetik Synth**: the synthesis methods Neumetik's other crates play,
//! each written from scratch on public material whose license lets
//! Neumetik ship it closed source (`docs/synth-methods.md` has each
//! method's sources and why it's clear).
//!
//! - [`opl`]: FM the OPL's way (Yamaha's OPL2 and OPL3): a patch played
//!   as a voice, for tracker modules' AdLib instruments, and the chip by
//!   its registers.
//! - [`sequence`]: OctaMED's synth and hybrid sounds, single-cycle
//!   waveforms stepped through by a waveform and a volume sequence.
//! - [`puma`]: Pumatracker's instruments, a volume and a pitch script
//!   over the song's samples and the replayer's built-in cycles.
//! - [`sel`]: Face The Music's SEL scripts, a script a track playing
//!   the song's samples by program (pitch, volume, sample, LFOs, other
//!   tracks), run by the player as its host.
//! - [`musicline`]: Musicline Editor's replayer, every channel its own
//!   sequence and speed, instruments whose waves five effects remake
//!   every tick (a morph, a phase squeeze, a mix, a resonance, a filter)
//!   or whose loop moves through the sample.
//! - [`ptk`]: ProTrekkr's and NoiseTrekker's replayer, sixteen tracks
//!   each through its own filter, flanger and compressor into a chorus
//!   and a reverb, instruments that play samples and a two-oscillator
//!   synth, and two TB-303s.
//! - [`replay`]: songs played by their own replayer (the synth and chip
//!   trackers): a [`replay::Machine`] plays any of them.
//! - [`ahx`]: AHX and HivelyTracker, every sound a filtered triangle,
//!   sawtooth, square or noise stepped through by a play list.
//! - [`paula`]: the Amiga's sound chip, as the Amiga replayers play it.
//! - [`fc`]: Future Composer 1.3 and 1.4, instruments of a frequency and
//!   a volume sequence over waveforms and samples.
//! - [`sidmon1`]: SidMon 1, read from its rip by its replayer's own
//!   instructions; waveforms and samples under an envelope and arpeggio.
//! - [`sidmon2`]: SidMon 2, samples stepped through lists under an
//!   envelope, a point of each inverted as it plays.
//! - [`soundmon`]: BP SoundMon 2.0 and 2.2, samples and synth waveforms
//!   remade as they play by an EG, averaging, morphing and modulation.
//! - [`deltamusic1`]: Delta Music 1, samples and waveform tables under an
//!   envelope, arpeggio, vibrato and bends the song's effects reshape.
//! - [`deltamusic2`]: Delta Music 2, samples and waveform tables under
//!   five-stage volumes and vibratos, each channel its own block list.
//! - [`mugician`]: Digital Mugician 1 and 2, samples and waveforms
//!   remade by fifteen effects as they play; 2's seven channels.
//! - [`sonic`]: Sonic Arranger, samples and waveforms remade by 17
//!   effects under ADSR and AMF tables, at the song's own tempo.
//! - [`instereo`]: InStereo! 1.0 and 2.0, samples and synth waveforms
//!   stepped through by envelope generators.
//! - [`musicassembler`]: Music Assembler, samples and sweeping
//!   waveforms under an attack, hold and release, from its rips.
//! - [`synthesis`]: Synthesis, InStereo! 1.0's sibling, its waveforms
//!   reshaped as they play by 16 effects.
//! - [`voodoo`]: Voodoo Supreme Synthesizer, two waveforms made into a
//!   third each tick (ring modulation, XOR, morphing), or a sample
//!   stepped through block by block at Paula's audio interrupt.
//! - [`am`]: StarTrekker's AM instruments, a looped 32-point cycle
//!   under a five-stage envelope.
//! - [`jam`]: JamCracker's AM instruments, a looped 64-point cycle
//!   mixed with itself at a moving phase.
//! - [`cycle`]: single-cycle waveforms made into looping samples (the
//!   Tracker's synths).
//!

pub mod ahx;
pub mod am;
pub mod cycle;
pub mod deltamusic1;
pub mod deltamusic2;
pub mod fc;
pub mod instereo;
pub mod jam;
pub mod mugician;
pub mod musicassembler;
pub mod musicline;
pub mod opl;
pub mod paula;
pub mod ptk;
pub mod puma;
pub mod replay;
pub mod sel;
pub mod sequence;
pub mod sidmon1;
pub mod sidmon2;
pub mod sonic;
pub mod soundmon;
pub mod synthesis;
pub mod voodoo;
