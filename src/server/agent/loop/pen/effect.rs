//! Tool effects, turned into rows.

use anyhow::Result;

use crate::server::agent::r#loop::session::Birth;
use crate::server::agent::r#loop::turn::batch::Effect;
use crate::server::storage::{Ref, Row, Submit, Writer};

/// A child session the loop still has to book.
pub struct Born {
    pub birth: Birth,
    pub start: String,
}

/// Write every effect. Returns the child sessions created.
pub fn write(w: &Writer, effects: &[Effect]) -> Result<Vec<Born>> {
    let mut rows = Vec::new();
    let mut children = Vec::new();
    for effect in effects {
        match effect {
            Effect::Memo {
                anchor,
                name,
                content,
                ts,
            } => rows.push(Row::Memo {
                anchor: Ref::Known(*anchor),
                name: name.clone(),
                content: content.clone(),
                ts: ts.clone(),
            }),
            Effect::Alias {
                anchor,
                name,
                command,
                ts,
            } => rows.push(Row::Alias {
                anchor: Ref::Known(*anchor),
                name: name.clone(),
                command: command.clone(),
                ts: ts.clone(),
            }),
            Effect::Subagent {
                anchor,
                provider,
                model,
                system,
                tools,
                cwd,
                start,
            } => {
                let id = super::birth::child(
                    w,
                    &crate::server::platform::time::now_stamp(),
                    &cwd.to_string_lossy(),
                    provider,
                    model,
                    Ref::Known(*anchor),
                    None,
                )?;
                children.push(Born {
                    birth: Birth {
                        session: id,
                        provider: provider.clone(),
                        model: model.clone(),
                        system: system.clone(),
                        tools: tools.clone(),
                        meta: None,
                        cwd: Some(cwd.clone()),
                    },
                    start: start.clone(),
                });
            }
        }
    }
    if !rows.is_empty() {
        w.push(Submit { rows })?;
    }
    Ok(children)
}
