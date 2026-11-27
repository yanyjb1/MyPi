# Every `/` word the bot answers, and the wording of each answer.

import os

import read
import words
from bridge import Refused
from show import address, clock, shorten, size, split_address


# One command word, dispatched; anything else is said back as unknown.
def run(app, word, rest):
    table = {
        "help": help_,
        "start": help_,
        "list": ls,
        "ls": ls,
        "new": new,
        "attach": attach,
        "host": host,
        "quit": quit_,
        "draft": draft,
        "switch": switch,
        "model": switch,
        "effort": effort,
        "cwd": cwd,
        "cd": cwd,
        "name": rename,
        "profile": profile,
        "usage": usage,
        "compact": compact,
        "esc": esc,
        "stop": esc,
    }
    handler = table.get(word)
    if handler is None:
        app.say(words.t("unknown_command", word=word))
        return
    try:
        handler(app, rest)
    except Refused as e:
        app.say(f"…{e.message}")


def help_(app, rest):
    app.say(words.t("help"))


def ls(app, rest):
    rows = app.door.call("sessions").get("rows") or []
    if not rows:
        app.say(words.t("no_sessions"))
        return
    lines = []
    for row in rows[:30]:
        session = row.get("id")
        title = row.get("name") or shorten(row.get("first_user") or "", 28) or words.t("empty_session")
        mark = "▶" if session == app.state.current else ("●" if session in app.state.hosted else "·")
        lines.append(f"{mark} {session} {shorten(title, 34)}")
        lines.append(f"    {clock(row.get('last_ts') or row.get('started_at'))} · {size(row.get('bytes'))} · {shorten(row.get('cwd') or '', 34)}")
    if len(rows) > 30:
        lines.append(words.t("more_sessions", count=len(rows) - 30))
    lines.append("")
    lines.append(words.t("ls_legend"))
    app.say("\n".join(lines))


def new(app, rest):
    app.state.new_draft(rest or None)
    plan = app.state.draft
    app.say(words.t("draft_new", **slots(plan)))


def draft(app, rest):
    plan = app.state.draft
    if not plan:
        app.say(words.t("no_draft"))
        return
    app.say(words.t("draft_show", **slots(plan)))


# A draft's four slots, each said the way an empty one reads.
def slots(plan):
    return {
        "cwd": plan.get("cwd") or words.t("default_value"),
        "model": plan.get("model") or words.t("default_value"),
        "effort": plan.get("effort") or "auto",
        "profile": plan.get("profile") or words.t("default_value"),
    }


# Talking to a session: the door gives it back if it was keeping it, and one stream of ours holds it from here.
def attach(app, rest):
    session = pick(app, rest, "list", int)
    if session is None:
        app.say(words.t("attach_need_id"))
        return
    meta = app.door.call("meta", {"session": session})
    # The session being left is handed to the door, and the one picked is taken back from it.
    before = app.state.current
    if before is not None and before != session and not app.host(before):
        app.say(words.t("attach_full", session=before))
        return
    app.unhost(session)
    app.state.current = session
    app.state.save()
    app.listen(session)
    app.say(words.t("attached", session=session, name=shorten(meta.get("name") or words.t("unnamed"), 30), address=address(meta), cwd=shorten(meta.get("cwd") or "", 40)))
    # The last words this session wrote, whole: the receipt that closed its round only named them.
    text = read.newest_words(app.door, session)
    if text:
        app.say(text)


def host(app, rest):
    session = app.state.current
    if session is None:
        app.say(words.t("host_no_current"))
        return
    if not app.host(session):
        app.say(words.t("host_full"))
        return
    app.say(words.t("hosted", session=session))


def quit_(app, rest):
    session = app.state.current if app.state.current is not None else pick(app, rest, None, int)
    if session is None:
        app.say(words.t("quit_no_session"))
        return
    if app.running(session):
        app.say(words.t("quit_running", session=session))
        return
    app.release(session)
    app.say(words.t("quit_done", session=session))


def switch(app, rest):
    target = app.target()
    if target is None:
        app.say(words.t("no_target"))
        return
    if not rest:
        models = app.door.call("pools").get("models") or []
        app.picks = [model.get("address") for model in models]
        app.picks_kind = "switch"
        if not models:
            app.say(words.t("no_models"))
            return
        lines = [words.t("pick_models")]
        for index, model in enumerate(models, 1):
            lines.append(f"{index}. {model.get('address')}  {size(model.get('context_window'))}")
        app.say("\n".join(lines))
        return
    chosen = pick(app, rest, "switch", str)
    if chosen is None:
        app.say(words.t("no_pick_switch"))
        return
    pair = split_address(chosen)
    if pair is None:
        app.say(words.t("bad_address"))
        return
    if target["draft"]:
        app.state.draft["model"] = chosen
        app.state.save()
        app.say(words.t("draft_model", model=chosen))
        return
    session = target["session"]
    app.door.post("meta", {"name": "switch", "session": session, "content": {"session": session, "model": {"provider": pair[0], "model": pair[1]}}})
    app.say(words.t("session_model", session=session, model=chosen))


def effort(app, rest):
    target = app.target()
    if target is None:
        app.say(words.t("no_target"))
        return
    rungs = app.rungs(target)
    if not rest:
        if not rungs:
            app.say(words.t("no_rungs"))
            return
        app.picks = rungs
        app.picks_kind = "effort"
        app.say(words.t("pick_rungs") + " " + " ".join(rungs))
        return
    chosen = pick(app, rest, "effort", str) or rest
    if rungs and chosen not in rungs:
        app.say(words.t("rungs_only", rungs=" ".join(rungs)))
        return
    if target["draft"]:
        app.state.draft["effort"] = chosen
        app.state.save()
        app.say(words.t("draft_effort", effort=chosen))
        return
    session = target["session"]
    app.door.post("meta", {"name": "effort", "session": session, "content": {"session": session, "effort": chosen}})
    app.say(words.t("session_effort", session=session, effort=chosen))


def cwd(app, rest):
    target = app.target()
    if target is None:
        app.say(words.t("no_target"))
        return
    here = app.where(target)
    if not rest:
        listing = app.door.ask("local", "list", {"path": "", "cwd": here or ""})
        names = [entry for entry in listing.get("entries") or [] if entry.get("dir")]
        app.picks = [entry["name"] for entry in names]
        app.picks_kind = "cwd"
        lines = [words.t("pick_dirs", dir=listing.get("dir") or here or "?")]
        if names:
            for index, entry in enumerate(names, 1):
                lines.append(f"{index}. {entry['name']}")
        else:
            lines.append(words.t("no_dirs"))
        lines.append(words.t("up_one"))
        lines.append(words.t("or_path"))
        app.say("\n".join(lines))
        return
    chosen = pick(app, rest, "cwd", str)
    if chosen is None:
        app.say(words.t("no_pick_cwd"))
        return
    if chosen == "..":
        path = os.path.dirname((here or "").rstrip("/")) or "/"
    elif os.path.isabs(chosen) or chosen.startswith("~"):
        path = chosen
    else:
        path = os.path.join(here or "", chosen)
    if target["draft"]:
        app.state.draft["cwd"] = path
        app.state.save()
        app.say(words.t("draft_cwd", cwd=path))
        return
    session = target["session"]
    app.door.post("meta", {"name": "cwd", "session": session, "content": {"session": session, "path": path}})
    app.say(words.t("session_cwd", session=session, cwd=path))


def rename(app, rest):
    target = app.target()
    if target is None:
        app.say(words.t("no_current"))
        return
    if not rest:
        app.say(words.t("name_need"))
        return
    if target["draft"]:
        app.say(words.t("draft_no_name"))
        return
    session = target["session"]
    app.door.post("meta", {"name": "name", "session": session, "content": {"session": session, "name": rest}})
    app.say(words.t("renamed", session=session, name=shorten(rest, 40)))


def profile(app, rest):
    if not app.state.draft:
        app.say(words.t("profile_fixed"))
        return
    if not rest:
        profiles = app.door.call("pools").get("profiles") or []
        app.picks = profiles
        app.picks_kind = "profile"
        if not profiles:
            app.say(words.t("no_profiles"))
            return
        lines = [words.t("pick_profiles")]
        for index, item in enumerate(profiles, 1):
            lines.append(f"{index}. {item}")
        app.say("\n".join(lines))
        return
    chosen = pick(app, rest, "profile", str)
    if chosen is None:
        app.say(words.t("no_pick_profile"))
        return
    app.state.draft["profile"] = chosen
    app.state.save()
    app.say(words.t("draft_profile", profile=chosen))


def usage(app, rest):
    target = app.target()
    if target is None or target["draft"]:
        app.say(words.t("no_bill"))
        return
    session = target["session"]
    bill = app.door.call("usage", {"session": session})
    meta = app.door.call("meta", {"session": session})
    lines = [words.t("bill_head", session=session, address=address(meta))]
    total = int(bill.get("total_tokens") or 0)
    if total:
        lines.append(words.t("bill_tokens", total=size(total), prompt=size(bill.get("prompt_tokens")), completion=size(bill.get("completion_tokens"))))
        hit = round(float(bill.get("cache_hit_rate") or 0) * 100)
        if hit:
            lines.append(words.t("bill_cache", percent=hit))
    else:
        lines.append(words.t("bill_no_tokens"))
    if bill.get("cny") is not None:
        lines.append(words.t("bill_cost", cny=float(bill.get("cny") or 0), usd=float(bill.get("usd") or 0)))
    context = bill.get("context")
    if context:
        window = app.window_of(address(meta))
        mark = "≈" if bill.get("estimated") else ""
        if window:
            lines.append(words.t("bill_context_window", mark=mark, context=size(context), window=size(window), percent=round(100.0 * context / window)))
        else:
            lines.append(words.t("bill_context", mark=mark, context=size(context)))
    rounds = sum(int(group.get("rounds") or 0) for group in bill.get("groups") or [])
    if rounds:
        lines.append(words.t("bill_rounds", rounds=rounds))
    app.say("\n".join(lines))


def compact(app, rest):
    target = app.target()
    if target is None or target["draft"]:
        app.say(words.t("no_compact"))
        return
    session = target["session"]
    payload = {"session": session}
    if rest:
        payload["focus"] = rest
    app.door.subscribe("compact", payload)
    app.say(words.t("compacting", session=session))


def esc(app, rest):
    target = app.target()
    if target is None or target["draft"]:
        app.say(words.t("no_stop"))
        return
    session = target["session"]
    app.door.post("interrupt", {"session": session})
    app.say(words.t("stopped", session=session))


# A word as an index into what was last listed, else the word itself.
def pick(app, word, kind, cast):
    word = (word or "").strip()
    if not word:
        return None
    if kind and app.picks_kind == kind and app.picks:
        try:
            index = int(word)
        except ValueError:
            index = 0
        if 1 <= index <= len(app.picks):
            return cast(app.picks[index - 1])
    try:
        return cast(word)
    except (TypeError, ValueError):
        return None