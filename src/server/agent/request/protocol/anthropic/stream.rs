use std::io::BufRead;

use anyhow::Result;

pub(crate) struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

pub(crate) fn read_event(reader: &mut dyn BufRead) -> Result<Option<SseEvent>> {
    let mut event = None;
    let mut data = String::new();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return if data.is_empty() && event.is_none() {
                Ok(None)
            } else {
                Ok(Some(SseEvent { event, data }))
            };
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if data.is_empty() && event.is_none() {
                continue;
            }
            return Ok(Some(SseEvent { event, data }));
        }
        if let Some(rest) = line.strip_prefix("event: ") {
            event = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("data: ") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest);
        }
    }
}
