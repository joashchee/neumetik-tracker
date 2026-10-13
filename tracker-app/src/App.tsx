/**
 * Neumetik Tracker's frame: its name and what's playing, the editor
 * (components/TrackerPanel.tsx), and a status line for what it says and
 * what it's doing. The platform (lib/host.ts) is made ready first.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import type { ActivityUpdate } from "./lib/activity";
import { host, type Host } from "./lib/host";
import { Dialog } from "./components/Dialog";
import { TrackerPanel } from "./components/TrackerPanel";

const MIT = `Copyright (c) 2026 ansiapps

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.`;

type Said = { text: string; error: boolean } | null;

export function App() {
  const [h, setH] = useState<Host | null>(null);
  const [said, setSaid] = useState<Said>(null);
  const [busy, setBusy] = useState<Map<string, string>>(new Map());
  const [playing, setPlaying] = useState(false);
  const [about, setAbout] = useState(false);
  const hRef = useRef<Host | null>(null);

  useEffect(() => {
    host().then(
      (made) => {
        hRef.current = made;
        setH(made);
        if (made.noSound) setSaid({ text: `There's no sound: ${made.noSound}`, error: true });
      },
      (e) => setSaid({ text: `The tracker couldn't start. (${String(e)})`, error: true }),
    );
  }, []);

  // A browser plays sound only once it's been touched or typed into: the first of either starts it.
  useEffect(() => {
    const wake = () => hRef.current?.wake();
    window.addEventListener("pointerdown", wake, true);
    window.addEventListener("keydown", wake, true);
    return () => {
      window.removeEventListener("pointerdown", wake, true);
      window.removeEventListener("keydown", wake, true);
    };
  }, []);

  const showStatus = useCallback((text: string) => setSaid({ text, error: false }), []);
  const showError = useCallback((text: string) => setSaid({ text: text.replace(/^Error: /, ""), error: true }), []);

  const runActivity = useCallback(async <T,>(label: string, task: (update: ActivityUpdate) => Promise<T>, key = label): Promise<T> => {
    setBusy((b) => new Map(b).set(key, label));
    try {
      return await task(() => {});
    } finally {
      setBusy((b) => {
        const n = new Map(b);
        n.delete(key);
        return n;
      });
    }
  }, []);
  const isBusy = useCallback((key: string) => busy.has(key), [busy]);
  const doing = Array.from(busy.values())[0];

  return (
    <div className="app">
      <header className="menu-bar">
        <span className="menu-app">
          <img className="app-mark" src={`${import.meta.env.BASE_URL}favicon.svg`} alt="" />
          Neumetik<span className="accent">.</span> Tracker
        </span>
        <span className={`play-dot${playing ? "" : " off"}`} role="img" aria-label={playing ? "Playing" : undefined} />
        <span className="menu-state">{h ? "" : "Starting…"}</span>
        <button type="button" className="small about" onClick={() => setAbout(true)}>
          About
        </button>
      </header>
      <main className="desk">
        {h && (
          <TrackerPanel
            runActivity={runActivity}
            isBusy={isBusy}
            showStatus={showStatus}
            showError={showError}
            audioReady={!h.noSound}
            active
            onPlaying={setPlaying}
            touch={h.mobile}
          />
        )}
      </main>
      <footer className="status-bar">
        <div aria-live="polite">
          {doing ? (
            <span className="scan-status-label">{doing}</span>
          ) : (
            said && (
              <p className={said.error ? "error" : "status-message"} title={said.text}>
                {said.text}
              </p>
            )
          )}
        </div>
      </footer>
      <Dialog open={about} onClose={() => setAbout(false)} title="About Neumetik Tracker" className="dialog-wide" actions={<button type="button" className="primary" onClick={() => setAbout(false)}>OK</button>}>
        <p>{`Version ${__APP_VERSION__}. Tracker modules played and edited, in some eighty formats, written from scratch on the formats' public descriptions.`}</p>
        <p>Open source under the MIT License:</p>
        <pre className="licenses-text">{MIT}</pre>
        <p>Its fonts are IBM Plex Sans and IBM Plex Mono, under the SIL Open Font License 1.1.</p>
      </Dialog>
    </div>
  );
}
