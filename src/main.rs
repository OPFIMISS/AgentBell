#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adapters;
mod discovery;
mod integrations;
mod model;
mod server;
mod store;
mod tray;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use model::EventKind;
use std::{net::SocketAddr, time::Duration};
use tracing::info;

#[derive(Parser)]
#[command(name = "AgentBell", version, about = "把 Agent 完成状态送到你的手机")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(long, default_value_t = 0)]
        port: u16,
        #[arg(long)]
        no_open: bool,
    },
    Emit {
        #[arg(long)]
        agent: String,
        #[arg(long, value_enum, default_value_t = KindArg::Completed)]
        kind: KindArg,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        message: Option<String>,
        #[arg(long)]
        conversation_id: Option<String>,
        #[arg(trailing_var_arg = true)]
        payload: Vec<String>,
    },
    Install {
        #[arg(value_enum)]
        agent: InstallAgent,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum InstallAgent {
    Codex,
    All,
}

#[derive(Clone, Copy, ValueEnum)]
enum KindArg {
    Started,
    Completed,
    Failed,
    NeedsInput,
    ApprovalRequired,
}

impl From<KindArg> for EventKind {
    fn from(value: KindArg) -> Self {
        match value {
            KindArg::Started => Self::Started,
            KindArg::Completed => Self::Completed,
            KindArg::Failed => Self::Failed,
            KindArg::NeedsInput => Self::NeedsInput,
            KindArg::ApprovalRequired => Self::ApprovalRequired,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let log_dir = store::Store::data_dir()?;
    std::fs::create_dir_all(&log_dir)?;
    let file_appender = tracing_appender::rolling::never(&log_dir, "agentbell.log");
    let (writer, _log_guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "agentbell=info".into()),
        )
        .with_writer(writer)
        .with_ansi(false)
        .with_target(false)
        .init();
    match Cli::parse().command.unwrap_or(Command::Serve {
        port: 0,
        no_open: false,
    }) {
        Command::Serve { port, no_open } => serve(port, no_open).await,
        Command::Emit {
            agent,
            kind,
            title,
            project,
            message,
            conversation_id,
            payload,
        } => {
            let store = store::Store::load(0)?;
            let payload = (!payload.is_empty()).then(|| payload.join(" "));
            let event = adapters::event_from_payload(
                &agent,
                payload.as_deref(),
                title,
                project,
                message,
                conversation_id,
                kind.into(),
            )
            .normalize();
            adapters::post_local(event, store.config.port, &store.config.emitter_token)
                .await
                .context("AgentBell 未运行，无法发送通知")
        }
        Command::Install { agent } => {
            match agent {
                InstallAgent::Codex | InstallAgent::All => {
                    let path = adapters::install_codex(&std::env::current_exe()?)?;
                    println!("Codex 已接入 AgentBell：{}", path.display());
                }
            }
            Ok(())
        }
    }
}

async fn serve(port_override: u16, no_open: bool) -> Result<()> {
    let store = store::Store::load(port_override)?;
    let port = store.config.port;
    let integration_dir = store.dir.clone();
    let device_id = store.config.device_id.clone();
    let device_name = store.config.device_name.clone();
    tokio::task::spawn_blocking(move || integrations::configure(port, integration_dir));
    let state = server::AppState::new(store);
    adapters::start_watchers(state.clone());
    if let Err(err) = discovery::start(state.clone(), device_id, device_name, port).await {
        tracing::warn!(%err, "局域网发现未启动，扫码和直接访问仍可使用");
    }
    let app = server::router(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("端口 {port} 已被占用"))?;
    let url = format!("http://127.0.0.1:{port}");
    info!(%url, "AgentBell 已启动");
    tray::start(url.clone());
    if !no_open {
        let open_url = url.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let _ = open::that(open_url);
        });
    }
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}
