//! `sp-cli hook handoff-check`.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;

use common::{Out, sp_cli, temp_dir};
use serde_json::{Value, json};


/// Answers every request with `{"data": tasks}`. Returns the base URL.
fn fake_sp(tasks: Value) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let body = json!({"data": tasks}).to_string();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    url
}

/// A transcript that wrote `handoff`, which exists with `text`.
fn session(dir: &tempfile::TempDir, text: &str) -> (String, String) {
    let handoff = dir.path().join("HANDOFF-2026-11-09-b.md");
    std::fs::write(&handoff, text).unwrap();
    let handoff = handoff.to_str().unwrap().to_owned();
    let entry = json!({"message": {"content": [{"type": "tool_use", "name": "Write", "input": {"file_path": handoff}}]}});
    let transcript = dir.path().join("t.jsonl");
    std::fs::write(&transcript, format!("{{\"type\":\"user\"}}\n{entry}\n")).unwrap();
    (json!({"transcript_path": transcript.to_str().unwrap(), "stop_hook_active": false}).to_string(), handoff)
}

fn check(payload: &str, sp: &str) -> Out {
    sp_cli(&["hook", "handoff-check"], payload, &[("SP_API_URL", sp), ("SP_API_TOKEN", "t")])
}

#[test]
fn a_handoff_without_a_task_blocks_the_stop() {
    let dir = temp_dir();
    let (payload, handoff) = session(&dir, "# Handoff\n");
    let done = check(&payload, &fake_sp(json!([])));
    assert_eq!(done.code, 2);
    assert_eq!(
        done.stderr,
        format!(
            "Handoff written without a matching SP task:\n- {handoff}: no SP task carries this path. Add one (tag handoff; \
             notes = path, blank line, Continue with block; run the super-productivity:handoff-sp-task skill).\n"
        )
    );
}

#[test]
fn a_task_with_the_prompt_passes_and_an_open_superseded_one_blocks() {
    let dir = temp_dir();
    let (payload, handoff) = session(&dir, "Supersedes: `HANDOFF-2026-11-08-a.md`\n");
    let good = json!({"id": "n", "isDone": false, "notes": format!("{handoff}\n\nRead first: {handoff}\nGo on.")});
    assert_eq!(check(&payload, &fake_sp(json!([good]))).code, 0);
    let old = json!({"id": "o", "isDone": false, "notes": "HANDOFF-2026-11-08-a.md"});
    let done = check(&payload, &fake_sp(json!([good, old])));
    assert_eq!(done.code, 2);
    assert!(done.stderr.contains("supersedes HANDOFF-2026-11-08-a, but its SP task o is still open"), "{}", done.stderr);
    let closed = json!({"id": "o", "isDone": true, "notes": "HANDOFF-2026-11-08-a.md"});
    assert_eq!(check(&payload, &fake_sp(json!([good, closed]))).code, 0);
}

#[test]
fn the_retry_and_an_unreachable_sp_pass() {
    let dir = temp_dir();
    let (payload, _) = session(&dir, "x");
    let retry = payload.replace("\"stop_hook_active\":false", "\"stop_hook_active\":true");
    assert_eq!(check(&retry, &fake_sp(json!([]))).code, 0);
    let down = check(&payload, "http://127.0.0.1:9");
    assert_eq!(down.code, 0);
    assert!(down.stderr.starts_with("handoff-check: SP not reachable (SP not reachable at http://127.0.0.1:9"));
}

#[test]
fn nothing_to_check_passes_without_calling_sp() {
    let dir = temp_dir();
    let transcript = dir.path().join("t.jsonl");
    std::fs::write(&transcript, "{}\n").unwrap();
    for payload in ["{bad", "{}", &json!({"transcript_path": transcript.to_str().unwrap()}).to_string()] {
        assert_eq!(check(payload, "http://127.0.0.1:9"), Out { code: 0, stdout: String::new(), stderr: String::new() });
    }
}
