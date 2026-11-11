use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::{net::TcpStream, sync::mpsc};
use tokio_tungstenite::{
  connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream,
};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
const QUERIES: [&str; 4] = [
  "query monitors",
  "query paused",
  "query binding-modes",
  "query tiling-direction",
];

pub async fn run(mut commands: mpsc::Receiver<String>) {
  loop {
    if let Err(error) = connected(&mut commands).await {
      crate::publish(|m| m.wm.connected = false);
      eprintln!("GlazeWM connection: {error:#}");
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    while commands.try_recv().is_ok() {} // Never replay stale actions
                                         // after reconnect.
  }
}

async fn connected(commands: &mut mpsc::Receiver<String>) -> Result<()> {
  let (mut socket, _) = tokio::time::timeout(
    Duration::from_secs(3),
    connect_async("ws://127.0.0.1:6123"),
  )
  .await??;
  socket.send(Message::Text("sub -e all".into())).await?;
  let mut pending = std::collections::HashSet::new();
  request(&mut socket, &mut pending, &QUERIES).await?;
  let mut watchdog = tokio::time::interval(Duration::from_secs(15));
  watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
  let mut response_deadline = None;
  let mut dirty = std::collections::HashSet::<String>::new();
  loop {
    tokio::select! {
        command = commands.recv() => {
            let Some(command) = command else { return Ok(()); };
            socket.send(Message::Text(command.into())).await?;
        }
        message = socket.next() => {
            let message = message.context("IPC closed")??;
            if message.is_close() { anyhow::bail!("IPC closed"); }
            if !message.is_text() { continue; }
            let reply: Value = serde_json::from_str(message.to_text()?)?;
            if reply["success"] == false { eprintln!("GlazeWM rejected a request: {}", reply["error"]); continue; }
            if reply.get("clientMessage").is_some() {
                if let Some(command) = reply["clientMessage"].as_str() {
                    pending.remove(command);
                    apply_reply(&reply)?;
                    if dirty.remove(command) { request(&mut socket, &mut pending, &[command]).await?; }
                }
            } else if let Some(kind) = reply["data"]["eventType"].as_str() {
                let data = &reply["data"];
                match kind {
                    "pause_changed" => crate::publish(|m| m.wm.paused = data["isPaused"].as_bool().unwrap_or(false)),
                    "binding_modes_changed" => crate::publish(|m| m.wm.binding_modes = binding_names(&data["newBindingModes"])),
                    "tiling_direction_changed" => crate::publish(|m| m.wm.direction = data["newTilingDirection"].as_str().unwrap_or("horizontal").into()),
                    _ => {},
                }
                let queries = event_queries(kind);
                for query in queries { if pending.contains(*query) { dirty.insert((*query).into()); } }
                request(&mut socket, &mut pending, queries).await?;
            }
            if pending.is_empty() { response_deadline = None; }
        }
        _ = watchdog.tick() => {
            if response_deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) { anyhow::bail!("IPC response timeout"); }
            request(&mut socket, &mut pending, &QUERIES).await?;
            response_deadline = Some(std::time::Instant::now() + Duration::from_secs(15));
        }
    }
  }
}

async fn request(
  socket: &mut Socket,
  pending: &mut std::collections::HashSet<String>,
  queries: &[&str],
) -> Result<()> {
  for &query in queries {
    if pending.insert(query.into()) {
      socket.send(Message::Text(query.into())).await?;
    }
  }
  Ok(())
}

fn apply_reply(reply: &Value) -> Result<()> {
  let command = reply["clientMessage"].as_str().unwrap_or_default();
  let data = &reply["data"];
  match command {
    "query monitors" => {
      let monitors = serde_json::from_value(data["monitors"].clone())?;
      crate::publish(|m| {
        m.wm.monitors = monitors;
        m.wm.connected = true;
      });
    }
    "query paused" => {
      crate::publish(|m| m.wm.paused = data.as_bool().unwrap_or(false))
    }
    "query binding-modes" => crate::publish(|m| {
      m.wm.binding_modes = binding_names(&data["bindingModes"])
    }),
    "query tiling-direction" => crate::publish(|m| {
      m.wm.direction = data["tilingDirection"]
        .as_str()
        .unwrap_or("horizontal")
        .into()
    }),
    _ => {}
  }
  Ok(())
}

fn binding_names(value: &Value) -> Vec<String> {
  value
    .as_array()
    .map(|modes| {
      modes
        .iter()
        .filter_map(|v| v["name"].as_str().map(str::to_owned))
        .collect()
    })
    .unwrap_or_default()
}

fn event_queries(kind: &str) -> &'static [&'static str] {
  match kind {
    "focus_changed" | "focused_container_moved" => {
      &["query monitors", "query tiling-direction"]
    }
    "window_managed"
    | "window_unmanaged"
    | "workspace_activated"
    | "workspace_deactivated"
    | "workspace_updated"
    | "monitor_added"
    | "monitor_removed"
    | "monitor_updated" => &["query monitors"],
    "user_config_changed" => &QUERIES,
    _ => &[],
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn lifecycle_queries_are_incremental_and_ignore_unrelated_events() {
    assert_eq!(event_queries("window_managed"), ["query monitors"]);
    assert_eq!(
      event_queries("focus_changed"),
      ["query monitors", "query tiling-direction"]
    );
    assert!(event_queries("pause_changed").is_empty());
    assert!(event_queries("application_exiting").is_empty());
  }
}
