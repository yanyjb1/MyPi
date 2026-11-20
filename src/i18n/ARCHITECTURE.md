# i18n — architecture

## Files

| path | holds |
|---|---|
| `mod.rs` | the module's declarations and its public surface |
| `table.rs` | the language codes, the embedded JSON, the parsed tables, `text`, `resolve`, `languages` |
| `render.rs` | `render` and the `{name}` filling it delegates to |
| `en.json` | the base language |
| `zh.json` | the Chinese text |

## Relations

- `table.rs` embeds each JSON with `include_str!` and parses it once, on first lookup.
- `LANGUAGES` is the only hand-kept list: a new language is a new file plus one line there.
- The keys are caller-side strings. This module owns no key list and no key constants.
- `render.rs` asks `table.rs` for a template and fills its `{name}` holes; `text` returns the
  raw template. Both take a language code, never a type.
- This module names no other module.

## Must stay in step

1. `LANGUAGES` and the JSON files: every listed file exists, every file is listed.
2. `en.json` and the other files: the same key set. A gap shows as `<key> NOT FOUND`, not as the
   other language.

## Not wired

1. The front end reaches this module through `ui::tui::lang`; nothing calls it outside the TUI yet.
   caller outside it yet.
2. Command details and notices are keyed; the rest of the front end's static text is not.
