//! `sp-cli` against a stub SP that keeps state.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use common::sp_cli;
use serde_json::{Map, Value, json};

const DAY: i64 = 86_400_000;

#[derive(Clone, Copy, PartialEq)]
enum Bridge {
    On,
    Off,
    Broken,
}

struct State {
    tasks: Vec<Value>,
    projects: Vec<Value>,
    tags: Vec<Value>,
    next: u32,
    bridge: Bridge,
    view_tag: Option<&'static str>,
    log: Vec<Value>,
}

/// One clock for both runs of a scenario, so seeded timestamps match.
fn now_ms() -> i64 {
    static NOW: std::sync::LazyLock<i64> = std::sync::LazyLock::new(|| chrono::Local::now().timestamp_millis());
    *NOW
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn seed_tasks() -> Vec<Value> {
    let now = now_ms();
    let today = today();
    let tasks = json!([
        {"id": "s1", "title": "Ünïcode → task", "projectId": "p1", "tagIds": ["g1", "TODAY"], "dueDay": "2026-11-01",
         "isDone": false, "created": now - 20 * DAY,
         "notes": "https://example.atlassian.net/browse/ABC-1?focusedCommentId=9&x=1\nTell Sam: \"x\"\nDecide: keep"},
        {"id": "s2", "title": "Repeating", "projectId": "INBOX_PROJECT", "tagIds": ["g2"], "dueWithTime": now - 3 * DAY,
         "repeatCfgId": "r1", "isDone": false, "created": now, "notes": "Rolled: 4 (last)",
         "timeSpentOnDay": {today.clone(): 900000}},
        {"id": "s3", "title": "Done focus", "tagIds": ["g3", "g4", "g5"], "isDone": true, "doneOn": now - 3600000,
         "timeSpentOnDay": {today.clone(): 5400000}, "created": now,
         "notes": "https://example.slack.com/archives/C1/p1792480207111589"},
        {"id": "s4", "title": "Old deferred", "tagIds": ["g2"], "isDone": false, "created": now - 30 * DAY, "parentId": "s1",
         "attachments": [{"id": "a1", "type": "LINK", "path": "https://github.com/o/r/pull/7", "title": "r PR #7"}],
         "notes": "see https://github.com/o/r/pull/7 and XYZ-12, DEF-4."},
        {"id": "s5", "title": "Focus open", "tagIds": ["g5"], "isDone": false, "timeSpentOnDay": {today: 900000}, "created": now},
    ]);
    tasks.as_array().unwrap().clone()
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn find(tasks: &mut [Value], id: &str) -> Option<usize> {
    tasks.iter().position(|t| t["id"] == id)
}

fn handle(st: &mut State, method: &str, target: &str, body: Option<Value>) -> (u16, Value) {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let q: Map<String, Value> =
        query.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_owned(), json!(decode(v)))).collect();
    let missing = (404, json!({"ok": false, "error": format!("Task {path} not found")}));
    st.next += 1;
    let n = st.next;
    match (method, path) {
        ("GET", "/projects") => (200, json!(st.projects)),
        ("GET", "/tags") => (200, json!(st.tags)),
        ("GET", "/tasks") => {
            let hit = |t: &&Value| {
                (q.get("includeDone") == Some(&json!("true")) || t["isDone"] != true)
                    && q.get("projectId").is_none_or(|p| &t["projectId"] == p)
                    && q.get("tagId").is_none_or(|g| t["tagIds"].as_array().is_some_and(|a| a.contains(g)))
                    && q.get("query").is_none_or(|s| {
                        t["title"].as_str().unwrap_or_default().to_lowercase().contains(&s.as_str().unwrap().to_lowercase())
                    })
            };
            (200, json!(st.tasks.iter().filter(hit).collect::<Vec<_>>()))
        }
        ("POST", "/task-repeat-cfgs") => (200, json!({"id": format!("rc{n}")})),
        ("POST", "/tasks") => {
            let mut t = json!({"id": format!("t{n}"), "isDone": false, "tagIds": [], "created": 1});
            for (k, v) in body.unwrap().as_object().unwrap() {
                if k != "isIgnoreShortSyntax" {
                    t[k] = v.clone();
                }
            }
            if let Some(g) = st.view_tag
                && t.get("parentId").is_none()
            {
                t["tagIds"].as_array_mut().unwrap().push(json!(g));
            }
            if t["title"] == "@@sp-cmd" && st.bridge != Bridge::Off {
                let cmd: Value = serde_json::from_str(t["notes"].as_str().unwrap()).unwrap();
                if st.bridge == Bridge::Broken {
                    t["title"] = json!("@@sp-cmd failed");
                    t["notes"] = json!("Error: boom");
                    st.tasks.push(t.clone());
                    return (200, t);
                }
                match cmd["op"].as_str().unwrap() {
                    "addProject" => st.projects.push(json!({"id": format!("p{n}"), "title": cmd["title"]})),
                    "addTag" => st.tags.push(json!({"id": format!("g{n}"), "title": cmd["title"]})),
                    _ => {
                        if let Some(i) = find(&mut st.tasks, cmd["taskId"].as_str().unwrap()) {
                            st.tasks[i]["attachments"] = cmd["attachments"].clone();
                        }
                    }
                }
                return (200, t);
            }
            st.tasks.push(t.clone());
            (200, t)
        }
        (_, p) if p.starts_with("/tasks/") => {
            let Some(i) = find(&mut st.tasks, &p[7..]) else { return missing };
            match method {
                "GET" => (200, st.tasks[i].clone()),
                "PATCH" => {
                    for (k, v) in body.unwrap().as_object().unwrap() {
                        st.tasks[i][k] = v.clone();
                    }
                    (200, st.tasks[i].clone())
                }
                "DELETE" => (200, json!({"id": st.tasks.remove(i)["id"]})),
                _ => missing,
            }
        }
        _ => missing,
    }
}

fn serve_one(stream: TcpStream, state: &Mutex<State>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap().to_owned(), parts.next().unwrap().to_owned());
    let mut length = 0;
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).unwrap();
        if h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
        {
            length = v.trim().parse().unwrap();
        }
    }
    let mut raw = vec![0; length];
    reader.read_exact(&mut raw).unwrap();
    let body: Option<Value> = (length > 0).then(|| serde_json::from_slice(&raw).unwrap());
    let mut st = state.lock().unwrap();
    st.log.push(json!({"method": method, "path": target, "body": body}));
    let (code, data) = handle(&mut st, &method, &target, body);
    drop(st);
    let text = if code == 200 { json!({"ok": true, "data": data}).to_string() } else { data.to_string() };
    let mut stream = stream;
    let _ = write!(
        stream,
        "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
}

fn fake_sp(bridge: Bridge, view_tag: Option<&'static str>) -> (String, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State {
        tasks: seed_tasks(),
        projects: json!([{"id": "INBOX_PROJECT", "title": "Inbox"}, {"id": "p1", "title": "Work"}]).as_array().unwrap().clone(),
        tags: ["waiting", "deferred", "urgent", "important", "focus"]
            .iter()
            .enumerate()
            .map(|(i, t)| json!({"id": format!("g{}", i + 1), "title": t}))
            .collect(),
        next: 0,
        bridge,
        view_tag,
        log: Vec::new(),
    }));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let shared = state.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            serve_one(stream, &shared);
        }
    });
    (url, state)
}

#[test]
fn selftest_against_the_stub() {
    let (url, state) = fake_sp(Bridge::On, None);
    let o = sp_cli(&["selftest"], "", &[("SP_API_URL", &url), ("SP_API_TOKEN", "t")]);
    assert_eq!((o.code, o.stdout.as_str()), (0, "selftest ok\n"), "{}", o.stderr);
    assert!(state.lock().unwrap().tasks.iter().all(|t| !t["title"].as_str().unwrap().contains("selftest")));
}

#[test]
fn bad_arguments_exit_2() {
    for args in [&[][..], &["nope"], &["add"], &["repeat", "x"], &["add", "t", "--note", "x"]] {
        assert_eq!(sp_cli(args, "", &[]).code, 2, "{args:?}");
    }
    let help = sp_cli(&["-h"], "", &[]);
    assert!(help.code == 0 && help.stdout.contains("sp-cli add TITLE"));
}
