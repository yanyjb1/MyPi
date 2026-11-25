# What the bot remembers between restarts: the pairing, the hosted set, the pointer, the draft.

import json
import os
import tempfile

FILE = "telegram-bridge.json"


# The same directory the bridge reads `adapters.yml` from.
def config_dir():
    override = os.environ.get("MYPI_CONFIG_DIR")
    if override:
        return override
    if os.name == "nt":
        base = os.environ.get("APPDATA") or os.path.join(os.path.expanduser("~"), "AppData", "Roaming")
    else:
        base = os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")
    return os.path.join(base, "mypi")


def default_path():
    return os.path.join(config_dir(), FILE)


class State:
    def __init__(self, path=None):
        self.path = path or default_path()
        self.bot_token = None
        self.user = None
        self.hosted = []
        self.current = None
        self.draft = None
        self.load()

    def load(self):
        try:
            with open(self.path, encoding="utf-8") as f:
                data = json.load(f)
        except (OSError, ValueError):
            return
        self.bot_token = data.get("botToken")
        self.user = data.get("user")
        self.hosted = [int(n) for n in data.get("hosted") or []]
        self.current = data.get("current")
        self.draft = data.get("draft")

    # Written whole or not at all: a half-written state file would lose the pairing on the next start.
    def save(self):
        data = {
            "botToken": self.bot_token,
            "user": self.user,
            "hosted": self.hosted,
            "current": self.current,
            "draft": self.draft,
        }
        os.makedirs(os.path.dirname(self.path), exist_ok=True)
        handle, temp = tempfile.mkstemp(dir=os.path.dirname(self.path), prefix=".tg-", suffix=".json")
        try:
            with os.fdopen(handle, "w", encoding="utf-8") as f:
                json.dump(data, f, ensure_ascii=False, indent="\t")
                f.write("\n")
            os.replace(temp, self.path)
        except OSError:
            try:
                os.unlink(temp)
            except OSError:
                pass
            raise

    # The hosted set is a list of at most three; the pointer is one of them or a draft.
    def host(self, session):
        if session in self.hosted:
            return False
        self.hosted.append(session)
        self.save()
        return True

    def drop(self, session):
        if session not in self.hosted:
            return False
        self.hosted.remove(session)
        if self.current == session:
            self.current = None
        self.save()
        return True

    def new_draft(self, cwd=None):
        self.draft = {"cwd": cwd, "model": None, "effort": "auto", "profile": None}
        self.current = None
        self.save()
        return self.draft
