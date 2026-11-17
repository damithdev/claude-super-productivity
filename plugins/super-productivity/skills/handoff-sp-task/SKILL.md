---
name: handoff-sp-task
description: Add or update the Super Productivity task for a handoff you just wrote, with the handoff path and the Continue with prompt in its notes. Use right after the handoff skill, or any other skill that writes HANDOFF*.md or */handoffs/*.md. The sp-cli handoff-check hook blocks the stop when this is missing.
---

# Handoff SP task

Every handoff gets a Super Productivity task. Then the work can restart from the todo list
without a search for the file. Do this after you write the handoff. Do not ask first.

1. Use the Write tool to save the notes to a scratchpad file. Content: the absolute path of the
   handoff. On the next line, `Supersedes: <parent's absolute path>` if there is a parent. Then a
   blank line. Then the **Continue with** block, word for word (not in a code fence). The forward-link
   edit changes the parent. `sp-cli hook handoff-check` flags each edited handoff when no SP task
   holds its path or when no prompt lines follow it. So put `Supersedes:` above the block, never last.
2. If an open task already tracks this work (`sp-cli list --project <P> --search <word>`), update
   it: `sp-cli update <id> --notes-file <file> --tag handoff <its other tags>`.
   `--tag` replaces all tags, so pass the tags it already has. Otherwise add one:
   `sp-cli add "<next step, imperative>" --project <P> --tag handoff --notes-file <file>`.
   `<P>` is the project where the existing tasks for this work live. If none, use `Inbox`.
3. Use `--notes-file`, never `--notes`. `--notes` turns `\n` into a newline and breaks paths like
   `C:\dev\node`. Keep the notes ASCII. An en dash or curly quote can come out garbled.
4. Run `sp-cli show <id>`. Check that the paths are correct. Say the task title in the reply.

If SP is not reachable, say that the task was not saved.
