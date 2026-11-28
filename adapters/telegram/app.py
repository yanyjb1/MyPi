# mypi's Telegram front end: it talks to Telegram for updates and to the bridge's door for everything else.
#
# The session being talked to keeps one stream open, and what it writes is said as each block lands. A session
# handed over is the bridge's to keep booked; it has no stream of ours, and one poll says when its round ended.

import os
import sys
import threading
import time

import attach
import commands
import read
import words
from bot import Bot, BotError
from bridge import CLOSED, IDLE, Door, Refused, said
from show import address, now, shorten, split_address, stamp
from state import State
from turn import Turn, counts

# How many sessions the bot may hand to the bridge; a fourth is refused, never bought at another's cost.
MOST = 3

# How long the door may take to answer one question.
PATIENCE = 60

# How often the bot says it is typing in the session being talked to.
TYPING = 4.0

# How long the bot waits before asking for a dropped stream again.
RECONNECT = 2

# How long the loop waits before reading the queue again after a failure it did not expect.
RETRY = 5

# The most the bot waits between two tries at a stream the proxy keeps dropping.
RECONNECT_CAP = 30

# How often a session handed to the bridge is asked whether its round is still running.
POLL = 3.0

HOST = os.environ.get("TG_BRIDGE_HOST", "127.0.0.1")
PORT = int(os.environ.get("TG_BRIDGE_PORT", "8737"))

# The name this adapter answers to in adapters.yml; the door refuses any other.
ADAPTER = "telegram"

# How much of a round's words a phone shows when one line has to carry them.
PREVIEW = 50

# This script's own path, so the line telling the model how to send a file is one it can run as it stands.
SCRIPT = os.path.abspath(__file__)


class Live:
    # The session being talked to: its stream, the round in it, and its own reader.

    def __init__(self, app, session):
        self.app = app
        self.session = session
        self.feed = app.door.watch(session)
        self.turn = Turn()
        self.dropped = False
        self.typing = 0.0
        self.rturn = None
        self.seen = 0
        self.skip = 0
        self.thread = threading.Thread(target=self.run, name=f"live-{session}", daemon=True)
        self.thread.start()

    # A stream that ends is asked for again: a bridge restart only means the connection went.
    def run(self):
        patience = RECONNECT
        while True:
            frame = self.feed.take(0.5)
            if self.dropped:
                return
            if frame is CLOSED:
                self.app.log(words.t("stream_lost", session=self.session))
                time.sleep(patience)
                patience = min(patience * 2, RECONNECT_CAP)
                # The door replays a running round to whoever asks again: say only what this bot has not said.
                self.skip = self.seen
                if self.dropped:
                    return
                self.feed = self.app.door.watch(self.session)
            patience = RECONNECT
            if frame is IDLE:
                self.quiet()
                continue
            self.absorb(frame)

    # No frame arrived: a round still running says what it is doing, at a beat and no faster.
    def quiet(self):
        now = time.time()
        if self.turn.due(now):
            self.app.say(counts(self.turn.drain(now)))
        if self.turn.running and now - self.typing >= TYPING:
            self.typing = now
            self.app.bot.typing(self.app.chat)

    def absorb(self, frame):
        name = frame.get("name")
        payload = frame.get("payload") or {}

        if name == "signal":
            return
        # The door buffers a running round frame by frame and replays it whole to a reconnect: what was said is counted, and swallowed on its way back.
        turn = payload.get("turn")
        if turn is not None and turn != self.rturn:
            self.rturn = turn
            self.seen = 0
            self.skip = 0
        if payload.get("ranges") is not None:
            self.skip = 0
            self.close_round(turn)
            return
        if self.skip:
            self.skip -= 1
            return
        self.seen += 1
        if name == "notice":
            text = payload.get("text")
            self.app.say(said(text) if isinstance(text, dict) else str(text))
            return
        kind = payload.get("kind")
        if not self.turn.running:
            # A round opens with its first piece; a turn the loop opened itself is named by that piece's own frame.
            self.turn.begin(turn, name, time.time())
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

    # One block ended: its words leave as one message, so a long round is several of them.
    def close_block(self):
        text = self.turn.flush()
        if text:
            self.app.say(text)

    # The round is over: the tail no block ended still leaves whole.
    def close_round(self, round_id):
        closed = self.turn.close(round_id)
        if closed is None:
            return
        unsaid, _ = closed
        if unsaid:
            self.app.say(unsaid)

    def stop(self):
        self.dropped = True
        self.feed.close()


class App:
    def __init__(self, state):
        self.state = state
        self.door = Door(HOST, PORT, ADAPTER, PATIENCE)
        self.lives = {}
        self.turns = {}
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
        raise SystemExit(words.t("no_token"))

    # The language the whole bot speaks, read from the server once at startup.
    def speak(self):
        try:
            answer = self.door.call("config", {"key": "config.ui.lang"})
        except Refused as e:
            self.log(f"cannot read config.ui.lang: {e.message}")
            return
        words.choose(answer.get("value"))

    def log(self, line):
        print(f"{time.strftime('%H:%M:%S')} {line}", flush=True)

    def say(self, text):
        if not self.chat or not text:
            return
        try:
            self.bot.send(self.chat, text)
        except BotError as e:
            self.log(f"send failed: {e}")

    def current(self):
        return self.state.current

    # Say what a session writes as it writes it: one stream, which is what keeps it alive on the server.
    def listen(self, session):
        if session not in self.lives:
            self.lives[session] = Live(self, session)

    # Hand a session to the bridge: its stream ends here, and the door keeps it booked instead.
    def host(self, session):
        if session in self.state.hosted:
            return True
        if len(self.state.hosted) >= MOST:
            return False
        try:
            self.door.ask("host", "attach", {"session": session})
        except Refused as e:
            self.log(f"cannot host {session}: {e.message}")
            return False
        live = self.lives.pop(session, None)
        if live:
            # The round it was in when handed over is the truth the first poll compares against.
            self.turns[session] = live.turn.running
            live.stop()
        if self.state.current == session:
            self.state.current = None
        self.state.host(session)
        return True

    # Take a session back from the door: whoever watches it now is what keeps it alive.
    def unhost(self, session):
        if session not in self.state.hosted:
            return
        try:
            self.door.ask("host", "release", {"session": session})
        except Refused:
            pass
        self.state.drop(session)
        self.turns.pop(session, None)

    # Forget it entirely: no stream of ours, nothing kept by the door.
    def release(self, session):
        live = self.lives.pop(session, None)
        if live:
            live.stop()
        try:
            self.door.ask("host", "release", {"session": session})
        except Refused:
            pass
        self.state.drop(session)
        self.turns.pop(session, None)

    # Hand every session the bot held when it last ran back to the door, so the poll still reports their rounds.
    def bring(self):
        for session in list(self.state.hosted):
            try:
                self.door.ask("host", "attach", {"session": session})
            except Refused as e:
                self.log(f"cannot host {session}: {e.message}")
                self.state.drop(session)

    # Whether that session is mid-round, as the door last said.
    def running(self, session):
        live = self.lives.get(session)
        if live is not None:
            return live.turn.running
        return self.turns.get(session, False)

    # Ask at once and then at a beat, so a session just handed over is reported the moment its round ends.
    def poll(self):
        while True:
            try:
                self.check()
            except Exception as e:  # noqa: BLE001 - a kept session outlives any one question about it
                self.log(f"check failed: {e!r}")
            time.sleep(POLL)

    # What the door says of every kept session now: one whose round ended is said once, with the words it wrote.
    def check(self):
        if not self.state.hosted:
            self.turns.clear()
            return
        try:
            listing = self.door.ask("host", "list")
        except Refused:
            return
        running = {item.get("session"): bool(item.get("turn")) for item in listing.get("sessions") or []}
        for session in list(self.state.hosted):
            now_ = running.get(session, False)
            was = self.turns.get(session, False)
            self.turns[session] = now_
            if was and not now_ and self.current() != session:
                text = read.newest_words(self.door, session)
                line = words.t("round_ended", session=session, words=shorten(text, PREVIEW) or words.t("no_words"))
                self.say(f"[{session}] {line}")

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
            entry = self.door.call("config", {"key": f"models.model.{model}"})
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
            entry = self.door.call("config", {"key": f"models.model.{model}"})
        except Refused:
            return 0
        return int((entry.get("value") or {}).get("contextWindow") or 0)

    def handle(self, message):
        try:
            item = attach.receive(self.bot, message)
        except BotError as e:
            self.say(words.t("attach_failed", words=e))
            return
        text = (message.get("text") or message.get("caption") or "").strip()
        # An attachment is a message to the model whatever its caption says, and the line naming its path rides with it.
        if item is not None:
            note = attach.note(item, SCRIPT)
            self.talk(f"{text}\n\n{note}" if text else note, stamp(message.get("date")))
            return
        kind = attach.untaken(message)
        if kind:
            self.say(words.t("attach_untaken", kind=kind))
            return
        if not text:
            self.say(words.t("text_only"))
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
        self.say(words.t("no_current"))

    def send_to(self, session, text, sent=None):
        self.listen(session)
        ask = {"session": session, "text": text}
        ask.update(self.decorate(session, sent))
        try:
            self.door.subscribe("submit", ask)
        except Refused as e:
            if e.code == "busy":
                self.door.post("mailbox", {"session": session, "text": text, **ask})
                self.say(words.t("queued", session=session))
                return
            self.say(f"…{e.message}")

    # The two moments a turn is bounded by, as the prefix and suffix the server takes: a chat has no clock the model can otherwise see.
    def decorate(self, session, sent):
        return {"prefix": words.t("reply_prefix", at=self.last_reply(session)), "suffix": words.t("message_suffix", at=sent or now())}

    # When this session's own words last landed: the newest assistant block's stamp, read the same way `/attach` reads its words.
    def last_reply(self, session):
        return read.newest_stamp(self.door, session) or words.t("never")

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
            self.say(words.t("spawn_failed", words=e.message))
            return
        session = answer.get("session")
        if not session:
            self.say(words.t("spawn_no_id"))
            return
        self.state.draft = None
        self.state.current = session
        self.state.save()
        self.listen(session)
        note = words.t("spawned", session=session)
        if answer.get("cwd"):
            note += words.t("spawn_cwd", cwd=answer["cwd"])
        self.say(note)
        self.send_to(session, text, sent)

    def run(self):
        me = self.bot.me()
        self.log(f"bot @{me.get('username')}")
        self.bot.drop_webhook()
        self.bot.skip_backlog()
        self.speak()
        if self.state.user:
            self.chat = self.state.user
            self.log(f"paired with {self.chat}")
        self.bring()
        if self.state.current is not None:
            self.listen(self.state.current)
        threading.Thread(target=self.poll, name="poll", daemon=True).start()

        while True:
            try:
                self.cycle()
            except Exception as e:  # noqa: BLE001 - a long conversation outlives any one failure of the loop
                self.log(f"loop failed: {e!r}")
                time.sleep(RETRY)

    # One read of the phone's queue, every message in it answered: a failure here is the loop's to retry, not the process's to die of.
    def cycle(self):
        for update in self.bot.poll(30):
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
                self.log(f"paired with {self.chat}")
                self.say(words.t("paired"))
            if chat.get("id") != self.state.user:
                try:
                    self.bot.send(chat.get("id"), words.t("not_yours"))
                except BotError:
                    pass
                continue
            self.chat = self.state.user
            try:
                self.handle(message)
            except Exception as e:  # noqa: BLE001 - one bad command must not stop the bot
                self.log(f"handle failed: {e!r}")
                self.say(words.t("handle_failed", words=e))


def main(argv):
    # A line the console cannot encode is a line lost: a front end started without a terminal runs on a code page, and `zh` does not fit in one. The log survives, the thread stays up.
    hook = getattr(sys.stdout, "reconfigure", None)
    if hook:
        hook(errors="replace")
    state = State()

    if argv and argv[0] in ("--say", "--file"):
        return once(state, argv)

    App(state).run()
    return 0


# A one-shot line or file to the paired account: what a model holding a path can run, and the only way to test from this side.
def once(state, argv):
    app = App(state)
    app.speak()
    if not state.user:
        raise SystemExit(words.t("no_pairing"))
    app.chat = state.user
    if argv[0] == "--say":
        app.bot.send(state.user, " ".join(argv[1:]))
        return 0
    path, caption = options(argv[1:])
    if not path or not os.path.isfile(path):
        raise SystemExit(words.t("file_missing", path=path or ""))
    name = os.path.basename(path)
    app.bot.send_file(state.user, path, caption, photo=is_photo(name))
    app.bot.send(state.user, words.t("file_sent", name=name))
    print(words.t("file_sent", name=name))
    return 0


# `--text` says what the file is; the path is the one argument that must be there.
def options(argv):
    rest = list(argv)
    path = None
    caption = None
    while rest:
        one = rest.pop(0)
        if one == "--text" and rest:
            caption = rest.pop(0)
        elif path is None:
            path = one
        elif caption is None:
            caption = one
        else:
            caption += " " + one
    return path, caption


# Whether this file goes as a picture: the four Telegram shows in place, everything else as a document.
def is_photo(name):
    return os.path.splitext(name)[1].lower() in (".png", ".jpg", ".jpeg", ".gif", ".webp")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))