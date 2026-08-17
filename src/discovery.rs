use crate::{
    model::{DiscoveredPeer, now_ms},
    server::AppState,
};
use anyhow::{Context, Result};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
    time::Duration,
};
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

const PORT: u16 = 43820;
const GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 83, 21);

pub fn local_ipv4s() -> Vec<Ipv4Addr> {
    let mut out: Vec<(i32, usize, Ipv4Addr)> = Vec::new();
    if let Ok(items) = if_addrs::get_if_addrs() {
        for (index, item) in items.into_iter().enumerate() {
            if let if_addrs::IfAddr::V4(v4) = item.addr {
                let ip = v4.ip;
                if ip.is_loopback()
                    || ip.is_link_local()
                    || ip.is_unspecified()
                    || ip.is_multicast()
                {
                    continue;
                }
                let name = item.name.to_ascii_lowercase();
                let mut score = if name.contains("wi-fi")
                    || name.contains("wifi")
                    || name.contains("wlan")
                    || name.contains("ethernet")
                {
                    100
                } else {
                    0
                };
                if [
                    "wsl",
                    "docker",
                    "hyper-v",
                    "vmware",
                    "virtual",
                    "tailscale",
                    "zerotier",
                    "vpn",
                ]
                .iter()
                .any(|p| name.contains(p))
                {
                    score -= 200;
                }
                let octets = ip.octets();
                if octets[0] == 192 && octets[1] == 168 {
                    score += 30;
                } else if octets[0] == 10 {
                    score += 20;
                }
                out.push((score, index, ip));
            }
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut ips = Vec::new();
    for (_, _, ip) in out {
        if !ips.contains(&ip) {
            ips.push(ip);
        }
    }
    ips
}

pub async fn start(
    state: Arc<AppState>,
    device_id: String,
    name: String,
    http_port: u16,
) -> Result<()> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    socket.set_broadcast(true)?;
    socket.set_multicast_ttl_v4(1)?;
    socket.set_nonblocking(true)?;
    socket
        .bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PORT).into())
        .context("绑定 AgentBell 发现端口失败")?;
    let ifaces = local_ipv4s();
    for ip in &ifaces {
        if let Err(err) = socket.join_multicast_v4(&GROUP, ip) {
            debug!(%ip, %err, "加入组播失败");
        }
    }
    info!(interfaces = ?ifaces, port = PORT, "局域网发现已启动");
    let socket = Arc::new(UdpSocket::from_std(socket.into())?);
    let sender = socket.clone();
    tokio::spawn(async move {
        let instance = uuid::Uuid::new_v4();
        let mut seq = 0u64;
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        loop {
            tick.tick().await;
            seq += 1;
            let payload = format!(
                "AGENTBELL1|ver=1|role=pc|dev={}|inst={}|seq={}|port={}|name={}|model={}",
                device_id,
                instance,
                seq,
                http_port,
                encode(&name),
                encode(&name)
            );
            let mut targets = vec![
                SocketAddrV4::new(GROUP, PORT),
                SocketAddrV4::new(Ipv4Addr::BROADCAST, PORT),
            ];
            if let Ok(items) = if_addrs::get_if_addrs() {
                for item in items {
                    if let if_addrs::IfAddr::V4(v4) = item.addr {
                        if let Some(ip) = v4.broadcast {
                            targets.push(SocketAddrV4::new(ip, PORT));
                        }
                    }
                }
            }
            targets.sort();
            targets.dedup();
            for target in targets {
                if let Err(err) = sender.send_to(payload.as_bytes(), target).await {
                    warn!(%target, %err, "发现信标发送失败");
                }
            }
        }
    });
    tokio::spawn(async move {
        let mut buf = [0u8; 2048];
        loop {
            match socket.recv_from(&mut buf).await {
                Ok((n, from)) => {
                    if let Ok(text) = std::str::from_utf8(&buf[..n]) {
                        if let Some(mut peer) = parse_beacon(text) {
                            if peer.device_id == state.store.lock().await.config.device_id {
                                continue;
                            }
                            peer.ip = from.ip().to_string();
                            peer.last_seen_ms = now_ms();
                            debug!(name = %peer.name, role = %peer.role, ip = %peer.ip, "发现 AgentBell 设备");
                            let mut peers = state.discovered.write().await;
                            peers.retain(|_, p| now_ms() - p.last_seen_ms < 8_000);
                            peers.insert(peer.device_id.clone(), peer);
                        }
                    }
                }
                Err(err) => {
                    warn!(%err, "发现接收失败");
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
    });
    Ok(())
}

fn parse_beacon(text: &str) -> Option<DiscoveredPeer> {
    if !text.starts_with("AGENTBELL1|") {
        return None;
    }
    let fields: HashMap<&str, &str> = text
        .split('|')
        .skip(1)
        .filter_map(|part| part.split_once('='))
        .collect();
    let device_id = fields.get("dev")?.to_string();
    let role = fields.get("role").copied().unwrap_or("unknown").to_string();
    let name = fields
        .get("name")
        .copied()
        .unwrap_or("AgentBell 设备")
        .to_string();
    let model = fields.get("model").copied().unwrap_or(&name).to_string();
    let port = fields.get("port").and_then(|v| v.parse().ok()).unwrap_or(0);
    Some(DiscoveredPeer {
        device_id,
        role,
        name,
        model,
        ip: String::new(),
        port,
        last_seen_ms: 0,
    })
}

fn encode(input: &str) -> String {
    input
        .chars()
        .filter(|c| !c.is_control() && *c != '|')
        .take(80)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_mobile_beacon() {
        let peer = parse_beacon(
            "AGENTBELL1|ver=1|role=mobile|dev=abc|port=0|name=MEIZU 21|model=MEIZU 21",
        )
        .unwrap();
        assert_eq!(peer.role, "mobile");
        assert_eq!(peer.name, "MEIZU 21");
    }
}
