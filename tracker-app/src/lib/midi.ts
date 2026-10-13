/**
 * A MIDI keyboard, through the Web MIDI API where the platform has it
 * (Chrome, Edge and Firefox, and the app on Windows): its notes play and
 * are written as the computer keyboard's are. Elsewhere there are no
 * inputs to choose, and nothing else changes.
 */
export interface Note {
  /** MIDI's numbers: 60 is middle C. */
  note: number;
  velocity: number;
  on: boolean;
}

let access: Promise<MIDIAccess | null> | null = null;

function midi(): Promise<MIDIAccess | null> {
  access ??= "requestMIDIAccess" in navigator ? navigator.requestMIDIAccess().catch(() => null) : Promise.resolve(null);
  return access;
}

/** The MIDI keyboards there are, by name; none where there's no MIDI. */
export async function inputs(): Promise<string[]> {
  const a = await midi();
  return a ? Array.from(a.inputs.values()).map((i) => i.name ?? i.id) : [];
}

/** Listens to the input named (none: to nothing), each note to `on`. */
export async function listenTo(name: string | null, on: (n: Note) => void): Promise<void> {
  const a = await midi();
  if (!a) return;
  for (const input of a.inputs.values()) {
    const chosen = name !== null && (input.name ?? input.id) === name;
    input.onmidimessage = chosen
      ? (e) => {
          const d = e.data;
          if (!d || d.length < 3) return;
          const kind = d[0] & 0xf0;
          if (kind === 0x90 && d[2] > 0) on({ note: d[1], velocity: d[2], on: true });
          else if (kind === 0x80 || (kind === 0x90 && d[2] === 0)) on({ note: d[1], velocity: 0, on: false });
        }
      : null;
  }
}
