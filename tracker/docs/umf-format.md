# UMF, the Universal Mod Format

A UMF file is a tracker song that refers to its samples rather than
holding them. Every format neumetik-tracker reads becomes UMF without
loss: one model wide enough for MOD, S3M, XM, IT and the rest
(`src/model.rs`), with `Kind` saying whose effect rules it plays by.

Its samples are in the **universal tracker samples repository**
(`repository.md`), each named by an ID made from its content. A UMF
player reads the song, then asks the repository for each waveform by its
ID. A UMF saved **with samples** carries its waveforms too and plays
anywhere, repository or not; the waveforms in it are checked against
their IDs just as the repository's are. A UMF with its samples is saved
as **`.ntt`** ("Neumetik Tracker type"), so a file that plays on its own
(an NTT player's) is told at a glance from one that needs the repository
(a UMF player's); the layout is the same, and flag bit 0 says which.

Reading and writing are `src/umf.rs`; `examples/umf_player.rs` is a UMF
player in a few lines.

## Layout

Little-endian throughout. Strings are a u16 length, then UTF-8. IDs are
32 bytes (SHA-256, `repository.md`); all zeros means none.

```text
"UMF" 1A            magic
version             u16: 1
flags               u16: bit 0, the waveforms are in the file
chunks, each:
  id                4 bytes
  length            u32
  data              length bytes
```

Chunks come in this order; a reader passes over chunks it doesn't know,
so later versions can add more without breaking older readers.

### INFO: the song

| Field | Type | |
| --- | --- | --- |
| title, artist, message | string ×3 | the message's lines end in `\n` |
| format | string | the format it came from (`mod`, `xm`...; `umf` once saved from UMF) |
| format name, tracker | string ×2 | in words: "FastTracker 2", "MilkyTracker 1.00" |
| kind | u8 | whose effect rules: 0 MOD, 1 S3M, 2 XM, 3 IT |
| flags | u16 | bits: 0 linear slides, 1 Amiga limits, 2 instruments, 3 IT old effects, 4 IT compatible Gxx, 5 ST3.00 fast slides, 6 vertical-blank timing |
| speed | u8 | ticks a row |
| tempo | u16 | beats a minute (a tick is 2.5 / tempo seconds) |
| global volume | u8 | 0 to 128 |
| mix volume | u8 | 0 to 128 |
| separation | u8 | stereo, 0 to 128 |
| restart | u16 | the order the song goes back to |
| source | ID | the module it was made from: the SHA-256 of that file |
| repository | string | the ID of the repository its samples were kept in |

### CHAN: the channels

u16 count, then for each: pan (u8, 0 left to 255 right), volume (u8, 0
to 64), flags (u8: 1 surround, 2 muted), name (string).

### ORDR: the order list

u16 count, then u16 each: a pattern, `FFFE` a skip (+++), `FFFF` the end.

### PATT: the patterns

The whole chunk is packed (`src/lz.rs`, a stream starting `NLZ1`).
Unpacked: u16 count, then for each pattern: rows (u16), channels (u16),
name (string), and its cells row by row. A cell is a flags byte (1 note,
2 instrument, 4 volume column, 8 effect), then what the flags say:

| Flag | Bytes | |
| --- | --- | --- |
| 1 | note | 1 to 120 (C-0 to B-9, C-5 is 61), 253 fade, 254 cut, 255 off |
| 2 | instrument | 1-based; a sample when the song has no instruments |
| 4 | kind, value | the volume column (`Vol`): 1 volume 0-64, 2 panning 0-64, 3/4 slide up/down, 5/6 fine up/down, 7 vibrato speed, 8 vibrato depth, 9/10 panning slide left/right, 11 tone portamento, 12/13 portamento up/down, 14 offset |
| 8 | effect, parameter | the effect (`Fx`, below) and its parameter, as its format meant it |

Effects (`Fx`), shown in the editor by their letter: 1 arpeggio (J), 2
portamento up (F), 3 down (E), 4 tone portamento (G), 5 vibrato (H), 6
tone portamento and volume slide (L), 7 vibrato and volume slide (K), 8
tremolo (R), 9 panning (X), 10 offset (O), 11 volume slide (D), 12
position jump (B), 13 volume (c), 14 pattern break (C, the row as a
number), 15 retrigger (Q), 16 speed (A), 17 tempo (T), 18 tremor (I), 19
MOD/XM extended (e), 20 S3M/IT special (S), 21 channel volume (M), 22
channel volume slide (N), 23 global volume (V, 0-128), 24 global volume
slide (W), 25 key off (k), 26 fine vibrato (U), 27 panbrello (Y), 28
extra-fine portamento (x), 29 panning slide (P), 30 envelope position
(l), 31 MIDI macro (Z), 32 smooth MIDI macro (\\), 33 delay-cut (:), 34
parameter extension (#), 35/36 note slide up/down (u, n), 37/38 fine
portamento up/down (f, d), 39/40 fine volume slide up/down (a, b), 41
set finetune (t), 42 start SEL script xx (s; Face The Music, below; 40 and
up stops the channel's), 43 phase (p; JamCracker: the channel's AM wave
mixed with itself, the mix moving x, the low nibble, a tick from the
start; FF sets it back).

### SMPL: the samples

u16 count, then for each slot (in order; empty slots too, for their
names):

| Field | Type | |
| --- | --- | --- |
| name, file name | string ×2 | |
| sample ID | ID | in the repository; zeros for an empty slot |
| waveform ID | ID | |
| frames | u32 | the waveform's length (its canonical form: `repository.md`) |
| bits, channels | u8 ×2 | 8 or 16; 1 or 2 |
| C-5 rate | u32 | hertz |
| volume, global volume | u8 ×2 | 0-64 each |
| panning | u8 on, u8 value | |
| loop, sustain loop | (u8 kind, u32 start, u32 end) ×2 | kind 0 off, 1 forward, 2 ping-pong; end just past the last frame |
| auto-vibrato | u8 ×4 | kind (0 sine, 1 ramp down, 2 square, 3 random, 4 ramp up), speed, depth, sweep |

The settings are the file's own, so a song can tune, loop and level a
sample differently from the repository's record of it.

### INST: the instruments

u16 count, then for each: its ID in the repository (ID), name and file
name (strings), the keymap (120 × u8 note played and u16 1-based sample),
three envelopes (volume, panning, pitch), fadeout (u32, taken off the
volume each tick once released, of 65,536), global volume (u8, 0-128),
panning (u8 on, u8 value), new note action, duplicate check and its
action, pitch-pan separation (i8) and centre, random volume and panning
(u8 ×7 together), filter cutoff and resonance (u8 on, u8 value each).

An envelope: flags (u8: 1 on, 2 carry, 4 drives the filter), point count
(u8), each point a u16 tick and an i8 value (volume 0 to 64; panning and
pitch -32 to 32), then the loop's first and last point and the sustain's
(u8 ×4, `FF` none).

### WAVE: a waveform carried in the file

Only in files saved with their samples, one for each waveform used:
its ID (32 bytes), then its repository file (`UMFW`, `repository.md`;
version 2 files, packed by `wavepack`, need a Neumetik from 0.11 on).

### What came after version 1's first readers

Written only when the song uses what they hold, after INST; a reader
that doesn't know them passes them over and plays the song without them
(no effect slots, one order list, classic tempo). Each reads into the
song its earlier chunks made, so they come after those.

**SONG**: the tempo mode (u8: 0 classic, 2.5 ÷ tempo seconds a tick; 1
alternative, 1 ÷ tempo; 2 modern, 60 ÷ (tempo × rows a beat) a row),
rows a beat and a measure (u16 ×2), swing (u8, 0 to 100: each pair of
rows' first held that much over its second, 100 being 3:1), the order
list being edited (u16), the order lists (u16 count; each a name, its
restart (u16) and its orders (u16 count, u16 each); ORDR is the one
being edited), bookmarks (u16 count; each a name, an order and a
pattern (u16, `FFFF` none) and a row (u16)), and the muted matrix slots
(u16 count; each an order and a channel, u16 ×2).

**PATX**: the patterns with something of their own (u16 count; each
its index (u16), flags (u8: 1 its own time, 2 its own swing), then
rows a beat and a measure (u16 ×2) and swing (u8) as the flags say, and
its automation lanes (u16 count; each a target (3 bytes: kind 1 global
volume, 2 tempo, 3 a channel's volume, 4 a channel's panning, 5 an
effect slot's parameter; then the channel or slot, and the parameter),
and its points (u16 count; each a row (u16) and a value 0 to 1 (f32)))).

**CHNX**: each channel's effect slot (u16 count, u8 each, `FF` none).

**SMPX**: each sample's credit (string), slices (u32 count, u32 frames
each), and synth (u8 1 if it has one, then wave, pulse width, FM ratio,
FM depth, beating and softening, u8 ×6).

**SYNV**: the samples that play a synth in their place (u16 count; each
the sample's index (u16) and its kind (u8)). Kind 1, an FM (OPL)
instrument: its eleven bytes as Scream Tracker 3 keeps an AdLib
instrument (the modulator's and carrier's registers 0x20, 0x40, 0x60,
0x80 and 0xE0 in turn, then 0xC0). Kind 2, an OctaMED synth sound:
hybrid (u8: its waveform 0 is the sample's own waveform), the volume and
waveform sequences' speeds (u8 ×2), the volume sequence and the waveform
sequence (each u16 count, then its bytes), and its waveforms (u16
count; each u16 points, then signed 8-bit points). Kind 3, a JamCracker
AM instrument, has nothing more: its sample holds its bytes (a 64-point
cycle twice, then a copy for each of four channels), and its waves are
those copies, then the cycle mixed with itself (each point the average,
rounded down, of the point and the one 0 to 63 on), which a reader makes
itself; `Fx` 43 picks between them. A reader without it plays those
samples silent (a JamCracker one as its bytes, a plain sample).

**PUMA**: the samples that play a Pumatracker instrument in their place
(u16 count; each the sample's index (u16); the samples its first
waveforms are (u16 count, then each sample's index); then its volume
script and its pitch script, each a u16 count of four-byte commands).
Its waveforms after those samples are the replayer's 42 built-in
cycles, which a reader makes itself. A reader without it plays those
samples silent.

**SELS**: Face The Music's SEL scripts: rows a measure (u16), the count
(u8), then each: its number (u8, 0 to 63), its line count (u16, at most
512) and its lines (4 bytes each: the command, then its three bytes, as
the file has them). A reader without it plays the song without them.

**MLED**: a Musicline Editor song, which plays by its own replayer:
its tunes (count u16; each its title, tempo u16, speed u8, groove u8,
volume u16, mixer mode u8, channel count u8 and each channel's 256
sequence words), its parts that aren't empty (count u16; each its
number u16, its line count u8 and lines of 13 bytes: the line, note,
instrument, five effects and their parameters), its arpeggio tables the
same way (lines of 7 bytes: the line, transpose, sample, two effects
and parameters), its instruments (count u16; each its 206 bytes as the
file has them and whether its sample came after it, u8) and its
samples' settings (count u16; each its name, kind u8, length u16 in
words, loop start u32 in bytes, loop length u16 in words, finetune and
semitone i16, and the points the file stored u32). The samples' points
are the module's samples, in order, made up to that many with silence.
A reader without it plays the patterns, which show the song roughly.

**PTRK**: a ProTrekkr or NoiseTrekker song, which plays by its own
replayer (neumetik-synth's `ptk`), packed as PATT is. Unpacked, it's
little-endian, a version byte (1) first: the file's version letter (u8); the sequence (256 bytes) and its
length (u16); its patterns (count u16; each its rows u8 and the cells
that aren't empty: count u32, then each its place u16 (row × 16 +
track), its sixteen notes, sixteen instruments, volume, panning and four
effects with their parameters); every pattern's rows (128 × u16); its
128 instruments (each its name, synth program u8, synth (its fields in
the order of `SynthParams`: bytes, i32s and the ADSRs as attack, decay
i32, sustain u8, release i32), volume f32, beat sync u8, beat rows u16
and its sixteen splits: u8 0 none, 1 mono, 2 stereo, then the split's
name, base note u8, loop start and end u32, loop kind u8, finetune i8,
amplify and filter decay f32 and its length u32); its sixteen tracks'
settings (twelve f32s: cutoff, inertia, panning, distortion threshold
and clamp, reverb send, volume, EQ low, mid and high, compressor
threshold and ratio; then filter kind, resonance and chorus send as
u32; distortion, surround, muted and compressor on as u8; polyphony,
notes, effect columns and denoise u8; the LFO (on u8, rate, filter,
volume, panning and scale f32); the flanger (on u8, amount, dephase,
rate, amplitude, feedback f32, delay u32)); the tracks shown (u8); each
position's tracks on (256 × u16 bit sets); tempo, rows a beat (u32
each), volume f32, shuffle u32; the reverb (on u8, threshold u32, decays
count u8 and f32s, feedback, cutoff, resonance, damping f32, stereo u8);
the chorus (delays and filter kind, cutoff and resonance u32, feedbacks
f32); the mix's compressor (threshold and ratio f32); the two TB-303s
(selected pattern, tune, cutoff, resonance, envelope, decay, accent,
waveform, scale u8; 32 pattern lengths; 32 × (16 tones, 16 flags);
volume f32). The splits' points are the module's samples, in order (a
stereo one's sides interleaved), each made up to its length with
silence. A reader without it plays the patterns, which show the song
roughly.

**TUNE**: a synth or chip tracker's song (AHX, HivelyTracker...), which
plays by its own replayer (neumetik-synth's `replay`): the format's id
(u8 length, then its letters: `ahx`, `hvl`...) and the file itself,
packed as PATT is. The song's own reader makes it ready to play again
(`replay::open`). A reader without it plays the patterns, which show the
song but make no sound (its instruments are samples with no points).

**INSX**: each instrument's envelope release nodes (volume, panning,
pitch: u8 ×3, `FF` none), MIDI output (u8 1 if it has one, then the
channel (u8), program (u8, `FF` none) and bank (u16, `FFFF` none)),
effect slot (u8, `FF` none), and phrase (u8 1 if it has one, then
looped (u8), the note it's written for (u8), and its cells (u16 count,
6 bytes each: note, instrument, volume command and value, effect and
parameter)).

**EFCT**: the effect slots (u16 count; each its kind (u8: reverb, delay,
chorus, flanger, distortion, compressor, equalizer, filter, gargle), a
name, its mix (f32), bypass (u8), the slot its sound goes into (u8,
`FF` the mix; only a later slot), and its parameters (u8 count, f32
each, 0 to 1)).

**MACR**: the MIDI macros, 16 parametered then 128 fixed (strings).

**VISL**: how the song likes to be seen as it plays (a string: a
neumetik-visual settings line, `bars;palette=fire;gain=1.5;...`).
Written only when the song keeps one; a player without a visualizer, or
one that doesn't know the method, passes it over.

**COVR**: the song's cover, always written: an ANSI art file (80 by 25
when painted; SAUCE and all, as Stylus opens it), LZ-packed as PATT is.
It's the song's own (art made in Stylus, or another painting) or, when
it has none, one painted from the song by neumetik-visual's cover
painter as the file is saved (`visual/docs/cover-art.md`). A reader
that doesn't show covers passes it over.

### END

Empty; the file ends here.

## Playing a UMF

1. Read the chunks.
2. For each sample with a waveform ID: take the waveform from the file's
   WAVE chunks if it has them, otherwise from the repository. Check it:
   its SHA-256 must be the ID, and its frames the slot's frames.
3. A waveform found nowhere leaves its sample silent; the player says
   which (`Loaded::missing`) so the user can import the soundpak that has
   it, or open a copy saved with its samples.
