#!/usr/bin/env python3
"""Build a bench DB: one session with ~8000 mixed entries of random content.

Writes to $1 (default /tmp/mypi-bench/sessions.db). Uses the real schema
(store.rs) and payload formats (entry.rs) so the TUI loads it unmodified.
"""
import json, os, random, sqlite3, string, sys, math

random.seed(20260924)
db = sys.argv[1] if len(sys.argv) > 1 else '/tmp/mypi-bench/sessions.db'
os.makedirs(os.path.dirname(db), exist_ok=True)
if os.path.exists(db):
    os.remove(db)
for suf in ('-wal', '-shm'):
    if os.path.exists(db + suf):
        os.remove(db + suf)

N = int(os.environ.get('N', '8000'))
TARGET_BYTES = int(os.environ.get('BYTES', '12000000'))  # ~12 MB of text total (typical long-lived session scale)

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
CREATE TABLE IF NOT EXISTS artifacts (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    name TEXT NOT NULL,
    total_lines INTEGER NOT NULL,
    content TEXT NOT NULL
);
""")
# store.rs expects a `leaf` column; created above. Column order in CREATE
# matches migrate() — verify:
cols = [r[1] for r in conn.execute("PRAGMA table_info(sessions)")]
assert cols == ['id', 'name', 'started_at', 'cwd', 'leaf'], cols

WORDS = ('渲染 主题 缓存 token 帧率 滚动 高亮 列表 边框 diff 内存 epoch '
         'syntect markdown palette transcript viewport budget evict LRU '
         'row spans gutter italic fenced inline quote heading').split()

def lorem(n):
    return ' '.join(random.choice(WORDS) for _ in range(n))

def rand_text(paras, wpl):
    out = []
    for _ in range(paras):
        out.append(lorem(random.randint(wpl[0], wpl[1])))
    return '\n\n'.join(out)

def md_sample(i):
    """A realistic markdown turn: headings, lists, code, quote."""
    parts = [f'## 第 {i} 节：{lorem(3)}',
             f'{rand_text(2, (8, 20))}',
             f'1. {lorem(6)}\n2. {lorem(6)}\n3. {lorem(5)}',
             f'- {lorem(4)}\n- `{random.choice(WORDS)}_{i}` 内联代码 {lorem(3)}',
             f'> {lorem(10)}',
             '```rust',
             f'fn demo_{i}(seed: u64) -> u64 {{',
             '    let mut h = 5381u64;',
             '    for b in seed.to_le_bytes() {',
             '        h = h.wrapping_mul(33).wrapping_add(b as u64);',
             '    }',
             f'    h // {lorem(3)}',
             '}',
             '```']
    return '\n\n'.join(parts)

def rand_chars(n):
    pool = string.ascii_letters + string.digits + '，。：；喵'
    return ''.join(random.choice(pool) for _ in range(n))

ARTIFACT_HEAD = (
    '[工具输出共 {lines} 行 / {kb:.0f}KB，过大已存为巨物 #{aid}（{tool}）。'
    '取用：#{aid}（等价文件内容，参与管道：#{aid} | grep 关键词 | head -50；'
    '裸 #{aid} 无过滤会再次巨物化）。前 3 行：'
)

def artifact_placeholder(aid, tool, content):
    lines = content.count(chr(10)) + 1
    head = '\n'.join(content.splitlines()[:3])
    return ARTIFACT_HEAD.format(lines=lines, kb=len(content.encode()) / 1024,
                                aid=aid, tool=tool) + '\n' + head

# avg bytes per entry so total lands near TARGET_BYTES
avg = TARGET_BYTES / N
rows = []
artifacts = []  # (aid, tool_name, full_content) — spilled to the artifacts table
aid_counter = [1]
seq = 0
i = 0
while seq < N:
    kind = random.choices(
        ['user', 'assistant', 'tool_pair', 'system', 'error', 'big_md'],
        weights=[22, 26, 30, 8, 4, 10])[0]
    ts = '2026-09-24 09:00:00'
    if kind == 'user':
        payload = json.dumps({'content': rand_text(2, (6, 24))}, ensure_ascii=False)
    elif kind == 'assistant':
        reasoning = rand_text(1, (10, 25)) if random.random() < 0.5 else None
        payload = json.dumps({
            'content': rand_text(3, (10, 30)),
            'usage': {'total_tokens': random.randint(500, 4000),
                      'prompt_tokens': random.randint(400, 3500),
                      'cached_tokens': random.randint(0, 2000),
                      'completion_tokens': random.randint(80, 900),
                      'reasoning_tokens': random.randint(0, 400)},
            'reasoning': reasoning,
        }, ensure_ascii=False)
    elif kind == 'tool_pair':
        name = random.choice(['ls', 'cat', 'jq', 'tree', 'grep', 'bash'])
        cid = f'c{seq}'
        args = json.dumps({'command': f'{name} {"-la " if name=="ls" else ""}{random.choice(WORDS)}_{seq}'},
                          ensure_ascii=False)
        rows.append(('tool_request', json.dumps(
            {'call_id': cid, 'name': name, 'args': args,
             'intent': lorem(4)}, ensure_ascii=False), ts))
        seq += 1
        # Artifact era: ~1 in 3 tool outputs (weighted to tree/bash/grep)
        # blows past the spill threshold. The DB gets the full content in
        # artifacts, the context gets the 4-line placeholder — THAT is
        # what the renderer must chew, not the full text.
        spill = random.random() < 0.33 or name in ('tree',)
        if spill:
            nlines = random.randint(600, 3000)
            full = '\n'.join(f'{lorem(3)}  {rand_chars(24)}' for _ in range(nlines))
            aid = aid_counter[0]
            aid_counter[0] += 1
            artifacts.append((aid, name, full))
            result = artifact_placeholder(aid, name, full)
        else:
            nlines = random.randint(3, 40)
            result = '\n'.join(f'{lorem(3)}  {rand_chars(12)}' for _ in range(nlines))
        payload = json.dumps({'call_id': cid, 'name': name, 'ok': random.random() > 0.12,
                              'result': result}, ensure_ascii=False)
    elif kind == 'system':
        payload = json.dumps({'text': lorem(5), 'align': 'Center'}, ensure_ascii=False)
    elif kind == 'error':
        payload = json.dumps({'text': f'error[{seq}]: {lorem(6)}'}, ensure_ascii=False)
    else:  # big_md — the markdown stress case
        payload = json.dumps({'content': md_sample(seq), 'usage': None, 'reasoning': None},
                             ensure_ascii=False)
    rows.append((kind.replace('_pair', '_result') if False else kind, payload, ts))
    seq += 1

# Big-MD entries are assistant kind; fix kinds to the real enum names.
fixed = []
for kind, payload, ts in rows:
    if kind == 'big_md':
        kind = 'assistant'
    elif kind == 'tool_pair':
        kind = 'tool_result'
    fixed.append((kind, payload, ts))

sid = conn.execute(
    "INSERT INTO sessions (id, name, started_at, cwd, leaf) VALUES (1, 'bench-8k', '2026-09-24 09:00:00', '/tmp/mypi-bench', NULL)"
).lastrowid
conn.execute("INSERT INTO cwd_history (session_id, seq, ts, cwd) VALUES (1, 0, '2026-09-24 09:00:00', '/tmp/mypi-bench')")
for aid, tool, content in artifacts:
    conn.execute(
        "INSERT INTO artifacts (id, session_id, name, total_lines, content) VALUES (?1, 1, ?2, ?3, ?4)",
        (aid, tool, content.count(chr(10)) + 1, content))

parent = None
ts = '2026-09-24 09:00:00'
for seq, (kind, payload, _) in enumerate(fixed, start=1):
    conn.execute(
        'INSERT INTO entries (session_id, seq, ts, kind, payload, parent_seq) '
        'VALUES (1, ?, ?, ?, ?, ?)', (seq, ts, kind, payload, parent))
    parent = seq
conn.execute('UPDATE sessions SET leaf = ? WHERE id = 1', (parent,))
conn.commit()

n = conn.execute('SELECT COUNT(*) FROM entries WHERE session_id=1').fetchone()[0]
total = conn.execute("SELECT SUM(LENGTH(payload)) FROM entries WHERE session_id=1").fetchone()[0]
per = conn.execute("SELECT kind, COUNT(*), SUM(LENGTH(payload)) FROM entries WHERE session_id=1 GROUP BY kind").fetchall()
na = conn.execute('SELECT COUNT(*) FROM artifacts WHERE session_id=1').fetchone()[0]
atot = conn.execute("SELECT SUM(LENGTH(content)) FROM artifacts WHERE session_id=1").fetchone()[0] or 0
print(f'session 1: {n} entries, {total/1e6:.1f} MB payload; {na} artifacts, {atot/1e6:.1f} MB spilled')
for k, c, b in per:
    print(f'  {k:14s} {c:5d}  {b/1e6:6.2f} MB')
print('db size:', os.path.getsize(db) / 1e6, 'MB')
