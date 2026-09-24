#!/usr/bin/env python3
"""Build a tiny DB for /compact debugging (cheap, small, deterministic).

20 rounds of everyday chat with a few tool calls sprinkled in — sized so
that with a small `compact.retain_tail` (e.g. 300) a /compact run costs
pennies while still exercising: block grouping, tool-pair protection,
the replay request, and the shrink check.

Usage:
    python3 examples/gen_compact_dbg.py /tmp/mypi-compact/db.sqlite

Then point a debug session at it (or copy over the live session DB) and
set in config.yaml:
    compact:
      retain_tail: 300
"""
import json, os, sqlite3, sys

db = sys.argv[1] if len(sys.argv) > 1 else '/tmp/mypi-compact/db.sqlite'
os.makedirs(os.path.dirname(db), exist_ok=True)
if os.path.exists(db):
    os.remove(db)
for suf in ('-wal', '-shm'):
    if os.path.exists(db + suf):
        os.remove(db + suf)

conn = sqlite3.connect(db)
conn.execute("PRAGMA journal_mode=WAL")
conn.executescript("""
CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY, name TEXT, started_at TEXT, cwd TEXT, leaf INTEGER
);
CREATE TABLE IF NOT EXISTS entries (
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    seq INTEGER NOT NULL, ts TEXT NOT NULL, kind TEXT NOT NULL,
    payload TEXT NOT NULL, parent_seq INTEGER,
    PRIMARY KEY (session_id, seq)
);
CREATE TABLE IF NOT EXISTS cwd_history (
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    seq INTEGER NOT NULL, ts TEXT NOT NULL, cwd TEXT NOT NULL
);
""")

TOPICS = (
    ('render', '块缓存的 LRU 淘汰为什么用行预算而不是块数？'),
    ('theme', '主题 epoch 进缓存键的语义是什么？'),
    ('artifact', '巨物占位符在重放请求里会不会被二次展开？'),
    ('bash_guard', 'temp 目录的许可区为什么不允许根目录本身？'),
    ('prefix', '前缀缓存命中需要消息逐字节一致吗？'),
    ('shrink', 'shrink 检查比较的是估算 token 还是网关返回值？'),
    ('profile', 'profile 切换的时机约束为什么卡在重建点？'),
    ('context_tool', 'context 工具的 anchor 支持哪些寻址形式？'),
)

rows = []  # (kind, payload)
artifacts = []


def user(text):
    rows.append(('user', json.dumps({'content': text}, ensure_ascii=False)))


def assistant(text):
    rows.append(('assistant', json.dumps(
        {'content': text, 'usage': None, 'reasoning': None}, ensure_ascii=False)))


def tool_pair(call_id, name, args, result, ok=True):
    rows.append(('tool_request', json.dumps(
        {'call_id': call_id, 'name': name, 'args': args, 'intent': f'调试用 {name} 调用'},
        ensure_ascii=False)))
    rows.append(('tool_result', json.dumps(
        {'call_id': call_id, 'name': name, 'ok': ok, 'result': result},
        ensure_ascii=False)))


rounds = 0
cid = 0
for i, (topic, q) in enumerate(TOPICS):
    user(f'[{i+1}] {q}')
    assistant(f'关于 {topic}：这里是一段两三行的解释，说明 {q} 的关键点，'
              f'并带一个可以继续追问的细节钩子（第 {i+1} 轮）。')
    rounds += 1
    if i in (2, 6):  # a couple of read calls mid-history
        cid += 1
        tool_pair(f'call-{cid}', 'read',
                  json.dumps({'intent': '看代码', 'path': f'src/demo_{i}.rs'}),
                  '1: fn main() {}\n2: // demo\n')
    if i in (3, 7):  # a bash call
        cid += 1
        tool_pair(f'call-{cid}', 'bash',
                  json.dumps({'intent': '跑测试', 'command': 'cargo test --lib'}),
                  f'test result: ok. {40+i} passed')

# trailing tool result that never got its reply (leaf on a ToolResult is legal)
sid = conn.execute(
    "INSERT INTO sessions (id, name, started_at, cwd, leaf) "
    "VALUES (1, 'compact-dbg', '2026-09-24 10:00:00', '/tmp/mypi-compact', NULL)"
).lastrowid
conn.execute("INSERT INTO cwd_history (session_id, seq, ts, cwd) "
             "VALUES (1, 0, '2026-09-24 10:00:00', '/tmp/mypi-compact')")

parent = None
ts = '2026-09-24 10:00:00'
for seq, (kind, payload) in enumerate(rows, start=1):
    conn.execute(
        'INSERT INTO entries (session_id, seq, ts, kind, payload, parent_seq) '
        'VALUES (1, ?, ?, ?, ?, ?)', (seq, ts, kind, payload, parent))
    parent = seq
conn.execute('UPDATE sessions SET leaf = ? WHERE id = 1', (parent,))
conn.commit()

n = conn.execute('SELECT COUNT(*) FROM entries WHERE session_id=1').fetchone()[0]
total = conn.execute("SELECT SUM(LENGTH(payload)) FROM entries WHERE session_id=1").fetchone()[0]
print(f'{n} entries, {total} payload bytes → {db}')
print('config.yaml 建议：\ncompact:\n  retain_tail: 300')
