//! Events to the app, plus the little bookkeeping phones need around them: when records last
//! changed (to tell when a sync burst has gone quiet) and when the home node was last reached
//! ("last synced 4 min ago").

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use hoi_core::dto::{Event, NetStatus};
use serde_json::json;

use crate::EventListener;

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Write "last synced" to disk at most this often while connected.
const SYNC_WRITE_EVERY: u64 = 30;

pub(crate) struct Events {
    listener: Arc<dyn EventListener>,
    last_net: Mutex<Option<String>>,
    last_change: Mutex<Instant>,
    /// (last time the home node was reached, last value written to `sync_file`)
    last_sync: Mutex<(Option<u64>, u64)>,
    sync_file: PathBuf,
}

impl Events {
    pub(crate) fn new(listener: Arc<dyn EventListener>, sync_file: PathBuf) -> Events {
        let saved = std::fs::read_to_string(&sync_file).ok().and_then(|s| s.trim().parse::<u64>().ok());
        Events { listener, last_net: Mutex::new(None), last_change: Mutex::new(Instant::now()), last_sync: Mutex::new((saved, saved.unwrap_or(0))), sync_file }
    }

    pub(crate) fn emit(&self, json: String) {
        self.listener.on_event(json);
    }

    /// Core events from `take_events()`, one JSON string each.
    pub(crate) fn core(&self, events: Vec<Event>) {
        for e in events {
            if let Event::Net { status } = &e {
                self.net(status);
                continue;
            }
            if matches!(e, Event::Changed { .. }) {
                *lock(&self.last_change) = Instant::now();
            }
            if let Ok(s) = serde_json::to_string(&e) {
                self.emit(s);
            }
        }
    }

    /// A `net` event when the status differs from the last one sent. Notes contact with the
    /// home node for "last synced".
    pub(crate) fn net(&self, st: &NetStatus) {
        if st.home_node.as_ref().is_some_and(|h| h.connected) {
            self.synced_now();
        }
        let s = json!({ "type": "net", "status": st }).to_string();
        let mut last = lock(&self.last_net);
        if last.as_deref() != Some(s.as_str()) {
            *last = Some(s.clone());
            drop(last);
            self.emit(s);
        }
    }

    pub(crate) fn stamp_start(&self, bits: u32, estimate_ms: u64) {
        self.emit(json!({ "type": "stamp", "phase": "start", "bits": bits, "estimateMs": estimate_ms }).to_string());
    }

    pub(crate) fn stamp_done(&self, bits: u32, estimate_ms: u64, took_ms: u64) {
        self.emit(json!({ "type": "stamp", "phase": "done", "bits": bits, "estimateMs": estimate_ms, "tookMs": took_ms }).to_string());
    }

    pub(crate) fn since_change(&self) -> std::time::Duration {
        lock(&self.last_change).elapsed()
    }

    fn synced_now(&self) {
        let now = unix_now();
        let mut s = lock(&self.last_sync);
        s.0 = Some(now);
        if now >= s.1 + SYNC_WRITE_EVERY {
            s.1 = now;
            let _ = std::fs::write(&self.sync_file, now.to_string());
        }
    }

    /// Writes the latest "last synced" now (link stopping, app going away).
    pub(crate) fn flush_sync(&self) {
        let mut s = lock(&self.last_sync);
        if let Some(t) = s.0
            && t != s.1
        {
            s.1 = t;
            let _ = std::fs::write(&self.sync_file, t.to_string());
        }
    }

    pub(crate) fn last_synced_at(&self) -> Option<u64> {
        lock(&self.last_sync).0
    }

    pub(crate) fn forget_sync(&self) {
        *lock(&self.last_sync) = (None, 0);
        let _ = std::fs::remove_file(&self.sync_file);
    }
}
