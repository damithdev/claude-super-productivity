//! Runs sp-cli as a subprocess. It never reaches the real SP or the real registry: the API URL
//! points at a closed port and the registry folder is a temp folder, unless a test overrides them.
#![allow(dead_code)]

use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Debug, PartialEq)]
pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn sp_cli(args: &[&str], stdin: &str, env: &[(&str, &str)]) -> Out {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sp-cli"));
    cmd.args(args)
        .env("SP_HOOKS", "")
        .env("SP_API_URL", "http://127.0.0.1:9")
        .env("SP_STATE_DIR", std::env::temp_dir().join("sp-cli-tests"))
        .env_remove("SP_DAILY_LOG")
        .env_remove("SP_JIRA_BASE")
        .env_remove("SP_JIRA_KEYS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn");
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let text = |b: &[u8]| String::from_utf8_lossy(b).replace("\r\n", "\n");
    Out { code: out.status.code().unwrap_or(-1), stdout: text(&out.stdout), stderr: text(&out.stderr) }
}

pub fn temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}
