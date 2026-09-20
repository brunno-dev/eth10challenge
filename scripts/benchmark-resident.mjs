#!/usr/bin/env node
// Compare process startup with the resident JSONL engine using one binary.
// All evidence stays below a new output directory and targets the zero address.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, readFile, realpath, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { performance } from 'node:perf_hooks';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const target = '0x0000000000000000000000000000000000000000';
const pattern = 'dutch ? ? ? fog ? ? ? ? ? ? parrot';
const pools = [
  'abandon ability able about above absent absorb abstract absurd',
  'abuse access accident account accuse achieve acid acoustic acquire',
  'adapt add addict address adjust admit adult advance advice',
];
const totalPerRow = 362880;
const limit = 100000;
const rounds = Math.ceil(totalPerRow / limit);
const maxLineBytes = 1024 * 1024;
const maxOutputBytes = 16 * 1024 * 1024;
const hitPattern = /MATCH FOUND|FOUND MATCH|Found matching|Match found/i;
const secondsSince = (start) => (performance.now() - start) / 1000;
const json = async (path) => JSON.parse(await readFile(path, 'utf8'));
const saveJson = (path, value) => writeFile(path, `${JSON.stringify(value, null, 2)}\n`);

function options(argv) {
  const out = { trials: 3, timeoutMs: 120000 };
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i];
    if (key === '--help' || key === '-h') return { help: true };
    if (!['--binary', '--output', '--trials', '--timeout-ms'].includes(key) || !argv[i + 1]) {
      throw new Error(`Unknown option or missing value: ${key}`);
    }
    if (key === '--binary') out.binary = resolve(argv[i + 1]);
    if (key === '--output') out.output = resolve(argv[i + 1]);
    if (key === '--trials') out.trials = Number(argv[i + 1]);
    if (key === '--timeout-ms') out.timeoutMs = Number(argv[i + 1]);
  }
  assert(out.binary && out.output, '--binary and --output are required');
  assert(Number.isInteger(out.trials) && out.trials >= 1 && out.trials <= 100, 'Invalid trials');
  assert(Number.isInteger(out.timeoutMs) && out.timeoutMs >= 1000, 'Invalid timeout');
  return out;
}

function deferred() {
  let resolvePromise, reject;
  const promise = new Promise((yes, no) => { resolvePromise = yes; reject = no; });
  // A child can exit before the caller reaches the associated await.
  promise.catch(() => {});
  return { promise, resolve: resolvePromise, reject };
}

async function timed(promise, milliseconds, label, abort) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => {
        abort();
        reject(new Error(`${label} timed out after ${milliseconds} ms; not retried`));
      }, milliseconds);
    })]);
  } finally {
    clearTimeout(timer);
  }
}

// Manually frame output so an unterminated line cannot grow without a bound.
function launch(binary, args, env, onLine) {
  const started = performance.now();
  const child = spawn(binary, args, { cwd: root, env, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  const closed = deferred();
  const logs = [];
  let error;
  let bytes = 0;
  const fail = (cause) => {
    error ??= cause;
    child.kill();
  };
  for (const [stream, stderr] of [[child.stdout, false], [child.stderr, true]]) {
    let pending = '';
    stream.setEncoding('utf8');
    const emit = (line) => {
      const clean = line.replace(/\r$/, '');
      logs.push({ stderr, line: clean });
      try { onLine?.(clean, stderr); } catch (cause) { fail(cause); }
    };
    stream.on('data', (chunk) => {
      if (error) return;
      bytes += Buffer.byteLength(chunk);
      if (bytes > maxOutputBytes) return fail(new Error('Engine output exceeded 16 MiB'));
      pending += chunk;
      let newline;
      while ((newline = pending.indexOf('\n')) >= 0) {
        const line = pending.slice(0, newline);
        pending = pending.slice(newline + 1);
        if (Buffer.byteLength(line) > maxLineBytes) return fail(new Error('Engine line exceeded 1 MiB'));
        emit(line);
        if (error) return;
      }
      if (Buffer.byteLength(pending) > maxLineBytes) fail(new Error('Engine line exceeded 1 MiB'));
    });
    stream.on('end', () => { if (pending && !error) emit(pending); });
    stream.on('error', fail);
  }
  child.on('error', fail);
  child.stdin.on('error', fail);
  child.on('close', (code, signal) => closed.resolve({ code, signal, error, wall_seconds: secondsSince(started) }));
  return { child, closed: closed.promise, logs, fail };
}

async function fresh(binary, args, env, timeoutMs) {
  const run = launch(binary, args, env, (line) => {
    if (hitPattern.test(line)) throw new Error('Unexpected match; benchmark stopped');
  });
  run.child.stdin.end();
  const end = await timed(run.closed, timeoutMs, 'CLI job', () => run.child.kill());
  if (end.error) throw end.error;
  assert.equal(end.code, 0, `CLI failed: ${end.signal ?? end.code}`);
  return { wall_seconds: end.wall_seconds, logs: run.logs };
}

class Resident {
  constructor(binary, env, timeoutMs) {
    this.timeoutMs = timeoutMs;
    this.ready = deferred();
    this.seenReady = false;
    this.active = null;
    this.fatal = null;
    this.shuttingDown = false;
    this.run = launch(binary, ['--worker'], env, (line, stderr) => {
      if (stderr) throw new Error(`Unframed worker stderr: ${line}`);
      let event;
      try { event = JSON.parse(line); } catch { throw new Error('Invalid worker JSONL'); }
      if (event.event === 'ready') {
        assert.equal(event.version, 1, 'Unsupported worker protocol');
        assert(!this.seenReady && !this.active, 'Duplicate/late worker ready');
        this.seenReady = true;
        this.ready.resolve();
        return;
      }
      assert(this.active && event.id === this.active.id, 'Unexpected worker job id');
      if (event.event === 'log') {
        assert.equal(typeof event.line, 'string');
        assert.equal(typeof event.stderr, 'boolean');
        if (hitPattern.test(event.line)) throw new Error('Unexpected match; benchmark stopped');
        this.active.logs.push({ stderr: event.stderr, line: event.line });
      } else if (event.event === 'done') {
        const job = this.active;
        this.active = null;
        if (event.success !== true || event.error !== null) {
          job.done.reject(new Error(`Worker job ${event.id} failed: ${event.error ?? 'unspecified error'}`));
          return;
        }
        job.done.resolve({ wall_seconds: secondsSince(job.started), logs: job.logs });
      } else {
        throw new Error(`Unknown worker event: ${event.event}`);
      }
    });
    this.run.closed.then((end) => {
      if (!this.shuttingDown || end.error || end.code !== 0) {
        const error = end.error ?? new Error(`Worker exited unexpectedly (${end.signal ?? end.code}); not retried`);
        this.fatal = error;
        this.ready.reject(error);
        this.active?.done.reject(error);
      }
    });
  }

  async start() {
    await timed(this.ready.promise, this.timeoutMs, 'Worker ready', () => this.abort());
  }

  async job(id, args) {
    if (this.fatal) throw this.fatal;
    assert(this.seenReady && !this.active && !this.shuttingDown);
    const done = deferred();
    this.active = { id, done, started: performance.now(), logs: [] };
    this.run.child.stdin.write(`${JSON.stringify({ id, args })}\n`);
    return timed(done.promise, this.timeoutMs, `Worker job ${id}`, () => this.abort());
  }

  async close() {
    assert(!this.active, 'Cannot close stdin before the last done');
    this.shuttingDown = true;
    this.run.child.stdin.end();
    const end = await timed(this.run.closed, 10000, 'Worker shutdown', () => this.abort());
    if (end.error) throw end.error;
    assert.equal(end.code, 0, 'Worker shutdown failed');
  }

  abort() { this.run.child.kill(); }
}

async function prepare(directory) {
  const jobs = [];
  for (let row = 0; row < pools.length; row++) {
    const folder = join(directory, `row-${row + 1}`);
    await mkdir(folder, { recursive: true });
    for (let round = 1; round <= rounds; round++) {
      const job = {
        id: `row-${row + 1}-round-${round}`, row: row + 1, round,
        expected_raw: Math.min(limit, totalPerRow - (round - 1) * limit),
        checkpoint: join(folder, 'checkpoint.json'),
        record: join(folder, 'negative.json'),
        metrics: join(folder, `round-${round}.metrics.json`),
        log: join(folder, `round-${round}.log`),
      };
      job.args = [target, '--pattern', pattern, '--pool', pools[row],
        '--batch-size', '1048576', '--block-size', '64', '--max-candidates', String(limit),
        '--checkpoint', job.checkpoint, '--record-progress', job.record,
        '--metrics-json', job.metrics, '--metrics'];
      if (round > 1) job.args.push('--resume');
      jobs.push(job);
    }
  }
  return jobs;
}

function validateMetrics(metrics, expected) {
  assert.equal(metrics.backend, 'CUDA', 'Benchmark must use CUDA');
  assert.equal(metrics.completed_raw, expected, 'Unexpected completed count');
  assert.equal(metrics.excluded, 0, 'Synthetic benchmark must not use exclusions');
  assert.equal(metrics.pruned, 0, 'Synthetic benchmark must not prune');
  assert.equal(metrics.retained, expected, 'Unexpected retained count');
  assert(Number.isInteger(metrics.checksum_survivors) && metrics.checksum_survivors > 0
    && metrics.checksum_survivors <= expected, 'Invalid checksum count');
}

async function variant(options, trial, name, env) {
  const directory = join(options.output, `trial-${trial}`, name);
  const jobs = await prepare(directory);
  let worker;
  const outputs = [];
  const started = performance.now();
  try {
    if (name === 'resident') {
      worker = new Resident(options.binary, env, options.timeoutMs);
      await worker.start();
    }
    for (const job of jobs) {
      outputs.push(worker
        ? await worker.job(job.id, job.args)
        : await fresh(options.binary, job.args, env, options.timeoutMs));
    }
    if (worker) await worker.close();
  } catch (error) {
    worker?.abort();
    if (worker) await saveJson(join(directory, 'worker.failure.json'), worker.run.logs);
    throw error;
  }
  const wall_seconds = secondsSince(started);
  const measurements = [];
  for (let i = 0; i < jobs.length; i++) {
    const job = jobs[i];
    const output = outputs[i];
    await writeFile(job.log, output.logs.map(({ stderr, line }) => `${stderr ? '[stderr] ' : ''}${line}`).join('\n') + '\n');
    const text = output.logs.map((entry) => entry.line).join('\n');
    assert(!hitPattern.test(text), 'Unexpected matching result');
    assert(text.includes(job.round === rounds ? 'Exhausted search without a match' : 'Candidate limit reached'), 'Unexpected search outcome');
    const init = output.logs.filter(({ line }) => line === 'CUDA context initialized').length;
    const reuse = output.logs.filter(({ line }) => line === 'Reusing initialized CUDA context').length;
    const expectReuse = name === 'resident' && i > 0;
    assert.equal(init, expectReuse ? 0 : 1, `Unexpected CUDA initialization: ${job.id}`);
    assert.equal(reuse, expectReuse ? 1 : 0, `Missing/unexpected CUDA reuse: ${job.id}`);
    const metrics = await json(job.metrics);
    validateMetrics(metrics, job.expected_raw);
    measurements.push({ row: job.row, round: job.round, wall_seconds: output.wall_seconds, metrics });
  }
  const rows = [];
  for (let row = 1; row <= pools.length; row++) {
    const job = jobs.find((entry) => entry.row === row);
    const checkpoint = await json(job.checkpoint);
    const record = await json(job.record);
    assert.equal(checkpoint.checked, totalPerRow, 'Incomplete checkpoint');
    assert.equal(checkpoint.excluded, 0);
    assert.equal(checkpoint.cursor.done, true, 'Cursor must be exhausted');
    assert.equal(record.record.result, 'complete_negative');
    assert.equal(record.record.completed_candidates, totalPerRow);
    assert.deepEqual(record.record.target, Array(20).fill(0));
    assert.match(record.sha256, /^[0-9a-f]{64}$/);
    assert.equal(createHash('sha256').update(JSON.stringify(record.record)).digest('hex'), record.sha256, 'Negative record digest is invalid');
    rows.push({ row, checkpoint, negative_sha256: record.sha256 });
  }
  return { trial, variant: name, wall_seconds, completed_raw: pools.length * totalPerRow, jobs: measurements, rows };
}

function compare(freshResult, residentResult) {
  assert.deepEqual(freshResult.rows, residentResult.rows, 'Checkpoint or negative record mismatch');
  for (let i = 0; i < freshResult.jobs.length; i++) {
    const left = freshResult.jobs[i];
    const right = residentResult.jobs[i];
    assert.equal(left.row, right.row);
    assert.equal(left.round, right.round);
    for (const field of ['completed_raw', 'excluded', 'pruned', 'retained', 'checksum_survivors']) {
      assert.equal(left.metrics[field], right.metrics[field], `${field} mismatch at row ${left.row} round ${left.round}`);
    }
  }
}

async function main() {
  const opts = options(process.argv.slice(2));
  if (opts.help) {
    console.log('Usage: node scripts/benchmark-resident.mjs --binary <engine.exe> --output <NEW directory> [--trials 3] [--timeout-ms 120000]\nThree synthetic 9! spaces, 100,000 candidates per job, four jobs per row. CUDA required.');
    return;
  }
  opts.binary = await realpath(opts.binary);
  await mkdir(dirname(opts.output), { recursive: true });
  await mkdir(opts.output); // Refuse existing output directories, including partial runs.
  const env = { ...process.env, CUDA_CACHE_PATH: join(root, '.cuda', 'jit-cache') };
  await mkdir(env.CUDA_CACHE_PATH, { recursive: true });
  const report = {
    binary: opts.binary, binary_sha256: createHash('sha256').update(await readFile(opts.binary)).digest('hex'),
    target, pattern, pools, candidates_per_row: totalPerRow, candidates_per_job_limit: limit,
    trials: opts.trials, results: [], pairs: [],
    timing: 'Wall total includes process startup/shutdown, sequential job submission and engine file writes. Per-job resident timing excludes initial worker spawn. Post-run verification is outside timing.',
  };
  const resultsPath = join(opts.output, 'results.json');
  await saveJson(resultsPath, report);
  // Warm the same binary/JIT cache once; this separate process is not timed.
  const warmupJobs = await prepare(join(opts.output, 'warmup'));
  const warmup = await fresh(opts.binary, warmupJobs[0].args, env, opts.timeoutMs);
  await saveJson(join(opts.output, 'warmup', 'log.json'), warmup.logs);
  validateMetrics(await json(warmupJobs[0].metrics), limit);
  for (let trial = 1; trial <= opts.trials; trial++) {
    const pair = {};
    for (const name of trial % 2 ? ['fresh', 'resident'] : ['resident', 'fresh']) {
      const result = await variant(opts, trial, name, env);
      pair[name] = result;
      report.results.push(result);
      await saveJson(resultsPath, report);
      console.log(JSON.stringify({ trial, variant: name, wall_seconds: result.wall_seconds, completed_raw: result.completed_raw }));
    }
    compare(pair.fresh, pair.resident);
    const measured = { trial, verified_equal: true, fresh_seconds: pair.fresh.wall_seconds,
      resident_seconds: pair.resident.wall_seconds, speedup: pair.fresh.wall_seconds / pair.resident.wall_seconds };
    report.pairs.push(measured);
    await saveJson(resultsPath, report);
    console.log(JSON.stringify(measured));
  }
}

main().catch((error) => {
  console.error(`Benchmark stopped without retry: ${error.stack ?? error}`);
  process.exitCode = 1;
});
