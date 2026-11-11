mod model;
#[cfg(target_os = "macos")]
mod system;
#[cfg(target_os = "macos")]
mod ui;
mod wm;

use std::sync::{Arc, Mutex, OnceLock};

use model::{Action, Model};

struct Shared {
  model: Mutex<Model>,
  wm_commands: tokio::sync::mpsc::Sender<String>,
  system_commands: std::sync::mpsc::SyncSender<Action>,
}
static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();

fn publish(update: impl FnOnce(&mut Model)) {
  if let Some(shared) = SHARED.get() {
    update(
      &mut shared
        .model
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    #[cfg(target_os = "macos")]
    ui::invalidate();
  }
}

fn action(action: Action) {
  let Some(shared) = SHARED.get() else {
    return;
  };
  match action {
    Action::Wm(command) => {
      if let Err(error) = shared.wm_commands.try_send(command) {
        eprintln!("WM command queue: {error}");
      }
    }
    action => {
      if let Err(error) = shared.system_commands.try_send(action) {
        eprintln!("System command queue: {error}");
      }
    }
  }
}

fn main() -> anyhow::Result<()> {
  if std::env::args().any(|a| a == "--version" || a == "-V") {
    println!("zebar {} (native Rust/AppKit)", env!("CARGO_PKG_VERSION"));
    return Ok(());
  }
  #[cfg(not(target_os = "macos"))]
  anyhow::bail!("The native edition requires macOS.");
  #[cfg(target_os = "macos")]
  {
    let (wm_tx, wm_rx) = tokio::sync::mpsc::channel(32);
    let (system_tx, system_rx) = std::sync::mpsc::sync_channel(16);
    SHARED
      .set(Arc::new(Shared {
        model: Mutex::new(Model::default()),
        wm_commands: wm_tx,
        system_commands: system_tx,
      }))
      .map_err(|_| anyhow::anyhow!("Already initialized"))?;
    ui::run(move || {
      std::thread::Builder::new()
        .name("wm-ipc".into())
        .spawn(move || {
          let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("IPC runtime");
          runtime.block_on(wm::run(wm_rx));
        })
        .expect("IPC thread");
      std::thread::Builder::new()
        .name("system-stats".into())
        .spawn(move || system::run(system_rx))
        .expect("Stats thread");
    })
  }
}
