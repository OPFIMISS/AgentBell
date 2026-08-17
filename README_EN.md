# AgentBell

English | [简体中文](README.md)

AgentBell is a local Windows notification hub for long-running coding agents. Run one lightweight EXE on the PC, pair the Android app over the same Wi-Fi/LAN, and receive native notifications when an agent finishes, fails, or needs attention.

## Supported Agents

- **Codex Desktop**: watches definitive root-task `task_complete` events in rollout files and ignores sub-agent completions.
- **Deepseek Harness EAC**: watches `~/.dsh/sessions/**/session.jsonl.zstd`, supports current `turn/end` events, legacy assistant completions, packed rows, and delegation filtering. A task already running when AgentBell starts will still notify when its current turn ends.
- **Claude Code Haha**: activates only while Haha's local desktop service is online and supports both desktop `assistant.stop_reason=end_turn` and CLI `result` completion records.
- **OpenCode**: automatically writes a global plugin when OpenCode is detected, listens for the official `session.idle` / `session.error` events, and filters child sessions through `parentID`.
- **OpenClaw**: automatically generates, installs, and enables a local plugin when the `openclaw` CLI is detected, using the official typed `agent_end` hook. A running Gateway may need one restart after first-time setup.
- **Hermes Agent**: automatically writes and enables a user plugin that listens for the official `on_session_end` lifecycle hook and ignores interrupted, unfinished sessions.
- All six adapters are enabled automatically. File watchers establish a baseline and never replay historical completions; plugin integrations become active the next time their agent starts.
- **Zcode is intentionally not integrated** because it currently exposes neither public source nor a stable plugin/hook contract. AgentBell does not reverse-engineer its private task database and label that fragile behavior as support.

## Features

- Automatic PC/Android discovery through multicast, broadcast, and TCP subnet fallback scanning.
- Stable device-ID pairing with PC approval and revocable tokens.
- Native Android high-priority notifications backed by a foreground service.
- Boot/package-update recovery, sticky service recovery, short wake locks, and a battery-optimization exemption shortcut.
- Real device manufacturer/model display, such as `MEIZU 21`.
- Windows tray icon with Open WebUI and Exit commands.
- Persistent diagnostics without logging tokens, source code, or full conversation contents.

## How Agent Integration Works

AgentBell does not guess completion from elapsed idle time. For Codex, Deepseek Harness EAC, and Claude Code Haha, the Windows process observes existing local session records in read-only mode and recognizes each agent's definitive turn boundary. OpenCode, OpenClaw, and Hermes use their public plugin systems to subscribe to `session.idle`, `agent_end`, and `on_session_end`. Sub-agents, side-chain sessions, and incomplete output are filtered out.

On startup, every adapter records a baseline of current session files and completion positions, so old tasks are not replayed. A newly observed terminal record is normalized into an `AgentEvent` containing only bounded agent, title, project, and short result fields. AgentBell does not read or upload source-code bodies.

The PC stores the event in a local queue and delivers it over two LAN paths. An open WebUI receives it immediately through WebSocket, while the Android foreground service polls incremental events with its approved per-device token and hands them to the native Android notification system. Pairing, token validation, history, and delivery stay on the PC and local network; no AgentBell cloud service is required.

The PC WebUI and its local history file retain at most 100 events. Use **Select** or long-press a task card to select all or delete records in batches. Android only receives notifications and does not maintain this PC history list.

## Quick Start

1. Run `dist\AgentBell.exe` and allow it on Windows private networks.
2. Install `dist\AgentBell.apk` on Android.
3. Open the app, grant nearby-device and notification permissions, then select the discovered PC.
4. Approve the matching device and pairing code in the PC WebUI.
5. Use **Send test** to verify the notification path.

Supported agents are watched or configured automatically. If OpenCode, OpenClaw, or Hermes was already running during first-time configuration, restart that agent once so it loads the new local plugin.

The local WebUI is available at `http://127.0.0.1:43821`. On Android emulators, `http://10.0.2.2:43821` can be used when the emulator provides the standard host bridge.

## Network and Security

- HTTP/API: `43821/TCP`
- Discovery: `43820/UDP`
- Discovery packets never contain task titles or conversation contents.
- Device tokens remain in AgentBell's local application data and can be revoked from the WebUI.
- Empty or unknown event cursors establish a safe baseline instead of replaying all history.

## Custom Events

```powershell
.\AgentBell.exe emit --agent custom --kind completed --title "Task complete" --project "demo"
```

## Build

```powershell
powershell -ExecutionPolicy Bypass -File .\tools\build.ps1
```

Artifacts are written to `dist\AgentBell.exe` and `dist\AgentBell.apk`.

## Data and Diagnostics

Configuration, trusted devices, recent events, and `agentbell.log` are stored in the current user's local application-data directory. Open **Diagnostics** in the WebUI to inspect adapter and delivery activity.
