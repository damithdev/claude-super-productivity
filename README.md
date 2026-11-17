# claude-super-productivity

A Claude Code plugin that makes [Super Productivity](https://super-productivity.com/) (SP) your one todo list. Claude adds, updates and closes your tasks in SP. Every session handoff gets its own task, so you can pick up work from the todo list.

It ships `sp-cli`, a prebuilt Rust CLI for SP's local REST API. You don't need a Rust toolchain.

Platforms: Windows x64, macOS arm64, macOS x64.

## Install

In Claude Code:

```
/plugin marketplace add damithdev/claude-super-productivity
/plugin install super-productivity@claude-super-productivity
```

Restart Claude Code, then run:

```
/super-productivity:sp-setup
```

Setup asks before it installs anything. It does these steps:

1. Installs SP, or updates it, through winget (Windows) or Homebrew (macOS).
2. Checks that SP's local REST API is on. If it is off, open SP, go to **Settings > Misc** and switch on **Local REST API**.
3. Writes an optional SP plugin zip (the bridge). Load it once under **Settings > Plugins** to let `sp-cli` create projects and tags. All other commands work without it.
4. Offers a backup through SP's **Local file** sync, pointed at a folder in your cloud drive. Use it from one computer only.

### Install for a team or project

To enable the plugin for everyone on a repo, add this to the project's `.claude/settings.json`:

```json
{
  "extraKnownMarketplaces": {
    "claude-super-productivity": {
      "source": { "source": "github", "repo": "damithdev/claude-super-productivity" }
    }
  },
  "enabledPlugins": {
    "super-productivity@claude-super-productivity": true
  }
}
```

Each person still runs `/super-productivity:sp-setup` once on their own machine.

## What you get

### Skills

- `sp-setup`: first-run setup (above).
- `sp-todo`: SP's Inbox is the only todo list. Each task's first note line is its source link. Closed tasks stay closed.
- `handoff`: writes `HANDOFF-<date>-<slug>.md` with a **Continue with** block to paste into a fresh session. Then it adds the SP task.
- `handoff-sp-task`: adds or updates the SP task for a handoff. It works with any skill that writes `HANDOFF*.md` or `handoffs/*.md` files.

### Hooks

- **SessionStart**: `sp-cli ensure` runs at most once a day, in the background. It updates SP when you have given consent and SP is closed. It warns when SP is open with its REST API off.
- **Stop**: `sp-cli hook handoff-check` blocks the end of a session once when a handoff written in that session has no SP task. A second stop always goes through. If SP is not running, it only warns.

`SP_HOOKS=off` turns off the Stop hook. `SP_AUTO_UPDATE=0` turns off the SessionStart check.

### sp-cli

The plugin puts `sp-cli` on Claude's path. Commands: `list`, `add`, `update`, `done`, `undone`, `delete`, `show`, `repeat`, `projects`, `tags`, `review`, `log`, `add-project`, `add-tag`, `links`, `bridge-plugin`, `selftest`, `ensure`. Run `sp-cli -h` for details.

`sp-cli add` skips duplicates. When the first note line is a link that an earlier task already used, it prints that task's id and creates nothing.

## Configuration

All settings are optional environment variables. Set them in the `env` block of your Claude Code `settings.json`.

| Variable | Use |
|---|---|
| `SP_API_URL` | SP REST API address. Default `http://127.0.0.1:3876`. |
| `SP_API_TOKEN` | API token. Overrides SP's own token file. |
| `SP_JIRA_BASE` | Jira browse URL, e.g. `https://example.atlassian.net/browse/`. With `SP_JIRA_KEYS`, ticket keys in notes become task links. |
| `SP_JIRA_KEYS` | Jira project keys to link, comma-separated, e.g. `ABC,DEF`. |
| `SP_DAILY_LOG` | Markdown file that `sp-cli log` appends a daily line to. |
| `SP_STATE_DIR` | Folder for the duplicate registry and setup state. Default `~/.claude/state`. |
| `SP_AUTO_UPDATE=0` | Turn off the SessionStart check (no SP updates, no API warning). |
| `SP_HOOKS=off` | Turn off the Stop hook. |

## Use it in your own workflow

- Other skills can add todos by calling `sp-cli add "<title>" --project Inbox --notes "<source link>"`. Put the source link on the first notes line so that duplicates are found.
- Your own handoff skill works with the Stop hook when it writes `HANDOFF*.md` or `handoffs/*.md` files and then runs `handoff-sp-task`.
- To tell Claude about the plugin in every session, add a line like this to your `CLAUDE.md`: `Todos go to Super Productivity through the super-productivity plugin (sp-todo skill).`

## Build from source

```
cd crate
cargo test
cargo build --release
```

A version tag (`v*`) runs the release workflow. It builds the three binaries, commits them to `plugins/super-productivity/dist/`, and sets the plugin version.

## License

MIT. Super Productivity is a separate project with its own licence.
