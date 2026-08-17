use crate::model::{AgentEvent, Device, new_id};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::PathBuf,
};

const MAX_EVENTS: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub device_id: String,
    pub device_name: String,
    pub emitter_token: String,
    pub port: u16,
    #[serde(default)]
    pub devices: Vec<Device>,
    #[serde(default)]
    pub ntfy_topic: String,
}

pub struct Store {
    pub dir: PathBuf,
    pub config: Config,
    pub events: VecDeque<AgentEvent>,
}

impl Store {
    pub fn data_dir() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os("AGENTBELL_DATA_DIR") {
            return Ok(PathBuf::from(path));
        }
        Ok(
            directories::ProjectDirs::from("com", "AgentBell", "AgentBell")
                .context("无法确定 AgentBell 数据目录")?
                .data_local_dir()
                .to_path_buf(),
        )
    }

    pub fn load(port: u16) -> Result<Self> {
        let dir = Self::data_dir()?;
        fs::create_dir_all(&dir)?;
        let path = dir.join("config.json");
        let mut config = if path.exists() {
            let mut c: Config = serde_json::from_slice(&fs::read(&path)?)?;
            if port != 0 {
                c.port = port;
            }
            c
        } else {
            Config {
                device_id: new_id(),
                device_name: hostname::get()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                emitter_token: format!("ab_{}", new_id().replace('-', "")),
                port: if port == 0 { 43821 } else { port },
                devices: vec![],
                ntfy_topic: String::new(),
            }
        };
        let mut unique = HashMap::<String, Device>::new();
        for device in config.devices.drain(..) {
            let key = if device.client_id.is_empty() {
                format!("{}|{}", device.name, device.last_ip)
            } else {
                format!("id|{}", device.client_id)
            };
            match unique.get(&key) {
                Some(existing) if existing.last_seen_ms >= device.last_seen_ms => {}
                _ => {
                    unique.insert(key, device);
                }
            }
        }
        config.devices = unique.into_values().collect();
        let mut events = VecDeque::new();
        let ep = dir.join("events.jsonl");
        if let Ok(text) = fs::read_to_string(ep) {
            for line in text
                .lines()
                .rev()
                .take(MAX_EVENTS)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
            {
                if let Ok(ev) = serde_json::from_str(line) {
                    events.push_back(ev);
                }
            }
        }
        let mut store = Self {
            dir,
            config,
            events,
        };
        store.save_config()?;
        Ok(store)
    }

    pub fn save_config(&mut self) -> Result<()> {
        let path = self.dir.join("config.json");
        let tmp = self.dir.join("config.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&self.config)?)?;
        if path.exists() {
            let _ = fs::remove_file(&path);
        }
        fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn add_event(&mut self, event: AgentEvent) -> Result<()> {
        self.events.push_back(event);
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
        self.save_events()?;
        Ok(())
    }

    pub fn delete_events(&mut self, ids: &[String]) -> Result<usize> {
        let ids = ids.iter().map(String::as_str).collect::<HashSet<_>>();
        let before = self.events.len();
        self.events.retain(|event| !ids.contains(event.id.as_str()));
        let removed = before - self.events.len();
        if removed > 0 {
            self.save_events()?;
        }
        Ok(removed)
    }

    fn save_events(&self) -> Result<()> {
        let path = self.dir.join("events.jsonl");
        let tmp = self.dir.join("events.jsonl.tmp");
        let mut text = self
            .events
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()?
            .join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        fs::write(&tmp, text)?;
        if path.exists() {
            fs::remove_file(&path)?;
        }
        fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn is_token_trusted(&self, token: &str) -> bool {
        self.config
            .devices
            .iter()
            .any(|d| d.trusted && d.token == token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EventKind, now_ms};

    fn event(id: usize) -> AgentEvent {
        AgentEvent {
            id: format!("event-{id}"),
            agent: "test".into(),
            kind: EventKind::Completed,
            conversation_id: "conversation".into(),
            title: format!("Task {id}"),
            project: "AgentBell".into(),
            message: "done".into(),
            duration_ms: None,
            timestamp_ms: now_ms(),
        }
    }

    fn store(dir: PathBuf) -> Store {
        Store {
            dir,
            config: Config {
                device_id: "pc".into(),
                device_name: "test".into(),
                emitter_token: "token".into(),
                port: 43821,
                devices: vec![],
                ntfy_topic: String::new(),
            },
            events: VecDeque::new(),
        }
    }

    #[test]
    fn event_history_is_bounded_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path().to_path_buf());
        for id in 0..105 {
            store.add_event(event(id)).unwrap();
        }

        assert_eq!(store.events.len(), MAX_EVENTS);
        let lines = fs::read_to_string(dir.path().join("events.jsonl")).unwrap();
        assert_eq!(lines.lines().count(), MAX_EVENTS);
        assert!(!lines.contains("\"id\":\"event-0\""));
        assert!(lines.contains("\"id\":\"event-104\""));
    }

    #[test]
    fn delete_events_updates_memory_and_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path().to_path_buf());
        for id in 0..3 {
            store.add_event(event(id)).unwrap();
        }

        let removed = store
            .delete_events(&["event-0".into(), "event-2".into()])
            .unwrap();

        assert_eq!(removed, 2);
        assert_eq!(store.events.len(), 1);
        let lines = fs::read_to_string(dir.path().join("events.jsonl")).unwrap();
        assert!(!lines.contains("event-0"));
        assert!(lines.contains("event-1"));
        assert!(!lines.contains("event-2"));
    }
}
