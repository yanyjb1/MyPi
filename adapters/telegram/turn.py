# One hosted session's round: the block being written, the calls it made, its receipt.

# How often a round that is still running reports the calls it made.
BEAT = 15.0


class Turn:
    # The round a stream is in: the block being written, and how its calls went.
    def __init__(self):
        self.running = False
        self.id = None
        self.opened_by = "submit"
        self.block = ""
        self.last = ""
        self.names = {}
        self.count = {}
        self.beat = 0.0

    def begin(self, round_id, opened_by, at):
        self.running = True
        self.id = round_id
        self.opened_by = opened_by
        self.block = ""
        self.last = ""
        self.names = {}
        self.count = {}
        self.beat = at

    def say(self, text):
        self.block += text

    # The block that just ended, emptied: one block is one message, never joined to another.
    def flush(self):
        text = self.block.strip()
        self.block = ""
        if text:
            self.last = text
        return text

    # A call opens with its name, which the wire sends only here; a call closes with its items' outcomes, named by the start it answers.
    def start(self, call_id, name):
        if call_id:
            self.names[call_id] = name or "tool"

    def end(self, call_id, items):
        name = self.names.pop(call_id, None) or "tool"
        failed = isinstance(items, list) and any(item.get("ok") is False for item in items)
        row = self.count.get(name, [0, 0])
        row[1 if failed else 0] += 1
        self.count[name] = row

    # Whether a round still running owes a line: it called something, and the beat ran out.
    def due(self, now, beat=BEAT):
        return self.running and bool(self.count) and now - self.beat >= beat

    # The counts since the last report, busiest first.
    def drain(self, at):
        rows = [(name, ok, failed) for name, (ok, failed) in self.count.items()]
        rows.sort(key=lambda row: (-(row[1] + row[2]), row[0]))
        self.count = {}
        self.beat = at
        return rows

    # `(unsaid, words)`: what the round never sent, and the last block it did; None once this receipt is its second, or another round's.
    def close(self, round_id):
        if not self.running or round_id != self.id:
            return None
        unsaid = self.block.strip()
        words = unsaid or self.last
        self.running = False
        self.block = ""
        self.last = ""
        self.names = {}
        self.count = {}
        return unsaid, words


# The counts of one round as the line a person reads.
def counts(rows):
    if not rows:
        return ""
    parts = []
    for name, ok, failed in rows:
        part = "%s ×%d" % (name, ok + failed)
        if failed:
            part += "（%d 失败）" % failed
        parts.append(part)
    return "跑着：" + " · ".join(parts)
