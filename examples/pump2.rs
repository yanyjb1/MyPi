use tungstenite::Message;
use std::time::Duration;
fn main() {
    let url = std::env::args().nth(1).unwrap();
    let (mut ws, _) = tungstenite::connect(&url).unwrap();
    if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
        s.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    }
    println!("[connected]");
    for i in 0..4 {
        // 模拟泵：read
        match ws.read() {
            Ok(m) => println!("[{i}] read ok {:?}", m.to_text().map(|s| &s[..s.len().min(60)])),
            Err(tungstenite::Error::Io(e)) => println!("[{i}] io {:?} (continue)", e.kind()),
            Err(e) => { println!("[{i}] FATAL {e}"); return; }
        }
    }
    ws.send(Message::text(r#"{"id":1,"method":"DOM.getDocument","params":{}}"#)).unwrap();
    println!("[send ok]");
    loop {
        match ws.read() {
            Ok(Message::Text(t)) => { println!("[resp] {}", &t[..t.len().min(120)]); return; }
            Ok(_) => continue,
            Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => { println!("[FATAL] {e}"); return; }
        }
    }
}
