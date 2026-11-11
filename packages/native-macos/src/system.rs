use std::{
  sync::mpsc::{Receiver, RecvTimeoutError},
  time::Duration,
};

use objc2::{
  msg_send,
  rc::{autoreleasepool, Retained},
  runtime::{AnyClass, AnyObject},
};
use objc2_app_kit::NSRunningApplication;
use objc2_foundation::NSString;

use crate::model::{Action, Media, Stats};

#[repr(C)]
struct Address {
  selector: u32,
  scope: u32,
  element: u32,
}
#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
  fn AudioObjectGetPropertyData(
    object: u32,
    address: *const Address,
    qualifier_size: u32,
    qualifier: *const std::ffi::c_void,
    size: *mut u32,
    data: *mut std::ffi::c_void,
  ) -> i32;
  fn AudioObjectSetPropertyData(
    object: u32,
    address: *const Address,
    qualifier_size: u32,
    qualifier: *const std::ffi::c_void,
    size: u32,
    data: *const std::ffi::c_void,
  ) -> i32;
}
#[link(name = "ScriptingBridge", kind = "framework")]
unsafe extern "C" {}

fn fourcc(value: &[u8; 4]) -> u32 {
  u32::from_be_bytes(*value)
}
fn read<T: Copy + Default>(object: u32, address: &Address) -> Option<T> {
  let mut value = T::default();
  let mut size = std::mem::size_of::<T>() as u32;
  // The size and initialized buffer have exactly the requested C property
  // type.
  let status = unsafe {
    AudioObjectGetPropertyData(
      object,
      address,
      0,
      std::ptr::null(),
      &mut size,
      std::ptr::from_mut(&mut value).cast(),
    )
  };
  (status == 0 && size == std::mem::size_of::<T>() as u32).then_some(value)
}
fn write<T>(object: u32, address: &Address, value: &T) -> bool {
  unsafe {
    AudioObjectSetPropertyData(
      object,
      address,
      0,
      std::ptr::null(),
      std::mem::size_of::<T>() as u32,
      std::ptr::from_ref(value).cast(),
    ) == 0
  }
}
fn output_device() -> Option<u32> {
  read(
    1,
    &Address {
      selector: fourcc(b"dOut"),
      scope: fourcc(b"glob"),
      element: 0,
    },
  )
}
fn volume(device: u32) -> Option<(f32, Address)> {
  for (selector, element) in [(b"vmvc", 0), (b"volm", 0), (b"volm", 1)] {
    let address = Address {
      selector: fourcc(selector),
      scope: fourcc(b"outp"),
      element,
    };
    if let Some(value) =
      read::<f32>(device, &address).filter(|v| v.is_finite())
    {
      return Some((value.clamp(0.0, 1.0), address));
    }
  }
  None
}
fn audio() -> Option<(u8, bool)> {
  let device = output_device()?;
  let (volume, _) = volume(device)?;
  let muted: u32 = read(
    device,
    &Address {
      selector: fourcc(b"mute"),
      scope: fourcc(b"outp"),
      element: 0,
    },
  )
  .unwrap_or(0);
  Some(((volume * 100.0).round() as u8, muted != 0))
}
fn adjust_audio(action: &Action) {
  let Some(device) = output_device() else {
    return;
  };
  match action {
    Action::Mute => {
      let address = Address {
        selector: fourcc(b"mute"),
        scope: fourcc(b"outp"),
        element: 0,
      };
      if let Some(current) = read::<u32>(device, &address) {
        if !write(device, &address, &u32::from(current == 0)) {
          eprintln!("The audio device rejected mute.");
        }
      }
    }
    Action::Volume(delta) => {
      if let Some((current, address)) = volume(device) {
        let next = (current + delta).clamp(0.0, 1.0);
        if !write(device, &address, &next) {
          eprintln!("The audio device rejected volume.");
        }
        if address.element == 1 {
          let _ = write(
            device,
            &Address {
              element: 2,
              ..address
            },
            &next,
          );
        }
      }
    }
    _ => {}
  }
}

fn media_application() -> Option<(Retained<AnyObject>, &'static str)> {
  let class = AnyClass::get(c"SBApplication")?;
  for bundle in ["com.spotify.client", "com.apple.Music"] {
    let identifier = NSString::from_str(bundle);
    if NSRunningApplication::runningApplicationsWithBundleIdentifier(
      &identifier,
    )
    .is_empty()
    {
      continue;
    }
    let app: Option<Retained<AnyObject>> = unsafe {
      msg_send![class, applicationWithBundleIdentifier: &*identifier]
    };
    if let Some(app) = app {
      // Apple-event timeouts are measured in ticks (60 per second).
      unsafe {
        let _: () = msg_send![&*app, setTimeout: 120_i64];
      }
      return Some((app, bundle));
    }
  }
  None
}
fn media() -> Option<Media> {
  let (app, bundle) = media_application()?;
  let state: u32 = unsafe { msg_send![&*app, playerState] };
  if state != fourcc(b"kPSP") && state != fourcc(b"kPSp") {
    return None;
  }
  let track: Option<Retained<AnyObject>> =
    unsafe { msg_send![&*app, currentTrack] };
  let track = track?;
  let title: Option<Retained<NSString>> =
    unsafe { msg_send![&*track, name] };
  let artist: Option<Retained<NSString>> =
    unsafe { msg_send![&*track, artist] };
  Some(Media {
    bundle: bundle.into(),
    title: title?.to_string(),
    artist: artist.map(|s| s.to_string()).unwrap_or_default(),
    playing: state == fourcc(b"kPSP"),
  })
}
fn control_media(command: &str) {
  let Some((app, _)) = media_application() else {
    return;
  };
  unsafe {
    match command {
      "playpause" => {
        let _: () = msg_send![&*app, playpause];
      }
      "previous" => {
        let _: () = msg_send![&*app, previousTrack];
      }
      "next" => {
        let _: () = msg_send![&*app, nextTrack];
      }
      _ => {}
    }
  }
}

pub fn run(commands: Receiver<Action>) {
  let mut system = sysinfo::System::new();
  let batteries = starship_battery::Manager::new().ok();
  loop {
    autoreleasepool(|_| {
      system.refresh_cpu();
      system.refresh_memory();
      let interfaces = netdev::get_interfaces();
      let physical = interfaces.iter().find(|i| {
        i.is_up()
          && (!i.ipv4.is_empty()
            || i.ipv6.iter().any(|ip| !ip.addr().is_unicast_link_local()))
          && matches!(
            i.if_type,
            netdev::interface::InterfaceType::Wireless80211
              | netdev::interface::InterfaceType::Ethernet
          )
      });
      let network = match physical.map(|i| i.if_type) {
        Some(netdev::interface::InterfaceType::Wireless80211) => "Wi-Fi",
        Some(_) => "Ethernet",
        None => "Offline",
      }
      .to_owned();
      let battery = batteries
        .as_ref()
        .and_then(|manager| manager.batteries().ok()?.find_map(Result::ok))
        .map(|b| {
          (
            (b.state_of_charge().value * 100.0).round() as u8,
            b.state() == starship_battery::State::Charging,
          )
        });
      let stats = Stats {
        cpu: system
          .global_cpu_info()
          .cpu_usage()
          .round()
          .clamp(0.0, 100.0) as u8,
        memory: (system.used_memory() as f64
          / system.total_memory().max(1) as f64
          * 100.0)
          .round() as u8,
        battery,
        network,
        volume: audio(),
        media: media(),
      };
      crate::publish(|model| model.stats = stats);
    });
    match commands.recv_timeout(Duration::from_secs(2)) {
      Ok(Action::Media(command)) => {
        autoreleasepool(|_| control_media(command))
      }
      Ok(action) => adjust_audio(&action),
      Err(RecvTimeoutError::Timeout) => {}
      Err(RecvTimeoutError::Disconnected) => break,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn scripting_bridge_is_linked_and_audio_codes_match_the_sdk() {
    assert!(AnyClass::get(c"SBApplication").is_some());
    assert_eq!(fourcc(b"dOut"), 0x644f7574);
    assert_eq!(fourcc(b"kPSP"), 0x6b505350);
  }
}
