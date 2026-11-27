# adapters/telegram

A bot that speaks to the bridge and carries a conversation to Telegram. It is an example: it uses nothing but the door's own methods, so a different front end may be written the same way.

## What it needs

1. Python 3.8 or newer — standard library only, nothing to install.
2. `mypi --bridge` running — the bot is a client of the door, not of the server.
3. An adapter block in `adapters.yml` — the door refuses a name that file does not carry.
4. A Telegram bot token — `getMe` is the first thing it does, and without one it exits.

`example_adapters.yaml` at the repository root carries the block to copy. Its `command` starts this script; `local-only: true` keeps the door on this machine. The program's name is the one the door knows it by: `adapters.yml` must call it `telegram`, and the bridge's `command` decides how it starts.

## The files

1. `app.py` — the entry: the bot's loop, the session being talked to, the ones the door keeps, who is being talked to.
2. `commands.py` — every `/` word and the wording of its answer.
3. `turn.py` — one round: the block being written, the calls it made, its receipt.
4. `bridge.py` — the door: one frame with its answer, one connection's stream.
5. `bot.py` — the telegram api, and one long body cut into messages.
6. `state.py` — what the bot remembers between restarts.
7. `show.py` — how a fact is written for a phone.
8. `read.py` — the newest words a session wrote: round by round, ranges asked for.
9. `words.py` — every line the bot says, in two languages.
10. `pyproject.toml` — metadata only, read by nothing at run time.
11. `__main__.py` — the `python -m` entry, from either side of this directory.

## One block is one message

The stream says a piece at a time. The bot sends a block when a `boundary` frame ends it, when the next call opens, or when the round's own receipt lands: an adapter fed no `boundary` still gets its prose out. A block is forgotten once sent, so two blocks are two messages even inside one round, and a long block is the only thing that is ever cut again.

The frame that closes a round is the one carrying `ranges`; a second receipt for a round already closed is ignored.

## Talking and hosting

Only the session being talked to has a stream of the bot's, which is what keeps it booked on the server. `/attach` takes a session back from the door and opens that stream; the session left behind is handed over instead of dropped, and says so when the bot has no room left to keep it.

`/host` hands the session being talked to back to the door, which keeps it booked with nobody watching. The bot hears nothing about it then: there is no stream, so nothing of its round is forwarded. It asks instead, every few seconds, and `host` / `list` says whether that session is still running. A round it sees end is said once, naming the first fifty characters of the newest assistant block.

`/quit` lets a session go for kept: the stream and the door's keeping both end.

## When the stream ends

A talked-to session outlives its stream. A bridge restart, or the ten minutes of silence after which the door lets a session go, ends the reader's connection; the bot asks for it again two seconds later.

## The language

`words.py` carries every line twice, English and Chinese, and the bot reads `config.ui.lang` from the server once at startup: `zh` speaks Chinese and anything else speaks English. A refusal the server keys and this bot knows is said in that language; any other refusal is shown with its own key.

## The token

The bot looks for it in three places, in this order:

1. `~/.config/mypi/telegram-bridge.json`, which it writes itself once it has one;
2. `TG_BOT_TOKEN` in the environment;
3. `token.txt` beside `app.py` — the file this repository does not carry.

Whichever it finds, it remembers. `token.txt` and the state file are both in `.gitignore`: a token in a repository is a token that has leaked.

## What it remembers

`~/.config/mypi/telegram-bridge.json` holds the token, the person it answers, the sessions the door keeps for it and the draft. Its location follows the bridge's own config root, so `MYPI_CONFIG_DIR` moves both together. Delete it to start over.

```sh
python -m adapters.telegram        # from the repository root
python -m telegram                 # from inside `adapters/`
python3 adapters/telegram/app.py   # the file itself, either way
```

Any of the three starts the same bot. The package form is the one that carries over to another machine: copy the directory, and one line runs it. `--say <words>` sends one line to the paired account and exits, which is the only way to test from this side without a round.

Run this way the bot is its own tree, so it stops when the terminal does. Under the bridge it is started and stopped with the door, which is the shape a machine needs: one line naming where this directory landed, and the door holds the port.