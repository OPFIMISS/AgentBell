use crate::{
    model::{AgentEvent, EventKind, new_id, now_ms},
    server::AppState,
};
use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

pub fn install_codex(exe: &Path) -> Result<PathBuf> {
    let base = directories::BaseDirs::new().context("无法确定用户目录")?;
    let dir = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| base.home_dir().join(".codex"));
    fs::create_dir_all(&dir)?;
    let path = dir.join("config.toml");
    let text = fs::read_to_string(&path).unwrap_or_default();
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .context("Codex config.toml 语法无效，AgentBell 未修改该文件")?;
    let args = [
        exe.to_string_lossy().to_string(),
        "emit".into(),
        "--agent".into(),
        "codex".into(),
    ];
    let mut array = toml_edit::Array::new();
    for arg in args {
        array.push(arg);
    }
    doc["notify"] = toml_edit::value(array);
    let tmp = path.with_extension("toml.agentbell.tmp");
    fs::write(&tmp, doc.to_string())?;
    if path.exists() {
        let backup = path.with_extension("toml.agentbell.bak");
        let _ = fs::copy(&path, backup);
        fs::remove_file(&path)?;
    }
    fs::rename(tmp, &path)?;
    Ok(path)
}

pub async fn post_local(event: AgentEvent, port: u16, token: &str) -> Result<()> {
    let url = format!("http://127.0.0.1:{port}/api/events");
    reqwest::Client::new()
        .post(url)
        .bearer_auth(token)
        .json(&event)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

pub fn event_from_payload(
    agent: &str,
    payload: Option<&str>,
    title: Option<String>,
    project: Option<String>,
    message: Option<String>,
    conversation_id: Option<String>,
    kind: EventKind,
) -> AgentEvent {
    let parsed = payload.and_then(|p| serde_json::from_str::<Value>(p).ok());
    let pick = |keys: &[&str]| -> String {
        parsed
            .as_ref()
            .and_then(|v| keys.iter().find_map(|k| v.get(*k)?.as_str()))
            .unwrap_or_default()
            .to_string()
    };
    let cwd = pick(&["cwd", "workdir", "project_path"]);
    let project_name = project.unwrap_or_else(|| {
        Path::new(&cwd)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    });
    let inferred_title = {
        let input = parsed
            .as_ref()
            .and_then(|v| v.get("input-messages").or_else(|| v.get("input_messages")))
            .and_then(|v| v.as_array())
            .and_then(|a| a.last())
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if input.is_empty() {
            "任务已完成".to_string()
        } else {
            input.chars().take(80).collect()
        }
    };
    AgentEvent {
        id: new_id(),
        agent: agent.to_string(),
        kind,
        conversation_id: conversation_id
            .unwrap_or_else(|| pick(&["thread-id", "thread_id", "session_id", "turn-id"])),
        title: title.unwrap_or(inferred_title),
        project: project_name,
        message: message.unwrap_or_else(|| {
            let value = pick(&[
                "last-assistant-message",
                "last_assistant_message",
                "message",
            ]);
            if value.is_empty() {
                "任务已完成".into()
            } else {
                value.chars().take(260).collect()
            }
        }),
        duration_ms: None,
        timestamp_ms: now_ms(),
    }
}

pub fn start_watchers(state: Arc<AppState>) {
    tokio::spawn(watch_codex(state.clone()));
    tokio::spawn(watch_deepseek(state.clone()));
    tokio::spawn(watch_haha(state));
}

async fn publish(state: &Arc<AppState>, event: AgentEvent) {
    let event = event.normalize();
    if let Err(err) = state.store.lock().await.add_event(event.clone()) {
        warn!(%err, "适配器事件保存失败");
        return;
    }
    let receivers = state.tx.send(event.clone()).unwrap_or(0);
    info!(agent = %event.agent, conversation = %event.conversation_id, receivers, "适配器识别到任务完成并发布事件");
}

#[derive(Clone, Debug)]
struct CodexCompletion {
    turn_id: String,
    session_id: String,
    title: String,
    project: String,
    message: String,
}

#[derive(Debug)]
struct CodexScan {
    path: PathBuf,
    completions: Vec<CodexCompletion>,
}

async fn watch_codex(state: Arc<AppState>) {
    let Some(base) = directories::BaseDirs::new() else {
        return;
    };
    let root = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| base.home_dir().join(".codex"))
        .join("sessions");
    info!(path = %root.display(), "Codex Desktop 会话监听已启动");
    let seen = Mutex::new(HashMap::<PathBuf, HashSet<String>>::new());
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        tick.tick().await;
        let scan_root = root.clone();
        let rows = tokio::task::spawn_blocking(move || scan_codex(&scan_root))
            .await
            .unwrap_or_default();
        let mut map = seen.lock().await;
        for row in rows {
            let current: HashSet<String> =
                row.completions.iter().map(|c| c.turn_id.clone()).collect();
            let Some(previous) = map.get_mut(&row.path) else {
                debug!(path = %row.path.display(), completions = current.len(), "Codex 会话建立基线");
                map.insert(row.path, current);
                continue;
            };
            for completion in row
                .completions
                .iter()
                .filter(|c| !previous.contains(&c.turn_id))
            {
                publish(
                    &state,
                    AgentEvent {
                        id: new_id(),
                        agent: "Codex".into(),
                        kind: EventKind::Completed,
                        conversation_id: if completion.session_id.is_empty() {
                            completion.turn_id.clone()
                        } else {
                            completion.session_id.clone()
                        },
                        title: completion.title.clone(),
                        project: completion.project.clone(),
                        message: completion.message.clone(),
                        duration_ms: None,
                        timestamp_ms: now_ms(),
                    },
                )
                .await;
            }
            *previous = current;
        }
    }
}

fn scan_codex(root: &Path) -> Vec<CodexScan> {
    if !root.exists() {
        return vec![];
    }
    WalkDir::new(root)
        .max_depth(5)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.file_name().to_string_lossy().starts_with("rollout-")
                && e.path().extension().and_then(|x| x.to_str()) == Some("jsonl")
        })
        .filter_map(|entry| scan_codex_file(entry.path()))
        .collect()
}

fn scan_codex_file(path: &Path) -> Option<CodexScan> {
    let text = fs::read_to_string(path).ok()?;
    let mut session_id = String::new();
    let mut project = String::new();
    let mut root_session = false;
    let mut last_user = String::new();
    let mut last_agent = String::new();
    let mut completions = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) == Some("session_meta") {
            let payload = v.get("payload").unwrap_or(&Value::Null);
            let is_subagent = !payload
                .get("parent_thread_id")
                .unwrap_or(&Value::Null)
                .is_null()
                || payload.pointer("/source/subagent").is_some();
            if is_subagent {
                debug!(path = %path.display(), "忽略 Codex 子代理会话");
                return None;
            }
            root_session = true;
            session_id = payload
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            project = payload
                .get("cwd")
                .and_then(Value::as_str)
                .and_then(|p| Path::new(p).file_name())
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            continue;
        }
        let Some(payload) = v.get("payload") else {
            continue;
        };
        match payload.get("type").and_then(Value::as_str) {
            Some("user_message") => {
                last_user = payload
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .chars()
                    .take(100)
                    .collect()
            }
            Some("agent_message") => {
                last_agent = payload
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .chars()
                    .take(260)
                    .collect()
            }
            Some("task_complete") => {
                let turn_id = payload
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if !turn_id.is_empty() {
                    completions.push(CodexCompletion {
                        turn_id,
                        session_id: session_id.clone(),
                        title: if last_user.is_empty() {
                            "Codex 任务完成".into()
                        } else {
                            last_user.clone()
                        },
                        project: project.clone(),
                        message: if last_agent.is_empty() {
                            "本轮任务已完成".into()
                        } else {
                            last_agent.clone()
                        },
                    });
                }
            }
            _ => {}
        }
    }
    root_session.then(|| CodexScan {
        path: path.to_path_buf(),
        completions,
    })
}

async fn watch_deepseek(state: Arc<AppState>) {
    let Some(base) = directories::BaseDirs::new() else {
        return;
    };
    let root = std::env::var_os("DSH_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| base.home_dir().join(".dsh"));
    let seen = Mutex::new(HashMap::<PathBuf, usize>::new());
    let mut baseline_logged = false;
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        tick.tick().await;
        let scan_root = root.clone();
        let rows = tokio::task::spawn_blocking(move || scan_dsh(&scan_root))
            .await
            .unwrap_or_default();
        if !baseline_logged {
            info!(root = %root.display(), sessions = rows.len(), completions = rows.iter().map(|row| row.1).sum::<usize>(), "Deepseek Harness EAC 默认监听已建立基线");
            baseline_logged = true;
        }
        let mut map = seen.lock().await;
        for (path, count, title, session, project) in rows {
            let old = map.insert(path, count).unwrap_or(count);
            if count > old {
                publish(
                    &state,
                    AgentEvent {
                        id: new_id(),
                        agent: "Deepseek Harness EAC".into(),
                        kind: EventKind::Completed,
                        conversation_id: session,
                        title,
                        project,
                        message: "本轮任务已完成".into(),
                        duration_ms: None,
                        timestamp_ms: now_ms(),
                    },
                )
                .await;
            }
        }
    }
}

fn scan_dsh(root: &Path) -> Vec<(PathBuf, usize, String, String, String)> {
    let sessions = root.join("sessions");
    if !sessions.exists() {
        return vec![];
    }
    WalkDir::new(sessions)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_name() == "session.jsonl.zstd")
        .filter_map(|entry| {
            let bytes = fs::read(entry.path()).ok()?;
            let text = zstd::stream::decode_all(bytes.as_slice()).ok()?;
            let text = String::from_utf8_lossy(&text);
            let mut count = 0;
            let mut assistant_count = 0;
            let mut title = String::new();
            let mut session = String::new();
            let mut project = String::new();
            let mut delegated = false;
            for line in text.lines() {
                let Ok(v) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                let values: Vec<&Value> = v
                    .as_array()
                    .map(|items| items.iter().collect())
                    .unwrap_or_else(|| vec![&v]);
                for value in values {
                    match value.get("type").and_then(Value::as_str) {
                        Some("turn/end") => count += 1,
                        Some("assistant") => assistant_count += 1,
                        Some("session/title") => {
                            title = value
                                .pointer("/data/title")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string()
                        }
                        Some("session") => {
                            session = value
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            project = value
                                .get("cwd")
                                .and_then(Value::as_str)
                                .and_then(|p| Path::new(p).file_name())
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_default();
                            delegated = value
                                .get("delegationDepth")
                                .or_else(|| value.pointer("/data/delegationDepth"))
                                .and_then(Value::as_u64)
                                .unwrap_or(0)
                                > 0;
                        }
                        _ => {}
                    }
                }
            }
            if delegated {
                return None;
            }
            if count == 0 {
                count = assistant_count;
            }
            if title.is_empty() {
                title = "DSH 任务完成".into();
            }
            Some((entry.path().to_path_buf(), count, title, session, project))
        })
        .collect()
}

async fn watch_haha(state: Arc<AppState>) {
    let Some(base) = directories::BaseDirs::new() else {
        return;
    };
    let claude_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| base.home_dir().join(".claude"));
    let roots = [claude_dir.join("projects")];
    let state_file = claude_dir.join("desktop-server-state.json");
    let seen = Mutex::new(HashMap::<PathBuf, usize>::new());
    let mut baseline_logged = false;
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        tick.tick().await;
        if !haha_server_running(&state_file).await {
            continue;
        }
        let scan_roots = roots.clone();
        let rows = tokio::task::spawn_blocking(move || scan_haha(&scan_roots))
            .await
            .unwrap_or_default();
        if !baseline_logged {
            info!(root = %roots[0].display(), sessions = rows.len(), completions = rows.iter().map(|row| row.1).sum::<usize>(), "Claude Code Haha 默认监听已建立基线");
            baseline_logged = true;
        }
        let mut map = seen.lock().await;
        for (path, count, session, title, project, failed) in rows {
            let old = map.insert(path, count).unwrap_or(count);
            if count > old {
                publish(
                    &state,
                    AgentEvent {
                        id: new_id(),
                        agent: "Claude Code Haha".into(),
                        kind: if failed {
                            EventKind::Failed
                        } else {
                            EventKind::Completed
                        },
                        conversation_id: session,
                        title,
                        project,
                        message: if failed {
                            "本轮任务执行失败".into()
                        } else {
                            "本轮任务已完成".into()
                        },
                        duration_ms: None,
                        timestamp_ms: now_ms(),
                    },
                )
                .await;
            }
        }
    }
}

async fn haha_server_running(state_file: &Path) -> bool {
    let Ok(text) = fs::read_to_string(state_file) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    let Some(port) = value
        .get("lastPort")
        .and_then(Value::as_u64)
        .filter(|port| *port > 0 && *port <= 65535)
    else {
        return false;
    };
    tokio::time::timeout(
        Duration::from_millis(350),
        tokio::net::TcpStream::connect(("127.0.0.1", port as u16)),
    )
    .await
    .map(|result| result.is_ok())
    .unwrap_or(false)
}

fn scan_haha(roots: &[PathBuf]) -> Vec<(PathBuf, usize, String, String, String, bool)> {
    let mut out = vec![];
    for root in roots {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root)
            .max_depth(8)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.path().extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(text) = fs::read_to_string(entry.path()) else {
                continue;
            };
            let mut count = 0;
            let mut session = String::new();
            let mut title = String::new();
            let mut project = String::new();
            let mut failed = false;
            for line in text.lines() {
                let Ok(v) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                if session.is_empty() {
                    session = v
                        .get("sessionId")
                        .or_else(|| v.get("session_id"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                }
                if v.get("type").and_then(Value::as_str) == Some("user") {
                    title = extract_text(&v).chars().take(80).collect();
                }
                if v.get("type").and_then(Value::as_str) == Some("session-meta") {
                    project = v
                        .get("workDir")
                        .and_then(Value::as_str)
                        .and_then(|p| Path::new(p).file_name())
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                }
                if v.get("type").and_then(Value::as_str) == Some("result") {
                    count += 1;
                    failed = v.get("is_error").and_then(Value::as_bool).unwrap_or(false)
                        || v.get("subtype")
                            .and_then(Value::as_str)
                            .map(|s| s.contains("error"))
                            .unwrap_or(false);
                }
                if v.get("type").and_then(Value::as_str) == Some("assistant")
                    && !v
                        .get("isSidechain")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    && v.pointer("/message/stop_reason").and_then(Value::as_str) == Some("end_turn")
                {
                    count += 1;
                }
            }
            if count == 0 {
                continue;
            }
            if title.is_empty() {
                title = "Haha 任务完成".into();
            }
            if project.is_empty() {
                project = entry
                    .path()
                    .parent()
                    .and_then(Path::file_name)
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
            }
            out.push((
                entry.path().to_path_buf(),
                count,
                session,
                title,
                project,
                failed,
            ));
        }
    }
    debug!(count = out.len(), "Claude Code Haha 会话扫描完成");
    out
}

fn extract_text(v: &Value) -> String {
    if let Some(s) = v.pointer("/message/content").and_then(Value::as_str) {
        return s.to_string();
    }
    if let Some(items) = v.pointer("/message/content").and_then(Value::as_array) {
        return items
            .iter()
            .filter_map(|x| x.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" ");
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_codex_notify_payload() {
        let payload = serde_json::json!({
            "thread-id": "thread-42",
            "cwd": "E:\\work\\sample-app",
            "input-messages": ["修复登录问题"],
            "last-assistant-message": "已经修复并通过测试"
        })
        .to_string();
        let event = event_from_payload(
            "codex",
            Some(&payload),
            None,
            None,
            None,
            None,
            EventKind::Completed,
        );
        assert_eq!(event.conversation_id, "thread-42");
        assert_eq!(event.project, "sample-app");
        assert_eq!(event.title, "修复登录问题");
    }

    #[test]
    fn codex_rollout_accepts_root_and_ignores_subagent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("rollout-root.jsonl");
        let lines = [
            serde_json::json!({"type":"session_meta","payload":{"session_id":"root-1","parent_thread_id":null,"source":"vscode","cwd":"E:\\AgentBell"}}),
            serde_json::json!({"type":"event_msg","payload":{"type":"user_message","message":"编译 APK"}}),
            serde_json::json!({"type":"event_msg","payload":{"type":"agent_message","message":"构建完成"}}),
            serde_json::json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1"}}),
        ].into_iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\n");
        fs::write(&root, lines).unwrap();
        let scan = scan_codex_file(&root).unwrap();
        assert_eq!(scan.completions.len(), 1);
        assert_eq!(scan.completions[0].title, "编译 APK");
        assert_eq!(scan.completions[0].message, "构建完成");

        let sub = dir.path().join("rollout-sub.jsonl");
        fs::write(&sub, serde_json::json!({"type":"session_meta","payload":{"session_id":"sub-1","parent_thread_id":"root-1","source":{"subagent":{"other":"test"}}}}).to_string()).unwrap();
        assert!(scan_codex_file(&sub).is_none());
    }

    #[test]
    fn deepseek_turn_end_is_definitive() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("sessions").join("demo");
        fs::create_dir_all(&session_dir).unwrap();
        let rows = [
            serde_json::json!({"type":"session","id":"dsh-1","cwd":"E:\\demo"}).to_string(),
            serde_json::json!({"type":"session/title","data":{"title":"构建项目"}}).to_string(),
            serde_json::json!({"type":"turn/start"}).to_string(),
            serde_json::json!({"type":"turn/end"}).to_string(),
        ]
        .join("\n");
        let compressed = zstd::stream::encode_all(rows.as_bytes(), 1).unwrap();
        fs::write(session_dir.join("session.jsonl.zstd"), compressed).unwrap();
        let found = scan_dsh(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, 1);
        assert_eq!(found[0].2, "构建项目");
        assert_eq!(found[0].3, "dsh-1");
    }

    #[test]
    fn deepseek_packed_rows_and_subagents_are_handled() {
        let dir = tempfile::tempdir().unwrap();
        let root_dir = dir.path().join("sessions").join("root");
        fs::create_dir_all(&root_dir).unwrap();
        let packed = serde_json::json!([
            {"type":"session","id":"root","cwd":"E:\\demo","delegationDepth":0},
            {"type":"turn/end"}
        ])
        .to_string();
        fs::write(
            root_dir.join("session.jsonl.zstd"),
            zstd::stream::encode_all(packed.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        let sub_dir = dir.path().join("sessions").join("sub");
        fs::create_dir_all(&sub_dir).unwrap();
        let sub = [
            serde_json::json!({"type":"session","id":"sub","delegationDepth":1}),
            serde_json::json!({"type":"turn/end"}),
        ]
        .into_iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        fs::write(
            sub_dir.join("session.jsonl.zstd"),
            zstd::stream::encode_all(sub.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        let found = scan_dsh(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].3, "root");
    }

    #[test]
    fn haha_result_is_definitive() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session.jsonl");
        let mut out = fs::File::create(&file).unwrap();
        writeln!(
            out,
            "{}",
            serde_json::json!({"type":"user","sessionId":"haha-1","message":{"content":"检查代码"}})
        )
        .unwrap();
        writeln!(
            out,
            "{}",
            serde_json::json!({"type":"assistant","message":{"content":"处理中"}})
        )
        .unwrap();
        writeln!(
            out,
            "{}",
            serde_json::json!({"type":"result","subtype":"success"})
        )
        .unwrap();
        let found = scan_haha(&[dir.path().to_path_buf()]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, 1);
        assert_eq!(found[0].2, "haha-1");
        assert_eq!(found[0].3, "检查代码");
        assert!(!found[0].5);
    }

    #[test]
    fn haha_desktop_end_turn_is_definitive() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("desktop.jsonl");
        let rows = [
            serde_json::json!({"type":"session-meta","workDir":"E:\\HahaProject"}),
            serde_json::json!({"type":"user","sessionId":"haha-desktop","message":{"content":[{"type":"text","text":"发布项目"}]}}),
            serde_json::json!({"type":"assistant","isSidechain":false,"message":{"stop_reason":"end_turn","content":[{"type":"text","text":"完成"}]}}),
        ].into_iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\n");
        fs::write(file, rows).unwrap();
        let found = scan_haha(&[dir.path().to_path_buf()]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, 1);
        assert_eq!(found[0].2, "haha-desktop");
        assert_eq!(found[0].4, "HahaProject");
    }
}
