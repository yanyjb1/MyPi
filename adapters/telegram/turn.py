"""One hosted session's live round, folded to: its own words, and how its tools went."""

import time

# How often a round that is still running reports the calls it made.
BEAT = 15.0

# The pieces a round may carry that are not worth counting as "the agent is working".
QUIET = ("thinking", "boundary", "arguments", "tool_start", "tool_progress", "wake")


class Turn:
    """The round a feed is in: its text, its call counts, and when it last reported."""

    def __init__(self):
        self.running = False
        self.turn = None
        self.opened_by = "submit"
        self.text = ""
        self.calls = {}
        self.failed = {}
        self.last_beat = 0.0
        self.last_mark = 0.0

    def begin(self, turn, opened_by, at):
        self.running = True
        self.turn = turn
        self.opened_by = opened_by
        self.text = ""
        self.calls = {}
        self.failed = {}
        self.last_beat = at
        self.last_mark = at

    def say(self, text, at):
        self.text += text
        self.last_mark = at

    def tool(self, name, items):
        """One call counted once: `error` when any of the items it named failed."""
        failed = isinstance(items, list) and any(item.get("ok") is False for item in items)
        self.calls[name] = self.calls.get(name, 0) + 1
        if failed:
            self.failed[name] = self.failed.get(name, 0) + 1
        self.last_mark = time.time()

    def due(self, now, beat=BEAT):
        """Whether a round still running owes a line: it called something, and the beat ran out."""
        return self.running and bool(self.calls) and now - self.last_beat >= beat

    def drain(self):
        """The counts since the last report, one `(name, ok, failed)` per call name, busiest first."""
        rows = [(name, count, self.failed.get(name, 0)) for name, count in self.calls.items()]
        rows.sort(key=lambda row: (-row[1], row[0]))
        self.calls = {}
        self.failed = {}
        self.last_beat = time.time()
        return rows


def counts(rows):
    """Counts as the lines a person reads."""
    lines = []
    for name, count, failed in rows:
        lines.append("%s · ok · %d" % (name, count - failed))
        if failed:
            lines.append("%s · error · %d" % (name, failed))
    return "\n".join(lines)
