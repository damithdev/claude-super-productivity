//! `sp-cli`: Super Productivity CLI over its local REST API (Settings > Misc > Local REST API).

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike};
use fancy_regex::Regex;
use serde_json::{Map, Value, json};

use crate::util;

type Obj = Map<String, Value>;

const HELP: &str = "Super Productivity CLI over its local REST API (Settings > Misc > Local REST API).

Token: read from SP's own local-rest-api-token file; SP_API_TOKEN env var overrides.
Projects and tags are referenced by title, case-insensitive.

  sp-cli list [--project P] [--tag T] [--today] [--search S] [--done]
  sp-cli add TITLE [--project P] [--tag T ...] [--notes N | --notes-file F] [--due YYYY-MM-DD] [--at HH:MM] [--parent ID] [--force]
      Dedupe: a link on the first notes line is the task's source. If any task ever had that source
      (open, done, archived or deleted; registry in ~/.claude/state/sp-sources.json,
      or $SP_STATE_DIR/sp-sources.json), add creates
      nothing and prints that task's id (the open one first). --force is for a different ask only.
  sp-cli update ID [--title ..] [--notes ..|--notes-file F] [--project P] [--tag T ...] [--due ..] [--at ..] [--no-due]
  sp-cli done ID | undone ID | delete ID | show ID
  sp-cli repeat ID (--weekdays | --daily | --days mon,fri)
  sp-cli projects | tags | selftest
  sp-cli review                                 (Weekly review lists: waiting, deferred, Decide, Who set this?, Friday test, Tell)
  sp-cli log [--day YYYY-MM-DD]                 (Wrap Up: append a line to the file in $SP_DAILY_LOG)
  sp-cli add-project TITLE | add-tag TITLE      (needs the bridge plugin, see below)
  sp-cli links [ID ...]                         (attach notes URLs and Jira keys as SP links; no ID = all open)
      add/update do this on their own whenever notes are set (bridge plugin 1.1+).
      Jira keys in notes link only when SP_JIRA_BASE (URL prefix, e.g. https://x.atlassian.net/browse/)
      and SP_JIRA_KEYS (comma list of project keys, e.g. ABC,DEF) are both set.
  sp-cli bridge-plugin [--out ZIP]

The REST API cannot create projects or tags; SP's plugin API can. The bridge plugin (install
once under SP Settings > Plugins) watches for a task titled @@sp-cmd, runs the JSON command
in its notes, then deletes the task. So the REST API stays the only transport.
";

const DAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
const CMD_TITLE: &str = "@@sp-cmd";
const MAX_LINKS: usize = 10;
/// Python text-mode writes on Windows; the daily log is CRLF throughout.
const NL: &str = if cfg!(windows) { "\r\n" } else { "\n" };

pub fn base() -> String {
    std::env::var("SP_API_URL").unwrap_or_else(|_| "http://127.0.0.1:3876".to_owned())
}

pub fn token() -> Option<String> {
    if let Some(t) = util::env_nonempty("SP_API_TOKEN") {
        return Some(t);
    }
    std::fs::read_to_string(token_file()?).ok().map(|t| t.trim().to_owned())
}

/// Where SP's Electron userData keeps the REST API token. macOS path not yet checked on a Mac.
pub fn token_file() -> Option<PathBuf> {
    let data = if cfg!(target_os = "macos") {
        util::home().join("Library").join("Application Support")
    } else {
        PathBuf::from(std::env::var_os("APPDATA")?)
    };
    Some(data.join("superProductivity").join("local-rest-api-token"))
}

/// The response's `data`. Errors carry the same text sp.py exits with.
pub fn call(method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
    let base = base();
    let req = ureq::request(method, &format!("{base}{path}"))
        .set("Authorization", &format!("Bearer {}", token().unwrap_or_default()))
        .timeout(Duration::from_secs(10));
    let sent = match body {
        Some(b) => req.set("Content-Type", "application/json").send_string(&b.to_string()),
        None => req.call(),
    };
    match sent {
        Ok(resp) => {
            let mut parsed: Value = serde_json::from_str(&resp.into_string()?)?;
            parsed.get_mut("data").map(Value::take).ok_or_else(|| anyhow!("'data'"))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let text: String = resp.into_string().unwrap_or_default().chars().take(300).collect();
            Err(anyhow!("SP API {method} {path}: {code} {text}"))
        }
        Err(e) => Err(anyhow!("SP not reachable at {base} ({e}). Is the app open with the local REST API on?")),
    }
}

fn get(path: &str) -> Result<Value> {
    call("GET", path, None)
}

fn send(method: &str, path: &str, body: Value) -> Result<Value> {
    call(method, path, Some(&body))
}

fn list(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

fn field<'a>(t: &'a Value, key: &str) -> &'a str {
    t.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn id_of(t: &Value) -> String {
    t.get("id").map(util::text).unwrap_or_else(|| "None".into())
}

fn truthy(t: &Value, key: &str) -> bool {
    util::truthy(t.get(key))
}

fn tag_ids(t: &Value) -> Vec<&Value> {
    t.get("tagIds").map(list).unwrap_or_default().iter().collect()
}

fn has_tag(t: &Value, id: &Value) -> bool {
    tag_ids(t).contains(&id)
}

fn rx(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid pattern")
}

fn captures<'t>(re: &Regex, s: &'t str) -> Option<fancy_regex::Captures<'t>> {
    re.captures(s).ok().flatten()
}

fn matches(re: &Regex, s: &str) -> bool {
    re.is_match(s).unwrap_or(false)
}

fn find_all<'t>(re: &Regex, s: &'t str) -> Vec<&'t str> {
    re.find_iter(s).flatten().map(|m| m.as_str()).collect()
}

fn local(ms: f64) -> chrono::DateTime<Local> {
    Local.timestamp_opt((ms / 1000.0).floor() as i64, 0).single().unwrap_or_default()
}

/// `datetime(...).timestamp()` for a naive local time, in ms; an ambiguous time takes the earlier.
fn local_ms(t: NaiveDateTime) -> Result<i64> {
    Local.from_local_datetime(&t).earliest().map(|t| t.timestamp_millis()).ok_or_else(|| anyhow!("no such local time {t}"))
}

fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

fn iso(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// `datetime.date.fromisoformat`.
fn date(s: &str) -> Result<NaiveDate> {
    let ok = s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-';
    ok.then(|| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()).flatten().ok_or_else(|| anyhow!("Invalid isoformat string: '{s}'"))
}

fn resolve(items: &Value, title: &str, kind: &str) -> Result<Value> {
    list(items)
        .iter()
        .find(|i| field(i, "title").to_lowercase() == title.to_lowercase())
        .map(|i| i["id"].clone())
        .ok_or_else(|| anyhow!("No {kind} named '{title}'. Create it in the SP app first (the API cannot)."))
}

fn due_fields(day: Option<&str>, at: Option<&str>) -> Result<Obj> {
    let day = day.filter(|d| !d.is_empty());
    let mut out = Obj::new();
    if let Some(d) = day {
        out.insert("dueDay".into(), json!(d));
    }
    if let Some(at) = at.filter(|a| !a.is_empty()) {
        let d = match day {
            Some(d) => date(d)?,
            None => Local::now().date_naive(),
        };
        let parts: Vec<&str> = at.split(':').collect();
        let [h, m] = parts[..] else { bail!("--at must be HH:MM, got '{at}'") };
        let int = |x: &str| x.trim().parse::<u32>().map_err(|_| anyhow!("invalid literal for int() with base 10: '{x}'"));
        let t = d.and_hms_opt(int(h)?, int(m)?, 0).ok_or_else(|| anyhow!("--at out of range: '{at}'"))?;
        out.insert("dueDay".into(), json!(iso(d)));
        out.insert("dueWithTime".into(), json!(local_ms(t)?));
    }
    Ok(out)
}

fn task_body(a: &Parsed, title: Option<&str>) -> Result<Obj> {
    let mut body = Obj::new();
    if let Some(t) = title.filter(|t| !t.is_empty()) {
        body.insert("title".into(), json!(t));
    }
    if let Some(f) = a.one("--notes-file").filter(|f| !f.is_empty()) {
        let text = util::read_text(Path::new(f)).ok_or_else(|| anyhow!("No such file or directory: '{f}'"))?;
        body.insert("notes".into(), json!(text.trim_end_matches('\n')));
    } else if let Some(n) = a.one("--notes") {
        body.insert("notes".into(), json!(n.replace("\\n", "\n")));
    }
    if let Some(p) = a.one("--project").filter(|p| !p.is_empty()) {
        body.insert("projectId".into(), resolve(&get("/projects")?, p, "project")?);
    }
    if let Some(wanted) = a.all("--tag") {
        let tags = get("/tags")?;
        let ids = wanted.iter().map(|t| resolve(&tags, t, "tag")).collect::<Result<Vec<_>>>()?;
        body.insert("tagIds".into(), Value::Array(ids));
    }
    body.extend(due_fields(a.one("--due"), a.one("--at"))?);
    Ok(body)
}

fn titles(items: &Value) -> HashMap<String, String> {
    list(items).iter().map(|i| (id_of(i), i.get("title").map(util::text).unwrap_or_default())).collect()
}

fn fmt(t: &Value, projects: &HashMap<String, String>, tags: &HashMap<String, String>) -> String {
    let p = t.get("projectId").map(util::text).and_then(|id| projects.get(&id).cloned()).unwrap_or_default();
    let tg: Vec<String> = tag_ids(t)
        .into_iter()
        .map(util::text)
        .filter(|i| i != "TODAY")
        .map(|i| tags.get(&i).cloned().unwrap_or(i))
        .collect();
    let tg = tg.join(",");
    let mut due = if truthy(t, "dueDay") { util::text(&t["dueDay"]) } else { String::new() };
    if truthy(t, "dueWithTime") {
        due = local(t["dueWithTime"].as_f64().unwrap_or(0.0)).format("%Y-%m-%d %H:%M").to_string();
    }
    let flags = format!(
        "{}{}{}",
        if truthy(t, "isDone") { "x " } else { "" },
        if truthy(t, "repeatCfgId") { "R " } else { "" },
        if truthy(t, "parentId") { "  " } else { "" }
    );
    let mut line = format!("{}  {flags}{}  [{p}]", id_of(t), t.get("title").map(util::text).unwrap_or_default());
    if !tg.is_empty() {
        line += &format!(" #{tg}");
    }
    if !due.is_empty() {
        line += &format!("  due {due}");
    }
    line
}

/// `urllib.request.quote(s)`: UTF-8 percent-encoding, `/` kept.
fn quote(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' | b'/' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn cmd_list(a: &Parsed) -> Result<()> {
    let mut q = Vec::new();
    if let Some(p) = a.one("--project").filter(|p| !p.is_empty()) {
        q.push(format!("projectId={}", util::text(&resolve(&get("/projects")?, p, "project")?)));
    }
    if a.has("--today") {
        q.push("tagId=TODAY".to_owned());
    } else if let Some(t) = a.one("--tag").filter(|t| !t.is_empty()) {
        q.push(format!("tagId={}", util::text(&resolve(&get("/tags")?, t, "tag")?)));
    }
    if let Some(s) = a.one("--search").filter(|s| !s.is_empty()) {
        q.push(format!("query={}", quote(s)));
    }
    if a.has("--done") {
        q.push("includeDone=true".to_owned());
    }
    let tasks = get(&format!("/tasks{}", if q.is_empty() { String::new() } else { format!("?{}", q.join("&")) }))?;
    let projects = titles(&get("/projects")?);
    let tags = titles(&get("/tags")?);
    for t in list(&tasks) {
        println!("{}", fmt(t, &projects, &tags));
    }
    Ok(())
}

fn sources_path() -> PathBuf {
    util::state_dir().join("sp-sources.json")
}

static SLACK_MESSAGE: LazyLock<Regex> = LazyLock::new(|| rx(r"^https://[\w.]*slack\.com/archives/(\w+)/p(\d{10})(\d{6})"));
static SLACK_THREAD: LazyLock<Regex> = LazyLock::new(|| rx(r"thread_ts=([\d.]+)"));
static SLACK_BARE: LazyLock<Regex> = LazyLock::new(|| rx(r"^https://[\w.]*slack\.com/archives/\w+/?$"));
static FOCUSED_COMMENT: LazyLock<Regex> = LazyLock::new(|| rx(r"focusedCommentId=\d+"));

/// The source link on the first line of the notes, normalised so one message = one key.
fn source_key(notes: Option<&str>) -> Option<String> {
    let notes = notes.unwrap_or_default();
    if notes.trim().is_empty() {
        return None;
    }
    let first = notes.trim_start().split('\n').next()?.split_whitespace().next()?;
    if !first.starts_with("http") {
        return None;
    }
    if let Some(m) = captures(&SLACK_MESSAGE, first) {
        let ts = match captures(&SLACK_THREAD, first) {
            Some(root) => root[1].to_owned(),
            None => format!("{}.{}", &m[2], &m[3]),
        };
        return Some(format!("slack:{}:{ts}", &m[1]));
    }
    if matches(&SLACK_BARE, first) {
        return None; // a bare DM/channel link names no message
    }
    let (base, query) = first.split_once('?').unwrap_or((first, ""));
    let keep = FOCUSED_COMMENT.find(query).ok().flatten().map(|m| format!("?{}", m.as_str())).unwrap_or_default();
    Some(format!("{}{keep}", base.trim_end_matches('/').to_lowercase()))
}

/// The saved registry: empty when the file is missing, an error when it exists and is not a JSON object.
fn read_sources(path: &Path) -> Result<Obj> {
    if !path.exists() {
        return Ok(Obj::new());
    }
    match util::read_json(path)? {
        Value::Object(m) => Ok(m),
        _ => bail!("{} is not a JSON object", path.display()),
    }
}

/// Registry of every source key ever seen -> task ids. Survives SP archiving and deletes.
fn known_sources(tasks: &[Value], path: &Path) -> Result<Obj> {
    let mut reg = read_sources(path)?;
    for t in tasks {
        let Some(k) = source_key(t.get("notes").and_then(Value::as_str)) else { continue };
        let ids = reg.entry(k).or_insert_with(|| json!([]));
        if !list(ids).contains(&t["id"])
            && let Some(ids) = ids.as_array_mut()
        {
            ids.push(t["id"].clone());
        }
    }
    Ok(reg)
}

/// Merges `reg` into what is on disk now, so ids another `sp-cli add` saved since our read stay.
fn save_sources(path: &Path, reg: &Obj) -> Result<()> {
    let mut merged = read_sources(path)?;
    for (key, ids) in reg {
        let have = merged.entry(key.clone()).or_insert_with(|| json!([]));
        for id in list(ids) {
            if !list(have).contains(id)
                && let Some(have) = have.as_array_mut()
            {
                have.push(id.clone());
            }
        }
    }
    util::write_atomic(path, util::dumps_indent(&Value::Object(merged), 0).as_bytes())?;
    Ok(())
}

fn cmd_add(a: &Parsed) -> Result<()> {
    let title = a.pos[0].as_str();
    let mut body = task_body(a, Some(title))?;
    let k = source_key(body.get("notes").and_then(Value::as_str));
    let tasks = if k.is_some() { get("/tasks?includeDone=true")? } else { json!([]) };
    let mut reg = if k.is_some() { known_sources(list(&tasks), &sources_path())? } else { Obj::new() };
    let parent = a.one("--parent").filter(|p| !p.is_empty());
    if let Some(k) = &k
        && reg.get(k).is_some_and(|ids| !list(ids).is_empty())
        && !a.has("--force")
    {
        let live: HashMap<String, &Value> = list(&tasks).iter().map(|t| (id_of(t), t)).collect();
        let hits: Vec<(String, Option<&Value>)> = list(&reg[k]).iter().map(util::text).map(|i| (i.clone(), live.get(&i).copied())).collect();
        for (i, t) in &hits {
            let state = match t {
                None => "gone",
                Some(t) if truthy(t, "isDone") => "done",
                Some(_) => "open",
            };
            eprintln!("exists {state} {i} {}", t.map(|t| util::text(&t["title"])).unwrap_or_default());
        }
        eprintln!("not created: same source as above; never reopen a closed one. --force only for a different ask");
        let open = hits.iter().find(|(_, t)| t.is_some_and(|t| !truthy(t, "isDone")));
        save_sources(&sources_path(), &reg)?;
        println!("{}", open.unwrap_or(hits.last().unwrap()).0);
        return Ok(());
    }
    body.insert("isIgnoreShortSyntax".into(), json!(true));
    body.entry("dueDay").or_insert(Value::Null); // the API otherwise defaults new tasks to today
    if let Some(p) = parent {
        body.insert("parentId".into(), json!(p));
        body.shift_remove("projectId");
        body.shift_remove("tagIds");
    }
    let body = Value::Object(body);
    let t = send("POST", "/tasks", body.clone())?;
    let want = body.get("tagIds").cloned().unwrap_or(json!([]));
    let id = id_of(&t);
    // SP adds the tag view open in the app (e.g. urgent) to new tasks; keep only what was asked
    if parent.is_none() && t.get("tagIds").cloned().unwrap_or(json!([])) != want {
        send("PATCH", &format!("/tasks/{id}"), json!({"tagIds": want}))?;
    }
    if let Some(k) = k {
        if let Some(ids) = reg.entry(k).or_insert_with(|| json!([])).as_array_mut() {
            ids.push(t["id"].clone());
        }
        save_sources(&sources_path(), &reg)?;
    }
    println!("{id}");
    if truthy(&body, "notes") {
        sync_links(&id)?;
    }
    Ok(())
}

/// Replace a task's notes and attach the links in them, as `sp-cli update --notes` does.
pub fn set_notes(id: &str, notes: &str) -> Result<()> {
    send("PATCH", &format!("/tasks/{id}"), json!({"notes": notes}))?;
    sync_links(id)
}

fn cmd_update(a: &Parsed) -> Result<()> {
    let id = &a.pos[0];
    let mut body = task_body(a, a.one("--title"))?;
    if a.has("--no-due") {
        body.insert("dueDay".into(), Value::Null);
        body.insert("dueWithTime".into(), Value::Null);
    }
    if body.is_empty() {
        bail!("Nothing to update.");
    }
    let notes = body.contains_key("notes");
    send("PATCH", &format!("/tasks/{id}"), Value::Object(body))?;
    println!("updated {id}");
    if notes {
        sync_links(id)?;
    }
    Ok(())
}

/// Where bare Jira keys in notes link to, and which project keys count as keys.
struct Jira {
    base: String,
    key: Regex,
}

/// `None` unless both are set and name at least one project key.
fn jira_from(base: Option<String>, keys: Option<String>) -> Option<Jira> {
    let keys: Vec<String> = keys?.split(',').map(|k| fancy_regex::escape(k.trim()).into_owned()).filter(|k| !k.is_empty()).collect();
    if keys.is_empty() {
        return None;
    }
    Some(Jira { base: base?, key: rx(&format!(r"(?i)\b(?:{})-\d+\b", keys.join("|"))) })
}

fn jira_from_env() -> Option<Jira> {
    jira_from(util::env_nonempty("SP_JIRA_BASE"), util::env_nonempty("SP_JIRA_KEYS"))
}

static URL: LazyLock<Regex> = LazyLock::new(|| rx(r#"https?://[^\s)\]>`'"|]+"#));
static LINK_TITLES: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
    [
        (r"(?i)/browse/([A-Z][A-Z0-9]+-\d+)", "{0}"),
        (r"(?i)bitbucket\.org/[^/]+/([^/]+)/pull-requests/(\d+)", "{0} PR {1}"),
        (r"(?i)github\.com/[^/]+/([^/]+)/pull/(\d+)", "{0} PR #{1}"),
        (r"(?i)atlassian\.net/wiki/.*/pages/\d+/([^/?#]+)", "Confluence: {0}"),
    ]
    .into_iter()
    .map(|(p, f)| (rx(p), f))
    .collect()
});

fn link_title(url: &str) -> String {
    for (i, (re, form)) in LINK_TITLES.iter().enumerate() {
        if let Some(m) = captures(re, url) {
            let groups = (1..m.len()).map(|g| m.get(g).map_or("", |x| x.as_str()));
            let groups: Vec<String> = groups.map(|g| if i == 0 { g.to_uppercase() } else { g.replace('+', " ") }).collect();
            return groups.iter().enumerate().fold(form.to_string(), |s, (n, g)| s.replace(&format!("{{{n}}}"), g));
        }
    }
    for (host, name) in [("slack.com", "Slack"), ("teams.microsoft.com", "Teams"), ("atlassian.net/wiki", "Confluence")] {
        if url.contains(host) {
            return name.into();
        }
    }
    url.split('/').nth(2).unwrap_or_default().into()
}

fn unique(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items.into_iter().filter(|i| seen.insert(i.clone())).collect()
}

/// (title, url) for every URL in the notes, plus a Jira link for each bare ticket key when `jira` is set.
fn links_from(notes: &str, jira: Option<&Jira>) -> Vec<(String, String)> {
    let mut urls = unique(find_all(&URL, notes).into_iter().map(|u| u.trim_end_matches(['.', ',', ';', ':']).to_owned()));
    let linked = urls.join(" ").to_uppercase();
    if let Some(jira) = jira {
        let keys = unique(find_all(&jira.key, notes).into_iter().map(str::to_uppercase));
        urls.extend(keys.into_iter().filter(|k| !linked.contains(&format!("/BROWSE/{k}"))).map(|k| format!("{}{k}", jira.base)));
    }
    urls.into_iter().take(MAX_LINKS).map(|u| (link_title(&u), u)).collect()
}

/// Notes links -> SP link attachments, so they open in one click. REST drops `attachments`,
/// so this goes through the bridge plugin (1.1+). Never fails the caller: warns and returns.
fn sync_links(tid: &str) -> Result<()> {
    let t = get(&format!("/tasks/{tid}"))?;
    let have: Vec<Value> = t.get("attachments").map(list).unwrap_or_default().to_vec();
    let paths: Vec<Option<&Value>> = have.iter().map(|x| x.get("path")).collect();
    let new: Vec<(String, String)> = links_from(field(&t, "notes"), jira_from_env().as_ref())
        .into_iter()
        .filter(|(_, u)| !paths.contains(&Some(&json!(u))))
        .collect();
    if new.is_empty() {
        return Ok(());
    }
    let stamp = now_ms();
    let mut want = have;
    want.extend(new.iter().enumerate().map(|(i, (title, u))| {
        json!({"id": format!("l{stamp}{i}"), "type": "LINK", "path": u, "title": title, "icon": "link"})
    }));
    let notes = util::dumps(&json!({"op": "setLinks", "taskId": tid, "attachments": want}));
    let cid = id_of(&send("POST", "/tasks", json!({"title": CMD_TITLE, "isIgnoreShortSyntax": true, "notes": notes}))?);
    for _ in 0..25 {
        std::thread::sleep(Duration::from_millis(200));
        if get(&format!("/tasks/{tid}"))?.get("attachments").map(list).unwrap_or_default().len() >= want.len() {
            eprintln!("linked {} {tid}", new.len()); // stdout stays the bare id for callers
            return Ok(());
        }
    }
    let left = leftover(&cid)?;
    let why = match &left {
        Some(l) => util::head(&l.get("notes").map(util::text).unwrap_or_default(), 120).to_owned(),
        None => "no reply".into(),
    };
    eprintln!(
        "links not attached to {tid}: bridge plugin 1.1 not installed or failed ({why}). Run sp-cli bridge-plugin and install the zip."
    );
    Ok(())
}

/// The bridge command task, deleted, if the plugin left it behind.
fn leftover(cid: &str) -> Result<Option<Value>> {
    let tasks = get("/tasks?includeDone=true")?;
    let left = list(&tasks).iter().find(|x| id_of(x) == cid).cloned();
    if left.is_some() {
        call("DELETE", &format!("/tasks/{cid}"), None)?;
    }
    Ok(left)
}

fn cmd_links(a: &Parsed) -> Result<()> {
    let ids = if a.pos.is_empty() {
        list(&get("/tasks")?).iter().filter(|t| truthy(t, "notes")).map(id_of).collect()
    } else {
        a.pos.clone()
    };
    ids.iter().try_for_each(|t| sync_links(t))
}

fn undated(t: &Value) -> bool {
    !truthy(t, "dueDay") && !truthy(t, "dueWithTime")
}

static ROLLED: LazyLock<Regex> = LazyLock::new(|| rx(r"(?m)^Rolled: (\d+)"));
static DECIDE: LazyLock<Regex> = LazyLock::new(|| rx(r"(?m)^Decide:"));

fn rolls(t: &Value) -> u64 {
    captures(&ROLLED, field(t, "notes")).map_or(0, |m| m[1].parse().unwrap_or(u64::MAX))
}

fn num(t: &Value, key: &str, default: f64) -> f64 {
    t.get(key).and_then(Value::as_f64).unwrap_or(default)
}

fn review_buckets<'a>(tasks: &'a [Value], waiting: &Value, deferred: &Value, now_ms: i64, ui: [&Value; 2]) -> Vec<(&'static str, Vec<&'a Value>)> {
    let now_f = now_ms as f64;
    let old = now_f - 14.0 * 86_400_000.0;
    let now = local(now_f).naive_local();
    let today = iso(now.date());
    let monday = now.date() - chrono::Days::new(now.weekday().num_days_from_monday().into());
    let week_start = local_ms(monday.and_hms_opt(0, 0, 0).unwrap()).unwrap_or(0) as f64;
    let (done, open): (Vec<&Value>, Vec<&Value>) = tasks.iter().partition(|t| truthy(t, "isDone"));
    let pick = |f: &dyn Fn(&Value) -> bool| open.iter().copied().filter(|t| f(t)).collect::<Vec<_>>();
    vec![
        ("waiting", pick(&|t| has_tag(t, waiting))),
        ("deferred, no date", pick(&|t| has_tag(t, deferred) && undated(t))),
        ("no date, older than 14 days", pick(&|t| undated(t) && num(t, "created", now_f) < old && !truthy(t, "repeatCfgId"))),
        ("Decide (rolled 3+ or Decide: line)", pick(&|t| rolls(t) >= 3 || matches(&DECIDE, field(t, "notes")))),
        (
            "Who set this? (repeating, overdue or rolled 3+)",
            pick(&|t| {
                let due = if truthy(t, "dueDay") { field(t, "dueDay") } else { "9" };
                truthy(t, "repeatCfgId") && (due < today.as_str() || rolls(t) >= 3)
            }),
        ),
        (
            "Friday test (urgent+important done this week)",
            done.into_iter().filter(|t| ui.iter().all(|u| has_tag(t, u)) && num(t, "doneOn", 0.0) >= week_start).collect(),
        ),
    ]
}

fn tell_lines(tasks: &[Value]) -> Vec<(&Value, &str)> {
    tasks
        .iter()
        .filter(|t| !truthy(t, "isDone"))
        .flat_map(|t| util::splitlines(field(t, "notes")).into_iter().filter(|l| l.starts_with("Tell ")).map(move |l| (t, l)))
        .collect()
}

fn cmd_review() -> Result<()> {
    let tasks = get("/tasks?includeDone=true")?;
    let tags = get("/tags")?;
    let projects = titles(&get("/projects")?);
    let names = titles(&tags);
    let ui = [resolve(&tags, "urgent", "tag")?, resolve(&tags, "important", "tag")?];
    let (waiting, deferred) = (resolve(&tags, "waiting", "tag")?, resolve(&tags, "deferred", "tag")?);
    for (head, items) in review_buckets(list(&tasks), &waiting, &deferred, now_ms(), [&ui[0], &ui[1]]) {
        println!("== {head} ({})", items.len());
        for t in items {
            println!("{}", fmt(t, &projects, &names));
        }
    }
    let tells = tell_lines(list(&tasks));
    println!("== Tell, not sent ({})", tells.len());
    for (t, line) in tells {
        println!("{}  {}  -> {line}", id_of(t), util::text(&t["title"]));
    }
    Ok(())
}

fn log_line(tasks: &[Value], focus: &Value, day: &str) -> String {
    let spent = |ts: &mut dyn Iterator<Item = &Value>| {
        ts.map(|t| t.get("timeSpentOnDay").and_then(|d| d.get(day)).and_then(Value::as_f64).unwrap_or(0.0)).sum::<f64>() / 3_600_000.0
    };
    let focused = spent(&mut tasks.iter().filter(|t| has_tag(t, focus)));
    let done = tasks
        .iter()
        .filter(|t| truthy(t, "isDone") && truthy(t, "doneOn") && iso(local(num(t, "doneOn", 0.0)).date_naive()) == day)
        .count();
    format!("| {day} | {focused:.1} | {:.1} | {done} |  |", spent(&mut tasks.iter()))
}

fn cmd_log(a: &Parsed) -> Result<()> {
    let day = a.one("--day").filter(|d| !d.is_empty()).map(String::from).unwrap_or_else(|| iso(Local::now().date_naive()));
    let tasks = get("/tasks?includeDone=true")?;
    let line = log_line(list(&tasks), &resolve(&get("/tags")?, "focus", "tag")?, &day);
    let path = util::env_nonempty("SP_DAILY_LOG")
        .ok_or_else(|| anyhow!("SP_DAILY_LOG is not set: point it at the daily log markdown file"))?;
    if !Path::new(&path).exists() {
        let head = "# Daily log\n\nWritten by `sp-cli log`. Focus = time tracked on #focus tasks. Evening hours: fill in by hand.\n\n| Day | Focus h | Tracked h | Done | Evening h |\n|:-|-:|-:|-:|-:|\n";
        std::fs::write(&path, head.replace('\n', NL))?;
    }
    std::fs::OpenOptions::new().append(true).open(&path)?.write_all(format!("{line}{NL}").as_bytes())?;
    println!("{line} -> {path}");
    Ok(())
}

fn cmd_repeat(a: &Parsed) -> Result<()> {
    let mut body = Obj::new();
    if a.has("--weekdays") {
        body.insert("quickSetting".into(), json!("MONDAY_TO_FRIDAY"));
    } else if a.has("--daily") {
        body.insert("quickSetting".into(), json!("DAILY"));
    } else {
        let picked: HashSet<String> =
            a.one("--days").unwrap_or_default().split(',').map(|d| d.trim().to_lowercase().chars().take(3).collect()).collect();
        body.insert("quickSetting".into(), json!("CUSTOM"));
        body.insert("repeatCycle".into(), json!("WEEKLY"));
        body.insert("repeatEvery".into(), json!(1));
        for d in DAYS {
            body.insert(d.into(), json!(picked.contains(&d[..3])));
        }
    }
    body.insert("taskId".into(), json!(a.pos[0]));
    println!("{}", id_of(&send("POST", "/task-repeat-cfgs", Value::Object(body))?));
    Ok(())
}

const BRIDGE_JS: &str = "const CMD = '@@sp-cmd';
const exists = (list, title) => list.some((i) => i.title.toLowerCase() === title.toLowerCase());
const OPS = {
  addProject: async ({ title }) => exists(await PluginAPI.getAllProjects(), title) || PluginAPI.addProject({ title }),
  addTag: async ({ title }) => exists(await PluginAPI.getAllTags(), title) || PluginAPI.addTag({ title }),
  setLinks: async ({ taskId, attachments }) => PluginAPI.updateTask(taskId, { attachments }),
};
PluginAPI.registerHook(PluginAPI.Hooks.TASK_CREATED, async ({ task }) => {
  if (!task || task.title !== CMD) return;
  try {
    const cmd = JSON.parse(task.notes || '{}');
    if (!OPS[cmd.op]) throw new Error('unknown op ' + cmd.op);
    await OPS[cmd.op](cmd);
    await PluginAPI.deleteTask(task.id);
  } catch (e) {
    await PluginAPI.updateTask(task.id, { title: CMD + ' failed', notes: String(e) });
  }
});
";

fn cmd_bridge_plugin(a: &Parsed) -> Result<()> {
    let manifest = json!({
        "id": "sp-cli-bridge", "name": "sp-cli bridge", "version": "1.1.0", "manifestVersion": 1,
        "minSupVersion": "19.0.0",
        "description": format!("Lets sp-cli create projects and tags and attach links: runs tasks titled {CMD_TITLE}, then deletes them."),
        "hooks": ["taskCreated"],
        "permissions": ["registerHook", "getAllProjects", "addProject", "getAllTags", "addTag", "updateTask", "deleteTask"],
        "iFrame": false, "isSkipMenuEntry": true,
    });
    let out = a.one("--out").unwrap_or("sp-cli-bridge.zip");
    let files = [("manifest.json", util::dumps_indent(&manifest, 2)), ("plugin.js", BRIDGE_JS.to_owned())];
    std::fs::write(out, stored_zip(&files))?;
    println!("{}", std::path::absolute(out)?.display());
    Ok(())
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// An uncompressed zip, as Python's `ZipFile.writestr` makes by default.
fn stored_zip(files: &[(&str, String)]) -> Vec<u8> {
    let now = Local::now().naive_local();
    let time = (now.hour() << 11 | now.minute() << 5 | (now.second() / 2)) as u16;
    let date = ((now.year() as u32 - 1980) << 9 | now.month() << 5 | now.day()) as u16;
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, text) in files {
        let (name, data) = (name.as_bytes(), text.as_bytes());
        let (crc, size, offset) = (crc32(data), data.len() as u32, out.len() as u32);
        let common = |buf: &mut Vec<u8>| {
            for v in [20u16, 0, 0, time, date] {
                buf.extend(v.to_le_bytes());
            }
            for v in [crc, size, size] {
                buf.extend(v.to_le_bytes());
            }
            buf.extend((name.len() as u16).to_le_bytes());
            buf.extend(0u16.to_le_bytes());
        };
        out.extend(0x0403_4b50u32.to_le_bytes());
        common(&mut out);
        out.extend(name);
        out.extend(data);
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend(20u16.to_le_bytes());
        common(&mut central);
        central.extend([0u8; 6]); // comment length, disk, internal attributes
        central.extend((0o600u32 << 16).to_le_bytes());
        central.extend(offset.to_le_bytes());
        central.extend(name);
    }
    let (start, len) = (out.len() as u32, central.len() as u32);
    out.extend(central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend(len.to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

fn bridge(op: &str, title: &str, path: &str) -> Result<()> {
    let same = |i: &&Value| field(i, "title").to_lowercase() == title.to_lowercase();
    if list(&get(path)?).iter().any(|i| same(&i)) {
        println!("exists {title}");
        return Ok(());
    }
    let notes = util::dumps(&json!({"op": op, "title": title}));
    let tid = id_of(&send("POST", "/tasks", json!({"title": CMD_TITLE, "notes": notes, "isIgnoreShortSyntax": true}))?);
    for _ in 0..25 {
        std::thread::sleep(Duration::from_millis(200));
        if let Some(hit) = list(&get(path)?).iter().find(same) {
            println!("{} {title}", id_of(hit));
            return Ok(());
        }
    }
    let why = match leftover(&tid)? {
        Some(l) if field(&l, "title") != CMD_TITLE => l.get("notes").map(util::text).unwrap_or_else(|| "None".into()),
        _ => "bridge plugin not installed or not enabled".into(),
    };
    bail!("Could not create '{title}': {why}")
}

/// The checks sp.py's selftest makes before it touches SP.
fn checks() {
    assert_eq!(due_fields(Some("2026-10-14"), None).unwrap(), json!({"dueDay": "2026-10-14"}).as_object().unwrap().clone());
    let f = due_fields(Some("2026-10-14"), Some("08:45")).unwrap();
    assert_eq!(local(f["dueWithTime"].as_f64().unwrap()).format("%H:%M").to_string(), "08:45");
    assert_eq!(resolve(&json!([{"id": "a", "title": "Focus"}]), "focus", "tag").unwrap(), json!("a"));
    let (w, d, ui) = (json!("w"), json!("d"), [json!("EM_URGENT"), json!("EM_IMPORTANT")]);
    let counts = |b: Vec<(&str, Vec<&Value>)>| b.into_iter().map(|(_, v)| v.len()).collect::<Vec<_>>();
    let (day_ms, now) = (86_400_000i64, 100 * 86_400_000i64);
    let tasks = json!([{"tagIds": ["w"], "dueDay": "2026-10-20"}, {"tagIds": ["d"]},
                       {"tagIds": ["d"], "dueDay": "2026-10-20"}, {"tagIds": [], "created": now - 20 * day_ms},
                       {"tagIds": [], "created": now - 20 * day_ms, "repeatCfgId": "r"},
                       {"tagIds": [], "created": now - day_ms}]);
    assert_eq!(counts(review_buckets(list(&tasks), &w, &d, now, [&ui[0], &ui[1]])), [1, 1, 1, 0, 0, 0]);
    let at = |d: u32, h: u32| local_ms(NaiveDate::from_ymd_opt(2026, if d > 20 { 10 } else { 11 }, d).unwrap().and_hms_opt(h, 0, 0).unwrap()).unwrap();
    let (now, wed, last_week) = (at(6, 16), at(4, 12), at(30, 12));
    let tasks = json!([{"tagIds": [], "notes": "src\nRolled: 3 (last 2026-11-05)"},
                       {"tagIds": [], "notes": "Decide: drop / defer - pick: drop"},
                       {"tagIds": [], "notes": "Rolled: 2 (last 2026-11-05)"},
                       {"tagIds": [], "repeatCfgId": "r", "dueDay": "2026-11-05", "created": now},
                       {"tagIds": [], "repeatCfgId": "r", "dueDay": "2026-11-06", "created": now},
                       {"tagIds": ["U", "I"], "isDone": true, "doneOn": wed},
                       {"tagIds": ["U", "I"], "isDone": true, "doneOn": last_week},
                       {"tagIds": ["I"], "isDone": true, "doneOn": wed}]);
    let (u, i) = (json!("U"), json!("I"));
    assert_eq!(counts(review_buckets(list(&tasks), &w, &d, now, [&u, &i]))[3..], [2, 1, 1]);
    let tells = json!([{"notes": "x\nTell Sam: \"Not this week\""}, {"isDone": true, "notes": "Tell Alex: x"}]);
    assert_eq!(tell_lines(list(&tells)).into_iter().map(|(_, l)| l).collect::<Vec<_>>(), ["Tell Sam: \"Not this week\""]);
    let done_ms = local_ms(NaiveDate::from_ymd_opt(2026, 10, 14).unwrap().and_hms_opt(15, 0, 0).unwrap()).unwrap();
    let tasks = json!([{"tagIds": ["f"], "timeSpentOnDay": {"2026-10-14": 5400000}, "isDone": true, "doneOn": done_ms},
                       {"tagIds": [], "timeSpentOnDay": {"2026-10-14": 1800000, "2026-10-13": 9e6}}]);
    assert_eq!(log_line(list(&tasks), &json!("f"), "2026-10-14"), "| 2026-10-14 | 1.5 | 2.0 | 1 |  |");
    let sk = |s: &str| source_key(Some(s));
    assert_eq!(sk("https://example.slack.com/archives/C1/p1792480288712159?thread_ts=1792480207.111589&cid=C1\n- x").as_deref(), Some("slack:C1:1792480207.111589"));
    assert_eq!(sk("https://example.slack.com/archives/C1/p1792480207111589").as_deref(), Some("slack:C1:1792480207.111589"));
    assert!(sk("https://example.slack.com/archives/D05M5SK8411").is_none() && sk("Filed X").is_none() && sk("").is_none());
    assert_eq!(
        sk("https://example.atlassian.net/browse/ABC-1?focusedCommentId=9&page=x").as_deref(),
        Some("https://example.atlassian.net/browse/abc-1?focusedCommentId=9")
    );
    let jira = jira_from(Some("https://example.atlassian.net/browse/".into()), Some("ABC,DEF,XYZ".into()));
    let lf = links_from("https://example.slack.com/archives/C1/p1\nABC-9 and https://example.atlassian.net/browse/ABC-1. see https://bitbucket.org/acme/app/pull-requests/6711 abc-1", jira.as_ref());
    let pair = |a: &str, b: &str| (a.to_owned(), b.to_owned());
    assert_eq!(
        lf,
        [
            pair("Slack", "https://example.slack.com/archives/C1/p1"),
            pair("ABC-1", "https://example.atlassian.net/browse/ABC-1"),
            pair("app PR 6711", "https://bitbucket.org/acme/app/pull-requests/6711"),
            pair("ABC-9", "https://example.atlassian.net/browse/ABC-9"),
        ]
    );
    assert!(links_from(r"C:\x\HANDOFF.md", jira.as_ref()).is_empty());
    assert!(links_from("ABC-9", None).is_empty());
    assert!(jira_from(Some("https://j/browse/".into()), None).is_none() && jira_from(None, Some("A".into())).is_none());
    assert_eq!(links_from("axb-1 a.b-3 AB-2", jira_from(Some("https://j/browse/".into()), Some("a.b, ,AB".into())).as_ref()).len(), 2);
}

fn selftest() -> Result<()> {
    std::panic::catch_unwind(checks).map_err(|_| anyhow!("selftest failed"))?;
    let title = "sp-cli selftest #x";
    let tid = id_of(&send("POST", "/tasks", json!({"title": title, "isIgnoreShortSyntax": true}))?);
    let path = format!("/tasks/{tid}");
    let live = || -> Result<()> {
        anyhow::ensure!(field(&get(&path)?, "title") == title, "title did not round-trip");
        let mut patch = due_fields(None, Some("09:00"))?;
        patch.insert("notes".into(), json!("n"));
        send("PATCH", &path, Value::Object(patch))?;
        anyhow::ensure!(field(&get(&path)?, "notes") == "n", "notes did not round-trip");
        send("PATCH", &path, json!({"isDone": true}))?;
        anyhow::ensure!(list(&get("/tasks")?).iter().all(|t| id_of(t) != tid), "a done task is still listed");
        Ok(())
    };
    let result = live();
    call("DELETE", &path, None)?;
    result?;
    println!("selftest ok");
    Ok(())
}

/// argparse's share of sp.py: options anywhere, `--opt=value`, unique prefixes, `--` ends options.
#[derive(Default)]
struct Parsed {
    pos: Vec<String>,
    opts: HashMap<&'static str, Vec<String>>,
}

impl Parsed {
    fn has(&self, k: &str) -> bool {
        self.opts.contains_key(k)
    }
    fn one(&self, k: &str) -> Option<&str> {
        self.opts.get(k).and_then(|v| v.last()).map(String::as_str)
    }
    fn all(&self, k: &str) -> Option<&Vec<String>> {
        self.opts.get(k)
    }
}

struct Spec {
    values: &'static [&'static str],
    flags: &'static [&'static str],
    pos: (usize, usize),
}

const fn spec(values: &'static [&'static str], flags: &'static [&'static str], min: usize, max: usize) -> Spec {
    Spec { values, flags, pos: (min, max) }
}

const TASK_OPTS: [&str; 6] = ["--project", "--tag", "--notes", "--notes-file", "--due", "--at"];
const ADD_OPTS: [&str; 7] = ["--parent", TASK_OPTS[0], TASK_OPTS[1], TASK_OPTS[2], TASK_OPTS[3], TASK_OPTS[4], TASK_OPTS[5]];
const UPDATE_OPTS: [&str; 7] = ["--title", TASK_OPTS[0], TASK_OPTS[1], TASK_OPTS[2], TASK_OPTS[3], TASK_OPTS[4], TASK_OPTS[5]];
const MANY: usize = usize::MAX;

fn spec_for(cmd: &str) -> Option<Spec> {
    Some(match cmd {
        "list" => spec(&["--project", "--tag", "--search"], &["--today", "--done"], 0, 0),
        "add" => spec(&ADD_OPTS, &["--force"], 1, 1),
        "update" => spec(&UPDATE_OPTS, &["--no-due"], 1, 1),
        "done" | "undone" | "delete" | "show" => spec(&[], &[], 1, 1),
        "repeat" => spec(&["--days"], &["--weekdays", "--daily"], 1, 1),
        "projects" | "tags" | "selftest" | "review" => spec(&[], &[], 0, 0),
        "log" => spec(&["--day"], &[], 0, 0),
        "add-project" | "add-tag" => spec(&[], &[], 1, MANY),
        "links" => spec(&[], &[], 0, MANY),
        "bridge-plugin" => spec(&["--out"], &[], 0, 0),
        _ => return None,
    })
}

fn parse(spec: &Spec, args: &[String]) -> std::result::Result<Parsed, String> {
    let mut p = Parsed::default();
    let names: Vec<&'static str> = spec.values.iter().chain(spec.flags).copied().collect();
    let mut it = args.iter();
    let mut options_done = false;
    while let Some(arg) = it.next() {
        if options_done || !arg.starts_with('-') || arg == "-" {
            p.pos.push(arg.clone());
            continue;
        }
        if arg == "--" {
            options_done = true;
            continue;
        }
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) => (f, Some(v.to_owned())),
            None => (arg.as_str(), None),
        };
        let hits: Vec<&'static str> = match names.iter().find(|n| **n == flag) {
            Some(n) => vec![n],
            None => names.iter().copied().filter(|n| flag.len() > 2 && n.starts_with(flag)).collect(),
        };
        let name = match hits[..] {
            [n] => n,
            [] => return Err(format!("unrecognized arguments: {arg}")),
            _ => return Err(format!("ambiguous option: {flag} could match {}", hits.join(", "))),
        };
        let entry = p.opts.entry(name).or_default();
        if spec.flags.contains(&name) {
            if let Some(v) = inline {
                return Err(format!("argument {name}: ignored explicit argument '{v}'"));
            }
        } else {
            match inline.or_else(|| it.next().cloned()) {
                Some(v) => entry.push(v),
                None => return Err(format!("argument {name}: expected one argument")),
            }
        }
    }
    let (min, max) = spec.pos;
    if p.pos.len() < min {
        return Err("the following arguments are required: ".to_owned() + if max == 1 { "id/title" } else { "titles" });
    }
    if p.pos.len() > max {
        return Err(format!("unrecognized arguments: {}", p.pos[max..].join(" ")));
    }
    Ok(p)
}

fn usage_error(cmd: &str, msg: &str) -> u8 {
    eprintln!("usage: sp-cli {cmd} ... (sp-cli -h for help)\nsp-cli {cmd}: error: {msg}");
    2
}

pub fn main(args: &[String]) -> u8 {
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        if args.is_empty() {
            return usage_error("", "the following arguments are required: cmd");
        }
        print!("{HELP}");
        return 0;
    }
    let cmd = args[0].as_str();
    let Some(spec) = spec_for(cmd) else { return usage_error("", &format!("argument cmd: invalid choice: '{cmd}'")) };
    let a = match parse(&spec, &args[1..]) {
        Ok(a) => a,
        Err(msg) => return usage_error(cmd, &msg),
    };
    if cmd == "repeat" && [a.has("--weekdays"), a.has("--daily"), a.has("--days")].iter().filter(|x| **x).count() != 1 {
        return usage_error(cmd, "exactly one of the arguments --weekdays --daily --days is required");
    }
    if cmd != "bridge-plugin" && token().is_none() {
        eprintln!("No SP token. Turn on SP Settings > Misc > Local REST API, or set SP_API_TOKEN.");
        return 1;
    }
    let id = || a.pos.first().cloned().unwrap_or_default();
    let done = |patch: Value, word: &str| -> Result<()> {
        println!("{} {word}", id_of(&send("PATCH", &format!("/tasks/{}", id()), patch)?));
        Ok(())
    };
    let ran = match cmd {
        "list" => cmd_list(&a),
        "add" => cmd_add(&a),
        "update" => cmd_update(&a),
        "done" => done(json!({"isDone": true}), "done"),
        "undone" => done(json!({"isDone": false}), "reopened"),
        "delete" => call("DELETE", &format!("/tasks/{}", id()), None).map(|r| println!("{} deleted", id_of(&r))),
        "show" => get(&format!("/tasks/{}", id())).map(|t| println!("{}", util::dumps_indent(&t, 1))),
        "repeat" => cmd_repeat(&a),
        "projects" | "tags" => get(&format!("/{cmd}")).map(|items| {
            for i in list(&items) {
                println!("{} {}", id_of(i), i.get("title").map(util::text).unwrap_or_else(|| "None".into()));
            }
        }),
        "add-project" => a.pos.iter().try_for_each(|t| bridge("addProject", t, "/projects")),
        "add-tag" => a.pos.iter().try_for_each(|t| bridge("addTag", t, "/tags")),
        "links" => cmd_links(&a),
        "bridge-plugin" => cmd_bridge_plugin(&a),
        "selftest" => selftest(),
        "review" => cmd_review(),
        "log" => cmd_log(&a),
        _ => unreachable!(),
    };
    match ran {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selftest_checks_pass() {
        checks();
    }

    fn task(id: &str, notes: &str) -> Value {
        json!({"id": id, "title": "t", "notes": notes})
    }

    #[test]
    fn known_sources_rejects_a_sources_file_that_does_not_parse() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sp-sources.json");
        std::fs::write(&f, "{\"https://a\": [\"1\"").unwrap();
        assert!(known_sources(&[task("2", "https://b/x")], &f).is_err());
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "{\"https://a\": [\"1\"");
    }

    #[test]
    fn known_sources_treats_a_missing_file_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let reg = known_sources(&[task("2", "https://b/x")], &dir.path().join("none.json")).unwrap();
        assert_eq!(reg["https://b/x"], json!(["2"]));
    }

    #[test]
    fn save_sources_merges_what_another_writer_saved_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sp-sources.json");
        std::fs::write(&f, r#"{"https://a": ["1"], "https://c": ["9"]}"#).unwrap();
        let mine: Obj = serde_json::from_str(r#"{"https://a": ["3"], "https://b": ["2"]}"#).unwrap();
        save_sources(&f, &mine).unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(saved, json!({"https://a": ["1", "3"], "https://c": ["9"], "https://b": ["2"]}));
    }

    #[test]
    fn save_sources_refuses_to_overwrite_a_file_that_does_not_parse() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sp-sources.json");
        std::fs::write(&f, "not json").unwrap();
        assert!(save_sources(&f, &Obj::new()).is_err());
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "not json");
    }

    #[test]
    fn floats_round_like_python() {
        assert_eq!(format!("{:.1} {:.1} {:.1}", 0.25, 0.75, 0.05), "0.2 0.8 0.1");
    }

    #[test]
    fn quote_matches_urllib() {
        assert_eq!(quote("a b/ü&x~"), "a%20b/%C3%BC%26x~");
    }

    #[test]
    fn crc32_matches_zlib() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn options_parse_like_argparse() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let p = parse(&spec_for("add").unwrap(), &s(&["T", "--tag", "a", "--tag=b", "--proj", "P", "--force"])).unwrap();
        assert_eq!((p.pos.clone(), p.all("--tag").cloned(), p.one("--project"), p.has("--force")), (s(&["T"]), Some(s(&["a", "b"])), Some("P"), true));
        assert!(parse(&spec_for("add").unwrap(), &s(&["T", "--note", "x"])).is_err());
        assert!(parse(&spec_for("add").unwrap(), &s(&[])).is_err());
        assert!(parse(&spec_for("list").unwrap(), &s(&["x"])).is_err());
    }
}
