//! Ordered diagnostics for normal CLI use and the resident JSONL protocol.

use serde::Serialize;
use std::fmt;
use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};

#[derive(Default)]
struct State {
    worker: bool,
    id: Option<String>,
    broken: bool,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State::default()))
}

pub fn enable_worker() {
    state().lock().unwrap().worker = true;
}

pub fn set_job(id: Option<&str>) {
    state().lock().unwrap().id = id.map(str::to_owned);
}

fn write_event(value: &impl Serialize) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, value)?;
    stdout.write_all(b"\n")?;
    stdout.flush()
}

pub fn event(value: &impl Serialize) -> io::Result<()> {
    // Serialize events and logs together, including logs emitted by producers.
    let mut state = state().lock().unwrap();
    if state.broken {
        return Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "Worker output is closed",
        ));
    }
    let result = write_event(value);
    state.broken |= result.is_err();
    result
}

pub fn line(stderr: bool, args: fmt::Arguments<'_>) {
    let mut state = state().lock().unwrap();
    if !state.worker {
        drop(state);
        if stderr {
            std::eprintln!("{args}");
        } else {
            std::println!("{args}");
        }
        return;
    }
    if state.broken {
        return;
    }
    let text = args.to_string();
    for line in text.split('\n') {
        let value = serde_json::json!({
            "event": "log", "id": state.id, "stderr": stderr,
            "line": line.strip_suffix('\r').unwrap_or(line),
        });
        if write_event(&value).is_err() {
            state.broken = true;
            break;
        }
    }
}
