"""mypi's Telegram bridge: a front end that reads, not one that shows.

The bot talks to Telegram for updates and to the bridge's door for everything else. A hosted
session keeps one connection open, which is what holds it alive on the server while nobody
watches it; the session being talked to answers with its own words, the others report that
their round ended.
"""

import os
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from bridge import Door, Feed, Refused, said  # noqa: E402
from state import State  # noqa: E402
from telegram import Bot, BotError  # noqa: E402
from turn import BEAT, Turn, counts  # noqa: E402

# How many sessions the bot may hold at once; a fourth is refused, never bought at another's cost.
MOST = 3

# How long the door may take to answer one question.
PATIENCE = 60

# How often the bot says it is typing in the session being talked to.
TYPING = 4.0

HOST = os.environ.get("TG_BRIDGE_HOST", "127.0.0.1")
PORT = int(os.environ.get("TG_BRIDGE_PORT", "8737"))


def shorten(text, width=40):
    """One line, cut where a listing needs it, at a space when one is near."""
    line = " ".join((text or "").split())
    if len(line) <= width:
        return line
    cut = line[:width]
    space = cut.rfind(" ")
    if space > width // 2:
        cut = cut[:space]
    return cut + "…"


def size(count):
    """A byte or token count in the unit a person reads."""
    count = float(count or 0)
    for unit in ("", "K", "M", "G"):
        if count < 1024 or unit == "G":
            if unit == "":
                return "%d" % count
            return "%.1f%s" % (count, unit)
        count /= 1024.0
    return "%d" % count


def stamp_of(unix):
    """A Telegram `date`, written the way the server writes its own: `2026-11-23 02:58:13`."""
    if not unix:
        return None
    return time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(int(unix)))


def now():
    """This machine's moment, in the shape the server stamps its own rows with."""
    return time.strftime("%Y-%m-%d %H:%M:%S")


def clock(stamp):
    """`2026-11-24 15:12:59` as the short form: it is read on a phone."""
    if not stamp:
        return "—"
    return str(stamp)[5:16]


def address_of(meta):
    """A row's model as `provider:model`."""
    provider = (meta or {}).get("provider") or ""
    model = (meta or {}).get("model") or ""
    if provider and model:
        return "%s:%s" % (provider, model)
    return model or ""


def parse_address(text):
    """`provider:model`, split at the first colon only: a model id may carry more."""
    text = (text or "").strip()
    at = text.find(":")
    if at <= 0 or at == len(text) - 1:
        return None
    return text[:at], text[at + 1:]


class Hosted:
    """One session the bot holds: its connection, the round in it, and its own reader."""

    def __init__(self, app, session):
        self.app = app
        self.session = session
        self.feed = Feed(app.door, session).open()
        self.turn = Turn()
        self.dropped = False
        self.thread = threading.Thread(target=self.run, name="watch-%d" % session, daemon=True)
        self.thread.start()

    def run(self):
        typing_at = 0.0
        while not self.dropped:
            frame = self.feed.take(0.5)
            if frame is None:
                if self.dropped:
                    return
                # The door let the stream go: reopen it, which watches the session again.
                self.app.log("session %d: stream closed, reopening" % self.session)
                time.sleep(2)
                if self.dropped:
                    return
                self.feed = Feed(self.app.door, self.session).open()
                continue
            now = time.time()
            if frame != "idle":
                self.absorb(frame, now)
            if self.turn.due(now):
                self.report()
            if self.turn.running and now - typing_at >= TYPING and self.app.current() == self.session:
                typing_at = now
                self.app.bot.typing(self.app.chat)

    def absorb(self, frame, now):
        name = frame.get("name")
        payload = frame.get("payload") or {}
        if name == "notice":
            text = payload.get("text")
            self.app.say("%s%s" % (self.prefix(), said(text) if isinstance(text, dict) else str(text)))
            return
        if name == "signal":
            return
        if payload.get("ranges") is not None:
            self.close_round()
            return
        kind = payload.get("kind")
        if not self.turn.running:
            # A round opens with its first piece; a turn the loop opened itself is named by that piece's own frame.
            self.turn.begin(payload.get("turn"), name, now)
        if kind == "text":
            self.turn.say(
                str(payload.get("text") or "").replace("\r\n", "\n").replace("\r", "\n"),
                now,
            )
        elif kind == "tool_end":
            self.turn.tool(str(payload.get("name") or "tool"), payload.get("ok"))

    def report(self):
        """The call counts of a round still running: proof it is working, said at a beat."""
        body = counts(self.turn.drain())
        if body:
            self.app.say("%s%s" % (self.prefix(), body))

    def close_round(self):
        """The round is over: the one being talked to says its words, the others say they are done."""
        text = self.turn.text.strip()
        leftover = counts(self.turn.drain())
        self.turn.running = False
        if self.app.current() == self.session:
            body = "\n".join(part for part in (leftover, text) if part)
            self.app.say(body or "（这一轮没有正文）")
            return
        head = shorten(text.split("\n")[0] if text else "", 60)
        line = "%s这轮跑完了" % self.prefix()
        if head:
            line += "：%s" % head
        self.app.say(line)

    def prefix(self):
        """What tells a background session's lines from the one being talked to."""
        return "" if self.app.current() == self.session else "[%d] " % self.session

    def stop(self):
        self.dropped = True
        self.feed.close()


class App:
    def __init__(self, state):
        self.state = state
        self.door = Door(HOST, PORT, PATIENCE)
        self.hosted = {}
        self.chat = None
        self.picks = []
        self.picks_kind = None
        token = self.find_token()
        self.bot = Bot(token)
        self.state.bot_token = token

    def find_token(self):
        """The token: remembered, from the environment, or a `token.txt` beside this script."""
        token = self.state.bot_token or os.environ.get("TG_BOT_TOKEN")
        if token:
            return token.strip()
        beside = os.path.join(os.path.dirname(os.path.abspath(__file__)), "token.txt")
        if os.path.isfile(beside):
            with open(beside, encoding="utf-8") as f:
                token = f.read().strip()
            if token:
                return token
        raise SystemExit("没有 bot token：放一个 token.txt 在旁边，或者设 TG_BOT_TOKEN")

    def log(self, line):
        print("%s %s" % (time.strftime("%H:%M:%S"), line), flush=True)

    # --- saying --------------------------------------------------------------

    def say(self, text):
        if not self.chat or not text:
            return
        try:
            self.bot.send(self.chat, text)
        except BotError as e:
            self.log("send failed: %s" % e)

    def current(self):
        return self.state.current

    # --- holding -------------------------------------------------------------

    def bring(self):
        """Open a reader for every session the bot held when it last ran."""
        for session in list(self.state.hosted):
            if session not in self.hosted:
                self.hosted[session] = Hosted(self, session)

    def hold(self, session):
        """Start holding one session. Answers whether there was room for it."""
        if session in self.hosted:
            return True
        if len(self.hosted) >= MOST:
            return False
        self.hosted[session] = Hosted(self, session)
        self.state.host(session)
        return True

    def release(self, session):
        held = self.hosted.pop(session, None)
        if held:
            held.stop()
        self.state.drop(session)

    # --- commands ------------------------------------------------------------

    def handle(self, message):
        text = (message.get("text") or "").strip()
        if not text:
            self.say("暂时只认文字。")
            return
        if text.startswith("/"):
            head, _, rest = text.partition(" ")
            self.command(head[1:].split("@")[0].lower(), rest.strip())
            return
        self.talk(text, stamp_of(message.get("date")))

    def command(self, word, rest):
        table = {
            "help": self.cmd_help,
            "start": self.cmd_help,
            "list": self.cmd_list,
            "ls": self.cmd_list,
            "new": self.cmd_new,
            "attach": self.cmd_attach,
            "host": self.cmd_host,
            "quit": self.cmd_quit,
            "draft": self.cmd_draft,
            "switch": self.cmd_switch,
            "model": self.cmd_switch,
            "effort": self.cmd_effort,
            "cwd": self.cmd_cwd,
            "cd": self.cmd_cwd,
            "name": self.cmd_name,
            "profile": self.cmd_profile,
            "usage": self.cmd_usage,
            "compact": self.cmd_compact,
            "esc": self.cmd_esc,
            "stop": self.cmd_esc,
        }
        run = table.get(word)
        if run is None:
            self.say("没有 /%s 这个命令。/help 看有什么。" % word)
            return
        try:
            run(rest)
        except Refused as e:
            self.say("…%s" % e.message)
        except BotError as e:
            self.log("bot: %s" % e)
            self.say("Telegram 那边没发出去：%s" % e)

    def cmd_help(self, rest):
        self.say(
            "\n".join(
                [
                    "把话直接发过来，就是发给当前会话。命令：",
                    "",
                    "/list 会话列表",
                    "/attach 编号 换到那个会话",
                    "/new [目录] 立一份草稿",
                    "/draft 看草稿",
                    "/host 把当前会话交给后台",
                    "/quit 让当前会话退出",
                    "/switch [模型] 不带参数就列模型",
                    "/effort [档位] 不带参数就列档位",
                    "/profile [档] 只对草稿有用，不带参数就列",
                    "/cwd [路径] 不带参数就列目录",
                    "/name 名字",
                    "/usage 这个会话的账",
                    "/compact [重点] 压缩上下文",
                    "/esc 打断正在跑的这一轮",
                ]
            )
        )

    def cmd_list(self, rest):
        rows = self.door.call("sessions").get("rows") or []
        if not rows:
            self.say("还没有会话。/new 起一个。")
            return
        lines = []
        for row in rows[:30]:
            session = row.get("id")
            title = row.get("name") or shorten(row.get("first_user") or "", 28) or "（空会话）"
            mark = "▶" if session == self.state.current else ("●" if session in self.hosted else "·")
            lines.append("%s %d %s" % (mark, session, shorten(title, 34)))
            lines.append(
                "    %s · %s · %s"
                % (
                    clock(row.get("last_ts") or row.get("started_at")),
                    size(row.get("bytes")),
                    shorten(row.get("cwd") or "", 34),
                )
            )
        if len(rows) > 30:
            lines.append("…还有 %d 个" % (len(rows) - 30))
        lines.append("")
        lines.append("▶ 正在说　● 托管中　· 都没")
        self.say("\n".join(lines))

    def cmd_new(self, rest):
        self.state.new_draft(rest or None)
        draft = self.state.draft
        self.say(
            "草稿起来了：目录 %s、模型 %s、档位 %s、档 %s。\n改好之后直接说第一句话，会话就建出来了。"
            % (
                draft.get("cwd") or "（默认）",
                draft.get("model") or "（默认）",
                draft.get("effort") or "auto",
                draft.get("profile") or "（默认）",
            )
        )

    def cmd_draft(self, rest):
        draft = self.state.draft
        if not draft:
            self.say("现在没有草稿。/new 立一份。")
            return
        self.say(
            "草稿：\n目录 %s\n模型 %s\n档位 %s\n档 %s"
            % (
                draft.get("cwd") or "（默认）",
                draft.get("model") or "（默认）",
                draft.get("effort") or "auto",
                draft.get("profile") or "（默认）",
            )
        )

    def cmd_attach(self, rest):
        session = self.pick(rest, "list", int)
        if session is None:
            self.say("/attach 后面跟一个编号，编号在 /list 里。")
            return
        try:
            meta = self.door.call("meta", {"session": session})
        except Refused as e:
            self.say("…%s" % e.message)
            return
        if not self.hold(session):
            self.say("已经托管了 %d 个，放掉一个再换：/quit。" % MOST)
            return
        self.state.current = session
        self.state.save()
        self.say(
            "对着 %d 号说话了。\n%s · %s · %s"
            % (
                session,
                shorten(meta.get("name") or "（无名）", 30),
                address_of(meta),
                shorten(meta.get("cwd") or "", 40),
            )
        )

    def cmd_host(self, rest):
        session = self.state.current
        if session is None:
            self.say("现在没对着会话。/list 挑一个，或者 /new 起一个。")
            return
        if not self.hold(session):
            self.say("已经托管了 %d 个，放掉一个再来：/quit。" % MOST)
            return
        self.state.current = None
        self.state.save()
        self.say("%d 号交给后台了，跑完我说一声。\n/attach 换个会话，或者 /new 起一个。" % session)

    def cmd_quit(self, rest):
        session = self.state.current if self.state.current is not None else self.pick(rest, None, int)
        if session is None:
            self.say("/quit 要对着一个会话：先 /attach 一个。")
            return
        rounds = self.door.call("init", {"session": session}).get("structure") or []
        if rounds and self.running(session):
            self.say("%d 号还在跑。/esc 断掉再退。" % session)
            return
        self.release(session)
        self.say("%d 号退出了，桥那边也撒手了。" % session)

    def running(self, session):
        """Whether that session is mid-round, read from the one thing the door says without writing."""
        held = self.hosted.get(session)
        if held is not None:
            return held.turn.running
        return False

    def cmd_switch(self, rest):
        target = self.target()
        if target is None:
            self.say("现在没对着会话也没草稿。/list 或 /new。")
            return
        if not rest:
            models = self.door.call("pools").get("models") or []
            self.picks = [model.get("address") for model in models]
            self.picks_kind = "switch"
            if not models:
                self.say("没有可用的模型。")
                return
            lines = ["模型，发 /switch 编号："]
            for index, model in enumerate(models, 1):
                lines.append(
                    "%d. %s  %s"
                    % (index, model.get("address"), size(model.get("context_window")))
                )
            self.say("\n".join(lines))
            return
        chosen = self.pick(rest, "switch", str)
        if chosen is None:
            self.say("没这个编号。/switch 看清单。")
            return
        pair = parse_address(chosen)
        if pair is None:
            self.say("要写成 provider:model。/switch 看清单。")
            return
        if target["draft"]:
            self.state.draft["model"] = chosen
            self.state.save()
            self.say("草稿的模型换成 %s 了。" % chosen)
            return
        session = target["session"]
        self.door.post(
            "meta",
            {
                "name": "switch",
                "session": session,
                "content": {"session": session, "model": {"provider": pair[0], "model": pair[1]}},
            },
        )
        self.say("%d 号换成 %s 了。" % (session, chosen))

    def cmd_effort(self, rest):
        target = self.target()
        if target is None:
            self.say("现在没对着会话也没草稿。/list 或 /new。")
            return
        rungs = self.rungs(target)
        if not rest:
            if not rungs:
                self.say("这个模型不吃思考强度。")
                return
            self.picks = rungs
            self.picks_kind = "effort"
            self.say("档位，发 /effort 名字：\n" + " ".join(rungs))
            return
        chosen = self.pick(rest, "effort", str) or rest
        if rungs and chosen not in rungs:
            self.say("这个模型只认这些：%s" % " ".join(rungs))
            return
        if target["draft"]:
            self.state.draft["effort"] = chosen
            self.state.save()
            self.say("草稿的档位换成 %s 了。" % chosen)
            return
        session = target["session"]
        self.door.post(
            "meta",
            {"name": "effort", "session": session, "content": {"session": session, "effort": chosen}},
        )
        self.say("%d 号的档位换成 %s 了。" % (session, chosen))

    def rungs(self, target):
        """The rungs the target's model declares, from that model's own entry."""
        model = self.state.draft.get("model") if target["draft"] else None
        if not model and not target["draft"]:
            try:
                model = address_of(self.door.call("meta", {"session": target["session"]}))
            except Refused:
                return []
        if not model:
            try:
                model = self.door.call("config", {"key": "config.server.api.defaultModel"}).get("value")
            except Refused:
                return []
        if not model:
            return []
        try:
            entry = self.door.call("config", {"key": "models.model.%s" % model})
        except Refused:
            return []
        return [rung for rung in (entry.get("value") or {}).get("reasoningEfforts") or []]

    def cmd_cwd(self, rest):
        target = self.target()
        if target is None:
            self.say("现在没对着会话也没草稿。/list 或 /new。")
            return
        here = self.where(target)
        if not rest:
            listing = self.door.ask("host", "list", {"path": "", "cwd": here or ""})
            names = [entry for entry in listing.get("entries") or [] if entry.get("dir")]
            self.picks = [entry["name"] for entry in names]
            self.picks_kind = "cwd"
            lines = ["%s 下的目录，发 /cwd 编号：" % (listing.get("dir") or here or "?")]
            if names:
                for index, entry in enumerate(names, 1):
                    lines.append("%d. %s" % (index, entry["name"]))
            else:
                lines.append("（没有子目录）")
            lines.append("上一级：/cwd ..")
            lines.append("或者直接给路径：/cwd /某个/地方")
            self.say("\n".join(lines))
            return
        chosen = self.pick(rest, "cwd", str)
        if chosen is None:
            self.say("没这个编号。/cwd 看清单。")
            return
        if chosen == "..":
            path = os.path.dirname((here or "").rstrip("/")) or "/"
        elif os.path.isabs(chosen) or chosen.startswith("~"):
            path = chosen
        else:
            path = os.path.join(here or "", chosen)
        if target["draft"]:
            self.state.draft["cwd"] = path
            self.state.save()
            self.say("草稿的目录换成 %s 了。" % path)
            return
        session = target["session"]
        try:
            self.door.post(
                "meta",
                {"name": "cwd", "session": session, "content": {"session": session, "path": path}},
            )
        except Refused as e:
            self.say("…%s" % e.message)
            return
        self.say("%d 号的目录换成 %s 了。" % (session, path))

    def where(self, target):
        if target["draft"]:
            return self.state.draft.get("cwd") or ""
        try:
            return self.door.call("meta", {"session": target["session"]}).get("cwd") or ""
        except Refused:
            return ""

    def cmd_name(self, rest):
        target = self.target()
        if target is None:
            self.say("现在没对着会话。/list 或 /new。")
            return
        if not rest:
            self.say("/name 后面跟一个名字。")
            return
        if target["draft"]:
            self.say("草稿还没有名字，等它建出来。")
            return
        session = target["session"]
        self.door.post(
            "meta",
            {"name": "name", "session": session, "content": {"session": session, "name": rest}},
        )
        self.say("%d 号改名叫「%s」了。" % (session, shorten(rest, 40)))

    def cmd_profile(self, rest):
        if not self.state.draft:
            self.say("档只在建会话那一刻定下来，之后改不了。/new 立个草稿再挑。")
            return
        if not rest:
            profiles = self.door.call("pools").get("profiles") or []
            self.picks = profiles
            self.picks_kind = "profile"
            if not profiles:
                self.say("这个目录下没有可用的档。")
                return
            lines = ["档，发 /profile 编号："]
            for index, name in enumerate(profiles, 1):
                lines.append("%d. %s" % (index, name))
            self.say("\n".join(lines))
            return
        chosen = self.pick(rest, "profile", str)
        if chosen is None:
            self.say("没这个编号。/profile 看清单。")
            return
        self.state.draft["profile"] = chosen
        self.state.save()
        self.say("草稿的档换成 %s 了。" % chosen)

    def cmd_usage(self, rest):
        target = self.target()
        if target is None or target["draft"]:
            self.say("还没有会话，没有账。")
            return
        session = target["session"]
        bill = self.door.call("usage", {"session": session})
        meta = self.door.call("meta", {"session": session})
        lines = ["%d 号 · %s" % (session, address_of(meta))]
        total = int(bill.get("total_tokens") or 0)
        if total:
            lines.append(
                "token  %s（进 %s / 出 %s）"
                % (
                    size(total),
                    size(bill.get("prompt_tokens")),
                    size(bill.get("completion_tokens")),
                )
            )
            hit = round(float(bill.get("cache_hit_rate") or 0) * 100)
            if hit:
                lines.append("缓存  命中 %d%%" % hit)
        else:
            lines.append("token  还没有")
        if bill.get("cny") is not None:
            lines.append(
                "花费  ¥%.3f / $%.3f" % (float(bill.get("cny") or 0), float(bill.get("usd") or 0))
            )
        context = bill.get("context")
        if context:
            window = self.window_of(address_of(meta))
            mark = "≈" if bill.get("estimated") else ""
            if window:
                lines.append(
                    "上下文  %s%s / %s（%d%%）"
                    % (mark, size(context), size(window), round(100.0 * context / window))
                )
            else:
                lines.append("上下文  %s%s" % (mark, size(context)))
        rounds = sum(int(group.get("rounds") or 0) for group in bill.get("groups") or [])
        if rounds:
            lines.append("轮次  %d" % rounds)
        self.say("\n".join(lines))

    def window_of(self, model):
        if not model:
            return 0
        try:
            entry = self.door.call("config", {"key": "models.model.%s" % model})
        except Refused:
            return 0
        return int((entry.get("value") or {}).get("contextWindow") or 0)

    def cmd_compact(self, rest):
        target = self.target()
        if target is None or target["draft"]:
            self.say("还没有会话，没什么可压的。")
            return
        session = target["session"]
        payload = {"session": session}
        if rest:
            payload["focus"] = rest
        self.door.subscribe("compact", payload)
        self.say("%d 号开始压缩了。" % session)

    def cmd_esc(self, rest):
        target = self.target()
        if target is None or target["draft"]:
            self.say("现在没有正在跑的会话。")
            return
        session = target["session"]
        self.door.post("interrupt", {"session": session})
        self.say("%d 号断了。" % session)

    # --- talking -------------------------------------------------------------

    def talk(self, text, sent=None):
        if self.state.current is not None:
            self.send_to(self.state.current, text, sent)
            return
        if self.state.draft:
            self.spawn(text, sent)
            return
        self.say("现在没对着任何会话。\n/list 挑一个，或者 /new 起一个。")

    def send_to(self, session, text, sent=None):
        if not self.hold(session):
            self.say("已经托管了 %d 个，放掉一个再来：/quit。" % MOST)
            return
        # The model is told when it last spoke and when this line was sent, so a reply that waited
        # overnight reads as one: a chat has no clock the model can otherwise see.
        ask = {"session": session, "text": text}
        stamp = self.decorate(session, sent)
        ask.update(stamp)
        try:
            self.door.subscribe("submit", ask)
        except Refused as e:
            if e.code == "busy":
                self.door.post("mailbox", {"session": session, "text": text, **stamp})
                self.say("%d 号在跑，这句话排进去了。" % session)
                return
            self.say("…%s" % e.message)

    def decorate(self, session, sent):
        """The two moments a turn is bounded by, as the prefix and suffix the server takes."""
        return {
            "prefix": "你上次回复于：%s\n" % self.last_reply(session),
            "suffix": "\n本条消息发送于：%s（来自 telegram）" % (sent or now()),
        }

    def last_reply(self, session):
        """When this session's own words last landed: the newest assistant block's stamp."""
        try:
            blocks = self.door.call("init", {"session": session}).get("blocks") or []
        except Refused:
            return "没有过"
        newest = None
        for block in blocks:
            if block.get("kind") != "assistant":
                continue
            ts = block.get("ts")
            if ts and (newest is None or ts > newest):
                newest = ts
        return newest or "没有过"

    def spawn(self, text, sent=None):
        draft = self.state.draft
        ask = {"cwd": draft.get("cwd") or "~", "fallback": not draft.get("cwd")}
        if draft.get("model"):
            pair = parse_address(draft["model"])
            if pair:
                ask["model"] = {"provider": pair[0], "model": pair[1]}
        if draft.get("effort"):
            ask["effort"] = draft["effort"]
        if draft.get("profile"):
            ask["profile"] = draft["profile"]
        try:
            answer = self.door.post("init", ask)
        except Refused as e:
            self.say("建不出来：%s" % e.message)
            return
        session = answer.get("session")
        if not session:
            self.say("建不出来：桥没有给出编号。")
            return
        self.state.draft = None
        self.state.current = session
        self.state.save()
        if not self.hold(session):
            self.say("%d 号建好了，但托管满了，先 /quit 放掉一个。" % session)
            return
        note = "%d 号起来了。" % session
        if answer.get("cwd"):
            note += "\n目录落在 %s" % answer["cwd"]
        self.say(note)
        self.send_to(session, text, sent)

    # --- picking -------------------------------------------------------------

    def target(self):
        """Where a setting lands: the session being talked to, else the draft."""
        if self.state.current is not None:
            return {"session": self.state.current, "draft": False}
        if self.state.draft:
            return {"session": None, "draft": True}
        return None

    def pick(self, word, kind, cast):
        """A word as an index into what was last listed, else the word itself."""
        word = (word or "").strip()
        if not word:
            return None
        if kind and self.picks_kind == kind and self.picks:
            try:
                index = int(word)
            except ValueError:
                index = 0
            if 1 <= index <= len(self.picks):
                return cast(self.picks[index - 1])
        try:
            return cast(word)
        except (TypeError, ValueError):
            return None

    # --- the loop ------------------------------------------------------------

    def run(self):
        me = self.bot.me()
        self.log("bot @%s" % me.get("username"))
        self.bot.drop_webhook()
        self.bot.skip_backlog()
        if self.state.user:
            self.chat = self.state.user
            self.log("paired with %s" % self.chat)
        self.bring()
        while True:
            try:
                updates = self.bot.poll(30)
            except BotError as e:
                self.log("poll failed: %s" % e)
                time.sleep(3)
                continue
            for update in updates:
                message = update.get("message")
                if not message:
                    continue
                chat = message.get("chat") or {}
                sender = message.get("from") or {}
                if chat.get("type") != "private" or sender.get("is_bot"):
                    continue
                if self.state.user is None:
                    self.state.user = chat.get("id")
                    self.state.save()
                    self.chat = self.state.user
                    self.log("paired with %s" % self.chat)
                    self.say("绑上了。/help 看命令。")
                if chat.get("id") != self.state.user:
                    try:
                        self.bot.send(chat.get("id"), "这个 bot 不给别人用。")
                    except BotError:
                        pass
                    continue
                self.chat = self.state.user
                try:
                    self.handle(message)
                except Exception as e:  # one bad command must not stop the bot
                    self.log("handle failed: %r" % e)
                    self.say("这句话没办成：%s" % e)


def main(argv):
    state = State()
    if argv and argv[0] == "--say":
        # A one-shot line to the paired account: the only way to test from this side.
        app = App(state)
        if not state.user:
            raise SystemExit("还没有绑定的账号")
        app.bot.send(state.user, " ".join(argv[1:]))
        return 0
    App(state).run()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
