import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import "./App.css";
import "./responsive.css";
import { App } from "./App";

/** On the web, the warning the apps show natively before anything loads (src-tauri/src/first_run.rs). */
const ACCEPTED = "neumetik-tracker.in-development-accepted";
const isTauri = "__TAURI_INTERNALS__" in window;

function accepted(): boolean {
  try {
    return isTauri || localStorage.getItem(ACCEPTED) === "yes";
  } catch {
    return false;
  }
}

function FirstRun() {
  const [answer, setAnswer] = useState<"ok" | "later" | null>(accepted() ? "ok" : null);
  if (answer === "ok") return <App />;
  return (
    <div className="first-run">
      <div className="dialog-box" role="alertdialog" aria-labelledby="first-run-title" aria-describedby="first-run-words">
        <h2 id="first-run-title">Welcome to Neumetik Tracker</h2>
        {answer === "later" ? (
          <p className="gone">Come back any time.</p>
        ) : (
          <>
            <p id="first-run-words">Neumetik Tracker is still in development. Some features may be missing or not work as expected. Use it at your own risk.</p>
            <div className="dialog-actions">
              <button type="button" onClick={() => setAnswer("later")}>
                I'll Be Back.
              </button>
              <button
                type="button"
                className="primary"
                autoFocus
                onClick={() => {
                  try {
                    localStorage.setItem(ACCEPTED, "yes");
                  } catch {
                    // Asked again next time.
                  }
                  setAnswer("ok");
                }}
              >
                OK
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <FirstRun />
  </StrictMode>,
);
