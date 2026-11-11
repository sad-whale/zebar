use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Container {
  #[serde(default)]
  pub id: String,
  #[serde(rename = "type", default)]
  pub kind: String,
  #[serde(default)]
  pub name: String,
  pub display_name: Option<String>,
  #[serde(default)]
  pub children: Vec<Container>,
  #[serde(default)]
  pub has_focus: bool,
  #[serde(default)]
  pub is_displayed: bool,
  #[serde(default)]
  pub process_name: String,
  #[serde(default)]
  pub title: String,
  #[serde(default)]
  pub x: i32,
  #[serde(default)]
  pub y: i32,
}

impl Container {
  pub fn focused_window(&self) -> Option<&Self> {
    if self.kind == "window" && self.has_focus {
      return Some(self);
    }
    self.children.iter().find_map(Self::focused_window)
  }

  pub fn applications(&self) -> Vec<&str> {
    fn collect<'a>(node: &'a Container, apps: &mut Vec<&'a str>) {
      if node.kind == "window"
        && !node.process_name.is_empty()
        && !apps.contains(&node.process_name.as_str())
      {
        apps.push(&node.process_name);
      }
      for child in &node.children {
        collect(child, apps);
      }
    }
    let mut result = Vec::new();
    collect(self, &mut result);
    result
  }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WmState {
  pub connected: bool,
  pub monitors: Vec<Container>,
  pub paused: bool,
  pub binding_modes: Vec<String>,
  pub direction: String,
}

impl WmState {
  pub fn monitor_at(&self, x: i32, y: i32) -> Option<&Container> {
    self.monitors.iter().min_by_key(|m| {
      let dx = i64::from(m.x) - i64::from(x);
      let dy = i64::from(m.y) - i64::from(y);
      dx * dx + dy * dy
    })
  }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Media {
  pub bundle: String,
  pub artist: String,
  pub title: String,
  pub playing: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
  pub cpu: u8,
  pub memory: u8,
  pub battery: Option<(u8, bool)>,
  pub network: String,
  pub volume: Option<(u8, bool)>,
  pub media: Option<Media>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
  pub wm: WmState,
  pub stats: Stats,
}

#[derive(Clone, Debug)]
pub enum Action {
  Wm(String),
  Mute,
  Volume(f32),
  Media(&'static str),
}

pub fn workspace_command(name: &str) -> Option<String> {
  // Names become CLI tokens, never shell commands.
  (!name.is_empty()
    && !name
      .chars()
      .any(|c| c.is_whitespace() || c == '"' || c == '\\'))
  .then(|| format!("command focus --workspace {name}"))
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn nested_windows_are_deduplicated_and_focus_survives_splits() {
    let root: Container = serde_json::from_value(serde_json::json!({"type":"monitor","children":[{"type":"workspace","children":[{"type":"split","children":[{"type":"window","processName":"Firefox","hasFocus":true}]},{"type":"window","processName":"Firefox"},{"type":"window","processName":"Ghostty"}]}]})).unwrap();
    assert_eq!(root.applications(), ["Firefox", "Ghostty"]);
    assert_eq!(root.focused_window().unwrap().process_name, "Firefox");
  }
  #[test]
  fn monitor_selection_handles_negative_coordinates_and_empty_topology() {
    let mut wm = WmState::default();
    assert!(wm.monitor_at(0, 0).is_none());
    wm.monitors = vec![
      Container {
        x: -3440,
        ..Container::default()
      },
      Container::default(),
    ];
    assert_eq!(wm.monitor_at(-3430, 0).unwrap().x, -3440);
  }
  #[test]
  fn workspace_names_cannot_inject_additional_cli_tokens() {
    assert_eq!(
      workspace_command("work-stuff").unwrap(),
      "command focus --workspace work-stuff"
    );
    assert!(workspace_command("work stuff").is_none());
    assert!(workspace_command("x\" --workspace 2").is_none());
    assert!(workspace_command("x\ncommand close").is_none());
  }
}
