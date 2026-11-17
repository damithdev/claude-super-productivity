//! sp-cli: `sp-cli <sp command> [args]` (`sp-cli -h`), `sp-cli hook handoff-check`, `sp-cli ensure`.

use std::process::ExitCode;

use sp_cli::{ensure, run_hook, sp};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("hook") => run_hook(args.get(1).map(String::as_str)),
        Some("ensure") => ensure::ensure(&args[1..]),
        _ => sp::main(&args),
    };
    ExitCode::from(code)
}
