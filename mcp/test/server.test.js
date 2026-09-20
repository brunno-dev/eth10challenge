import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import { createServer, toConfig, CHALLENGE_TARGET } from '../server.js';

const hypothesis = { name: 'Hipótese de teste', post: 'dutch@1 fiber fork dinner cloud live', video: 'fog@5 fiber wood winter rib parrot@12' };
function decoded(reply) {
  assert.notEqual(reply.isError, true, JSON.stringify(reply));
  const value = JSON.parse(reply.content[0].text);
  assert.deepEqual(reply.structuredContent, value);
  return value;
}
async function session(t, bridge = { call() { throw new Error('Unexpected desktop access'); } }) {
  const server = createServer(bridge);
  const client = new Client({ name: 'adapter-tests', version: '1.0.0' });
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
  t.after(async () => { await client.close(); await server.close(); });
  await server.connect(serverTransport);
  await client.connect(clientTransport);
  return client;
}

test('real SDK discovery exposes tools, evidence resources, workflow, and only implemented external filters', async t => {
  const client = await session(t);
  assert.equal(client.getServerVersion().name, 'eth-search');
  assert.match(client.getInstructions(), /somente RO1/);
  const { tools } = await client.listTools();
  assert.deepEqual(tools.map(tool => tool.name).sort(), ['eth_get_challenge', 'eth_get_run', 'eth_get_state', 'eth_pause_search', 'eth_preflight', 'eth_resume_search', 'eth_start_search', 'eth_wait_run'].sort());
  assert.equal(tools.find(tool => tool.name === 'eth_start_search').annotations.readOnlyHint, false);
  assert.equal(tools.find(tool => tool.name === 'eth_get_state').annotations.readOnlyHint, true);
  for (const tool of tools) {
    assert.equal(tool.inputSchema.type, 'object', tool.name);
    assert.equal(typeof tool.inputSchema.properties, 'object', tool.name);
    if (!['eth_get_challenge', 'eth_pause_search'].includes(tool.name)) {
      assert.equal(tool.inputSchema.additionalProperties, false, tool.name);
    }
  }
  const challenge = decoded(await client.callTool({ name: 'eth_get_challenge', arguments: {} }));
  assert.equal(challenge.target, CHALLENGE_TARGET);
  assert.deepEqual(challenge.anchorsUsed, { dutch: 1, fog: 5, parrot: 12 });
  assert.deepEqual(challenge.externalHistory.filter(entry => entry.filterImplemented).map(entry => entry.id), ['RO1']);
  assert.equal(challenge.videoCandidates.filterImplemented, false);
  assert.equal(challenge.videoCandidates.status, 'candidate_words_only');
  assert.deepEqual(challenge.videoCandidates.groups.map(g=>g.count), [5,109,110,192]);
  assert.equal(challenge.checksumPolicies.default, 'valid_only');
  const { resources } = await client.listResources();
  assert.equal(resources.length, 4);
  for (const resource of resources) {
    const reply = await client.readResource({ uri: resource.uri });
    assert.equal(reply.contents[0].uri, resource.uri);
    assert.ok(reply.contents[0].text.length > 100);
  }
  const catalog = await client.readResource({ uri: 'eth://history/catalog' });
  assert.equal(JSON.parse(catalog.contents[0].text).target, CHALLENGE_TARGET);
  await assert.rejects(client.readResource({ uri: 'file:///C:/Windows/win.ini' }));
  assert.equal((await client.listPrompts()).prompts[0].name, 'explorar-desafio');
  const prompt = await client.getPrompt({ name: 'explorar-desafio', arguments: { objective: 'Variar posições', rounds: '2' } });
  assert.match(prompt.messages[0].content.text, /no máximo 2 rodadas de até 100000/);
  assert.match(prompt.messages[0].content.text, /Não continue rodadas adicionais/);
});

test('configuration defaults preserve the challenge and history compatibility', () => {
  const config = toConfig(hypothesis);
  assert.equal(config.target, CHALLENGE_TARGET);
  assert.equal(config.maxCandidates, '100000');
  assert.equal(config.excludeRo1, true);
  assert.deepEqual(config.excludeRecords, []);
  assert.equal(toConfig({ ...hypothesis, target: '0x' + '0'.repeat(40) }).excludeRo1, false);
  assert.equal(toConfig({ ...hypothesis, language: 'portuguese' }).excludeRo1, false);
  assert.equal(toConfig({ ...hypothesis, noChecksum: true }).excludeRo1, false);
  assert.equal(toConfig({ ...hypothesis, noChecksum: true, excludeRo1: true }).excludeRo1, false);
  assert.equal(toConfig({ ...hypothesis, excludeRo1: false }).excludeRo1, false);
});

test('zero-argument tools accept omitted arguments but reject unknown fields', async t => {
  const calls = [];
  const client = await session(t, { async call(method) {
    calls.push(method);
    return { id: '1-2-3', status: 'pausing', logs: [] };
  } });
  assert.equal(decoded(await client.callTool({ name: 'eth_get_challenge' })).target, CHALLENGE_TARGET);
  assert.equal(decoded(await client.callTool({ name: 'eth_pause_search' })).status, 'pausing');
  assert.deepEqual(calls, ['pause_search']);
  for (const name of ['eth_get_challenge', 'eth_pause_search']) {
    assert.equal((await client.callTool({ name, arguments: { unexpected: true } })).isError, true);
  }
  assert.deepEqual(calls, ['pause_search']);
});

test('invalid search fields and unbounded budgets never reach the desktop', async t => {
  let calls = 0;
  const client = await session(t, { async call() { calls++; return {}; } });
  for (const change of [
    { maxCandidates: 0 }, { maxCandidates: -1 }, { maxCandidates: 1000000001 }, { maxCandidates: 1.5 },
    { maxCandidates: '100000' }, { threads: 1025 }, { batchSize: 0 },
    { target: '0x0; calc.exe' }, { backend: 'cuda --record-search x' },
    { checkpoint: '../../outside.json' }, { excludeRecords: ['C:/outside.json'] },
    { historyEnabled: false }, { command: 'powershell' },
  ]) {
    const reply = await client.callTool({ name: 'eth_start_search', arguments: { search: { ...hypothesis, ...change } } });
    assert.equal(reply.isError, true, JSON.stringify(change));
  }
  for (const args of [{ id: '../outside' }, { id: '123', seconds: 31 }, { id: '123', seconds: 0 }]) {
    assert.equal((await client.callTool({ name: 'eth_wait_run', arguments: args })).isError, true);
  }
  assert.equal((await client.callTool({ name: 'eth_start_search', arguments: { search: hypothesis, command: 'powershell' } })).isError, true);
  assert.equal((await client.callTool({ name: 'eth_get_state', arguments: { unknown: true } })).isError, true);
  assert.equal(calls, 0);
});

test('start, pagination, details, pause, terminal wait and resume preserve native command semantics', async t => {
  const calls = [];
  const config = toConfig(hypothesis);
  const run = { id: '123-45-6', status: 'running', checked: '0', config, metrics: { speed: 42 }, logs: Array.from({ length: 30 }, (_, index) => `log-${index}`) };
  const older = { ...run, id: '100-1-0', status: 'completed' };
  const bridge = { async call(method, params) {
    calls.push({ method, params });
    if (method === 'get_state') return { active: run.status === 'running' ? run : null, runs: [run, older] };
    if (method === 'preflight_search') return { fullyCovered: false, total: '181440', records: 3 };
    if (method === 'start_search') return run;
    if (method === 'pause_search') { run.status = 'paused'; return run; }
    if (method === 'resume_search') { run.status = 'running'; return run; }
    throw new Error(`Unexpected command ${method}`);
  } };
  const client = await session(t, bridge);
  assert.equal(decoded(await client.callTool({ name: 'eth_preflight', arguments: { search: hypothesis } })).fullyCovered, false);
  const started = decoded(await client.callTool({ name: 'eth_start_search', arguments: { search: hypothesis } }));
  assert.equal(started.status, 'running');
  assert.equal(started.logs.length, 5);
  assert.equal(started.config, undefined);
  assert.deepEqual(calls[1], { method: 'start_search', params: { config } });
  const state = decoded(await client.callTool({ name: 'eth_get_state', arguments: { offset: 0, limit: 1 } }));
  assert.equal(state.totalRuns, 2);
  assert.equal(state.runs.length, 1);
  assert.equal(state.nextOffset, 1);
  assert.equal(state.runs[0].logs, undefined);
  const details = decoded(await client.callTool({ name: 'eth_get_run', arguments: { id: run.id, logLines: 2 } }));
  assert.deepEqual(details.config, config);
  assert.deepEqual(details.metrics, { speed: 42 });
  assert.deepEqual(details.logs, ['log-28', 'log-29']);
  assert.equal(decoded(await client.callTool({ name: 'eth_pause_search', arguments: {} })).status, 'paused');
  const waited = decoded(await client.callTool({ name: 'eth_wait_run', arguments: { id: run.id, seconds: 1 } }));
  assert.equal(waited.terminal, true);
  assert.equal(waited.timedOut, false);
  assert.equal(decoded(await client.callTool({ name: 'eth_resume_search', arguments: { id: run.id } })).status, 'running');
  assert.deepEqual(calls.at(-1), { method: 'resume_search', params: { id: run.id } });
  assert.equal((await client.callTool({ name: 'eth_get_run', arguments: { id: '999' } })).isError, true);
});

test('covered searches return an actionable tool error without retrying start', async t => {
  let calls = 0;
  const client = await session(t, { async call() { calls++; throw new Error('Todas as combinações desta busca já foram testadas.'); } });
  const reply = await client.callTool({ name: 'eth_start_search', arguments: { search: hypothesis } });
  assert.equal(reply.isError, true);
  assert.match(reply.content[0].text, /já foram testadas/);
  assert.equal(calls, 1);
});

test('wait distinguishes an asynchronous terminal transition from an active timeout', async t => {
  let polls = 0;
  let finishOnPoll = true;
  const client = await session(t, { async call(method) {
    assert.equal(method, 'get_state');
    polls++;
    const status = finishOnPoll && polls > 1 ? 'completed' : 'running';
    const run = { id: '1-2-3', status, checked: status === 'completed' ? '10' : '5' };
    return { active: status === 'running' ? run : null, runs: [run] };
  } });
  const completed = decoded(await client.callTool({ name: 'eth_wait_run', arguments: { id: '1-2-3', seconds: 2 } }));
  assert.equal(completed.terminal, true);
  assert.equal(completed.timedOut, false);
  assert.equal(completed.run.checked, '10');
  assert.equal(polls, 2);
  finishOnPoll = false;
  const timeout = decoded(await client.callTool({ name: 'eth_wait_run', arguments: { id: '1-2-3', seconds: 1 } }));
  assert.equal(timeout.terminal, false);
  assert.equal(timeout.timedOut, true);
  assert.equal(timeout.run.status, 'running');
});

test('real stdio child completes MCP handshake and reads archive without launching the desktop', { timeout: 15000 }, async t => {
  const transport = new StdioClientTransport({ command: process.execPath, args: [fileURLToPath(new URL('../server.js', import.meta.url))], env: { ...process.env, ETH_MCP_AUTO_LAUNCH: '0', ETH_STUDIO_DATA_DIR: fileURLToPath(new URL('./nonexistent-data-directory', import.meta.url)) }, stderr: 'pipe' });
  const client = new Client({ name: 'stdio-tests', version: '1.0.0' });
  let stderr = '';
  transport.stderr.on('data', chunk => { stderr += chunk; });
  t.after(async () => { await client.close(); await transport.close(); });
  await client.connect(transport);
  assert.ok(transport.pid > 0);
  assert.equal((await client.listTools()).tools.length, 8);
  assert.equal(decoded(await client.callTool({ name: 'eth_get_challenge', arguments: {} })).target, CHALLENGE_TARGET);
  const catalog = await client.readResource({ uri: 'eth://history/catalog' });
  assert.equal(JSON.parse(catalog.contents[0].text).target, CHALLENGE_TARGET);
  const unavailable = await client.callTool({ name: 'eth_get_state', arguments: {} });
  assert.equal(unavailable.isError, true);
  assert.match(unavailable.content[0].text, /Abra o ETH Search Studio/);
  assert.equal(stderr, '');
});
