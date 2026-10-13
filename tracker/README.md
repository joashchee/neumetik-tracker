# neumetik-tracker

Tracker modules read, played, edited and written, in Rust, with no
third-party dependencies. MIT licensed (the `LICENSE` at the
repository's root).

- **Reading** (`src/formats/`): some eighty formats into one model,
  `Module`: every ProTracker MOD variant and the SoundTrackers, S3M,
  XM, IT and MPTM, MTM, 669, STM, PTM, ULT, FAR, IMF, Oktalyzer,
  OctaMED and MED, DigiBooster, the synth trackers (AHX, HivelyTracker,
  Future Composer, SidMon, SoundMon, Delta Music, Digital Mugician,
  Sonic Arranger and more, played by their own replayers in
  `neumetik-synth`), ProTrekkr, Musicline, the MODs forty packers made,
  and modules zipped and gzipped. `docs/tracker-formats.md` lists every
  format known and which are read.
- **Playing** (`src/player.rs`): a module tick by tick, by its format's
  own effect rules, with envelopes, new note actions, the resonant
  filter, interpolation and declicking. `render` never allocates once
  made, so it runs in an audio callback.
- **Editing** (`src/edit.rs`, `src/sample_edit.rs`, `src/samplefile.rs`):
  blocks interpolated and amplified, paste modes, find and replace,
  cleanup; the sample editor and every sample file kind; songs written
  as MOD, S3M, XM and IT (`src/export.rs`). `docs/editor-features.md`
  lists it all.
- **UMF** (`src/umf.rs`, `docs/umf-format.md`): the Universal Mod
  Format. `umf::write_ntt` saves a song with its samples (an NTT),
  everything it is kept; `load` reads it back.

```rust
use std::sync::Arc;
use neumetik_tracker::{load, player};

let bytes = std::fs::read("song.xm")?;
let module = load(&bytes, "song.xm")?;
let audio = player::render(Arc::new(module), 44_100, 600.0, 1.0); // stereo, interleaved
```

Written from the formats' public descriptions; everything under them is
written here too: SHA-256 (`src/hash.rs`), inflate (`src/unpack.rs`),
Ogg Vorbis (`src/vorbis.rs`), FLAC (`src/samplefile.rs`), the effects
(`src/dsp.rs`), and the compression UMF uses (`src/lz.rs`,
`src/wavepack.rs`).
