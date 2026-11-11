use serde::Deserialize;

#[cfg(any(target_os = "macos", windows))]
use super::komorebi::KomorebiProviderConfig;
#[cfg(windows)]
use super::{
  audio::AudioProviderConfig, keyboard::KeyboardProviderConfig,
  media::MediaProviderConfig, systray::SystrayProviderConfig,
};
use super::{
  battery::BatteryProviderConfig, cpu::CpuProviderConfig,
  disk::DiskProviderConfig, host::HostProviderConfig,
  ip::IpProviderConfig, memory::MemoryProviderConfig,
  network::NetworkProviderConfig, weather::WeatherProviderConfig,
};

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderConfig {
  #[cfg(windows)]
  Audio(AudioProviderConfig),
  Battery(BatteryProviderConfig),
  Cpu(CpuProviderConfig),
  Host(HostProviderConfig),
  Ip(IpProviderConfig),
  #[cfg(any(target_os = "macos", windows))]
  Komorebi(KomorebiProviderConfig),
  #[cfg(windows)]
  Media(MediaProviderConfig),
  Memory(MemoryProviderConfig),
  Disk(DiskProviderConfig),
  Network(NetworkProviderConfig),
  #[cfg(windows)]
  Systray(SystrayProviderConfig),
  Weather(WeatherProviderConfig),
  #[cfg(windows)]
  Keyboard(KeyboardProviderConfig),
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn polling_configs_require_positive_integer_intervals() {
    let providers = [
      "battery",
      "cpu",
      "host",
      "ip",
      "memory",
      "disk",
      "network",
      "weather",
      #[cfg(windows)]
      "keyboard",
    ];
    for provider in providers {
      for invalid in [
        serde_json::json!(0),
        serde_json::json!(-1),
        serde_json::json!(0.5),
      ] {
        let config = serde_json::json!({"type": provider, "refreshInterval": invalid});
        assert!(
          serde_json::from_value::<ProviderConfig>(config).is_err(),
          "{provider}"
        );
      }
      let config =
        serde_json::json!({"type": provider, "refreshInterval": 1});
      assert!(
        serde_json::from_value::<ProviderConfig>(config).is_ok(),
        "{provider}"
      );
    }
  }
}
