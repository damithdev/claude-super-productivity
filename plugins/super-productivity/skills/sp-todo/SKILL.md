---
name: sp-todo
description: Use Super Productivity as the one todo list. Use whenever a skill, digest, meeting summary or handoff produces todo items, or when the user asks to add, update, close or list tasks.
---

# One todo list: Super Productivity Inbox

Super Productivity (SP) is the only todo list. Add every todo item to the SP **Inbox**:

`sp-cli add "<title>" --project Inbox --notes "<link + context>"`

- Never make a second list. No other todo app, no markdown todo file, no list left only in chat.
- Each item holds the link to its source (chat thread, ticket key, email) as the first line of its
  notes. `sp-cli add` uses that link to find duplicates and prints the existing id instead of
  a new task. Use `--force` only for a different ask on the same source.
- Closed is final. Never run `sp-cli undone` on, or re-create, a task the user closed. A new ask on
  the same source gets its own new task.
- Every handoff gets an SP task in the same turn, without asking. Use the `handoff-sp-task` skill.
- Do not copy tickets or pull requests into SP as tasks. Link them in a task only when the user
  owns an action on them.
- If `sp-cli` says SP is not reachable, list the items in chat and say they were not saved.
