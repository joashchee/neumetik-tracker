# neumetik-synth

The synths tracker modules need, in Rust, with no dependencies, each
written from scratch on public material (`docs/synth-methods.md` has
every method's sources and why it's free to use). MIT licensed (the
`LICENSE` at the repository's root).

- `opl`: FM the OPL's way (Yamaha's OPL2 and OPL3), for AdLib
  instruments, and the chip by its registers.
- `sequence`: OctaMED's synth and hybrid sounds.
- `replay`: songs played by their own replayers: `ahx` (AHX and
  HivelyTracker), `fc` (Future Composer), `sidmon1`, `sidmon2`,
  `soundmon`, `deltamusic1`, `deltamusic2`, `mugician`, `sonic`,
  `instereo`, `musicassembler`, `synthesis`, `voodoo`, `musicline` and
  `ptk` (ProTrekkr and NoiseTrekker), through `paula`, the Amiga's
  sound chip.
- `puma`, `sel`, `am`, `jam`: Pumatracker's instruments, Face The
  Music's scripts, StarTrekker's and JamCracker's AM instruments.
- `cycle`: single-cycle waveforms made into looping samples.
