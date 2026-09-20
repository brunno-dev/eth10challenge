// Real desktop/engine integration, using the authenticated local MCP bridge.
// Requires a DEBUG Tauri executable; test RPCs do not exist in release builds.
// Usage: node scripts/test-resident-queue.mjs --backend cpu [--inject-failure]
//        node scripts/test-resident-queue.mjs --backend auto --inject-failure
// Build first: cargo build --manifest-path desktop/src-tauri/Cargo.toml --locked
// Set CARGO_TARGET_DIR to ../desktop-target and prepare the current CUDA sidecar.
// Faster with an existing release cache: add --release --features tauri/custom-protocol
// --config 'profile.release.package.eth-search-studio.debug-assertions=true', then
// copy that TEST executable plus engine/ into a disposable directory named debug.
// Pass --app <that directory>/eth-search-studio.exe. Never ship this test build.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, readdir, realpath, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as sleep } from 'node:timers/promises';
import { StudioBridge } from '../mcp/bridge.js';

const root = fileURLToPath(new URL('../', import.meta.url));
const options = new Map();
for (let i = 2; i < process.argv.length; i++) {
  const key = process.argv[i];
  assert(['--app', '--backend', '--output', '--inject-failure'].includes(key), `Unknown option ${key}`);
  options.set(key, key === '--inject-failure' ? true : process.argv[++i]);
}
const appPath = path.resolve(options.get('--app') || path.join(root, '../desktop-target/debug/eth-search-studio.exe'));
assert.equal(path.basename(path.dirname(appPath)).toLowerCase(), 'debug', 'Use the isolated debug build, never the packaged release executable.');
const backend = options.get('--backend') || 'cpu';
assert(['cpu', 'auto'].includes(backend), 'backend must be cpu or auto');
await realpath(appPath);
const outputRoot = path.resolve(options.get('--output') || path.join(root, 'output/import-check/resident-queue'));
await mkdir(outputRoot, { recursive: true });
const dataDir = await mkdtemp(path.join(outputRoot, `${backend}-`));
const ordinaryData = path.resolve(process.env.LOCALAPPDATA || '', 'dev.ethsearch.studio');
const relative = path.relative(ordinaryData.toLowerCase(), dataDir.toLowerCase());
assert(relative.startsWith('..') || path.isAbsolute(relative), 'Test data must be outside ordinary application data.');
assert.equal((await readdir(dataDir)).length, 0);

// This module derives the witness with @scure/bip39 + @scure/bip32 + Keccak,
// verifies the standard public vector, and writes only synthetic fixture files.
await import('../mcp/import-fixtures.js');
const fixture = JSON.parse(await readFile(path.join(root, 'output/import-check/fixture.json'), 'utf8'));
const zero = '0x0000000000000000000000000000000000000000';
const challenge = '0x9c2f44efad0c1e852a09df9939e6daf061140caf';
assert.notEqual(fixture.target, challenge);
const config = (name, target = zero, extra = {}) => ({
  name, target, mode: 'template', pattern: '', pool: '', fill: '', post: '', video: '',
  backend, language: 'english', maxCandidates: '1', batchSize: 256, threads: 1,
  adaptive: false, noChecksum: false, excludeRo1: false, excludeRecords: [], ...extra,
});
const bridge = new StudioBridge({ dataDir, appPath, autoLaunch: false });
const report = { version: 1, backend, appPath, dataDir, cases: [], processes: [] };
let child;
let childError;
const alive = pid => { try { process.kill(pid, 0); return true; } catch { return false; } };
const pids = run => [...new Set(run.logs.flatMap(line => {
  const matched = /^Motor residente: processo (\d+)\.$/.exec(line);
  return matched ? [Number(matched[1])] : [];
}))];

async function waitFor(description, predicate, timeout = 60000) {
  const deadline = Date.now() + timeout;
  let state;
  while (Date.now() < deadline) {
    if (childError) throw childError;
    state = await bridge.call('test_state');
    if (predicate(state)) return state;
    await sleep(20);
  }
  throw new Error(`Timed out: ${description}; queue=${state?.state.queue?.status}; active=${state?.state.active?.status}`);
}
async function boot() {
  childError = undefined;
  child = spawn(appPath, ['--mcp-auto-launch'], {
    cwd: root, windowsHide: true, stdio: 'ignore',
    env: { ...process.env, ETH_STUDIO_DATA_DIR: dataDir, ETH_STUDIO_TEST_MODE: 'resident-queue-v1' },
  });
  child.on('error', error => { childError = error; });
  for (let attempt = 0; attempt < 150; attempt++) {
    if (childError) throw childError;
    if (child.exitCode !== null) throw new Error(`Debug app exited ${child.exitCode} during startup.`);
    try {
      const info = await bridge.call('test_state');
      const [actual, expected] = await Promise.all([stat(info.dataDir, { bigint: true }), stat(dataDir, { bigint: true })]);
      assert.equal(actual.dev, expected.dev);
      assert.equal(actual.ino, expected.ino, 'Debug app did not select the isolated test directory.');
      return info;
    } catch (error) {
      if (attempt === 149) throw error;
      await sleep(100);
    }
  }
}
async function startQueue(name, text, target = zero, extra = {}) {
  const before = await bridge.call('test_state');
  assert(!before.queueWorker && !before.state.active, 'Previous queue must settle first.');
  const preview = await bridge.call('test_import_word_bytes', { filename: `${name}.txt`, bytes: [...Buffer.from(text)] });
  assert(preview.validCount > 0);
  const began = performance.now();
  const queue = await bridge.call('test_start_file_queue', { config: config(name, target, extra), importId: preview.importId });
  return { queue, began, preview };
}
async function settled(id, expected) {
  return (await waitFor(`queue ${expected}`, view => view.state.queue?.id === id
    && view.state.queue.status === expected && !view.queueWorker && !view.state.active)).state;
}
function queueRuns(state) {
  return state.queue.rows.filter(row => row.runId).map(row => {
    const run = state.runs.find(run => run.id === row.runId);
    assert(run, `Run ${row.runId} was not persisted.`);
    assert.equal(run.config.pattern, 'dutch ? ? ? fog ? ? ? ? ? ? parrot');
    assert.equal(run.config.excludeRo1, false);
    assert.notEqual(run.config.target.toLowerCase(), challenge);
    return run;
  });
}
async function checkpoint(run) {
  const value = JSON.parse(await readFile(path.join(dataDir, 'runs', run.id, 'checkpoint.json'), 'utf8'));
  assert.equal(String(value.checked), run.checked);
  return value;
}
async function assertProcessesClosed(runList) {
  const ids = [...new Set(runList.flatMap(pids))];
  assert(ids.length > 0, 'No resident worker PID evidence.');
  for (const pid of ids) assert(!alive(pid), `Resident worker ${pid} outlived its queue.`);
  report.processes.push(...ids);
  return ids;
}
async function stopApp() {
  if (!child || child.exitCode !== null) return;
  await bridge.call('test_shutdown');
  const deadline = Date.now() + 20000;
  while (child.exitCode === null && Date.now() < deadline) await sleep(20);
  assert.notEqual(child.exitCode, null, 'App did not close after its final checkpoint.');
}

try {
  const initial = await boot();
  assert.equal(initial.state.runs.length, 0);
  assert.equal(initial.state.queue, null);
  const bootstrap = await bridge.call('bootstrap');
  report.enginePath = bootstrap.enginePath;
  report.engineSha256 = createHash('sha256').update(await readFile(bootstrap.enginePath)).digest('hex');

  const negative = await startQueue('negative-reuse', fixture.negative);
  let state = await settled(negative.queue.id, 'completed');
  assert.deepEqual(state.queue.rows.map(row => row.status), ['completed', 'duplicate', 'completed', 'invalid', 'invalid']);
  const negativeRuns = queueRuns(state);
  for (const run of negativeRuns) {
    assert.equal(run.checked, run.total);
    assert.equal(run.historyExportEligible, true);
    assert.equal(run.historySnapshot, true);
    assert(run.metrics && typeof run.metrics === 'object', 'Run metrics were not persisted.');
    await readdir(path.join(dataDir, 'runs', run.id, 'history'));
    await checkpoint(run);
  }
  const workerPids = await assertProcessesClosed(negativeRuns);
  assert.equal(workerPids.length, 1, 'Rows/resumptions must reuse exactly one engine process.');
  if (backend === 'auto') {
    assert(negativeRuns.every(run => run.backend === 'CUDA'), 'Requested CUDA test fell back to CPU.');
    assert(negativeRuns.some(run => run.logs.some(line => line === 'Reusing initialized CUDA context')), 'No GPU context reuse evidence.');
  }
  report.cases.push({ name: 'negative-reuse', milliseconds: performance.now() - negative.began, pid: workerPids[0], checked: negativeRuns.map(run => run.checked) });

  const repeated = await startQueue('already-covered', fixture.negative);
  state = await settled(repeated.queue.id, 'completed');
  const covered = queueRuns(state);
  assert(covered.every(run => run.status === 'covered' && run.checked === '0' && pids(run).length === 0));
  report.cases.push({ name: 'already-covered', engineStarted: false });

  const found = await startQueue('witness-found', fixture.success, fixture.target, { maxCandidates: '32' });
  state = await settled(found.queue.id, 'found');
  const witness = queueRuns(state);
  assert.equal(witness.length, 1, 'A hit must stop before the next row starts.');
  assert.equal(witness[0].status, 'found');
  assert.equal(witness[0].historyExportEligible, false, 'A hit is not a negative history certificate.');
  assert(witness[0].logs.some(line => line.startsWith('Found matching mnemonic:') && line.includes(fixture.phrase)));
  assert.equal(state.queue.rows[1].status, 'pending');
  await assertProcessesClosed(witness);
  report.cases.push({ name: 'witness-found', target: fixture.target, milliseconds: performance.now() - found.began });

  const long = await startQueue('pause-resume-cancel', fixture.long, zero, { maxCandidates: '512', noChecksum: true });
  let live = await waitFor('active engine for pause', view => view.state.active?.logs.some(line => line.startsWith('Using ')));
  const runId = live.state.active.id;
  const firstPid = pids(live.state.active).at(-1);
  assert(firstPid);
  await bridge.call('test_pause_file_queue');
  state = await settled(long.queue.id, 'paused');
  let run = queueRuns(state)[0];
  assert.equal(run.id, runId);
  assert(['paused', 'limited'].includes(run.status), `Unexpected pause outcome: ${run.status}`);
  const prefix = BigInt(run.checked);
  await checkpoint(run);
  assert(!alive(firstPid), 'Pause retained the worker process.');

  await bridge.call('test_resume_file_queue');
  live = await waitFor('new worker on explicit resume', view => view.state.active?.id === runId && pids(view.state.active).some(pid => pid !== firstPid));
  const resumedPid = pids(live.state.active).at(-1);
  assert.notEqual(resumedPid, firstPid);
  await bridge.call('test_cancel_file_queue');
  state = await settled(long.queue.id, 'cancelled');
  run = queueRuns(state)[0];
  assert.equal(run.id, runId);
  assert(BigInt(run.checked) >= prefix, 'Resume lost a confirmed prefix.');
  await checkpoint(run);
  await assertProcessesClosed([run]);
  report.cases.push({ name: 'pause-resume-cancel', runId, firstPid, resumedPid, prefix: prefix.toString(), finalChecked: run.checked });

  if (options.has('--inject-failure')) {
    const fault = await startQueue('worker-failure', fixture.long.replace('lake', 'zoo'), zero, { maxCandidates: '512', noChecksum: true });
    live = await waitFor('confirmed prefix before worker failure', view => view.state.active && BigInt(view.state.active.checked) >= 512n && pids(view.state.active).length > 0);
    const failedId = live.state.active.id;
    const failedPid = pids(live.state.active).at(-1);
    // This PID comes from the private test app's active worker, never a process
    // search or a production session. The test deliberately kills this child.
    process.kill(failedPid);
    state = await settled(fault.queue.id, 'failed');
    const failed = queueRuns(state)[0];
    assert.equal(failed.id, failedId);
    assert.equal(failed.status, 'failed');
    assert.equal(failed.historyExportEligible, false);
    await checkpoint(failed);
    await sleep(300);
    const unchanged = await bridge.call('test_state');
    assert(!unchanged.state.active && !unchanged.queueWorker && unchanged.state.queue.status === 'failed', 'Failed work was silently retried.');
    await bridge.call('test_resume_file_queue');
    await waitFor('explicit recovery after failure', view => view.state.active?.id === failedId && pids(view.state.active).some(pid => pid !== failedPid));
    await bridge.call('test_cancel_file_queue');
    state = await settled(fault.queue.id, 'cancelled');
    const recovered = queueRuns(state)[0];
    assert.equal(recovered.id, failedId);
    assert(BigInt(recovered.checked) >= BigInt(failed.checked));
    await checkpoint(recovered);
    await assertProcessesClosed([recovered]);
    report.cases.push({ name: 'worker-failure', failedPid, checkedAtFailure: failed.checked, recoveredChecked: recovered.checked, automaticRetry: false });
  }

  const closing = await startQueue('close-active', fixture.long.replace('lake', 'zone'), zero, { maxCandidates: '512', noChecksum: true });
  live = await waitFor('active worker before closing', view => view.state.active?.logs.some(line => line.startsWith('Using ')));
  const closeId = live.state.active.id;
  const closePid = pids(live.state.active).at(-1);
  assert(closePid);
  await stopApp();
  assert(!alive(closePid), 'Closing the app left a worker process behind.');
  const persisted = JSON.parse(await readFile(path.join(dataDir, 'runs', closeId, 'state.json'), 'utf8'));
  // A small GPU batch may finish at its limit while the close request is in
  // transit. Both outcomes preserve a resumable prefix; the queue must recover
  // paused below, and neither outcome may restart work automatically.
  assert(['paused', 'limited'].includes(persisted.status), `Unexpected close outcome: ${persisted.status}`);
  await checkpoint(persisted);
  await boot();
  const recovered = await bridge.call('test_state');
  assert.equal(recovered.state.queue.id, closing.queue.id);
  assert.equal(recovered.state.queue.status, 'paused');
  assert(!recovered.queueWorker && !recovered.state.active, 'Recovery automatically restarted work.');
  assert.equal(recovered.state.runs.find(run => run.id === closeId).checked, persisted.checked);
  report.cases.push({ name: 'close-recover', closePid, checked: persisted.checked, runStatus: persisted.status, automaticRestart: false });

  for (const name of await readdir(path.join(dataDir, 'history'))) {
    if (!name.endsWith('.json')) continue;
    const record = JSON.parse(await readFile(path.join(dataDir, 'history', name), 'utf8')).record;
    const target = `0x${Buffer.from(record.target).toString('hex')}`;
    assert.notEqual(target, challenge);
    assert.equal(target, zero, 'Successful witness must not become negative history.');
  }
  report.success = true;
} catch (error) {
  report.success = false;
  report.error = error.stack || String(error);
  process.exitCode = 1;
} finally {
  try { await stopApp(); } catch (error) {
    report.shutdownError = String(error);
    report.success = false;
    process.exitCode = 1;
    // Only the exact debug test child is terminated if graceful shutdown broke.
    if (child?.exitCode === null) child.kill();
  }
  await writeFile(path.join(dataDir, 'integration-report.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
}
