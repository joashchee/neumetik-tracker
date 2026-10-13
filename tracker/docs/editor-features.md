# The UMF editor: what it does, and what's to come

The Tracker tab's editor (`app/src/components/TrackerPanel.tsx`) follows
the trackers people know best, FastTracker 2, Impulse Tracker and
OpenMPT (with ProTracker, Scream Tracker 3, MilkyTracker, Schism Tracker
and Renoise for reference), and did first what they all do, then what only some of them do. Both are
listed below; the second list says which tracker each came from, and
where Neumetik's is narrower than that tracker's.

## Done: what every tracker does

**Song**: open any format read (and UMF), new, save as UMF (with or
without its samples), export WAV, undo and redo (200 steps), title,
artist, message, speed, tempo, global volume, mix level, restart order,
linear or Amiga slides.

**Orders**: the list, the pattern at a position up and down, insert and
delete positions, a new pattern or a copy of this one after the
position, the playing position shown.

**Patterns**: the grid of notes, instruments, volume column and effects,
in the DOS colors; rows 1 to 256; cursor movement by arrows, Tab, Page
Up/Down, Home/End and the mouse; the piano on the computer's keyboard
(two octaves, FastTracker's and Impulse Tracker's layout) with the
octave chosen; note off, cut and fade; hex entry for instruments, volume
and effect values, letters for the volume column's commands and the
effects; edit step; edit on and off (Space), the piano playing either
way; clear field or block; insert and delete rows in a channel or all;
block marking (Shift+arrows, Cmd+A), copy, cut and paste; transpose by
semitone or octave; follow the song as it plays; channel mute and solo,
each channel's level shown.

**Playing**: the song, the pattern over and over, from the cursor, stop
(F5 to F8); notes played as they're typed, through the song's own
instruments and samples.

**Samples**: the list; the waveform with its loops drawn; name, volume,
global volume, C-5 rate (and a semitone up or down), panning, loop and
sustain loop (off, forward, ping-pong, start, end), auto-vibrato (kind,
speed, depth, sweep); load and save WAV; normalize, reverse, trim to the
loop; clear; play it; take one from the repository.

**Instruments**: turning instruments on for a song that plays samples
directly; the list; name, global volume, fadeout, new note action; the
keymap by ranges; the volume, panning and pitch envelopes drawn and
edited by hand (points added, moved and taken out; on, sustain, loop,
filter).

**Channels**: names, panning, volume, adding and removing.

**The repository**: storage used and what it saved, search, categories
and tags, sorting, a sample's details (names, waveform, loops, pitch,
every module it was found in and where), filing (categories, tags,
rating, notes), previewing, using a sample in the song, saving it as
WAV, removing it, scanning a folder, exporting and importing soundpaks.

## Done too: what some trackers do

**Pattern editing**
- A block's volumes or effect values interpolated (IT's Alt+K and Alt+X,
  OpenMPT's), amplified or faded (IT's Alt+J).
- OpenMPT's paste modes: mix (into empty fields), fill (into empty cells,
  IT's mix paste), flood (to the pattern's end), insert (push forward).
- Instruments replaced in a block or the whole song, and two swapped
  through the song, their places and every note (IT, FT2).
- Chords typed across channels (FT2's multichannel record): notes held
  together go into channels side by side.
- Recording from a MIDI keyboard (IT, OpenMPT, Renoise), its velocity to
  the volume column; the MIDI input chosen in Settings.
- Template notes (OpenMPT's: Cmd+T makes the cell under the cursor the
  template, notes typed get its volume and effect) and chord macros
  (each key writes a chord across channels).
- Renoise's phrases (an instrument's column of notes, played from its
  key, transposed) and pattern-matrix slots muted (an order's channel
  silent there). Renoise's aliases (one slot shared by many) aren't.
- Per-pattern rows a beat and a measure, and swing, over the song's
  (OpenMPT, Renoise); the grid highlights them.
- Pattern names shown in the order list and edited beside it (OpenMPT).
- Find and replace: notes, instrument, volume column and effect, by
  value ranges, in the song, a pattern or a block; replace sets,
  transposes or clears (IT, OpenMPT).
- The effect under the cursor in words, and OpenMPT's effect dialog
  (Cmd+E): the command from a list, its parameter whole or by nibbles.
- Bookmarks of a pattern's row (Cmd+B) and order, and several order
  lists in one song (OpenMPT's sequences, MED's sections).

**Playback**
- Play the row under the cursor (IT's 8) and its note (IT's 4); play from
  the cursor in the pattern only (Shift+F7).
- Channel faders as the song plays (IT's info screen), in the Mix tab.
- Interpolation chosen: none (the Amiga's, with its A500 low-pass and the
  LED filter E00/E01 on MODs), linear (FT2's), cubic (IT's), windowed
  sinc (OpenMPT's).
- More of each format's quirks: ProTracker's instrument swap, FT2's
  backwards arpeggio (and its stuck one past 16 ticks), FT2's Lxx moving
  the panning envelope only with the volume envelope's sustain on, ST3's
  shared effect memory, IT's envelope carry, IT's duplicate check only
  among the instrument's own notes. Not every corner of every tracker:
  `tracker_render` and the regression examples are where a difference
  found would be pinned down.
- MIDI macros (Zxx and \xx with SFx), editable, as IT and OpenMPT have
  them: the filter's cutoff, resonance and mode, effect slots'
  parameters, and messages to the instrument's MIDI output.
- Effect chains: built-in effects (reverb, delay, chorus, flanger,
  distortion, compressor, equalizer, filter, gargle, what OpenMPT's DMO
  plugins are), a channel or instrument into a slot, a slot into a
  later one. VST plugins aren't hosted: a song must play anywhere UMF
  does.

**Samples**
- A sample editor: mark, cut, copy, paste, crop, silence, fade in and
  out, volume, normalize, reverse, resample, 8 or 16 bits, mono or
  stereo, DC offset removed, drawn with the pencil (FT2, OpenMPT,
  Renoise).
- Loops dragged on the waveform, crossfaded, and their end moved to
  where it comes round most smoothly (OpenMPT, Renoise).
- The waveform zoomed and scrolled; played from a point (double-click)
  or a marked range.
- Recording from the sound input (Renoise, OpenMPT).
- Samples read and written as WAV (with its loop), AIFF, IFF 8SVX (and
  16SV read), FLAC, ITS, XI (FT2's instrument, with its samples) and PAT
  (Gravis, several waves read as an instrument).
- Time-stretch and pitch-shift (overlap-add, WSOLA's way).
- Slices: found at the beats or put by hand, and made samples of their
  own with an instrument that plays them a key each (Renoise).

**Instruments**
- The keymap drawn as a piano, a key or a range mapped to a sample and
  to a note it plays as (IT's note-sample table).
- Envelope tools: a point put in between two, fitted to its sample's
  length, sped up or slowed; OpenMPT's release node.
- Pitch-pan separation, random volume and panning, the duplicate check
  and its action, edited.
- The filter's cutoff and resonance edited, the cutoff in hertz, and the
  filter envelope's range shown.
- A MIDI output for an instrument: channel, program and bank (IT,
  OpenMPT); the output chosen in Settings.
- An instrument through an effect slot, and synths: a sample made from
  oscillators (sine, saw, square, triangle, noise, FM, beating),
  looped, as the trackers' chip instruments. Live synths and VSTi
  aren't: what's made is a sample, so it's kept in the repository and
  plays anywhere.

**Song**
- Several order lists in one UMF (OpenMPT's sequences).
- Statistics (its length, notes, unplayed patterns, unused samples and
  instruments) and OpenMPT's Cleanup.
- Each channel rendered to its own WAV (stems); they add up to the song.
- Export to MOD, S3M, XM and IT, with what each can't keep said and
  counted.
- Tempo modes (classic, alternative, modern), rows a beat, and swing.
- The song's comments read with their formatting (# headings, lists,
  bold, italic, code); the song's artist and each sample's credit.
- Automation lanes (Renoise's, OpenMPT's parameter control events): the
  global volume, the tempo, a channel's volume or panning, or an effect
  slot's parameter, drawn over a pattern's rows.

**The repository**
- Samples found by sound: each waveform's spectrum (log-spaced, compared
  slid up to three octaves either way) and loudness over time, so a
  resampled or re-levelled copy is found, and ones like it.
- Several samples filed at once; tags and categories renamed or merged.
- Waveforms drawn small in the results.
- The UMF files that play a sample, and, when it's taken out, those
  files pointed at another (one that sounds like it, or any).
- A repository shared over the local network (an address and a code),
  and another's fetched and merged as a soundpak is.
