use std::{
  cell::RefCell,
  collections::HashMap,
  os::fd::AsRawFd,
  sync::atomic::{AtomicBool, Ordering},
};

use objc2::{
  define_class, msg_send, rc::Retained, runtime::ProtocolObject, sel,
  DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::*;
use objc2_foundation::{
  NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol,
  NSPoint, NSRect, NSSize, NSString,
};

use crate::model::{workspace_command, Action, Model};

thread_local! { static CONTROLLER: RefCell<Option<Retained<Controller>>> = const { RefCell::new(None) }; }
static UPDATE_PENDING: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct Ui {
  panels: Vec<(NSRect, Retained<NSPanel>)>,
  actions: HashMap<isize, Action>,
  next_tag: isize,
  previous: Option<Model>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = RefCell<Ui>]
    struct Controller;

    unsafe impl NSObjectProtocol for Controller {}
    unsafe impl NSApplicationDelegate for Controller {}

    impl Controller {
        #[unsafe(method(clicked:))]
        fn clicked(&self, sender: &NSButton) {
            if let Some(action) = self.ivars().borrow().actions.get(&sender.tag()).cloned() { crate::action(action); }
        }
        #[unsafe(method(screensChanged:))]
        fn screens_changed(&self, _notification: &NSNotification) {
            // A transient empty topology during sleep must retain the panels.
            if !NSScreen::screens(self.mtm()).is_empty() {
                let mut ui = self.ivars().borrow_mut();
                for (_, panel) in ui.panels.drain(..) { panel.close(); }
                ui.previous = None;
            }
            invalidate();
        }
    }
);

define_class!(
    #[unsafe(super(NSButton))]
    #[thread_kind = MainThreadOnly]
    struct VolumeButton;
    impl VolumeButton {
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let delta = event.scrollingDeltaY();
            if delta.abs() > 0.1 { crate::action(Action::Volume(if delta > 0.0 { 0.05 } else { -0.05 })); }
        }
    }
);

pub fn invalidate() {
  if UPDATE_PENDING.swap(true, Ordering::AcqRel) {
    return;
  }
  dispatch2::DispatchQueue::main().exec_async(|| {
    UPDATE_PENDING.store(false, Ordering::Release);
    CONTROLLER.with(|controller| {
      if let Some(controller) = controller.borrow().as_ref() {
        controller.render();
      }
    });
  });
}

impl Controller {
  fn new(mtm: MainThreadMarker) -> Retained<Self> {
    let this = Self::alloc(mtm).set_ivars(RefCell::new(Ui::default()));
    unsafe { msg_send![super(this), init] }
  }

  fn render(&self) {
    let model = crate::SHARED
      .get()
      .unwrap()
      .model
      .lock()
      .unwrap_or_else(std::sync::PoisonError::into_inner)
      .clone();
    let mut ui = self.ivars().borrow_mut();
    if ui.previous.as_ref() == Some(&model) {
      return;
    }
    let mtm = self.mtm();
    let screens = NSScreen::screens(mtm);
    if screens.is_empty() {
      return;
    }
    let primary_top = screens.objectAtIndex(0).frame().max().y;
    if ui.panels.len() != screens.len() {
      for (_, panel) in ui.panels.drain(..) {
        panel.close();
      }
      for screen in screens.iter() {
        let visible = screen.visibleFrame();
        let frame = NSRect::new(
          NSPoint::new(visible.origin.x, visible.max().y - 40.0),
          NSSize::new(visible.size.width, 40.0),
        );
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
          NSPanel::alloc(mtm),
          frame,
          NSWindowStyleMask::Borderless
            | NSWindowStyleMask::NonactivatingPanel,
          NSBackingStoreType::Buffered,
          false,
        );
        unsafe {
          panel.setReleasedWhenClosed(false);
        }
        panel.setTitle(&NSString::from_str("Zebar Native"));
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(false);
        panel.setHidesOnDeactivate(false);
        panel.setLevel(NSMainMenuWindowLevel + 1);
        panel.setCollectionBehavior(
          NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
        );
        panel.orderFrontRegardless();
        ui.panels.push((frame, panel));
      }
    }
    ui.actions.clear();
    for index in 0..ui.panels.len() {
      let frame = ui.panels[index].0;
      let screen = screens.objectAtIndex(index);
      let screen_frame = screen.frame();
      let monitor = model.wm.monitor_at(
        screen_frame.origin.x.round() as i32,
        (primary_top - screen_frame.max().y).round() as i32,
      );
      let width = frame.size.width;
      let content = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::ZERO, frame.size),
      );
      let mut left = vec![("".into(), None, false)];
      if let Some(monitor) = monitor {
        for ws in &monitor.children {
          let apps = ws
            .applications()
            .iter()
            .map(|app| app_icon(app))
            .collect::<Vec<_>>()
            .join(" ");
          let name = ws.display_name.as_deref().unwrap_or(&ws.name);
          left.push((
            format!("{name}  {apps}"),
            workspace_command(&ws.name).map(Action::Wm),
            ws.has_focus,
          ));
        }
      } else {
        left.push(("GlazeWM: connecting…".into(), None, false));
      }
      let left_view = self.group(&mut ui, left, width * 0.4, mtm);
      left_view.setFrameOrigin(NSPoint::new(8.0, 6.0));
      content.addSubview(&left_view);

      let mut right = Vec::new();
      if !model.wm.connected {
        right.push(("WM offline".into(), None, true));
      }
      if model.wm.paused {
        right.push((
          "PAUSED".into(),
          Some(Action::Wm("command wm-toggle-pause".into())),
          true,
        ));
      }
      for mode in &model.wm.binding_modes {
        if !mode
          .chars()
          .any(|c| c.is_whitespace() || c == '"' || c == '\\')
        {
          right.push((
            mode.clone(),
            Some(Action::Wm(format!(
              "command wm-disable-binding-mode --name {mode}"
            ))),
            true,
          ));
        }
      }
      right.push((
        (if model.wm.direction == "vertical" {
          "↕"
        } else {
          "↔"
        })
        .into(),
        Some(Action::Wm("command toggle-tiling-direction".into())),
        false,
      ));
      if let Some((volume, muted)) = model.stats.volume {
        right.push((
          if muted {
            "♪ mute".into()
          } else {
            format!("♪ {volume}%")
          },
          Some(Action::Mute),
          false,
        ));
      }
      right.push((model.stats.network.clone(), None, false));
      right.push((format!("RAM {}%", model.stats.memory), None, false));
      right.push((
        format!("CPU {}%", model.stats.cpu),
        None,
        model.stats.cpu > 85,
      ));
      if let Some((percent, charging)) = model.stats.battery {
        right.push((
          format!("{} {percent}%", if charging { "ϟ" } else { "▰" }),
          None,
          percent <= 20 && !charging,
        ));
      }
      let right_view = self.group(&mut ui, right, width * 0.55, mtm);
      let right_x = width - right_view.frame().size.width - 8.0;
      right_view.setFrameOrigin(NSPoint::new(right_x, 6.0));
      content.addSubview(&right_view);

      let mut right_edge = right_x;
      if let Some(media) = &model.stats.media {
        let label = if media.artist.is_empty() {
          media.title.clone()
        } else {
          format!("{} — {}", media.artist, media.title)
        };
        let controls = vec![
          ("⏮".into(), Some(Action::Media("previous")), false),
          (
            (if media.playing { "⏸" } else { "▶" }).into(),
            Some(Action::Media("playpause")),
            false,
          ),
          ("⏭".into(), Some(Action::Media("next")), false),
          (label, Some(Action::Media("playpause")), false),
        ];
        let media_view = self.group(&mut ui, controls, width * 0.28, mtm);
        right_edge -= media_view.frame().size.width + 8.0;
        media_view.setFrameOrigin(NSPoint::new(right_edge, 6.0));
        content.addSubview(&media_view);
      }
      if let Some(focused) =
        model.wm.monitors.iter().find_map(|m| m.focused_window())
      {
        let label = format!(
          "{}  {}  {}",
          app_icon(&focused.process_name),
          focused.process_name,
          focused.title
        );
        let available = (right_edge - left_view.frame().max().x - 24.0)
          .max(0.0)
          .min(width * 0.4);
        if available > 60.0 {
          let center = self.group(
            &mut ui,
            vec![(label, None, false)],
            available,
            mtm,
          );
          let x = ((width - center.frame().size.width) / 2.0)
            .max(left_view.frame().max().x + 8.0)
            .min(right_edge - center.frame().size.width - 8.0);
          center.setFrameOrigin(NSPoint::new(x, 6.0));
          content.addSubview(&center);
        }
      }
      ui.panels[index].1.setContentView(Some(&content));
    }
    ui.previous = Some(model);
  }

  fn group(
    &self,
    ui: &mut Ui,
    items: Vec<(String, Option<Action>, bool)>,
    max_width: f64,
    mtm: MainThreadMarker,
  ) -> Retained<NSView> {
    let view = NSView::initWithFrame(
      NSView::alloc(mtm),
      NSRect::new(NSPoint::ZERO, NSSize::new(10.0, 28.0)),
    );
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
      layer.setCornerRadius(9.0);
      layer.setBackgroundColor(Some(
        &NSColor::colorWithCalibratedRed_green_blue_alpha(
          0.086, 0.086, 0.118, 0.92,
        )
        .CGColor(),
      ));
      layer.setBorderWidth(1.0);
      layer.setBorderColor(Some(
        &NSColor::colorWithCalibratedWhite_alpha(1.0, 0.08).CGColor(),
      ));
    }
    let mut x = 5.0;
    for (title, action, highlighted) in items {
      if action.is_none() {
        let label =
          NSTextField::labelWithString(&NSString::from_str(&title), mtm);
        label.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(
          12.0,
          unsafe { NSFontWeightRegular },
        )));
        label.setTextColor(Some(
          &NSColor::colorWithCalibratedWhite_alpha(1.0, 0.92),
        ));
        label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        label.setToolTip(Some(&NSString::from_str(&title)));
        label.sizeToFit();
        let label_width = (label.frame().size.width + 12.0)
          .min((max_width - x - 5.0).max(0.0));
        label.setFrame(NSRect::new(
          NSPoint::new(x + 6.0, 5.0),
          NSSize::new((label_width - 12.0).max(0.0), 18.0),
        ));
        if label_width > 0.0 {
          view.addSubview(&label);
          x += label_width;
        }
        continue;
      }
      let button: Retained<NSButton> = if matches!(
        action,
        Some(Action::Mute)
      ) {
        let button: Retained<VolumeButton> = unsafe {
          msg_send![VolumeButton::alloc(mtm), initWithFrame: NSRect::ZERO]
        };
        unsafe {
          button.setTarget(Some(self));
          button.setAction(Some(sel!(clicked:)));
        }
        button.into_super()
      } else {
        unsafe {
          NSButton::buttonWithTitle_target_action(
            &NSString::from_str(&title),
            Some(self),
            Some(sel!(clicked:)),
            mtm,
          )
        }
      };
      button.setTitle(&NSString::from_str(&title));
      button.setBordered(false);
      button.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(
        12.0,
        unsafe { NSFontWeightRegular },
      )));
      button.setContentTintColor(Some(
        &NSColor::colorWithCalibratedWhite_alpha(1.0, 0.92),
      ));
      button.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
      button.setToolTip(Some(&NSString::from_str(&title)));
      button.sizeToFit();
      let button_width = (button.frame().size.width + 12.0)
        .min((max_width - x - 5.0).max(0.0));
      button.setFrame(NSRect::new(
        NSPoint::new(x, 4.0),
        NSSize::new(button_width, 20.0),
      ));
      if highlighted {
        button.setWantsLayer(true);
        if let Some(layer) = button.layer() {
          layer.setCornerRadius(6.0);
          layer.setBackgroundColor(Some(
            &NSColor::colorWithCalibratedRed_green_blue_alpha(
              0.37, 0.49, 1.0, 0.85,
            )
            .CGColor(),
          ));
        }
      }
      if let Some(action) = action {
        ui.next_tag += 1;
        button.setTag(ui.next_tag);
        ui.actions.insert(ui.next_tag, action);
      } else {
        unsafe {
          button.setTarget(None);
          button.setAction(None);
        }
      }
      if button_width > 0.0 {
        view.addSubview(&button);
        x += button_width;
      }
    }
    view.setFrameSize(NSSize::new(x + 5.0, 28.0));
    view
  }
}

fn app_icon(name: &str) -> &'static str {
  match name.to_lowercase().as_str() {
    "firefox" => "🦊",
    "ghostty" | "terminal" => "⌘",
    "telegram" => "➤",
    "microsoft outlook" => "✉",
    "docker desktop" => "▣",
    "pt messenger" | "толк" => "☏",
    "safari" | "google chrome" => "◎",
    "music" | "spotify" | "яндекс музыка" => "♫",
    _ => "□",
  }
}

pub fn run(start_workers: impl FnOnce()) -> anyhow::Result<()> {
  let mtm = MainThreadMarker::new()
    .ok_or_else(|| anyhow::anyhow!("AppKit requires the main thread"))?;
  let lock_path = std::env::var_os("HOME")
    .map(std::path::PathBuf::from)
    .ok_or_else(|| anyhow::anyhow!("Home directory unavailable"))?
    .join("Library/Application Support/zebar-native.lock");
  let lock = std::fs::OpenOptions::new()
    .create(true)
    .truncate(false)
    .read(true)
    .write(true)
    .open(lock_path)?;
  if unsafe {
    libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB)
  } != 0
  {
    return Ok(());
  }
  let app = NSApplication::sharedApplication(mtm);
  app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
  let controller = Controller::new(mtm);
  app.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
  unsafe {
    NSNotificationCenter::defaultCenter()
      .addObserver_selector_name_object(
        &controller,
        sel!(screensChanged:),
        Some(NSApplicationDidChangeScreenParametersNotification),
        None,
      );
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    for name in [
      NSWorkspaceDidWakeNotification,
      NSWorkspaceScreensDidWakeNotification,
      NSWorkspaceSessionDidBecomeActiveNotification,
    ] {
      center.addObserver_selector_name_object(
        &controller,
        sel!(screensChanged:),
        Some(name),
        None,
      );
    }
  }
  CONTROLLER.with(|slot| *slot.borrow_mut() = Some(controller.clone()));
  controller.render();
  start_workers();
  app.run();
  unsafe {
    NSNotificationCenter::defaultCenter().removeObserver(&controller);
    NSWorkspace::sharedWorkspace()
      .notificationCenter()
      .removeObserver(&controller);
  }
  CONTROLLER.with(|slot| slot.borrow_mut().take());
  Ok(())
}
