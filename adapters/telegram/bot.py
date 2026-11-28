# The bot's own API: long polling in, plain messages out, one file either way.

import json
import os
import time
import urllib.error
import urllib.request
import uuid

# Telegram's own ceiling for one message; anything longer is refused by the server.
LIMIT = 4096

# How long a 429 makes the bot wait when Telegram names no delay.
PAUSE = 3

# What one sent file may weigh, and what a caption may carry: Telegram's own two ceilings.
SEND = 50 * 1024 * 1024
CAPTION = 1024


class BotError(Exception):
    pass


class Bot:
    def __init__(self, token):
        self.token = token
        self.offset = None

    # One JSON call, with the wait Telegram asks for honoured once before the failure is raised.
    def _call(self, method, body, timeout=60):
        answer = self._post(method, json.dumps(body, ensure_ascii=False).encode(), "application/json", timeout)
        if not answer.get("ok"):
            raise BotError(answer.get("description") or "the bot api refused")
        return answer.get("result")

    def _post(self, method, data, ctype, timeout=60):
        url = f"https://api.telegram.org/bot{self.token}/{method}"
        for attempt in (0, 1):
            request = urllib.request.Request(url, data=data, method="POST", headers={"content-type": ctype})
            try:
                with urllib.request.urlopen(request, timeout=timeout) as res:
                    return self._read(res.read())
            except urllib.error.HTTPError as e:
                answer = self._read(e.read())
                if e.code == 429 and attempt == 0:
                    wait = int((answer.get("parameters") or {}).get("retry_after") or PAUSE)
                    time.sleep(min(wait, 60))
                    continue
                raise BotError(answer.get("description") or str(e))
            except (urllib.error.URLError, OSError) as e:
                raise BotError(str(e))
            except Exception as e:  # noqa: BLE001 - a proxy that answers rubbish is a call that failed, not a bot that died
                raise BotError(str(e))
        raise BotError("the bot api refused")

    @staticmethod
    def _read(raw):
        try:
            return json.loads(raw)
        except ValueError:
            return {"ok": False, "description": "the bot api answered something unreadable"}

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

    # Where Telegram keeps one file, as a path that `download` asks for.
    def lookup(self, file_id):
        found = self._call("getFile", {"file_id": file_id})
        remote = found.get("file_path")
        if not remote:
            raise BotError("the bot api named no path for that file")
        return remote

    # The file itself, which Telegram serves from a second address carrying the token.
    def download(self, remote):
        url = f"https://api.telegram.org/file/bot{self.token}/{remote}"
        try:
            with urllib.request.urlopen(url, timeout=120) as res:
                return res.read()
        except (urllib.error.URLError, OSError) as e:
            raise BotError(str(e))

    # Both at once: the path Telegram knows it by and the bytes, since a caller wants both.
    def fetch(self, file_id):
        remote = self.lookup(file_id)
        return remote, self.download(remote)

    # One file to the paired account: a picture as a picture, anything else as a document it is not changed by.
    def send_file(self, chat, path, caption=None, photo=False):
        with open(path, "rb") as f:
            data = f.read()
        fields = {"chat_id": str(chat)}
        if caption:
            fields["caption"] = caption[:CAPTION]
        name = os.path.basename(path).replace('"', "")
        if photo:
            fields["photo"] = (name, data)
            return self._multipart("sendPhoto", fields)
        fields["document"] = (name, data)
        return self._multipart("sendDocument", fields)

    # One multipart body, the shape Telegram takes for a file: every other field rides as plain text.
    def _multipart(self, method, fields):
        boundary = f"mypi-{uuid.uuid4().hex}"
        body = bytearray()
        for name, value in fields.items():
            body += f"--{boundary}\r\n".encode()
            if isinstance(value, tuple):
                filename, data = value
                body += f'Content-Disposition: form-data; name="{name}"; filename="{filename}"\r\n'.encode()
                body += b"Content-Type: application/octet-stream\r\n\r\n"
                body += data
            else:
                body += f'Content-Disposition: form-data; name="{name}"\r\n\r\n'.encode()
                body += str(value).encode()
            body += b"\r\n"
        body += f"--{boundary}--\r\n".encode()
        answer = self._post(method, bytes(body), f"multipart/form-data; boundary={boundary}", timeout=180)
        if not answer.get("ok"):
            raise BotError(answer.get("description") or "the bot api refused")
        return answer.get("result")


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