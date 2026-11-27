# git — architecture

Two readers of one repository: the statusline's count of a working tree, and the file tools' way of committing what a session wrote.

## Files

1. `mod.rs` — module declarations and the re-exports.
2. `status.rs` — a directory's branch and how many files it changed, and which of a set of paths already carry a change.
3. `repo.rs` — the repository above a directory, how a file is named against its root, and what git knows of one file: tracked, ignored, and whether the user's own uncommitted change sits in it.
4. `stage.rs` — one `add` and one commit by path, so the index's other entries stay the user's.
5. `run.rs` — one git invocation in the root's own frame, and its first line of complaint.

## Relations

1. **Consumed by**: `ui::tui`'s statusline for `snapshot`; `fs/commit` and `fs/write`, `fs/edit` and `fs/splice` for the rest.
2. **Asks `platform`**: one tree per git call (`proc::Process`), so a helper git forks cannot outlive the deadline.
3. **Knows nothing about**: tools, sessions, the model, or any layer above it. It runs `git` and reads what came back.
4. **The front end and the backend both name it, and neither names the other.**

## Must stay in step

1. The paths every function takes are named against the repository root, with slashes; a caller holding a real path asks `relative` first, and a path outside the tree has no name.
2. A commit names its paths; it never commits the index, because the user's staged work is theirs and a commit that swept it up would put it under the model's message.