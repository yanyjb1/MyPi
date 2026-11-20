# i18n

Compiled-in display text: one JSON file per language, looked up by a key the caller supplies.

## Schema

`<code>.json` is a flat object of `key -> sentence`. The key is a plain string chosen by the
caller; this module neither declares the keys nor turns them into constants.

```json
{"busy": "the session is already in a turn"}
```

`en.json` is the base language. `mod.rs` lists the files with `include_str!`, so adding a language
is adding `<code>.json` and one line there.

## Lookup

- `text(code, key)` returns the sentence, or `not found` when the language or the key has none.
- `resolve(code)` returns the compiled language, or `en` when the code has no file.
- `languages()` lists the codes this binary carries.
- `Kind` says what a piece of text is for: `Error`, `Warning`, `Notice`.

## Design intent

- A sentence is complete. Nothing here fills variables; the caller formats around this call.
- The server sends a `Kind` and a key, never a sentence it made up. Text from an upstream (a
  provider error, a tool result) passes through as `text`.
- A missing key is `not found`, not a guessed English sentence.

## Unresolved

- The front end's own static text (`ui::lang::Text`, the local `Notice` table) still carries en/zh
  inline; it moves here in a later pass.

## Config fields

None. The language choice stays where the front end reads it.