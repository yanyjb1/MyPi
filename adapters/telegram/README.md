# adapters/telegram

A bot that speaks to the bridge and carries a conversation to Telegram. It is an example: it uses nothing but the door's own methods, so a different front end may be written the same way.

## What it needs

1. Python 3.8 or newer — standard library only, nothing to install.
2. `mypi --bridge` running — the bot is a client of the door, not of the server.
3. An adapter block in `adapters.yml` — the door refuses a name that file does not carry.
4. A Telegram bot token — `getMe` is the first thing it does, and without one it exits.

`example_adapters.yaml` at the repository root carries the block to copy. Its `command` starts this script; `local-only: true` keeps the door on this machine. The program's name is the one the door knows it by: `adapters.yml` must call it `telegram`, and the bridge's `command` decides how it starts.

## The files

- `app.py` — the entry: the bot's loop, the session being talked to, the ones the door keeps, who is being talked to.
- `commands.py` — every `/` word and the wording of its answer.
- `attach.py` — the two attachments a phone sends, landed in the temp dir, and the line naming their path.
- `turn.py` — one round: the block being written, the calls it made, its receipt.
- `bridge.py` — the door: one frame with its answer, one connection's stream.
- `bot.py` — the telegram api: one long body cut into messages, one file either way.
- `state.py` — what the bot remembers between restarts.
- `show.py` — how a fact is written for a phone.
- `read.py` — the newest words a session wrote: round by round, ranges asked for.
- `words.py` — every line the bot says, in two languages.
- `pyproject.toml` — metadata only, read by nothing at run time.
- `__main__.py` — the `python -m` entry, from either side of this directory.

## One block is one message

The stream says a piece at a time. The bot sends a block when a `boundary` frame ends it, when the next call opens, or when the round's own receipt lands: an adapter fed no `boundary` still gets its prose out. A block is forgotten once sent, so two blocks are two messages even inside one round, and a long block is the only thing that is ever cut again.

The frame that closes a round is the one carrying `ranges`; a second receipt for a round already closed is ignored.

## Talking and hosting

Only the session being talked to has a stream of the bot's, which is what keeps it booked on the server. `/attach` takes a session back from the door and opens that stream; the session left behind is handed over instead of dropped, and says so when the bot has no room left to keep it.

`/host` hands the session being talked to back to the door, which keeps it booked with nobody watching. The bot hears nothing about it then: there is no stream, so nothing of its round is forwarded. It asks instead, every few seconds, and `host` / `list` says whether that session is still running. The first round it sees end is said once, naming the first fifty characters of the newest assistant block; every round after that under the same hand-over is silence, however often another front end keeps the session alive in between.

`/quit` lets a session go for kept: the stream and the door's keeping both end.

## When the stream ends

A talked-to session outlives its stream. A bridge restart, a proxy that drops the connection without a word, or the ten minutes of silence after which the door lets a session go all end the reader's connection; the bot asks for it again, waiting twice as long after each failure up to half a minute. A stream silent for ninety seconds counts as ended: a socket a dropped proxy left behind never reports its own end.

The door hands a reconnect the round from its first frame again, so the bot swallows the pieces it has already said and speaks only what followed. A receipt lost with the connection ends a round the bot never saw end, so a dropped stream first says the words no block ended yet, then gives the round up and claims to run one only when the door streams one again.

## Attachments

A session has no field for a file, so an attachment becomes words: the file lands in `mypi-telegram/` under the system temp dir, and the message the model reads carries its path. The model then decides — a picture is read as a picture when the session's model takes images, anything else as the file it is.

Only two kinds are taken:

1. `photo` — the largest size Telegram offers, saved as `photo-<message id>`, with its pixel count named in the message.
2. `document` — its own name, prefixed with the message id, refused when Telegram's twenty-megabyte limit is already passed.

Voice, video, audio and stickers are answered as words the bot does not keep. The line naming the path also names how to send a file back: the one-off `--file` mode, written out whole so a session can run it as it stands.

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

Any of the three starts the same bot. The package form is the one that carries over to another machine: copy the directory, and one line runs it.

`--say <words>` sends one line to the paired account and exits. `--file <path> --text <caption>` sends one file and exits, the caption optional; a picture goes as a picture, anything else as a document Telegram does not change. Both print what they did before leaving, which is how a session learns a file was sent; a file over Telegram's fifty megabytes, or one the api refuses, says why and exits non-zero.

Run this way the bot is its own tree, so it stops when the terminal does. Under the bridge it is started and stopped with the door, which is the shape a machine needs: one line naming where this directory landed, and the door holds the port.

`/restart` replaces the running script with a fresh copy of itself: the pid stays the same, so the door's child is still its child, and nothing outside has to know. The phone is told before the swap and again once the new process is up, since a bot nobody hears from is a bot nobody can tell is dead.

Every boot says one line on the phone for the same reason: a process that came back cannot otherwise be told from one that never went.
