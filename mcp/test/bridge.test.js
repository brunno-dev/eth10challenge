import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { mkdtemp, writeFile, unlink, rmdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { StudioBridge } from '../bridge.js';

const token = 'a'.repeat(64);
async function temporaryBridge(t, descriptor) {
  const dataDir = await mkdtemp(path.join(tmpdir(), 'eth-mcp-test-'));
  const filename = path.join(dataDir, 'mcp-connection.json');
  await writeFile(filename, JSON.stringify(descriptor));
  t.after(async () => { await unlink(filename); await rmdir(dataDir); });
  return new StudioBridge({ dataDir, autoLaunch: false });
}
async function endpoint(t, handler) {
  const server = http.createServer(async (req, res) => {
    let body = '';
    for await (const chunk of req) body += chunk;
    handler(req, res, JSON.parse(body));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(async () => { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); });
  return { version: 1, host: '127.0.0.1', port: server.address().port, token };
}

test('bridge confines discovery to loopback and validates credentials', async t => {
  for (const invalid of [
    { host: 'example.com' }, { host: 'localhost' }, { host: '127.0.0.1@evil.test' },
    { port: 0 }, { port: 65536 }, { port: '1234' }, { version: 2 }, { token: 'invalid' },
  ]) {
    const bridge = await temporaryBridge(t, { version: 1, host: '127.0.0.1', port: 1234, token, ...invalid });
    await assert.rejects(bridge.discovery(), /Conexão MCP incompatível/);
  }
});

test('bridge authenticates probe and command and preserves typed request parameters', async t => {
  const requests = [];
  const descriptor = await endpoint(t, (req, res, body) => {
    requests.push({ path: req.url, auth: req.headers.authorization, contentType: req.headers['content-type'], body });
    res.setHeader('content-type', 'application/json');
    res.end(JSON.stringify({ result: body.method === 'get_state' ? { active: null, runs: [] } : { id: '1-2-3', status: 'running' } }));
  });
  const bridge = await temporaryBridge(t, descriptor);
  assert.deepEqual(await bridge.call('start_search', { config: { name: 'test' } }), { id: '1-2-3', status: 'running' });
  assert.deepEqual(requests.map(request => request.body.method), ['get_state', 'start_search']);
  for (const request of requests) {
    assert.equal(request.path, '/rpc');
    assert.equal(request.auth, `Bearer ${token}`);
    assert.equal(request.contentType, 'application/json');
  }
  assert.deepEqual(requests[1].body.params, { config: { name: 'test' } });
});

test('accepted mutation with interrupted response is never retried', async t => {
  const commands = [];
  const descriptor = await endpoint(t, (req, res, body) => {
    commands.push(body.method);
    if (body.method === 'get_state') res.end(JSON.stringify({ result: { active: null, runs: [] } }));
    else req.socket.destroy();
  });
  const bridge = await temporaryBridge(t, descriptor);
  await assert.rejects(bridge.call('start_search', { config: { name: 'test' } }), /Consulte eth_get_state antes de repetir/);
  assert.deepEqual(commands, ['get_state', 'start_search']);
});

test('native error and HTTP failure are surfaced without repeating mutation', async t => {
  for (const failure of ['native', 'http']) {
    const commands = [];
    const descriptor = await endpoint(t, (req, res, body) => {
      commands.push(body.method);
      if (body.method === 'get_state') res.end(JSON.stringify({ result: { active: null, runs: [] } }));
      else if (failure === 'native') res.end(JSON.stringify({ error: 'Todas as combinações já foram testadas.' }));
      else { res.statusCode = 503; res.end('{}'); }
    });
    const bridge = await temporaryBridge(t, descriptor);
    await assert.rejects(bridge.call('resume_search', { id: '1-2-3' }), failure === 'native' ? /já foram testadas/ : /HTTP 503/);
    assert.deepEqual(commands, ['get_state', 'resume_search']);
  }
});

test('bridge refuses redirects so the bearer never reaches another HTTP endpoint', async t => {
  let destinationRequests = 0;
  const destination = await endpoint(t, (_req, res) => { destinationRequests++; res.end(JSON.stringify({ result: {} })); });
  const descriptor = await endpoint(t, (_req, res) => {
    res.writeHead(307, { location: `http://127.0.0.1:${destination.port}/rpc` });
    res.end();
  });
  const bridge = await temporaryBridge(t, descriptor);
  await assert.rejects(bridge.send(descriptor, 'get_state', {}));
  assert.equal(destinationRequests, 0);
});
