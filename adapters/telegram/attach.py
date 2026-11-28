# Attachments: the two things a phone sends besides words, landed in the temp dir, and the line that tells the model where.

import os
import tempfile

import words
from bot import BotError
from show import size

# The one directory attachments land in, under the system temp dir: a path in an old notice stays readable.
INBOX = "mypi-telegram"

# What Telegram hands a bot in one file.
LIMIT = 20 * 1024 * 1024

# What a phone sends that this bot does not take: its caption must not pass for words the user wrote alone.
OTHERS = ("voice", "audio", "video", "video_note", "sticker", "animation")


class Item:
    # One received attachment: where it landed, what it is, and the facts the model cannot see for itself.
    def __init__(self, path, kind, name, length, width=0, height=0):
        self.path = path
        self.kind = kind
        self.name = name
        self.length = length
        self.width = width
        self.height = height


# The kind of attachment this message carries that is not taken, or None.
def untaken(message):
    for kind in OTHERS:
        if message.get(kind) is not None:
            return kind
    return None


# The attachment this message carries, already on this machine; None for a message that is only words.
def receive(bot, message):
    photos = message.get("photo") or []
    if photos:
        largest = max(photos, key=lambda one: int(one.get("width") or 0) * int(one.get("height") or 0))
        remote, data = bot.fetch(largest["file_id"])
        name = f"photo-{message.get('message_id')}{os.path.splitext(remote)[1] or '.jpg'}"
        return save(name, data, "photo", int(largest.get("width") or 0), int(largest.get("height") or 0))
    document = message.get("document")
    if document:
        if int(document.get("file_size") or 0) > LIMIT:
            raise BotError(words.t("attach_too_big", size=size(LIMIT)))
        remote, data = bot.fetch(document["file_id"])
        named = document.get("file_name") or os.path.basename(remote)
        return save(f"{message.get('message_id')}-{safe(named)}", data, "document")
    return None


# The line the model reads once the file is here: what it is, where it is, and the command that sends one back.
def note(item, script):
    if item.kind == "photo":
        said = words.t("attach_photo", size=f"{item.width}×{item.height}", bytes=size(item.length), path=item.path)
    else:
        said = words.t("attach_file", name=item.name, bytes=size(item.length), path=item.path)
    return said + "\n" + words.t("attach_send", script=script)


# The inbox, made once and left: every notice names a path that outlives this process.
def inbox():
    where = os.path.join(tempfile.gettempdir(), INBOX)
    os.makedirs(where, exist_ok=True)
    return where


def save(name, data, kind, width=0, height=0):
    path = os.path.join(inbox(), name)
    with open(path, "wb") as f:
        f.write(data)
    return Item(path, kind, name, len(data), width, height)


# One file name, kept whole but for what a path may not carry.
def safe(name):
    base = os.path.basename((name or "").replace("\\", "/")).strip().lstrip(".")
    return base or "file"