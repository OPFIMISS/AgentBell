use anyhow::{Context, Result};
use std::{fs, path::Path, path::PathBuf, process::Command};
use tracing::{info, warn};

const OPENCODE_PLUGIN: &str = r#"export const AgentBell = async ({ client, directory }) => {
  const parentSessions = new Map()
  const failedSessions = new Set()

  const post = async (payload) => {
    try {
      await fetch("http://127.0.0.1:__PORT__/api/events", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      })
    } catch {}
  }

  const loadSession = async (sessionID) => {
    try {
      const response = await client.session.get({ path: { id: sessionID } })
      return response?.data || response
    } catch {
      try {
        const response = await client.session.get({ sessionID })
        return response?.data || response
      } catch {
        return null
      }
    }
  }

  return {
    event: async ({ event }) => {
      const sessionID = event?.properties?.sessionID || event?.properties?.info?.id
      if (!sessionID) return

      if (event.type === "session.created" || event.type === "session.updated") {
        const info = event.properties?.info
        if (info) parentSessions.set(sessionID, info.parentID || null)
        return
      }
      if (event.type === "session.error") {
        failedSessions.add(sessionID)
        return
      }
      if (event.type !== "session.idle") return

      const session = await loadSession(sessionID)
      const parentID = session?.parentID ?? parentSessions.get(sessionID)
      if (parentID) return

      const failed = failedSessions.delete(sessionID)
      const folder = session?.directory || directory || ""
      const project = folder.split(/[\\/]/).filter(Boolean).pop() || "OpenCode"
      await post({
        id: crypto.randomUUID(),
        agent: "OpenCode",
        kind: failed ? "failed" : "completed",
        conversation_id: sessionID,
        title: session?.title || (failed ? "OpenCode 任务执行失败" : "OpenCode 任务已完成"),
        project,
        message: failed ? "OpenCode 本轮执行失败" : "OpenCode 本轮执行完成",
        timestamp_ms: Date.now(),
      })
    },
  }
}
"#;

const HERMES_PLUGIN_YAML: &str = r#"name: agentbell
version: 1.0.3
description: Send Hermes session completion events to the local AgentBell service.
provides_hooks:
  - on_session_end
"#;

const HERMES_PLUGIN: &str = r#"import json
import threading
import time
import urllib.request
import uuid


def _post(payload):
    try:
        request = urllib.request.Request(
            "http://127.0.0.1:__PORT__/api/events",
            data=json.dumps(payload).encode("utf-8"),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        urllib.request.urlopen(request, timeout=2).close()
    except Exception:
        pass


def _on_session_end(session_id, completed, interrupted, failed, platform, **kwargs):
    if interrupted or (not completed and not failed):
        return
    is_failed = bool(failed)
    payload = {
        "id": str(uuid.uuid4()),
        "agent": "Hermes Agent",
        "kind": "failed" if is_failed else "completed",
        "conversation_id": session_id or "hermes",
        "title": "Hermes 任务执行失败" if is_failed else "Hermes 任务已完成",
        "project": platform or "Hermes",
        "message": "Hermes 会话执行失败" if is_failed else "Hermes 会话执行完成",
        "timestamp_ms": int(time.time() * 1000),
    }
    threading.Thread(target=_post, args=(payload,), daemon=True).start()


def register(ctx):
    ctx.register_hook("on_session_end", _on_session_end, priority=0)
"#;

const OPENCLAW_PACKAGE: &str = r#"{
  "name": "agentbell-openclaw",
  "version": "1.0.3",
  "type": "module",
  "private": true,
  "openclaw": {
    "extensions": ["./index.js"]
  },
  "peerDependencies": {
    "openclaw": ">=2026.5.17"
  }
}
"#;

const OPENCLAW_MANIFEST: &str = r#"{
  "id": "agentbell",
  "name": "AgentBell",
  "description": "Send OpenClaw agent completion events to the local AgentBell service.",
  "version": "1.0.3",
  "activation": { "onStartup": true },
  "configSchema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {}
  }
}
"#;

const OPENCLAW_PLUGIN: &str = r#"import { definePluginEntry } from "openclaw/plugin-sdk/core"

export default definePluginEntry({
  id: "agentbell",
  name: "AgentBell",
  description: "Send OpenClaw completion events to AgentBell.",
  version: "1.0.3",
  register(api) {
    api.on("agent_end", async (event, ctx) => {
      const success = event?.success !== false && !event?.error
      const conversationID = ctx?.sessionKey || event?.sessionKey || event?.runId || crypto.randomUUID()
      try {
        await fetch("http://127.0.0.1:__PORT__/api/events", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            id: crypto.randomUUID(),
            agent: "OpenClaw",
            kind: success ? "completed" : "failed",
            conversation_id: conversationID,
            title: success ? "OpenClaw 任务已完成" : "OpenClaw 任务执行失败",
            project: ctx?.agentId || "OpenClaw",
            message: success ? "OpenClaw 本轮执行完成" : "OpenClaw 本轮执行失败",
            duration_ms: event?.durationMs || null,
            timestamp_ms: Date.now(),
          }),
        })
      } catch {}
    }, { priority: 0 })
  },
})
"#;

pub fn configure(port: u16, data_dir: PathBuf) {
    let Some(base) = directories::BaseDirs::new() else {
        warn!("无法确定用户目录，跳过外部 Agent Hook 配置");
        return;
    };
    let home = base.home_dir();

    if let Err(err) = create_hermes_bundle(&data_dir, port) {
        warn!(%err, "Hermes AgentBell 插件包生成失败");
    }
    let openclaw_bundle = match create_openclaw_bundle(&data_dir, port) {
        Ok(path) => Some(path),
        Err(err) => {
            warn!(%err, "OpenClaw AgentBell 插件包生成失败");
            None
        }
    };

    if detected(home, "opencode", &[home.join(".local/share/opencode")]) {
        match install_opencode(home, port) {
            Ok(path) => info!(path = %path.display(), "OpenCode AgentBell 插件已就绪"),
            Err(err) => warn!(%err, "OpenCode AgentBell 插件配置失败"),
        }
    }

    if detected(home, "hermes", &[home.join(".hermes")]) || command_available("hermes-agent") {
        match install_hermes(home, port) {
            Ok(path) => {
                let command = if command_available("hermes") {
                    Some("hermes")
                } else if command_available("hermes-agent") {
                    Some("hermes-agent")
                } else {
                    None
                };
                if let Some(command) = command
                    && !run(
                        command,
                        &["plugins", "enable", "agentbell", "--no-allow-tool-override"],
                    )
                {
                    warn!("Hermes 插件已写入，但自动启用失败；下次启动将重试");
                }
                info!(path = %path.display(), "Hermes AgentBell 插件已就绪");
            }
            Err(err) => warn!(%err, "Hermes AgentBell 插件配置失败"),
        }
    }

    if detected(home, "openclaw", &[home.join(".openclaw")])
        && let Some(path) = openclaw_bundle
    {
        configure_openclaw(&path);
    }
}

fn detected(home: &Path, command: &str, data_paths: &[PathBuf]) -> bool {
    command_available(command)
        || data_paths.iter().any(|path| path.exists())
        || home.join(format!(".{command}")).exists()
}

fn install_opencode(home: &Path, port: u16) -> Result<PathBuf> {
    let path = home.join(".config/opencode/plugins/agentbell.js");
    write(
        &path,
        &OPENCODE_PLUGIN.replace("__PORT__", &port.to_string()),
    )?;
    Ok(path)
}

fn install_hermes(home: &Path, port: u16) -> Result<PathBuf> {
    let dir = home.join(".hermes/plugins/agentbell");
    write_hermes(&dir, port)?;
    Ok(dir)
}

fn create_hermes_bundle(data_dir: &Path, port: u16) -> Result<PathBuf> {
    let dir = data_dir.join("integrations/hermes-agentbell");
    write_hermes(&dir, port)?;
    Ok(dir)
}

fn write_hermes(dir: &Path, port: u16) -> Result<()> {
    write(&dir.join("plugin.yaml"), HERMES_PLUGIN_YAML)?;
    write(
        &dir.join("__init__.py"),
        &HERMES_PLUGIN.replace("__PORT__", &port.to_string()),
    )?;
    Ok(())
}

fn create_openclaw_bundle(data_dir: &Path, port: u16) -> Result<PathBuf> {
    let dir = data_dir.join("integrations/openclaw-agentbell");
    write(&dir.join("package.json"), OPENCLAW_PACKAGE)?;
    write(&dir.join("openclaw.plugin.json"), OPENCLAW_MANIFEST)?;
    write(
        &dir.join("index.js"),
        &OPENCLAW_PLUGIN.replace("__PORT__", &port.to_string()),
    )?;
    Ok(dir)
}

fn configure_openclaw(path: &Path) {
    if !command_available("openclaw") {
        warn!(path = %path.display(), "已生成 OpenClaw 插件，但未找到 openclaw 命令，暂时无法自动安装");
        return;
    }
    let _installed = run(
        "openclaw",
        &["plugins", "install", &path.display().to_string()],
    );
    let enabled = run("openclaw", &["plugins", "enable", "agentbell"]);
    let permitted = run(
        "openclaw",
        &[
            "config",
            "set",
            "plugins.entries.agentbell.hooks.allowConversationAccess",
            "true",
        ],
    );
    if enabled && permitted {
        info!(path = %path.display(), "OpenClaw AgentBell 插件已就绪");
    } else {
        warn!(path = %path.display(), "OpenClaw 插件已生成，但自动安装或授权失败");
    }
}

fn write(path: &Path, content: &str) -> Result<()> {
    if fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(());
    }
    let parent = path.parent().context("插件路径缺少父目录")?;
    fs::create_dir_all(parent)?;
    fs::write(path, content)?;
    Ok(())
}

fn command_available(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let extensions = std::env::var_os("PATHEXT")
        .map(|value| {
            value
                .to_string_lossy()
                .split(';')
                .filter(|value| !value.is_empty())
                .map(str::to_ascii_lowercase)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![".exe".into(), ".cmd".into(), ".bat".into()]);
    std::env::split_paths(&path).any(|dir| {
        dir.join(name).is_file()
            || extensions
                .iter()
                .any(|extension| dir.join(format!("{name}{extension}")).is_file())
    })
}

fn run(program: &str, args: &[&str]) -> bool {
    let mut command = Command::new(program);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    match command.output() {
        Ok(output) if output.status.success() => true,
        Ok(output) => {
            warn!(
                program,
                status = ?output.status.code(),
                stderr = %String::from_utf8_lossy(&output.stderr).chars().take(240).collect::<String>(),
                "Agent 插件命令执行失败"
            );
            false
        }
        Err(err) => {
            warn!(program, %err, "无法执行 Agent 插件命令");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_opencode_lifecycle_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let path = install_opencode(dir.path(), 43900).unwrap();
        let text = fs::read_to_string(path).unwrap();
        assert!(text.contains("session.idle"));
        assert!(text.contains("session.error"));
        assert!(text.contains("127.0.0.1:43900"));
        assert!(text.contains("parentID"));
    }

    #[test]
    fn writes_hermes_session_end_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let path = install_hermes(dir.path(), 43901).unwrap();
        let code = fs::read_to_string(path.join("__init__.py")).unwrap();
        assert!(code.contains("on_session_end"));
        assert!(code.contains("127.0.0.1:43901"));
        assert!(path.join("plugin.yaml").exists());
    }

    #[test]
    fn writes_openclaw_agent_end_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_openclaw_bundle(dir.path(), 43902).unwrap();
        let code = fs::read_to_string(path.join("index.js")).unwrap();
        assert!(code.contains("agent_end"));
        assert!(code.contains("127.0.0.1:43902"));
        assert!(path.join("openclaw.plugin.json").exists());
    }
}
