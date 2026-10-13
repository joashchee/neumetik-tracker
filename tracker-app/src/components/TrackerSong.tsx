/**
 * The tracker's Song pane: the song's words and settings, its timing
 * (OpenMPT's tempo modes, rows a beat and a measure, swing), its order
 * lists (OpenMPT's sequences, MED's sections), its comments as written
 * and as they read with their formatting, its credits, its channels and
 * the effect slot each goes through, and the way to its statistics and
 * cleanup and its MIDI macros (TrackerDialogs.tsx).
 */
import { useEffect, useState } from "react";
import * as tk from "../lib/tracker";
import { Comments, NumberField, TextField } from "./TrackerParts";

interface Props {
  doc: tk.Doc;
  letters: tk.Letters | null;
  run: (edits: tk.Edit[], then?: (d: tk.Doc) => void) => void;
  onStats: () => void;
  onMacros: () => void;
}

export function SongPane({ doc, letters, run, onStats, onMacros }: Props) {
  const song = (patch: Partial<tk.Doc>) => {
    const n = { ...doc, ...patch };
    run([{ op: "song", title: n.title, artist: n.artist, message: n.message, speed: n.speed, tempo: n.tempo, globalVolume: n.globalVolume, mixVolume: n.mixVolume, restart: n.restart, linear: n.linear }]);
  };
  const timing = (patch: Partial<{ tempoMode: number; rowsPerBeat: number; rowsPerMeasure: number; swing: number }>) => {
    const t = { tempoMode: doc.tempoMode, rowsPerBeat: doc.rowsPerBeat, rowsPerMeasure: doc.rowsPerMeasure, swing: doc.swing, ...patch };
    run([{ op: "timing", ...t }]);
  };
  const [message, setMessage] = useState(doc.message);
  const [preview, setPreview] = useState(false);
  const [seqName, setSeqName] = useState("");
  useEffect(() => setMessage(doc.message), [doc.message]);
  useEffect(() => setSeqName(doc.sequences[doc.sequence] ?? ""), [doc.sequences, doc.sequence]);
  const [ch, setCh] = useState(0);
  const c = doc.channels[ch];
  const credits = doc.samples.map((s, i) => [i, s] as const).filter(([, s]) => s.credit.trim() !== "");
  const fxName = (i: number) => `${i + 1}: ${doc.effects[i]?.name || letters?.effectKinds[doc.effects[i]?.kind ?? 0]?.[0] || "Effect"}`;
  return (
    <div className="tracker-pane" data-testid="tracker-song">
      <div className="tracker-form">
        <TextField label="Title" value={doc.title} onCommit={(title) => song({ title })} testId="tracker-song-title" wide />
        <TextField label="Artist" value={doc.artist} onCommit={(artist) => song({ artist })} wide />
        <NumberField label="Speed" value={doc.speed} min={1} max={255} onCommit={(speed) => song({ speed })} title="Ticks a row" />
        <NumberField label="Tempo" value={doc.tempo} min={32} max={999} onCommit={(tempo) => song({ tempo })} title="Beats a minute" />
        <NumberField label="Global vol." value={doc.globalVolume} min={0} max={128} onCommit={(globalVolume) => song({ globalVolume })} />
        <NumberField label="Mix level" value={doc.mixVolume} min={1} max={128} onCommit={(mixVolume) => song({ mixVolume })} />
        <NumberField label="Restart at" value={doc.restart} min={0} max={Math.max(0, doc.orders.length - 1)} onCommit={(restart) => song({ restart })} title="The order the song goes back to at its end" />
        <label title="Pitch slides in 64ths of a semitone, not the Amiga's periods">Linear slides</label>
        <input type="checkbox" checked={doc.linear} onChange={(e) => song({ linear: e.currentTarget.checked })} />
      </div>
      <p className="tracker-head">Timing</p>
      <div className="tracker-form">
        <label title="Classic: a tick is 2.5 ÷ tempo seconds, as every tracker had it. Alternative: tempo is ticks a second. Modern: tempo is beats a minute whatever the speed (OpenMPT's).">Tempo mode</label>
        <select value={doc.tempoMode} data-testid="tracker-tempo-mode" onChange={(e) => timing({ tempoMode: Number(e.currentTarget.value) })}>
          {tk.TEMPO_MODES.map((m, i) => (
            <option key={m} value={i}>
              {m}
            </option>
          ))}
        </select>
        <NumberField label="Rows a beat" value={doc.rowsPerBeat} min={1} max={64} onCommit={(rowsPerBeat) => timing({ rowsPerBeat })} title="The rows highlighted as beats, and the beat Modern tempo and swing count in" />
        <NumberField label="Rows a measure" value={doc.rowsPerMeasure} min={1} max={256} onCommit={(rowsPerMeasure) => timing({ rowsPerMeasure })} />
        <NumberField label="Swing" value={doc.swing} min={0} max={100} onCommit={(swing) => timing({ swing })} title="Each pair of rows' first held longer, the second shorter, in percent (66 is a triplet's swing)" />
      </div>
      <p className="tracker-head">Order lists</p>
      <div className="tracker-inline">
        <select aria-label="The order list being edited" value={doc.sequence} data-testid="tracker-sequence" onChange={(e) => run([{ op: "switchSequence", index: Number(e.currentTarget.value) }])}>
          {doc.sequences.map((n, i) => (
            <option key={i} value={i}>{`${i + 1}: ${n || (doc.sequences.length === 1 ? "the song" : "untitled")}`}</option>
          ))}
        </select>
        <button type="button" className="small" title="A new, empty order list (a song of its own, sharing the patterns)" onClick={() => run([{ op: "addSequence", name: `Song ${doc.sequences.length + 1}`, copy: false }])}>
          New
        </button>
        <button type="button" className="small" title="A copy of this order list" onClick={() => run([{ op: "addSequence", name: `${doc.sequences[doc.sequence] || "Song"} (copy)`, copy: true }])}>
          Copy
        </button>
        <button type="button" className="small" disabled={doc.sequences.length <= 1} onClick={() => run([{ op: "removeSequence", index: doc.sequence }])}>
          Remove
        </button>
      </div>
      {doc.sequences.length > 1 && (
        <div className="tracker-form">
          <TextField label="Its name" value={seqName} onCommit={(name) => run([{ op: "renameSequence", index: doc.sequence, name }])} wide />
        </div>
      )}
      <div className="tracker-inline">
        <span className="tracker-head">Message</span>
        <button type="button" className={`small${preview ? "" : " selected"}`} aria-pressed={!preview} onClick={() => setPreview(false)}>
          As Written
        </button>
        <button type="button" className={`small${preview ? " selected" : ""}`} aria-pressed={preview} title="With its formatting: # headings, - lists, **bold**, *italic*, `code`" onClick={() => setPreview(true)}>
          As It Reads
        </button>
      </div>
      {preview ? (
        <Comments text={doc.message} />
      ) : (
        <textarea aria-label="Message" className="tracker-message" value={message} onChange={(e) => setMessage(e.currentTarget.value)} onBlur={() => message !== doc.message && song({ message })} />
      )}
      <p className="tracker-head">Credits</p>
      <p className="tracker-facts" data-testid="tracker-credits">
        {[doc.artist ? `Song by ${doc.artist}.` : "", ...credits.map(([i, s]) => `Sample ${tk.hex2(i + 1)} (${s.name || "unnamed"}): ${s.credit}.`)].filter(Boolean).join(" ") || "None given: a sample's credit is in the Samples tab, the song's artist above."}
      </p>
      <p className="tracker-head">Channels</p>
      <div className="tracker-inline">
        <select aria-label="Channel" value={ch} onChange={(e) => setCh(Number(e.currentTarget.value))}>
          {doc.channels.map((x, i) => (
            <option key={i} value={i}>{`${String(i + 1).padStart(2, "0")} ${x.name}`}</option>
          ))}
        </select>
        <button type="button" className="small" disabled={doc.channels.length >= 64} onClick={() => run([{ op: "addChannel" }])}>
          Add
        </button>
        <button type="button" className="small" disabled={doc.channels.length <= 1} onClick={() => run([{ op: "removeChannel", index: ch }], () => setCh(Math.max(0, ch - 1)))}>
          Remove
        </button>
      </div>
      {c && (
        <div className="tracker-form">
          <TextField label="Name" value={c.name} onCommit={(name) => run([{ op: "channel", index: ch, name, pan: c.pan, volume: c.volume, surround: c.surround }])} wide />
          <label>Panning</label>
          <input type="range" min={0} max={255} value={c.pan} aria-label="Panning" onChange={(e) => run([{ op: "channel", index: ch, name: c.name, pan: Number(e.currentTarget.value), volume: c.volume, surround: c.surround }])} />
          <NumberField label="Volume" value={c.volume} min={0} max={64} onCommit={(volume) => run([{ op: "channel", index: ch, name: c.name, pan: c.pan, volume, surround: c.surround }])} />
          <label title="Heard the same in both ears, the far side inverted (Impulse Tracker's surround)">Surround</label>
          <input type="checkbox" checked={c.surround} onChange={(e) => run([{ op: "channel", index: ch, name: c.name, pan: c.pan, volume: c.volume, surround: e.currentTarget.checked }])} />
          <label title="Its sound goes through this effect slot (the Mix tab has them)">Effect slot</label>
          <select value={c.effect ?? -1} onChange={(e) => run([{ op: "channelEffect", index: ch, effect: Number(e.currentTarget.value) < 0 ? null : Number(e.currentTarget.value) }])}>
            <option value={-1}>None: the mix</option>
            {doc.effects.map((_, i) => (
              <option key={i} value={i}>
                {fxName(i)}
              </option>
            ))}
          </select>
        </div>
      )}
      <div className="tracker-list-buttons">
        <button type="button" className="small" data-testid="tracker-stats" onClick={onStats} title="How long it plays, and what it doesn't use (OpenMPT's Cleanup)">
          Song Info & Cleanup…
        </button>
        <button type="button" className="small" onClick={onMacros} title="What Zxx and \xx send">
          MIDI Macros…
        </button>
      </div>
      <p className="tracker-facts" data-testid="tracker-song-facts">
        {`${doc.formatName}${doc.tracker ? `, made with ${doc.tracker}` : ""}. Plays by ${doc.kind.toUpperCase()}'s rules. ${doc.patterns.length} patterns, ${doc.orders.length} orders, ${doc.samples.filter((s) => s.frames > 0).length} samples${doc.instrumentsMode ? `, ${doc.instruments.length} instruments` : ""}.`}
      </p>
    </div>
  );
}
