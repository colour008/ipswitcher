use crate::config::{load_host_history, update_host_history};
use crate::types::HostHistory;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as native;
#[cfg(not(windows))]
mod native {
    use super::*;
    pub(super) fn bindings(_: &str, _: u32) -> Result<Vec<Binding>, String> {
        Err("主机扫描仅支持 Windows".into())
    }
    pub(super) fn probe(_: Binding, _: Ipv4Addr) -> (bool, Option<String>) { (false, None) }
    pub(super) fn neighbors(_: u32) -> HashSet<Ipv4Addr> { HashSet::new() }
    pub(super) fn hostname(_: Ipv4Addr) -> Option<String> { None }
}

const LIMIT: usize = 4096;
const PROBE_WORKERS: usize = 32;
const BATCH_DELAY: Duration = Duration::from_millis(60);
const REPEAT_DELAY: Duration = Duration::from_secs(30);

#[derive(Clone, Copy)]
struct Binding {
    ip: Ipv4Addr,
    prefix: u8,
    ethernet: bool,
}

impl Binding {
    fn range(self) -> (u32, u32) {
        let host_bits = u32::MAX.checked_shr(self.prefix as u32).unwrap_or(0);
        let base = u32::from(self.ip) & !host_bits;
        (base, base | host_bits)
    }
    fn cidr(self) -> String {
        format!("{}/{}", Ipv4Addr::from(self.range().0), self.prefix)
    }
    fn local_slice(self) -> Option<Self> {
        (self.prefix < 20).then_some(Self { prefix: 24, ..self })
    }
}

#[tauri::command]
pub async fn list_scan_subnets(adapter: String, if_index: u32) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let bindings = native::bindings(&adapter, if_index)?;
        let mut choices = BTreeSet::new();
        for binding in bindings {
            choices.insert(binding.cidr());
            if let Some(slice) = binding.local_slice() {
                choices.insert(slice.cidr());
            }
        }
        Ok(choices.into_iter().collect())
    }).await.map_err(|e| e.to_string())?
}

struct Target {
    ip: Ipv4Addr,
    source: Binding,
    network: Arc<str>,
}

fn targets(mut bindings: Vec<Binding>, filter: Option<&str>) -> Result<Vec<Target>, String> {
    let local: HashSet<_> = bindings.iter().map(|b| b.ip).collect();
    // Overlapping networks use the most specific source. Alias addresses are never scanned.
    bindings.sort_by_key(|b| (std::cmp::Reverse(b.prefix), b.ip));
    let mut networks = HashSet::new();
    let mut targets = BTreeMap::new();
    for source in bindings {
        let network: Arc<str> = source.cidr().into();
        let range = match filter {
            Some(f) if f != network.as_ref() => match source.local_slice() {
                Some(slice) if slice.cidr() == f => slice,
                _ => continue,
            },
            _ => source,
        };
        if !networks.insert(range.cidr()) {
            continue;
        }
        let (base, broadcast) = range.range();
        let skip = u32::from(range.prefix < 31);
        let start = base + skip;
        let end = broadcast - skip;
        if u64::from(end) - u64::from(start) + 1 > LIMIT as u64 + local.len() as u64 {
            return Err(format!("网段 {network} 超过单轮 {LIMIT} 个地址的上限，可在网段列表选择本机所在的 /24 范围"));
        }
        for ip in (start..=end).map(Ipv4Addr::from).filter(|v| !local.contains(v)) {
            targets.entry(ip).or_insert_with(|| Target {
                ip,
                source,
                network: network.clone(),
            });
            if targets.len() > LIMIT {
                return Err(format!("所选网段合计超过 {LIMIT} 个地址，请指定单个网段"));
            }
        }
    }
    if targets.is_empty() {
        return Err("所选网段已变化或没有其他可扫描地址".into());
    }
    Ok(targets.into_values().collect())
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

struct Session {
    id: String,
    stop: Arc<AtomicBool>,
    wake: mpsc::Sender<()>,
}
static ACTIVE: Mutex<Option<Session>> = Mutex::new(None);

#[derive(Clone, Serialize)]
struct HostRow {
    ip: Ipv4Addr,
    status: &'static str,
    last_seen: Option<u64>,
    hostname: Option<String>,
    mac: Option<String>,
}

enum Update {
    Probe(usize, bool, Option<String>),
    Name(usize, Option<String>),
}

// A slow system resolver cannot consume probe workers or accumulate threads on successive scans.
struct NameJob {
    index: usize,
    ip: Ipv4Addr,
    active: Arc<AtomicBool>,
    tx: mpsc::Sender<Update>,
}
fn name_queue() -> &'static mpsc::SyncSender<NameJob> {
    static QUEUE: OnceLock<mpsc::SyncSender<NameJob>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<NameJob>(128);
        let rx = Arc::new(Mutex::new(rx));
        for _ in 0..4 {
            let rx = rx.clone();
            let _ = std::thread::Builder::new().name("scan-hostname".into()).spawn(move || loop {
                let job = rx.lock().unwrap().recv();
                let Ok(job) = job else { return };
                if job.active.load(Ordering::Relaxed) {
                    let _ = job.tx.send(Update::Name(job.index, native::hostname(job.ip)));
                }
            });
        }
        tx
    })
}

struct Scan {
    app: AppHandle,
    id: String,
    adapter: String,
    if_index: u32,
    stop: Arc<AtomicBool>,
    scanned: usize,
}

impl Scan {
    fn flush(&self, batch: &mut Vec<HostRow>) {
        if batch.is_empty() {
            return;
        }
        let _ = self.app.emit_to("main", "scan-host-results", serde_json::json!({
            "task_id": self.id, "scanned": self.scanned, "rows": batch
        }));
        batch.clear();
    }

    fn round(&mut self, targets: &mut [Target], confirmed: &mut HashSet<(Arc<str>, Ipv4Addr)>) -> Result<(), String> {
        let history: HashMap<_, _> = load_host_history(&self.app)?.into_iter()
            .filter(|h| h.adapter == self.adapter)
            .filter_map(|h| {
                let ip = h.ip.parse::<Ipv4Addr>().ok()?;
                Some(((Arc::<str>::from(h.network.as_str()), ip), h))
            })
            .collect();
        let ips: Vec<_> = targets.iter().map(|t| t.ip).collect();
        self.app.emit_to("main", "scan-round-started", serde_json::json!({
            "task_id": self.id, "ips": ips
        })).map_err(|e| e.to_string())?;
        let neighbors = native::neighbors(self.if_index);
        targets.sort_by_cached_key(|t| {
            if neighbors.contains(&t.ip) { 0 }
            else if history.contains_key(&(t.network.clone(), t.ip)) { 1 }
            else { 2 }
        });
        let targets: &[Target] = targets;

        let mut online = HashMap::<usize, HostRow>::new();
        let mut batch = Vec::new();
        let name_active = Arc::new(AtomicBool::new(true));
        self.scanned = 0;
        let next = AtomicUsize::new(0);
        let (tx, rx) = mpsc::channel();
        std::thread::scope(|scope| -> Result<(), String> {
            for worker in 0..PROBE_WORKERS.min(targets.len()) {
                let tx = tx.clone();
                let next = &next;
                let stop = &self.stop;
                let started = std::thread::Builder::new().name("scan-probe".into()).spawn_scoped(scope, move || {
                    while !stop.load(Ordering::Relaxed) {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(t) = targets.get(i) else { break };
                        let (up, mac) = native::probe(t.source, t.ip);
                        if tx.send(Update::Probe(i, up, mac)).is_err() {
                            break;
                        }
                    }
                });
                if let Err(error) = started {
                    if worker == 0 {
                        return Err(format!("无法启动扫描线程：{error}"));
                    }
                    break;
                }
            }
            let mut queued_names = 0usize;
            let mut dns_deadline = None;
            let mut flush_at = Instant::now() + BATCH_DELAY;
            while !self.stop.load(Ordering::Relaxed) {
                let wait = flush_at.saturating_duration_since(Instant::now());
                match rx.recv_timeout(wait) {
                    Ok(Update::Probe(i, up, mac)) => {
                        self.scanned += 1;
                        let t = &targets[i];
                        let key = (t.network.clone(), t.ip);
                        let old = history.get(&key);
                        let same_device = old.filter(|h| mac.is_none() || h.mac.is_none() || h.mac == mac);
                        let row = HostRow {
                            ip: t.ip,
                            status: if up || confirmed.contains(&key) {
                                "online"
                            } else if old.is_some() {
                                "offline"
                            } else {
                                "undiscovered"
                            },
                            last_seen: if up { Some(now()) } else { old.map(|h| h.last_seen) },
                            hostname: same_device.and_then(|h| h.hostname.clone()),
                            mac: mac.or_else(|| old.and_then(|h| h.mac.clone())),
                        };
                        if up {
                            if row.hostname.is_none() && name_queue().try_send(NameJob {
                                index: i, ip: t.ip, active: name_active.clone(), tx: tx.clone()
                            }).is_ok() {
                                queued_names += 1;
                            }
                            online.insert(i, row.clone());
                        }
                        batch.push(row);
                        if self.scanned == targets.len() {
                            dns_deadline = Some(Instant::now() + Duration::from_secs(2));
                        }
                    }
                    Ok(Update::Name(i, name)) => {
                        queued_names -= 1;
                        if let (Some(row), Some(name)) = (online.get_mut(&i), name) {
                            row.hostname = Some(name);
                            batch.push(row.clone());
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                let finished = dns_deadline.is_some_and(|d| queued_names == 0 || Instant::now() >= d);
                if batch.len() >= 32 || Instant::now() >= flush_at || finished {
                    self.flush(&mut batch);
                    flush_at = Instant::now() + BATCH_DELAY;
                }
                if finished { break; }
            }
            name_active.store(false, Ordering::Relaxed);
            Ok(())
        })?;
        self.flush(&mut batch);
        *confirmed = online.keys().map(|&i| (targets[i].network.clone(), targets[i].ip)).collect();
        if online.is_empty() { return Ok(()); }
        update_host_history(&self.app, |history| {
            let mut incoming: HashMap<_, _> = online.into_iter().map(|(i, row)| {
                let record = HostHistory {
                    adapter: self.adapter.clone(),
                    network: targets[i].network.to_string(),
                    ip: row.ip.to_string(),
                    last_seen: row.last_seen.unwrap(),
                    hostname: row.hostname,
                    mac: row.mac,
                };
                ((record.network.clone(), record.ip.clone()), record)
            }).collect();
            for h in history.iter_mut().filter(|h| h.adapter == self.adapter) {
                if let Some(new) = incoming.remove(&(h.network.clone(), h.ip.clone())) { *h = new; }
            }
            history.extend(incoming.into_values());
        })
    }
}

#[tauri::command]
pub fn run_host_scan(app: AppHandle, task_id: String, adapter: String, if_index: u32,
    subnet: Option<String>, repeat: bool) -> Result<(), String> {
    let stop = Arc::new(AtomicBool::new(false));
    let (wake, wait) = mpsc::channel();
    {
        let mut active = ACTIVE.lock().map_err(|_| "扫描状态不可用")?;
        if active.is_some() { return Err("上一次扫描仍在运行，请稍后重试".into()); }
        *active = Some(Session { id: task_id.clone(), stop: stop.clone(), wake });
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut scan = Scan { app, id: task_id, adapter, if_index, stop, scanned: 0 };
        let mut confirmed = HashSet::new();
        let result: Result<(), String> = (|| {
            while !scan.stop.load(Ordering::Relaxed) {
                let mut plan = targets(native::bindings(&scan.adapter, if_index)?, subnet.as_deref())?;
                scan.round(&mut plan, &mut confirmed)?;
                if !repeat || scan.stop.load(Ordering::Relaxed) { break; }
                let _ = scan.app.emit_to("main", "scan-waiting", &scan.id);
                if wait.recv_timeout(REPEAT_DELAY).is_ok() { break; }
            }
            Ok(())
        })();
        if let Ok(mut active) = ACTIVE.lock() { active.take(); }
        let _ = scan.app.emit_to("main", "scan-done", serde_json::json!({
            "task_id": scan.id, "scanned": scan.scanned, "cancelled": scan.stop.load(Ordering::Relaxed),
            "error": result.err()
        }));
    });
    Ok(())
}

#[tauri::command]
pub fn stop_host_scan(task_id: String) -> Result<(), String> {
    let active = ACTIVE.lock().map_err(|_| "扫描状态不可用")?;
    if let Some(s) = active.as_ref().filter(|s| s.id == task_id) {
        s.stop.store(true, Ordering::Relaxed);
        let _ = s.wake.send(());
    }
    Ok(())
}
