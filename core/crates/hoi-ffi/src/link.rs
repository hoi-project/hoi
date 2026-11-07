//! The phone's link to its home node: a WebSocket kept up by [`hoi_net::dial`] while the app is
//! in the foreground, stopped in the background (TECHNICAL.md §4: no background sockets).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hoi_net::Shared;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;

use crate::events::Events;

/// How often a stopped link's leftovers are closed while its dial loop winds down.
const REAP_EVERY: Duration = Duration::from_millis(200);
/// How long a stopped dial loop gets to notice before it is cancelled.
const WIND_DOWN: Duration = Duration::from_secs(2);
/// How often the link's status is checked for `net` events.
const WATCH_EVERY: Duration = Duration::from_secs(1);

struct Active {
    url: String,
    stop: Arc<AtomicBool>,
    task: JoinHandle<()>,
}

#[derive(Default)]
pub(crate) struct Link {
    active: Option<Active>,
    /// A stopped loop still winding down; the next link waits for it.
    winding: Option<JoinHandle<()>>,
}

impl Link {
    pub(crate) fn is_running(&self) -> bool {
        self.active.as_ref().is_some_and(|a| !a.task.is_finished())
    }

    /// Starts (or keeps) a link to `url`. Returns false when it was already up.
    pub(crate) fn start(&mut self, rt: &Handle, shared: &Arc<Shared>, events: &Arc<Events>, url: String) -> bool {
        if self.active.as_ref().is_some_and(|a| a.url == url && !a.task.is_finished()) {
            return false;
        }
        self.stop(rt, shared);
        let prev = self.winding.take();
        let stop = Arc::new(AtomicBool::new(false));
        let keep_going: Arc<dyn Fn() -> bool + Send + Sync> = {
            let stop = stop.clone();
            Arc::new(move || !stop.load(Ordering::SeqCst))
        };
        let task = {
            let shared = shared.clone();
            let url = url.clone();
            rt.spawn(async move {
                if let Some(prev) = prev {
                    let _ = prev.await;
                }
                hoi_net::dial(shared, url, keep_going).await;
            })
        };
        rt.spawn(watch(shared.clone(), events.clone(), stop.clone()));
        self.active = Some(Active { url, stop, task });
        true
    }

    /// Stops the link. Its socket closes now; the dial loop winds down in the background.
    pub(crate) fn stop(&mut self, rt: &Handle, shared: &Arc<Shared>) {
        let Some(a) = self.active.take() else { return };
        a.stop.store(true, Ordering::SeqCst);
        shared.close_all("home-");
        let prev = self.winding.take();
        let shared = shared.clone();
        self.winding = Some(rt.spawn(async move {
            if let Some(prev) = prev {
                let _ = prev.await;
            }
            wind_down(shared, a.task).await;
        }));
    }
}

/// Waits for a stopped dial loop to return. A connect that was still in flight when the link
/// stopped can finish afterwards, so whatever it opens is closed until the loop notices. A loop
/// still running after [`WIND_DOWN`] is sleeping out a retry backoff or stuck in a slow connect,
/// neither of which holds a link, so it is cancelled.
async fn wind_down(shared: Arc<Shared>, mut task: JoinHandle<()>) {
    let deadline = tokio::time::sleep(WIND_DOWN);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut task => return,
            _ = &mut deadline => break,
            _ = tokio::time::sleep(REAP_EVERY) => shared.close_all("home-"),
        }
    }
    shared.close_all("home-");
    tokio::time::sleep(Duration::from_millis(50)).await;
    task.abort();
    let _ = task.await;
}

/// Sends `net` events while a link runs, and once more after it stops.
async fn watch(shared: Arc<Shared>, events: Arc<Events>, stop: Arc<AtomicBool>) {
    let mut tick = tokio::time::interval(WATCH_EVERY);
    let mut after_stop = 0;
    loop {
        tick.tick().await;
        let st = shared.with(|n| n.net_status());
        let connected = st.home_node.as_ref().is_some_and(|h| h.connected);
        events.net(&st);
        if stop.load(Ordering::SeqCst) {
            after_stop += 1;
            if !connected || after_stop > 3 {
                events.flush_sync();
                return;
            }
        }
    }
}
