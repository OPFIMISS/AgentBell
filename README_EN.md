# AgentBell

English | [简体中文](README.md)

AgentBell is a local Windows notification hub for long-running coding agents. Run one lightweight EXE on the PC, pair the Android app over the same Wi-Fi/LAN, and receive native notifications when an agent finishes, fails, or needs attention.

## Supported Agents

- **Codex Desktop**: watches definitive root-task `task_complete` events in rollout files and ignores sub-agent completions.
- **Deepseek Harness EAC**: watches `~/.dsh/sessions/**/session.jsonl.zstd`, supports current `turn/end` events, legacy assistant completions, packed rows, and delegation filtering. A task already running when AgentBell starts will still notify when its current turn ends.
- **Claude Code Haha**: activates only while Haha's local desktop service is online and supports both desktop `assistant.stop_reason=end_turn` and CLI `result` completion records.
- All three adapters are enabled automatically. AgentBell establishes a baseline on startup and never replays historical completions.

## Features

- Automatic PC/Android discovery through multicast, broadcast, and TCP subnet fallback scanning.
- Stable device-ID pairing with PC approval and revocable tokens.
- Native Android high-priority notifications backed by a foreground service.
- Boot/package-update recovery, sticky service recovery, short wake locks, and a battery-optimization exemption shortcut.
- Real device manufacturer/model display, such as `MEIZU 21`.
- Windows tray icon with Open WebUI and Exit commands.
- Persistent diagnostics without logging tokens, source code, or full conversation contents.

## Quick Start

1. Run `dist\AgentBell.exe` and allow it on Windows private networks.
2. Install `dist\AgentBell.apk` on Android.
3. Open the app, grant nearby-device and notification permissions, then select the discovered PC.
4. Approve the matching device and pairing code in the PC WebUI.
5. Use **Send test** to verify the notification path.

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
