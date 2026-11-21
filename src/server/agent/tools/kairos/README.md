# kairos

A long command or a text note scheduled for later: the call starts a job and returns, and a
finished job wakes the session. A slow command therefore does not hold a round open.

## 1. Design / Schema

### One call

| field | value |
|---|---|
| `type` | `now`, `after`, `every`, `list`, `status`, `result`, `cancel` |
| `tasks` | `now`/`after`/`every`: one item per task, with `content`, `kind` (`command`/`text`, default `command`), `timeout`, `delay`, `expr`, `count` (default 3) |
| `id` | `status`/`result`/`cancel`: the job to ask for |

`now`, `after` and `every` take an array; `list`, `status`, `result` and `cancel` are query
operations and refuse one. A text job has no completion of its own, so it refuses `now`.

Every command is admitted at call time through `shell::admit` and the admitted line is frozen, so a
job exists only for a command that already passed the guard. The guard sees the final text once,
after aliases and `#ref` are resolved.

### Jobs

- A job is created by the loop, never by a tool. The tool writes a `contract::job::JobSpec` and
  holds the session's `JobHost`; the scheduler, the pool, the per-run files and the wakes live in
  `loop/job`.
- A session's service is built the first time anything asks for it, so a session that never runs a
  job keeps no scheduler and no output folder.
- One job, one completion: a command finishes, a child submits, or the clock arrives. The receipt
  names the job; the wake reports it done under the same id.
- A starting operation answers one receipt per task, so its memo is an array.

### Answers

`list`, `status`, `result` and `cancel` answer JSON, and the same value rides along as the memo.
`now`, `after` and `every` answer the receipts they started. The history summary names the
operation; a `kairos` call's own parameters are the interesting part.

A task that runs a session of its own carries that session's `label`, the stem of its answer file.
`status` also counts the round trips it has billed: one per block carrying a usage stamp, which is
the only figure that says what a subagent has actually cost.

## 2. Design intent

- **Long work leaves the round.** A job is started and forgotten until its wake; the model's turn
  is not held open by a slow command.
- **Admission happens once.** The command is admitted before the job exists, so a blocked command
  never starts later through the clock.
- **One completion per job.** A command finishing, a child submitting and a clock arriving are the
  same event to the caller: one receipt, one wake.

## 3. Unresolved

1. A `kairos` call in a sub-session fails at call time: a child has no job service, and the roster
   does not screen for it.

## 4. Config fields

| key | use |
|---|---|
| `config.server.agent.tools.kairos.maxConcurrent` | jobs one session may run at once |
| `config.server.agent.tools.kairos.timeoutSecs` | one job's deadline |
| `config.server.agent.tools.kairos.maxOutputBytes` | retained output per job |
| `config.server.agent.tools.kairos.maxRetained` | finished jobs kept for `result` |

The numbers are read by the loop, not by the tool.