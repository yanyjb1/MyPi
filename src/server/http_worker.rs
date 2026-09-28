use std::sync::atomic::{AtomicU32, Ordering};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    Stream {
        id: u64,
        base_url: String,
        api_key: String,
        body: serde_json::Value,
        /// 父进程算好的额外请求头。worker 不认识它们的意思，只是发出去——
        /// 这是它"不知道 body 是什么"那条边界的同一条边。
        #[serde(default)]
        headers: Vec<(String, String)>,
    },
    Shutdown,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    /// 启动握手：worker 就绪，只发一次。
    ///
    /// `spawn` 读它来确认子进程**真的在干活**——只看 `Command::spawn`
    /// 成功是不够的：参数不认识、二进制不符、起来就崩，都会得到一个
    /// "进程在但不干活"的子进程，之后每个请求都撞在同一条死管道上。
    Ready,
    Chunk { id: u64, line: String },
    Done { id: u64 },
    Error { id: u64, message: String },
}

#[derive(Debug)]
pub struct HttpWorker {
    child: Child,
    pid: Arc<AtomicU32>,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next_id: u64,
}
pub type SharedWorker = Arc<Mutex<HttpWorker>>;

impl HttpWorker {
    pub fn pid(slot: &SharedWorker) -> Option<u32> {
        slot.lock().ok().map(|w| w.pid.load(Ordering::Relaxed)).filter(|p| *p != 0)
    }
    pub fn kill_pid(pid: u32) {
        let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).status();
    }
    pub fn replace(slot: &SharedWorker) -> Result<()> {
        let fresh = Self::spawn()?;
        let mut old = slot.lock().map_err(|_| anyhow::anyhow!("HTTP worker lock poisoned"))?;
        let mut new = fresh.lock().map_err(|_| anyhow::anyhow!("HTTP worker lock poisoned"))?;
        let _ = old.child.kill();
        let _ = old.child.wait();
        std::mem::swap(&mut *old, &mut *new);
        Ok(())
    }

    pub fn spawn() -> Result<SharedWorker> { 
        let exe = std::env::current_exe().context("cannot locate the mypi executable")?;
        let mut child = Command::new(exe)
            .arg("--http-worker")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().context("failed to start the HTTP worker")?;
        let pid = Arc::new(AtomicU32::new(child.id()));
        let input = child.stdin.take().context("HTTP worker stdin unavailable")?;
        let mut output = BufReader::new(child.stdout.take().context("HTTP worker stdout unavailable")?);
        // 握手：确认子进程真的在跑 worker。只看"进程起来了"会误判——
        // 不认识参数的二进制（测试里 `current_exe()` 就是测试二进制）、
        // 被换掉的二进制、启动即崩的，都会得到一个不干活的子进程，之后
        // 每个请求都撞在同一根死管道上。握手失败就当场收掉，返回 Err，
        // 调用方（`Client::new`）据此退回直连。
        let mut hello = String::new();
        let ok = std::io::BufRead::read_line(&mut output, &mut hello).is_ok()
            && matches!(
                serde_json::from_str::<Response>(hello.trim()),
                Ok(Response::Ready)
            );
        if !ok {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("HTTP worker never became ready");
        }
        Ok(Arc::new(Mutex::new(Self { child, pid, input, output, next_id: 1 })))
    }
    fn send(&mut self, req: &Request) -> Result<()> {
        serde_json::to_writer(&mut self.input, req)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        Ok(())
    }
    pub fn stream<F>(
        &mut self,
        base_url: &str,
        api_key: &str,
        body: serde_json::Value,
        headers: &[(String, String)],
        mut on_line: F,
    ) -> Result<()>
    where F: FnMut(&str) -> bool {
        let id = self.next_id; self.next_id += 1;
        self.send(&Request::Stream {
            id,
            base_url: base_url.into(),
            api_key: api_key.into(),
            body,
            headers: headers.to_vec(),
        })?;
        let mut raw = String::new();
        loop {
            raw.clear();
            let n = self.output.read_line(&mut raw)?;
            if n == 0 { anyhow::bail!("HTTP worker exited") }
            let msg: Response = serde_json::from_str(raw.trim_end())?;
            match msg {
                Response::Chunk { id: got, line } if got == id => {
                    if !on_line(&line) { anyhow::bail!("HTTP stream interrupted") }
                }
                Response::Done { id: got } if got == id => return Ok(()),
                Response::Error { id: got, message } if got == id => anyhow::bail!("{message}"),
                _ => {}
            }
        }
    }
    pub fn kill_and_replace(slot: &SharedWorker) -> Result<()> {
        let mut old = slot.lock().map_err(|_| anyhow::anyhow!("HTTP worker lock poisoned"))?;
        let _ = old.child.kill();
        let _ = old.child.wait();
        let fresh = Self::spawn()?;
        let mut replacement = fresh.lock().map_err(|_| anyhow::anyhow!("HTTP worker lock poisoned"))?;
        std::mem::swap(&mut *old, &mut *replacement);
        Ok(())
    }
    pub fn reap(&mut self) { let _ = self.child.wait(); }
}

pub fn run() -> Result<()> {
    let stdin = std::io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    // 握手先行：父进程（`spawn`）据这一行判断这个子进程真的在跑 worker。
    // 不先发的话，父进程只能靠"进程起来了"猜——猜错的代价是每个请求都
    // 撞死在管道上。
    serde_json::to_writer(&mut out, &Response::Ready)?;
    out.write_all(b"\n")?;
    out.flush()?;
    let mut raw = String::new();
    while { raw.clear(); input.read_line(&mut raw)? != 0 } {
        let req: Request = serde_json::from_str(raw.trim_end())?;
        match req {
            Request::Stream { id, base_url, api_key, body, headers } => {
                let agent: ureq::Agent = ureq::Agent::config_builder()
                    .timeout_connect(Some(std::time::Duration::from_secs(30)))
                    .timeout_recv_response(Some(std::time::Duration::from_secs(120)))
                    .build().into();
                let mut req = agent
                    .post(format!("{}/chat/completions", base_url.trim_end_matches('/')))
                    .header("Authorization", &format!("Bearer {api_key}"));
                for (name, value) in &headers {
                    req = req.header(name, value);
                }
                let resp = req.send_json(&body);
                match resp {
                    Ok(response) => {
                        let mut reader = BufReader::new(response.into_body().into_reader());
                        let mut line = String::new();
                        loop {
                            line.clear();
                            match reader.read_line(&mut line) {
                                Ok(0) => break,
                                Ok(_) => { serde_json::to_writer(&mut out, &Response::Chunk { id, line: line.trim_end_matches(['\r','\n']).into() })?; out.write_all(b"\n")?; out.flush()?; }
                                Err(e) => { serde_json::to_writer(&mut out, &Response::Error { id, message: e.to_string() })?; out.write_all(b"\n")?; break; }
                            }
                        }
                        serde_json::to_writer(&mut out, &Response::Done { id })?; out.write_all(b"\n")?; out.flush()?;
                    }
                    Err(e) => { serde_json::to_writer(&mut out, &Response::Error { id, message: e.to_string() })?; out.write_all(b"\n")?; out.flush()?; }
                }
            }
            Request::Shutdown => return Ok(()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "起了但不干活"的子进程必须被拒绝。
    ///
    /// 测试进程的 `current_exe()` 就是测试二进制：它不认识 `--http-worker`，
    /// 起来就退——正是这个 bug 的真身。修之前 `spawn` 只看"进程起来了"，
    /// 于是给出一个"可用"的 worker，之后每个请求都撞死在同一根管道上
    /// （hub 那 6 个集成测试就是这么整片超时的）。修之后这里必须 Err，
    /// 调用方据此退回直连。
    #[test]
    fn a_child_that_does_not_run_the_worker_is_refused() {
        assert!(
            HttpWorker::spawn().is_err(),
            "不跑 worker 的子进程必须被拒绝，否则请求会撞死在死管道上"
        );
    }
}
