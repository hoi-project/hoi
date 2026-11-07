//! A one-shot WebSocket client: dial a hub, sync, run one action, sync again, leave. The node
//! owns the conversation ([`hoi_core::Node`] `net_*`); this only moves bytes. It mirrors
//! `hoi_net::dial` but does not loop forever — an agent invocation connects, acts, and exits, so
//! an LLM (or a human) can call it over and over as independent steps.

use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use hoi_core::{Node, Role};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default().max_message_size(Some(hoi_sync::MAX_MESSAGE)).max_frame_size(Some(hoi_sync::MAX_MESSAGE))
}

/// How long to wait for the hub to go quiet before deciding a sync is done, and the overall cap.
const QUIET: Duration = Duration::from_millis(400);
const DEADLINE: Duration = Duration::from_secs(20);

/// Connects to `url`, syncs, runs `act` once (its result is returned), syncs again, and closes.
/// The node is committed before returning. On a connection error the node still comes back (with
/// `act` run offline) so a caller can at least do local work and report it.
pub async fn session<T>(node: &mut Node, url: &str, mut act: impl FnMut(&mut Node) -> T) -> Result<T, String> {
    let (ws, _) =
        tokio_tungstenite::connect_async_with_config(url, Some(ws_config()), false).await.map_err(|e| format!("can't reach the hub at {url}: {e}"))?;
    let (mut sink, mut stream) = ws.split();

    // We dial our own home node; from our side the peer is "home" (hello carries our token).
    node.net_connect("hub", Role::Home);
    pump(node, &mut sink, &mut stream).await?;

    let out = act(node);

    // Push whatever the action produced and pull anything that came back.
    pump(node, &mut sink, &mut stream).await?;
    let _ = sink.close().await;
    node.net_disconnect("hub");
    node.commit().map_err(|e| e.to_string())?;
    Ok(out)
}

/// Connects, syncs, then pushes raw wire records straight at the hub (as an attacker's tool would,
/// speaking the sync protocol by hand), syncs again and closes. Returns whether the hub kept the
/// link (it drops peers that send too many invalid records).
pub async fn inject(node: &mut Node, url: &str, raw: Vec<Vec<u8>>) -> Result<bool, String> {
    let (ws, _) =
        tokio_tungstenite::connect_async_with_config(url, Some(ws_config()), false).await.map_err(|e| format!("can't reach the hub at {url}: {e}"))?;
    let (mut sink, mut stream) = ws.split();
    node.net_connect("hub", Role::Home);
    pump(node, &mut sink, &mut stream).await?;

    let mut kept = true;
    for chunk in raw.chunks(hoi_sync::MAX_RECS) {
        let msg = hoi_sync::Msg::Recs { recs: chunk.to_vec() };
        if sink.send(Message::Binary(msg.encode().into())).await.is_err() {
            kept = false;
            break;
        }
    }
    // Give the hub a moment to act, then drain.
    if kept {
        pump(node, &mut sink, &mut stream).await?;
    }
    let _ = sink.close().await;
    node.net_disconnect("hub");
    node.commit().map_err(|e| e.to_string())?;
    Ok(kept)
}

async fn pump<S, R>(node: &mut Node, sink: &mut S, stream: &mut R) -> Result<(), String>
where
    S: SinkExt<Message> + Unpin,
    <S as futures_util::Sink<Message>>::Error: std::fmt::Display,
    R: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let overall = Instant::now();
    let mut last = Instant::now();
    loop {
        // Flush anything the node queued.
        let outbox = node.take_outbox();
        let had_out = !outbox.is_empty();
        for (_, m) in outbox {
            sink.send(Message::Binary(m.into())).await.map_err(|e| format!("send failed: {e}"))?;
            last = Instant::now();
        }
        if had_out {
            continue;
        }
        if last.elapsed() >= QUIET || overall.elapsed() >= DEADLINE {
            break;
        }
        match tokio::time::timeout(QUIET, stream.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => {
                node.net_receive("hub", &b);
                last = Instant::now();
            }
            Ok(Some(Ok(Message::Ping(p)))) => {
                sink.send(Message::Pong(p)).await.map_err(|e| format!("pong failed: {e}"))?;
            }
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) => break,
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(e))) => return Err(format!("link error: {e}")),
            Err(_) => { /* quiet window with no inbound: loop to re-check the idle/deadline */ }
        }
    }
    Ok(())
}
