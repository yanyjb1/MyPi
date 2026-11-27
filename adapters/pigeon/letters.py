# Letters on disk: one json file per letter under `cache/<mailbox>/`, claimed by rename before being read.

import json
import os
import time
import uuid

CACHE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "cache")


def box(name):
    if not name or name in (".", "..") or "/" in name or "\\" in name:
        raise ValueError(f"not a mailbox name: {name!r}")
    return os.path.join(CACHE, name)


def post(sender, recipient, body):
    path = box(recipient)
    os.makedirs(path, exist_ok=True)
    letter = {"from": sender, "body": body}
    name = f"{time.time_ns():020d}-{uuid.uuid4().hex}.json"
    partial = os.path.join(path, f".{name}")
    with open(partial, "w", encoding="utf-8") as f:
        json.dump(letter, f, ensure_ascii=False)
    os.replace(partial, os.path.join(path, name))
    return letter


def fetch(recipient):
    path = box(recipient)
    try:
        names = sorted(name for name in os.listdir(path) if name.endswith(".json"))
    except FileNotFoundError:
        return []
    letters = []
    for name in names:
        whole = os.path.join(path, name)
        claimed = f"{whole}.claim-{os.getpid()}-{uuid.uuid4().hex}"
        try:
            os.rename(whole, claimed)
        except FileNotFoundError:
            continue
        try:
            with open(claimed, encoding="utf-8") as f:
                letters.append(json.load(f))
        finally:
            os.unlink(claimed)
    try:
        os.rmdir(path)
    except OSError:
        pass
    return letters
