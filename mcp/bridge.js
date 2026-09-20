import { readFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

const root = fileURLToPath(new URL('../', import.meta.url));
export class StudioBridge {
  constructor({ dataDir = process.env.ETH_STUDIO_DATA_DIR || path.join(process.env.LOCALAPPDATA || '', 'dev.ethsearch.studio'), appPath = process.env.ETH_STUDIO_EXE || path.join(root, 'desktop/release/ETH Search Studio.exe'), autoLaunch = process.env.ETH_MCP_AUTO_LAUNCH !== '0' } = {}) {
    this.discoveryPath = path.join(dataDir, 'mcp-connection.json');
    this.appPath = appPath;
    this.autoLaunch = autoLaunch;
    this.connecting = null;
  }
  async discovery() {
    const raw = await readFile(this.discoveryPath, 'utf8');
    if (raw.length > 4096) throw new Error('Arquivo de conexão inválido.');
    const d = JSON.parse(raw);
    if (d.version !== 1 || d.host !== '127.0.0.1' || !Number.isInteger(d.port) || d.port < 1 || d.port > 65535 || typeof d.token !== 'string' || !/^[a-fA-F0-9]{64}$/.test(d.token)) throw new Error('Conexão MCP incompatível. Abra a versão atualizada do ETH Search Studio.');
    return d;
  }
  async send(d, method, params, timeout = 120000) {
    const response = await fetch(`http://127.0.0.1:${d.port}/rpc`, {
      method: 'POST', redirect: 'error', signal: AbortSignal.timeout(timeout),
      headers: { authorization: `Bearer ${d.token}`, 'content-type': 'application/json' },
      body: JSON.stringify({ method, params }),
    });
    if (!response.ok) throw new Error(`A conexão local respondeu HTTP ${response.status}. Reabra o aplicativo atualizado.`);
    const data = await response.json();
    if (data.error) throw new Error(data.error);
    if (!Object.hasOwn(data, 'result')) throw new Error('Resposta inválida do aplicativo.');
    return data.result;
  }
  async connect() {
    try {
      const d = await this.discovery();
      await this.send(d, 'get_state', {}, 3000);
      return d;
    } catch { /* A missing or stale descriptor can be recovered by opening the app. */ }
    if (!this.autoLaunch) throw new Error('Abra o ETH Search Studio atualizado para conectar o MCP.');
    const child = spawn(this.appPath, ['--mcp-auto-launch'], { detached: true, stdio: 'ignore', windowsHide: true });
    let launchError;
    child.on('error', error => { launchError = error; });
    child.unref();
    for (let attempt = 0; attempt < 30; attempt++) {
      await sleep(400);
      if (launchError) throw new Error(`Não foi possível abrir o aplicativo: ${launchError.message}`);
      try {
        const d = await this.discovery();
        await this.send(d, 'get_state', {}, 2000);
        return d;
      } catch { /* Startup is asynchronous. */ }
    }
    throw new Error('O aplicativo não disponibilizou a conexão. Feche versões antigas e abra o ETH Search Studio atualizado.');
  }
  async call(method, params = {}) {
    if (!this.connecting) this.connecting = this.connect().finally(() => { this.connecting = null; });
    const descriptor = await this.connecting;
    try { return await this.send(descriptor, method, params); }
    catch (error) {
      if (error.name === 'TimeoutError' || error.name === 'TypeError') {
        throw new Error('A resposta do aplicativo foi interrompida. Consulte eth_get_state antes de repetir uma ação; ela pode ter sido aceita.');
      }
      throw error;
    }
  }
}
