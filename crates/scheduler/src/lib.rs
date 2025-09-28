use std::collections::HashMap;
use std::time::{Duration, Instant};

use multiconfig::{AntialiasSetting, MultiConfig, PlaylistMode};
use rand::prelude::*;

#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("playlist '{0}' not found")]
    UnknownPlaylist(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetId(pub String);

impl TargetId {
    pub fn new(id: impl Into<String>) -> Self {
        TargetId(id.into())
    }
}

#[derive(Debug, Clone)]
pub struct ScheduledItem {
    pub handle: String,
    pub duration: Duration,
    pub fps: Option<f32>,
    pub antialias: Option<AntialiasSetting>,
    pub refresh_once: bool,
    pub crossfade: Duration,
}

#[derive(Debug, Clone)]
pub struct SelectionChange {
    pub target: TargetId,
    pub item: ScheduledItem,
    pub started_at: Instant,
}

pub struct Scheduler {
    playlists: HashMap<String, PlaylistRuntime>,
    targets: HashMap<TargetId, TargetState>,
    rng: StdRng,
}

fn normalize_fps(value: Option<f32>) -> Option<f32> {
    value.and_then(|fps| if fps > 0.0 { Some(fps) } else { None })
}

impl Scheduler {
    pub fn new(config: &MultiConfig, seed: u64) -> Self {
        let playlists = config
            .playlists
            .iter()
            .map(|(name, playlist)| {
                (
                    name.clone(),
                    PlaylistRuntime::from_config(playlist, &config.defaults),
                )
            })
            .collect();
        Self {
            playlists,
            targets: HashMap::new(),
            rng: StdRng::seed_from_u64(seed),
        }
    }

    pub fn set_target(
        &mut self,
        target: TargetId,
        playlist: &str,
        now: Instant,
    ) -> Result<SelectionChange, SchedulerError> {
        let runtime = self
            .playlists
            .get(playlist)
            .ok_or_else(|| SchedulerError::UnknownPlaylist(playlist.to_string()))?
            .clone();

        let state = TargetState::new(runtime, now, &mut self.rng);
        let item = state.current_scheduled_item();
        self.targets.insert(target.clone(), state);
        Ok(SelectionChange {
            target,
            item,
            started_at: now,
        })
    }

    pub fn remove_target(&mut self, target: &TargetId) {
        self.targets.remove(target);
    }

    pub fn skip_target(&mut self, target: &TargetId, now: Instant) -> Option<SelectionChange> {
        let state = self.targets.get_mut(target)?;
        if state.playlist.items.len() <= 1 {
            return None;
        }
        state.advance_to_next(now, &mut self.rng);
        let item = state.current_scheduled_item();
        Some(SelectionChange {
            target: target.clone(),
            item,
            started_at: now,
        })
    }

    pub fn tick(&mut self, now: Instant) -> Vec<SelectionChange> {
        let mut changes = Vec::new();
        for (target, state) in self.targets.iter_mut() {
            if state.advance_if_elapsed(now, &mut self.rng) {
                let item = state.current_scheduled_item();
                changes.push(SelectionChange {
                    target: target.clone(),
                    item,
                    started_at: now,
                });
            }
        }
        changes
    }
}

#[derive(Clone)]
struct PlaylistRuntime {
    mode: PlaylistMode,
    crossfade: Duration,
    items: Vec<RuntimeItem>,
}

impl PlaylistRuntime {
    fn from_config(src: &multiconfig::Playlist, defaults: &multiconfig::Defaults) -> Self {
        let items = src
            .items
            .iter()
            .map(|item| RuntimeItem {
                handle: item.handle.clone(),
                duration: item.duration.unwrap_or(src.item_duration),
                fps: normalize_fps(item.fps)
                    .or_else(|| normalize_fps(src.fps))
                    .or_else(|| normalize_fps(defaults.fps)),
                antialias: item.antialias.or(src.antialias).or(defaults.antialias),
                refresh_once: item.refresh_once,
            })
            .collect();
        Self {
            mode: src.mode.clone(),
            crossfade: src.crossfade,
            items,
        }
    }
}

#[derive(Clone)]
struct RuntimeItem {
    handle: String,
    duration: Duration,
    fps: Option<f32>,
    antialias: Option<AntialiasSetting>,
    refresh_once: bool,
}

struct TargetState {
    playlist: PlaylistRuntime,
    order: Vec<usize>,
    cursor: usize,
    last_started: Instant,
}

impl TargetState {
    fn new(playlist: PlaylistRuntime, now: Instant, rng: &mut StdRng) -> Self {
        let order = build_order(playlist.items.len(), &playlist.mode, rng);
        Self {
            playlist,
            order,
            cursor: 0,
            last_started: now,
        }
    }

    fn current_index(&self) -> usize {
        self.order[self.cursor]
    }

    fn advance_if_elapsed(&mut self, now: Instant, rng: &mut StdRng) -> bool {
        if self.playlist.items.len() <= 1 {
            return false;
        }
        let idx = self.current_index();
        let item = &self.playlist.items[idx];
        if now.duration_since(self.last_started) >= item.duration {
            self.advance_to_next(now, rng);
            true
        } else {
            false
        }
    }

    fn advance_to_next(&mut self, now: Instant, rng: &mut StdRng) {
        if self.playlist.items.len() <= 1 {
            self.last_started = now;
            return;
        }
        self.cursor += 1;
        if self.cursor >= self.order.len() {
            self.order = build_order(self.playlist.items.len(), &self.playlist.mode, rng);
            self.cursor = 0;
        }
        self.last_started = now;
    }

    fn current_scheduled_item(&self) -> ScheduledItem {
        let idx = self.current_index();
        let item = &self.playlist.items[idx];
        ScheduledItem {
            handle: item.handle.clone(),
            duration: item.duration,
            fps: item.fps,
            antialias: item.antialias,
            refresh_once: item.refresh_once,
            crossfade: self.playlist.crossfade,
        }
    }
}

fn build_order(len: usize, mode: &PlaylistMode, rng: &mut StdRng) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).collect();
    match mode {
        PlaylistMode::Continuous => {}
        PlaylistMode::Shuffle => {
            order.shuffle(rng);
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use multiconfig::MultiConfig;

    const CONFIG: &str = r#"
version = 1

[playlists.test]
mode = "continuous"
item_duration = 1

[[playlists.test.items]]
handle = "one"
[[playlists.test.items]]
handle = "two"

"#;

    #[test]
    fn advances_continuous_playlist() {
        let config = MultiConfig::from_toml_str(CONFIG).unwrap();
        let mut scheduler = Scheduler::new(&config, 1);
        let target = TargetId::new("output:A");
        let mut now = Instant::now();
        let first = scheduler.set_target(target.clone(), "test", now).unwrap();
        assert_eq!(first.item.handle, "one");
        now = now + Duration::from_secs(1);
        let changes = scheduler.tick(now);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].item.handle, "two");
    }

    #[test]
    fn shuffle_generates_order() {
        let config = MultiConfig::from_toml_str(
            r#"
version = 1

[playlists.test]
mode = "shuffle"
item_duration = 1

[[playlists.test.items]]
handle = "one"
[[playlists.test.items]]
handle = "two"
[[playlists.test.items]]
handle = "three"
"#,
        )
        .unwrap();
        let mut scheduler = Scheduler::new(&config, 42);
        let target = TargetId::new("output:A");
        let first = scheduler
            .set_target(target.clone(), "test", Instant::now())
            .unwrap();
        assert!(matches!(
            first.item.handle.as_str(),
            "one" | "two" | "three"
        ));
    }

    #[test]
    fn applies_global_defaults() {
        let config = MultiConfig::from_toml_str(
            r#"
version = 1

[defaults]
fps = 48
antialias = "8"

[playlists.test]
mode = "continuous"
item_duration = 1

[[playlists.test.items]]
handle = "local/demo"
"#,
        )
        .unwrap();

        let mut scheduler = Scheduler::new(&config, 7);
        let target = TargetId::new("output:A");
        let change = scheduler
            .set_target(target.clone(), "test", Instant::now())
            .unwrap();
        assert_eq!(change.item.fps, Some(48.0));
        assert_eq!(change.item.antialias, Some(AntialiasSetting::Samples8));
        assert_eq!(change.item.crossfade, Duration::from_secs_f32(1.0));
    }

    #[test]
    fn zero_fps_treated_as_uncapped() {
        let config = MultiConfig::from_toml_str(
            r#"
version = 1

[playlists.test]
mode = "continuous"
item_duration = 1
fps = 0

[[playlists.test.items]]
handle = "local/demo"
fps = 0
"#,
        )
        .unwrap();

        let mut scheduler = Scheduler::new(&config, 3);
        let target = TargetId::new("surface:1");
        let change = scheduler
            .set_target(target.clone(), "test", Instant::now())
            .unwrap();
        assert_eq!(change.item.fps, None, "fps=0 should map to uncapped");
    }

    #[test]
    fn skip_advances_playlist() {
        let config = MultiConfig::from_toml_str(
            r#"
version = 1

[playlists.test]
mode = "continuous"
item_duration = 1

[[playlists.test.items]]
handle = "one"

[[playlists.test.items]]
handle = "two"
"#,
        )
        .unwrap();

        let mut scheduler = Scheduler::new(&config, 11);
        let target = TargetId::new("output:A");
        let first = scheduler
            .set_target(target.clone(), "test", Instant::now())
            .unwrap();
        assert_eq!(first.item.handle, "one");
        let skipped = scheduler
            .skip_target(&target, Instant::now())
            .expect("skip result");
        assert_eq!(skipped.item.handle, "two");
    }
}
