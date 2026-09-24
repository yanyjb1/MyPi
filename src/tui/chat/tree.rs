//! Conversation-tree navigation — pi's branch semantics: `/resume`,
//! the tree picker, and moving the leaf pointer to any stored row.
//!
//! Moving the leaf **deletes nothing**; the next append forks from the
//! new leaf. Re-projection rebuilds the transcript from the stored
//! entries, so the render layer and the protocol agree on "current
//! branch" without duplicating the projection logic.

use crate::entry;
use crate::tui::app::App;

impl App {
    // Navigate the conversation tree to an arbitrary stored row (pi's
    // branch() semantic: move the leaf pointer, delete nothing). The next
    // append forks from there. Rebuilds transcript + protocol from the new
    // projection; the prefix cache is keyed on the rebuilt history, so a
    // cache hit survives navigation to a shared prefix.
    pub(crate) fn tree_navigate_to(&mut self, seq: i64) {
        let Some(sid) = self.session.session_id() else {
            return;
        };
        // Store work (leaf move + re-projection + name lookup) inside a
        // scoped borrow; the SessionState mutation happens after it ends.
        let (entries, effective, draft) = {
            let Some(st) = self.session.store_mut() else {
                return;
            };
            if let Err(e) = st.set_leaf(sid, Some(seq)) {
                self.session.echo(entry::Entry::Error {
                    text: format!("回溯失败：{e:#}"),
                });
                return;
            }
            let entries = match st.load_entries(sid) {
                Ok(e) => e,
                Err(e) => {
                    self.session.echo(entry::Entry::Error {
                        text: format!("重投影失败：{e:#}"),
                    });
                    return;
                }
            };
            let effective = st.effective_name(sid).ok().flatten();
            let draft = user_text_at(st, sid, seq);
            (entries, effective, draft)
        };

        // 1+3) Rendering layer + name: navigate_to replaces transcript,
        // clears pending and merges the effective name (never clobbering).
        self.session.navigate_to(entries.clone(), effective);

        // 2) Protocol rebuild: the shared routine — dangling tool tails
        // (leaf on a request without results) are repaired inside, so the
        // next run() always starts from a protocol-legal boundary.
        self.session.rebuild_chat(&entries);

        // 4) Echo + editor draft semantics: navigating to a user entry puts
        // that message back into the editor (pi behavior) — you usually
        // rewound in order to rewrite it.
        self.session.echo(entry::Entry::System {
            text: format!("已回到节点 #{seq}（后续消息仍保留在树中）"),
            align: entry::Align::Center,
        });
        if let Some(d) = draft {
            self.load_into_editor(&d);
        }
    }

    // Confirm restoring the highlighted session.
    //
    // Restore four things: the transcript (rendering), the chat context
    // (cross-turn memory + cache prefix), the working directory (the
    // session's last persisted migration), and the session name.
    // The first turn after resume appends a cwd note (pending_cwd_note)
    // — appended only, history untouched, cache prefix intact.
    pub(crate) fn resume_confirm(&mut self) {
        let Some((items, sel)) = self.resume_pick.take() else {
            return;
        };
        let Some((id, name)) = items.get(sel).cloned() else {
            return;
        };
        // Store reads (projection + metadata) in a scoped borrow; state
        // mutations happen after it ends.
        let (entries, meta, effective, last_cwd_seq) = {
            let Some(st) = self.session.store() else {
                return;
            };
            let entries = match st.load_entries(id) {
                Ok(e) => e,
                Err(e) => {
                    self.session.echo(entry::Entry::Error {
                        text: format!("读取会话失败：{e:#}"),
                    });
                    return;
                }
            };
            let meta = st.session(id).ok();
            let effective = st
                .effective_name(id)
                .ok()
                .flatten()
                .or_else(|| meta.as_ref().and_then(|m| m.name.clone()));
            let last_cwd_seq = st
                .cwd_history(id)
                .ok()
                .and_then(|h| h.last().map(|(seq, _)| *seq))
                .unwrap_or(0);
            (entries, meta, effective, last_cwd_seq)
        };

        // 1) Rendering layer
        self.session.commit_round(entries.clone());

        // 2) Chat context: rebuild from the entries projection (the
        // inverse of collect_turn). Compaction markers, if any, split the
        // projection inside entries_to_context.
        self.session.rebuild_chat(&entries);

        // 3) Working directory: the session's last persisted migration (falls back to the initial cwd on record)
        if let Some(m) = &meta
            && let Some(cwd_str) = &m.cwd
        {
            let p = std::path::PathBuf::from(cwd_str);
            if p.is_dir() {
                self.session.set_cwd(p);
            }
        }

        // 4) Session identity and state
        let n_entries = entries.len();
        self.session.adopt_session(id, entries, effective);
        self.session.set_cwd_seq(last_cwd_seq);
        // Inject the cwd note into the first turn after resume (append only; history untouched)
        self.pending_cwd_note = Some(format!(
            "[工作目录已恢复为 {}，相对路径以此为基准]",
            self.session.cwd().display()
        ));

        self.session.echo(entry::Entry::Error {
            text: format!("已恢复会话：{name}（{n_entries} 条记录）"),
        });
    }

    // Load a text into the editor (history recall and programmatic
    // fill both use this).
    //
    // Routed through `insert_paste` instead of a direct insert: history
    // stores the **expanded** text, and pouring it in raw would blow up
    // the input box; this re-folds it into markers by the same rules.
}

// The user message text stored at `seq`, if that row is a user entry.
// Used for the navigate-to-user-node draft refill (pi's editorText).
fn user_text_at(st: &crate::store::Store, sid: i64, seq: i64) -> Option<String> {
    let tree = st.load_tree(sid).ok()?;
    let row = tree.iter().find(|n| n.seq == seq)?;
    if row.kind != "user" {
        return None;
    }
    match entry::Entry::from_payload(&row.kind, &row.payload) {
        Some(entry::Entry::User { content }) => Some(content),
        _ => None,
    }
}
