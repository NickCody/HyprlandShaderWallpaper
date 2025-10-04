use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to parse configuration: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaylistMode {
    Continuous,
    Shuffle,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MultiConfig {
    pub version: u32,
    #[serde(
        default = "default_workspace_crossfade",
        deserialize_with = "deserialize_duration_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub workspace_switch_crossfade: Option<Duration>,
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub playlists: BTreeMap<String, Playlist>,
    #[serde(default)]
    pub targets: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Defaults {
    pub playlist: Option<String>,
    pub fps: Option<f32>,
    #[serde(default, deserialize_with = "deserialize_antialias_opt")]
    pub antialias: Option<AntialiasSetting>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Playlist {
    pub mode: PlaylistMode,
    #[serde(
        default = "default_duration",
        deserialize_with = "deserialize_duration"
    )]
    pub item_duration: Duration,
    #[serde(
        default = "default_crossfade",
        deserialize_with = "deserialize_duration"
    )]
    pub crossfade: Duration,
    #[serde(default)]
    pub fps: Option<f32>,
    #[serde(default, deserialize_with = "deserialize_antialias_opt")]
    pub antialias: Option<AntialiasSetting>,
    #[serde(default)]
    pub items: Vec<PlaylistItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaylistItem {
    pub handle: String,
    #[serde(default, deserialize_with = "deserialize_duration_opt")]
    pub duration: Option<Duration>,
    #[serde(default)]
    pub fps: Option<f32>,
    #[serde(default, deserialize_with = "deserialize_antialias_opt")]
    pub antialias: Option<AntialiasSetting>,
    #[serde(default)]
    pub refresh_once: bool,
    #[serde(default)]
    pub mode: Option<PlaylistItemMode>,
    #[serde(default, deserialize_with = "deserialize_duration_opt")]
    pub still_time: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaylistItemMode {
    Animate,
    Still,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AntialiasSetting {
    Auto,
    Off,
    Samples2,
    Samples4,
    Samples8,
    Samples16,
}

impl AntialiasSetting {
    pub fn from_samples(samples: u32) -> Option<Self> {
        match samples {
            0 | 1 => Some(Self::Off),
            2 => Some(Self::Samples2),
            4 => Some(Self::Samples4),
            8 => Some(Self::Samples8),
            16 => Some(Self::Samples16),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedItem {
    pub handle: String,
    pub duration: Duration,
    pub fps: Option<f32>,
    pub antialias: Option<AntialiasSetting>,
    pub refresh_once: bool,
    pub crossfade: Duration,
}

fn default_workspace_crossfade() -> Option<Duration> {
    Some(Duration::from_secs_f32(1.0))
}

fn default_crossfade() -> Duration {
    Duration::from_secs_f32(1.0)
}

fn default_duration() -> Duration {
    Duration::from_secs(60)
}

fn deserialize_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_duration_opt(deserializer).map(|d| d.unwrap_or(default_duration()))
}

fn deserialize_duration_opt<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    struct Visitor;
    impl<'de> de::Visitor<'de> for Visitor {
        type Value = Option<Duration>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a duration as number of seconds or human-readable string")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            humantime::parse_duration(v)
                .map(Some)
                .map_err(|err| E::custom(format!("invalid duration '{v}': {err}")))
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if v == 0 {
                Ok(Some(Duration::from_secs(0)))
            } else {
                Ok(Some(Duration::from_secs(v)))
            }
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if v < 0 {
                return Err(E::custom("duration must be non-negative"));
            }
            Ok(Some(Duration::from_secs(v as u64)))
        }

        fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if v.is_nan() || v.is_sign_negative() {
                return Err(E::custom("duration must be non-negative"));
            }
            Ok(Some(Duration::from_secs_f64(v)))
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }
    }

    deserializer.deserialize_any(Visitor)
}

fn deserialize_antialias_opt<'de, D>(deserializer: D) -> Result<Option<AntialiasSetting>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Helper {
        Str(String),
        Num(i64),
    }

    let helper: Option<Helper> = Option::deserialize(deserializer)?;
    let result = match helper {
        None => None,
        Some(Helper::Str(raw)) => Some(parse_antialias(&raw).map_err(de::Error::custom)?),
        Some(Helper::Num(value)) => {
            if value < 0 {
                return Err(de::Error::custom("antialias value must be non-negative"));
            }
            let raw = value.to_string();
            Some(parse_antialias(&raw).map_err(de::Error::custom)?)
        }
    };
    Ok(result)
}

fn parse_antialias(raw: &str) -> Result<AntialiasSetting, String> {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "auto" | "max" | "default" => Ok(AntialiasSetting::Auto),
        "off" | "none" | "disable" | "disabled" | "0" => Ok(AntialiasSetting::Off),
        "2" => Ok(AntialiasSetting::Samples2),
        "4" => Ok(AntialiasSetting::Samples4),
        "8" => Ok(AntialiasSetting::Samples8),
        "16" => Ok(AntialiasSetting::Samples16),
        other => Err(format!("invalid antialias setting '{other}'")),
    }
}

impl MultiConfig {
    pub fn from_toml_str(input: &str) -> Result<Self, ConfigError> {
        let raw: MultiConfig = toml::from_str(input)?;
        raw.validate()?;
        Ok(raw)
    }

    pub fn playlist(&self, name: &str) -> Option<&Playlist> {
        self.playlists.get(name)
    }

    pub fn default_playlist(&self) -> Option<&str> {
        self.defaults.playlist.as_deref()
    }

    pub fn workspace_switch_crossfade(&self) -> Duration {
        self.workspace_switch_crossfade
            .unwrap_or_else(|| Duration::from_secs_f32(1.0))
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.version != 1 {
            return Err(ConfigError::Invalid(format!(
                "unsupported config version {}; expected 1",
                self.version
            )));
        }

        if self.playlists.is_empty() {
            return Err(ConfigError::Invalid(
                "config must define at least one playlist".into(),
            ));
        }

        for (name, playlist) in &self.playlists {
            if playlist.items.is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "playlist '{name}' must contain at least one item"
                )));
            }

            if playlist.item_duration.is_zero() {
                return Err(ConfigError::Invalid(format!(
                    "playlist '{name}' item_duration must be greater than zero"
                )));
            }

            if playlist.crossfade.as_secs_f32() < 0.0 {
                return Err(ConfigError::Invalid(format!(
                    "playlist '{name}' crossfade must be non-negative"
                )));
            }

            if let Some(fps) = playlist.fps {
                if fps < 0.0 {
                    return Err(ConfigError::Invalid(format!(
                        "playlist '{name}' fps must be >= 0"
                    )));
                }
            }

            for item in &playlist.items {
                if item.handle.trim().is_empty() {
                    return Err(ConfigError::Invalid(format!(
                        "playlist '{name}' contains an item with empty handle"
                    )));
                }

                if let Some(duration) = item.duration {
                    if duration.is_zero() {
                        return Err(ConfigError::Invalid(format!(
                            "playlist '{name}' item '{}' duration must be > 0",
                            item.handle
                        )));
                    }
                }

                if let Some(fps) = item.fps {
                    if fps < 0.0 {
                        return Err(ConfigError::Invalid(format!(
                            "playlist '{name}' item '{}' fps must be >= 0",
                            item.handle
                        )));
                    }
                }
            }
        }

        if let Some(default_playlist) = &self.defaults.playlist {
            if !self.playlists.contains_key(default_playlist) {
                return Err(ConfigError::Invalid(format!(
                    "defaults.playlist references unknown playlist '{default_playlist}'"
                )));
            }
        }

        for (selector, playlist) in &self.targets {
            if selector.trim().is_empty() {
                return Err(ConfigError::Invalid(
                    "target selector may not be empty".into(),
                ));
            }

            validate_target_selector(selector)?;

            if !self.playlists.contains_key(playlist) {
                return Err(ConfigError::Invalid(format!(
                    "target '{selector}' references unknown playlist '{playlist}'"
                )));
            }
        }

        if let Some(fps) = self.defaults.fps {
            if fps < 0.0 {
                return Err(ConfigError::Invalid("defaults.fps must be >= 0".into()));
            }
        }

        Ok(())
    }
}

fn validate_target_selector(selector: &str) -> Result<(), ConfigError> {
    if selector == "_default" {
        return Ok(());
    }

    if let Some(rest) = selector.strip_prefix("workspace:") {
        if rest.is_empty() {
            return Err(ConfigError::Invalid(
                "workspace selector must include a name or id".into(),
            ));
        }
        return Ok(());
    }

    if let Some(rest) = selector.strip_prefix("output:") {
        if rest.is_empty() {
            return Err(ConfigError::Invalid(
                "output selector must include an output name".into(),
            ));
        }
        return Ok(());
    }

    if selector.chars().all(|ch| ch.is_ascii_digit()) {
        return Ok(());
    }

    Err(ConfigError::Invalid(format!(
        "target selector '{selector}' is invalid; expected 'workspace:<name-or-id>', 'output:<name>', '_default', or a numeric workspace id"
    )))
}

impl Playlist {
    pub fn resolved_item(&self, index: usize, defaults: &Defaults) -> Option<ResolvedItem> {
        let item = self.items.get(index)?;
        let duration = item.duration.unwrap_or(self.item_duration);
        let fps = item.fps.or(self.fps).or(defaults.fps);
        let antialias = item.antialias.or(self.antialias).or(defaults.antialias);

        Some(ResolvedItem {
            handle: item.handle.clone(),
            duration,
            fps,
            antialias,
            refresh_once: item.refresh_once,
            crossfade: self.crossfade,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
version = 1
workspace_switch_crossfade = "1.5s"

[defaults]
playlist = "ambient"
fps = 0
antialias = "auto"

[playlists.ambient]
mode = "continuous"
item_duration = "300s"
crossfade = "1s"

[[playlists.ambient.items]]
handle = "shader://grayday"

[[playlists.ambient.items]]
handle = "shadertoy://3dXyWj"
duration = "120s"
fps = 60
antialias = 4
refresh_once = true

[playlists.focus]
mode = "shuffle"
item_duration = "90s"

[[playlists.focus.items]]
handle = "shader://simplex"

[targets]
"workspace:2" = "focus"
"_default" = "ambient"
"#;

    #[test]
    fn parses_sample_config() {
        let config = MultiConfig::from_toml_str(SAMPLE).expect("parse config");
        assert_eq!(config.version, 1);
        assert!(config.playlists.contains_key("ambient"));
        assert_eq!(config.defaults.playlist.as_deref(), Some("ambient"));
        assert_eq!(
            config.targets.get("workspace:2").map(|s| s.as_str()),
            Some("focus")
        );
    }

    #[test]
    fn rejects_unknown_playlist() {
        let config = r#"
version = 1

[playlists.main]
mode = "continuous"
item_duration = 60

[[playlists.main.items]]
handle = "shader://demo"

[defaults]
playlist = "missing"
"#;
        let err = MultiConfig::from_toml_str(config).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
    }

    #[test]
    fn rejects_invalid_selector() {
        let config = r#"
version = 1

[playlists.main]
mode = "continuous"
item_duration = 60

[[playlists.main.items]]
handle = "shader://demo"

[targets]
"badselector" = "main"
"#;
        let err = MultiConfig::from_toml_str(config).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
    }

    #[test]
    fn resolves_item_with_defaults() {
        let config = MultiConfig::from_toml_str(
            r#"
version = 1

[defaults]
fps = 24
antialias = "4"

[playlists.main]
mode = "continuous"
item_duration = "5s"

[[playlists.main.items]]
handle = "local/demo"
"#,
        )
        .unwrap();

        let playlist = config.playlist("main").unwrap();
        let resolved = playlist.resolved_item(0, &config.defaults).unwrap();
        assert_eq!(resolved.handle, "local/demo");
        assert_eq!(resolved.duration, Duration::from_secs(5));
        assert_eq!(resolved.fps, Some(24.0));
        assert_eq!(resolved.antialias, Some(AntialiasSetting::Samples4));
        assert_eq!(resolved.crossfade, Duration::from_secs_f32(1.0));
        assert!(!resolved.refresh_once);
    }
}
