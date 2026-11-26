# git

The working tree as two readers need it: the statusline's branch and counts, and the file tools'
commit.

## 1. Design / Schema

- `snapshot(dir)` → the branch, the files needing `add`, and the files already staged. `None` when
  the directory is in no repository, or git is not installed.
- `changed(root, paths)` → which of these paths already carry a change.
- `root(dir)` → the repository above a directory; `relative(root, file)` → how a file is named
  against it.
- `facts(file)` → `(tracked, dirty)`: whether git holds the file, and whether an uncommitted change
  of the user's already sat in it.
- `ignored(root, path)` → whether a rule refuses the path.
- `add(root, paths)` and `record(root, message, paths)`: one commit of exactly those paths.

## 2. Design intent

- **A commit by path, never the index.** The user may have staged work of their own; a plain
  `git commit` would record it under the model's message. Naming the paths keeps it theirs.
- **The repository is found, not assumed.** A session's directory is often a subdirectory of the
  tree, so every git call is made in the root's own frame.
- **The statusline and the commit never meet.** They read the same repository through different
  files, and neither knows the other exists.

## 3. Unresolved

- A commit cannot be undone by this layer. A pair of hooks could, but a repository's hooks are the
  user's.

## 4. Config fields

None. Every session reads the repository it stands in.