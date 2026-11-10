//! Take the terminal, run the frame loop, give it back.

use std::time::Instant;

use anyhow::Result;
use serde_json::json;

use crate::ui::session::Session;
use crate::ui::tui::app;
use crate::ui::tui::signal;
use crate::ui::tui::term;
use crate::ui::tui::zone::Frame;

/// The loop owns the terminal; the app owns the session.
///
/// One round: what the terminal said, then what the connection said, then the side effects,
/// then a draw only when the showing zone's rows changed or an animation fell due.
pub fn run(session: Session, startup: app::Startup) -> Result<()> {
    let mut term = term::Term::enter()?;
    let size = term.size()?;
    let mut app = app::App::boot(session, size, startup, crate::ui::tui::zone::zones());
    let mut stop = signal::Stop::new()?;
    let mut last = size;
    let mut killed = None;

    loop {
        // A stop signal is the only thing that ends the loop besides the app itself.
        if let Some(signal) = stop.poll() {
            killed = Some(signal);
            break;
        }
        let now = term.size()?;
        if now != last {
            last = now;
            app.input(app::Input::Local(Frame::new(
                "local",
                "resized",
                json!({"cols": now.cols, "rows": now.rows}),
            )));
        }

        // The user is always served before the connection: a frame batch is appended, never laid.
        for input in term.drain()? {
            match input {
                term::Input::Action(action) => app.input(app::Input::Action(action)),
                term::Input::Focus(gained) => app.input(app::Input::Local(Frame::new(
                    "local",
                    "focus",
                    json!({"gained": gained}),
                ))),
            }
        }
        app.pump();
        app.tick();
        // A failed draw means the terminal is gone; the exit path below still runs.
        if let Some(lines) = app.frame(Instant::now())
            && term.draw(lines, app.cursor()).is_err()
        {
            break;
        }
        if app.finished() {
            break;
        }

        term.wait()?;

        // What the frame put on the clipboard leaves between draws, never inside one.
        for text in app.take_copy() {
            let _ = term.copy(&text);
        }
    }

    app.bye();
    term.leave();
    if let Some(signal) = killed {
        std::process::exit(128 + signal);
    }
    Ok(())
}
