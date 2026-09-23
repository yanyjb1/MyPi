// 诊断用：最小 WS 收发探针（不入库，随 examples 保留）
use tungstenite::Message;
use std::time::Duration;

fn main() {
    let url = std::env::args().nth(1).expect("usage: ws_probe <ws-url> [msg]");
    let msg = std::env::args().nth(2).unwrap_or_else(|| r#"{"id":1,"method":"Runtime.evaluate","params":{"expression":"1+1","returnByValue":true}}"#.into());
    let (mut ws, _) = tungstenite::connect(&url).expect("connect");
    println!("[connected]");
    ws.send(Message::text(msg)).unwrap();
    match ws.get_mut() {
        tungstenite::stream::MaybeTlsStream::Plain(s) => s.set_read_timeout(Some(Duration::from_secs(6))).ok(),
        _ => None,
    };
    loop {
        match ws.read() {
            Ok(Message::Text(t)) => { println!("[text] {t}"); break; }
            Ok(m) => println!("[other] {m:?}"),
            Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                println!("[timeout] no message in 6s"); break;
            }
            Err(e) => { println!("[err] {e}"); break; }
        }
    }
}
