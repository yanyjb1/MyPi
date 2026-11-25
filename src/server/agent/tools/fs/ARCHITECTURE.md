# fs — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | module declarations |
| `change.rs` | one line-addressed change: the context around it, its receipt and its line arithmetic |
| `fault.rs` | the batch's audit: checks and tests after the last write, and where each fault lands |
| `gate.rs` | the process gate: one child, its deadline, the turn's stop and its own group |
| `eol.rs` | the line ending a written text keeps: read as plain lines, written back in the file's own ending |
| `skeleton.rs` | the report skeleton: numbered source lines, pieces and the seam around an error |
| `edit/` | one unique text run replaced in place |
| `grep/` | structural search over the tree a language claims |
| `lang/` | the language registry and the languages it holds |
| `read/` | files by line numbers, directories by name |
| `splice/` | whole lines replaced by number |
| `write/` | a whole file overwritten |

## Relations

- The five tools are consumed by the engine and the registry like every tool.
- Every written tool hands its report to the shared audit; the audit runs one check per unit after
  the batch's last write.
- The parser, checker and test run are the file's language's own, run through the shared process
  gate.
- `edit` and `splice` share the change context; `read`, `grep` and every report share the skeleton.
- `read`, `grep`, `edit` and `splice` read the same language table.
- `lang` names no tool; a tool asks the registry, never a language.