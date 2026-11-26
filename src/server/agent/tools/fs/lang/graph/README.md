# graph — design

One file's neighbourhood, built from a language's facts and printed by that language or by a default.

## 1. Design

A language supplies its module facts. This layer asks for units, declarations, exports, edges and a search domain, then aggregates them into one neighbourhood: the unit being viewed, its base for printed paths, its interface, its children, the line that declares it, what it reaches (`out`), who reaches it (`up`), and where each walk stopped.

The language resolves text to files. A raw edge carries its landing candidates, the symbols it brings, the forwarding entries it passed, and whether it is itself a forwarding declaration. This layer takes the first candidate that exists, maps it to its unit, drops forwarding edges from `incoming`, normalizes incoming targets to the unit itself, and merges per writing file.

`up` is a walk: it climbs one directory at a time and scans that level's tree. The first level always finishes; later levels share a deadline and a level that overruns is dropped whole. A unit whose own directory is the domain root has no level above it, so that directory is the one level read. Files a language does not claim are never read.

Pairing: a **unit** is a file, a directory, or a named thing with no entry; a **decl** is one unit a parent declares, with the line and the parent's tags for it; a **child** adds size, whether it declares units of its own, its exports, and the edges that land inside the viewed unit; an **edge** is one reference, at a file and line, to a unit or to nothing.

## 2. Design intent

- One view for every language, so a new language is one file of facts and one registration entry.
- No language knowledge here: no `mod.rs`, no `pub`, no `Cargo.toml`. Only the registry answers which suffixes a walk may read.
- Facts speak files; the machine speaks units. That split keeps unit identity and inward/outward counting in one place.
- The machine never decides how to narrate. It offers parts and a default arrangement; a language that wants its own wording writes a view.

## 3. Unsolved

- A walk may ask a language for the same file's edges only once per view; a cross-view cache would need a lifetime it does not have.

## 4. Config

None. The view carries no budget: it is printed whole.