# i18n

Compiled-in display text: one JSON file per language, looked up by a key the caller supplies.

## Schema

`<code>.json` is a flat object of `key -> sentence`. The key is a plain string chosen by the caller;
this module neither declares the keys nor turns them into constants.

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
- One table serves both halves: the server sends a keyed line, and the front end resolves its own
  static text through the same keys. Neither side invents a sentence.
- A missing key is `<key> NOT FOUND`: no guessed sentence, and no fallback to another language.

## Config fields

None. The language choice stays where the front end reads it.
