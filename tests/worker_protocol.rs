use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const TARGET: &str = "0x9858EfFD232B4033E47d90003D41EC34EcaEda94";

struct Worker {
    child: Child,
    stdin: Option<ChildStdin>,
    events: mpsc::Receiver<Value>,
    event_timeout: Duration,
}

impl Worker {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_words-breaker"))
            .arg("--worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (sender, events) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let event = serde_json::from_str(&line.unwrap()).unwrap();
                if sender.send(event).is_err() {
                    break;
                }
            }
        });
        let worker = Self {
            child,
            stdin,
            events,
            event_timeout: Duration::from_secs(15),
        };
        assert_eq!(worker.event(), json!({"event":"ready", "version":1}));
        worker
    }

    fn event(&self) -> Value {
        self.events
            .recv_timeout(self.event_timeout)
            .expect("worker event timeout or invalid JSONL")
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.stdin.as_mut().unwrap(), "{message}").unwrap();
    }

    fn until_done(&self) -> Vec<Value> {
        let mut events = Vec::new();
        loop {
            let event = self.event();
            let done = event["event"] == "done";
            events.push(event);
            if done {
                return events;
            }
        }
    }

    fn finish(mut self) {
        self.stdin.take();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "worker failed to exit after EOF");
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut stderr = String::new();
        self.child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert!(stderr.is_empty(), "{stderr}");
    }
}

#[cfg(feature = "cuda")]
fn fixture_address(mnemonic: &bip39::Mnemonic) -> String {
    use bitcoin::bip32::{DerivationPath, Xpriv};
    use bitcoin::secp256k1::Secp256k1;
    use sha3::{Digest, Keccak256};

    // CPU library oracle, independent of the resident process and CUDA code.
    let secp = Secp256k1::new();
    let path: DerivationPath = "m/44'/60'/0'/0/0".parse().unwrap();
    let root =
        Xpriv::new_master(bitcoin::Network::Bitcoin, &mnemonic.to_seed_normalized("")).unwrap();
    let key = root.derive_priv(&secp, &path).unwrap().private_key;
    let public = key.public_key(&secp).serialize_uncompressed();
    let hash = Keccak256::digest(&public[1..]);
    format!("0x{}", hex::encode(&hash[12..]))
}

#[cfg(feature = "cuda")]
fn cuda_test_directory(label: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "worker-cuda-{label}-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&path).unwrap();
    path
}

#[cfg(feature = "cuda")]
#[test]
#[ignore = "requires a physical CUDA device; run explicitly after warming the driver JIT cache"]
fn cuda_worker_reuses_context_and_resets_target_checksum_and_language() {
    use bip39::{Language, Mnemonic};

    let english = Mnemonic::parse_in_normalized(Language::English, PHRASE).unwrap();
    assert_eq!(fixture_address(&english), TARGET.to_lowercase());
    let invalid_phrase = ["abandon"; 12].join(" ");
    assert!(Mnemonic::parse_in_normalized(Language::English, &invalid_phrase).is_err());
    let unchecked =
        Mnemonic::parse_in_normalized_without_checksum_check(Language::English, &invalid_phrase)
            .unwrap();
    let unchecked_target = fixture_address(&unchecked);
    let japanese = Mnemonic::from_entropy_in(Language::Japanese, &[0; 16]).unwrap();
    let japanese_phrase = japanese.to_string();
    assert!(japanese_phrase.len() > 128, "exercise long HMAC key path");
    let japanese_target = fixture_address(&japanese);
    let dir = cuda_test_directory("reuse");
    let negative_metrics = dir.join("negative-metrics.json");
    let hit_record = dir.join("hit-must-not-be-negative.json");
    let requests = [
        (
            "english-hit",
            json!([
                TARGET,
                "--pattern",
                PHRASE,
                "--record-progress",
                hit_record.to_str().unwrap()
            ]),
            true,
        ),
        (
            "same-seed-miss",
            json!([
                "0x0000000000000000000000000000000000000000",
                "--pattern",
                PHRASE,
                "--metrics-json",
                negative_metrics.to_str().unwrap()
            ]),
            false,
        ),
        (
            "checksum-reject",
            json!([unchecked_target, "--pattern", invalid_phrase]),
            false,
        ),
        (
            "unchecked-hit",
            json!([
                unchecked_target,
                "--pattern",
                invalid_phrase,
                "--no-checksum"
            ]),
            true,
        ),
        (
            "japanese-hit",
            json!([
                japanese_target,
                "--pattern",
                japanese_phrase,
                "--language",
                "japanese"
            ]),
            true,
        ),
    ];
    let mut worker = Worker::start();
    worker.event_timeout = Duration::from_secs(120);
    let mut all_events = Vec::new();
    for (id, args, expect_hit) in requests {
        worker.send(&json!({"id":id, "args":args}));
        let events = worker.until_done();
        assert_eq!(
            events.last().unwrap(),
            &json!({"event":"done", "id":id, "success":true, "error":null})
        );
        assert!(events.iter().all(|e| e["id"] == id));
        assert!(
            events.iter().any(|e| e["line"]
                .as_str()
                .is_some_and(|s| s.starts_with("Using GPU (CUDA)"))),
            "physical GPU required: {events:?}"
        );
        let hits = events
            .iter()
            .filter(|e| {
                e["line"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("Found matching mnemonic:"))
            })
            .count();
        assert_eq!(hits, usize::from(expect_hit), "{id}");
        assert_eq!(
            events.iter().any(|e| e["line"]
                .as_str()
                .is_some_and(|s| s.starts_with("Exhausted search without a match"))),
            !expect_hit
        );
        all_events.extend(events);
    }
    worker.finish();
    assert_eq!(
        all_events
            .iter()
            .filter(|e| e["line"] == "CUDA context initialized")
            .count(),
        1
    );
    assert_eq!(
        all_events
            .iter()
            .filter(|e| e["line"] == "Reusing initialized CUDA context")
            .count(),
        4
    );
    assert!(
        !hit_record.exists(),
        "a hit must never create negative evidence"
    );
    let metrics: Value =
        serde_json::from_slice(&std::fs::read(&negative_metrics).unwrap()).unwrap();
    assert_eq!(metrics["backend"], "CUDA");
    assert_eq!(metrics["completed_raw"], 1);
    assert_eq!(metrics["checksum_survivors"], 1);
    for field in ["gpu_seed_seconds", "gpu_address_seconds"] {
        let seconds = metrics[field].as_f64().expect("CUDA event metric missing");
        assert!(seconds.is_finite() && seconds > 0.0, "{field}: {seconds}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "cuda")]
#[test]
#[ignore = "requires a physical CUDA device; run explicitly after warming the driver JIT cache"]
fn cuda_worker_owner_eof_stops_after_confirmed_batches() {
    let dir = cuda_test_directory("eof");
    let checkpoint = dir.join("checkpoint.json");
    let record = dir.join("negative.json");
    let metrics = dir.join("metrics.json");
    let stop = dir.join("pause.request");
    let mut worker = Worker::start();
    worker.event_timeout = Duration::from_secs(120);
    worker.send(&json!({"id":"cuda-eof", "args":[
        "0x0000000000000000000000000000000000000000", "--pattern",
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon ? ? ?",
        "--batch-size", "65536", "--no-checksum", "--checkpoint", checkpoint.to_str().unwrap(),
        "--record-progress", record.to_str().unwrap(), "--metrics-json", metrics.to_str().unwrap(),
        "--stop-file", stop.to_str().unwrap()
    ]}));
    let mut events = Vec::new();
    // Wait for device work to complete, not merely for GPU initialization.
    // The existing periodic progress log is emitted only after committed batches.
    loop {
        let event = worker.event();
        assert_ne!(
            event["event"], "done",
            "test domain must outlast the first progress report"
        );
        assert!(
            !event["line"]
                .as_str()
                .is_some_and(|s| s.starts_with("Using CPU")),
            "physical CUDA required"
        );
        let progressed = event["line"]
            .as_str()
            .is_some_and(|s| s.starts_with("Checked "));
        events.push(event);
        if progressed {
            break;
        }
    }
    worker.stdin.take();
    events.extend(worker.until_done());
    worker.finish();
    assert_eq!(
        events.last().unwrap(),
        &json!({"event":"done", "id":"cuda-eof", "success":true, "error":null})
    );
    assert!(events.iter().all(|e| e["id"] == "cuda-eof"));
    assert!(events.iter().any(|e| e["line"]
        .as_str()
        .is_some_and(|s| s.starts_with("Paused search; checkpoint saved"))));
    assert!(!events.iter().any(|e| e["line"]
        .as_str()
        .is_some_and(|s| s.starts_with("Exhausted search without a match"))));
    assert!(stop.is_file());
    let saved: Value = serde_json::from_slice(&std::fs::read(&checkpoint).unwrap()).unwrap();
    let record: Value = serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
    let metrics: Value = serde_json::from_slice(&std::fs::read(&metrics).unwrap()).unwrap();
    let checked = saved["checked"].as_u64().unwrap();
    assert!(checked > 0 && checked < 2048u64.pow(3));
    assert_eq!(record["record"]["result"], "partial_negative");
    assert_eq!(record["record"]["completed_candidates"], checked);
    assert_eq!(metrics["backend"], "CUDA");
    assert_eq!(metrics["completed_raw"], checked);
    assert_eq!(metrics["checksum_survivors"], checked);
    assert!(metrics["gpu_seed_seconds"].as_f64().unwrap() > 0.0);
    assert!(metrics["gpu_address_seconds"].as_f64().unwrap() > 0.0);
    std::fs::remove_dir_all(dir).unwrap();
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Exercise the actual pipe boundary, a successful public test vector and a
/// following job with a different CPU pool size in exactly the same process.
#[test]
fn worker_correlates_logs_and_recovers_from_invalid_requests_without_replaying() {
    let mut worker = Worker::start();
    let mut events = Vec::new();
    for message in [
        json!({"id":"invalid", "args":["--selftest"]}),
        json!({"id":"match", "args":[TARGET, "--pattern", PHRASE, "--cpu", "--threads", "1", "--metrics"]}),
        json!({"id":"miss", "args":["0x0000000000000000000000000000000000000000", "--pattern", PHRASE, "--cpu", "--threads", "2", "--metrics"]}),
    ] {
        worker.send(&message);
        events.extend(worker.until_done());
    }
    worker.finish();
    let done: Vec<_> = events.iter().filter(|v| v["event"] == "done").collect();
    assert_eq!(done.len(), 3);
    assert_eq!(done[0]["id"], "invalid");
    assert_eq!(done[0]["success"], false);
    assert_eq!(
        done[1],
        &json!({"event":"done", "id":"match", "success":true, "error":null})
    );
    assert_eq!(
        done[2],
        &json!({"event":"done", "id":"miss", "success":true, "error":null})
    );
    let mut current = "invalid";
    let mut hit_count = 0;
    for event in &events {
        assert_eq!(event["id"], current);
        if event["event"] == "done" {
            current = match current {
                "invalid" => "match",
                "match" => "miss",
                "miss" => "finished",
                _ => panic!(),
            };
        } else {
            assert_eq!(event["event"], "log");
            assert!(event["stderr"].is_boolean());
            let line = event["line"].as_str().unwrap();
            assert!(!line.contains('\n'));
            if line.starts_with("Found matching mnemonic:") {
                hit_count += 1;
                assert_eq!(current, "match");
            }
            if line.starts_with("Exhausted search without a match") {
                assert_eq!(current, "miss");
            }
        }
    }
    assert_eq!(hit_count, 1);
    assert_eq!(current, "finished");
    assert!(events
        .iter()
        .any(|v| v["id"] == "match" && v["line"] == "Using CPU with 1 threads"));
    assert!(events
        .iter()
        .any(|v| v["id"] == "miss" && v["line"] == "Using CPU with 2 threads"));
}

#[test]
fn worker_reports_malformed_frame_then_accepts_next_job() {
    let mut worker = Worker::start();
    worker
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"not-json\n")
        .unwrap();
    let malformed = worker.event();
    assert_eq!(malformed["event"], "done");
    assert_eq!(malformed["id"], Value::Null);
    assert_eq!(malformed["success"], false);
    worker.send(
        &json!({"id":"next", "args":[TARGET, "--pattern", PHRASE, "--cpu", "--threads", "1"]}),
    );
    let events = worker.until_done();
    assert_eq!(
        events.last().unwrap(),
        &json!({"event":"done", "id":"next", "success":true, "error":null})
    );
    worker.finish();
}

#[test]
fn owner_eof_stops_active_search_and_saves_only_confirmed_prefix() {
    for external_stop in [false, true] {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("worker-eof-test-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let checkpoint = dir.join("checkpoint.json");
        let record = dir.join("negative.json");
        let stop = dir.join("pause.request");
        let mut args = vec![
            "0x0000000000000000000000000000000000000000".to_owned(),
            "--pattern".into(),
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon ? ?"
                .into(),
            "--cpu".into(),
            "--threads".into(),
            "1".into(),
            "--batch-size".into(),
            "8".into(),
            "--no-checksum".into(),
            "--checkpoint".into(),
            checkpoint.to_str().unwrap().into(),
            "--record-progress".into(),
            record.to_str().unwrap().into(),
        ];
        if external_stop {
            args.extend(["--stop-file".into(), stop.to_str().unwrap().into()]);
        }
        let mut worker = Worker::start();
        worker.send(&json!({"id":"cancel", "args":args}));
        loop {
            let event = worker.event();
            assert_ne!(event["event"], "done", "search must still be active");
            if event["line"] == "Using CPU with 1 threads" {
                break;
            }
        }
        worker.stdin.take();
        let events = worker.until_done();
        assert!(events.iter().any(|v| v["line"]
            .as_str()
            .is_some_and(|line| line.starts_with("Paused search; checkpoint saved"))));
        assert!(!events.iter().any(|v| v["line"]
            .as_str()
            .is_some_and(|line| line.starts_with("Exhausted search without a match"))));
        worker.finish();
        let saved: Value = serde_json::from_slice(&std::fs::read(&checkpoint).unwrap()).unwrap();
        let checked = saved["checked"].as_u64().unwrap();
        assert!(checked < 2048 * 2048);
        if record.exists() {
            assert!(checked > 0);
            let record: Value = serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
            assert_eq!(record["record"]["completed_candidates"], saved["checked"]);
            assert_eq!(record["record"]["result"], "partial_negative");
        } else {
            assert_eq!(checked, 0);
        }
        assert_eq!(stop.exists(), external_stop);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
