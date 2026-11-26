# The one read this bot makes of a session's history: the newest words the model wrote.

from bridge import Refused


# The newest assistant block of a session, or None when it never wrote one. Rounds are walked newest first, and a round's blocks are asked for only while no earlier round answered.
def newest(door, session):
    try:
        start = door.call("init", {"session": session})
    except Refused:
        return None
    for round in reversed(start.get("structure") or []):
        ranges = round.get("ranges") or []
        if not ranges:
            continue
        try:
            answer = door.call("blocks", {"session": session, "ranges": ranges})
        except Refused:
            continue
        for block in reversed(answer.get("blocks") or []):
            if block.get("kind") != "assistant":
                continue
            if (block.get("content") or "").strip():
                return block
    return None


# The newest words themselves, or an empty line.
def newest_words(door, session):
    block = newest(door, session)
    return (block or {}).get("content") or ""


# When those words landed, in the shape a stamp takes.
def newest_stamp(door, session):
    block = newest(door, session)
    return (block or {}).get("ts")