---
name: handoff
description: Write a handoff file so a fresh session can continue this work, then add its Super Productivity task. Use on /super-productivity:handoff, or when the user asks to hand off, wrap up the session or write a handoff.
argument-hint: "What will the next session be used for?"
---

# Handoff

Write a handoff that lets a fresh agent with no context continue the work. Then give it an SP task.

## Where to write it

1. If the project has a `handoffs/` folder, write there. If it uses another place for handoffs
   (its CLAUDE.md says), write there.
2. Otherwise write in the root of the repo or folder you worked in.

A handoff is scratch state, not a deliverable. In a git repo, check that `.gitignore` or
`.git/info/exclude` covers `HANDOFF-*.md` and `handoffs/`. If neither does, say in the reply that the file is
untracked and must stay that way.

Name it `HANDOFF-<YYYY-MM-DD>-<slug>.md`. Leave earlier handoffs in place.

## Chain

1. **Parent**: the handoff this session started from (its `Read first:` path), else the newest
   `HANDOFF-*.md` in the same folder. None: this is the first in the chain.
2. Start the body with `Supersedes: <parent file name>` (first in chain: `Supersedes: none`).
3. After you write the file, add `Superseded by: <this file name>` under the parent's `Supersedes`
   line. That is the only edit an earlier handoff gets.

## Sections

Always these, in this order:

- **Where things stand**: what is done, what is in progress.
- **Blocking or undecided**: name each decision and whose it is.
- **Next steps**: in order.
- **Pointers**: paths to specs, plans, tickets, commits, PRs. Link to them; don't copy them.
- **Learnings**: one bullet per lesson that would change how the next session works (a gotcha, a
  wrong assumption and what disproved it). Nothing learned: one bullet that says so.
- **Continue with**: one fenced block for the user to paste into the fresh session:
  1. `Read first: <absolute path of this handoff>`
  2. Two to four lines: where to pick up, what the user already approved, the working folder.

Print the Continue with block in the reply too, after the handoff path.

## Before you save

- Check that every path you cite exists. Mark anything not checked as unverified.
- Remove API keys, passwords, tokens and personal data.
- Keep it short enough to read into a fresh session.

If the user passed arguments, they say what the next session is for. If not, infer it and state
the inference in one line at the top.

## SP task

Then run the `handoff-sp-task` skill for this file, without asking. The plugin's Stop hook blocks
the end of the session while a handoff has no SP task.
