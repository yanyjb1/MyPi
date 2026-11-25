# mypi's Telegram front end: it talks to Telegram for updates and to the bridge's door for everything else.
#
# A hosted session keeps one connection open, which is what holds it alive on the server while nobody watches it.
# The session being talked to answers with its own words as each block lands; a hosted one only says its round ended.

import os
import sys
import threading
import time

import commands
from bridge import CLOSED, IDLE, Door, Refused, said
from show import address, now, shorten, split_address, stamp
from state import State
from telegram import Bot, BotError
from turn import Turn, counts

# How many sessions the bot may hold at once; a fourth is refused, never bought at another's cost.
MOST = 3

# How long the door may take to answer one question.
PATIENCE = 60

# How often the bot says it is typing in the session being talked to.
TYPING = 4.0

HOST = os.environ.get("TG_BRIDGE_HOST", "127.0.0.1")
PORT = int(os.environ.get("TG_BRIDGE_PORT", "8737"))

# The name this adapter answers to in adapters.yml; the door refuses any other.
ADAPTER = "telegram"

# How much of a round's words `/host` names when a phone must show only one line.
PREVIEW = 50


class Hosted:
    # One session the bot holds: its connection, the round in it, and its own reader.
    def __init__(self, app, session):
        self.app = app
        self.session = session
        self.feed = app.door.watch(session)
        self.turn = Turn()
        self.dropped = False
        self.typing = 0.0
        self.thread = threading.Thread(target=self.run, name="watch-%d" % session, daemon=True)
        self.thread.start()

    def run(self):
        while True:
            frame = self.feed.take(0.5)
            if frame is CLOSED or self.dropped:
                return
            if frame is IDLE:
                self.quiet()
                continue
            self.absorb(frame)

    # No frame arrived: a round still running says what it is doing, at a beat and no faster.
    def quiet(self):
        if self.app.current() != self.session:
            return
        now = time.time()
        if self.turn.due(now):
            self.app.say(counts(self.turn.drain(now)))
        if self.turn.running and now - self.typing >= TYPING:
            self.typing = now
            self.app.bot.typing(self.app.chat)

    def absorb(self, frame):
        name = frame.get("name")
        payload = frame.get("payload") or {}
        if name == "notice":
            text = payload.get("text")
            self.app.say("%s%s" % (self.prefix(), said(text) if isinstance(text, dict) else str(text)))
            return
        if name == "signal":
            return
        if payload.get("ranges") is not None:
            self.close_round(payload.get("turn"))
            return
        kind = payload.get("kind")
        if not self.turn.running:
            # A round opens with its first piece; a turn the loop opened itself is named by that piece's own frame.
            self.turn.begin(payload.get("turn"), name, time.time())
        if kind == "text":
            self.turn.say(str(payload.get("text") or "").replace("\r\n", "\n").replace("\r", "\n"))
        elif kind == "boundary":
            self.close_block()
        elif kind == "tool_start":
            # A call opening ends the prose before it: an adapter fed no `boundary` has no other end.
            self.close_block()
            self.turn.start(payload.get("call_id"), payload.get("name"))
        elif kind == "tool_end":
            self.turn.end(payload.get("call_id"), payload.get("ok"))

    # One block ended: its words leave as one message, and only the one being talked to hears them.
    def close_block(self):
        text = self.turn.flush()
        if text and self.app.current() == self.session:
            self.app.say(text)

    # The round is over: the one being talked to hears nothing more, a hosted one says it ended.
    def close_round(self, round_id):
        closed = self.turn.close(round_id)
        if closed is None:
            return
        unsaid, words = closed
        if self.app.current() == self.session:
            if unsaid:
                self.app.say(unsaid)
            return
        line = "会话#%d结束了，正文消息：%s" % (self.session, shorten(words, PREVIEW) or "（没有正文）")
        self.app.say("%s%s" % (self.prefix(), line))

    # What tells a background session's lines from the one being talked to.
    def prefix(self):
        return "" if self.app.current() == self.session else "[%d] " % self.session

    def stop(self):
        self.dropped = True
        self.feed.close()


class App:
    def __init__(self, state):
        self.state = state
        self.door = Door(HOST, PORT, ADAPTER, PATIENCE)
        self.hosted = {}
        self.chat = None
        self.picks = []
        self.picks_kind = None
        token = self.find_token()
        self.bot = Bot(token)
        self.state.bot_token = token

    # The token: remembered, from the environment, or a `token.txt` beside this script.
    def find_token(self):
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

    def say(self, text):
        if not self.chat or not text:
            return
        try:
            self.bot.send(self.chat, text)
        except BotError as e:
            self.log("send failed: %s" % e)

    def current(self):
        return self.state.current

    # Open a reader for every session the bot held when it last ran.
    def bring(self):
        for session in list(self.state.hosted):
            if session not in self.hosted:
                self.hosted[session] = Hosted(self, session)

    # Start holding one session. Answers whether there was room for it.
    def hold(self, session):
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

    # Whether that session is mid-round, read from the stream this bot already holds.
    def running(self, session):
        held = self.hosted.get(session)
        return held is not None and held.turn.running

    # Where a setting lands: the session being talked to, else the draft.
    def target(self):
        if self.state.current is not None:
            return {"session": self.state.current, "draft": False}
        if self.state.draft:
            return {"session": None, "draft": True}
        return None

    # The rungs the target's model declares, from that model's own entry.
    def rungs(self, target):
        model = self.state.draft.get("model") if target["draft"] else None
        if not model and not target["draft"]:
            try:
                model = address(self.door.call("meta", {"session": target["session"]}))
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

    def where(self, target):
        if target["draft"]:
            return self.state.draft.get("cwd") or ""
        try:
            return self.door.call("meta", {"session": target["session"]}).get("cwd") or ""
        except Refused:
            return ""

    def window_of(self, model):
        if not model:
            return 0
        try:
            entry = self.door.call("config", {"key": "models.model.%s" % model})
        except Refused:
            return 0
        return int((entry.get("value") or {}).get("contextWindow") or 0)

    def handle(self, message):
        text = (message.get("text") or "").strip()
        if not text:
            self.say("暂时只认文字。")
            return
        if text.startswith("/"):
            head, _, rest = text.partition(" ")
            commands.run(self, head[1:].split("@")[0].lower(), rest.strip())
            return
        self.talk(text, stamp(message.get("date")))

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
        ask = {"session": session, "text": text}
        ask.update(self.decorate(session, sent))
        try:
            self.door.subscribe("submit", ask)
        except Refused as e:
            if e.code == "busy":
                self.door.post("mailbox", {"session": session, "text": text, **ask})
                self.say("%d 号在跑，这句话排进去了。" % session)
                return
            self.say("…%s" % e.message)

    # The two moments a turn is bounded by, as the prefix and suffix the server takes: a chat has no clock the model can otherwise see.
    def decorate(self, session, sent):
        return {"prefix": "你上次回复于：%s\n" % self.last_reply(session), "suffix": "\n本条消息发送于：%s（来自 telegram）" % (sent or now())}

    # When this session's own words last landed: the newest assistant block's stamp.
    def last_reply(self, session):
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
            pair = split_address(draft["model"])
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
                except Exception as e:
                    # One bad command must not stop the bot.
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
