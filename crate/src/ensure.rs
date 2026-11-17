//! `sp-cli ensure [--consent] [--now]`: installs and updates the Super Productivity app through
//! winget (Windows) or Homebrew (macOS). Runs from the SessionStart hook at most once a day.
//! Installs only after `--consent`; updates only while SP is closed. Always exits 0: a problem is
//! one warning line, never a blocked session.

use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::{sp, util};

const WINGET_ID: &str = "JohannesMillan.superProductivity";
const CASK: &str = "super-productivity";
const RELEASES: &str = "https://github.com/johannesjo/super-productivity/releases";
const SETUP: &str = "/super-productivity:sp-setup";

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Os {
    Windows,
    Mac,
    Other,
}

/// Exit code and stdout of a command; None when it could not start.
pub type Runner<'a> = &'a mut dyn FnMut(&str, &[&str]) -> Option<(i32, String)>;

pub struct Ctx {
    pub os: Os,
    pub consent: bool,
    pub token_present: bool,
    /// Something accepts connections on the REST API port.
    pub api_listening: bool,
    /// macOS only: an app bundle exists that Homebrew does not manage.
    pub mac_app_present: bool,
}

/// What to print. Pure apart from the commands it runs, so tests drive it with a fake runner.
pub fn decide(ctx: &Ctx, run: Runner) -> Vec<String> {
    let mut out = Vec::new();
    let (installed, outdated, running, install, upgrade): (bool, bool, bool, Vec<&str>, Vec<&str>) = match ctx.os {
        Os::Other => return vec![format!("sp-cli ensure: no winget or Homebrew on this OS; install SP from {RELEASES}")],
        Os::Windows => {
            let Some((code, list)) = run("winget", &["list", "--id", WINGET_ID, "-e", "--accept-source-agreements"]) else {
                return vec![format!("sp-cli ensure: winget not found; install SP from {RELEASES}")];
            };
            let installed = code == 0 && list.contains(WINGET_ID);
            let outdated = installed
                && run("winget", &["list", "--id", WINGET_ID, "-e", "--upgrade-available", "--accept-source-agreements"])
                    .is_some_and(|(c, o)| c == 0 && o.contains(WINGET_ID));
            let running = run("tasklist", &["/FI", "IMAGENAME eq Super Productivity.exe", "/NH"])
                .is_some_and(|(_, o)| o.contains("Super Productivity.exe"));
            let common = ["--id", WINGET_ID, "-e", "--silent", "--accept-source-agreements", "--accept-package-agreements"];
            (installed, outdated, running, [&["install"][..], &common].concat(), [&["upgrade"][..], &common].concat())
        }
        Os::Mac => {
            let Some((code, _)) = run("brew", &["list", "--cask", CASK]) else {
                return vec![format!("sp-cli ensure: Homebrew not found; install SP from {RELEASES}")];
            };
            let running = run("pgrep", &["-x", "Super Productivity"]).is_some_and(|(c, _)| c == 0);
            if code != 0 && ctx.mac_app_present {
                // Installed by hand: brew install would fail on the existing app, and brew can't upgrade it.
                return api_hint(ctx, running);
            }
            let installed = code == 0;
            let outdated = installed && run("brew", &["outdated", "--cask", CASK]).is_some_and(|(_, o)| o.contains(CASK));
            (installed, outdated, running, vec!["install", "--cask", CASK], vec!["upgrade", "--cask", CASK])
        }
    };
    let tool = if ctx.os == Os::Windows { "winget" } else { "brew" };
    if !installed {
        if !ctx.consent {
            return vec![format!("Super Productivity is not installed. Run {SETUP} to install it.")];
        }
        match run(tool, &install) {
            Some((0, _)) => out.push("Super Productivity installed. Open it once to finish setup.".into()),
            r => return vec![format!("sp-cli ensure: install failed ({})", failure(r))],
        }
    } else if outdated {
        if !ctx.consent {
            out.push(format!("A Super Productivity update is available. Run {SETUP} to turn on auto-update."));
        } else if running {
            out.push("A Super Productivity update is available. It installs at the next session start while SP is closed.".into());
        } else {
            match run(tool, &upgrade) {
                Some((0, _)) => out.push(format!("Super Productivity updated. {UPDATED_NOTE}")),
                r => out.push(format!("sp-cli ensure: update failed ({})", failure(r))),
            }
        }
    }
    out.extend(api_hint(ctx, running));
    out
}

fn failure(r: Option<(i32, String)>) -> String {
    r.map_or("could not start".into(), |(c, _)| format!("exit {c}"))
}

// Updates can switch the setting off (19.2.0 -> 19.3.1 did).
const UPDATED_NOTE: &str = "After you open it, check that Settings > Misc > \"Local REST API\" is still on.";

/// No token yet, or SP is open with the API port closed.
fn api_hint(ctx: &Ctx, running: bool) -> Vec<String> {
    if ctx.token_present && (ctx.api_listening || !running) {
        return vec![];
    }
    vec!["SP REST API is off: in Super Productivity open Settings > Misc and switch on \"Local REST API\".".into()]
}

fn api_listening() -> bool {
    let base = sp::base();
    let host = base.trim_start_matches("http://").split('/').next().unwrap_or_default();
    host.to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .is_some_and(|a| TcpStream::connect_timeout(&a, Duration::from_secs(2)).is_ok())
}

fn data_dir() -> PathBuf {
    util::state_dir().join("sp-cli")
}

fn real_run(cmd: &str, args: &[&str]) -> Option<(i32, String)> {
    let out = Command::new(cmd).args(args).output().ok()?;
    Some((out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned()))
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// True unless a run already finished cleanly today (`--now` always runs).
fn due(stamp: &Path, now: bool) -> bool {
    now || !std::fs::read_to_string(stamp).is_ok_and(|d| d.trim() == today())
}

/// Stamps today only when no step failed, so a failed install or update retries at the next session start.
fn finish(stamp: &Path, lines: &[String]) {
    if !lines.iter().any(|l| l.starts_with("sp-cli ensure:")) {
        let _ = util::write_atomic(stamp, today().as_bytes());
    }
}

pub fn ensure(args: &[String]) -> u8 {
    let dir = data_dir();
    let consent_file = dir.join("consent");
    if args.iter().any(|a| a == "--consent") {
        return match std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&consent_file, today())) {
            Ok(()) => {
                println!("Consent saved: sp-cli installs and auto-updates Super Productivity. SP_AUTO_UPDATE=0 turns updates off.");
                0
            }
            Err(e) => {
                println!("sp-cli ensure: could not save consent in {}: {e}", dir.display());
                0
            }
        };
    }
    if util::env_nonempty("SP_AUTO_UPDATE").as_deref() == Some("0") {
        return 0;
    }
    let _ = std::fs::create_dir_all(&dir);
    let stamp = dir.join("last-ensure");
    if !due(&stamp, args.iter().any(|a| a == "--now")) {
        return 0;
    }
    let os = if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::Mac
    } else {
        Os::Other
    };
    let ctx = Ctx {
        os,
        consent: consent_file.exists(),
        token_present: sp::token().is_some(),
        api_listening: api_listening(),
        mac_app_present: Path::new("/Applications/Super Productivity.app").exists(),
    };
    let lines = decide(&ctx, &mut real_run);
    for line in &lines {
        println!("{line}");
    }
    finish(&stamp, &lines);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(os: Os, consent: bool) -> Ctx {
        Ctx { os, consent, token_present: true, api_listening: true, mac_app_present: false }
    }

    fn calls_of(answers: Vec<(&'static str, &'static str, Option<(i32, &'static str)>)>, c: &Ctx) -> (Vec<String>, Vec<String>) {
        let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let log = calls.clone();
        let mut f = move |cmd: &str, args: &[&str]| {
            let line = format!("{cmd} {}", args.join(" "));
            log.borrow_mut().push(line.clone());
            answers.iter().find(|(k, a, _)| *k == cmd && (a.is_empty() || args.first() == Some(a))).and_then(|(_, _, r)| r.map(|(c, o)| (c, o.to_owned())))
        };
        let out = decide(c, &mut f);
        let calls = calls.borrow().clone();
        (out, calls)
    }

    const LISTED: Option<(i32, &str)> = Some((0, "Super Productivity JohannesMillan.superProductivity 19.2.0"));
    const NONE: Option<(i32, &str)> = Some((0, ""));

    #[test]
    fn decide_when_not_installed_without_consent_should_only_point_to_setup() {
        let (out, calls) = calls_of(vec![("winget", "list", Some((1, "No installed package")))], &ctx(Os::Windows, false));
        assert_eq!(out, vec![format!("Super Productivity is not installed. Run {SETUP} to install it.")]);
        assert!(!calls.iter().any(|c| c.starts_with("winget install")));
    }

    #[test]
    fn decide_when_not_installed_with_consent_should_install_silently() {
        let (out, calls) = calls_of(
            vec![("winget", "install", Some((0, ""))), ("winget", "list", Some((1, ""))), ("tasklist", "", NONE)],
            &ctx(Os::Windows, true),
        );
        assert!(calls.iter().any(|c| c.starts_with("winget install --id JohannesMillan.superProductivity -e --silent")));
        assert_eq!(out[0], "Super Productivity installed. Open it once to finish setup.");
    }

    #[test]
    fn decide_when_outdated_and_running_should_not_upgrade() {
        let (out, calls) = calls_of(
            vec![("winget", "list", LISTED), ("tasklist", "", Some((0, "Super Productivity.exe  1234 Console")))],
            &ctx(Os::Windows, true),
        );
        assert!(!calls.iter().any(|c| c.starts_with("winget upgrade")));
        assert!(out[0].contains("installs at the next session start"));
    }

    #[test]
    fn decide_when_outdated_and_closed_should_upgrade() {
        let (out, calls) = calls_of(
            vec![("winget", "upgrade", Some((0, ""))), ("winget", "list", LISTED), ("tasklist", "", Some((0, "INFO: No tasks")))],
            &ctx(Os::Windows, true),
        );
        assert!(calls.iter().any(|c| c.starts_with("winget upgrade --id JohannesMillan.superProductivity")));
        assert_eq!(out, vec![format!("Super Productivity updated. {UPDATED_NOTE}")]);
    }

    #[test]
    fn decide_when_outdated_without_consent_should_only_notify() {
        let (out, calls) = calls_of(vec![("winget", "list", LISTED), ("tasklist", "", NONE)], &ctx(Os::Windows, false));
        assert!(!calls.iter().any(|c| c.starts_with("winget upgrade")));
        assert!(out[0].contains("update is available"));
    }

    #[test]
    fn decide_when_winget_missing_should_point_to_releases() {
        let (out, _) = calls_of(vec![], &ctx(Os::Windows, true));
        assert_eq!(out, vec![format!("sp-cli ensure: winget not found; install SP from {RELEASES}")]);
    }

    #[test]
    fn decide_when_mac_app_installed_by_hand_should_not_call_brew_install() {
        let c = Ctx { mac_app_present: true, ..ctx(Os::Mac, true) };
        let (out, calls) = calls_of(vec![("brew", "list", Some((1, "")))], &c);
        assert!(out.is_empty());
        assert_eq!(calls, vec!["brew list --cask super-productivity".to_string(), "pgrep -x Super Productivity".to_string()]);
    }

    #[test]
    fn decide_when_mac_outdated_and_closed_should_brew_upgrade() {
        let (out, calls) = calls_of(
            vec![("brew", "list", Some((0, ""))), ("brew", "outdated", Some((0, "super-productivity"))), ("brew", "upgrade", Some((0, ""))), ("pgrep", "", Some((1, "")))],
            &ctx(Os::Mac, true),
        );
        assert!(calls.contains(&"brew upgrade --cask super-productivity".to_string()));
        assert_eq!(out, vec![format!("Super Productivity updated. {UPDATED_NOTE}")]);
    }

    #[test]
    fn decide_when_token_missing_should_name_the_setting() {
        let c = Ctx { token_present: false, ..ctx(Os::Windows, true) };
        let (out, _) = calls_of(vec![("winget", "list", Some((0, "JohannesMillan.superProductivity"))), ("tasklist", "", NONE)], &c);
        assert!(out.iter().any(|l| l.contains("Settings > Misc") && l.contains("Local REST API")));
    }

    #[test]
    fn decide_when_running_with_api_port_closed_should_name_the_setting() {
        let c = Ctx { api_listening: false, ..ctx(Os::Windows, true) };
        let installed = ("winget", "list", Some((0, "JohannesMillan.superProductivity")));
        let (out, _) = calls_of(vec![installed, ("tasklist", "", Some((0, "Super Productivity.exe  1234 Console")))], &c);
        assert!(out.iter().any(|l| l.contains("Local REST API")));
        let (out, _) = calls_of(vec![installed, ("tasklist", "", Some((0, "INFO: No tasks")))], &c);
        assert!(!out.iter().any(|l| l.contains("is off")), "a closed SP has no port to check");
    }

    #[test]
    fn due_when_clean_run_today_should_skip_unless_now_or_failed() {
        let dir = tempfile::tempdir().unwrap();
        let stamp = dir.path().join("last-ensure");
        assert!(due(&stamp, false));
        finish(&stamp, &["sp-cli ensure: update failed (exit 1)".into()]);
        assert!(due(&stamp, false), "a failed run retries");
        finish(&stamp, &["Super Productivity updated.".into()]);
        assert!(!due(&stamp, false));
        assert!(due(&stamp, true));
    }
}
