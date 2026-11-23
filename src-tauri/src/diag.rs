use crate::encoding::decode_bytes;
use crate::process::suppress_window;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter};

/// 正在运行的诊断子进程，按 `task_id` 登记，供停止命令终止。
static CHILDREN: OnceLock<Mutex<HashMap<String, Child>>> = OnceLock::new();

fn children() -> &'static Mutex<HashMap<String, Child>> {
    CHILDREN.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 逐行读取子进程输出并实时通过 `diag-line` 事件回流。
fn emit_lines<R: Read>(reader: R, app: &AppHandle, task_id: &str) {
    let mut br = BufReader::new(reader);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        match br.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {
                let line = decode_bytes(&buf);
                let line = line.trim_end_matches(&['\r', '\n'][..]).to_string();
                if !line.is_empty() {
                    let _ = app.emit_to(
                        "main",
                        "diag-line",
                        serde_json::json!({ "task_id": task_id, "line": line }),
                    );
                }
            }
            Err(_) => break,
        }
    }
}

/// 执行网络诊断命令：独立线程启动子进程，stdout/stderr 逐行实时回流，
/// 结束后发送 `diag-done`。命令立即返回，不阻塞界面；不同任务以 `task_id` 隔离。
#[tauri::command]
pub fn run_diagnostic(
    app: AppHandle,
    task_id: String,
    preset: String,
    target: Option<String>,
    extra_args: Vec<String>,
) -> Result<(), String> {
    let (program, mut base_args): (&str, Vec<String>) = match preset.as_str() {
        "ipconfig_all" => ("ipconfig", vec!["/all".to_string()]),
        "arp" => ("arp", vec!["-a".to_string()]),
        "route" => ("route", vec!["print".to_string()]),
        "flushdns" => ("ipconfig", vec!["/flushdns".to_string()]),
        "ping" => ("ping", vec![]),
        "tracert" => ("tracert", vec![]),
        "nslookup" => ("nslookup", vec![]),
        "netstat" => ("netstat", vec![]),
        other => return Err(format!("未知诊断命令：{other}")),
    };

    match preset.as_str() {
        "ping" | "tracert" | "nslookup" => {
            if let Some(t) = target.filter(|s| !s.is_empty()) {
                base_args.push(t);
            }
        }
        _ => {}
    }
    base_args.extend(extra_args);

    let mut cmd = Command::new(program);
    cmd.args(&base_args);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    suppress_window(&mut cmd);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = app.emit_to(
                "main",
                "diag-line",
                serde_json::json!({ "task_id": task_id, "line": format!("命令启动失败：{e}") }),
            );
            let _ = app.emit_to(
                "main",
                "diag-done",
                serde_json::json!({ "task_id": task_id, "code": -1 }),
            );
            return Ok(());
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    children().lock().unwrap().insert(task_id.clone(), child);

    let app_clone = app.clone();
    let tid = task_id.clone();
    std::thread::spawn(move || {
        std::thread::scope(|scope| {
            if let Some(e) = stderr {
                scope.spawn(|| emit_lines(e, &app_clone, &tid));
            }
            if let Some(s) = stdout {
                emit_lines(s, &app_clone, &tid);
            }
        });
        let child = children().lock().unwrap().remove(&tid);
        let code = match child {
            Some(mut c) => c.wait().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1),
            None => -2, // 已被停止命令终止
        };
        let _ = app_clone.emit_to(
            "main",
            "diag-done",
            serde_json::json!({ "task_id": tid, "code": code }),
        );
    });

    Ok(())
}

/// 终止正在运行的诊断子进程（等价于在 CMD 中 Ctrl+C / 结束进程）。
#[tauri::command]
pub fn stop_diagnostic(task_id: String) -> Result<(), String> {
    let child = children().lock().unwrap().remove(&task_id);
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
    Ok(())
}
