use super::*;
use file_import::{ImportPreview, ImportRow, FIXED_PATTERN};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueRow {
    pub line: usize,
    pub words: Vec<String>,
    pub pool: String,
    pub status: String,
    pub run_id: Option<String>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileQueue {
    pub version: u32,
    pub id: String,
    pub filename: String,
    pub status: String,
    pub config: SearchConfig,
    pub rows: Vec<QueueRow>,
    pub current_index: usize,
    pub error: Option<String>,
}

pub fn reserved(queue: &FileQueue) -> bool {
    !["completed", "cancelled", "found"].contains(&queue.status.as_str())
}
pub fn authorize_launch(inner: &Inner, run: &RunSnapshot) -> UiResult<()> {
    if let Some(queue) = inner.queue.as_ref().filter(|q| reserved(q)) {
        if run.queue_id.as_deref() != Some(&queue.id) {
            return Err("Existe uma fila de arquivo. Continue ou encerre a fila antes de iniciar outra busca.".into());
        }
        if queue.status != "running" {
            return Err("A fila está pausada; nenhuma nova linha foi iniciada.".into());
        }
    } else if run.queue_id.is_some() {
        return Err("Esta execução pertence a uma fila encerrada. Crie uma nova busca para usar suas palavras.".into());
    }
    Ok(())
}

fn save(path: &Path, queue: &FileQueue) -> UiResult<()> {
    let temporary = path.with_file_name(format!("file-queue-{}.tmp", unique_id()));
    let bytes = serde_json::to_vec(queue).map_err(|e| e.to_string())?;
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e| format!("Não foi possível salvar a fila: {e}"))
}

pub fn recover(path: &Path) -> UiResult<Option<FileQueue>> {
    if !path.exists() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("O arquivo da fila excede o limite permitido.".into());
    }
    let mut queue: FileQueue =
        serde_json::from_slice(&bytes).map_err(|e| format!("Fila salva inválida: {e}"))?;
    if queue.version != 1
        || queue.rows.len() > file_import::MAX_ROWS
        || queue.current_index > queue.rows.len()
    {
        return Err("Versão ou tamanho da fila inválido.".into());
    }
    if ["running", "pausing"].contains(&queue.status.as_str()) {
        queue.status = "paused".into();
        queue.error =
            Some("Fila recuperada. Use Continuar fila para retomar do ponto salvo.".into());
        save(path, &queue)?;
    }
    Ok(Some(queue))
}

fn queued_row(row: ImportRow) -> QueueRow {
    let status = if row.error.is_some() {
        "invalid"
    } else if row.duplicate_of.is_some() {
        "duplicate"
    } else {
        "pending"
    };
    let error = row.error.or_else(|| {
        row.duplicate_of
            .map(|n| format!("Mesmas palavras da linha {n}."))
    });
    QueueRow {
        line: row.line,
        words: row.words,
        pool: row.pool,
        status: status.into(),
        run_id: None,
        error,
    }
}

#[tauri::command]
pub async fn import_word_bytes(
    app: AppHandle,
    filename: String,
    bytes: Vec<u8>,
) -> UiResult<ImportPreview> {
    if filename.len() > 255 {
        return Err("Nome de arquivo muito longo.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let preview = file_import::parse_bytes(&bytes, &filename)?;
        app.state::<Studio>().lock()?.imported = Some(preview.clone());
        Ok(preview)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn import_word_file(
    app: AppHandle,
    path: Option<String>,
) -> UiResult<Option<ImportPreview>> {
    // The optional local path also supports automated desktop integration tests.
    // This command is not exposed by the authenticated MCP bridge.
    tauri::async_runtime::spawn_blocking(move || {
        let path = match path {
            Some(path) => PathBuf::from(path),
            None => match rfd::FileDialog::new()
                .add_filter("Listas de palavras", &["txt", "doc", "docx"])
                .pick_file()
            {
                Some(path) => path,
                None => return Ok(None),
            },
        };
        let preview = file_import::parse_file(&path)?;
        app.state::<Studio>().lock()?.imported = Some(preview.clone());
        Ok(Some(preview))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn start_file_queue(
    app: AppHandle,
    mut config: SearchConfig,
    import_id: String,
) -> UiResult<FileQueue> {
    config.mode = "template".into();
    config.pattern = FIXED_PATTERN.into();
    config.fill.clear();
    config.post.clear();
    config.video.clear();
    config.language = "english".into();
    validate(&mut config)?;
    let state = app.state::<Studio>();
    let mut inner = state.lock()?;
    if inner.active.is_some()
        || inner.queue_worker
        || inner.close_when_done
        || inner.queue.as_ref().is_some_and(reserved)
    {
        return Err("Conclua ou encerre a fila/busca atual antes de iniciar outra.".into());
    }
    let preview = inner
        .imported
        .as_ref()
        .filter(|p| p.import_id == import_id)
        .ok_or("Importe o arquivo novamente para preparar a fila.")?;
    if preview.valid_count == 0 {
        return Err("O arquivo não tem linhas válidas para executar.".into());
    }
    let queue = FileQueue {
        version: 1,
        id: unique_id(),
        filename: preview.filename.clone(),
        status: "running".into(),
        config,
        rows: preview.rows.clone().into_iter().map(queued_row).collect(),
        current_index: 0,
        error: None,
    };
    save(&state.queue_path, &queue)?;
    inner.queue = Some(queue.clone());
    inner.queue_worker = true;
    drop(inner);
    spawn_worker(app.clone());
    Ok(queue)
}

pub fn mark_paused(state: &Studio) -> UiResult<()> {
    let mut inner = state.lock()?;
    let active = inner.active.is_some();
    if let Some(queue) = inner
        .queue
        .as_mut()
        .filter(|q| ["running", "pausing"].contains(&q.status.as_str()))
    {
        queue.status = if active { "pausing" } else { "paused" }.into();
        save(&state.queue_path, queue)?;
    }
    Ok(())
}
#[tauri::command]
pub fn pause_file_queue(state: State<'_, Studio>) -> UiResult<FileQueue> {
    mark_paused(&state)?;
    if state.lock()?.active.is_some() {
        let _ = request_stop(&state);
    }
    state
        .lock()?
        .queue
        .clone()
        .ok_or("Nenhuma fila disponível.".into())
}
#[tauri::command]
pub fn cancel_file_queue(state: State<'_, Studio>) -> UiResult<FileQueue> {
    {
        let mut inner = state.lock()?;
        let queue = inner.queue.as_mut().ok_or("Nenhuma fila disponível.")?;
        queue.status = "cancelled".into();
        queue.error = None;
        save(&state.queue_path, queue)?;
    }
    if state.lock()?.active.is_some() {
        let _ = request_stop(&state);
    }
    state
        .lock()?
        .queue
        .clone()
        .ok_or("Nenhuma fila disponível.".into())
}
#[tauri::command]
pub fn resume_file_queue(app: AppHandle) -> UiResult<FileQueue> {
    let state = app.state::<Studio>();
    let mut inner = state.lock()?;
    if inner.active.is_some() || inner.queue_worker || inner.close_when_done {
        return Err("Aguarde a confirmação da pausa antes de continuar a fila.".into());
    }
    let queue = inner.queue.as_mut().ok_or("Nenhuma fila disponível.")?;
    if !["paused", "failed"].contains(&queue.status.as_str()) {
        return Err("Esta fila não pode ser retomada.".into());
    }
    queue.status = "running".into();
    queue.error = None;
    save(&state.queue_path, queue)?;
    let result = queue.clone();
    inner.queue_worker = true;
    drop(inner);
    spawn_worker(app.clone());
    Ok(result)
}

fn spawn_worker(app: AppHandle) {
    std::thread::spawn(move || {
        let result = work(&app);
        let state = app.state::<Studio>();
        if let Ok(mut inner) = state.lock() {
            inner.queue_worker = false;
            let current_run = inner
                .queue
                .as_ref()
                .and_then(|q| q.rows.get(q.current_index))
                .and_then(|r| r.run_id.as_ref())
                .and_then(|id| inner.runs.iter().find(|r| &r.id == id))
                .cloned();
            if let Some(queue) = inner.queue.as_mut() {
                if let Some(row) = queue.rows.get_mut(queue.current_index) {
                    if let Some(run) = current_run {
                        row.status = run.status;
                        row.error = run.error;
                    }
                }
                if let Err(error) = result {
                    if reserved(queue) {
                        queue.status = "failed".into();
                        queue.error = Some(error.clone());
                        if let Some(row) = queue.rows.get_mut(queue.current_index) {
                            row.status = "failed".into();
                            row.error = Some(error);
                        }
                    }
                }
                if queue.status == "pausing" {
                    queue.status = "paused".into();
                }
                let _ = save(&state.queue_path, queue);
            }
        };
    });
}

fn work(app: &AppHandle) -> UiResult<()> {
    loop {
        let state = app.state::<Studio>();
        // Reserve the row/run identity durably before launch. This lets recovery
        // reconcile an engine result even if the app closes between rows.
        let next = {
            let mut inner = state.lock()?;
            let active = inner.active.clone();
            if active.is_some() {
                drop(inner);
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            let known = inner.runs.clone();
            let queue = inner.queue.as_mut().ok_or("Fila indisponível.")?;
            if queue.status != "running" {
                return Ok(());
            }
            while queue.current_index < queue.rows.len()
                && ["completed", "covered", "invalid", "duplicate"]
                    .contains(&queue.rows[queue.current_index].status.as_str())
            {
                queue.current_index += 1;
            }
            if queue.current_index == queue.rows.len() {
                queue.status = "completed".into();
                save(&state.queue_path, queue)?;
                return Ok(());
            }
            let row = &mut queue.rows[queue.current_index];
            let next = if let Some(id) = &row.run_id {
                if let Some(run) = known.iter().find(|r| &r.id == id) {
                    match run.status.as_str() {
                        "completed" | "covered" => {
                            row.status = run.status.clone();
                            queue.current_index += 1;
                            save(&state.queue_path, queue)?;
                            continue;
                        }
                        "found" => {
                            row.status = "found".into();
                            queue.status = "found".into();
                            save(&state.queue_path, queue)?;
                            return Ok(());
                        }
                        "limited" | "paused" => {
                            row.status = "running".into();
                            (run.clone(), true)
                        }
                        "failed" => {
                            if checkpoint_counts(&state.run_dir(&run.id)?).is_some() {
                                row.status = "running".into();
                                (run.clone(), true)
                            } else if run.checked == "0" {
                                row.run_id = None;
                                row.status = "pending".into();
                                row.error = None;
                                save(&state.queue_path, queue)?;
                                continue;
                            } else {
                                return Err("A linha falhou sem checkpoint recuperável. Encerre a fila e importe novamente para reaproveitar o histórico confirmado.".into());
                            }
                        }
                        _ => return Err("Estado da linha atual não permite continuação.".into()),
                    }
                } else {
                    if row.status == "failed" && !state.run_dir(id)?.exists() {
                        row.run_id = None;
                        row.status = "pending".into();
                        row.error = None;
                        save(&state.queue_path, queue)?;
                        continue;
                    }
                    return Err("A execução da linha atual não foi recuperada. Encerre a fila e importe o arquivo novamente; o histórico confirmado será reaproveitado.".into());
                }
            } else {
                let mut config = queue.config.clone();
                config.name = format!(
                    "{} · linha {}",
                    queue.config.name.chars().take(70).collect::<String>(),
                    row.line
                );
                config.mode = "template".into();
                config.pattern = FIXED_PATTERN.into();
                config.pool = row.pool.clone();
                config.fill.clear();
                config.post.clear();
                config.video.clear();
                config.language = "english".into();
                let mut run = new_run(config);
                run.queue_id = Some(queue.id.clone());
                row.run_id = Some(run.id.clone());
                row.status = "running".into();
                (run, false)
            };
            save(&state.queue_path, queue)?;
            next
        };
        let next_id = next.0.id.clone();
        if let Err(error) = launch(app, next.0, next.1) {
            let mut inner = state.lock()?;
            if inner.queue.as_ref().is_some_and(|q| q.status != "running") {
                if !inner.runs.iter().any(|r| r.id == next_id)
                    && !inner.active.as_ref().is_some_and(|r| r.id == next_id)
                {
                    if let Some(queue) = inner.queue.as_mut() {
                        if let Some(row) = queue.rows.get_mut(queue.current_index) {
                            row.run_id = None;
                            row.status = "pending".into();
                        }
                        save(&state.queue_path, queue)?;
                    }
                }
                return Ok(());
            }
            if error != HISTORY_COVERED_MESSAGE {
                return Err(error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn queue() -> FileQueue {
        FileQueue {
            version: 1,
            id: unique_id(),
            filename: "test.txt".into(),
            status: "running".into(),
            config: super::super::tests::config(),
            rows: vec![],
            current_index: 0,
            error: None,
        }
    }
    #[test]
    fn queue_reservation_blocks_manual_runs_and_paused_owner() {
        let mut inner = Inner {
            queue: Some(queue()),
            ..Default::default()
        };
        let mut run = new_run(super::super::tests::config());
        assert!(authorize_launch(&inner, &run).is_err());
        run.queue_id = Some(inner.queue.as_ref().unwrap().id.clone());
        assert!(authorize_launch(&inner, &run).is_ok());
        inner.queue.as_mut().unwrap().status = "paused".into();
        assert!(authorize_launch(&inner, &run).is_err());
        inner.queue.as_mut().unwrap().status = "cancelled".into();
        run.queue_id = None;
        assert!(authorize_launch(&inner, &run).is_ok());
    }
    #[test]
    fn saved_running_queue_recovers_paused_without_autostart() {
        let path = std::env::temp_dir().join(format!("eth-queue-{}.json", unique_id()));
        let q = queue();
        save(&path, &q).unwrap();
        let recovered = recover(&path).unwrap().unwrap();
        assert_eq!(recovered.status, "paused");
        assert_eq!(recovered.id, q.id);
        fs::remove_file(path).unwrap();
    }
}
