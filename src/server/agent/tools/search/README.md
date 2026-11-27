# search

The `search` tool entry: one query against exactly one provider, on the session's search tab; the engines, the page walk and the hit parsing live in `src/browser`, and this tool is the model's verb.

## 1. Design / Schema

1. `query` — always required; the text to search for.
2. `provider` — always required; `google`, `bing` or `baidu`.
3. `pages` — 1 by default; pages 1..=`pages`.

`provider` is `search`'s alone, because one search runs against exactly one provider and a call that names none is refused rather than spread across several tabs; `search` also takes no array.
See `src/browser/README.md` for the engines, the page walk and the answer's shape.

## 2. Design intent

- **One query, one provider.** A search that names no provider is a question, and spreading it across tabs would put three different result sets into one answer.
- **A separate tab.** Search and interactive work have different lifetimes, and `search` holds the search tab so a page being read is not replaced by a query.

## 3. Unresolved

See `src/browser/README.md`, since the tool layer adds none.

## 4. Config fields

None at the tool layer.