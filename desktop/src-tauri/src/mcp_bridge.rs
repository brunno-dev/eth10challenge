//! Authenticated local transport shared by the stdio MCP adapter and the GUI.
//! Only the desktop owns the engine and its history; this is not a second runner.
use super::*;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const MAX_BODY: usize = 65_536;

struct BridgeState {
    discovery: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcRequest {
    method: String,
    #[serde(default = "empty_params")]
    params: Value,
}

fn empty_params() -> Value {
    serde_json::json!({})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigParams {
    config: SearchConfig,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResumeParams {
    id: String,
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> UiResult<T> {
    serde_json::from_value(value).map_err(|e| format!("Parâmetros inválidos: {e}"))
}

fn no_params(value: &Value) -> UiResult<()> {
    if value.as_object().is_some_and(|params| params.is_empty()) {
        Ok(())
    } else {
        Err("Este método não recebe parâmetros.".into())
    }
}

fn mcp_config(params: Value) -> UiResult<SearchConfig> {
    let mut config = decode::<ConfigParams>(params)?.config;
    // A connected model can use the managed automatic history and RO1. Manual
    // local file selection stays in the GUI instead of becoming a path API.
    if !config.exclude_records.is_empty() {
        return Err(
            "O MCP usa o histórico automático; selecione arquivos manuais pela interface.".into(),
        );
    }
    validate(&mut config)?;
    Ok(config)
}

fn preflight(app: &AppHandle, config: SearchConfig) -> UiResult<Value> {
    let state = app.state::<Studio>();
    let _preparation = state
        .preparation
        .lock()
        .map_err(|_| "A preparação da busca não está disponível.")?;
    if !state.engine.is_file() {
        return Err("Motor de busca indisponível.".into());
    }
    // Import only checkpoint prefixes already certified by a negative engine
    // outcome. No run entry, candidate derivation, or checkpoint is created.
    let imported = import_previous_runs(&state)?;
    let mut args = selector_args(&config);
    args.extend([
        "--exclude-record-dir".into(),
        state.history_dir.as_os_str().to_owned(),
        "--preflight-history".into(),
    ]);
    let output = engine_control(&state, &state.history_dir, args)?;
    let json = output
        .lines()
        .find_map(|line| line.strip_prefix("History preflight: "))
        .ok_or("O motor não retornou a verificação de histórico esperada.")?;
    let mut value: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if value.get("fullyCovered").and_then(Value::as_bool).is_none()
        || json_count(&value, "total").is_none()
    {
        return Err("Resposta de histórico inválida.".into());
    }
    value["importedRecords"] = imported.into();
    value["currentlyRunning"] = state.lock()?.active.is_some().into();
    Ok(value)
}

fn dispatch(app: &AppHandle, rpc: RpcRequest) -> UiResult<Value> {
    #[cfg(debug_assertions)]
    if rpc.method.starts_with("test_") {
        return dispatch_test(app, rpc);
    }
    match rpc.method.as_str() {
        "bootstrap" => {
            no_params(&rpc.params)?;
            serde_json::to_value(bootstrap(app.state::<Studio>())?).map_err(|e| e.to_string())
        }
        "get_state" => {
            no_params(&rpc.params)?;
            serde_json::to_value(refresh(&app.state::<Studio>())?).map_err(|e| e.to_string())
        }
        "start_search" => {
            let config = mcp_config(rpc.params)?;
            let run = tauri::async_runtime::block_on(start_search(app.clone(), config))?;
            serde_json::to_value(run).map_err(|e| e.to_string())
        }
        "preflight_search" => preflight(app, mcp_config(rpc.params)?),
        "resume_search" => {
            let params = decode::<ResumeParams>(rpc.params)?;
            let run = tauri::async_runtime::block_on(resume_search(app.clone(), params.id))?;
            serde_json::to_value(run).map_err(|e| e.to_string())
        }
        "pause_search" => {
            no_params(&rpc.params)?;
            serde_json::to_value(pause_search(app.state::<Studio>())?).map_err(|e| e.to_string())
        }
        _ => Err("Método de controle desconhecido.".into()),
    }
}

// These wrappers exist only in a debug build and an explicitly isolated test
// session. Production MCP keeps its existing control surface and no file API.
#[cfg(debug_assertions)]
pub(super) fn verify_test_directory(actual: &Path, normal: &Path) -> UiResult<()> {
    if std::env::var_os("ETH_STUDIO_DATA_DIR").is_none() || !actual.is_absolute() {
        return Err("O harness precisa de ETH_STUDIO_DATA_DIR absoluto e isolado.".into());
    }
    test_directories_disjoint(actual, normal)
}

#[cfg(any(debug_assertions, test))]
fn test_directories_disjoint(actual: &Path, normal: &Path) -> UiResult<()> {
    let actual = actual.canonicalize().map_err(|e| {
        format!("Crie a pasta descartável do harness antes de abrir o aplicativo: {e}")
    })?;
    // same_file resolves Windows casing, verbatim prefixes and junctions.
    // Reject both descendants and ancestors of the ordinary app data folder.
    for ancestor in normal.ancestors().filter(|path| path.exists()) {
        if same_file::is_same_file(&actual, ancestor).map_err(|e| e.to_string())? {
            return Err("A pasta do harness sobrepõe os dados reais do aplicativo.".into());
        }
    }
    if normal.exists() {
        for ancestor in actual.ancestors() {
            if same_file::is_same_file(ancestor, normal).map_err(|e| e.to_string())? {
                return Err("A pasta do harness sobrepõe os dados reais do aplicativo.".into());
            }
        }
    }
    Ok(())
}

#[cfg(debug_assertions)]
fn dispatch_test(app: &AppHandle, rpc: RpcRequest) -> UiResult<Value> {
    if std::env::var("ETH_STUDIO_TEST_MODE").as_deref() != Ok("resident-queue-v1") {
        return Err("O harness de integração não está habilitado.".into());
    }
    let state = app.state::<Studio>();
    let data = state
        .runs_dir
        .parent()
        .ok_or("Diretório de teste indisponível.")?;
    verify_test_directory(
        data,
        &app.path().app_local_data_dir().map_err(|e| e.to_string())?,
    )?;
    match rpc.method.as_str() {
        "test_state" => {
            no_params(&rpc.params)?;
            let queue_worker = state.lock()?.queue_worker;
            Ok(
                serde_json::json!({"dataDir":data, "queueWorker":queue_worker, "state":refresh(&state)?}),
            )
        }
        "test_import_word_bytes" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Import {
                filename: String,
                bytes: Vec<u8>,
            }
            let params = decode::<Import>(rpc.params)?;
            serde_json::to_value(tauri::async_runtime::block_on(
                file_queue::import_word_bytes(app.clone(), params.filename, params.bytes),
            )?)
            .map_err(|e| e.to_string())
        }
        "test_start_file_queue" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Start {
                config: SearchConfig,
                import_id: String,
            }
            let params = decode::<Start>(rpc.params)?;
            if !params.config.exclude_records.is_empty() {
                return Err("O harness não aceita arquivos de histórico externos.".into());
            }
            serde_json::to_value(tauri::async_runtime::block_on(
                file_queue::start_file_queue(app.clone(), params.config, params.import_id),
            )?)
            .map_err(|e| e.to_string())
        }
        "test_pause_file_queue" | "test_resume_file_queue" | "test_cancel_file_queue" => {
            no_params(&rpc.params)?;
            let queue = match rpc.method.as_str() {
                "test_pause_file_queue" => file_queue::pause_file_queue(state)?,
                "test_resume_file_queue" => file_queue::resume_file_queue(app.clone())?,
                _ => file_queue::cancel_file_queue(state)?,
            };
            serde_json::to_value(queue).map_err(|e| e.to_string())
        }
        "test_shutdown" => {
            no_params(&rpc.params)?;
            // Follow the window's close path: active work pauses, checkpoints,
            // and closes through finish(); idle shutdown releases the worker.
            if !defer_close(app) {
                let app = app.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    app.exit(0);
                });
            }
            Ok(serde_json::json!({"closing":true}))
        }
        _ => Err("Método de teste desconhecido.".into()),
    }
}

fn token_matches(candidate: &str, expected: &str) -> bool {
    // Tokens are fixed-size random ASCII. Do not short-circuit comparisons of
    // matching-length values, and never reflect the supplied token in errors.
    if candidate.len() != expected.len() {
        return false;
    }
    candidate
        .bytes()
        .zip(expected.bytes())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|header| header.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|header| header.value.as_str())
}

fn authorized_headers(
    host: Option<&str>,
    origin: Option<&str>,
    authorization: Option<&str>,
    port: u16,
    token: &str,
) -> bool {
    origin.is_none()
        && host == Some(format!("127.0.0.1:{port}").as_str())
        && authorization
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|candidate| token_matches(candidate, token))
}

fn respond(request: Request, status: u16, body: Value) {
    let response = Response::from_string(body.to_string())
        .with_status_code(StatusCode(status))
        .with_header(Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap())
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
    let _ = request.respond(response);
}

fn handle(app: &AppHandle, mut request: Request, port: u16, token: &str) {
    let loopback = request
        .remote_addr()
        .is_some_and(|remote| remote.ip().is_loopback());
    if !loopback
        || !authorized_headers(
            header(&request, "Host"),
            header(&request, "Origin"),
            header(&request, "Authorization"),
            port,
            token,
        )
    {
        respond(
            request,
            403,
            serde_json::json!({"error":"Conexão local não autorizada."}),
        );
        return;
    }
    if request.method() != &Method::Post || request.url() != "/rpc" {
        respond(
            request,
            404,
            serde_json::json!({"error":"Endpoint indisponível."}),
        );
        return;
    }
    if !header(&request, "Content-Type").is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
    }) {
        respond(
            request,
            415,
            serde_json::json!({"error":"Envie application/json."}),
        );
        return;
    }
    if !request
        .body_length()
        .is_some_and(|length| length <= MAX_BODY)
    {
        respond(
            request,
            413,
            serde_json::json!({"error":"Requisição ausente ou maior que 64 KiB."}),
        );
        return;
    }
    let mut bytes = Vec::new();
    if request
        .as_reader()
        .take((MAX_BODY + 1) as u64)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > MAX_BODY
    {
        respond(
            request,
            400,
            serde_json::json!({"error":"Não foi possível ler a requisição."}),
        );
        return;
    }
    let rpc = match serde_json::from_slice::<RpcRequest>(&bytes) {
        Ok(rpc) => rpc,
        Err(_) => {
            respond(
                request,
                400,
                serde_json::json!({"error":"Requisição JSON inválida."}),
            );
            return;
        }
    };
    match dispatch(app, rpc) {
        Ok(result) => respond(request, 200, serde_json::json!({"result":result})),
        Err(error) => respond(request, 200, serde_json::json!({"error":error})),
    }
}

pub(super) fn start(app: &AppHandle, app_data: &Path) -> UiResult<()> {
    let server = Server::http((IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .map_err(|e| format!("Não foi possível abrir o controle local MCP: {e}"))?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or("Endereço local MCP inválido.")?
        .port();
    let mut random = [0u8; 32];
    getrandom::fill(&mut random)
        .map_err(|e| format!("Não foi possível proteger o controle local MCP: {e}"))?;
    let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let discovery = app_data.join("mcp-connection.json");
    let temporary = app_data.join(format!("mcp-connection-{}.tmp", unique_id()));
    let value = serde_json::json!({"version":1,"transport":"http","host":"127.0.0.1","port":port,"token":token,"pid":std::process::id()});
    // App-local data is private to the OS user. The token is never exposed in
    // tool results or logs and rotates every desktop launch.
    fs::write(&temporary, value.to_string()).map_err(|e| e.to_string())?;
    fs::rename(&temporary, &discovery).map_err(|e| e.to_string())?;
    app.manage(BridgeState { discovery });
    // A bounded pool keeps status and pause available while another request
    // prepares history or initializes the engine. Launch/import still share the
    // preparation mutex, and the existing Studio reservation permits one run.
    let server = Arc::new(server);
    let token = Arc::new(token);
    for worker in 0..4 {
        let app = app.clone();
        let server = Arc::clone(&server);
        let token = Arc::clone(&token);
        std::thread::Builder::new()
            .name(format!("mcp-local-control-{worker}"))
            .spawn(move || {
                for request in server.incoming_requests() {
                    handle(&app, request, port, &token);
                }
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(super) fn cleanup(app: &AppHandle) {
    if let Some(state) = app.try_state::<BridgeState>() {
        let _ = fs::remove_file(&state.discovery);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_directory_must_be_physically_disjoint_from_real_data() {
        let base = std::env::temp_dir().join(format!("bridge-isolation-{}", unique_id()));
        let normal = base.join("normal");
        let nested = normal.join("nested");
        let isolated = base.join("isolated");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir(&isolated).unwrap();
        assert!(test_directories_disjoint(&normal, &normal).is_err());
        assert!(test_directories_disjoint(&nested, &normal).is_err());
        assert!(test_directories_disjoint(&base, &normal).is_err());
        assert!(test_directories_disjoint(&isolated, &normal).is_ok());
        #[cfg(windows)]
        assert!(test_directories_disjoint(&normal.canonicalize().unwrap(), &normal).is_err());
        fs::remove_dir(nested).unwrap();
        fs::remove_dir(normal).unwrap();
        fs::remove_dir(isolated).unwrap();
        fs::remove_dir(base).unwrap();
    }

    #[test]
    fn requires_exact_host_bearer_and_no_browser_origin() {
        let token = "a".repeat(64);
        let auth = format!("Bearer {token}");
        assert!(authorized_headers(
            Some("127.0.0.1:32123"),
            None,
            Some(&auth),
            32123,
            &token
        ));
        for host in [
            None,
            Some("localhost:32123"),
            Some("evil.example:32123"),
            Some("127.0.0.1:32124"),
        ] {
            assert!(!authorized_headers(host, None, Some(&auth), 32123, &token));
        }
        assert!(!authorized_headers(
            Some("127.0.0.1:32123"),
            Some("null"),
            Some(&auth),
            32123,
            &token
        ));
        assert!(!authorized_headers(
            Some("127.0.0.1:32123"),
            None,
            None,
            32123,
            &token
        ));
        assert!(!token_matches(&"b".repeat(64), &token));
        assert!(!token_matches("", &token));
    }

    #[test]
    fn rpc_envelope_and_parameters_reject_extra_fields() {
        assert!(
            serde_json::from_str::<RpcRequest>(r#"{"method":"get_state","command":"whoami"}"#)
                .is_err()
        );
        assert!(
            decode::<ResumeParams>(serde_json::json!({"id":"1-2-3","path":"C:/outside"})).is_err()
        );
        assert!(no_params(&serde_json::json!({"path":"C:/outside"})).is_err());
        let rpc: RpcRequest = serde_json::from_str(r#"{"method":"get_state"}"#).unwrap();
        assert!(no_params(&rpc.params).is_ok());
    }
}
