// Copyright 2026 ansiapps. Neumetik Tracker: MIT licensed, see the LICENSE at the repository root.

//! Sound out of the speakers: one output stream (cpal) whose callback
//! owns the session's sound (`Engine`). Commands reach it through a
//! channel, and it says where the song is through `Shared`, which it
//! only ever tries to lock, so it never waits.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use neumetik_tracker_session::{Audio, Cmd, Engine, Status};

/// The sound output, as the session sees it.
pub struct Output {
    tx: Sender<Cmd>,
    rate: Option<u32>,
    shared: Arc<Mutex<Status>>,
    /// Kept so the sound plays on; None when there's no output.
    _stream: Option<Stream>,
}

/// cpal's stream, which may only be touched on the thread that made it on
/// some platforms: it's only ever kept, never used, once playing.
struct Stream(#[allow(dead_code)] cpal::Stream);
// SAFETY: the stream is never used after it's made; it's only kept alive
// (and dropped when the app quits).
unsafe impl Send for Stream {}

impl Audio for Output {
    fn rate(&self) -> Option<u32> {
        self.rate
    }

    fn send(&mut self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    fn status(&mut self) -> Status {
        self.shared.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Output {
    /// The default output, playing. When there's none, an output that
    /// plays nothing, and why.
    pub fn start() -> (Output, Option<String>) {
        let (tx, rx) = channel();
        let shared = Arc::new(Mutex::new(Status { levels: Vec::with_capacity(256), ..Status::default() }));
        match open(rx, shared.clone()) {
            Ok((stream, rate)) => (Output { tx, rate: Some(rate), shared, _stream: Some(Stream(stream)) }, None),
            Err(e) => (Output { tx, rate: None, shared, _stream: None }, Some(e)),
        }
    }
}

fn open(rx: Receiver<Cmd>, shared: Arc<Mutex<Status>>) -> Result<(cpal::Stream, u32), String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("No sound output was found. Check that speakers or headphones are connected.")?;
    let supported = device.default_output_config().map_err(|e| format!("The sound output couldn't be set up. ({e})"))?;
    let rate = supported.sample_rate().0;
    let channels = usize::from(supported.channels());
    let config: cpal::StreamConfig = supported.config();
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config, rx, shared, channels),
        cpal::SampleFormat::I16 => build::<i16>(&device, &config, rx, shared, channels),
        cpal::SampleFormat::U16 => build::<u16>(&device, &config, rx, shared, channels),
        cpal::SampleFormat::I32 => build::<i32>(&device, &config, rx, shared, channels),
        f => return Err(format!("The sound output's format ({f}) isn't one that's played.")),
    }?;
    stream.play().map_err(|e| format!("The sound output couldn't start. ({e})"))?;
    Ok((stream, rate))
}

/// Frames rendered at a time, at most: room made before the sound starts.
const BLOCK: usize = 4096;

fn build<T: SizedSample + FromSample<f32>>(device: &cpal::Device, config: &cpal::StreamConfig, rx: Receiver<Cmd>, shared: Arc<Mutex<Status>>, channels: usize) -> Result<cpal::Stream, String> {
    let mut engine = Engine::new();
    let (mut left, mut right) = (vec![0f32; BLOCK], vec![0f32; BLOCK]);
    let callback = move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
        while let Ok(cmd) = rx.try_recv() {
            engine.apply(cmd);
        }
        for chunk in data.chunks_mut(BLOCK * channels.max(1)) {
            let frames = chunk.len() / channels.max(1);
            let (l, r) = (&mut left[..frames], &mut right[..frames]);
            l.fill(0.0);
            r.fill(0.0);
            engine.render(l, r);
            for (i, frame) in chunk.chunks_mut(channels.max(1)).enumerate() {
                let (a, b) = (l[i].clamp(-1.0, 1.0), r[i].clamp(-1.0, 1.0));
                for (c, out) in frame.iter_mut().enumerate() {
                    *out = T::from_sample(match c {
                        0 => a,
                        1 => b,
                        _ => 0.0,
                    });
                }
                if channels == 1 {
                    frame[0] = T::from_sample((a + b) * 0.5);
                }
            }
        }
        if let Ok(mut s) = shared.try_lock() {
            engine.status(&mut s);
        }
    };
    device.build_output_stream(config, callback, |e| eprintln!("sound output error: {e}"), None).map_err(|e| format!("The sound output couldn't be opened. ({e})"))
}
