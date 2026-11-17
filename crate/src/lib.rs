//! Super Productivity CLI (`sp`) and the handoff Stop hook (`handoff_check`).

use std::panic::{self, AssertUnwindSafe};

pub mod ensure;
pub mod handoff_check;
pub mod sp;
mod util;

/// `sp-cli hook <name>`: the JSON payload comes on stdin. Exit 0 allows, exit 2 blocks with the
/// message on stderr. SP_HOOKS=off, an unknown name or a panic all exit 0, so a broken hook can
/// never block a stop.
pub fn run_hook(name: Option<&str>) -> u8 {
    if util::hooks_off() {
        return 0;
    }
    let Some("handoff-check") = name else { return 0 };
    let raw = util::read_stdin();
    panic::set_hook(Box::new(|info| eprintln!("sp-cli hook failed: {info}")));
    panic::catch_unwind(AssertUnwindSafe(|| handoff_check::run(&raw))).unwrap_or(0)
}
