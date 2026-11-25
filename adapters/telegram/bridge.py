# The door: one frame with its answer, and the one connection a hosted session streams down.

import json
import queue
import threading
import urllib.error
import urllib.request

# The stream ended: a closed connection is reported, never reopened.
CLOSED = object()

# No frame arrived within the wait: the caller keeps whatever it was doing.
IDLE = object()


class Refused(Exception):
    # The door or the server said no; `code` is the machine key, `message` is what to show.
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code
        self.message = message


def said(text):
    # One keyed refusal as a line a person can read.
    if not isinstance(text, dict):
        return str(text)
    if text.get("text"):
        return text["text"]
    key = text.get("key") or text.get("code") or "refused"
    args = text.get("args") or {}
    if not args:
        return key
    return key + " " + json.dumps(args, ensure_ascii=False)


class Door:
    # One address, two methods: POST for a frame, and the stream a hosted session answers on.
    def __init__(self, host, port, adapter, timeout=60):
        self.base = "http://%s:%d/" % (host, port)
        self.adapter = adapter
        self.timeout = timeout

    def ask(self, kind, name, payload=None, timeout=None):
        request = self.request(kind, name, payload)
        try:
            with urllib.request.urlopen(request, timeout=timeout or self.timeout) as res:
                answer = json.loads(res.read())
        except urllib.error.HTTPError as e:
            answer = self._read(e)
            code, message = self._reason(answer, "http_%d" % e.code)
            raise Refused(code, message)
        except (urllib.error.URLError, OSError) as e:
            raise Refused("unreachable", "桥没有应答：%s" % e)
        except ValueError:
            raise Refused("unreadable", "桥回了读不懂的东西")
        code, message = self._reason(answer, None)
        if code:
            raise Refused(code, message)
        return answer

    def call(self, name, payload=None):
        return self.ask("call", name, payload)

    def post(self, name, payload=None):
        return self.ask("post", name, payload)

    def subscribe(self, name, payload=None):
        return self.ask("subscribe", name, payload)

    def watch(self, session):
        # The stream one held session answers on, already reading.
        return Feed(self, session).open()

    def request(self, kind, name, payload):
        body = json.dumps({"kind": kind, "name": name, "payload": payload or {}}, ensure_ascii=False).encode()
        return urllib.request.Request(self.base, data=body, method="POST", headers={"content-type": "application/json", "X-Bridge-Adapter": self.adapter})

    @staticmethod
    def _read(error):
        try:
            return json.loads(error.read())
        except Exception:
            return {}

    @staticmethod
    def _reason(answer, fallback):
        # The pair a refusal carries: the machine code, and the line to show. No refusal is `(None, None)`.
        if not isinstance(answer, dict):
            return fallback, fallback
        bad = answer.get("error")
        if not isinstance(bad, dict):
            return fallback, fallback
        # The door refuses with `code`; the server refuses with a keyed text under `key`.
        code = bad.get("code") or bad.get("key") or fallback
        return code, said(bad)


class Feed:
    # One hosted session's frames, read on their own thread and handed out in order.
    def __init__(self, door, session):
        self.session = session
        self._door = door
        self._frames = queue.Queue()
        self._closed = False
        self._thread = None

    def open(self):
        self._thread = threading.Thread(target=self._run, name="feed-%d" % self.session, daemon=True)
        self._thread.start()
        return self

    def _run(self):
        request = self._door.request("subscribe", "watch", {"session": self.session})
        try:
            # No read timeout: this connection stays open for as long as we host the session.
            with urllib.request.urlopen(request, timeout=None) as res:
                # Read by line, never by size: the door sends one frame per line and flushes it, and a size-kinded read would hold a whole short round until later frames filled the buffer.
                for raw in res:
                    if self._closed:
                        break
                    line = raw.decode("utf-8", "replace").rstrip("\r\n")
                    if not line.startswith("data: "):
                        continue
                    try:
                        self._frames.put(json.loads(line[6:]))
                    except ValueError:
                        pass
        except Exception:
            pass
        finally:
            self._frames.put(CLOSED)

    def take(self, timeout=0.5):
        # The next frame, `CLOSED` once the stream ended, or `IDLE` when nothing arrived in time.
        if self._closed:
            return CLOSED
        try:
            return self._frames.get(timeout=timeout)
        except queue.Empty:
            return IDLE

    def close(self):
        self._closed = True
        self._frames.put(CLOSED)
