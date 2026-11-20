# i18n — architecture

## Files

| path | holds |
|---|---|
| `mod.rs` | `text`, `resolve`, `languages`, `Kind`, the parsed JSON tables |
| `en.json` | the base language |
| `zh.json` | the Chinese text |

## Relations

- `mod.rs` embeds each JSON with `include_str!` and parses it once, on first lookup.
- `LANGUAGES` is the only hand-kept list: a new language is a new file plus one line there.
- The keys are caller-side strings. This module owns no key list and no key constants.
- No other module is named here; this one names none of them.

## Must stay in step

1. `LANGUAGES` and the JSON files: every listed file exists, every file is listed.
2. `en.json` and the other files: the same key set.

## Not wired

1. Server errors and notices still carry English sentences; they move to keys next.
2. `ui::lang::Text` and the local `Notice` table are still en/zh inline.