#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, State};

mod file_import;
mod file_queue;
mod mcp_bridge;

type UiResult<T> = Result<T, String>;
const LOG_LIMIT: usize = 300;
const HISTORY_COVERED_MESSAGE: &str = "Todas as combinações desta busca já foram testadas. Altere as palavras ou as posições para explorar um espaço novo.";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchConfig {
    name: String,
    target: String,
    mode: String,
    pattern: String,
    pool: String,
    fill: String,
    post: String,
    video: String,
    backend: String,
    language: String,
    max_candidates: String,
    batch_size: usize,
    threads: usize,
    adaptive: bool,
    no_checksum: bool,
    exclude_ro1: bool,
    exclude_records: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSnapshot {
    id: String,
    name: String,
    status: String,
    config: SearchConfig,
    started_at: String,
    elapsed_seconds: f64,
    checked: String,
    excluded: String,
    total: Option<String>,
    rate: f64,
    backend: String,
    logs: Vec<String>,
    metrics: Option<Value>,
    error: Option<String>,
    #[serde(default)]
    auto_history_records: usize,
    #[serde(default)]
    history_snapshot: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    history_export_eligible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    queue_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    engine_available: bool,
    engine_path: String,
    runs: Vec<RunSnapshot>,
    active: Option<RunSnapshot>,
    queue: Option<file_queue::FileQueue>,
}

#[derive(Serialize)]
pub struct ViewState {
    active: Option<RunSnapshot>,
    runs: Vec<RunSnapshot>,
    queue: Option<file_queue::FileQueue>,
}

struct Runtime {
    started: Instant,
    base_elapsed: f64,
    base_checked: u128,
    found: bool,
    exhausted: bool,
    limited: bool,
    pause_reported: bool,
    pause_requested: bool,
}

#[derive(Default)]
struct Inner {
    active: Option<RunSnapshot>,
    runtime: Option<Runtime>,
    runs: Vec<RunSnapshot>,
    close_when_done: bool,
    queue: Option<file_queue::FileQueue>,
    imported: Option<file_import::ImportPreview>,
    queue_worker: bool,
}

struct Studio {
    inner: Mutex<Inner>,
    // Serialize history migration and snapshot preparation across GUI and MCP.
    preparation: Mutex<()>,
    runs_dir: PathBuf,
    engine: PathBuf,
    history_dir: PathBuf,
    queue_path: PathBuf,
    // Held for the entire application lifetime. Never remove the lock file:
    // deleting it would let another process lock a different file at this path.
    _instance_lock: fs::File,
}

fn acquire_instance_lock(path: &Path) -> Result<fs::File, fs::TryLockError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(fs::TryLockError::Error)?;
    file.try_lock()?;
    Ok(file)
}

/// Validate the physical parent, not the spelling returned by Windows for an
/// absolute path. Canonicalization first resolves a run junction/symlink, so an
/// alias to a directory outside the managed root still fails this check.
fn managed_run_path(root: &Path, candidate: &Path) -> UiResult<PathBuf> {
    let resolved = candidate.canonicalize().map_err(|e| e.to_string())?;
    if !resolved.is_dir() {
        return Err("A execução não aponta para uma pasta válida.".into());
    }
    let parent = resolved
        .parent()
        .ok_or("A pasta da execução não possui diretório pai.")?;
    let same_parent = same_file::is_same_file(parent, root)
        .map_err(|e| format!("Não foi possível verificar a pasta da execução: {e}"))?;
    if !same_parent {
        return Err("A pasta da execução está fora do diretório gerenciado.".into());
    }
    Ok(resolved)
}

impl Studio {
    fn lock(&self) -> UiResult<MutexGuard<'_, Inner>> {
        self.inner
            .lock()
            .map_err(|_| "O estado da aplicação não está disponível.".into())
    }

    fn run_dir(&self, id: &str) -> UiResult<PathBuf> {
        if id.is_empty() || id.len() > 80 || !id.bytes().all(|c| c.is_ascii_digit() || c == b'-') {
            return Err("Identificador de execução inválido.".into());
        }
        let path = self.runs_dir.join(id);
        // Canonicalize existing run directories to reject links outside app data.
        if path.exists() {
            managed_run_path(&self.runs_dir, &path)
        } else {
            Ok(path)
        }
    }
}

fn unique_id() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        time::OffsetDateTime::now_utc().unix_timestamp_nanos(),
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn now_string() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

fn push_log(run: &mut RunSnapshot, line: String) {
    run.logs.push(line.chars().take(4000).collect());
    if run.logs.len() > LOG_LIMIT {
        run.logs.drain(..run.logs.len() - LOG_LIMIT);
    }
}

fn persist(dir: &Path, run: &RunSnapshot) -> UiResult<()> {
    let bytes = serde_json::to_vec_pretty(run).map_err(|e| e.to_string())?;
    let temporary = dir.join(format!("state-{}.tmp", unique_id()));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, dir.join("state.json"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e| format!("Não foi possível salvar a execução: {e}"))
}

fn read_json(path: &Path) -> Option<Value> {
    // Files are produced by the local engine. Cap reads in case an unrelated
    // file has been placed in the managed directory.
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > 8 * 1024 * 1024 {
        return None;
    }
    serde_json::from_reader(file).ok()
}

fn json_count(value: &Value, key: &str) -> Option<String> {
    let value = value.get(key)?;
    let text = if let Some(text) = value.as_str() {
        text.to_owned()
    } else {
        value.to_string()
    };
    text.parse::<u128>().ok().map(|n| n.to_string())
}

fn checkpoint_counts(dir: &Path) -> Option<(String, String)> {
    let value = read_json(&dir.join("checkpoint.json"))?;
    Some((
        json_count(&value, "checked")?,
        json_count(&value, "excluded").unwrap_or_else(|| "0".into()),
    ))
}

fn recover_run(dir: &Path) -> Option<RunSnapshot> {
    let value = read_json(&dir.join("state.json"))?;
    let mut run: RunSnapshot = serde_json::from_value(value).ok()?;
    if dir.file_name().and_then(|name| name.to_str()) != Some(run.id.as_str()) {
        return None;
    }
    run.logs = run
        .logs
        .into_iter()
        .rev()
        .take(LOG_LIMIT)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if ["running", "pausing"].contains(&run.status.as_str()) {
        // An interrupted UI session has no confirmed final engine outcome.
        run.history_export_eligible = Some(false);
        if let Some((checked, excluded)) = checkpoint_counts(dir) {
            run.checked = checked;
            run.excluded = excluded;
            run.status = "paused".into();
            push_log(
                &mut run,
                "Sessão anterior interrompida. Checkpoint disponível para retomada.".into(),
            );
        } else {
            run.status = "failed".into();
            run.error = Some("Sessão anterior interrompida antes do primeiro checkpoint.".into());
        }
        let _ = persist(dir, &run);
    }
    Some(run)
}

fn engine_path(app: &AppHandle) -> PathBuf {
    let bundled = app
        .path()
        .resource_dir()
        .unwrap_or_default()
        .join("engine/words-breaker.exe");
    if bundled.is_file() {
        return bundled;
    }
    #[cfg(debug_assertions)]
    {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        for candidate in [
            manifest.join("engine/words-breaker.exe"),
            manifest.join("../../../gpu-target/release/words-breaker.exe"),
            manifest.join("../../target/release/words-breaker.exe"),
        ] {
            if candidate.is_file() {
                return candidate.canonicalize().unwrap_or(candidate);
            }
        }
    }
    bundled
}

fn validate(config: &mut SearchConfig) -> UiResult<()> {
    // RO1 only certifies the checksum-filtered domain. Never apply that
    // certificate to an expanded search, including calls from MCP or a queue.
    if config.no_checksum {
        config.exclude_ro1 = false;
    }
    config.name = config.name.trim().to_owned();
    config.target = config.target.trim().to_owned();
    config.max_candidates = config.max_candidates.trim().to_owned();
    if config.name.is_empty() || config.name.chars().count() > 100 {
        return Err("Dê um nome de até 100 caracteres para a busca.".into());
    }
    if config.target.len() != 42
        || !config.target.starts_with("0x")
        || !config.target[2..].bytes().all(|c| c.is_ascii_hexdigit())
    {
        return Err("Informe um endereço Ethereum válido, começando com 0x.".into());
    }
    if !["auto", "cpu"].contains(&config.backend.as_str()) {
        return Err("Processador inválido.".into());
    }
    if ![
        "english",
        "portuguese",
        "spanish",
        "french",
        "italian",
        "czech",
        "korean",
        "japanese",
        "chinese-simplified",
        "chinese-traditional",
    ]
    .contains(&config.language.as_str())
    {
        return Err("Idioma inválido.".into());
    }
    match config.mode.as_str() {
        "template" if config.pattern.split_whitespace().count() == 12 => {}
        "template" => {
            return Err("O modelo precisa ter 12 posições; use ? nas desconhecidas.".into())
        }
        "batches" if !config.post.trim().is_empty() && !config.video.trim().is_empty() => {}
        "batches" => return Err("Preencha as palavras do post e do vídeo.".into()),
        _ => return Err("Modo de busca inválido.".into()),
    }
    if config.batch_size == 0
        || config.batch_size > (u32::MAX / 12) as usize
        || config.threads > 1024
    {
        return Err("Tamanho do lote ou quantidade de threads inválidos.".into());
    }
    if !config.max_candidates.is_empty()
        && !config.max_candidates.parse::<usize>().is_ok_and(|n| n > 0)
    {
        return Err("O limite precisa ser um número inteiro positivo ou ficar em branco.".into());
    }
    if [
        &config.pattern,
        &config.pool,
        &config.fill,
        &config.post,
        &config.video,
    ]
    .iter()
    .any(|text| text.len() > 65_536 || text.contains('\0'))
    {
        return Err("Lista de palavras inválida ou muito extensa.".into());
    }
    if config.exclude_records.len() > 64 {
        return Err("Selecione no máximo 64 registros por busca.".into());
    }
    for path in &mut config.exclude_records {
        let resolved = Path::new(path)
            .canonicalize()
            .map_err(|e| format!("Registro indisponível: {e}"))?;
        if !resolved.is_file()
            || !resolved
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            return Err("Os registros precisam ser arquivos JSON existentes.".into());
        }
        *path = resolved.to_string_lossy().into_owned();
    }
    Ok(())
}

fn selector_args(config: &SearchConfig) -> Vec<std::ffi::OsString> {
    let mut args = vec![config.target.clone().into()];
    let mut add = |key: &str, value: &str| {
        args.push(key.into());
        args.push(value.into());
    };
    if config.mode == "template" {
        add("--pattern", &config.pattern);
        if !config.pool.trim().is_empty() {
            add("--pool", &config.pool);
        }
        if !config.fill.trim().is_empty() {
            add("--fill", &config.fill);
        }
    } else {
        add("--post", &config.post);
        add("--video", &config.video);
    }
    add("--language", &config.language);
    for record in &config.exclude_records {
        add("--exclude-record", record);
    }
    if config.exclude_ro1 {
        add("--exclude-tested", "RO1");
    }
    if config.no_checksum {
        args.push("--no-checksum".into());
    }
    args
}

fn exclusion_args(run: &RunSnapshot, dir: &Path) -> Vec<std::ffi::OsString> {
    let mut args = selector_args(&run.config);
    if run.history_snapshot {
        args.extend([
            "--exclude-record-dir".into(),
            dir.join("history").into_os_string(),
        ]);
    }
    args
}

fn command_args(
    config: &SearchConfig,
    dir: &Path,
    metrics: &Path,
    resume: bool,
) -> Vec<std::ffi::OsString> {
    let mut args = selector_args(config);
    args.extend([
        "--batch-size".into(),
        config.batch_size.to_string().into(),
        "--threads".into(),
        config.threads.to_string().into(),
    ]);
    if !config.max_candidates.is_empty() {
        args.extend([
            "--max-candidates".into(),
            config.max_candidates.clone().into(),
        ]);
    }
    if config.backend == "cpu" {
        args.push("--cpu".into());
    }
    if config.adaptive {
        args.push("--adaptive-batch".into());
    }
    if resume {
        args.push("--resume".into());
    }
    args.extend([
        "--checkpoint".into(),
        dir.join("checkpoint.json").into_os_string(),
        "--stop-file".into(),
        dir.join("stop").into_os_string(),
        "--metrics-json".into(),
        metrics.as_os_str().to_owned(),
    ]);
    args
}

fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = command;
}

fn engine_control(state: &Studio, dir: &Path, args: Vec<std::ffi::OsString>) -> UiResult<String> {
    let mut command = Command::new(&state.engine);
    command
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut command);
    // wait_with_output drains stdout and stderr together; these control modes
    // never initialize CUDA or perform address derivation.
    let output = command
        .output()
        .map_err(|e| format!("Não foi possível consultar o histórico: {e}"))?;
    if !output.status.success() {
        let details = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Não foi possível verificar o histórico: {}",
            details.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn eligible_history_export(run: &RunSnapshot) -> bool {
    ["completed", "limited", "paused"].contains(&run.status.as_str())
        && run.history_export_eligible.unwrap_or_else(|| {
            run.status != "paused"
                || !run
                    .logs
                    .iter()
                    .any(|line| line.starts_with("Sessão anterior interrompida."))
        })
}

fn import_previous_runs(state: &Studio) -> UiResult<usize> {
    let runs = state.lock()?.runs.clone();
    let mut imported = 0;
    for run in runs.iter().filter(|run| eligible_history_export(run)) {
        let dir = state.run_dir(&run.id)?;
        let Some((checked, _)) = checkpoint_counts(&dir) else {
            continue;
        };
        if checked == "0" {
            continue;
        }
        let destination = state.history_dir.join(format!("{}.json", run.id));
        if destination.exists() {
            continue;
        }
        if run.history_snapshot && !dir.join("history").is_dir() {
            return Err(format!(
                "O histórico fixo da execução '{}' não está disponível.",
                run.name
            ));
        }
        let mut args = exclusion_args(run, &dir);
        args.extend([
            "--checkpoint".into(),
            dir.join("checkpoint.json").into_os_string(),
            "--export-checkpoint-record".into(),
            destination.as_os_str().to_owned(),
        ]);
        engine_control(state, &dir, args)
            .map_err(|error| format!("Histórico de '{}': {error}", run.name))?;
        imported += usize::from(destination.is_file());
    }
    Ok(imported)
}

fn json_files(dir: &Path) -> UiResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn snapshot_history(source: &Path, dir: &Path, run: &mut RunSnapshot) -> UiResult<()> {
    let destination = dir.join("history");
    fs::create_dir(&destination)
        .map_err(|e| format!("Não foi possível fixar o histórico desta busca: {e}"))?;
    let records = json_files(source)?;
    for source in &records {
        let name = source.file_name().ok_or("Nome de registro inválido.")?;
        fs::copy(source, destination.join(name))
            .map_err(|e| format!("Não foi possível copiar um registro do histórico: {e}"))?;
    }
    // Explicit selections are also copied. A later change to the original
    // file must not silently alter a resumed run's exclusion fingerprint.
    if !run.config.exclude_records.is_empty() {
        let selected = dir.join("selected-records");
        fs::create_dir(&selected).map_err(|e| e.to_string())?;
        for (index, source) in run.config.exclude_records.iter_mut().enumerate() {
            let destination = selected.join(format!("{index}.json"));
            fs::copy(&*source, &destination)
                .map_err(|e| format!("Não foi possível fixar o registro selecionado: {e}"))?;
            *source = destination.to_string_lossy().into_owned();
        }
    }
    run.auto_history_records = records.len();
    run.history_snapshot = true;
    push_log(run, format!("{} arquivos de histórico automático fixados nesta busca; o motor seleciona os compatíveis.", records.len()));
    Ok(())
}

fn history_preflight(state: &Studio, dir: &Path, run: &mut RunSnapshot) -> UiResult<()> {
    let mut args = exclusion_args(run, dir);
    args.push("--preflight-history".into());
    let output = engine_control(state, dir, args)?;
    let json = output
        .lines()
        .find_map(|line| line.strip_prefix("History preflight: "))
        .ok_or("O motor não retornou a verificação de histórico esperada.")?;
    let value: Value =
        serde_json::from_str(json).map_err(|e| format!("Resposta de histórico inválida: {e}"))?;
    let covered = value
        .get("fullyCovered")
        .and_then(Value::as_bool)
        .ok_or("Verificação de cobertura inválida.")?;
    run.total =
        Some(json_count(&value, "total").ok_or("Total de combinações inválido no histórico.")?);
    if covered {
        return Err(HISTORY_COVERED_MESSAGE.into());
    }
    Ok(())
}

fn preparation_failure(mut run: RunSnapshot, error: &str) -> RunSnapshot {
    run.error = Some(error.to_owned());
    run.history_export_eligible = Some(false);
    if error == HISTORY_COVERED_MESSAGE {
        run.status = "covered".into();
        run.checked = "0".into();
        run.excluded = "0".into();
        run.elapsed_seconds = 0.0;
        run.rate = 0.0;
        run.metrics = None;
        push_log(
            &mut run,
            "Busca não iniciada: todas as combinações já foram testadas em execuções anteriores."
                .into(),
        );
    } else {
        run.status = "failed".into();
    }
    run
}

fn update_timing(inner: &mut Inner) {
    if let (Some(run), Some(runtime)) = (&mut inner.active, &inner.runtime) {
        let seconds = runtime.started.elapsed().as_secs_f64();
        run.elapsed_seconds = runtime.base_elapsed + seconds;
        let checked = run.checked.parse::<u128>().unwrap_or(0);
        run.rate = if seconds > 0.0 {
            checked.saturating_sub(runtime.base_checked) as f64 / seconds
        } else {
            0.0
        };
    }
}

fn parse_line(run: &mut RunSnapshot, runtime: &mut Runtime, line: &str) {
    if let Some(exact) = line
        .split(" exact;")
        .next()
        .filter(|_| line.starts_with("Searching "))
    {
        if let Some(raw) = exact
            .rsplit('(')
            .next()
            .and_then(|n| n.parse::<u128>().ok())
        {
            run.total = Some(raw.to_string());
        }
    }
    if let Some(rest) = line.strip_prefix("Checked ") {
        if let Some(n) = rest.split_whitespace().next().and_then(parse_display_count) {
            // CPU prints cumulative progress; GPU prints this invocation only.
            let checked = if run.backend == "CUDA" {
                runtime.base_checked.saturating_add(n)
            } else {
                n
            };
            if checked > run.checked.parse::<u128>().unwrap_or(0) {
                run.checked = checked.to_string();
            }
        }
    }
    if line.starts_with("Using GPU") {
        run.backend = "CUDA".into();
    }
    if line.starts_with("Using CPU") {
        run.backend = "CPU".into();
    }
    if line.starts_with("History excluded ") {
        if let Some(raw) = line
            .split('(')
            .nth(1)
            .and_then(|part| part.split_whitespace().next())
            .and_then(parse_display_count)
        {
            run.excluded = raw.to_string();
        }
    }
    runtime.found |= line.starts_with("Found matching mnemonic:");
    runtime.exhausted |= line.starts_with("Exhausted search without a match");
    runtime.limited |= line.starts_with("Candidate limit reached");
    runtime.pause_reported |= line.to_ascii_lowercase().contains("search paused");
    runtime.pause_reported |= line.starts_with("Paused search");
}

fn parse_display_count(text: &str) -> Option<u128> {
    let text = text.replace([',', '_'], "");
    let scale = match text.as_bytes().last()? {
        b'K' => 1_000u128,
        b'M' => 1_000_000,
        b'G' => 1_000_000_000,
        b'T' => 1_000_000_000_000,
        _ => return text.parse().ok(),
    };
    let number = &text[..text.len() - 1];
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let base = whole.parse::<u128>().ok()?.checked_mul(scale)?;
    if fraction.is_empty() {
        return Some(base);
    }
    let divisor = 10u128.checked_pow(fraction.len().try_into().ok()?)?;
    base.checked_add(fraction.parse::<u128>().ok()?.checked_mul(scale)? / divisor)
}

fn pump(
    app: AppHandle,
    id: String,
    reader: impl Read + Send + 'static,
    stderr: bool,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            let line = match line {
                Ok(line) => line,
                Err(_) => break,
            };
            let state = app.state::<Studio>();
            let snapshot = {
                let Ok(mut inner) = state.lock() else { break };
                if !inner.active.as_ref().is_some_and(|run| run.id == id) {
                    break;
                }
                let Inner {
                    active, runtime, ..
                } = &mut *inner;
                let (Some(run), Some(runtime)) = (active, runtime) else {
                    break;
                };
                parse_line(run, runtime, &line);
                push_log(
                    run,
                    if stderr {
                        format!("[engine] {line}")
                    } else {
                        line
                    },
                );
                update_timing(&mut inner);
                inner.active.clone()
            };
            let _ = app.emit("run-updated", snapshot);
        }
    })
}

fn finish(
    app: &AppHandle,
    id: &str,
    dir: &Path,
    metrics_path: &Path,
    result: Result<std::process::ExitStatus, std::io::Error>,
) {
    let counts = checkpoint_counts(dir);
    let metrics = read_json(metrics_path);
    let stopped = dir.join("stop").exists();
    let state = app.state::<Studio>();
    let mut snapshot = {
        let Ok(mut inner) = state.lock() else { return };
        if !inner.active.as_ref().is_some_and(|run| run.id == id) {
            return;
        }
        update_timing(&mut inner);
        let mut run = inner.active.as_ref().unwrap().clone();
        let runtime = inner.runtime.take().unwrap();
        if let Some((checked, excluded)) = &counts {
            run.checked = checked.clone();
            run.excluded = excluded.clone();
        }
        run.metrics = metrics;
        let success = result.as_ref().is_ok_and(|status| status.success());
        run.status = if !success {
            run.error = Some(match result {
                Ok(status) => format!(
                    "O motor terminou com código {}. Confira o registro abaixo.",
                    status
                        .code()
                        .map_or("desconhecido".into(), |code| code.to_string())
                ),
                Err(error) => format!("Não foi possível acompanhar o motor: {error}"),
            });
            "failed"
        } else if runtime.found {
            "found"
        } else if counts.is_none() {
            run.error = Some("O motor terminou sem um checkpoint válido.".into());
            "failed"
        } else if runtime.exhausted {
            "completed"
        } else if stopped || runtime.pause_reported {
            "paused"
        } else if runtime.limited {
            "limited"
        } else if run
            .total
            .as_ref()
            .is_some_and(|n| Some(n) == Some(&run.checked))
        {
            "completed"
        } else {
            run.error = Some("O motor terminou sem indicar conclusão ou pausa.".into());
            "failed"
        }
        .into();
        run.history_export_eligible =
            Some(["completed", "limited", "paused"].contains(&run.status.as_str()));
        let seconds = runtime.started.elapsed().as_secs_f64();
        run.rate = if seconds > 0.0 {
            run.checked
                .parse::<u128>()
                .unwrap_or(0)
                .saturating_sub(runtime.base_checked) as f64
                / seconds
        } else {
            0.0
        };
        // Keep the reservation until persistence ends, so a quick resume
        // cannot be overwritten by this invocation's final state file.
        inner.active = Some(run.clone());
        run
    };
    if let Err(error) = persist(dir, &snapshot) {
        push_log(&mut snapshot, error);
    }
    let close = if let Ok(mut inner) = state.lock() {
        inner.active = None;
        inner.runs.retain(|old| old.id != id);
        inner.runs.insert(0, snapshot.clone());
        inner.close_when_done
    } else {
        false
    };
    let _ = app.emit("run-updated", &snapshot);
    if close {
        app.exit(0);
    }
}

fn launch(app: &AppHandle, mut run: RunSnapshot, resume: bool) -> UiResult<RunSnapshot> {
    let state = app.state::<Studio>();
    let _preparation = state
        .preparation
        .lock()
        .map_err(|_| "A preparação da busca não está disponível.")?;
    if !state.engine.is_file() {
        return Err("Motor de busca indisponível. Reinstale o aplicativo ou prepare o engine do build local.".into());
    }
    validate(&mut run.config)?;
    let dir = state.run_dir(&run.id)?;
    if resume && checkpoint_counts(&dir).is_none() {
        return Err("Esta execução não tem um checkpoint válido para retomar.".into());
    }
    // Reserve the single execution before any process or file operations.
    {
        let mut inner = state.lock()?;
        file_queue::authorize_launch(&inner, &run)?;
        if inner.active.is_some() || inner.close_when_done {
            return Err("Já existe uma busca em andamento. Pause-a antes de iniciar outra.".into());
        }
        run.status = "running".into();
        run.error = None;
        run.history_export_eligible = Some(false);
        inner.runtime = Some(Runtime {
            started: Instant::now(),
            base_elapsed: run.elapsed_seconds,
            base_checked: run.checked.parse().unwrap_or(0),
            found: false,
            exhausted: false,
            limited: false,
            pause_reported: false,
            pause_requested: false,
        });
        inner.active = Some(run.clone());
        inner.close_when_done = false;
        inner.runs.retain(|old| old.id != run.id);
    }
    let metrics_path = dir.join(format!("metrics-{}.json", unique_id()));
    let preparation = (|| -> UiResult<_> {
        if !resume {
            fs::create_dir(&dir).map_err(|e| format!("Não foi possível criar a execução: {e}"))?;
            let imported = import_previous_runs(&state)?;
            if imported > 0 {
                push_log(
                    &mut run,
                    format!(
                        "{imported} execuções anteriores importadas para o histórico automático."
                    ),
                );
            }
            snapshot_history(&state.history_dir, &dir, &mut run)?;
            history_preflight(&state, &dir, &mut run)?;
        } else if run.history_snapshot && !dir.join("history").is_dir() {
            return Err("O histórico fixo desta busca não está disponível; a retomada foi bloqueada para preservar o checkpoint.".into());
        }
        if resume {
            let imported = import_previous_runs(&state)?;
            if imported > 0 {
                push_log(
                    &mut run,
                    format!(
                        "{imported} execuções anteriores importadas para o histórico automático."
                    ),
                );
            }
            run.auto_history_records = json_files(&state.history_dir)?.len();
            push_log(&mut run, "Retomada com novos registros compatíveis do histórico automático; o histórico original do checkpoint permanece fixo.".into());
        }
        // This exact file is owned by this run; no user-supplied removal paths.
        if resume && dir.join("stop").exists() {
            fs::remove_file(dir.join("stop"))
                .map_err(|e| format!("Não foi possível liberar a pausa: {e}"))?;
        }
        // Preserve a close/pause request racing removal of an old stop file.
        if state
            .lock()?
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.pause_requested)
        {
            fs::write(dir.join("stop"), b"Pause requested during initialization\n")
                .map_err(|e| e.to_string())?;
        }
        {
            let mut inner = state.lock()?;
            if inner
                .runtime
                .as_ref()
                .is_some_and(|runtime| runtime.pause_requested)
            {
                run.status = "pausing".into();
            }
            inner.active = Some(run.clone());
        }
        persist(&dir, &run)?;
        let mut args = command_args(&run.config, &dir, &metrics_path, resume);
        if run.history_snapshot {
            args.extend([
                "--exclude-record-dir".into(),
                dir.join("history").into_os_string(),
            ]);
        }
        if resume {
            args.extend([
                "--additional-record-dir".into(),
                state.history_dir.as_os_str().to_owned(),
            ]);
        }
        args.extend([
            "--record-progress".into(),
            state
                .history_dir
                .join(format!("{}.json", run.id))
                .into_os_string(),
        ]);
        let mut command = Command::new(&state.engine);
        command
            .args(args)
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_console(&mut command);
        command
            .spawn()
            .map_err(|e| format!("Não foi possível iniciar o motor: {e}"))
    })();
    let mut child = match preparation {
        Ok(child) => child,
        Err(error) => {
            let mut inner = state.lock()?;
            // The prepared run owns the copied exclusions and preflight total;
            // the initial active reservation may not yet contain those fields.
            let failed = preparation_failure(run, &error);
            inner.runtime = None;
            inner.active = Some(failed.clone());
            drop(inner);
            if dir.is_dir() {
                let _ = persist(&dir, &failed);
            }
            let close = {
                let mut inner = state.lock()?;
                inner.active = None;
                inner.runs.insert(0, failed);
                inner.close_when_done
            };
            if close {
                app.exit(0);
            }
            return Err(error);
        }
    };
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let output = pump(app.clone(), run.id.clone(), stdout, false);
    let errors = pump(app.clone(), run.id.clone(), stderr, true);
    let handle = app.clone();
    let id = run.id.clone();
    std::thread::spawn(move || {
        let result = child.wait();
        let _ = output.join();
        let _ = errors.join();
        finish(&handle, &id, &dir, &metrics_path, result);
    });
    Ok(run)
}

fn refresh(state: &Studio) -> UiResult<ViewState> {
    let active_id = state.lock()?.active.as_ref().map(|run| run.id.clone());
    let counts = active_id
        .as_ref()
        .and_then(|id| state.run_dir(id).ok())
        .and_then(|dir| checkpoint_counts(&dir));
    let mut inner = state.lock()?;
    if let (Some(run), Some(id), Some((checked, excluded))) = (&mut inner.active, active_id, counts)
    {
        if run.id == id {
            // A ten-second checkpoint may trail a newer progress line.
            if checked.parse::<u128>().unwrap_or(0) >= run.checked.parse::<u128>().unwrap_or(0) {
                run.checked = checked;
            }
            run.excluded = excluded;
        }
    }
    update_timing(&mut inner);
    Ok(ViewState {
        active: inner.active.clone(),
        runs: inner.runs.clone(),
        queue: inner.queue.clone(),
    })
}

#[tauri::command]
fn bootstrap(state: State<'_, Studio>) -> UiResult<Bootstrap> {
    let view = refresh(&state)?;
    Ok(Bootstrap {
        engine_available: state.engine.is_file(),
        engine_path: state.engine.to_string_lossy().into_owned(),
        runs: view.runs,
        active: view.active,
        queue: view.queue,
    })
}

#[tauri::command]
fn get_state(state: State<'_, Studio>) -> UiResult<ViewState> {
    refresh(&state)
}

#[tauri::command]
async fn start_search(app: AppHandle, mut config: SearchConfig) -> UiResult<RunSnapshot> {
    validate(&mut config)?;
    let run = new_run(config);
    tauri::async_runtime::spawn_blocking(move || launch(&app, run, false))
        .await
        .map_err(|e| e.to_string())?
}

fn new_run(config: SearchConfig) -> RunSnapshot {
    RunSnapshot {
        id: unique_id(),
        name: config.name.clone(),
        backend: if config.backend == "cpu" {
            "CPU".into()
        } else {
            "auto".into()
        },
        config,
        status: "running".into(),
        started_at: now_string(),
        elapsed_seconds: 0.0,
        checked: "0".into(),
        excluded: "0".into(),
        total: None,
        rate: 0.0,
        logs: vec![],
        metrics: None,
        error: None,
        auto_history_records: 0,
        history_snapshot: false,
        history_export_eligible: Some(false),
        queue_id: None,
    }
}

fn request_stop(state: &Studio) -> UiResult<RunSnapshot> {
    let id = {
        let mut inner = state.lock()?;
        let id = inner
            .active
            .as_ref()
            .map(|run| run.id.clone())
            .ok_or("Nenhuma busca em andamento.")?;
        let runtime = inner
            .runtime
            .as_mut()
            .ok_or("A busca já terminou; salvando o resultado.")?;
        runtime.pause_requested = true;
        id
    };
    let dir = state.run_dir(&id)?;
    if dir.is_dir() {
        fs::write(dir.join("stop"), b"Pause requested by ETH Search Studio\n")
            .map_err(|e| format!("Não foi possível solicitar a pausa: {e}"))?;
    }
    let snapshot = {
        let mut inner = state.lock()?;
        let run = inner
            .active
            .as_mut()
            .filter(|run| run.id == id)
            .ok_or("A busca já terminou.")?;
        run.status = "pausing".into();
        push_log(
            run,
            "Pausa solicitada. Aguardando o lote atual e o checkpoint.".into(),
        );
        run.clone()
    };
    // The final worker persists the confirmed state after engine exit. Avoid
    // racing its final snapshot with a late write of a 'pausing' snapshot.
    Ok(snapshot)
}

#[tauri::command]
fn pause_search(state: State<'_, Studio>) -> UiResult<RunSnapshot> {
    file_queue::mark_paused(&state)?;
    request_stop(&state)
}

#[tauri::command]
async fn resume_search(app: AppHandle, run_id: String) -> UiResult<RunSnapshot> {
    let run = {
        let state = app.state::<Studio>();
        let inner = state.lock()?;
        if inner.queue.as_ref().is_some_and(file_queue::reserved) {
            return Err("Use Continuar fila para retomar uma importação, ou encerre a fila antes de retomar outra busca.".into());
        }
        let run = inner
            .runs
            .iter()
            .find(|run| run.id == run_id)
            .ok_or("Execução não encontrada.")?;
        if !["paused", "limited", "failed"].contains(&run.status.as_str()) {
            return Err("Esta execução não pode ser retomada.".into());
        }
        run.clone()
    };
    tauri::async_runtime::spawn_blocking(move || launch(&app, run, true))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn pick_records() -> UiResult<Vec<String>> {
    tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("Selecionar registros de buscas concluídas")
            .add_filter("Registros JSON", &["json"])
            .pick_files()
            .unwrap_or_default()
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_run_folder(state: State<'_, Studio>, run_id: String) -> UiResult<()> {
    {
        let inner = state.lock()?;
        if !inner.active.as_ref().is_some_and(|run| run.id == run_id)
            && !inner.runs.iter().any(|run| run.id == run_id)
        {
            return Err("Execução não encontrada.".into());
        }
    }
    let dir = state.run_dir(&run_id)?;
    if !dir.is_dir() {
        return Err("A pasta desta execução não está disponível.".into());
    }
    #[cfg(windows)]
    {
        let mut command = Command::new("explorer.exe");
        command
            .arg(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console(&mut command);
        command.spawn().map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(windows))]
    Err("A abertura de pastas está disponível no Windows.".into())
}

fn defer_close(app: &AppHandle) -> bool {
    let state = app.state::<Studio>();
    let _ = file_queue::mark_paused(&state);
    let (active, needs_stop) = if let Ok(mut inner) = state.lock() {
        inner.close_when_done = true;
        if inner.active.is_some() {
            inner.close_when_done = true;
            (true, inner.runtime.is_some())
        } else {
            (false, false)
        }
    } else {
        (false, false)
    };
    if needs_stop {
        if let Err(error) = request_stop(&state) {
            if let Ok(mut inner) = state.lock() {
                if inner.runtime.is_some() {
                    inner.close_when_done = false;
                    if let Some(run) = &mut inner.active {
                        push_log(run, error);
                    }
                }
            }
        }
    }
    active
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let app_data = app.path().app_local_data_dir()?;
            #[cfg(debug_assertions)]
            let app_data = std::env::var_os("ETH_STUDIO_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or(app_data);
            let runs_dir = app_data.join("runs");
            fs::create_dir_all(&runs_dir)?;
            // Acquire before inspecting or recovering any existing run state.
            let instance_lock = match acquire_instance_lock(&app_data.join("studio.lock")) {
                Ok(file) => file,
                Err(fs::TryLockError::WouldBlock) => {
                    // Adapter startup may race a GUI launch or encounter an
                    // older desktop without discovery support. Never leave a
                    // modal dialog behind for a background connection attempt.
                    if std::env::args().any(|arg| arg == "--mcp-auto-launch") {
                        std::process::exit(0);
                    }
                    let _ = rfd::MessageDialog::new()
                        .set_title("ETH Search Studio já está aberto")
                        .set_description("Use a janela já aberta para acompanhar ou retomar suas buscas.")
                        .set_level(rfd::MessageLevel::Info)
                        .set_buttons(rfd::MessageButtons::Ok)
                        .show();
                    // This instance has not loaded history or started an engine.
                    std::process::exit(0);
                }
                Err(fs::TryLockError::Error(error)) => {
                    let _ = rfd::MessageDialog::new()
                        .set_title("Não foi possível abrir o ETH Search Studio")
                        .set_description(format!("Não foi possível proteger o histórico de buscas contra uso simultâneo.\n\n{error}"))
                        .set_level(rfd::MessageLevel::Error)
                        .set_buttons(rfd::MessageButtons::Ok)
                        .show();
                    std::process::exit(1);
                }
            };
            let runs_dir = runs_dir.canonicalize()?;
            let history_dir = app_data.join("history");
            fs::create_dir_all(&history_dir)?;
            let history_dir = history_dir.canonicalize()?;
            let mut inner = Inner::default();
            for entry in fs::read_dir(&runs_dir)?.flatten() {
                let path = entry.path();
                if managed_run_path(&runs_dir, &path).is_err() {
                    continue;
                }
                if let Some(run) = recover_run(&path) {
                    inner.runs.push(run);
                }
            }
            inner
                .runs
                .sort_by(|left, right| right.started_at.cmp(&left.started_at));
            let engine = engine_path(app.handle());
            let queue_path = app_data.join("file-queue.json");
            inner.queue = file_queue::recover(&queue_path)?;
            app.manage(Studio {
                inner: Mutex::new(inner),
                preparation: Mutex::new(()),
                runs_dir,
                engine,
                history_dir,
                queue_path,
                _instance_lock: instance_lock,
            });
            mcp_bridge::start(app.handle(), &app_data)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            start_search,
            pause_search,
            resume_search,
            get_state,
            pick_records,
            open_run_folder
            ,file_queue::import_word_file
            ,file_queue::import_word_bytes
            ,file_queue::start_file_queue
            ,file_queue::pause_file_queue
            ,file_queue::resume_file_queue
            ,file_queue::cancel_file_queue
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if defer_close(window.app_handle()) {
                    api.prevent_close();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to initialize ETH Search Studio")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                mcp_bridge::cleanup(app);
            }
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if defer_close(app) {
                    api.prevent_exit();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> RunSnapshot {
        RunSnapshot {
            id: unique_id(),
            name: "Verificação do backend".into(),
            status: "running".into(),
            config: config(),
            started_at: now_string(),
            elapsed_seconds: 0.0,
            checked: "0".into(),
            excluded: "0".into(),
            total: None,
            rate: 0.0,
            backend: "auto".into(),
            logs: vec![],
            metrics: None,
            error: None,
            auto_history_records: 0,
            history_snapshot: false,
            history_export_eligible: None,
            queue_id: None,
        }
    }

    fn runtime(base_checked: u128) -> Runtime {
        Runtime {
            started: Instant::now(),
            base_elapsed: 0.0,
            base_checked,
            found: false,
            exhausted: false,
            limited: false,
            pause_reported: false,
            pause_requested: false,
        }
    }

    pub(super) fn config() -> SearchConfig {
        SearchConfig {
            name: "Demo".into(),
            target: format!("0x{}", "0".repeat(40)),
            mode: "template".into(),
            pattern: "? ? ? ? ? ? ? ? ? ? ? ?".into(),
            pool: "abandon ability".into(),
            fill: "a*".into(),
            post: String::new(),
            video: String::new(),
            backend: "auto".into(),
            language: "english".into(),
            max_candidates: "100".into(),
            batch_size: 1024,
            threads: 0,
            adaptive: true,
            no_checksum: false,
            exclude_ro1: false,
            exclude_records: vec![],
        }
    }

    #[test]
    fn expanded_search_disables_ro1_without_changing_the_search_space() {
        let mut strict = config();
        strict.exclude_ro1 = true;
        validate(&mut strict).unwrap();
        assert!(strict.exclude_ro1);
        let mut expanded = strict.clone();
        expanded.no_checksum = true;
        validate(&mut expanded).unwrap();
        assert!(!expanded.exclude_ro1);
        assert!(expanded.no_checksum);
        assert_eq!(expanded.pattern, strict.pattern);
        assert_eq!(expanded.pool, strict.pool);
        assert!(strict.exclude_ro1);
        assert!(!strict.no_checksum);
    }

    #[test]
    fn typed_arguments_preserve_values_without_a_shell() {
        let mut config = config();
        config.pool = "word with spaces & characters".into();
        validate(&mut config).unwrap();
        let args = command_args(
            &config,
            Path::new("managed run"),
            Path::new("metrics.json"),
            true,
        );
        let pos = args.iter().position(|arg| arg == "--pool").unwrap();
        assert_eq!(args[pos + 1], "word with spaces & characters");
        assert_eq!(args.iter().filter(|arg| *arg == "--resume").count(), 1);
        assert!(!args.iter().any(|arg| arg == "--post"));
        config.target = "--selftest".into();
        assert!(validate(&mut config).is_err());
    }

    #[test]
    fn checkpoint_counts_keep_u128_precision() {
        let value: Value = serde_json::from_str(
            "{\"checked\":340282366920938463463374607431768211455,\"excluded\":17}",
        )
        .unwrap();
        assert_eq!(json_count(&value, "checked"), Some(u128::MAX.to_string()));
        assert_eq!(json_count(&value, "excluded"), Some("17".into()));
    }

    #[test]
    fn automatic_history_import_only_accepts_confirmed_negative_states() {
        let mut run = snapshot();
        for status in ["completed", "limited", "paused"] {
            run.status = status.into();
            assert!(eligible_history_export(&run));
        }
        for status in ["running", "pausing", "found", "failed", "covered"] {
            run.status = status.into();
            assert!(!eligible_history_export(&run));
        }
        run.status = "paused".into();
        run.logs
            .push("Sessão anterior interrompida. Checkpoint disponível para retomada.".into());
        assert!(
            !eligible_history_export(&run),
            "older recovered sessions must not become approved negatives"
        );
        run.history_export_eligible = Some(true);
        assert!(
            eligible_history_export(&run),
            "a later confirmed negative completion can approve the prefix"
        );
        run.history_export_eligible = Some(false);
        run.logs.clear();
        assert!(!eligible_history_export(&run));
    }

    #[test]
    fn covered_preflight_preserves_prepared_snapshot_without_claiming_work() {
        let mut prepared = snapshot();
        prepared.history_snapshot = true;
        prepared.auto_history_records = 3;
        prepared.total = Some("181440".into());
        prepared.config.exclude_records = vec!["managed-run/selected-records/0.json".into()];
        let covered = preparation_failure(prepared.clone(), HISTORY_COVERED_MESSAGE);
        assert_eq!(covered.status, "covered");
        assert_eq!(
            (covered.checked.as_str(), covered.excluded.as_str()),
            ("0", "0")
        );
        assert!(covered.history_snapshot);
        assert_eq!(covered.auto_history_records, 3);
        assert_eq!(covered.total, prepared.total);
        assert_eq!(
            covered.config.exclude_records,
            prepared.config.exclude_records
        );
        assert!(covered
            .logs
            .iter()
            .any(|line| line.starts_with("Busca não iniciada:")));
        assert!(!eligible_history_export(&covered));
        let failure = preparation_failure(prepared, "Motor indisponível");
        assert_eq!(failure.status, "failed");
        assert_eq!(failure.error.as_deref(), Some("Motor indisponível"));
    }

    #[test]
    fn history_snapshots_preserve_auto_and_manual_records_and_legacy_arguments() {
        let base = std::env::temp_dir().join(unique_id());
        let global = base.join("global-history");
        let dir = base.join("run");
        fs::create_dir_all(&global).unwrap();
        fs::create_dir(&dir).unwrap();
        let auto = global.join("old-run.json");
        let manual = base.join("manual.json");
        fs::write(&auto, b"original automatic record").unwrap();
        fs::write(&manual, b"original explicit record").unwrap();
        let mut run = snapshot();
        run.config
            .exclude_records
            .push(manual.to_string_lossy().into_owned());
        let legacy = exclusion_args(&run, &dir);
        assert!(!legacy.iter().any(|arg| arg == "--exclude-record-dir"));
        snapshot_history(&global, &dir, &mut run).unwrap();
        assert!(run.history_snapshot);
        assert_eq!(run.auto_history_records, 1);
        fs::write(&auto, b"updated automatic record").unwrap();
        fs::write(&manual, b"updated explicit record").unwrap();
        assert_eq!(
            fs::read(dir.join("history/old-run.json")).unwrap(),
            b"original automatic record"
        );
        assert_eq!(
            fs::read(&run.config.exclude_records[0]).unwrap(),
            b"original explicit record"
        );
        let original_args = exclusion_args(&run, &dir);
        assert!(original_args
            .iter()
            .any(|arg| arg == "--exclude-record-dir"));
        assert!(!original_args
            .iter()
            .any(|arg| arg == "--additional-record-dir"));
        assert!(!original_args.iter().any(|arg| arg == "--resume"));
        assert!(!original_args.iter().any(|arg| arg == "--max-candidates"));
        assert!(
            snapshot_history(&global, &dir, &mut run).is_err(),
            "snapshots are never replaced"
        );
        fs::remove_file(dir.join("history/old-run.json")).unwrap();
        fs::remove_file(dir.join("selected-records/0.json")).unwrap();
        fs::remove_dir(dir.join("history")).unwrap();
        fs::remove_dir(dir.join("selected-records")).unwrap();
        fs::remove_file(auto).unwrap();
        fs::remove_file(manual).unwrap();
        fs::remove_dir(global).unwrap();
        fs::remove_dir(dir).unwrap();
        fs::remove_dir(base).unwrap();
    }

    #[test]
    fn ids_cannot_escape_run_directory() {
        let lock_path = std::env::temp_dir().join(format!("studio-test-{}.lock", unique_id()));
        let state = Studio {
            inner: Mutex::new(Inner::default()),
            preparation: Mutex::new(()),
            runs_dir: std::env::temp_dir(),
            engine: PathBuf::new(),
            history_dir: std::env::temp_dir().join("unused-studio-history"),
            queue_path: std::env::temp_dir().join("unused-studio-queue.json"),
            _instance_lock: acquire_instance_lock(&lock_path).unwrap(),
        };
        for id in ["", "../outside", "C:\\outside", "1/2", "run"] {
            assert!(state.run_dir(id).is_err());
        }
        assert_ne!(unique_id(), unique_id());
        drop(state);
        fs::remove_file(lock_path).unwrap();
    }

    #[test]
    fn managed_paths_accept_physical_children_and_reject_outside_or_nested_directories() {
        let base = std::env::temp_dir().join(unique_id());
        let root = base.join("Runs");
        let child = root.join(unique_id());
        let outside = base.join("outside");
        let nested = child.join(unique_id());
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir(&outside).unwrap();
        let canonical_root = root.canonicalize().unwrap();
        let canonical_child = child.canonicalize().unwrap();
        assert_eq!(managed_run_path(&root, &child).unwrap(), canonical_child);
        assert_eq!(
            managed_run_path(&canonical_root, &child).unwrap(),
            canonical_child
        );
        assert_eq!(
            managed_run_path(&root, &canonical_child).unwrap(),
            canonical_child
        );
        assert!(managed_run_path(&root, &outside).is_err());
        assert!(managed_run_path(&root, &nested).is_err());
        assert!(managed_run_path(&root, &root).is_err());
        let file = root.join("not-a-directory");
        fs::write(&file, b"data").unwrap();
        assert!(managed_run_path(&root, &file).is_err());
        fs::remove_file(file).unwrap();
        fs::remove_dir(nested).unwrap();
        fs::remove_dir(child).unwrap();
        fs::remove_dir(root).unwrap();
        fs::remove_dir(outside).unwrap();
        fs::remove_dir(base).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_managed_paths_accept_case_verbatim_and_separator_aliases() {
        let base = std::env::temp_dir().join(unique_id());
        let root = base.join("Runs");
        let child = root.join(unique_id());
        fs::create_dir_all(&child).unwrap();
        let canonical_child = child.canonicalize().unwrap();
        let mut uppercase = root.to_string_lossy().into_owned();
        uppercase.make_ascii_uppercase();
        let aliases = [
            root.clone(),
            root.canonicalize().unwrap(),
            PathBuf::from(uppercase),
            PathBuf::from(format!("{}\\\\Runs", base.display())),
        ];
        for alias in aliases {
            assert!(same_file::is_same_file(
                managed_run_path(&alias, &child).unwrap(),
                &canonical_child,
            )
            .unwrap());
            assert!(same_file::is_same_file(
                managed_run_path(&alias, &alias.join(child.file_name().unwrap())).unwrap(),
                &canonical_child,
            )
            .unwrap());
        }
        fs::remove_dir(child).unwrap();
        fs::remove_dir(root).unwrap();
        fs::remove_dir(base).unwrap();
    }

    #[test]
    fn instance_lock_excludes_other_handles_and_releases_on_drop() {
        let path = std::env::temp_dir().join(format!("studio-test-{}.lock", unique_id()));
        fs::write(&path, b"existing lock file").unwrap();
        let first = acquire_instance_lock(&path).unwrap();
        assert!(matches!(
            acquire_instance_lock(&path),
            Err(fs::TryLockError::WouldBlock)
        ));
        drop(first);
        let next = acquire_instance_lock(&path).unwrap();
        assert!(matches!(
            acquire_instance_lock(&path),
            Err(fs::TryLockError::WouldBlock)
        ));
        drop(next);
        assert_eq!(fs::read(&path).unwrap(), b"existing lock file");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn instance_lock_distinguishes_io_failure_from_an_open_window() {
        let missing = std::env::temp_dir().join(unique_id()).join("studio.lock");
        assert!(matches!(
            acquire_instance_lock(&missing),
            Err(fs::TryLockError::Error(_))
        ));
    }

    #[test]
    fn progress_parser_keeps_gpu_suffixes_and_resume_counts_separate() {
        let mut run = snapshot();
        run.checked = "1500".into();
        let mut current = runtime(1500);
        parse_line(
            &mut run,
            &mut current,
            "Using GPU (CUDA), batch=1024, block=64",
        );
        for (line, expected) in [
            ("Checked 2.5K candidates...", 4_000u128),
            ("Checked 1.2M candidates...", 1_201_500),
            ("Checked 3.4G candidates...", 3_400_001_500),
        ] {
            parse_line(&mut run, &mut current, line);
            assert_eq!(run.checked, expected.to_string());
            // Repeated output is a snapshot, not an increment.
            parse_line(&mut run, &mut current, line);
            assert_eq!(run.checked, expected.to_string());
        }
        parse_line(
            &mut run,
            &mut current,
            "Paused search; checkpoint saved: 3400001500 candidates",
        );
        assert!(current.pause_reported);
        let base = run.checked.parse::<u128>().unwrap();
        let mut resumed = runtime(base);
        parse_line(
            &mut run,
            &mut resumed,
            "Using GPU (CUDA), batch=1024, block=64",
        );
        parse_line(&mut run, &mut resumed, "Checked 2.0K candidates...");
        assert_eq!(run.checked, (base + 2000).to_string());
        parse_line(&mut run, &mut resumed, "Checked 1.0K candidates...");
        assert_eq!(
            run.checked,
            (base + 2000).to_string(),
            "older output must not move progress backwards"
        );
        assert_eq!(parse_display_count("1,234_567"), Some(1_234_567));
        assert_eq!(parse_display_count("1.5T"), Some(1_500_000_000_000));
        assert_eq!(parse_display_count("not-a-count"), None);
    }

    #[test]
    fn progress_parser_uses_cpu_cumulative_counts_and_exact_total_and_exclusions() {
        let mut run = snapshot();
        run.checked = "1500".into();
        let mut current = runtime(1500);
        parse_line(&mut run, &mut current, "Using CPU with 4 threads");
        parse_line(&mut run, &mut current, "Checked 2500 distinct candidates");
        assert_eq!(run.backend, "CPU");
        assert_eq!(
            run.checked, "2500",
            "CPU output already includes the resume base"
        );
        let exact = u128::MAX.to_string();
        parse_line(&mut run, &mut current, &format!("Searching 340282366920938463463374607.4T distinct candidates ({exact} exact; streamed)..."));
        assert_eq!(run.total, Some(exact.clone()));
        parse_line(&mut run, &mut current, "History excluded 25 arrangements this run (777 cumulative); limits and cursors count original candidates");
        assert_eq!(run.excluded, "777");
        parse_line(&mut run, &mut current, "Searching malformed count");
        parse_line(&mut run, &mut current, "unrelated output");
        assert_eq!(run.total, Some(exact));
        assert_eq!(run.checked, "2500");
        assert_eq!(run.excluded, "777");
        parse_line(
            &mut run,
            &mut current,
            "Candidate limit reached; search incomplete: 1000 candidates",
        );
        assert!(current.limited);
        assert!(!current.exhausted && !current.found);
    }

    #[test]
    fn existing_state_is_replaced_and_interrupted_run_recovers_exact_checkpoint() {
        let mut run = snapshot();
        let dir = std::env::temp_dir().join(&run.id);
        fs::create_dir(&dir).unwrap();
        persist(&dir, &run).unwrap();
        run.name = "Estado atualizado".into();
        run.status = "pausing".into();
        run.checked = "999999999999999999999999999999".into();
        run.total = Some(u128::MAX.to_string());
        // Preserve invocation-only metrics independently from cumulative counts.
        run.metrics = Some(serde_json::json!({"completed_raw": 13, "excluded": 7}));
        run.logs = (0..305).map(|n| format!("linha {n}")).collect();
        persist(&dir, &run).unwrap();
        let saved = read_json(&dir.join("state.json")).unwrap();
        assert_eq!(saved["name"], "Estado atualizado");
        assert_eq!(saved["status"], "pausing");
        fs::write(
            dir.join("checkpoint.json"),
            format!("{{\"checked\":{},\"excluded\":17}}", u128::MAX - 1),
        )
        .unwrap();
        let recovered = recover_run(&dir).unwrap();
        assert_eq!(recovered.status, "paused");
        assert_eq!(recovered.checked, (u128::MAX - 1).to_string());
        assert_eq!(recovered.excluded, "17");
        assert_eq!(recovered.total, Some(u128::MAX.to_string()));
        assert_eq!(recovered.metrics.as_ref().unwrap()["completed_raw"], 13);
        assert_eq!(recovered.logs.len(), LOG_LIMIT);
        assert_eq!(
            read_json(&dir.join("state.json")).unwrap()["status"],
            "paused"
        );
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            2,
            "successful writes must leave no temporary files"
        );
        // Recovery is idempotent: it does not append another interruption log.
        let again = recover_run(&dir).unwrap();
        assert_eq!(again.logs, recovered.logs);
        run.id = unique_id();
        persist(&dir, &run).unwrap();
        assert!(
            recover_run(&dir).is_none(),
            "mismatched directory and run ID must be rejected"
        );
        fs::remove_file(dir.join("checkpoint.json")).unwrap();
        fs::remove_file(dir.join("state.json")).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn recovery_without_checkpoint_cannot_claim_paused_or_complete() {
        let run = snapshot();
        let dir = std::env::temp_dir().join(&run.id);
        fs::create_dir(&dir).unwrap();
        persist(&dir, &run).unwrap();
        let recovered = recover_run(&dir).unwrap();
        assert_eq!(recovered.status, "failed");
        assert!(recovered.error.is_some());
        assert_eq!(recovered.checked, "0");
        fs::remove_file(dir.join("state.json")).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn language_and_batch_flags_match_engine_selectors() {
        for language in [
            "english",
            "portuguese",
            "spanish",
            "french",
            "italian",
            "czech",
            "korean",
            "japanese",
            "chinese-simplified",
            "chinese-traditional",
        ] {
            let mut config = config();
            config.language = language.into();
            config.mode = "batches".into();
            config.post = "dutch@1 cattle fiber fork forest wood".into();
            config.video = "fog@5 parrot@12 update winter lake also".into();
            config.backend = "cpu".into();
            config.no_checksum = true;
            validate(&mut config).unwrap();
            let args = command_args(&config, Path::new("run"), Path::new("metrics.json"), false);
            let position = args.iter().position(|arg| arg == "--language").unwrap();
            assert_eq!(args[position + 1], language);
            for flag in [
                "--post",
                "--video",
                "--cpu",
                "--no-checksum",
                "--adaptive-batch",
                "--checkpoint",
                "--stop-file",
                "--metrics-json",
            ] {
                assert!(args.iter().any(|arg| arg == flag));
            }
            for flag in ["--pattern", "--pool", "--fill", "--resume"] {
                assert!(!args.iter().any(|arg| arg == flag));
            }
        }
        let mut config = config();
        config.language = "unsupported".into();
        assert!(validate(&mut config).is_err());
    }
}
