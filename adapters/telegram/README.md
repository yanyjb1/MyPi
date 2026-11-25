# adapters/telegram

A bot that speaks to the bridge and carries a conversation to Telegram. It is an
example: it uses nothing but the door's own methods, so a different front end
may be written the same way.

## What it needs

| need | why |
|---|---|
| Python 3.8 or newer | standard library only; nothing to install |
| `mypi --bridge` running | the bot is a client of the door, not of the server |
| an adapter block in `adapters.yml` | the door refuses a name that file does not carry |
| a Telegram bot token | `getMe` is the first thing it does; without one it exits |

`example_adapters.yaml` at the repository root carries the block to copy. Its
`command` starts this script; `local-only: true` keeps the door on this machine.
The program's name is the one the door knows it by: `adapters.yml` must call it
`telegram`, and the bridge's `command` decides how it starts.

## The files

| file | holds |
|---|---|
| `app.py` | the entry: the bot's loop, the held sessions, who is being talked to |
| `commands.py` | every `/` word and the wording of its answer |
| `turn.py` | one round: the block being written, the calls it made, its receipt |
| `bridge.py` | the door: one frame with its answer, one connection's stream |
| `telegram.py` | the bot api, and one long body cut into messages |
| `state.py` | what the bot remembers between restarts |
| `show.py` | how a fact is written for a phone |
| `pyproject.toml` | metadata only; nothing reads it at run time |

## One block is one message

The stream says a piece at a time. The bot sends a block when a `boundary` frame
ends it, when the next call opens, or when the round's own receipt lands: an
adapter fed no `boundary` still gets its prose out. A block is forgotten once
sent, so two blocks are two messages even inside one round, and a long block is
the only thing that is ever cut again.

A hosted session is different: while nobody is talking to it, nothing of its
round is forwarded. Its round ends with one line —
`会话#id结束了，正文消息：<the first fifty characters>` — and `/attach` then
sends the last block whole.

The frame that closes a round is the one carrying `ranges`; a second receipt for
a round already closed is ignored.

## The token

The bot looks for it in three places, in this order:

1. `~/.config/mypi/telegram-bridge.json`, which it writes itself once it has one;
2. `TG_BOT_TOKEN` in the environment;
3. `token.txt` beside `app.py` — the file this repository does not carry.

Whichever it finds, it remembers. `token.txt` and the state file are both in
`.gitignore`: a token in a repository is a token that has leaked.

## What it remembers

`~/.config/mypi/telegram-bridge.json` holds the token, the person it answers, the
hosted sessions and the draft. Its location follows the bridge's own config root,
so `MYPI_CONFIG_DIR` moves both together. Delete it to start over.

## Running it by hand

```sh
python3 adapters/telegram/app.py
```

Under the bridge it is started and stopped with the door, so this is only for
watching it fail loudly.