use crate::model::{AgentEvent, Device, new_id};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::PathBuf,
};

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
                .take(100)
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
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("events.jsonl"))?;
        writeln!(f, "{}", serde_json::to_string(&event)?)?;
        self.events.push_back(event);
        while self.events.len() > 100 {
            self.events.pop_front();
        }
        Ok(())
    }

    pub fn is_token_trusted(&self, token: &str) -> bool {
        self.config
            .devices
            .iter()
            .any(|d| d.trusted && d.token == token)
    }
}
