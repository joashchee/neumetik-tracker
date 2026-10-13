# Every known music tracker format

This is the list neumetik-tracker works from: every tracker format known,
what it is, and whether Neumetik reads it. It was gathered from the
public record (the Modland archive's 300-odd format folders, the formats'
own documentation, and the trackers' release notes), checked against the
player lists of NostalgicPlayer (its players, its ProWizard and module
converters, and the libxmp formats it plays) and UADE (its eagleplayer.conf
and Wanted Team's players), and is kept here so "as many formats as
possible" has a list to be measured against.

Formats fall into four kinds, and only the first two concern the samples
repository (the third is read for the UMF library and the player):

1. **Sample trackers**: patterns of notes that play PCM samples. This is
   what "tracker" usually means, and what's read.
2. **Sample trackers with synthesis**: PCM samples alongside synthesized
   instruments (FM, wavetable, synth macros). Their samples belong in the
   repository; their synth voices are played by neumetik-synth (`synth/`),
   where every synth a format needs is written (the root README's rule).
3. **Synth and chip trackers**: no PCM at all, every sound made by a
   synth or a sound chip's emulation. Nothing for the repository; each
   is played by its own replayer in neumetik-synth (`synth/src/replay.rs`
   and a module a format), its synthesis and chips written there from
   scratch, as every synth is.
4. **Replayer formats and chip logs**: a composer's own player routine
   with its data (most of Modland's Amiga and C64 folders), or a sound
   chip's register log. Not trackers; listed so the line is drawn in the
   open.

Status: **Read** (neumetik-tracker reads it into a `Module`, plays it and
scans its samples), **Planned** (a tracker not read yet, in rough
order of how much music there is in it), **Out of scope** (not a
tracker).

## 1. Read

| Format | Extensions | Trackers | Notes |
| --- | --- | --- | --- |
| ProTracker MOD (M.K., M!K!, M&K!) | .mod | ProTracker 1-3, Fasttracker, OpenMPT and countless others | 4 channels, 31 samples; Amiga periods, ProTracker's limits |
| NoiseTracker (N.T., FEST, LARD, NSMS) | .mod, .nst | NoiseTracker, its hacked versions | read as MOD |
| StarTrekker (FLT4, FLT8, EXO4, EXO8) | .mod | StarTrekker | FLT8's paired 4-channel patterns joined into 8 |
| Multichannel MOD (xCHN, xxCH, xxCN, TDZx) | .mod | Fasttracker 1, TakeTracker | 1 to 99 channels; Fasttracker 1's own format (it wrote nothing else) |
| Oktalyser and Digital Tracker MOD (CD81, OKTA, OCTA, CD61, FA0x) | .mod | Oktalyser (Atari), Digital Tracker (Falcon) | 6 and 8 channels |
| Mod's Grave | .wow | Mod's Grave | an 8-channel MOD tagged M.K., found by its length |
| Ultimate SoundTracker and the 15-sample SoundTrackers | .mod, .stk, .m15 | Ultimate SoundTracker 1.0 to 2.0, Master SoundTracker 1.0, D.O.C. SoundTracker II, IV, VI and IX, SoundTracker 2.2 | no tag: recognized by checks that stand in for one |
| SoundTracker 2.6 and Ice Tracker (MTN, IT10) | .mod, .mtn, .ice | SoundTracker 2.6, Ice Tracker 1.0 and 1.1 | patterns built of tracks; IT10 is Ice Tracker's own (its only) format; F is always speed, both nibbles set a swing between the two |
| SoundFX 1.3 and 2.0 | .sfx, .sfx2 | SoundFX 1.x, SoundFX 1.99 and 2.0 (MultiMedia Sound, SO31) | 15 or 31 samples, CIA tempo |
| Scream Tracker 3 | .s3m | Scream Tracker 3, Impulse Tracker, Schism, OpenMPT, BeRoTracker, Imago Orpheus | ADPCM4 samples too; AdLib (OPL2 FM) instruments played by neumetik-synth on the nine AdLib melody channels (its drum channels stay silent) |
| Scream Tracker 2 | .stm | Scream Tracker 2, BMOD2STM | |
| Scream Tracker Music Interface Kit | .stx | STMIK (STM2STX 1.0 and 1.1) | Scream Tracker 2's song in an early S3M layout; played as STM is |
| FastTracker 2 | .xm | FastTracker 2, MilkyTracker, MadTracker, OpenMPT, Skale | version 1.04; delta samples, ModPlug ADPCM, OpenMPT's stereo samples |
| OggMod XM | .oxm | oggmod | an XM whose samples are Ogg Vorbis, decoded by Neumetik's own decoder (a stereo sample's two halves one after the other) |
| Impulse Tracker | .it | Impulse Tracker 1 and 2, Schism, OpenMPT, ModPlug | IT 2.14 and 2.15 compressed samples, old (IT 1) instruments |
| OpenMPT | .mptm | OpenMPT | read as IT; OpenMPT's extensions after the IT data are passed over |
| MultiTracker | .mtm | MultiTracker | |
| Composer 669, UNIS 669 | .669 | Composer 669, UNIS 669 | per-pattern speed and break row turned into effects |
| PolyTracker | .ptm | PolyTracker | delta samples |
| General DigiMusic | .gdm | 2GDM, BWSB | GDM converted from MOD, MTM, S3M, 669, FAR, ULT, STM, MED, PSM |
| Digital Sound Interface Kit | .dsm | DSIK | the RIFF DSMF format |
| UltraTracker | .ult | UltraTracker 1.3 to 1.6 | two effects a cell (one goes to the volume column) |
| Farandole Composer | .far | Farandole Composer | |
| Imago Orpheus | .imf | Imago Orpheus | instruments with envelopes, two effects a cell |
| Oktalyzer | .okt, .okta | Oktalyzer | 4 to 8 channels (each of 4 may be split) |
| OctaMED (MMD0, MMD1, MMD2, MMD3, MMDC) | .med, .mmd0-3, .mmdc | MED, OctaMED, OctaMED Soundstudio, MED Packer (MMDC: an MMD0 with run-length packed blocks) | sections and play sequences flattened to orders; tempos timed as OctaMED's own players time them (the PAL CIA timer, SoundTracker's speeds for 1 to 10, 8-channel mode's mixing buffers), rounded to whole beats a minute; synth and hybrid sounds played by neumetik-synth (a hybrid's sample kept in the repository) |
| DigiBooster | .digi | DigiBooster 1.x | packed and unpacked patterns |
| DigiBooster Pro | .dbm | DigiBooster Pro 2 | volume and panning envelopes, two effects a cell |
| Asylum | .amf | the Crusader games' music | |
| Images Music System | .ims | Images Music System | a MOD with three-byte cells; where the tag would be, where the samples start |
| Chiptracker | .mod, .kris | Chiptracker | tagged KRIS at 952; four transposed tracks an order; its unfinished synth sounds have nothing to play |
| Game Music Creator | .gmc | Game Music Creator | no title or tag: found by its sample headers and order list |
| Quadra Composer | .emod | Quadra Composer | IFF: EMIC, PATT, 8SMP |
| Digital Tracker | .dtm | Digital Tracker (Falcon), Digital Home Studio | IFF-like chunks; MOD, 2.04 and 2.06 (tick-based) patterns; stereo and 16-bit samples; Digital Home Studio's instruments |
| Astroidea XMF | .xmf | Astroidea's player (Imperium Galactica) | types 2 to 4; types 2 and 3 slide to a new note on their own, which isn't kept |
| Composer 670 (CDFM) | .c67 | CDFM | four PCM channels, and nine FM (OPL2) channels played by neumetik-synth |
| Funktracker | .fnk | Funktracker, Funktracker GOLD | its slides go on until stopped; here they play on the rows they're written; some of its effects (fan-in vibrato, reverb, volume crest) aren't kept |
| Disorder Tracker 2 | .plm | Disorder Tracker 2 | one long pattern of blocks laid along the song, read as patterns of 64 rows |
| Real Tracker | .rtm | Real Tracker | XM-like objects: instruments with envelopes, two effects a cell |
| ProTracker 3.6 | .ptm, .mod | ProTracker 3.6 | an IFF FORM MODL around a MOD tagged PATT, with its title, author and comments |
| Dynamic Studio | .dsm | Dynamic Studio Professional | its own (not DSIK's): up to 16 channels, 3D panning, 16-bit samples |
| Composer 667 | .667 | Composer 667 | 18 FM (OPL2) channels, no PCM: neumetik-synth plays them all |
| Graoumf Tracker | .gtk | Graoumf Tracker 1 (versions 1 to 4) | 16-bit effects (12-bit parameters); its logarithmic volumes made linear |
| DSMI Advanced Module Format | .amf, .dmf | DMP, DSMI; Webfoot's games (.dmf, compact, delta samples) | versions 1 and 8 to 14; tracks shared between patterns, each order its own pattern |
| AHX (Abyss' Highest eXperience) | .ahx, .thx | AHX 1 and 2 (Dexter/Abyss) | synth: no samples. Every sound a triangle, sawtooth, square (its pulse width moving) or noise, through one of 63 filters, under an envelope, stepped by a play list; played by neumetik-synth's `ahx` (the original replayer's waveforms and its 49.92 Hz ticks), checked against the original in UADE (chroma 0.89 to 1.00) |
| HivelyTracker | .hvl | HivelyTracker (Pete Gordon) | AHX with up to 16 channels, panning, ring modulation and two effects a cell; played as HivelyTracker's own replayer plays it (its floating-point filters, 50 Hz), matching it on every test song |
| Future Composer 1.3 and 1.4 | .fc, .fc13, .fc14, .smod | Future Composer (SuperSero, Jochen Hippel) | synth: each channel its own patterns; instruments of a frequency and a volume sequence over waveforms (1.3's own 47, 1.4's in the file) and samples (1.4's in sets of 20); its samples go to the repository. Played by neumetik-synth's `fc` through its Paula, ticked by the CIA at 50 a second; checked against the original replayers in UADE (chroma 0.89 to 1.00, mean 0.98) |
| SidMon 1 | .sid, .sid1, .sd1 | SidMon 1 (Reinier van Vliet) | synth: songs ripped with their replayer, read by following its instructions to the data; 32-point waveforms (stepped through lists, two remade as mixes) and samples under an envelope, a 16-step arpeggio, a phase shift, a pitch fall and bends; its samples go to the repository. Its periods are its replayer's own table, read from the rip; its speed counter starts where the rip left it. Checked against UADE (chroma 0.64 to 0.98, mean 0.90) |
| SidMon 2 | .sid2, .sd2 | SidMon 2 ("SIDMON II - THE MIDI VERSION") | synth: positions of a track a channel (transposed in note and instrument), packed rows; samples stepped through waveform lists under an envelope, arpeggio and vibrato lists, a pitch bend and a point of each sample inverted as it plays; ProTracker's effects. Its samples go to the repository. Checked against UADE (chroma 0.85 to 1.00, mean 0.97) |
| BP SoundMon 2.0 and 2.2 | .bp, .bp3 | Brian Postma's SoundMon ("V.2", "V.3") | synth: steps of a 16-row track a channel; samples and synth sounds, a waveform from the song's wave tables under an ADSR and an LFO, remade as it plays by an EG (part inverted), averaging, morphing toward another and a modulation. Its samples go to the repository. Checked against UADE (chroma 0.74 to 1.00, mean 0.95) |
| Delta Music 1 and 2 | .dm, .dm2 | Delta Music (Bent Nielsen), 1.3 ("ALL ") and 2.0 (".FNL") | synth: each channel its own list of 16-row blocks, looping on its own; samples and synth sounds stepping through a table of waveforms (2.0's first remade as noise each tick), under an envelope (2.0's in five stages), vibrato, bends, portamento and an arpeggio (1.3's every setting changeable by the song's effects). Their samples go to the repository. Checked against UADE (1.3: chroma 0.91 to 1.00, mean 0.97; 2.0: 0.72 to 1.00, mean 0.96) |
| Digital Mugician 1 to 3 | .dmu, .mug | Digital Mugician (Softeyes) | synth: up to 8 sub-songs of 64-row tracks a channel; samples and 128-point waveforms remade as they play by 15 effects (filters, mixes, scrolls, re-sampling, negation, morphs, gates), a volume and a pitch read from waveforms, arpeggios and bends. 2 and 3 play 7 channels, four of them mixed into one of the Amiga's, ticked once a mixing buffer. Their samples go to the repository. Checked against UADE (1: chroma 0.91 to 1.00 where both play; 2: 0.80 to 0.87; 3: no reference plays it) |
| Sonic Arranger | .sa | Sonic Arranger (Carsten Schlote, Branko Mikic) | synth: "SOARV1.0" songs and "final" ones (the replayer with the song after it); sub-songs of positions, each channel a run of the song's rows; samples and 128-point waveforms remade as they play by 17 effects (negators, rotations, morphs, filters, noise, an FM drum) under ADSR and AMF tables, vibrato, portamento, slides and arpeggio tables; ticked at each song's own tempo. Its samples go to the repository. Checked against UADE (chroma 0.82 to 0.99, mean 0.93 where UADE plays) |
| InStereo! 1.0 and 2.0 | .is, .is20 | InStereo! ("ISM!V1.2", "IS20DF10"), Sonic Arranger's forerunner | synth: sub-songs of positions, each channel a run of the song's rows; samples and synth sounds (1.0's waveform with its start inverted as far as an envelope table says; 2.0's two waveforms stepped through by an envelope generator), ADSR tables, vibrato, portamento between notes, slides and arpeggios. Their samples go to the repository. Checked against UADE (chroma 0.76 to 0.98, mean 0.91) |
| Music Assembler | .ma | Music Assembler (Oscar Giesen and Marco Swagerman), rips: the replayer's code before its data, which its own instructions find | synth: sub-songs of a position list a channel (tracks transposed and repeated), packed rows each held its own count of rows; samples and short waveforms whose start sweeps back and forth, an attack to a held sustain and a release, a three-note arpeggio, vibrato, portamento. Its samples go to the repository. Checked against UADE (chroma 0.95 to 1.00, mean 0.98) |
| Synthesis 4.0 and 4.2 | .syn | Synthesis ("Synth4.0", or "Synth4.2" after its own player), InStereo! 1.0's sibling | synth: laid out as InStereo! 1.0; synth waveforms with their start inverted as far as an envelope table says, or reshaped as they play by 16 effects (rotations, negations, other waveforms mixed in, octave folds, smoothing, laser and FM drum pitch sweeps); ADSR tables, arpeggios from the instrument or the row, vibrato, portamento between notes, slides. Its samples go to the repository. Checked against UADE (chroma 0.85 to 1.00, mean 0.96) |
| Voodoo Supreme Synthesizer | .vss | Voodoo Supreme Synthesizer (Tomas Partl; "VSS0" in its last 64 bytes) | synth: each sub-song a table of structures, its first four the channels' tracks (notes each held so many ticks, with calls, loops and jumps); volume envelopes, period tables (steps, octaves, semitone shifts), portamento; two waveforms made into a third every tick (ring modulation, XOR or morphing over a moving span) or a sample stepped through block by block at the audio interrupt. Its samples go to the repository. Checked against UADE (chroma 0.82 to 1.00, mean 0.94) |
| Epic MegaGames MASI | .psm | MASI (Epic Pinball, Jazz Jackrabbit, One Must Fall 2097, Sinaria) | new (and Sinaria's) and old (PSM16) layouts; subsongs as order lists |
| Liquid Tracker | .liq | Liquid Tracker | LIQ 0.00 to 1.x (packed patterns, stereo samples) and the older NO modules |
| Archimedes Tracker | .musx | Archimedes Tracker | logarithmic volumes and VIDC samples made linear; its slides of the logarithmic volume are close, not exact |
| Digital Symphony | .dsym | Digital Symphony (Archimedes) | LZW-packed blocks; μ-law, LZW and sigma-delta samples; its loops last while the note's held |
| Megatracker | .mgt | Megatracker (Falcon) | tracks shared between patterns; FastTracker 2's volume column; its second effect parameter isn't kept |
| Cubic Tiny XM | .mxm | Cubic Player's XM converter (Cubic Player's demos) | an XM made small; both of its sample layouts (lengths, and the card's addresses) |
| MikMod UNITRK | .uni | MikMod's converter | UN04 and UN05 (APlayer's and UN06 aren't read): tracks of MikMod's own commands, as the tracker it came from played them |
| Extreme's Tracker | .ams | Extreme's Tracker 1.x | packed samples (runs, bit planes, deltas) |
| Velvet Studio | .ams | Velvet Studio 2.00 to 2.02 | instruments with envelopes; shadow samples (another instrument's data with settings of their own); samples set to play backwards play forwards |
| Digitrakker | .mdl | Digitrakker 2 and 3 | its own packed samples; instruments of note ranges, envelopes; EFx's offset with its second column's byte |
| X-Tracker | .dmf | X-Tracker (D-Lusion) | Huffman-packed samples; the tempo (ticks a quarter-second or beats a minute) as a channel of its own, each order its own pattern; effect times in 256ths of a row made ticks; its loops last while the note's held |
| SoundTracker Pro II | .stp | SoundTracker Pro II | versions 0 to 2; its auto effects written into every row they play on; a sample's first loop kept (the rest, and the loop sequences that play them, aren't) |
| MadTracker 2 | .mt2 | MadTracker 2 | the drums' eight channels; stereo samples; groups (a sample, volume and pitch) for a note; automation, VST and MIDI instruments and the instruments' filters aren't kept (the filter envelope is) |
| Graoumf Tracker 2 | .gt2 | Graoumf Tracker 2 (Falcon and Windows) | chunks: instruments, envelopes, mix presets; the first instruments' envelope programs aren't run (a released note fades instead), nor its DSP (delays, filters), auto tempo and auto period, detune or logarithmic slides |
| Pumatracker | .puma | Pumatracker | every instrument a volume and a pitch script over the song's ten samples and 42 built-in cycles, played by neumetik-synth; the scripts' volume scales the channel's rather than adding to it |
| Jazz Jackrabbit 2 (Galaxy Sound System) | .j2b | Epic's converter for Jazz Jackrabbit 2 | the zlib-packed RIFF AM (and older AMFF) inside: instruments with envelopes, IT's rules |
| DigiTrekker | .dtm | DigiTrekker 3 | tracks shared between patterns; slides in 128ths of a semitone; a channel's left and right volumes made its panning and volume; the demo song's older speed chunk |
| PlayerPro (MADF, MADG) | .mad | PlayerPro (Mac), its first formats | MAD1-packed patterns; MOD's effects |
| PlayerPro (MADH, MADI, MADK) | .mad, .madk, .madi, .madh | PlayerPro 4 and 5 (MADK from 5.0) | XM's cells, instruments and envelopes in PlayerPro's own layout; its multichannel mode (each note a voice of its own) and its effect plug-ins aren't kept |
| Symphonie | .symmod | Symphonie, Symphonie Pro | run-length packed blocks; delta-packed 8- and 16-bit samples, and IFF, WAV, AIFF or raw ones; virtual and transwave instruments rendered; its sample processing (filters, fades, normalizing, over-amplifying, sustain repeats) done when read; effects that run at the output's rate made ticked ones row by row; its DSP (echo, delay, filter) isn't kept |
| Face The Music | .ftm | Face The Music | measures made patterns; loops written into the order list; pitch bends and volume fades written into the rows they last for; its SEL scripts run by neumetik-synth (41 of Modland's 95 use them) |
| Fashion Tracker | .ex | Fashion Tracker | a replay routine with its song after it: the song found from the code, both versions; the second's arpeggio |
| Digital Sound Studio | .dss | Digital Sound Studio | its own commands made ProTracker's; the master volume's slides aren't kept |
| Ken's Digital Music | .kdm, with WAVES.KWV | Ken Silverman's (Ken's Labyrinth, his demos) | notes in time at 120 ticks a second made rows (note delays between), its 16 voices as channels; fades and vibratos written out; the pitch scoops and panning curves aren't kept; the waves come from WAVES.KWV beside it |
| Digital Sound Interface Kit (first format) | .dsm | DSIK 1 | "DSM" 10h: packed patterns with obfuscated flag bytes (the RIFF format is read above) |
| StarTrekker AM | .mod with .nt | StarTrekker 1.2 and 1.3, AudioSculpture | the module read as MOD; the AM synth's instruments from the .NT beside it, played by neumetik-synth (a looped cycle under its envelope, with its vibrato and octave); the pitch fall isn't kept |
| MO3 | .mo3 | Un4seen's MO3 packer | an IT, XM, S3M, MOD or MTM made small: its music LZ-packed; samples raw, MO3's own delta packing, or Ogg Vorbis (decoded by Neumetik's own Vorbis decoder, shared headers too); MP3 samples (rare) stay silent: there's no MP3 decoder |
| Skale Tracker | .skm | Skale Tracker | no published description: worked out from the files (chunks; tracks of masked cells, FastTracker 2's effects; instruments with a 96-note keymap); samples raw or Ogg Vorbis; its envelopes aren't kept |
| MED (MED2, MED3, MED4) | .med | MED 1.12 to 3.x | the MED before OctaMED's MMD: nibble-packed blocks, synth and hybrid sounds (MED4) played by neumetik-synth; a song file's instruments read from the files beside it |
| TCB Tracker | .tcb, .mod | TCB Tracker (Atari ST) | its pitch bends made portamentos; its sample interrupt and continue aren't kept |
| EasyTrax | .etx | EasyTrax | its volume-down command isn't kept |
| Chuck Biscuits / Black Artist | .cba | CBA (Expoze) | MOD's commands, played by Scream Tracker's rules |
| SoundSmith, MegaTracker | .ss, .mtp (or none) | SoundSmith, MegaTracker (Apple IIgs) | the waves from the DOC RAM file beside the song (.W, .D, .DOC, .WB, DOC.DATA) or its instruments' ASIF files |
| FM Tracker | .fmt | Davey W Taylor's FM Tracker | eight OPL2 channels, played by neumetik-synth |
| His Master's Noise | .mod | NoiseTracker (His Master's Noise) | its "Mupp" wavetable sounds played by neumetik-synth's synth sequences; its mega-arpeggio isn't kept |
| Coconizer | .coco | Coconizer (Archimedes) | VIDC samples and logarithmic volumes made linear; its auto slides aren't kept |
| Magnetic Fields Packer | mfp.* with smp.* | Magnetic Fields Packer | the samples from the smp. file beside it |
| A.M. Composer | .amc | A.M. Composer 1.2 (Magic Jumper, Krymini) | a replayer's song: each voice a list of notes and a list of effects, read a row at a time as the replayer reads them; its periods made notes, its slides (a step every so many ticks) porta row by row, its arpeggios of eight periods a tick the tracker's arpeggio (of the same notes); its CIA tempo rounded to a whole beat a minute |
| Editeur Musical Sequentiel | .ems | Thomas Pimmel's EMS 1.22 to 1.25 (Modland's "Electronic Music System"; not EMS v6) | a MOD-like song of 15 samples; a row is the speed's ticks and one more; a note with no sample changes the pitch without striking it (a tone porta that arrives at once); its tick-by-tick pitch swing kept as a fast vibrato, its whole-sample loop command not kept; samples at the PAL Amiga's rate |
| JamCracker | .jam | JamCracker (Martin Gemmel's) | samples and AM waves (a 64-point cycle mixed with itself at a moving phase, played by neumetik-synth); the replayer's effects last from row to row and work on every tick, so it runs a tick at a time and each row is written with what it plays: the volume it starts at and its slide, pitch slides spread over the row (a jump a slide to the nearest note at once), the arpeggio (clamped at the period table's top, as the replayer clamps it) and the vibrato (its triangle made MOD's sine) on every row they last; one effect a cell, so a row with more keeps the pitch, then the volume, then the vibrato, and a pluck's slide over its arpeggio; the arpeggio starts again each row, where the replayer's runs on; each order its own pattern (shared when they come out the same); speeds in a fifth channel when a row has no free effect; samples at the PAL Amiga's rate |
| AMOS Music Bank | .abk | AMOS (its music bank, mostly SoundTracker and GMC songs converted) | each track a list of notes, waits and commands, its lasting effects written into every row they last; the tempo (rows at half of it a second) a speed and the beats a minute that time it; the first song of a bank; channels with different play lists get a pattern for each order |
| Musicline Editor | .ml | Musicline Editor 1.x (4 channels, or 8 mixed) | plays by its own replayer in neumetik-synth (`musicline`): each channel its own sequence and speed, instruments whose waves are remade every tick (morph, phase, mix, resonance, filter) or whose loop moves; every tune a sequence; the patterns lay the song out as it plays (a row wherever a channel reads one), for the screen, and editing them doesn't change what plays |
| ProTrekkr, NoiseTrekker | .ptk, .ntk | ProTrekkr (every version, 3 to X, tagged TWNNSNG or PROTREK), NoiseTrekker (versions 1 and 2) | plays by its own replayer in neumetik-synth (`ptk`), checked against ProTrekkr's own to the sample: sixteen tracks each through its filter (24 kinds), distortion, flanger, compressor, EQ and LFOs into a chorus and a reverb; instruments of up to sixteen split samples and a two-oscillator synth; two TB-303s. Each track's notes (up to sixteen a row) are a channel each on the screen; editing the patterns doesn't change what plays. NoiseTrekker's songs clip, as they do when ProTrekkr loads them |
| Packed ProTracker modules | p61a., np2., pp21., tp3., unic., ... (the packer's prefix, or none) | 40 packers: AC1D, Digital Illusions, Eureka, FC-M, Fuchs Tracker, Fuzzac, Heatseeker mc1.0, Hornet, Kefrens Sound Machine, Module Protector, NoisePacker 1 to 3, NoiseRunner, NovoTrade, Pha, The Player 4.0A, 4.0B, 4.1A, 5.0A, 6.0A and 6.1A, Promizer 1.0c and 1.8a, ProPacker 1.0, 2.1 and 3.0, ProRunner 1 and 2, SKYT, STIM (Slam Tilt), StarTrekker Packer, The Dark Demon, Titanics Player, Tracker Packer 1 to 3, Unic Tracker 1 and 2, Wanton, Xann, Zen | each turned back into the ProTracker MOD it was made from (`packed.rs`, from ProWizard's descriptions) and read as one: the same MOD ProWizard makes, byte for byte, for every test module (ProPacker 1.0's loop starts apart, where ProWizard sets an unlooped sample's to 1; STIM's checked against ProWizard's retired STIM depacker); The Player's delta and 4-bit packed samples; Promizer's finetuned periods made notes; a MOD isn't taken for a packer that keeps a MOD's tag (Unic, NoiseRunner) when its patterns read as a MOD's. Files crunched with XPK (SQSH) aren't opened |
| Unreal packages | .umx | Unreal, Unreal Tournament, Deus Ex... | the IT, S3M, XM or MOD inside |
| Packed modules | .zip, .gz, .mdz, .s3z, .xmz, .itz, .mdgz... | | the first module inside, inflated by Neumetik's own inflate |
| **UMF** | .umf, .ntt | Neumetik | the Universal Mod Format: see `umf-format.md` |

## 2. Planned: sample trackers not read yet

| Format | Extensions | Trackers | Notes |
| --- | --- | --- | --- |
| Sound Club | .sn, .sn2 | Sound Club 1 and 2 | patterns and samples bit-packed in a scheme no public description covers yet |
| Desktop Tracker | (no ext.) | Desktop Tracker (Archimedes) | no modules or description found yet (Archimedes Tracker's .musx is read) |
| Stonetracker | .spm, .sps | Stonetracker | song and samples in two files; its sample banks are DeltaHuffman-packed and its patterns packed; its developer documentation is in its own packed archives, unread |
| DreamStation | .dss | DreamStation | "DSFmt1": no description found yet (Digital Sound Studio's .dss is read) |
| Beaver Sweeper | .gtk | Beaver Sweeper | "+SNT": no description found yet (Graoumf Tracker's .gtk is read) |
| BeRoTracker | .brt | BeRoTracker | chunks (BRTF, INFO, BPMI...); no description found yet |
| Renoise | .xrns, .rns | Renoise | zip of XML and FLAC samples (FLAC decoding needed) |
| Psycle | .psy | Psycle | samples beside machines |
| Buzz | .bmx, .bmw | Jeskola Buzz | machines; samples in the wavetable |
| SunVox | .sunvox | SunVox | samples beside synth modules |
| MVS Tracker | .mus | MVS Tracker | |
| Polly Tracker | .mod | Polly Tracker | RLE-packed data; no description found yet |
| SynTracker | .synmod | SynTracker | |
| Sound Master | .smst | Sound Master | |
| Furnace (sample channels) | .fur | Furnace | mostly chips; its PCM samples could be kept |
| Flextrax | .flx | Flextrax (Falcon) | a MOD (M.K. or xCHN) with its DSP effects after it; no modules or description found yet |

### Packed ProTracker modules not read yet

Most games and demos shipped their MODs packed: the same song, its
patterns and sample headers rearranged (and often shrunk) by a ripper's or
a demo group's packer, its samples raw. ProWizard converts them back to
ProTracker MODs, and NostalgicPlayer and UADE play them so. 40 are read
(above); these, from NostalgicPlayer's ProWizard and UADE's players, aren't
yet, for want of test modules:

Alcatraz Packer, Ben Replay, Binary Packer, Blade Packer, Channel Player
1 to 3, Devils Replay, GnoiPacker, GnuPlayer, Gnu Protected Module, HCD
Protector, Janne Salmijarvi Optimizer, Laxity Tracker,
Module-Patterncompressor, Mosh Packer, Newtron Packer 1.0 and 2.0, Nick
Pelling Packer, Perfect Song 1 and 2, Polka Packer, Power Music, Promizer
0.1, 2.0 and 4.0, The Player 2.2A and 3.0A, TimeTracker
(TMK Replay), Tomy Tracker. Modules crunched with XPK (SQSH, the
"XPKF" files) need XPK unpacked first.

## 3. Planned: synth and chip trackers (no PCM)

Each to be played by its own replayer in neumetik-synth, its synthesis or
chip written there (AHX, HivelyTracker, Future Composer, SidMon 1 and 2, SoundMon 2.0 and 2.2, Delta Music 1 and 2, Digital Mugician, Sonic Arranger, InStereo!, Music Assembler, Synthesis and Voodoo Supreme Synthesizer, above, are read):

Future Composer BSI, Future Player,
BP SoundMon 1, Synth Dream, Synth
Pack, Dynamic Synthesizer, Beathoven
Synthesizer, Mark II, Medley, Music
Editor, The Musical Enlightenment, Pretracker, Sawteeth, Tunefish, V2, Chiptracker
(synth mode), JayTrax, Klystrack, GoatTracker 1 and 2, Cybertracker
(C64), Vic-Tracker, SID Duzz' It, FamiTracker, NerdTracker 2, Deflemask,
Furnace (chip channels), ArkosTracker, Starkos, AY STRC, AY Amadeus, Vortex
Tracker, Monotone, Beepola, PokeyNoise, Shroom, MoonBlaster, Sound
Programming Language, SCC-Musixx, TFM Music Maker, Organya 1
and 2, Gameboy Tracker, LSDJ, Piston Collage, Ixalance, Buzzic 1 and 2,
DarkWave Studio's synths, OPLLDRV, MGSDRV, MuSICA, PMD, MDX, FMP, Quartet.

## 4. Out of scope: replayer formats and chip logs

The Amiga's composer-specific players (Modland's folders named after
composers and their routines): Rob Hubbard (and his ST player), Ben
Daglish (and his SID-like player), David Whittaker, Jason Page, Jeroen
Tel, Jochen Hippel (and Hippel 7V, COSO, ST, MCMD), Martin Galway's, Tim
Follin's (Follin Player II), Wally Beben, Richard Joseph (and his Player
1.7A), Mark Cooksey (old and new), Dave Lowe (old, new and Deli), Fred
Gray, Paul Shields, Paul Robotham, Paul Summers, Paul Tonge, Steve
Barrett, Steve Turner, Sean Connolly (his EMS 3.01 to 6), Sean Conran,
Howie Davies, Mike Davies (Tiertex), Kris Hatlelid, Jesper Olsen, Anders
Oland, Andrew Parton, Darius Zendeh, Thomas Hermann, Peter Verswyvelen,
Janko Mrsic-Flogel, Dirk Bialluch, Ashley Hogg, Ron Klaren (CustomMade),
David Hanney, Jason Brooke, Kim Christensen, Martin Walker, Scott
Johnston, Major Tom, Maniacs Of Noise, Core Design, Cinemaware,
Infogrames, Silmarils, Lionheart, Art And Magic, Unique Development,
onEscapee, UFO (MicroProse), Maximum Effect, NTSP-system, AProSys, TFMX
(1.5, Pro, 7V, ST), Images Music System's game players, Delitracker
custom players, Actionamics, Activision Pro, EarAche, Fred Editor
(FredMon), GlueMon, G&T Game Systems, Riff Raff, SoundControl 3 to 5,
SoundFactory, SoundPlayer, Special FX (and ST), Speedy System and Speedy
A1 System, SunTronic, Tronic, Zoundmonitor, IFF-SMUS and Sonix Music
Driver (SMUS, TINY, SNX), Music-X Driver, MaxTrax, Professional Sound
Artists, Sound Images, Leggless Music Editor, MusicMaker V8 (4 and 8
voices), Art Of Noise (4 and 8 voices), Compoz, Desire, Digital Sonix And
Chrome, Digital Sound Creations, Forgotten Worlds, Pierre Adane Packer,
Delta Packer, MPK, Nintendo and Sega game rips.

Chip logs and console formats: SID (HVSC, RealSID, Stereo Sidplayer,
Sidplayer), SNDH, SC68, YM, YMST, VGM, GYM, CYM, S98, KSS, HES, NSF, GBS,
GBR, SPC, SNSF, PSF, PSF2, USF, SSF, DSF, 2SF, QSF, Capcom Q-Sound, Ultra64,
WonderSwan, Sam Coupe COP and SNG, Spectrum, Atari Digi-Mix, Westwood SND,
SCUMM, Sierra AGI, Euphony, MIDI-Loriciel, AND XSynth, Ad Lib (IMF, RAD, A2M, D00 and
the other OPL formats: no PCM). ADPCM mono (UADE's) is a sampled
stream, not music written in notes.
