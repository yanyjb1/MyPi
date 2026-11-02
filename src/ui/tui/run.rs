//! Take the terminal, run the frame loop, give it back.

use anyhow::Result;
use serde_json::json;

use crate::ui::session::Session;
use crate::ui::tui::app;
use crate::ui::tui::signal;
use crate::ui::tui::term;
use crate::ui::tui::zone::Frame;

/// The loop owns the terminal; the app owns the session.
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

        app.pump();
        // A failed draw means the terminal is gone; the exit path below still runs.
        if term.draw(app.render(), app.cursor()).is_err() {
            break;
        }
        if app.finished() {
            break;
        }

        match term.poll()? {
            Some(term::Input::Action(action)) => app.input(app::Input::Action(action)),
            Some(term::Input::Focus(gained)) => app.input(app::Input::Local(Frame::new(
                "local",
                "focus",
                json!({"gained": gained}),
            ))),
            None => {}
        }
    }

    app.bye();
    term.leave();
    if let Some(signal) = killed {
        std::process::exit(128 + signal);
    }
    Ok(())
}
