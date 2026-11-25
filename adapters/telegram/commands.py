# Every `/` word the bot answers, and the wording of each answer.

import os

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
        app.say("没有 /%s 这个命令。/help 看有什么。" % word)
        return
    try:
        handler(app, rest)
    except Refused as e:
        app.say("…%s" % e.message)


def help_(app, rest):
    lines = [
        "把话直接发过来，就是发给当前会话。命令：",
        "",
        "/list 会话列表",
        "/attach 编号 换到那个会话",
        "/new [目录] 立一份草稿",
        "/draft 看草稿",
        "/host 把当前会话交给后台",
        "/quit 让当前会话退出",
        "/switch [模型] 不带参数就列模型（/model 同）",
        "/effort [档位] 不带参数就列档位",
        "/profile [档] 只对草稿有用，不带参数就列",
        "/cwd [路径] 不带参数就列目录（/cd 同）",
        "/name 名字",
        "/usage 这个会话的账",
        "/compact [重点] 压缩上下文",
        "/esc 打断正在跑的这一轮（/stop 同）",
    ]
    app.say("\n".join(lines))


def ls(app, rest):
    rows = app.door.call("sessions").get("rows") or []
    if not rows:
        app.say("还没有会话。/new 起一个。")
        return
    lines = []
    for row in rows[:30]:
        session = row.get("id")
        title = row.get("name") or shorten(row.get("first_user") or "", 28) or "（空会话）"
        mark = "▶" if session == app.state.current else ("●" if session in app.hosted else "·")
        lines.append("%s %d %s" % (mark, session, shorten(title, 34)))
        lines.append("    %s · %s · %s" % (clock(row.get("last_ts") or row.get("started_at")), size(row.get("bytes")), shorten(row.get("cwd") or "", 34)))
    if len(rows) > 30:
        lines.append("…还有 %d 个" % (len(rows) - 30))
    lines.append("")
    lines.append("▶ 正在说　● 托管中　· 都没")
    app.say("\n".join(lines))


def new(app, rest):
    app.state.new_draft(rest or None)
    plan = app.state.draft
    app.say(
        "草稿起来了：目录 %s、模型 %s、档位 %s、档 %s。\n改好之后直接说第一句话，会话就建出来了。"
        % (plan.get("cwd") or "（默认）", plan.get("model") or "（默认）", plan.get("effort") or "auto", plan.get("profile") or "（默认）")
    )


def draft(app, rest):
    plan = app.state.draft
    if not plan:
        app.say("现在没有草稿。/new 立一份。")
        return
    app.say("草稿：\n目录 %s\n模型 %s\n档位 %s\n档 %s" % (plan.get("cwd") or "（默认）", plan.get("model") or "（默认）", plan.get("effort") or "auto", plan.get("profile") or "（默认）"))


def attach(app, rest):
    session = pick(app, rest, "list", int)
    if session is None:
        app.say("/attach 后面跟一个编号，编号在 /list 里。")
        return
    meta = app.door.call("meta", {"session": session})
    if not app.hold(session):
        app.say("托管满了，先 /quit 放掉一个再换。")
        return
    app.state.current = session
    app.state.save()
    app.say("对着 %d 号说话了。\n%s · %s · %s" % (session, shorten(meta.get("name") or "（无名）", 30), address(meta), shorten(meta.get("cwd") or "", 40)))
    # The last words this session wrote, whole: the receipt that closed its round only named them.
    text = last_text(app, session)
    if text:
        app.say(text)


# The newest assistant block's own words, or nothing when the session never spoke.
def last_text(app, session):
    try:
        blocks = app.door.call("init", {"session": session}).get("blocks") or []
    except Refused:
        return ""
    for block in reversed(blocks):
        if block.get("kind") == "assistant":
            return block.get("content") or ""
    return ""


def host(app, rest):
    session = app.state.current
    if session is None:
        app.say("现在没对着会话。/list 挑一个，或者 /new 起一个。")
        return
    if not app.hold(session):
        app.say("托管满了，先 /quit 放掉一个再来。")
        return
    app.state.current = None
    app.state.save()
    app.say("%d 号交给后台了，跑完我说一声。\n/attach 换个会话，或者 /new 起一个。" % session)


def quit_(app, rest):
    session = app.state.current if app.state.current is not None else pick(app, rest, None, int)
    if session is None:
        app.say("/quit 要对着一个会话：先 /attach 一个。")
        return
    if app.running(session):
        app.say("%d 号还在跑。/esc 断掉再退。" % session)
        return
    app.release(session)
    app.say("%d 号退出了，桥那边也撒手了。" % session)


def switch(app, rest):
    target = app.target()
    if target is None:
        app.say("现在没对着会话也没草稿。/list 或 /new。")
        return
    if not rest:
        models = app.door.call("pools").get("models") or []
        app.picks = [model.get("address") for model in models]
        app.picks_kind = "switch"
        if not models:
            app.say("没有可用的模型。")
            return
        lines = ["模型，发 /switch 编号："]
        for index, model in enumerate(models, 1):
            lines.append("%d. %s  %s" % (index, model.get("address"), size(model.get("context_window"))))
        app.say("\n".join(lines))
        return
    chosen = pick(app, rest, "switch", str)
    if chosen is None:
        app.say("没这个编号。/switch 看清单。")
        return
    pair = split_address(chosen)
    if pair is None:
        app.say("要写成 provider:model。/switch 看清单。")
        return
    if target["draft"]:
        app.state.draft["model"] = chosen
        app.state.save()
        app.say("草稿的模型换成 %s 了。" % chosen)
        return
    session = target["session"]
    app.door.post("meta", {"name": "switch", "session": session, "content": {"session": session, "model": {"provider": pair[0], "model": pair[1]}}})
    app.say("%d 号换成 %s 了。" % (session, chosen))


def effort(app, rest):
    target = app.target()
    if target is None:
        app.say("现在没对着会话也没草稿。/list 或 /new。")
        return
    rungs = app.rungs(target)
    if not rest:
        if not rungs:
            app.say("这个模型不吃思考强度。")
            return
        app.picks = rungs
        app.picks_kind = "effort"
        app.say("档位，发 /effort 名字：\n" + " ".join(rungs))
        return
    chosen = pick(app, rest, "effort", str) or rest
    if rungs and chosen not in rungs:
        app.say("这个模型只认这些：%s" % " ".join(rungs))
        return
    if target["draft"]:
        app.state.draft["effort"] = chosen
        app.state.save()
        app.say("草稿的档位换成 %s 了。" % chosen)
        return
    session = target["session"]
    app.door.post("meta", {"name": "effort", "session": session, "content": {"session": session, "effort": chosen}})
    app.say("%d 号的档位换成 %s 了。" % (session, chosen))


def cwd(app, rest):
    target = app.target()
    if target is None:
        app.say("现在没对着会话也没草稿。/list 或 /new。")
        return
    here = app.where(target)
    if not rest:
        listing = app.door.ask("host", "list", {"path": "", "cwd": here or ""})
        names = [entry for entry in listing.get("entries") or [] if entry.get("dir")]
        app.picks = [entry["name"] for entry in names]
        app.picks_kind = "cwd"
        lines = ["%s 下的目录，发 /cwd 编号：" % (listing.get("dir") or here or "?")]
        if names:
            for index, entry in enumerate(names, 1):
                lines.append("%d. %s" % (index, entry["name"]))
        else:
            lines.append("（没有子目录）")
        lines.append("上一级：/cwd ..")
        lines.append("或者直接给路径：/cwd /某个/地方")
        app.say("\n".join(lines))
        return
    chosen = pick(app, rest, "cwd", str)
    if chosen is None:
        app.say("没这个编号。/cwd 看清单。")
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
        app.say("草稿的目录换成 %s 了。" % path)
        return
    session = target["session"]
    app.door.post("meta", {"name": "cwd", "session": session, "content": {"session": session, "path": path}})
    app.say("%d 号的目录换成 %s 了。" % (session, path))


def rename(app, rest):
    target = app.target()
    if target is None:
        app.say("现在没对着会话。/list 或 /new。")
        return
    if not rest:
        app.say("/name 后面跟一个名字。")
        return
    if target["draft"]:
        app.say("草稿还没有名字，等它建出来。")
        return
    session = target["session"]
    app.door.post("meta", {"name": "name", "session": session, "content": {"session": session, "name": rest}})
    app.say("%d 号改名叫「%s」了。" % (session, shorten(rest, 40)))


def profile(app, rest):
    if not app.state.draft:
        app.say("档只在建会话那一刻定下来，之后改不了。/new 立个草稿再挑。")
        return
    if not rest:
        profiles = app.door.call("pools").get("profiles") or []
        app.picks = profiles
        app.picks_kind = "profile"
        if not profiles:
            app.say("这个目录下没有可用的档。")
            return
        lines = ["档，发 /profile 编号："]
        for index, item in enumerate(profiles, 1):
            lines.append("%d. %s" % (index, item))
        app.say("\n".join(lines))
        return
    chosen = pick(app, rest, "profile", str)
    if chosen is None:
        app.say("没这个编号。/profile 看清单。")
        return
    app.state.draft["profile"] = chosen
    app.state.save()
    app.say("草稿的档换成 %s 了。" % chosen)


def usage(app, rest):
    target = app.target()
    if target is None or target["draft"]:
        app.say("还没有会话，没有账。")
        return
    session = target["session"]
    bill = app.door.call("usage", {"session": session})
    meta = app.door.call("meta", {"session": session})
    lines = ["%d 号 · %s" % (session, address(meta))]
    total = int(bill.get("total_tokens") or 0)
    if total:
        lines.append("token  %s（进 %s / 出 %s）" % (size(total), size(bill.get("prompt_tokens")), size(bill.get("completion_tokens"))))
        hit = round(float(bill.get("cache_hit_rate") or 0) * 100)
        if hit:
            lines.append("缓存  命中 %d%%" % hit)
    else:
        lines.append("token  还没有")
    if bill.get("cny") is not None:
        lines.append("花费  ¥%.3f / $%.3f" % (float(bill.get("cny") or 0), float(bill.get("usd") or 0)))
    context = bill.get("context")
    if context:
        window = app.window_of(address(meta))
        mark = "≈" if bill.get("estimated") else ""
        if window:
            lines.append("上下文  %s%s / %s（%d%%）" % (mark, size(context), size(window), round(100.0 * context / window)))
        else:
            lines.append("上下文  %s%s" % (mark, size(context)))
    rounds = sum(int(group.get("rounds") or 0) for group in bill.get("groups") or [])
    if rounds:
        lines.append("轮次  %d" % rounds)
    app.say("\n".join(lines))


def compact(app, rest):
    target = app.target()
    if target is None or target["draft"]:
        app.say("还没有会话，没什么可压的。")
        return
    session = target["session"]
    payload = {"session": session}
    if rest:
        payload["focus"] = rest
    app.door.subscribe("compact", payload)
    app.say("%d 号开始压缩了。" % session)


def esc(app, rest):
    target = app.target()
    if target is None or target["draft"]:
        app.say("现在没有正在跑的会话。")
        return
    session = target["session"]
    app.door.post("interrupt", {"session": session})
    app.say("%d 号断了。" % session)


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
