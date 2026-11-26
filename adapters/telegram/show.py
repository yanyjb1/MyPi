# How a fact is written for a phone: one line, cut where a listing needs it.

import time


def shorten(text, width=40):
    # One line, cut at a space when one is near.
    line = " ".join((text or "").split())
    if len(line) <= width:
        return line
    cut = line[:width]
    space = cut.rfind(" ")
    if space > width // 2:
        cut = cut[:space]
    return cut + "…"


def size(count):
    # A byte or token count in the unit a person reads.
    count = float(count or 0)
    for unit in ("", "K", "M", "G"):
        if count < 1024 or unit == "G":
            if unit == "":
                return f"{count:.0f}"
            return f"{count:.1f}{unit}"
        count /= 1024.0
    return f"{count:.0f}"


def stamp(unix):
    # A Telegram `date`, in the shape the server stamps its own rows with.
    if not unix:
        return None
    return time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(int(unix)))


def now():
    # This machine's moment, in the shape the server stamps its own rows with.
    return time.strftime("%Y-%m-%d %H:%M:%S")


def clock(value):
    # A stamp as the short form read on a phone.
    if not value:
        return "—"
    return str(value)[5:16]


def address(meta):
    # A row's model as `provider:model`.
    provider = (meta or {}).get("provider") or ""
    model = (meta or {}).get("model") or ""
    if provider and model:
        return f"{provider}:{model}"
    return model or ""


def split_address(text):
    # `provider:model`, split at the first colon only: a model id may carry more.
    text = (text or "").strip()
    at = text.find(":")
    if at <= 0 or at == len(text) - 1:
        return None
    return text[:at], text[at + 1 :]
