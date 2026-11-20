# i18n

Compiled-in display text: one JSON file per language, looked up by a key the caller supplies.

## Schema

`<code>.json` is a flat object of `key -> sentence`. The key is a plain string chosen by the
caller; this module neither declares the keys nor turns them into constants.

```json
{"busy": "the session is already in a turn"}
```

`en.json` is the base language. `table.rs` lists the files with `include_str!`, so adding a language
is adding `<code>.json` and one line there.

## Lookup

- `text(code, key)` returns the template, or `None` when the language or the key has none.
- `render(code, key, args)` fills `{name}` from the JSON value; a key no table holds answers
  `<key> NOT FOUND`, in every language.
- `resolve(code)` returns the compiled language, or `en` when the code has no file.
- `languages()` lists the codes this binary carries.

## Design intent

- A template may carry `{name}` holes; `render` fills them from the optional `args` object.
- The server sends a kind and a key, never a sentence it made up. Text from an upstream (a
  provider error, a tool result) passes through as `text`.
- A missing key is `<key> NOT FOUND`: no guessed sentence, and no fallback to another language.

## Unresolved

- The front end's own static text (`ui::lang::Text`, the local `Notice` table) still carries en/zh
  inline; it moves here in a later pass.

## Config fields

None. The language choice stays where the front end reads it.
