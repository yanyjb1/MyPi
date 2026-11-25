# The bot's own API: long polling in, plain messages out.

import json
import urllib.error
import urllib.request

# Telegram's own ceiling for one message; anything longer is refused by the server.
LIMIT = 4096


class BotError(Exception):
    pass


class Bot:
    def __init__(self, token):
        self.token = token
        self.offset = None

    def _call(self, method, body, timeout=60):
        request = urllib.request.Request(
            "https://api.telegram.org/bot%s/%s" % (self.token, method),
            data=json.dumps(body, ensure_ascii=False).encode(),
            method="POST",
            headers={"content-type": "application/json"},
        )
        try:
            with urllib.request.urlopen(request, timeout=timeout) as res:
                answer = json.loads(res.read())
        except urllib.error.HTTPError as e:
            try:
                answer = json.loads(e.read())
            except ValueError:
                raise BotError(str(e))
            raise BotError(answer.get("description") or str(e))
        except (urllib.error.URLError, OSError) as e:
            raise BotError(str(e))
        if not answer.get("ok"):
            raise BotError(answer.get("description") or "the bot api refused")
        return answer.get("result")

    def me(self):
        return self._call("getMe", {})

    def drop_webhook(self):
        try:
            self._call("deleteWebhook", {"drop_pending_updates": False})
        except BotError:
            pass

    def skip_backlog(self):
        # Start from what arrives now: a bot chewing through yesterday's queue is no good.
        try:
            updates = self._call("getUpdates", {"offset": -1, "limit": 1, "timeout": 0})
            if updates:
                self.offset = updates[-1]["update_id"] + 1
        except BotError:
            pass

    def poll(self, wait=30):
        body = {"limit": 10, "timeout": wait, "allowed_updates": ["message"]}
        if self.offset is not None:
            body["offset"] = self.offset
        updates = self._call("getUpdates", body, timeout=wait + 30)
        for update in updates:
            self.offset = update["update_id"] + 1
        return updates

    def send(self, chat, text):
        sent = 0
        for piece in split(text):
            body = {"chat_id": chat, "text": piece, "link_preview_options": {"is_disabled": True}}
            self._call("sendMessage", body)
            sent += 1
        return sent

    def typing(self, chat):
        try:
            self._call("sendChatAction", {"chat_id": chat, "action": "typing"})
        except BotError:
            pass


def split(text):
    # One long body into messages Telegram accepts, cut at a paragraph or a line where one fits.
    text = (text or "").strip()
    if not text:
        return []
    if len(text) <= LIMIT:
        return [text]
    pieces = []
    held = ""
    for paragraph in text.split("\n\n"):
        while len(paragraph) > LIMIT:
            head, paragraph = paragraph[:LIMIT], paragraph[LIMIT:]
            if held:
                pieces.append(held)
                held = ""
            pieces.append(head)
        candidate = paragraph if not held else held + "\n\n" + paragraph
        if len(candidate) <= LIMIT:
            held = candidate
        else:
            pieces.append(held)
            held = paragraph
    if held:
        pieces.append(held)
    return pieces
