/**
 * The keys a phone or tablet plays and writes with, under the pattern:
 * a piano an octave and a note wide (the octave moved with − and +),
 * the cursor's moves, note off and cut, a row put in or taken out, and
 * the hex digits the instrument, volume and effect columns take. Each
 * one does what its computer key does in TrackerPanel.tsx.
 */
import * as tk from "../lib/tracker";

interface Props {
  octave: number;
  onOctave: (octave: number) => void;
  editing: boolean;
  onEditing: (on: boolean) => void;
  /** A computer key, as if typed into the pattern. */
  onKey: (key: string, code: string, shift?: boolean) => void;
  /** A piano key down (`semitone` over the octave's C) and up again. */
  onNoteDown: (semitone: number, id: string) => void;
  onNoteUp: (id: string) => void;
  onEffect: () => void;
}

const NOTES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B", "C"];
const HEX = "0123456789ABCDEF".split("");

export function TouchKeys({ octave, onOctave, editing, onEditing, onKey, onNoteDown, onNoteUp, onEffect }: Props) {
  const key = (label: string, title: string, k: string, code: string, shift = false) => (
    <button type="button" className="small" title={title} aria-label={title} onClick={() => onKey(k, code, shift)}>
      {label}
    </button>
  );
  return (
    <div className="touch-keys" data-testid="touch-keys">
      <div className="touch-row">
        <button type="button" className="small" aria-label="Octave down" onClick={() => onOctave(Math.max(0, octave - 1))}>
          −
        </button>
        <span className="touch-octave">{`Oct ${octave}`}</span>
        <button type="button" className="small" aria-label="Octave up" onClick={() => onOctave(Math.min(8, octave + 1))}>
          +
        </button>
        <button type="button" className={`small${editing ? " selected" : ""}`} aria-pressed={editing} onClick={() => onEditing(!editing)}>
          Edit
        </button>
        {key("↑", "Up a row", "ArrowUp", "ArrowUp")}
        {key("↓", "Down a row", "ArrowDown", "ArrowDown")}
        {key("←", "Left a column", "ArrowLeft", "ArrowLeft")}
        {key("→", "Right a column", "ArrowRight", "ArrowRight")}
        {key("⇥", "Next channel", "Tab", "Tab")}
        {key("===", "Note off", "`", "Backquote")}
        {key("^^^", "Note cut", "1", "Digit1")}
        {key("Del", "Clear what's under the cursor", "Delete", "Delete")}
        {key("+Row", "Put in a row", "Enter", "Enter", true)}
        {key("−Row", "Take out a row", "Backspace", "Backspace", true)}
        <button type="button" className="small" onClick={onEffect}>
          Effect…
        </button>
      </div>
      <div className="touch-row touch-hex" aria-label="Hex digits">
        {HEX.map((h) => (
          <button key={h} type="button" className="small" onClick={() => onKey(h.toLowerCase(), /\d/.test(h) ? `Digit${h}` : `Key${h}`)}>
            {h}
          </button>
        ))}
      </div>
      <div className="touch-piano" aria-label="Piano">
        {NOTES.map((n, i) => {
          const id = `touch-${i}`;
          return (
            <button
              key={i}
              type="button"
              className={`touch-note${n.includes("#") ? " sharp" : ""}`}
              aria-label={tk.noteWords(Math.min(120, octave * 12 + i + 1))}
              onPointerDown={(e) => {
                e.currentTarget.setPointerCapture(e.pointerId);
                onNoteDown(i, id);
              }}
              onPointerUp={() => onNoteUp(id)}
              onPointerCancel={() => onNoteUp(id)}
            >
              {n}
            </button>
          );
        })}
      </div>
    </div>
  );
}
