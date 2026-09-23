// 打印泵线程看到的所有错误种类
use tungstenite::Message;
use std::time::Duration;
fn main() {
    let url = std::env::args().nth(1).unwrap();
    let (mut ws, _) = tungstenite::connect(&url).unwrap();
    if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
        s.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    }
    ws.send(Message::text(r#"{"id":1,"method":"Page.enable","params":{}}"#)).unwrap();
    for i in 0..5 {
        match ws.read() {
            Ok(m) => println!("[{i}] ok: {:?}", m),
            Err(tungstenite::Error::Io(e)) => println!("[{i}] io: kind={:?} {e}", e.kind()),
            Err(e) => println!("[{i}] other: {e}"),
        }
    }
}
