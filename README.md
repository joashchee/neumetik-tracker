# Neumetik Tracker

Tracker modules played and edited: MOD, S3M, XM, IT and some eighty
formats more, from the Amiga's SoundTrackers to AHX, HivelyTracker,
Future Composer and ProTrekkr, each played the way its own tracker
played it. The open-source part of Neumetik,
by ansiapps, under the MIT License.

It runs on **macOS, Windows, Linux, Android, iOS and the web**: one
editor (React), one engine (Rust), the same everywhere.

- **Playing**: each format by its own effect rules, with envelopes, new
  note actions, the resonant filter, interpolation from none (the
  Amiga's) to windowed sinc, AdLib FM instruments, OctaMED's synth
  sounds and the synth trackers' own replayers.
- **Editing**: the pattern as FastTracker 2 and Impulse Tracker had it
  (the computer's keyboard as a piano, blocks, paste modes,
  interpolation, transposition, find and replace, bookmarks, the
  pattern matrix), samples (a waveform editor: cut, crop, fade,
  normalize, loop crossfades, time-stretch, pitch-shift, slices, synths
  made into samples), instruments (keymaps, envelopes, filters,
  phrases), the mix (faders, effect slots), automation lanes, several
  order lists, MIDI macros, cleanup.
- **Files**: any module read; songs saved as NTT, the song with its
  samples, everything kept; exported as WAV, each channel as WAV, or as
  MOD, S3M, XM and IT; samples read and written as WAV, AIFF, 8SVX,
  FLAC, ITS, XI and PAT.

No third-party code in the engine: every format is read from its public
description, and every algorithm under it (inflate, Ogg Vorbis, FLAC,
SHA-256, the synths) is written here.

## What's here

| Folder | What it is |
| --- | --- |
| `tracker/` | `neumetik-tracker`: modules read, played, edited and written (Rust, no dependencies) |
| `synth/` | `neumetik-synth`: the synths the formats need: FM (OPL2 and OPL3), OctaMED's synth sounds, the synth trackers' replayers |
| `tracker-app/session/` | The editor's session: the song, its undo, every command, the same on every platform |
| `tracker-app/src-tauri/` | The app on the desktop and phones (Tauri 2, sound through cpal) |
| `tracker-app/web/` | The engine as WebAssembly, for the browser (it plays in an AudioWorklet) |
| `tracker-app/src/` | The editor's screen (React and TypeScript), shared by every platform |

## Building

You need Rust (stable) and Node.js 22.

```sh
cd tracker && cargo test             # the engine
cd tracker-app
npm install
npm run tauri dev                    # the desktop app
scripts/build-release.sh             # a release build for this machine
npm run wasm && npm run dev          # the web build, at http://localhost:1480
npm run build:web                    # the web build, in dist/
npx tauri android init && npx tauri android dev   # Android 8.0 and later (SDK and NDK)
npx tauri ios init && npx tauri ios dev           # iOS (Xcode)
```

On Linux, Tauri needs `libwebkit2gtk-4.1-dev libappindicator3-dev
librsvg2-dev patchelf`, and the sound `libasound2-dev`.

GitHub Actions builds every platform (`.github/workflows/build.yml`)
on a version tag, attaches the desktop and Android builds to a release,
and publishes the web build to GitHub Pages. iOS builds need an Apple
developer team (the `APPLE_DEVELOPMENT_TEAM` secret) to be signed.

The Android AAB is signed for Google Play with an upload key, from the
secrets `ANDROID_KEYSTORE_BASE64` (the `.jks`, base64),
`ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` and
`ANDROID_KEY_PASSWORD`. With `PLAY_SERVICE_ACCOUNT_JSON` too (a Google
Cloud service account given release rights in the Play Console), a
version tag uploads it to Google Play: the internal track as a draft,
or the track and status in the `PLAY_TRACK` and `PLAY_RELEASE_STATUS`
variables. The app's first AAB is uploaded in the Play Console by hand.

## The keys

| Keys | |
| --- | --- |
| Z S X D C V… and Q 2 W 3 E R… | notes, two octaves from the octave chosen |
| ` and 1 | note off, note cut |
| 0-9 A-F, letters | instrument, volume and effect values; effect letters |
| [ and ] | octave down and up |
| Space | editing on or off |
| Shift+arrows, Cmd+C X V | blocks: mark, copy, cut, paste |
| Cmd+Z, Cmd+Shift+Z | undo, redo |
| F5 F6 F7 F8 | play the song, the pattern, from the cursor; stop |

On a phone or tablet, keys under the pattern do the same.

## License

MIT: see `LICENSE`. The fonts (IBM Plex) are under the SIL Open Font
License 1.1 (`tracker-app/public/fonts/IBM-Plex-LICENSE.txt`).

This repository is published from Neumetik's own; pull requests are
welcome, and are carried back there.
