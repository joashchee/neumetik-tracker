/**
 * The tracker's Samples pane: the song's samples, and the sample
 * editor (FastTracker 2's, OpenMPT's and Renoise's, in one): the waveform
 * marked, zoomed and scrolled, its loops dragged, drawn on with the
 * pencil, sliced; a range cut, copied, pasted, cropped, silenced, faded,
 * made louder, normalized, reversed, its DC offset taken out; the whole
 * resampled, made 8 or 16 bits and mono or stereo, time-stretched or
 * pitch-shifted, its loop crossfaded or moved where it's smooth; synths
 * made into samples; and every sample file kind read and written.
 */
import { useEffect, useState } from "react";
import { host } from "../lib/host";
import * as tk from "../lib/tracker";
import { NumberField, SampleCanvas, TextField, type WaveTool } from "./TrackerParts";

const LOOPS = ["Off", "Forward", "Ping-pong"];
const VIBRATOS = ["Sine", "Ramp down", "Square", "Random", "Ramp up"];
const SYNTH_WAVES = ["Sine", "Saw", "Square", "Triangle", "Noise", "FM"];
const WIDTH = 300;

interface Props {
  doc: tk.Doc;
  letters: tk.Letters | null;
  index: number;
  onIndex: (i: number) => void;
  run: (edits: tk.Edit[], then?: (d: tk.Doc) => void) => void;
  patIndex: number;
  setDoc: (d: tk.Doc) => void;
  playNote: (note: number) => void;
  showStatus: (m: string) => void;
  showError: (m: string) => void;
  onInstrument: (i: number) => void;
  audioReady: boolean;
}

export function SamplePane({ doc, letters, index, onIndex, run, patIndex, setDoc, playNote, showStatus, showError, onInstrument, audioReady }: Props) {
  const s = doc.samples[index];
  const frames = s?.frames ?? 0;
  const [view, setView] = useState<[number, number]>([0, frames]);
  const [sel, setSel] = useState<[number, number] | null>(null);
  const [tool, setTool] = useState<WaveTool>("select");
  const [peaks, setPeaks] = useState<[number, number][] | null>(null);
  const [amount, setAmount] = useState({ gain: 150, crossfade: 512, reach: 2000, stretch: 100, pitch: 0, rate: 22050, sensitivity: 0.5, first: 61 });
  const [saveKind, setSaveKind] = useState("wav");
  const [synth, setSynth] = useState<tk.Synth>({ wave: 1, width: 50, ratio: 2, depth: 30, beat: 0, soften: 0 });
  const [group, setGroup] = useState<"edit" | "loop" | "change" | "slices" | "synth">("edit");

  // A new sample (or one changed in length) is seen whole.
  useEffect(() => {
    setView([0, frames]);
    setSel(null);
  }, [index, frames]);
  useEffect(() => {
    if (!s || frames === 0) {
      setPeaks(null);
      return;
    }
    const [a, b] = view;
    tk.samplePeaksRange(index, WIDTH, a, Math.max(a + 1, b)).then(setPeaks, () => setPeaks(null));
  }, [doc, index, s, frames, view]);

  const fail = (e: unknown) => showError(String(e));
  const change = (patch: Partial<tk.Sample>) => {
    if (!s) return;
    const n = { ...s, ...patch };
    run([{ op: "sample", index, name: n.name, volume: n.volume, globalVolume: n.globalVolume, c5: n.c5, pan: n.pan, loopKind: n.loopKind, loopStart: n.loopStart, loopEnd: n.loopEnd, susKind: n.susKind, susStart: n.susStart, susEnd: n.susEnd, vibrato: n.vibrato, credit: n.credit }]);
  };
  /** The range marked, or the whole sample. */
  const range = (): [number, number] => (sel && sel[1] > sel[0] ? sel : [0, frames]);
  const edit = (op: tk.SampleEditOp, value = 0, whole = false) => {
    const [a, b] = whole ? [0, frames] : range();
    run([{ op: "sampleEdit", index, edit: op, from: a, to: b, value }]);
  };

  async function loadFile() {
    try {
      const exts = letters?.sampleReads ?? ["wav"];
      const [picked] = await (await host()).pick([{ name: "Sample files", extensions: exts }], false);
      if (!picked) return;
      const r = await tk.loadSample(index, picked.name, await picked.bytes(), patIndex);
      setDoc(r.edited.doc);
      if (r.instrument !== null) {
        showStatus(`Loaded ${picked.name}: ${r.samples} samples, and instrument ${tk.hex2(r.instrument + 1)} that plays them.`);
        onInstrument(r.instrument);
      } else {
        showStatus(`Loaded ${picked.name} into sample ${tk.hex2(index + 1)}.`);
      }
    } catch (e) {
      fail(e);
    }
  }

  async function saveAs() {
    if (!s) return;
    try {
      const kind = (letters?.sampleWrites ?? []).find(([e]) => e === saveKind);
      const [lost, bytes] = await tk.saveSample(index, saveKind);
      const to = await (await host()).save(`${s.name || `sample ${index + 1}`}.${saveKind}`, bytes, { name: kind?.[1] ?? "Sample", extensions: [saveKind] });
      if (to) showStatus(lost.length ? `Saved ${to}; ${lost.join("; ")}.` : `Saved ${to}.`);
    } catch (e) {
      fail(e);
    }
  }

  const zoom = (k: number) => {
    const [a, b] = view;
    const mid = sel && sel[1] > sel[0] ? (sel[0] + sel[1]) / 2 : (a + b) / 2;
    const len = Math.max(16, Math.min(frames, Math.round((b - a) * k)));
    const start = Math.max(0, Math.min(frames - len, Math.round(mid - len / 2)));
    setView([start, start + len]);
  };
  const marked = !!sel && sel[1] > sel[0];
  const selText = sel ? (sel[1] > sel[0] ? `${sel[0]} to ${sel[1]} (${sel[1] - sel[0]} frames)` : `at ${sel[0]}`) : "nothing marked";

  return (
    <div className="tracker-pane" data-testid="tracker-samples">
      <ul className="tracker-list" aria-label="Samples">
        {doc.samples.map((x, i) => (
          <li key={i}>
            <button type="button" className={`tracker-item${i === index ? " selected" : ""}${x.frames === 0 ? " empty" : ""}`} onClick={() => onIndex(i)} onDoubleClick={() => playNote(61)}>
              {`${tk.hex2(i + 1)} ${x.name || (x.frames === 0 ? "(empty)" : "(unnamed)")}`}
            </button>
          </li>
        ))}
      </ul>
      <div className="tracker-list-buttons">
        <button type="button" className="small" onClick={() => run([{ op: "addSample" }], (d) => onIndex(d.samples.length - 1))}>
          Add
        </button>
        <button type="button" className="small" disabled={!s} onClick={() => run([{ op: "clearSample", index }])}>
          Clear
        </button>
        <button type="button" className="small" disabled={!s || frames === 0} onClick={() => playNote(61)} title="Play it at C-5">
          ▶ C-5
        </button>
        <button type="button" className="small" data-testid="tracker-load-wav" onClick={() => void loadFile()} title={`Reads ${(letters?.sampleReads ?? []).join(", ").toUpperCase()}; an XI or PAT with several samples comes in as an instrument`}>
          Load…
        </button>
      </div>
      {s && (
        <>
          <SampleCanvas
            peaks={peaks}
            width={WIDTH}
            height={88}
            frames={frames}
            view={view}
            selection={sel}
            loop={s.loopKind ? [s.loopStart, s.loopEnd] : null}
            sustain={s.susKind ? [s.susStart, s.susEnd] : null}
            slices={s.slices}
            tool={tool}
            label={`Sample ${index + 1}'s waveform: ${frames} frames, ${selText}`}
            onSelect={setSel}
            onView={setView}
            onLoopEdge={(which, f) => change({ [which]: f } as Partial<tk.Sample>)}
            onDraw={(at, points) => run([{ op: "sampleDraw", index, at, points }])}
            onSlice={(f) => {
              const near = s.slices.findIndex((x) => Math.abs(x - f) <= Math.max(4, (view[1] - view[0]) / 100));
              run([{ op: "sampleSlices", index, slices: near >= 0 ? s.slices.filter((_, i) => i !== near) : [...s.slices, f] }]);
            }}
            onPlayFrom={(f) => audioReady && tk.sampleAudition(index, f, null).catch(fail)}
          />
          <div className="tracker-inline tracker-wave-bar">
            <button type="button" className={`small${tool === "select" ? " selected" : ""}`} aria-pressed={tool === "select"} title="Mark a range; drag the loops' edges" onClick={() => setTool("select")}>
              Mark
            </button>
            <button type="button" className={`small${tool === "draw" ? " selected" : ""}`} aria-pressed={tool === "draw"} title="Draw the waveform (zoom in to draw a frame at a time)" onClick={() => setTool("draw")}>
              ✎ Draw
            </button>
            <button type="button" className="small" disabled={frames === 0} title="Zoom in (Cmd+wheel)" onClick={() => zoom(0.5)}>
              +
            </button>
            <button type="button" className="small" disabled={frames === 0} title="Zoom out" onClick={() => zoom(2)}>
              −
            </button>
            <button type="button" className="small" disabled={frames === 0} title="All of it" onClick={() => setView([0, frames])}>
              All
            </button>
            <button type="button" className="small" disabled={!marked} title="Zoom to what's marked" onClick={() => sel && setView([sel[0], Math.max(sel[0] + 16, sel[1])])}>
              Fit
            </button>
            <button type="button" className="small" disabled={!audioReady || frames === 0} title="Play what's marked (or from the cursor; a double-click plays from there)" onClick={() => tk.sampleAudition(index, sel ? sel[0] : 0, marked && sel ? sel[1] : null).catch(fail)}>
              ▶
            </button>
            <button type="button" className="small" disabled={!audioReady} title="Stop" onClick={() => tk.sampleAudition(index, 0, 0).catch(() => {})}>
              ■
            </button>
          </div>
          <p className="tracker-facts">
            {frames === 0
              ? doc.missing > 0
                ? "No waveform: it wasn't saved in the file"
                : "Empty"
              : `${frames} frames · ${s.bits}-bit${s.channels === 2 ? " stereo" : ""} · ${tk.seconds(frames / Math.max(1, s.c5))} at C-5 · ${selText}${view[0] > 0 || view[1] < frames ? ` · showing ${view[0]} to ${view[1]}` : ""}${s.slices.length ? ` · ${s.slices.length} slices` : ""}`}
          </p>
          <div className="tracker-tabs small" role="tablist" aria-label="Sample tools">
            {(
              [
                ["edit", "Edit"],
                ["loop", "Loop"],
                ["change", "Change"],
                ["slices", "Slices"],
                ["synth", "Synth"],
              ] as const
            ).map(([k, n]) => (
              <button key={k} type="button" role="tab" aria-selected={group === k} className={group === k ? "selected" : ""} onClick={() => setGroup(k)}>
                {n}
              </button>
            ))}
          </div>
          {group === "edit" && (
            <div className="tracker-list-buttons" data-testid="tracker-sample-edit">
              <button type="button" className="small" disabled={!marked} onClick={() => sel && tk.sampleCopy(index, sel[0], sel[1], true, patIndex).then((r) => r && setDoc(r.doc), fail)}>
                Cut
              </button>
              <button type="button" className="small" disabled={!marked} onClick={() => sel && tk.sampleCopy(index, sel[0], sel[1], false, null).then(() => showStatus(`Copied ${sel[1] - sel[0]} frames.`), fail)}>
                Copy
              </button>
              <button type="button" className="small" title="At the cursor (the start of what's marked)" onClick={() => tk.samplePaste(index, sel ? sel[0] : frames, patIndex).then((r) => setDoc(r.doc), fail)}>
                Paste
              </button>
              <button type="button" className="small" disabled={!marked} onClick={() => edit("crop")}>
                Crop
              </button>
              <button type="button" className="small" disabled={frames === 0} onClick={() => edit("silence")}>
                Silence
              </button>
              <button type="button" className="small" disabled={frames === 0} onClick={() => edit("fadeIn")}>
                Fade In
              </button>
              <button type="button" className="small" disabled={frames === 0} onClick={() => edit("fadeOut")}>
                Fade Out
              </button>
              <button type="button" className="small" disabled={frames === 0} onClick={() => edit("normalize")}>
                Normalize
              </button>
              <button type="button" className="small" disabled={frames === 0} onClick={() => edit("reverse")}>
                Reverse
              </button>
              <button type="button" className="small" disabled={frames === 0} title="Take out the DC offset (each channel's average)" onClick={() => edit("dc")}>
                Remove DC
              </button>
              <span className="tracker-inline">
                <button type="button" className="small" disabled={frames === 0} onClick={() => edit("gain", amount.gain)}>
                  Volume
                </button>
                <input type="number" aria-label="Volume, in percent" min={0} max={800} value={amount.gain} onChange={(e) => setAmount({ ...amount, gain: Number(e.currentTarget.value) })} />%
              </span>
              <p className="tracker-facts">{marked ? "On what's marked." : "On all of it (mark a range to edit less)."}</p>
            </div>
          )}
          {group === "loop" && (
            <div className="tracker-pane-group">
              <div className="tracker-form">
                <label>Loop</label>
                <select value={s.loopKind} onChange={(e) => change({ loopKind: Number(e.currentTarget.value), loopEnd: s.loopEnd || s.frames })}>
                  {LOOPS.map((l, i) => (
                    <option key={l} value={i}>
                      {l}
                    </option>
                  ))}
                </select>
                <NumberField label="Loop start" value={s.loopStart} min={0} max={s.frames} onCommit={(loopStart) => change({ loopStart })} />
                <NumberField label="Loop end" value={s.loopEnd} min={0} max={s.frames} onCommit={(loopEnd) => change({ loopEnd })} />
                <label title="Played while the note's held (Impulse Tracker's)">Sustain loop</label>
                <select value={s.susKind} onChange={(e) => change({ susKind: Number(e.currentTarget.value), susEnd: s.susEnd || s.frames })}>
                  {LOOPS.map((l, i) => (
                    <option key={l} value={i}>
                      {l}
                    </option>
                  ))}
                </select>
                {s.susKind > 0 && (
                  <>
                    <NumberField label="Sustain start" value={s.susStart} min={0} max={s.frames} onCommit={(susStart) => change({ susStart })} />
                    <NumberField label="Sustain end" value={s.susEnd} min={0} max={s.frames} onCommit={(susEnd) => change({ susEnd })} />
                  </>
                )}
              </div>
              <div className="tracker-list-buttons">
                <button type="button" className="small" disabled={!marked} title="The loop round what's marked" onClick={() => sel && change({ loopKind: s.loopKind || 1, loopStart: sel[0], loopEnd: sel[1] })}>
                  Loop Marked
                </button>
                <span className="tracker-inline">
                  <button type="button" className="small" disabled={s.loopKind === 0} title="Blend the loop's end into what's before its start, so it comes round without a click" onClick={() => edit("crossfade", amount.crossfade, true)}>
                    Crossfade
                  </button>
                  <input type="number" aria-label="Crossfade frames" min={1} max={65536} value={amount.crossfade} onChange={(e) => setAmount({ ...amount, crossfade: Number(e.currentTarget.value) })} />
                </span>
                <span className="tracker-inline">
                  <button type="button" className="small" disabled={s.loopKind === 0} title="Move the loop's end, within this many frames, to where it comes round most smoothly" onClick={() => edit("smoothLoop", amount.reach, true)}>
                    Smooth End
                  </button>
                  <input type="number" aria-label="Frames to look within" min={1} max={1_000_000} value={amount.reach} onChange={(e) => setAmount({ ...amount, reach: Number(e.currentTarget.value) })} />
                </span>
                <button type="button" className="small" disabled={s.loopKind === 0} title="Cut what's after the loop's end: it's never heard" onClick={() => run([{ op: "trimToLoop", index }])}>
                  Trim to Loop
                </button>
              </div>
            </div>
          )}
          {group === "change" && (
            <div className="tracker-pane-group">
              <div className="tracker-form">
                <label>Depth</label>
                <select value={s.bits} onChange={(e) => run([{ op: "sampleFormat", index, bits: Number(e.currentTarget.value), channels: s.channels }])}>
                  <option value={8}>8 bits</option>
                  <option value={16}>16 bits</option>
                </select>
                <label>Channels</label>
                <select value={s.channels} onChange={(e) => run([{ op: "sampleFormat", index, bits: s.bits, channels: Number(e.currentTarget.value) }])}>
                  <option value={1}>Mono (mixed)</option>
                  <option value={2}>Stereo</option>
                </select>
              </div>
              <div className="tracker-list-buttons">
                <span className="tracker-inline">
                  <button type="button" className="small" disabled={frames === 0} title="Read it at another rate: the same sound, more or fewer frames" onClick={() => edit("resample", amount.rate, true)}>
                    Resample to
                  </button>
                  <input type="number" aria-label="Rate in hertz" min={1000} max={192000} value={amount.rate} onChange={(e) => setAmount({ ...amount, rate: Number(e.currentTarget.value) })} />
                  Hz
                </span>
                <span className="tracker-inline">
                  <button type="button" className="small" disabled={frames === 0} title="Longer or shorter, the pitch the same" onClick={() => edit("stretch", amount.stretch / 100, true)}>
                    Stretch to
                  </button>
                  <input type="number" aria-label="Length in percent" min={25} max={400} value={amount.stretch} onChange={(e) => setAmount({ ...amount, stretch: Number(e.currentTarget.value) })} />%
                </span>
                <span className="tracker-inline">
                  <button type="button" className="small" disabled={frames === 0} title="Higher or lower, the length the same" onClick={() => edit("pitch", amount.pitch, true)}>
                    Shift Pitch
                  </button>
                  <input type="number" aria-label="Semitones" min={-24} max={24} step={0.5} value={amount.pitch} onChange={(e) => setAmount({ ...amount, pitch: Number(e.currentTarget.value) })} />
                  semitones
                </span>
              </div>
            </div>
          )}
          {group === "slices" && (
            <div className="tracker-list-buttons">
              <span className="tracker-inline">
                <button type="button" className="small" disabled={frames === 0} title="Slice it where each beat starts" onClick={() => edit("slices", amount.sensitivity, true)}>
                  Find Beats
                </button>
                <input type="range" aria-label="Sensitivity" min={0} max={1} step={0.05} value={amount.sensitivity} onChange={(e) => setAmount({ ...amount, sensitivity: Number(e.currentTarget.value) })} />
              </span>
              <button type="button" className="small" disabled={s.slices.length === 0} onClick={() => edit("clearSlices", 0, true)}>
                Clear Slices
              </button>
              <span className="tracker-inline">
                <button type="button" className="small" disabled={s.slices.length === 0} title="Each slice a sample of its own, and an instrument that plays them a key each" onClick={() => run([{ op: "slicesToInstrument", index, first: amount.first }], (d) => onInstrument(d.instruments.length - 1))}>
                  Slices to Instrument from
                </button>
                <select aria-label="The first slice's key" value={amount.first} onChange={(e) => setAmount({ ...amount, first: Number(e.currentTarget.value) })}>
                  {Array.from({ length: 120 }, (_, n) => (
                    <option key={n} value={n + 1}>
                      {tk.noteText(n + 1)}
                    </option>
                  ))}
                </select>
              </span>
              <p className="tracker-facts">Shift+click on the waveform adds a slice, or takes one away.</p>
            </div>
          )}
          {group === "synth" && (
            <div className="tracker-pane-group">
              <div className="tracker-form">
                <label>Wave</label>
                <select value={synth.wave} onChange={(e) => setSynth({ ...synth, wave: Number(e.currentTarget.value) })}>
                  {SYNTH_WAVES.map((w, i) => (
                    <option key={w} value={i}>
                      {w}
                    </option>
                  ))}
                </select>
                {synth.wave === 2 && <NumberField label="Pulse width" value={synth.width} min={1} max={99} onCommit={(width) => setSynth({ ...synth, width })} />}
                {synth.wave === 5 && (
                  <>
                    <NumberField label="FM ratio" value={synth.ratio} min={1} max={16} onCommit={(ratio) => setSynth({ ...synth, ratio })} />
                    <NumberField label="FM depth" value={synth.depth} min={0} max={100} onCommit={(depth) => setSynth({ ...synth, depth })} />
                  </>
                )}
                <NumberField label="Beating" value={synth.beat} min={0} max={64} onCommit={(beat) => setSynth({ ...synth, beat })} title="A second oscillator, slowly out of step with the first (0: none)" />
                <NumberField label="Soften" value={synth.soften} min={0} max={100} onCommit={(soften) => setSynth({ ...synth, soften })} title="A low-pass over it" />
              </div>
              <div className="tracker-list-buttons">
                <button type="button" className="small primary" onClick={() => run([{ op: "synth", index, synth }])}>
                  Make It the Sample
                </button>
                {s.synth && (
                  <button type="button" className="small" onClick={() => s.synth && setSynth(s.synth)}>
                    Its Settings
                  </button>
                )}
              </div>
              <p className="tracker-facts">A cycle (or, beating, a few) looped, tuned to middle C: it plays at any note, as the trackers' chip instruments did.</p>
            </div>
          )}
          <div className="tracker-form">
            <TextField label="Name" value={s.name} onCommit={(name) => change({ name })} testId="tracker-sample-name" wide />
            <TextField label="Credit" value={s.credit} onCommit={(credit) => change({ credit })} title="Who made it, or where it's from" wide />
            <NumberField label="Volume" value={s.volume} min={0} max={64} onCommit={(volume) => change({ volume })} />
            <NumberField label="Global vol." value={s.globalVolume} min={0} max={64} onCommit={(globalVolume) => change({ globalVolume })} title="The sample's own level, under its volume (Impulse Tracker's)" />
            <NumberField label="C-5 rate" value={s.c5} min={100} max={2000000} onCommit={(c5) => change({ c5 })} title="The rate it plays at for middle C, in hertz" />
            <span />
            <span className="tracker-inline">
              <button type="button" className="small" title="A semitone down" onClick={() => change({ c5: Math.round(s.c5 / Math.pow(2, 1 / 12)) })}>
                −1
              </button>
              <button type="button" className="small" title="A semitone up" onClick={() => change({ c5: Math.round(s.c5 * Math.pow(2, 1 / 12)) })}>
                +1
              </button>
            </span>
            <label>Panning</label>
            <span className="tracker-inline">
              <input type="checkbox" checked={s.pan !== null} aria-label="Its own panning" onChange={(e) => change({ pan: e.currentTarget.checked ? 128 : null })} />
              <input type="range" min={0} max={255} value={s.pan ?? 128} disabled={s.pan === null} aria-label="Panning" onChange={(e) => change({ pan: Number(e.currentTarget.value) })} />
            </span>
            <label>Vibrato</label>
            <select value={s.vibrato[0]} onChange={(e) => change({ vibrato: [Number(e.currentTarget.value), s.vibrato[1], s.vibrato[2], s.vibrato[3]] })}>
              {VIBRATOS.map((v, i) => (
                <option key={v} value={i}>
                  {v}
                </option>
              ))}
            </select>
            <NumberField label="Vib. speed" value={s.vibrato[1]} min={0} max={255} onCommit={(v) => change({ vibrato: [s.vibrato[0], v, s.vibrato[2], s.vibrato[3]] })} />
            <NumberField label="Vib. depth" value={s.vibrato[2]} min={0} max={255} onCommit={(v) => change({ vibrato: [s.vibrato[0], s.vibrato[1], v, s.vibrato[3]] })} />
            <NumberField label="Vib. sweep" value={s.vibrato[3]} min={0} max={255} onCommit={(v) => change({ vibrato: [s.vibrato[0], s.vibrato[1], s.vibrato[2], v] })} />
          </div>
          <div className="tracker-inline">
            <button type="button" className="small" disabled={frames === 0} onClick={() => void saveAs()}>
              Save As…
            </button>
            <select aria-label="The kind of file" value={saveKind} onChange={(e) => setSaveKind(e.currentTarget.value)}>
              {(letters?.sampleWrites ?? [["wav", "WAV"]]).map(([e, n]) => (
                <option key={e} value={e}>
                  {n}
                </option>
              ))}
            </select>
          </div>
        </>
      )}
    </div>
  );
}
