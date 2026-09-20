//! A queue owns one sequential engine process. Protocol failures are terminal:
//! the caller preserves the checkpoint and requires an explicit queue resume.
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_FRAME: usize = 1024 * 1024;
const START_TIMEOUT: Duration = Duration::from_secs(15);
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Deserialize)]
#[serde(tag = "event", rename_all = "lowercase", deny_unknown_fields)]
enum Frame {
    Ready {
        version: u32,
    },
    Log {
        id: String,
        stderr: bool,
        line: String,
    },
    Done {
        id: String,
        success: bool,
        error: Option<String>,
    },
}

#[derive(Serialize)]
struct Request<'a> {
    id: &'a str,
    args: &'a [String],
}

/// Bounded reads also reject truncated frames, invalid UTF-8 and malformed JSON.
fn read_frame(reader: &mut impl BufRead) -> Result<Frame, String> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| format!("Falha ao ler o motor residente: {e}"))?;
    if bytes.is_empty() {
        return Err("O motor residente encerrou antes de confirmar a busca.".into());
    }
    if bytes.len() > MAX_FRAME || bytes.last() != Some(&b'\n') {
        return Err("Resposta incompleta ou grande demais do motor residente.".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("Resposta inválida do motor residente: {e}"))
}

pub struct EngineWorker {
    child: Child,
    input: Option<ChildStdin>,
    frames: Option<mpsc::Receiver<Result<Frame, String>>>,
    output: Option<JoinHandle<()>>,
    errors: Option<JoinHandle<()>>,
    broken: bool,
}

impl EngineWorker {
    pub fn process_id(&self) -> u32 {
        self.child.id()
    }

    pub fn start(command: &mut Command) -> Result<Self, String> {
        Self::start_with_timeout(command, START_TIMEOUT)
    }

    fn start_with_timeout(command: &mut Command, timeout: Duration) -> Result<Self, String> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        super::hide_console(command);
        let mut child = command
            .spawn()
            .map_err(|e| format!("Não foi possível iniciar o motor residente: {e}"))?;
        let input = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let (sender, receiver) = mpsc::sync_channel(64);
        let output = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let frame = read_frame(&mut reader);
                let failed = frame.is_err();
                if sender.send(frame).is_err() || failed {
                    break;
                }
            }
        });
        // Raw stderr belongs to process startup/diagnostics, never to a later
        // request. Only id-tagged protocol logs may update a run's state.
        let diagnostics = Arc::new(Mutex::new(VecDeque::<u8>::new()));
        let capture = diagnostics.clone();
        let errors = thread::spawn(move || {
            let mut reader = stderr;
            let mut bytes = [0u8; 4096];
            while let Ok(count) = reader.read(&mut bytes) {
                if count == 0 {
                    break;
                }
                if let Ok(mut stored) = capture.lock() {
                    stored.extend(&bytes[..count]);
                    while stored.len() > 8192 {
                        stored.pop_front();
                    }
                }
            }
        });
        let mut worker = Self {
            child,
            input,
            frames: Some(receiver),
            output: Some(output),
            errors: Some(errors),
            broken: false,
        };
        let ready = worker
            .frames
            .as_ref()
            .unwrap()
            .recv_timeout(timeout)
            .map_err(|e| format!("O motor residente não confirmou a inicialização: {e}"))
            .and_then(|frame| frame)
            .and_then(|frame| match frame {
                Frame::Ready { version: 1 } => Ok(()),
                _ => Err("Versão ou mensagem inicial incompatível do motor residente.".into()),
            });
        if let Err(mut error) = ready {
            worker.shutdown();
            if let Ok(bytes) = diagnostics.lock() {
                let bytes: Vec<_> = bytes.iter().copied().collect();
                let detail = String::from_utf8_lossy(&bytes);
                if !detail.trim().is_empty() {
                    error.push_str(&format!(" Diagnóstico de inicialização: {}", detail.trim()));
                }
            }
            return Err(error);
        }
        Ok(worker)
    }

    /// The caller runs this on its background execution thread, never the UI.
    /// Successful completion includes negative, found, limited and paused runs;
    /// the existing log/checkpoint classifier retains those distinctions.
    pub fn execute(
        &mut self,
        id: &str,
        args: &[String],
        mut log: impl FnMut(String, bool),
    ) -> Result<(), String> {
        if self.broken {
            return Err("Motor residente indisponível após uma falha. Continue a fila para criar outra sessão.".into());
        }
        let result = self.execute_inner(id, args, &mut log);
        if result.is_err() {
            self.broken = true;
            self.shutdown();
        }
        result
    }

    fn execute_inner(
        &mut self,
        id: &str,
        args: &[String],
        log: &mut impl FnMut(String, bool),
    ) -> Result<(), String> {
        if id.is_empty()
            || id.len() > 128
            || args.len() > 4096
            || args.iter().any(|arg| arg.len() > 65536)
        {
            return Err("Requisição inválida para o motor residente.".into());
        }
        let mut request = serde_json::to_vec(&Request { id, args }).map_err(|e| e.to_string())?;
        request.push(b'\n');
        if request.len() > MAX_FRAME {
            return Err("A busca excede o limite de mensagem do motor residente.".into());
        }
        let frames = self.frames.as_ref().ok_or("Motor residente encerrado.")?;
        match frames.try_recv() {
            Err(mpsc::TryRecvError::Empty) => {}
            _ => {
                return Err(
                    "O motor residente enviou uma mensagem fora de uma busca ou encerrou.".into(),
                )
            }
        }
        let input = self
            .input
            .as_mut()
            .ok_or("Entrada do motor residente indisponível.")?;
        input
            .write_all(&request)
            .and_then(|_| input.flush())
            .map_err(|e| format!("Não foi possível enviar a busca ao motor residente: {e}"))?;
        loop {
            let frame = frames
                .recv()
                .map_err(|_| "O canal do motor residente encerrou sem confirmar a busca.")??;
            match frame {
                Frame::Log { id: received, stderr, line } if received == id => log(line, stderr),
                Frame::Done { id: received, success, error } if received == id => {
                    return if success && error.is_none() { Ok(()) } else { Err(error.unwrap_or_else(|| "O motor residente informou falha na busca.".into())) };
                },
                _ => return Err("Resposta do motor residente pertence a outra busca ou viola o protocolo. A fila foi interrompida.".into()),
            }
        }
    }

    fn shutdown(&mut self) {
        // Drop the bounded receiver before joining so a noisy child cannot
        // strand its reader on a full channel. EOF requests normal shutdown.
        self.frames.take();
        self.input.take();
        let deadline = Instant::now() + STOP_TIMEOUT;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                _ if Instant::now() >= deadline => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
                _ => thread::sleep(Duration::from_millis(10)),
            }
        }
        if let Some(thread) = self.output.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.errors.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for EngineWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_rejects_oversized_truncated_and_unknown_frames() {
        for bytes in [
            vec![b'x'; MAX_FRAME + 1],
            b"{\"event\":\"ready\",\"version\":1}".to_vec(),
            b"{\"event\":\"other\"}\n".to_vec(),
            b"\xff\n".to_vec(),
        ] {
            assert!(read_frame(&mut bytes.as_slice()).is_err());
        }
        assert!(matches!(
            read_frame(&mut b"{\"event\":\"ready\",\"version\":1}\r\n".as_slice()).unwrap(),
            Frame::Ready { version: 1 }
        ));
    }

    #[cfg(windows)]
    fn fake(script: &str) -> Command {
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        command
    }

    #[cfg(windows)]
    const SCRIPT: &str = r#"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::WriteLine('{"event":"ready","version":1}')
while ($null -ne ($line = [Console]::ReadLine())) {
  $r = $line | ConvertFrom-Json
  $mode = $r.args[0]
  if ($mode -eq 'eof') { exit 3 }
  $id = $r.id
  if ($mode -eq 'wrong-id') { $id = 'different' }
  $message = 'Exhausted search without a match; pid=' + $PID
  if ($mode -eq 'found') { $message = 'Found matching mnemonic: public test fixture' }
  if ($mode -eq 'pause') { $message = 'Searching fake workload' }
  [Console]::WriteLine((@{event='log';id=$id;stderr=$false;line=$message} | ConvertTo-Json -Compress))
  if ($mode -eq 'pause') {
    while (-not (Test-Path -LiteralPath $r.args[1])) { Start-Sleep -Milliseconds 10 }
    [Console]::WriteLine((@{event='log';id=$id;stderr=$false;line='Paused search'} | ConvertTo-Json -Compress))
  }
  $errorValue = $null
  if ($mode -eq 'fail') { $errorValue = 'fixture failure' }
  [Console]::WriteLine((@{event='done';id=$id;success=($mode -ne 'fail');error=$errorValue} | ConvertTo-Json -Compress))
}
"#;

    #[test]
    #[cfg(windows)]
    fn worker_reuses_one_process_and_preserves_negative_found_and_pause_logs() {
        let mut worker = EngineWorker::start(&mut fake(SCRIPT)).unwrap();
        let mut logs = Vec::new();
        for id in ["one", "two"] {
            worker
                .execute(id, &["negative".into()], |line, _| logs.push(line))
                .unwrap();
        }
        assert_eq!(logs[0], logs[1], "same process handles both requests");
        worker
            .execute("three", &["found".into()], |line, _| logs.push(line))
            .unwrap();
        assert!(logs[2].starts_with("Found matching mnemonic:"));
        let stop = std::env::temp_dir().join(format!("worker-stop-{}", super::super::unique_id()));
        worker
            .execute(
                "four",
                &["pause".into(), stop.to_str().unwrap().into()],
                |line, _| {
                    std::fs::write(&stop, b"stop").unwrap();
                    logs.push(line);
                },
            )
            .unwrap();
        assert_eq!(logs.last().unwrap(), "Paused search");
        worker.shutdown();
        assert!(worker.child.try_wait().unwrap().is_some());
        std::fs::remove_file(stop).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn worker_discards_failures_eof_and_mismatched_ids_without_retry() {
        for mode in ["fail", "eof", "wrong-id"] {
            let mut worker = EngineWorker::start(&mut fake(SCRIPT)).unwrap();
            assert!(worker.execute("first", &[mode.into()], |_, _| {}).is_err());
            assert!(worker.child.try_wait().unwrap().is_some());
            assert!(worker
                .execute("second", &["negative".into()], |_, _| panic!(
                    "failed session reused"
                ))
                .is_err());
        }
    }

    #[test]
    #[cfg(windows)]
    fn startup_timeout_and_diagnostics_are_not_attached_to_a_job() {
        let result = EngineWorker::start_with_timeout(
            &mut fake("[Console]::Error.WriteLine('startup fixture'); exit 2"),
            Duration::from_secs(5),
        );
        assert!(result.err().unwrap().contains("startup fixture"));
        let began = Instant::now();
        let result = EngineWorker::start_with_timeout(
            &mut fake("Start-Sleep -Seconds 20"),
            Duration::from_millis(100),
        );
        assert!(result.is_err());
        assert!(began.elapsed() < Duration::from_secs(5));
    }

    #[test]
    #[cfg(windows)]
    fn drop_closes_stdin_and_waits_for_process_exit() {
        let marker =
            std::env::temp_dir().join(format!("worker-drop-{}", super::super::unique_id()));
        let script = format!(
            "{SCRIPT}\n[System.IO.File]::WriteAllText('{}', 'closed')",
            marker.to_str().unwrap().replace('\'', "''")
        );
        let worker = EngineWorker::start(&mut fake(&script)).unwrap();
        drop(worker);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "closed");
        std::fs::remove_file(marker).unwrap();
    }
}
