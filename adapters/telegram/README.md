# adapters/telegram

A bot that speaks to the bridge and carries a conversation to Telegram. It is an
example: it uses nothing but the door's own two methods, so a different front end
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