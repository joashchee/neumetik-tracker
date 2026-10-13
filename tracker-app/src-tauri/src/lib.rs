// Copyright 2026 ansiapps. Neumetik Tracker: MIT licensed, see the LICENSE at the repository root.

//! **Neumetik Tracker** on the desktop (macOS, Windows, Linux) and on
//! phones (Android, iOS): the session (`neumetik-tracker-session`) with
//! its sound out of the speakers (`audio.rs`). The screen sends every
//! command through `call`, in the session's wire format, as raw bytes
//! both ways; files are read and written by the screen (the fs plugin,
//! which understands Android's and iOS's documents too), except that on
//! the desktop a module is opened by its path (`open_path`), so the
//! files beside it can be found.

mod audio;
mod first_run;

use std::sync::Mutex;

use neumetik_tracker_session::{wire, Prepared, Session};
use serde_json::json;
use tauri::ipc::{InvokeBody, Request, Response};
use tauri::{AppHandle, Manager, State};

use audio::Output;

struct App {
    session: Mutex<Session<Output>>,
    /// Why there's no sound, when there isn't.
    no_sound: Option<String>,
}

fn busy() -> String {
    "The tracker is in a bad state. Restart Neumetik Tracker.".into()
}

/// A command in the session's wire format. One that takes a while
/// (playing the song through to export it) runs off the session's lock,
/// so the song keeps playing and the screen keeps its place.
#[tauri::command]
async fn call(app: State<'_, App>, request: Request<'_>) -> Result<Response, String> {
    let InvokeBody::Raw(body) = request.body() else { return Err("The app sent a command that couldn't be read.".into()) };
    let (cmd, a, data) = match wire::request(body) {
        Ok(r) => r,
        Err(e) => return Ok(Response::new(wire::reply(Err(e)))),
    };
    let prepared = app.session.lock().map_err(|_| busy())?.prepare(&cmd, a, data);
    let reply = match prepared {
        Prepared::Done(r) => r,
        Prepared::Job(job) => tauri::async_runtime::spawn_blocking(job).await.map_err(|e| format!("It stopped unexpectedly. ({e})"))?,
    };
    Ok(Response::new(wire::reply(reply)))
}

/// A module opened from its path (the desktop's), with the files beside
/// it to hand for the formats that keep their samples there.
#[tauri::command]
async fn open_path(app: State<'_, App>, path: String) -> Result<Response, String> {
    let p = std::path::PathBuf::from(&path);
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let reply = match std::fs::read(&p) {
        Ok(data) => {
            let near = neumetik_tracker::beside(&p);
            app.session.lock().map_err(|_| busy())?.open(&data, &name, Some(path), &near).and_then(|o| serde_json::to_value(o).map_err(|e| e.to_string())).map(|json| neumetik_tracker_session::Reply { json, bytes: Vec::new() })
        }
        Err(e) => Err(format!("{name} couldn't be read. ({e})")),
    };
    Ok(Response::new(wire::reply(reply)))
}

/// What the platform is, and whether there's sound.
#[tauri::command]
fn platform(app: State<'_, App>) -> serde_json::Value {
    json!({ "os": std::env::consts::OS, "mobile": cfg!(mobile), "noSound": app.no_sound })
}

/// The window, once the first-run warning is accepted.
fn start(app: &AppHandle) {
    let (output, no_sound) = Output::start();
    app.manage(App { session: Mutex::new(Session::new(output)), no_sound });
    let config = app.config().app.windows.first().cloned();
    let built = match config {
        Some(c) => tauri::WebviewWindowBuilder::from_config(app, &c).and_then(|b| b.build()),
        None => tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default()).build(),
    };
    if let Err(e) = built {
        eprintln!("the window couldn't be made: {e}");
        app.exit(1);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![call, open_path, platform])
        .setup(|app| {
            let handle = app.handle().clone();
            let dir = app.path().app_data_dir()?;
            if first_run::accepted(&dir) {
                start(&handle);
            } else {
                first_run::ask(&handle, &dir, start);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Neumetik Tracker");
}
