# Google Play listing

What the Play Console asks for, ready to upload or paste. In the
console: Grow users → Store presence → Main store listing.

## Graphics (`graphics/`)

| Console field | File | Size |
| --- | --- | --- |
| App icon | `icon-512.png` | 512 × 512, 32-bit PNG |
| Feature graphic | `feature-graphic.png` | 1024 × 500 |
| Phone screenshots | `phone-1.png` … `phone-4.png` | 1080 × 1920 |
| 7-inch tablet screenshots | `tablet7-1.png`, `tablet7-2.png` | 1920 × 1200 |
| 10-inch tablet screenshots | `tablet10-1.png` … `tablet10-3.png` | 2560 × 1600 |

Upload the screenshots in their numbered order. The first phone
screenshots are the ones most people see.

The song in the screenshots is `demo.mod`, written for them by
`tools/make_mod.py`: its samples are synthesized and its notes are
original, so there's no one else's music in the listing.

## Text

**App name** (30 characters at most)

> Neumetik Tracker

**Short description** (80 characters at most)

> Play and edit MOD, S3M, XM, IT and 80+ tracker formats, the way they were made.

**Full description** (4000 characters at most)

> Neumetik Tracker plays and edits tracker modules: MOD, S3M, XM, IT
> and some eighty formats more, from the Amiga's SoundTrackers to AHX,
> HivelyTracker, Future Composer and ProTrekkr. Each one is played the
> way its own tracker played it.
>
> PLAY
> • Each format by its own effect rules, with envelopes, new note
>   actions and the resonant filter
> • Interpolation from none (the Amiga's) to windowed sinc
> • AdLib FM instruments, OctaMED's synth sounds and the synth
>   trackers' own replayers
>
> EDIT
> • The pattern editor as FastTracker 2 and Impulse Tracker had it:
>   blocks, paste modes, interpolation, transposition, find and replace,
>   bookmarks and the pattern matrix
> • Touch keys for phones and tablets: a piano, hex digits and the
>   editing moves, or a hardware keyboard and MIDI
> • A waveform editor for samples: cut, crop, fade, normalize, loop
>   crossfades, time-stretch, pitch-shift, slices, and synths made into
>   samples
> • Instruments with keymaps, envelopes, filters and phrases
> • A mixer with faders and effect slots, automation lanes, several
>   order lists and MIDI macros
>
> FILES
> • Open any module; save songs as NTT, with every sample kept
> • Export WAV, each channel as its own WAV, or MOD, S3M, XM and IT
> • Read and write samples as WAV, AIFF, 8SVX, FLAC, ITS, XI and PAT
>
> PRIVATE AND OPEN
> No accounts, no ads, no tracking: your files stay on your device.
> Neumetik Tracker is open source under the MIT License, and its engine
> is written from scratch, every format read from its public
> description.

**Category:** Music & Audio
**Tags** (Store settings): Music & audio, Music creation

## Privacy policy

Play asks every app for one. It's published with the web build, at

> https://joashchee.github.io/neumetik-tracker/privacy.html

(`tracker-app/public/privacy.html`), and kept here as
`privacy-policy.md` too. The two say the same: change them together.
Its URL goes in Policy → App content → Privacy policy.

## Making them again

After the app changes, from this folder:

```sh
cd ../../tracker-app && npm run build:web && npx vite preview --port 1490 &
cd ../store/google-play/tools
python3 make_mod.py demo.mod
# the screenshots, each device's views as in the commands below
node shoot.mjs raw phone play:10500 \
  "eval:document.querySelector('.tracker-grid-wrap').scrollIntoView({block:'start'})" shot:1-pattern \
  "eval:document.querySelector('.touch-keys').scrollIntoView({block:'end'})" shot:2-keys \
  tab:samples "eval:document.querySelector('.tracker-tabs').scrollIntoView({block:'start'})" shot:3-samples \
  tab:mix "eval:[...document.querySelectorAll('h3,h4,p,div,span')].find(e=>e.textContent.trim()==='Faders').scrollIntoView({block:'start'})" shot:4-mix
node shoot.mjs raw tablet7 play:10500 shot:1-pattern \
  tab:samples "eval:document.querySelector('.tracker-tabs').scrollIntoView({block:'start'})" shot:2-samples
node shoot.mjs raw tablet10 play:10500 shot:1-pattern tab:mix shot:2-mix tab:song shot:3-song
node compose.mjs ../graphics
python3 -c "from PIL import Image; Image.open('../graphics/icon-512.png').convert('RGBA').save('../graphics/icon-512.png')"
```

The scripts need Playwright and Chromium.
