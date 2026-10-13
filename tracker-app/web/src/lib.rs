// Copyright 2026 ansiapps. Neumetik Tracker: MIT licensed, see the LICENSE at the repository root.

//! **The tracker in the browser**: the session ([`Session`]) and its
//! sound ([`Engine`]) in one WebAssembly module, with no JavaScript glue,
//! so it runs where the browser makes sound (an AudioWorklet, which has
//! no text decoder) as well as in a Worker. The page speaks to it in the
//! session's wire format (`Session::call_wire`):
//!
//! - `nt_init(rate)`: a session, its sound at `rate` (0: no sound, for
//!   the Worker that exports).
//! - `nt_alloc(len)`: room for a request; `nt_call(ptr, len)` takes it
//!   (and frees it) and returns the reply, `nt_reply_len()` bytes long,
//!   good until the next call.
//! - `nt_render(frames)`: the next frames, left then right, `frames` each.

use std::cell::RefCell;

use neumetik_tracker_session::{Audio, Cmd, Engine, Session, Status};

/// The sound, made right here as the AudioWorklet asks for it.
struct Web {
    engine: Engine,
    rate: u32,
    status: Status,
}

impl Audio for Web {
    fn rate(&self) -> Option<u32> {
        (self.rate > 0).then_some(self.rate)
    }

    fn send(&mut self, cmd: Cmd) {
        self.engine.apply(cmd);
    }

    fn status(&mut self) -> Status {
        self.engine.status(&mut self.status);
        self.status.clone()
    }
}

thread_local! {
    static SESSION: RefCell<Option<Session<Web>>> = const { RefCell::new(None) };
    static REPLY: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static OUT: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

#[no_mangle]
pub extern "C" fn nt_init(rate: u32) {
    let web = Web { engine: Engine::new(), rate, status: Status::default() };
    SESSION.with(|s| *s.borrow_mut() = Some(Session::new(web)));
}

/// Room for `len` bytes, for the page to write a request into.
#[no_mangle]
pub extern "C" fn nt_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// A request (made by `nt_alloc`, `len` bytes written) done; its reply.
///
/// # Safety
/// `ptr` must come from `nt_alloc(len)`, and isn't used after.
#[no_mangle]
pub unsafe extern "C" fn nt_call(ptr: *mut u8, len: usize) -> *const u8 {
    // SAFETY: nt_alloc made it with room for at least `len`, and the page wrote `len` bytes.
    let msg = unsafe { Vec::from_raw_parts(ptr, len, len.max(1)) };
    let reply = SESSION.with(|s| match s.borrow_mut().as_mut() {
        Some(session) => session.call_wire(&msg),
        None => neumetik_tracker_session::wire::reply(Err("The tracker hasn't started.".into())),
    });
    REPLY.with(|r| {
        let mut r = r.borrow_mut();
        *r = reply;
        r.as_ptr()
    })
}

#[no_mangle]
pub extern "C" fn nt_reply_len() -> usize {
    REPLY.with(|r| r.borrow().len())
}

/// The next `frames` of sound: left, then right.
#[no_mangle]
pub extern "C" fn nt_render(frames: usize) -> *const f32 {
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        if o.len() < frames * 2 {
            o.resize(frames * 2, 0.0);
        }
        let (l, r) = o[..frames * 2].split_at_mut(frames);
        l.fill(0.0);
        r.fill(0.0);
        SESSION.with(|s| {
            if let Some(session) = s.borrow_mut().as_mut() {
                session.audio().engine.render(l, r);
            }
        });
        o.as_ptr()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use neumetik_tracker_session::wire;

    fn call(cmd: &str, a: &str) -> Result<serde_json::Value, String> {
        let msg = wire::encode(cmd, &serde_json::from_str(a).unwrap(), &[]);
        let p = nt_alloc(msg.len());
        unsafe { std::ptr::copy_nonoverlapping(msg.as_ptr(), p, msg.len()) };
        let out = unsafe { nt_call(p, msg.len()) };
        let reply = unsafe { std::slice::from_raw_parts(out, nt_reply_len()) }.to_vec();
        wire::decode(&reply).map(|(j, _)| j)
    }

    #[test]
    fn the_abi_plays_a_note() {
        nt_init(48_000);
        call("new", r#"{"channels":4}"#).unwrap();
        call("play", r#"{"order":0,"row":0,"hold":false}"#).unwrap();
        let p = nt_render(256);
        let out = unsafe { std::slice::from_raw_parts(p, 512) };
        assert!(out.iter().all(|v| v.is_finite()));
        assert_eq!(call("status", "null").unwrap()["playing"], true);
        assert!(call("nothing", "null").is_err());
    }
}
