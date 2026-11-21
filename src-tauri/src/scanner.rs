use crate::config::{load_config, save_config};
use crate::encoding::decode_bytes;
use crate::process::suppress_window;
use crate::types::HostHistory;
use std::collections::{HashMap, VecDeque};
use std::net::Ipv4Addr;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

static SCANS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn scans() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    SCANS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn ip_num(ip: Ipv4Addr) -> u32 {
    u32::from(ip)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn hosts_for_subnet(ip: &str, mask: &str) -> Result<(Vec<String>, String), String> {
    let ip: Ipv4Addr = ip.parse().map_err(|_| "所选网卡的 IPv4 地址无效".to_string())?;
    let mask: Ipv4Addr = mask.parse().map_err(|_| "所选网卡的子网掩码无效".to_string())?;
    let mask_n = ip_num(mask);
    let inverse = !mask_n;
    if inverse & inverse.wrapping_add(1) != 0 {
        return Err("子网掩码不是连续掩码，无法计算扫描范围".to_string());
    }

    let prefix = mask_n.count_ones();
    let network_n = ip_num(ip) & mask_n;
    let broadcast_n = network_n | inverse;
    let start = if prefix <= 30 { network_n + 1 } else { network_n };
    let end = if prefix <= 30 { broadcast_n.saturating_sub(1) } else { broadcast_n };
    let count = if end >= start { end - start + 1 } else { 0 };
    if count > 4096 {
        return Err(format!(
            "该网段有 {count} 个可扫描地址，单次扫描上限为 4096。请改用更小的子网掩码范围。"
        ));
    }

    let hosts = (start..=end)
        .map(|n| Ipv4Addr::from(n).to_string())
        .collect();
    let network = format!("{}/{}", Ipv4Addr::from(network_n), prefix);
    Ok((hosts, network))
}

fn ping(ip: &str, source: &str) -> (bool, Option<String>) {
    let mut cmd = Command::new("ping");
    cmd.args(["-4", "-a", "-n", "1", "-w", "500", "-S", source, ip]);
    suppress_window(&mut cmd);
    match cmd.output() {
        Ok(out) if out.status.success() => {
            let text = decode_bytes(&out.stdout);
            let online = text.contains("TTL=") || text.contains("ttl=");
            (online, parse_ping_hostname(&text, ip))
        }
        _ => (false, None),
    }
}

fn parse_ping_hostname(text: &str, ip: &str) -> Option<String> {
    let marker = format!("[{ip}]");
    for line in text.lines() {
        let Some(index) = line.find(&marker) else { continue };
        let before = line[..index].trim();
        let name = before
            .strip_prefix("Pinging ")
            .or_else(|| before.strip_prefix("正在 Ping "))
            .unwrap_or(before)
            .trim();
        if !name.is_empty() && name != ip {
            return Some(name.to_string());
        }
    }
    None
}

fn parse_mac(text: &str, ip: &str) -> Option<String> {
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.first().copied() != Some(ip) || fields.len() < 2 {
            continue;
        }
        let mac = fields[1].replace(':', "-").to_uppercase();
        let octets: Vec<&str> = mac.split('-').collect();
        if octets.len() == 6
            && octets
                .iter()
                .all(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_hexdigit()))
        {
            return Some(mac);
        }
    }
    None
}

fn lookup_mac(ip: &str) -> Option<String> {
    let mut cmd = Command::new("arp");
    cmd.args(["-a", ip]);
    suppress_window(&mut cmd);
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_mac(&decode_bytes(&output.stdout), ip)
}

#[tauri::command]
pub fn run_host_scan(
    app: AppHandle,
    task_id: String,
    adapter: String,
    ip: String,
    mask: String,
) -> Result<(), String> {
    let (hosts, network) = hosts_for_subnet(&ip, &mask)?;
    if hosts.is_empty() {
        return Err("该网段没有可扫描的其他主机地址".to_string());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut active = scans().lock().map_err(|_| "扫描状态不可用".to_string())?;
        if active.contains_key(&task_id) {
            return Err("该扫描任务已在运行".to_string());
        }
        active.insert(task_id.clone(), cancel.clone());
    }

    std::thread::spawn(move || {
        let initial_config = load_config(&app);
        let previous: HashMap<String, HostHistory> = initial_config
            .host_history
            .iter()
            .filter(|h| h.adapter == adapter && h.network == network)
            .map(|h| (h.ip.clone(), h.clone()))
            .collect();
        let queue = Arc::new(Mutex::new(VecDeque::from(hosts)));
        let workers = 48usize.min(queue.lock().map(|q| q.len()).unwrap_or(0)).max(1);
        let (tx, rx) = std::sync::mpsc::channel::<(String, bool, Option<String>, Option<String>)>();
        let mut joins = Vec::with_capacity(workers);

        for _ in 0..workers {
            let q = queue.clone();
            let tx = tx.clone();
            let stop = cancel.clone();
            let source = ip.clone();
            joins.push(std::thread::spawn(move || loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let next = q.lock().ok().and_then(|mut q| q.pop_front());
                let Some(host) = next else { break };
                let (responsive, hostname) = ping(&host, &source);
                let mac = if responsive { lookup_mac(&host) } else { None };
                let _ = tx.send((host, responsive, hostname, mac));
            }));
        }
        drop(tx);

        let mut scanned = 0usize;
        let mut online = Vec::new();
        for (host, responsive, hostname, mac) in rx {
            scanned += 1;
            let previous_host = previous.get(&host);
            let last_seen = if responsive { Some(now()) } else { previous_host.map(|h| h.last_seen) };
            let hostname = hostname.or_else(|| previous_host.and_then(|h| h.hostname.clone()));
            let mac = mac.or_else(|| previous_host.and_then(|h| h.mac.clone()));
            if responsive {
                online.push((
                    host.clone(),
                    last_seen.unwrap_or_else(now),
                    hostname.clone(),
                    mac.clone(),
                ));
            }
            let status = if responsive {
                "online"
            } else if previous_host.is_some() {
                "offline"
            } else {
                "undiscovered"
            };
            let _ = app.emit(
                "scan-host-result",
                serde_json::json!({
                    "task_id": task_id.clone(),
                    "ip": host,
                    "status": status,
                    "last_seen": last_seen,
                    "hostname": hostname,
                    "mac": mac,
                }),
            );
        }
        for join in joins {
            let _ = join.join();
        }

        let timestamp = now();
        let mut config = load_config(&app);
        for (host, seen_at, hostname, mac) in online {
            if let Some(existing) = config.host_history.iter_mut().find(|h| {
                h.adapter == adapter && h.network == network && h.ip == host
            }) {
                existing.last_seen = seen_at;
                existing.hostname = hostname;
                existing.mac = mac;
            } else {
                config.host_history.push(HostHistory {
                    adapter: adapter.clone(),
                    network: network.clone(),
                    ip: host,
                    last_seen: seen_at,
                    hostname,
                    mac,
                });
            }
        }
        let error = save_config(&app, &config).err();
        let cancelled = cancel.load(Ordering::Relaxed);
        if let Ok(mut active) = scans().lock() {
            active.remove(&task_id);
        }
        let _ = app.emit(
            "scan-done",
            serde_json::json!({
                "task_id": task_id.clone(),
                "scanned": scanned,
                "cancelled": cancelled,
                "error": error,
                "completed_at": timestamp,
            }),
        );
    });
    Ok(())
}

#[tauri::command]
pub fn stop_host_scan(task_id: String) -> Result<(), String> {
    if let Ok(active) = scans().lock() {
        if let Some(cancel) = active.get(&task_id) {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}
