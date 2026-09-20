//! Sequential resident-engine protocol. No request is automatically replayed.

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::io::{self, BufRead};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};

const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_ARGS: usize = 4096;
const MAX_ARG_BYTES: usize = 64 * 1024;
const MAX_ID_BYTES: usize = 128;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    id: String,
    args: Vec<String>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_BYTES && !id.chars().any(char::is_control)
}

fn parse_request(bytes: &[u8]) -> (Option<String>, Result<crate::Args>) {
    let value: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(_) => return (None, Err(anyhow::anyhow!("Invalid request JSON"))),
    };
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|id| valid_id(id))
        .map(str::to_owned);
    let parsed = (|| {
        let request: Request =
            serde_json::from_value(value).context("Expected only id and args")?;
        anyhow::ensure!(valid_id(&request.id), "Invalid request id");
        anyhow::ensure!(request.args.len() <= MAX_ARGS, "Too many arguments");
        anyhow::ensure!(
            request
                .args
                .iter()
                .all(|s| s.len() <= MAX_ARG_BYTES && !s.contains('\0')),
            "Argument is too large or contains NUL"
        );
        // These commands inspect/export state or terminate the CLI; resident
        // jobs are searches only. Clap parsing never prints or exits here.
        let forbidden = [
            "--worker",
            "--selftest",
            "--list-history",
            "--preflight-history",
            "--coverage-report",
            "--export-checkpoint-record",
            "--help",
            "-h",
            "--version",
            "-V",
        ];
        anyhow::ensure!(
            !request
                .args
                .iter()
                .any(|a| forbidden.contains(&a.split('=').next().unwrap_or(a))),
            "Worker requests accept search arguments only"
        );
        let args = crate::Args::try_parse_from(
            std::iter::once("words-breaker".to_owned()).chain(request.args),
        )?;
        anyhow::ensure!(args.target_address.is_some(), "Missing target address");
        Ok(args)
    })();
    (id, parsed)
}

enum Frame {
    Eof,
    Line(Vec<u8>),
    TooLarge,
}

/// Consume an oversized frame without ever allocating more than the bound.
fn read_frame(reader: &mut impl BufRead) -> io::Result<Frame> {
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(if oversized {
                Frame::TooLarge
            } else if line.is_empty() {
                Frame::Eof
            } else {
                Frame::Line(line)
            });
        }
        let newline = available.iter().position(|&b| b == b'\n');
        let take = newline.map_or(available.len(), |n| n + 1);
        if !oversized {
            if line.len() + take > MAX_LINE_BYTES {
                oversized = true;
                line.clear();
            } else {
                line.extend_from_slice(&available[..take]);
            }
        }
        reader.consume(take);
        if newline.is_some() {
            return Ok(if oversized {
                Frame::TooLarge
            } else {
                Frame::Line(line)
            });
        }
    }
}

fn done(id: Option<&str>, result: &Result<()>) -> serde_json::Value {
    serde_json::json!({"event":"done", "id":id, "success": result.is_ok(), "error":result.as_ref().err().map(|e| format!("{e:#}"))})
}

#[derive(Default)]
struct Owner {
    closed: bool,
    active_stop: Option<PathBuf>,
}

impl Owner {
    fn close(&mut self) -> io::Result<()> {
        self.closed = true;
        if let Some(path) = &self.active_stop {
            // Never truncate a caller's existing stop marker.
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_file() => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

struct ActiveJob {
    owner: Arc<Mutex<Owner>>,
    private_stop: Option<PathBuf>,
}

impl ActiveJob {
    fn register(owner: &Arc<Mutex<Owner>>, args: &mut crate::Args) -> Result<Self> {
        let private_stop = if args.stop_file.is_none() {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "words-breaker-worker-{}-{nonce}.stop",
                std::process::id()
            ));
            anyhow::ensure!(!path.exists(), "Worker stop marker already exists");
            args.stop_file = Some(path.clone());
            Some(path)
        } else {
            None
        };
        let mut state = owner.lock().unwrap();
        anyhow::ensure!(!state.closed, "Worker input closed before search started");
        state.active_stop = args.stop_file.clone();
        Ok(Self {
            owner: Arc::clone(owner),
            private_stop,
        })
    }
}

impl Drop for ActiveJob {
    fn drop(&mut self) {
        // Synchronize cleanup with EOF: the reader cannot recreate this marker
        // after cleanup or apply a previous job's stop to its successor.
        let mut owner = self.owner.lock().unwrap();
        owner.active_stop = None;
        if let Some(path) = &self.private_stop {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn run() -> Result<()> {
    crate::output::enable_worker();
    // A panic must not emit an unframed diagnostic or leave a questionable
    // session available for the next request. The loop emits one failed done.
    std::panic::set_hook(Box::new(|_| {}));
    let mut session = crate::Session::default();
    let owner = Arc::new(Mutex::new(Owner::default()));
    let reader_owner = Arc::clone(&owner);
    let (sender, receiver) = mpsc::sync_channel(1);
    // Input is independent from the search thread solely to detect a dead
    // caller. CUDA is initialized, used and dropped on the main thread.
    std::thread::Builder::new()
        .name("worker-input".into())
        .spawn(move || {
            let stdin = io::stdin();
            let mut input = stdin.lock();
            loop {
                let frame = read_frame(&mut input);
                let closed = matches!(frame, Ok(Frame::Eof) | Err(_));
                if closed && reader_owner.lock().unwrap().close().is_err() {
                    // An unavailable stop path must never strand a long-running
                    // orphan. Only already-saved checkpoints survive this exit.
                    std::process::exit(1);
                }
                match sender.try_send(frame) {
                    Ok(()) if !closed => {}
                    Err(mpsc::TrySendError::Full(_)) => {
                        // The protocol allows only one outstanding request.
                        // Never block the reader on pipelining: it must remain
                        // able to stop an orphan after the owner disconnects.
                        if reader_owner.lock().unwrap().close().is_err() {
                            std::process::exit(1);
                        }
                        break;
                    }
                    _ => break,
                }
            }
        })?;
    crate::output::event(&serde_json::json!({"event":"ready", "version":1}))?;
    loop {
        let frame = receiver.recv().context("Worker input reader stopped")??;
        let (id, args) = match frame {
            Frame::Eof => break,
            Frame::TooLarge => (None, Err(anyhow::anyhow!("Request exceeds 1 MiB"))),
            Frame::Line(bytes) => parse_request(&bytes),
        };
        crate::output::set_job(id.as_deref());
        let result = match args {
            Ok(mut args) => {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _active = ActiveJob::register(&owner, &mut args)?;
                    crate::execute(args, &mut session)
                })) {
                    Ok(result) => result,
                    Err(_) => {
                        session.fatal = true;
                        Err(anyhow::anyhow!(
                            "Search panicked; resident engine will exit"
                        ))
                    }
                }
            }
            Err(error) => Err(error),
        };
        crate::output::event(&done(id.as_deref(), &result))?;
        crate::output::set_job(None);
        if session.fatal || owner.lock().unwrap().closed {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(args: &[&str]) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"id":"job-1", "args":args})).unwrap()
    }

    #[test]
    fn validation_keeps_valid_ids_and_rejects_non_search_commands() {
        let (id, result) = parse_request(&request(&[
            "0x0000000000000000000000000000000000000000",
            "--cpu",
        ]));
        assert_eq!(id.as_deref(), Some("job-1"));
        assert!(result.is_ok());
        for args in [
            &["--worker"][..],
            &["--selftest"],
            &["--help"],
            &["--version"],
            &["--list-history"],
            &["--preflight-history"],
            &["--export-checkpoint-record=x"],
            &["--unknown"],
        ] {
            let (id, result) = parse_request(&request(args));
            assert_eq!(id.as_deref(), Some("job-1"));
            assert!(result.is_err());
        }
        let (id, result) = parse_request(br#"{"id":"correct-id","args":[],"unknown":true}"#);
        assert_eq!(id.as_deref(), Some("correct-id"));
        assert!(result.is_err());
        assert!(parse_request(b"invalid json").1.is_err());
    }

    #[test]
    fn requests_are_bounded_and_do_not_accept_control_ids() {
        for id in [
            String::new(),
            "x".repeat(MAX_ID_BYTES + 1),
            "job\nnext".into(),
        ] {
            let bytes = serde_json::to_vec(&serde_json::json!({"id":id,"args":[]})).unwrap();
            let (id, result) = parse_request(&bytes);
            assert!(id.is_none());
            assert!(result.is_err());
        }
        assert!(parse_request(&request(&vec!["x"; MAX_ARGS + 1])).1.is_err());
        assert!(parse_request(&request(&[&"x".repeat(MAX_ARG_BYTES + 1)]))
            .1
            .is_err());
        assert!(parse_request(&request(&["bad\0arg"])).1.is_err());
    }

    #[test]
    fn oversized_frame_is_drained_and_eof_final_frame_is_supported() {
        let mut bytes = vec![b'x'; MAX_LINE_BYTES + 100];
        bytes.extend_from_slice(b"\n{\"id\":\"next\",\"args\":[]}\nlast");
        let mut reader = io::BufReader::with_capacity(31, io::Cursor::new(bytes));
        assert!(matches!(read_frame(&mut reader).unwrap(), Frame::TooLarge));
        let Frame::Line(next) = read_frame(&mut reader).unwrap() else {
            panic!()
        };
        assert_eq!(parse_request(&next).0.as_deref(), Some("next"));
        let Frame::Line(last) = read_frame(&mut reader).unwrap() else {
            panic!()
        };
        assert_eq!(last, b"last");
        assert!(matches!(read_frame(&mut reader).unwrap(), Frame::Eof));
    }
}
