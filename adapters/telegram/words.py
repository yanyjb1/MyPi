# Everything the bot says, in two languages, picked once from `config.ui.lang` at startup.

# The language this bot speaks now; `zh` speaks Chinese and anything else English.
LANG = "en"

# One entry per thing the bot can say: English first, Chinese second.
WORDS = {
    "text_only": ("Only plain text for now.", "暂时只认文字。"),
    "no_current": ("Nothing is selected here. /list to pick a session, or /new to start one.", "现在没对着任何会话。/list 挑一个，或者 /new 起一个。"),
    "hosted_full": ("Already holding %(most)d sessions; release one with /quit.", "已经托管了 %(most)d 个，放掉一个再来：/quit。"),
    "queued": ("Session %(session)d is running; this line is waiting in its mailbox.", "%(session)d 号在跑，这句话排进去了。"),
    "reply_prefix": ("You last replied at %(at)s\n", "你上次回复于：%(at)s\n"),
    "message_suffix": ("\nThis message was sent at %(at)s (from telegram)", "\n本条消息发送于：%(at)s（来自 telegram）"),
    "never": ("never", "没有过"),
    "spawn_failed": ("Cannot start one: %(words)s", "建不出来：%(words)s"),
    "spawn_no_id": ("Cannot start one: the door gave no number.", "建不出来：桥没有给出编号。"),
    "spawned": ("Session %(session)d is up.", "%(session)d 号起来了。"),
    "spawn_cwd": ("\nIts directory is %(cwd)s", "\n目录落在 %(cwd)s"),
    "spawn_unhosted": ("Session %(session)d is up, but the bot holds no room; /quit one first.", "%(session)d 号建好了，但托管满了，先 /quit 放掉一个。"),
    "no_token": ("No bot token: put a token.txt beside this script, or set TG_BOT_TOKEN.", "没有 bot token：放一个 token.txt 在旁边，或者设 TG_BOT_TOKEN。"),
    "round_ended": ("Session #%(session)d finished; it said: %(words)s", "会话#%(session)d结束了，正文消息：%(words)s"),
    "no_words": ("(no words)", "（没有正文）"),
    "no_pairing": ("No account is paired yet.", "还没有绑定的账号。"),
    "paired": ("Paired. /help for the commands.", "绑上了。/help 看命令。"),
    "not_yours": ("This bot is not shared.", "这个 bot 不给别人用。"),
    "handle_failed": ("That did not go through: %(words)s", "这句话没办成：%(words)s"),
    "stream_lost": ("session %(session)d: the stream ended, reopening", "会话 %(session)d 的流断了，重新接上"),
    "unreachable": ("The door did not answer: %(words)s", "桥没有应答：%(words)s"),
    "unreadable": ("The door answered something unreadable.", "桥回了读不懂的东西"),
    "running": ("running: ", "跑着："),
    "failed_note": (" (%(count)d failed)", "（%(count)d 失败）"),
    "unknown_command": ("No such command /%(word)s. /help lists the ones there are.", "没有 /%(word)s 这个命令。/help 看有什么。"),
    "help": (
        "Whatever you send goes to the session being talked to. Commands: /list sessions, /attach <id> switch to that session, /new [dir] start a draft, /draft show the draft, /host hand the current session to the background, /quit let the current session go, /switch [model] pick a model (/model is the same), /effort [rung] pick a rung, /profile [name] drafts only, /cwd [path] pick a directory (/cd is the same), /name <name> name the session, /usage the bill, /compact [focus] compact the context, /esc interrupt the round running (/stop is the same).",
        "把话直接发过来，就是发给当前会话。命令：/list 会话列表，/attach 编号 换到那个会话，/new [目录] 立一份草稿，/draft 看草稿，/host 把当前会话交给后台，/quit 让当前会话退出，/switch [模型] 挑模型（/model 同），/effort [档位] 挑档位，/profile [档] 只对草稿有用，/cwd [路径] 挑目录（/cd 同），/name 名字，/usage 这个会话的账，/compact [重点] 压缩上下文，/esc 打断正在跑的这一轮（/stop 同）。",
    ),
    "no_sessions": ("No sessions yet. /new starts one.", "还没有会话。/new 起一个。"),
    "empty_session": ("(empty)", "（空会话）"),
    "more_sessions": ("…%(count)d more", "…还有 %(count)d 个"),
    "ls_legend": ("▶ talking　● hosted　· neither", "▶ 正在说　● 托管中　· 都没"),
    "draft_new": (
        "Draft started: directory %(cwd)s, model %(model)s, effort %(effort)s, profile %(profile)s. Say the first thing when it looks right, and the session comes into being.",
        "草稿起来了：目录 %(cwd)s、模型 %(model)s、档位 %(effort)s、档 %(profile)s。改好之后直接说第一句话，会话就建出来了。",
    ),
    "default_value": ("(default)", "（默认）"),
    "no_draft": ("There is no draft. /new starts one.", "现在没有草稿。/new 立一份。"),
    "draft_show": ("Draft:\ndirectory %(cwd)s\nmodel %(model)s\neffort %(effort)s\nprofile %(profile)s", "草稿：\n目录 %(cwd)s\n模型 %(model)s\n档位 %(effort)s\n档 %(profile)s"),
    "attach_need_id": ("/attach takes a number; /list shows them.", "/attach 后面跟一个编号，编号在 /list 里。"),
    "attach_full": ("The bot holds no room; /quit one before switching.", "托管满了，先 /quit 放掉一个再换。"),
    "attached": ("Talking to %(session)d.\n%(name)s · %(address)s · %(cwd)s", "对着 %(session)d 号说话了。\n%(name)s · %(address)s · %(cwd)s"),
    "unnamed": ("(unnamed)", "（无名）"),
    "host_no_current": ("Nothing is selected here. /list to pick a session, or /new to start one.", "现在没对着会话。/list 挑一个，或者 /new 起一个。"),
    "host_full": ("The bot holds no room; /quit one first.", "托管满了，先 /quit 放掉一个再来。"),
    "hosted": ("Session %(session)d is on the background; I will say when its round ends. /attach another session, or /new to start one.", "%(session)d 号交给后台了，跑完我说一声。/attach 换个会话，或者 /new 起一个。"),
    "quit_no_session": ("/quit needs a session: /attach one first.", "/quit 要对着一个会话：先 /attach 一个。"),
    "quit_running": ("Session %(session)d is still running. /esc it before letting go.", "%(session)d 号还在跑。/esc 断掉再退。"),
    "quit_done": ("Session %(session)d is let go, and the door with it.", "%(session)d 号退出了，桥那边也撒手了。"),
    "no_target": ("Nothing is selected and there is no draft. /list or /new.", "现在没对着会话也没草稿。/list 或 /new。"),
    "no_models": ("No model is available.", "没有可用的模型。"),
    "pick_models": ("Models; send /switch with the number:", "模型，发 /switch 编号："),
    "no_pick_switch": ("No such number. /switch lists them.", "没这个编号。/switch 看清单。"),
    "bad_address": ("It has to read provider:model. /switch lists them.", "要写成 provider:model。/switch 看清单。"),
    "draft_model": ("The draft is on %(model)s now.", "草稿的模型换成 %(model)s 了。"),
    "session_model": ("Session %(session)d is on %(model)s now.", "%(session)d 号换成 %(model)s 了。"),
    "no_rungs": ("This model takes no thinking effort.", "这个模型不吃思考强度。"),
    "pick_rungs": ("Rungs; send /effort with the name:", "档位，发 /effort 名字："),
    "rungs_only": ("This model takes only these: %(rungs)s", "这个模型只认这些：%(rungs)s"),
    "draft_effort": ("The draft's effort is %(effort)s now.", "草稿的档位换成 %(effort)s 了。"),
    "session_effort": ("Session %(session)d runs on %(effort)s now.", "%(session)d 号的档位换成 %(effort)s 了。"),
    "pick_dirs": ("Directories under %(dir)s; send /cwd with the number:", "%(dir)s 下的目录，发 /cwd 编号："),
    "no_dirs": ("(no subdirectory)", "（没有子目录）"),
    "up_one": ("One level up: /cwd ..", "上一级：/cwd .."),
    "or_path": ("Or give a path: /cwd /some/where", "或者直接给路径：/cwd /某个/地方"),
    "no_pick_cwd": ("No such number. /cwd lists them.", "没这个编号。/cwd 看清单。"),
    "draft_cwd": ("The draft's directory is %(cwd)s now.", "草稿的目录换成 %(cwd)s 了。"),
    "session_cwd": ("Session %(session)d stands in %(cwd)s now.", "%(session)d 号的目录换成 %(cwd)s 了。"),
    "name_need": ("/name takes a name.", "/name 后面跟一个名字。"),
    "draft_no_name": ("A draft has no name yet; it gets one when it is born.", "草稿还没有名字，等它建出来。"),
    "renamed": ("Session %(session)d is called「%(name)s」now.", "%(session)d 号改名叫「%(name)s」了。"),
    "profile_fixed": ("The profile is fixed when a session is born. /new a draft to pick one.", "档只在建会话那一刻定下来，之后改不了。/new 立个草稿再挑。"),
    "no_profiles": ("No profile under this directory.", "这个目录下没有可用的档。"),
    "pick_profiles": ("Profiles; send /profile with the number:", "档，发 /profile 编号："),
    "no_pick_profile": ("No such number. /profile lists them.", "没这个编号。/profile 看清单。"),
    "draft_profile": ("The draft's profile is %(profile)s now.", "草稿的档换成 %(profile)s 了。"),
    "no_bill": ("No session, no bill.", "还没有会话，没有账。"),
    "bill_head": ("Session %(session)d · %(address)s", "%(session)d 号 · %(address)s"),
    "bill_tokens": ("tokens  %(total)s (in %(prompt)s / out %(completion)s)", "token  %(total)s（进 %(prompt)s / 出 %(completion)s）"),
    "bill_cache": ("cache  %(percent)d%% hit", "缓存  命中 %(percent)d%%"),
    "bill_no_tokens": ("tokens  none yet", "token  还没有"),
    "bill_cost": ("cost  ¥%(cny).3f / $%(usd).3f", "花费  ¥%(cny).3f / $%(usd).3f"),
    "bill_context_window": ("context  %(mark)s%(context)s / %(window)s (%(percent)d%%)", "上下文  %(mark)s%(context)s / %(window)s（%(percent)d%%）"),
    "bill_context": ("context  %(mark)s%(context)s", "上下文  %(mark)s%(context)s"),
    "bill_rounds": ("rounds  %(rounds)d", "轮次  %(rounds)d"),
    "no_compact": ("No session, nothing to compact.", "还没有会话，没什么可压的。"),
    "compacting": ("Session %(session)d started compacting.", "%(session)d 号开始压缩了。"),
    "no_stop": ("Nothing is running here.", "现在没有正在跑的会话。"),
    "stopped": ("Session %(session)d is interrupted.", "%(session)d 号断了。"),
}

# The server's own refusal keys this bot can itself cause; any other key is shown as it arrived.
REFUSALS = {
    "busy": ("That session is already in a turn.", "那个会话正在跑一轮。"),
    "child_message": ("A child session takes no messages.", "子会话不收消息。"),
    "no_session": ("No such session.", "没有这个会话。"),
    "writer_required": ("Another front end is writing to that session.", "另一个前端正在写这个会话。"),
    "effort_unsupported": ("This model takes no thinking effort.", "这个模型不吃思考强度。"),
    "effort_not_declared": ("This model does not declare that rung.", "这个模型没有这一档。"),
    "config_key_unknown": ("No such configuration key.", "没有这个配置项。"),
    "no_default_model": ("No default model is configured.", "没有配默认模型。"),
}


# Speak the language the configuration named: `zh` reads the Chinese column, anything else English.
def choose(lang):
    global LANG
    LANG = "zh" if str(lang or "").strip().lower() == "zh" else "en"


# One entry, filled: `%(name)s` in the table takes the same keyword here.
def t(key, **args):
    pair = WORDS[key]
    line = pair[1] if LANG == "zh" else pair[0]
    return line % args if args else line


# One known refusal in the current language, or None when this bot does not know that key.
def refusal(key):
    pair = REFUSALS.get(key)
    if pair is None:
        return None
    return pair[1] if LANG == "zh" else pair[0]