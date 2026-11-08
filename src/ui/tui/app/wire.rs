//! The one table from a zone's ask to the session's action: door names live here and nowhere else.

use crate::ui::session::Session;
use crate::ui::tui::zone::MemoAsk;
use crate::ui::tui::zone::Request;

/// Land one ask on the session. The app keeps its own asks: submit, attach, detach, structure,
/// quit, and the profile a session will open with.
pub(super) fn land(session: &mut Session, request: Request) {
    match request {
        Request::Init => session.init(),
        Request::Meta => session.meta(),
        Request::Usage => session.usage(),
        Request::Pools => session.pools(),
        Request::Config { key } => session.config(&key),
        Request::Blocks { ranges } => session.blocks(ranges),
        Request::Sessions { under } => session.sessions(under.as_deref()),
        Request::Memo(ask) => memo(session, ask),
        Request::Interrupt => session.interrupt(),
        Request::Mailbox {
            prefix,
            text,
            suffix,
        } => session.write_mailbox(prefix.as_deref(), &text, suffix.as_deref()),
        Request::ClearMailbox => session.clear_mailbox(),
        Request::Rename(name) => session.rename(name.as_deref()),
        Request::SwitchModel(id) => session.switch_model(&id),
        Request::SetDefaultModel(id) => session.set_default_model(&id),
        Request::Compact { retention, focus } => session.compact(retention, focus.as_deref()),
        Request::Delete(id) => session.delete(id),
        Request::Rewind { turn, block } => session.rewind(turn, block),
        Request::Submit(_)
        | Request::SetCwd(_)
        | Request::Copy(_)
        | Request::Attach(_)
        | Request::Detach
        | Request::Structure
        | Request::Quit
        | Request::PickProfile(_) => {}
    }
}

/// One memo ask, as the two doors spell it.
fn memo(session: &mut Session, ask: MemoAsk) {
    match ask {
        MemoAsk::List => session.memo_list(),
        MemoAsk::History { scope, name } => session.memo_history(scope.as_str(), &name),
        MemoAsk::Add {
            scope,
            name,
            content,
        } => session.memo_write("add", scope.as_str(), &name, &content),
        MemoAsk::Mod {
            scope,
            name,
            content,
        } => session.memo_write("mod", scope.as_str(), &name, &content),
        MemoAsk::Del { scope, name } => session.memo_delete(scope.as_str(), &name),
    }
}
