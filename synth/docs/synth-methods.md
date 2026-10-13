# Synth: the synthesis methods, and why each is free to ship

Research for `neumetik-synth` (`synth/src/`), done 2026-10-07. Every
method has to be one that can ship in any binary, open or closed source:

- **Algorithms and hardware behavior aren't licensed; code is.** Every
  method is written from a published description (a datasheet, a file
  format's specification, a manual, a paper) in our own Rust, with no
  dependency and no code copied from an emulator or player. An emulator's
  license (GPL for DOSBox's and MAME's older OPL cores, LGPL for Nuked-OPL3
  and libxmp) therefore doesn't reach Neumetik, even where the same chip or
  format is emulated there. Their code was not used to write this; the one
  look was at libxmp's copy of the MMD `SynthInstr` structure, to check its
  field offsets against Kinnunen's description, which it reproduces.
- **Patents can cover methods.** Yamaha's FM patents (Chowning's 1975
  patent, licensed to Yamaha, and Yamaha's OPL-era ones) expired by the
  late 1990s and 2000s. Nothing patented and live is used.
- **Names stay generic** (the root README's Names rule): presets are called
  what they sound like, never a maker's instrument. "OPL", "AdLib" and
  "OctaMED" appear only in comments and docs to say what's emulated, and
  in UI text to say what a file's instruments are.

## The rule

Any new synthesis method that a Neumetik crate needs (a tracker format's
synth instruments, a chip, a new kind of voice) lands in `neumetik-synth`,
written from scratch on public material whose license lets it ship
closed source, with a row in the table below saying where it's from and
why it's clear. The crates that need it (`neumetik-tracker` first) call it.

## The methods

| Method | What it makes | Source | Free to ship because | In Neumetik |
|---|---|---|---|---|
| FM synthesis, two and four operators | The OPL's sound: a sine's phase bent by another's output | Chowning, "The Synthesis of Complex Audio Spectra by Means of Frequency Modulation" (JAES, 1973) | Published method; its patent (US 4,018,121) expired 1994 | `opl/op.rs`, `opl/voice.rs`, `opl/chip.rs` |
| The OPL2 and OPL3's registers, operators and envelopes | Yamaha YM3812 and YMF262 played by their registers: multiplier, total level, key scaling, attack/decay/sustain/release, feedback, connection, 4-op modes, stereo, rhythm mode | Yamaha's YM3812 and YMF262 application manuals (the datasheets: register maps, the rate tables, the feedback and KSL tables); the AdLib programming guide | Hardware behavior described in public manuals; the tables used are the datasheet's published values (KSL in dB, feedback in π), or computed from formulas | `opl/` |
| Log-sine and exponential lookups | The chip's waveforms and attenuation without a multiplier: a quarter sine's log, levels added in the log domain, back through 2^x | Public analysis of the chip's die (the "OPL2 decapsulated" write-ups), which describe the method and the tables' formulas | Mathematics: the tables are computed here from -log2(sin x) and 2^-x, not copied | `opl/tables.rs` |
| The envelope's rate steps | How an envelope's rate maps to time (each step of four doubles the speed, the two low bits fill eight clocks) | The datasheets' attack and decay time tables (e.g. decay rate 1: 39.28 s to -96 dB), checked by a test here | Published timings; the step pattern is the plain way to meet them | `opl/op.rs` (`ENV_STEPS`, a test checks rate 1 against the datasheet) |
| The OPL3's waveforms 4 to 7 and the rhythm mode's drums | Alternating sine, camel, square, derived (log) square; hi-hat, snare and cymbal from bits of two oscillators and noise | The YMF262 manual's waveform figures; public descriptions of the rhythm section's phase logic | Described behavior, written our own way; the noise register's taps are ours | `opl/op.rs` `wave_out`, `opl/chip.rs` `drums_sample` |
| S3M's AdLib instruments | An instrument's eleven OPL bytes (D00 to D0A), its volume, its C5 rate; volume scaling the carrier's total level | Scream Tracker 3's TECH.DOC (Sami Tammilehto, 1994), the format's public description | A file format's published description | `opl/voice.rs` `Patch::from_s3m`; `tracker/src/formats/s3m.rs` |
| Composer 670's FM instruments | Eleven bytes each, the connection byte first | CDFM's format as the community documented it | A file format's public description | `Patch::from_c67`; `tracker/src/formats/pc.rs` |
| OctaMED's synth and hybrid sounds | Single-cycle waveforms stepped through by a volume and a waveform sequence (SPD, WAI, CHD, CHU, EN1, EN2, JWS/JVS, ARP, VBD, VBS, VWF, RES, HLT, JMP, END) | "MED/OctaMED MMD0 and MMD1 file formats, revision 1" (Teijo Kinnunen, 1992) for `SynthInstr`'s layout; OctaMED's manual for the commands | A file format's and a program's published documentation | `sequence.rs`; `tracker/src/formats/amiga.rs` `med_synth` |
| Pumatracker's instruments | A volume script (C0 the waveform, or a run of built-in cycles stepped back and forth; A0 a volume ramp; B0 a jump; E0 stop) and a pitch script (D0 semitones for so many ticks; A0 a ramp of the Amiga period; B0; E0) over the song's ten samples and 42 built-in 32-point cycles | The format as the community documented it, and OpenMPT's reading of it (its BSD-licensed loader and script player, read for what the commands do, not copied); the 42 cycles are the replayer's own data, which the files depend on, generated here from their patterns (ramps, pulses, saws) with the three irregular ones as their points | A file format's public description; the cycles are facts of the format (like the TR-808's ratios above), not code | `puma.rs`; `tracker/src/formats/amiga2.rs` |
| Face The Music's SEL scripts | A script a track: waits, jumps, loops and conditions, interrupts on the pattern's next note, volume, sample, release, bend or fade; pitch, detune (a channel pair's), volume, the sample, its play position, start and loop; four LFOs (sine, square, triangle, ramps; on pitch, volume or each other); other tracks worked on or copied; the song's volume, tempo, speed and position | Face The Music's manual's description of the commands; OpenMPT's reading of the format (its BSD-licensed loader and script player, read for what each command does, not copied) | A program's published documentation | `sel.rs`; `tracker/src/formats/amiga3.rs` `load_ftm`, the player's `SelHost` |
| Musicline Editor's replayer | A song's channels each at their own speed and groove through a sequence of parts; instruments under a four-stage envelope, vibrato and tremolo (sine, ramp, saw, square, with delay and attack), arpeggio tables (notes, samples, slides, volumes); their single-cycle waves remade each tick by a morph through up to six waves, a phase squeeze (four blends), a mix with another wave, a resonance and a two-pole filter (normal or resonant), or a loop moving through the sample; ProTracker's slides, vibrato and tremolo too | NostalgicPlayer's MIT-licensed Musicline player and the original replayer's source (Musicline's mlineplayer, LGPL, kept with UADE), both read for what each command and effect does, neither copied; renders compared with UADE running the original; the replayer's tables (periods in 32nds of a semitone, the shapes, the filters' and resonance's coefficients, the phase's resampling) computed from formulas found to give them, the coefficient lists' few off-curve entries written as they are | A file format's and a program's behavior as publicly described; the tables are facts of the format, computed here | `musicline/`; `tracker/src/formats/musicline.rs`, the player's Musicline host |
| StarTrekker's AM instruments | A 32-point cycle (sine, ramp, square) or noise, looped, under an envelope of five stages each moving a level a tick (start, two attacks, decay to sustain, a hold, release), with a vibrato and an octave | StarTrekker 1.2's documentation of its AM synth (the parameters, what each stage does); the .NT file's layout as the community described it | A program's and a file format's published documentation; the cycles are their shapes (127 × sin rounded, a ramp, a square), computed here | `am.rs`; `tracker/src/formats/amiga3.rs` `startrekker_am` |
| JamCracker's AM instruments | A 64-point cycle looped (stored twice over), remade every tick while its channel's phase runs: each point the average, rounded down, of itself and the point a quarter of the phase on, the phase stepping 1 to 15 a tick through 256 (a comb sweeping across the cycle); each of four channels plays its own copy, kept as the file has it until first mixed into, then holding the last mix | JamCracker's replayer (Martin Gemmel's, adapted for EaglePlayer, kept with UADE) and NostalgicPlayer's MIT-licensed JamCracker player, both read for what the phase does, neither copied; renders compared with UADE running the original | A program's behavior as publicly described; the mix is plain arithmetic | `jam.rs`; `tracker/src/formats/extra.rs` `load_jam`, the player's `Fx::Phase` |
| ProTrekkr's and NoiseTrekker's replayer | A song at 44.1 kHz, frame by frame: rows and their effects (worked six or seven times a row), each track's sub-channels (a split sample, a synth, or both) through its filter (biquads from a cutoff and resonance; two- and four-pole resonant low- and high-passes; ring modulation; slew limiting; sample and hold; bell EQs; power-law distortion), its distortion, flanger, RMS compressor, three-band EQ, panning and LFOs, into the mix's chorus (a feedback delay a side), reverb (one-pole combs into allpasses and a resonant low-pass) and compressor. The synth: two oscillators (sine, saw, pulse, triangle, white noise, or the sample) and a sub-oscillator an interval under, combined (add, subtract, multiply, divide), with pulse width, hard sync, glide, two ADSRs and two LFOs (a sine under its own ADSR) each modulating pulse width, pitch, volume, cutoff, resonance and distortion, a soft clip and a two- or four-pole filter. The TB-303: a sawtooth or square through a four-pole resonant low-pass whose cutoff, resonance and envelope fall a little every frame, with accents and slides, stepped by a 32-pattern sequencer | ProTrekkr's replayer (Franck Charlet's, BSD-licensed, after Juan Antonio Arguelles Rius's NoiseTrekker) and its file loader, read for what each part does and the order it does it in, not copied; renders compared with ProTrekkr's own replayer built in a temporary folder for testing, never shipped (every one of its 49 example songs the same to the sample); its tables (the stock cycles, the sine, the panning's square roots, the reverb presets and allpass spacing) computed from their formulas or written as the facts of the format they are; its fast `pow` and `log` (a float's bits read as its exponent, the fraction's curve a parabola) worked out the same way, as every pitch and filter curve depends on them | A program's behavior and a file format, as publicly described; the BSD source was read, not copied; the formulas and constants are facts of the format | `ptk/`; `tracker/src/formats/ptk.rs`, the player's ProTrekkr path |
| Songs played by their own replayer | A synth or chip tracker's whole song, a tick at a time, each channel rendered between ticks; seeking (played silently from the start), mutes, gains, meters and a fade-out, the same for every format | Neumetik's own design, after the ProTrekkr and Musicline replayers here | Our own code | `replay.rs`; the Tracker's `Module::tune` and the player's tune path |
| AHX's and HivelyTracker's replayer | Waveforms made at start: triangles and sawtooths of 4 to 128 points, 32 squares of 128, 1,920 points of noise (a 32-bit register rotated and mixed), each through 31 low-passes and 31 high-passes (a state-variable filter); per tick, each channel's row (notes, transposes, portamento with a limit, volume slides, note cut and delay, pan), its envelope (attack, decay, sustain, release in frames), vibrato (a 64-point sine), its instrument's play list (waveform, note, two commands: filter, slides, the square's width, start and stop of its modulations, jumps, volumes, speed; HivelyTracker's ring modulation by a triangle or sawtooth and pan), the pulse width and filter sweeping between limits, a hard cut before the next instrument; a 640-point buffer of the waveform looped by the mixer at the Amiga's period | NostalgicPlayer's MIT-licensed HivelyTracker player and HivelyTracker's BSD-licensed replayer (`hvl_replay.c`), read for what each part does, neither copied; renders compared with the original AHX replayer in UADE and with HivelyTracker's own replayer built in a temporary folder for testing (hvl2wav, with 32-bit integers as its own builds have them), never shipped. The filters' starting states, which NostalgicPlayer lists as tables, are the filter run three times round each wave from rest in 16.16 fixed point, in the original's order (triangles, sawtooths, squares, noise): computed here, every one of the 1,395 the same; the periods are ProTracker's three octaves and two below them doubled; the vibrato is 255 × sin rounded down | A file format's and a program's behavior as publicly described; the tables are computed from formulas | `ahx/`; `tracker/src/formats/synth.rs` |
| The Amiga's Paula | Channels stepping through memory at a period of the PAL clock and a volume of 0 to 64, each point held until the next; a part played once, then its loop (a loop set while playing waits for the part to end, as the chip's registers do), silence without one; the memory read as it plays | The Amiga Hardware Reference Manual's description of the audio DMA (public); the replayers' own use of it | Hardware behavior as publicly described | `paula.rs` |
| Future Composer's replayer | Per channel, its patterns in turn (each transposed, its instruments by a sound transposition), 32 rows or to an END mark; every frame its frequency sequence (a waveform or sample, its loop, a sample of a set, jumps, sustains, pitch bends, vibrato, a transposition a frame) and volume sequence (levels, slides, sustains, jumps); vibrato by a triangle whose depth doubles each octave down; portamento every other frame; 1.3's 47 waveforms | NostalgicPlayer's MIT-licensed Future Composer player and its 1.3 converter, read for what each command does, not copied; renders compared with the original replayers in UADE. The period table is the original replayer's (its octaves 0 to 3, then an octave and two below 0, then 0 to 3 again in 1.4), which NostalgicPlayer's departs from past its 72nd note; 1.3's waveforms made from their patterns (a cycle with a run written over its second half, pulses of each width, sawtooths, a triangle), its two drawn ones as their points | A file format's and a program's behavior as publicly described; the tables are facts of the format | `fc.rs`; `tracker/src/formats/synth.rs` |
| SidMon 1's replayer | The song's data found by its replayer's own instructions in the rip (the LEA, ADDA and MULU that point at it, as NostalgicPlayer finds them); per channel, tracks of rows that last their own durations, each transposed; instruments of a waveform list or a sample, an attack, decay, sustain and release, a 16-step arpeggio, a phase shift (the period swung by a waveform), a pitch fall and bends to a note; two waveforms remade as a moving mix of two others, and one inverted a point a tick | NostalgicPlayer's MIT-licensed SidMon 1 player, read for what the replayer does and how its data is found, not copied; renders compared with UADE running the original. The period table is the rip's own (read from the file, as every rip carries it), with its fine-tunes' stride from the replayer's MULU | A file format's and a program's behavior as publicly described; the table is the file's | `sidmon1.rs`; `tracker/src/formats/synth.rs` |
| SidMon 2's replayer | Per channel, a track a position, read as packed rows (a note, instrument and effect, or a run of empty rows); samples stepped through a waveform list (the next waiting for the one playing), an attack, decay, sustain and release, arpeggio and vibrato lists, a pitch bend after a delay, a slide to a note; each sample playing has a point inverted a tick, the point moving forward, back or bouncing between two places; ProTracker's arpeggio, slides, volumes, pattern length and speed | NostalgicPlayer's MIT-licensed SidMon 2 player, read for what each part does, not copied; renders compared with UADE running the original. The period table is ProTracker's octave 2 four, eight and sixteen times, its octave 1, octave 2, octave 3 and three a quarter of octave 2's, computed here | A file format's and a program's behavior as publicly described; the table is computed | `sidmon2.rs`; `tracker/src/formats/synth.rs` |
| SoundMon's replayer | Per channel, a step's 16-row track, transposed in note and instrument, with optional commands (arpeggios, volume, speed, slides, vibrato, jumps, auto slides and arpeggios, changes of effect); synth sounds played from the song's 64-point wave tables, their volume an ADSR from a table, their period swung by an LFO from a table, their waveform remade every so often: an EG inverting part of it as far as a table says, averaging each point with the next, morphing it toward its own start, backwards or the next table, a modulation writing a point; the waveform put back when the note starts again | NostalgicPlayer's MIT-licensed SoundMon player, read for what each part does, not copied (its averaging, which tests the wrong counter and never runs, runs here: UADE's renders of the original agree); renders compared with UADE. The periods are SoundMon's own octave (ProTracker's to the nearest four) multiplied and divided, computed here | A file format's and a program's behavior as publicly described; the table is computed | `soundmon.rs`; `tracker/src/formats/synth.rs` |
| Delta Music's replayers | 2.0: per channel, a list of 16-row blocks (transposed) looping at its own place; samples, or synth sounds stepping through a 48-step table of 256-point waveforms at a delay, the first remade every tick as noise (a 32-bit register rotated, added and mixed, written a long at a time); five-stage volume and vibrato tables, a pitch bend, portamento and 64 arpeggio tables; speed, filter, bends, portamento, volume, global volume and arpeggio as effects. 1.3: the same lists with jumps, a speed counter per channel; synth sounds stepping through a table of 32-point waveforms in the sound (with delays set by the table), an attack, decay, sustain and release, an eight-step arpeggio, a triangle vibrato, a bend rate and portamento, each of which an effect can change for the instrument | NostalgicPlayer's MIT-licensed Delta Music 1.3 and 2.0 players, read for what each part does, not copied; renders compared with UADE running the originals. The periods are ProTracker's octave 2 sixteen, eight, four and two times, itself and octave 3, then its top held (1.3's table leaves out 1,016, so its notes from there on are a place lower: kept); computed here | A file format's and a program's behavior as publicly described; the tables are computed | `deltamusic1.rs`, `deltamusic2.rs`; `tracker/src/formats/synth.rs` |
| Digital Mugician's replayer | Up to eight sub-songs (the playable ones: whole and not empty), each a sequence of 64-row tracks a channel, transposed; a speed of two nibbles played in turn (shuffle); samples and 128-point waveforms, each instrument's waveform remade every so often (once a tick, however many channels play it) by one of fifteen effects: two-point and three-point filters, a moving mix of two waveforms, scrolls, up- and down-sampling, a moving negation, a ramp added, an addition, two morphs between waveforms, two gates; volume and pitch read from waveforms, eight 32-step arpeggios, bends to a note. Version 2 (and 3) plays seven channels, the last four mixed into one of the Amiga's at full level each, the song ticked once a mixing buffer | NostalgicPlayer's MIT-licensed Digital Mugician player, read for what each part does, not copied; Wanted Team's Mugician II player for EaglePlayer (its source kept with UADE), read for how version 2's mix and its timing work, not copied; renders compared with UADE. The period table (16 fine-tunes of 64 notes) is 4,825 × 2^(-note/12) × 2^(fine-tune/192) rounded, its three entries off that curve written as they are | A file format's and a program's behavior as publicly described; the table is computed | `mugician.rs`; `tracker/src/formats/synth.rs` |
| Sonic Arranger's replayer | Sub-songs (a first, last and restart position, rows a track, a speed and a tempo in ticks a second); each channel at a position a run of the song's rows from its own start, transposed in note and instrument (each row can turn either off); samples, or 128-point waveforms copied to the channel's own and remade every so often by one of 17 effects (a moving negation, a negation from a point a table says, vertical and horizontal rotations, a waveform added, a moving pair negated, shack waves, morphs toward another waveform and back, a laser's detune, an alias, two noises, two low-pass smoothings, an FM drum's falling pitch); volume from an ADSR table held at a sustain point while the note's held; pitch from an AMF table, a 256-step vibrato, portamento from the last note, slides and three arpeggio tables | NostalgicPlayer's MIT-licensed Sonic Arranger player and its converter for the "final" form, read for what each part does, not copied; renders compared with UADE. The periods are ProTracker's octaves 1 to 3 multiplied and halved, computed here; the vibrato table is the replayer's own (not quite a sine), written as it is | A file format's and a program's behavior as publicly described; the vibrato table is a fact of the format | `sonic.rs`; `tracker/src/formats/synth.rs` |
| InStereo!'s replayers | 1.0: positions and rows as Sonic Arranger's (its note and instrument transposition skipped by effects), sixteen 16-step arpeggio tables stepped by the speed counter, a triangle vibrato (made here: up by two, down, up), portamento between the last note and this one over so many ticks, a slide; synth sounds copy a 256-point waveform with its first points inverted, as many as an envelope table says (once or round and round), into one of two buffers in turn; ADSR tables. 2.0: Sonic Arranger's rows and tables; synth sounds of two 256-point waveforms, an envelope generator (free from a table, or calculated, rising and falling between two points) choosing where in them to play; an ADSR held at its sustain point, an LFO, three arpeggio tables | NostalgicPlayer's MIT-licensed InStereo! 1.0 and 2.0 players, read for what each part does, not copied; renders compared with UADE. The periods and 2.0's vibrato are Sonic Arranger's | A file format's and a program's behavior as publicly described | `instereo.rs`; `tracker/src/formats/synth.rs` |
| Music Assembler's replayer | Each channel a position list of tracks (transposed, repeated), each row held for its own count of the song's rows; an instrument's sound a sample (its loop following once started) or a waveform of up to 128 words, its start moved each tick by a level that turns back every so many ticks, wrapped to its length; volume up by the attack until past its top, then the sustain held so many ticks before the release; a three-step arpeggio (the note, then each nibble above it) whose speed may halve the volume on alternate steps; vibrato after a delay, its offset kept through portamento; portamento from the track, every other tick at an odd rate | NostalgicPlayer's MIT-licensed Music Assembler player, read for what each part does, not copied; where it and UADE differed, the rips' own 68000 code, read by hand; renders compared with UADE | A file format's and a program's behavior as publicly described | `musicassembler.rs`; `tracker/src/formats/synth.rs` |
| Synthesis's replayer | InStereo! 1.0's positions, rows, ADSR tables, envelope generator, vibrato and portamento (without its switch), plus: an instrument's own arpeggio (a start, length and repeat in the tables) when the row names none; row effects that switch transposition off, change the waveform effect and its arguments, turn it or the envelope generator off, or set a volume; 16 waveform effects on the channel's buffer while the envelope generator is off, most stepping once every so many ticks: a span raised, or turned round by a point; another waveform added, or its next point added to all; a point inverted moving along a span (alone, or with the last put back); an octave up (every other point twice) or down (each doubled); each point moved towards the next by a level or an envelope table's steps; each a step towards another waveform; the pitch slid by another waveform, a laser's sweep or an FM drum's | NostalgicPlayer's MIT-licensed Synthesis player, read for what each part does, not copied; its arpeggio, portamento and vibrato checked against the 68000 code of a player kept in a Synth4.2 file, read by hand; renders compared with UADE | A file format's and a program's behavior as publicly described | `synthesis.rs`, sharing `instereo.rs`'s reading of the file; `tracker/src/formats/synth.rs` |
| Voodoo Supreme Synthesizer's replayer | Each channel a stream of notes (held so many ticks), calls, loops and jumps; a volume envelope of steps and slides; a period table whose commands add or take away, double or halve, or shift the period by semitones (the replayer's own fractions); portamento of so many steps over its ticks; the period and volume reaching the chip a tick late; four ways of making the sound: ring modulation (the two waveforms averaged, the second turning round, the result's bits ORed with a mask), XOR (the second's masked points XORed into a span that moves), morphing (the span's points moved towards one of the two by a speed), or frequency mapping (a sample stepped through 128 points a block, each block chosen at Paula's audio interrupt, its step growing with the period so its length holds) | NostalgicPlayer's MIT-licensed Voodoo Supreme Synthesizer player, read for what each part does, not copied; Paula's reload-and-interrupt behavior from the Amiga's hardware as documented; renders compared with UADE | A file format's and a program's behavior as publicly described | `voodoo.rs`, `paula.rs` (its `Stepper`); `tracker/src/formats/synth.rs` |

The Tracker's single-cycle synths (sine, saw, pulse, triangle, noise, FM
made into looping samples) moved here rather than being copied
(`cycle.rs`): the Tracker already depends on this crate, so it calls it,
and its output is the same to the bit.


## Choices where the sources leave something open

- **The FM voice and the chip run at the chip's rate** (14.318 MHz / 288,
  49,716 Hz) and are resampled linearly to the output's, so envelope and
  LFO timings and their steps are the chip's at any output rate.
- **A patch played on its own** (`FmVoice`) turns a frequency into the
  F-number and block the chip would use (the lowest block it fits), so
  its pitch steps are the chip's.
- **Modulation depth**: an operator's output (±4,095) is added to the
  next one's 10-bit phase as it is, which with feedback's `(a + b) >>
  (9 - FB)` gives the datasheet's feedback table exactly (FB 1 = π/16,
  FB 7 = 4π).
- **Tracker volume on FM** follows Scream Tracker 3's way: the volume
  scales the carrier's total level (and the modulator's, additive),
  `63 - (63 - TL) × vol / 64`, so it falls in the chip's 0.75 dB steps.
  A note cut lets the key go, so the note rings out by its release. FM
  sits at 0.4 of a sample's level in the mix.
- **OctaMED**: a sequence steps once every `speed` ticks (a step's own
  SPD counts from the next); WAI waits whole ticks; CHD and CHU slide
  every tick; vibrato swings the period by up to its depth × 2 (depth
  64 is ±128); an arpeggio steps one entry a tick while the sequence
  goes on after ARE; a loop of commands with nothing in it halts. The
  period offsets are the Amiga's; the tracker player's periods are four
  times finer, so it scales them.
- **Pumatracker**: both scripts step every tick, the volume script's
  first; a command that takes time (a ramp, a pitch for so many ticks,
  a stop) ends the tick's reading, the rest (a waveform, a jump) don't;
  a volume ramp holds its start for two ticks, a pitch ramp for one,
  and both stop a step short of their end; a run of built-in cycles
  steps before the scripts, bouncing at its ends. The scripts' volume
  scales the note's (Pumatracker adds it); a loop of jumps with nothing
  in it ends the script.

- **Musicline**: the original replayer's source is followed where
  NostalgicPlayer departs from it: a channel past its sequence's end
  (or with none) goes round part 0, rather than stopping; a line naming
  an instrument the song lacks keeps the one playing; every jump in
  the sequence counts down its loop (no guard for one song's endless
  loop: a tick reads at most 256 entries). The 68000's 16-bit
  arithmetic is kept where songs hear it: the morph shifts its
  product's low word, the filters subtract their state as a word, the
  resonance's divide leaves its dividend when the quotient won't fit.
  Its own slips are kept too (the arpeggio's restart clears the
  resonance's and mix's memory on the wrong flags). The ProTracker
  tremolo plays as one (the original does; NostalgicPlayer tests it
  against the vibrato). Mixer mode is a tune's own: its eight channels
  each at half level (two to each of the Amiga's four), its ticks as
  long as its mix buffer (points at period 126, from the tune's tempo
  at its start; a tempo command doesn't change it); in a four-channel
  tune a tick is the CIA's period, 1,773,448 / tempo + 1 of 709,379 a
  second. A loop newly set waits for the loop playing to end, as the
  Amiga's channel does. Left out: the few hundred microseconds the
  replayer waits before restarting a channel, and the mixer's own
  nearest-point resampling at 28 kHz. A damaged song's reads past its
  data read silence.

- **AHX and HivelyTracker**: an AHX song plays as the original AHX
  replayer does on the Amiga, which UADE runs: ticks at the PAL screen's
  49.92 a second (313 lines of 227 clocks; 50 drifts a beat in a minute),
  its filters in fixed point with three warm-up rounds, a full swing of
  its noise down when bit 15 is set, its noise's seed rotated. A
  HivelyTracker song plays as HivelyTracker's replayer does: 50 ticks a
  second, the filters in floating point with one warm-up round, the
  noise's full swings all up and its seed shifted with its sign (that
  replayer's own slips, which its songs were made hearing). Both mix
  each channel by stepping through its 640-point buffer at the period's
  rate, nearest point, as both replayers do; HivelyTracker's pan law (a
  quarter sine) for its panning.

- **Future Composer**: ticked by the CIA timer at "50 a second" (14,188
  of its clocks), as the original's player in UADE is; a period worked
  as a word and held to 113 to 3,424 (one that goes below 0 is held at
  the bottom). A sample plays once to its end, then its loop (a
  sample whose loop is two bytes, the file's "none", just stops); a
  waveform loops whole; a new loop waits for the old to finish.

- **SidMon 1**: ticked by the CIA at "50 a second", as UADE plays it.
  A rip is the replayer's memory as it was: its start sets the row to
  -1 and the position to 1 but leaves the speed counter as it was saved,
  so the counter starts at the rip's (NostalgicPlayer starts it at the
  song's speed). A song saved with it at 0 reads its first row a tick
  early, which makes its first position a row longer: the original
  does, and UADE plays it so on every test song.

- **SidMon 2**: ticked by the CIA at "50 a second".

- **SoundMon**: ticked by the CIA at "50 a second". Its replayer writes
  voice 1 to the Amiga's channel 4, 2 to 3 and so on; both sides are the
  same (left and right swap with each other), so the voices stay in
  their own order here.

- **Delta Music**: both ticked by the CIA at "50 a second". 2.0's noise
  is written as the Amiga writes a long, high byte first (NostalgicPlayer
  writes it the other way round; it's noise either way). UADE's player
  starts a channel whose song is silent on the first instrument at the
  top period, which it then lets fade: not done here (the file doesn't
  say so).

- **Digital Mugician**: version 1 ticked by the CIA at "50 a second".
  Version 2's ticks come from its mixing buffer, as UADE's player times
  them: at its period 224, a buffer of (71,591 / 224 + 1) points rounded
  down to a word, about 49.48 ticks a second; version 3 (the one known
  song, Moonchild's) at 130/210 of that buffer, as that player plays
  that song. The mixed voices are added at full level, as its mixer
  adds them, but not clipped to its eight bits, and played at the
  output's rate rather than resampled to its 16 kHz.

- **Sonic Arranger**: ticked by the CIA at the song's tempo (709,379 /
  tempo + 1 of its clocks a tick). Three of NostalgicPlayer's readings
  are not followed, for what the original plainly does: the vibrato
  plays at the speed and depth the channel has (which the vibrato effect
  changes), not the instrument's; a sample whose repeat is 0 loops all
  of itself, not its first half; noise generator 2 rotates each point's
  bits, as the 68000's ROL does.

- **InStereo!**: 1.0 ticked by the CIA at "50 a second". 2.0's songs
  say a tempo, but UADE's player ticks 1.85% slower than it: 722,500 /
  tempo of the CIA's clocks a tick (measured on six songs at three
  tempos; that player has no source to read). A synth sound's envelope
  step that finds its channel stopped starts it, as writing the Amiga's
  registers with its DMA on does.

- **Music Assembler**: ticked by the CIA at "50 a second" (the vblank
  drifted against UADE). Paula's channels 0 to 3 play the sub-song's
  lists 0, 2, 3 and 1. Read from the rips' replayer, where
  NostalgicPlayer's differs: the attack ends once the volume passes its
  top (so a full attack plays one tick at full volume first), the
  arpeggio steps once its counter passes the speed, the vibrato's level
  is unsigned, and its offset is added every tick.

- **Synthesis**: ticked by the CIA at "50 a second", as InStereo! 1.0. Its
  file is read by InStereo! 1.0's code (the same layout, the instrument's
  bytes InStereo! leaves unused being Synthesis's arpeggio and effect).
  Each waveform effect works on a copy of the buffer with room after it,
  so one reaching past the end reads zeros. On one song (supermodule2)
  UADE plays a channel with a vibrato of about a semitone that the
  instrument's settings don't give; not found yet.

- **Voodoo Supreme Synthesizer**: ticked at 709,379 / 14,168 a second
  (50.07): UADE's player runs 0.14% faster than the CIA's 50 a second
  (measured on five songs over a minute; it sets no timer that can be
  read). Paula gained a `Stepper`: a part chosen afresh at each of a
  channel's reloads, as the format's audio interrupt chooses the next
  block of a frequency-mapped sample, so its blocks change on time
  rather than a tick late. The period table's twelfth entry is the
  replayer's 17,689 where the octave would say 17,024. Tracks have no
  rows; the patterns show each channel's notes by tick, a row as many
  ticks as all the notes' holds are multiples of.

- **ProTrekkr**: the replayer of ProTrekkr's editor (the one songs were
  made and heard in) is followed where its stand-alone player differs:
  a track switched off for a position lets its notes go once (so a
  TB-303 fired there again steps on, silent), and the mix's compressor
  is on when its ratio is over 1. It runs at its own 44.1 kHz and is
  resampled linearly to the output's. Its slips are kept: a new note
  never lets its synth go first (its test of whether to glide is always
  true), so each note column plays on the sub-channel it had; the
  retrigger takes the "oldest" sub-channel as it finds it (the last
  older than the first, by the frame its note was scheduled); the
  four-pole filter's right side filters the left's output; the
  sub-oscillator's ping-pong and a backward loop add a whole length; a
  wide reverb spread starts a right allpass's write in the line before
  it; the 0x2B compressor ratio takes the scaled threshold as a raw one;
  the old versions' synth is read as the C structure it was (the padding
  byte that became the combine mode with it), at the size it had then
  (today's ProTrekkr reads versions 5 to 9 two bytes off, its structure
  having grown since); the reverb's switch is the low byte of the word
  stored; NoiseTrekker's glide of 0 becomes 64. Sums and products are
  worked in the replayer's order (a modulated cutoff is its base plus
  each source in turn), so they round as its do. Its white noise is our own generator's (the replayer's is
  the C library's `rand`), as is the random cutoff's (0x0A). The meters
  are ours.

## Still to come

Every tracker format read today plays its synths here. The synth and
chip trackers still planned in `tracker/docs/tracker-formats.md`
(the SID, the AY and the NES's chips, the Ad
Lib register logs) each add a method here, under this rule, as the
tracker reads them, played through `replay.rs` as AHX and Future Composer are. The OPL chip (`opl::Chip`)
is already in place for the AdLib formats (RAD, A2M, IMF, D00, HSC and
the rest).
