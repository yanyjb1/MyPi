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
| `bot.py` | the telegram api, and one long body cut into messages |
| `state.py` | what the bot remembers between restarts |
| `show.py` | how a fact is written for a phone |
| `read.py` | the newest words a session wrote: round by round, ranges asked for |
| `words.py` | every line the bot says, in two languages |
| `pyproject.toml` | metadata only; nothing reads it at run time |
| `__main__.py` | the `python -m` entry, from either side of this directory |

## One block is one message

The stream says a piece at a time. The bot sends a block when a `boundary` frame
ends it, when the next call opens, or when the round's own receipt lands: an
adapter fed no `boundary` still gets its prose out. A block is forgotten once
sent, so two blocks are two messages even inside one round, and a long block is
the only thing that is ever cut again.

A hosted session is different: while nobody is talking to it, nothing of its
round is forwarded. Its round ends with one line naming the first fifty
characters of the prose that round opened with; a round the door filtered whole
still announces itself, and says it had no words. `/attach` then sends the newest
assistant block whole, asked for by its round's own ranges rather than taken
from the newest handful of rows.

The frame that closes a round is the one carrying `ranges`; a second receipt for
a round already closed is ignored.

## When the stream ends

A held session outlives its stream. A bridge restart, or the ten minutes of
silence after which the door lets a session go, ends the reader's connection;
the bot asks for it again two seconds later. Only `/quit` lets a session go for
kept.

## The language

`words.py` carries every line twice, English and Chinese, and the bot reads
`config.ui.lang` from the server once at startup: `zh` speaks Chinese and
anything else speaks English. A refusal the server keys and this bot knows is
said in that language; any other refusal is shown with its own key.

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

```sh
python -m adapters.telegram        # from the repository root
python -m telegram                 # from inside `adapters/`
python3 adapters/telegram/app.py   # the file itself, either way
```

Any of the three starts the same bot. The package form is the one that carries over to another machine: copy
the directory, and one line runs it. `--say <words>` sends one line to the paired account and exits, which is
the only way to test from this side without a round.

tree down when it stops, so no `exec`, and no program left holding the port. That is the copy-and-run shape: a
machine needs Python 3 and one line naming where this directory landed.

Under the bridge it is started and stopped with the door, so this is only for
watching it fail loudly.