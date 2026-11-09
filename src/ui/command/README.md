# command — the command vocabulary

## 1. Design / Schema

```text
CommandMeta { name, aliases, args, detail: Text }
Text { en, zh }
Arg = Path | Model | Profile | Number | Text
Candidate { value, detail }

lookup(table, name)        -> Option<&CommandMeta>   // name or alias, with its slash
split(table, input)        -> Option<(&CommandMeta, &str)>   // one line, name + argument text
valid(table)               -> bool                   // the table rules, checked at compile time
```

- `name` and every alias start with `/`; names never collide with names or aliases; both halves of
  `detail` are never empty.
- `args` is the command's argument positions in order. `Text` may only be last.
- A candidate carries the text one level inserts and one line of gray text. There is no "more"
  flag: a level has a next one when its inserted text ends where the next level starts — a `/` for a
  directory, a space for a command that takes arguments.
- The table lives in the front end, not here. `valid` is called from that table's own `const`.

## 2. Design Intent

**Vocabulary, not behavior.** This file names no command and runs nothing. Parsing, local actions
and completion candidates belong to the front end that shows them; the server has no notion of a
command at all, only doors.

**One table per front end.** A slash command is a front-end word: `/q` and `/resume` mean nothing to
a future bridge, while `/switch` means the same thing everywhere. A front end declares what it can
do; the shared part is the shape of the declaration.

**Argument kinds feed completion and help.** A position names where its candidates come from, so the
popup and the help line are read off the same declaration; nothing re-states an argument's kind in
the parser.

## 3. Unresolved

1. `detail` is only read by the completion popup; there is no help command yet.
2. The TUI is the only front end with a table, so no second declaration exercises the contract.

## 4. Config Fields

None.