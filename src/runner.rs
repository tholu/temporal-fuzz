use crate::case::{base64_encode, snippet, AdapterOutput, Op, OutcomeKind, RunOutcome};
use serde::Serialize;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Serialize)]
struct AdapterRequest<'a> {
    payload_b64: String,
    ops: &'a [Op],
}

pub fn run_adapter(target: &[String], payload: &[u8], ops: &[Op], timeout_ms: u64) -> RunOutcome {
    let started = Instant::now();
    let request = AdapterRequest {
        payload_b64: base64_encode(payload),
        ops,
    };
    let input = match serde_json::to_vec(&request) {
        Ok(input) => input,
        Err(err) => {
            return RunOutcome {
                kind: OutcomeKind::SpawnError,
                adapter: None,
                exit_code: None,
                stderr: String::new(),
                runtime_ms: started.elapsed().as_millis(),
                max_rss_kb: None,
                stdout_snippet: String::new(),
                error: Some(format!("failed to serialize adapter request: {err}")),
            };
        }
    };

    if target.is_empty() {
        return spawn_error(started, "empty target command".to_string());
    }

    let mut child = match Command::new(&target[0])
        .args(&target[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => return spawn_error(started, format!("failed to spawn target: {err}")),
    };

    let stdout_handle = child.stdout.take().map(|mut stdout| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf);
            buf
        })
    });
    let stderr_handle = child.stderr.take().map(|mut stderr| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf);
            buf
        })
    });

    let stdin_error = if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&input).and_then(|_| stdin.flush()).err()
    } else {
        None
    };

    let timeout = Duration::from_millis(timeout_ms);
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() >= timeout => {
                timed_out = true;
                let _ = child.kill();
                break child.wait().ok();
            }
            Ok(None) => thread::sleep(Duration::from_millis(5)),
            Err(_) => break None,
        }
    };

    let stdout = join_reader(stdout_handle);
    let stderr = join_reader(stderr_handle);
    let runtime_ms = started.elapsed().as_millis();
    let stderr_text = String::from_utf8_lossy(&stderr).to_string();
    let stdout_text = String::from_utf8_lossy(&stdout).to_string();

    if timed_out {
        return RunOutcome {
            kind: OutcomeKind::Timeout,
            adapter: None,
            exit_code: status.and_then(|status| status.code()),
            stderr: stderr_text,
            runtime_ms,
            max_rss_kb: None,
            stdout_snippet: snippet(&stdout_text, 4096),
            error: Some(format!("target exceeded timeout_ms={timeout_ms}")),
        };
    }

    let Some(status) = status else {
        return RunOutcome {
            kind: OutcomeKind::SpawnError,
            adapter: None,
            exit_code: None,
            stderr: stderr_text,
            runtime_ms,
            max_rss_kb: None,
            stdout_snippet: snippet(&stdout_text, 4096),
            error: Some("failed to wait for target".to_string()),
        };
    };

    if !status.success() {
        return RunOutcome {
            kind: OutcomeKind::Crash,
            adapter: None,
            exit_code: status.code(),
            stderr: stderr_text,
            runtime_ms,
            max_rss_kb: None,
            stdout_snippet: snippet(&stdout_text, 4096),
            error: stdin_error.map(|err| format!("failed to write target stdin: {err}")),
        };
    }

    match serde_json::from_slice::<AdapterOutput>(&stdout) {
        Ok(adapter) if valid_adapter_output(&adapter) => RunOutcome {
            kind: OutcomeKind::Ok,
            adapter: Some(adapter),
            exit_code: status.code(),
            stderr: stderr_text,
            runtime_ms,
            max_rss_kb: None,
            stdout_snippet: snippet(&stdout_text, 4096),
            error: None,
        },
        Ok(_) => RunOutcome {
            kind: OutcomeKind::InvalidStdout,
            adapter: None,
            exit_code: status.code(),
            stderr: stderr_text,
            runtime_ms,
            max_rss_kb: None,
            stdout_snippet: snippet(&stdout_text, 4096),
            error: Some("stdout JSON did not match adapter protocol".to_string()),
        },
        Err(err) => RunOutcome {
            kind: OutcomeKind::InvalidStdout,
            adapter: None,
            exit_code: status.code(),
            stderr: stderr_text,
            runtime_ms,
            max_rss_kb: None,
            stdout_snippet: snippet(&stdout_text, 4096),
            error: Some(format!("invalid stdout JSON: {err}")),
        },
    }
}

fn spawn_error(started: Instant, error: String) -> RunOutcome {
    RunOutcome {
        kind: OutcomeKind::SpawnError,
        adapter: None,
        exit_code: None,
        stderr: String::new(),
        runtime_ms: started.elapsed().as_millis(),
        max_rss_kb: None,
        stdout_snippet: String::new(),
        error: Some(error),
    }
}

fn join_reader(handle: Option<thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default()
}

fn valid_adapter_output(output: &AdapterOutput) -> bool {
    matches!(output.status.as_str(), "ok" | "error") && !output.output_hash.is_empty()
}
