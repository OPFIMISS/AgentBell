use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Started,
    Completed,
    Failed,
    NeedsInput,
    ApprovalRequired,
}

impl Default for EventKind {
    fn default() -> Self {
        Self::Completed
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentEvent {
    #[serde(default = "new_id")]
    pub id: String,
    pub agent: String,
    #[serde(default)]
    pub kind: EventKind,
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default = "now_ms")]
    pub timestamp_ms: i64,
}

impl AgentEvent {
    pub fn normalize(mut self) -> Self {
        self.agent = clean(&self.agent, 40);
        self.title = clean(&self.title, 160);
        self.project = clean(&self.project, 100);
        self.message = clean(&self.message, 320);
        self.conversation_id = clean(&self.conversation_id, 160);
        if self.title.is_empty() {
            self.title = "任务状态更新".into();
        }
        if self.message.is_empty() {
            self.message = match self.kind {
                EventKind::Started => "任务已开始",
                EventKind::Completed => "任务已完成",
                EventKind::Failed => "任务执行失败",
                EventKind::NeedsInput => "正在等待你的回复",
                EventKind::ApprovalRequired => "正在等待你的批准",
            }
            .into();
        }
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    #[serde(default)]
    pub client_id: String,
    pub name: String,
    pub token: String,
    pub trusted: bool,
    pub created_ms: i64,
    pub last_seen_ms: i64,
    #[serde(default)]
    pub last_ip: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingPair {
    pub id: String,
    #[serde(default)]
    pub client_id: String,
    pub name: String,
    pub code: String,
    pub request_token: String,
    pub ip: String,
    pub expires_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveredPeer {
    pub device_id: String,
    pub role: String,
    pub name: String,
    pub model: String,
    pub ip: String,
    pub port: u16,
    pub last_seen_ms: i64,
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn clean(value: &str, max: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}
