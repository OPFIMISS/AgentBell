# AgentBell

[English](README_EN.md) | 简体中文

AgentBell 是一个本地运行的 Windows Agent 通知中枢。电脑端启动一个轻量 `exe`，Android 端打开 APK 即可在同一 Wi-Fi / 局域网互相发现、配对，并在锁屏或后台时收到原生系统通知。二维码网页仍作为备用入口。

## 支持的 Agent

- **Codex Desktop**：监听根任务 rollout 的明确 `task_complete`，并保留原生 `notify` 回调作为双保险；不会把子代理完成误报为整个对话完成。
- **Deepseek Harness EAC**：默认监听 `~/.dsh/sessions/**/session.jsonl.zstd` 的 `turn/end`，兼容旧版 `assistant` 终态、打包记录和子代理过滤。AgentBell 在任务执行中启动也可以在本轮结束时通知，不要求新建对话。
- **Claude Code Haha**：仅在 Haha 本地服务在线时监听其项目会话，支持桌面端 `assistant.stop_reason=end_turn` 和 CLI `result` 终态，不会冒充普通 Claude Code Hook。
- **OpenCode**：检测到本机 OpenCode 后自动写入全局插件，监听官方 `session.idle` / `session.error` 事件，并通过 `parentID` 过滤子会话。
- **OpenClaw**：检测到 `openclaw` 后自动生成、安装并启用本地插件，监听官方类型化 `agent_end` Hook；首次配置后，正在运行的 OpenClaw Gateway 可能需要重启一次。
- **Hermes Agent**：检测到 Hermes 后自动写入用户插件并尝试启用，监听官方 `on_session_end` 生命周期 Hook，忽略主动中断且尚未结束的会话。
- 以上六个 Agent 均为**默认接入**，不需要点击安装按钮。文件监听型适配器启动时建立基线，不会补发旧任务；插件型适配器在对应 Agent 下次启动时自动生效。
- **Zcode**：当前未接入。它没有公开源码或稳定的插件 / Hook 契约，AgentBell 不通过逆向私有数据库来制造易失效的“支持”。
- Android APK 与 PC 自动互相发现，显示真实设备制造商/型号（例如 `MEIZU 21`）；也支持二维码网页和手动地址。
- APK 使用 Android 前台服务轮询新事件并发送高优先级原生通知，不依赖浏览器通知权限。
- APK 支持开机/应用更新恢复、前台服务自恢复、短时唤醒锁和电池优化豁免入口。
- Windows 托盘右键可打开 WebUI 或退出 AgentBell。
- WebSocket 实时通知、最近 100 条历史、前台响铃与震动。
- 可选 ntfy 后台推送接口（配置字段已预留）。

## Agent 接入与通知原理

AgentBell 不靠“多久没有输出”来猜测任务是否完成。对于 Codex、Deepseek Harness EAC 和 Claude Code Haha，Windows 进程以只读方式观察 Agent 已经写入本机的会话记录，并识别各自明确的回合终态。对于 OpenCode、OpenClaw 和 Hermes，则使用项目公开的插件系统订阅 `session.idle`、`agent_end`、`on_session_end` 生命周期事件。子代理、旁路会话和未结束的输出会被过滤。

启动时，每个适配器先为现有会话建立文件位置与完成序号基线，因此不会把旧任务重新通知；之后只在检测到新的明确终态时生成统一的 `AgentEvent`。事件仅保留经过长度限制的 Agent 名称、任务标题、项目名和简短结果，不读取或上传代码正文。

事件进入 PC 本地队列后会同时走两条局域网链路：打开的 WebUI 通过 WebSocket 实时收到更新；Android APK 的前台服务携带已批准设备的独立令牌轮询增量事件，再交给 Android 系统通知栏。配对、令牌验证、历史记录和传输都在本机及同一局域网内完成，不依赖 AgentBell 云服务。

PC WebUI 默认展示并持久化最近 100 条记录。点击任务列表右上角“选择”，或长按任意任务卡片，可以全选或批量删除；删除会同步清理本地历史文件。Android 端只负责接收通知，不维护这份 PC 历史列表。

## 使用

1. 双击 `dist\AgentBell.exe`。
2. Windows 防火墙弹窗中只允许“专用网络”。
3. 安装并打开 `dist\AgentBell.apk`，点击发现的电脑；电脑端批准设备。模拟器可手动连接 `http://10.0.2.2:43821`。
4. 支持的 Agent 会自动监听或自动配置本地插件，无需在 WebUI 点击接入。若 OpenCode、OpenClaw 或 Hermes 当时已经运行，首次配置后重启对应 Agent 一次。
5. 点击“发送测试”验证手机链路。

通用事件入口：

```powershell
.\AgentBell.exe emit --agent custom --kind completed --title "任务完成" --project "demo"
```

## 局域网与授权

- HTTP 服务默认端口：`43821/TCP`。
- 发现服务：`43820/UDP`，同时发送组播、有限广播和网卡定向广播。
- 扫码链接携带当前进程的一次运行密钥；用该二维码配对时由手机端确认授权。
- 不带扫码密钥直接访问时，必须由 PC 端批准配对码。
- 发现广播不包含 Agent 名称、任务标题或对话内容。
- 设备令牌保存在本地配置中，可以随时从 PC 端撤销。

## 手机通知

APK 首次连接时会请求 Android 系统通知权限。批准后，常驻的低优先级“AgentBell 已连接”通知表示后台链路正在工作；Agent 完成时会出现独立的高优先级通知。浏览器二维码页面仍受普通 HTTP 与浏览器后台冻结限制，因此锁屏通知请使用 APK。

魅族、小米等系统建议同时允许自启动、锁定后台，并在 APK 中点击“允许后台持续运行”。

## 运行与退出

- WebUI：`http://127.0.0.1:43821`
- 托盘双击：打开 WebUI
- 托盘右键：打开 AgentBell / 退出 AgentBell

PC 端“诊断”页面可读取持久日志；日志文件位于 AgentBell 数据目录下的 `agentbell.log`，会记录适配器启动、Codex 完成识别、事件广播与局域网发现，但不会记录令牌或完整代码内容。

## 构建

```powershell
powershell -ExecutionPolicy Bypass -File .\tools\build.ps1
```

发布文件输出到 `dist\AgentBell.exe` 和 `dist\AgentBell.apk`。

## 数据位置

配置、可信设备和最近事件位于当前 Windows 用户的 Local AppData `AgentBell` 数据目录。AgentBell 不读取或上传代码内容，只解析 Agent 自己产生的会话终态与有限标题字段。
