// Bounded, real MCP measurements of the video research hypotheses.
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import { fileURLToPath } from 'node:url';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { performance } from 'node:perf_hooks';

const reportUrl = new URL('../output/import-check/video-timing.json', import.meta.url);
const client = new Client({ name: 'video-research-measurement', version: '1.0.0' });
const transport = new StdioClientTransport({ command: process.execPath, args: [fileURLToPath(new URL('./server.js', import.meta.url))], stderr: 'pipe' });
transport.stderr?.on('data', chunk => process.stderr.write(chunk));
await client.connect(transport);
const call = async (name, args = {}) => {
  const reply = await client.callTool({ name, arguments: args }, undefined, { timeout: 150000 });
  if (reply.isError) throw new Error(reply.content.map(c => c.text || '').join('\n'));
  return reply.structuredContent;
};
const save = async report => {
  await mkdir(new URL('../output/import-check/', import.meta.url), { recursive: true });
  await writeFile(reportUrl, JSON.stringify(report, null, 2));
};
try {
  const state = await call('eth_get_state', { limit: 1 });
  if (state.active) throw new Error(`A search is active: ${state.active.id}. It was not interrupted.`);
  if (process.argv[2] === 'plan') {
    const challenge = await call('eth_get_challenge');
    const base = { target: challenge.target, mode: 'batches', post: 'dutch@1 fiber fork dinner cloud live', backend: 'auto', batchSize: 65536, maxCandidates: 1000000, noChecksum: false, excludeRo1: true };
    const baselineVideo = ['fiber', 'wood', 'winter', 'rib'];
    const cases = [];
    for (const group of challenge.videoCandidates.groups) {
      const search = { ...base, name: `MCP · vídeo ${group.label}`, video: `fog@5 parrot@12 ${[...new Set([...baselineVideo, ...group.words])].join(' ')}` };
      cases.push({ id: group.id, search, preflight: await call('eth_preflight', { search }) });
    }
    const compact = { ...base, name: 'MCP · cinco moedas · teste completo', maxCandidates: 2000000, video: `fog@5 parrot@12 ${challenge.videoCandidates.groups.find(g=>g.id==='coins').words.join(' ')}` };
    cases.unshift({ id:'compact', search:compact, preflight: await call('eth_preflight', {search:compact}) });
    const report = { date: new Date().toISOString(), transport:'MCP stdio', post:base.post, baselineVideo, cases, measurements:[] };
    await save(report);
    console.log(JSON.stringify({report:fileURLToPath(reportUrl),cases:cases.map(c=>({id:c.id,video:c.search.video,total:c.preflight.total,fullyCovered:c.preflight.fullyCovered}))},null,2));
  } else if (process.argv[2] === 'verify') {
    const report=JSON.parse(await readFile(reportUrl,'utf8'));
    report.verification=[];
    for(const measurement of report.measurements.filter(m=>m.run?.status==='completed')) {
      const preflight=await call('eth_preflight',{search:measurement.search});
      report.verification.push({id:measurement.id,noChecksum:measurement.search.noChecksum,runId:measurement.runId,fullyCovered:preflight.fullyCovered});
    }
    const finalState=await call('eth_get_state',{limit:1});
    report.activeAtEnd=finalState.active?.id||null;
    await save(report);
    console.log(JSON.stringify({verification:report.verification,active:report.activeAtEnd},null,2));
  } else {
    const report = JSON.parse(await readFile(reportUrl, 'utf8'));
    const id = process.argv[2];
    const entry = report.cases.find(c => c.id === id);
    if (!entry) throw new Error('Select a previously planned case.');
    const previous=process.argv.includes('--resume') ? report.measurements.findLast(m=>m.id===id&&['paused','limited'].includes(m.run?.status)) : null;
    if(process.argv.includes('--resume')&&!previous) throw new Error('No paused or limited measurement to resume.');
    const search = { ...(previous?.search || entry.search) };
    if (process.argv.includes('--expanded')) { search.noChecksum=true; search.excludeRo1=false; search.maxCandidates=id==='compact'?2000000:200000; search.name+=' · sem checksum'; }
    if (process.argv.includes('--complete')) {
      const total=Number(entry.preflight.total);
      if(!Number.isSafeInteger(total)||total>1000000000) throw new Error('This measurement does not allow a large complete sweep.');
      search.maxCandidates=total; search.name+=' · completa';
    }
    const preflight = await call('eth_preflight', {search});
    if (preflight.fullyCovered) { console.log(JSON.stringify({id,covered:true,preflight})); process.exitCode=0; }
    else {
      const begin = performance.now();
      const started = previous ? await call('eth_resume_search',{id:previous.runId}) : await call('eth_start_search', {search});
      const measurement = {id,search,preflight,runId:started.id,startedAt:new Date().toISOString(),resumed:!!previous};
      report.measurements.push(measurement); await save(report);
      console.log(JSON.stringify({started:started.id,id,limit:search.maxCandidates,total:preflight.total}));
      let finished;
      for (;;) {
        const waited = await call('eth_wait_run',{id:started.id,seconds:10});
        console.log(JSON.stringify({id,status:waited.run.status,checked:waited.run.checked,rate:waited.run.rate,elapsed:waited.run.elapsedSeconds}));
        if(waited.terminal) { finished=await call('eth_get_run',{id:started.id,logLines:40}); break; }
        if(performance.now()-begin>180000) { await call('eth_pause_search'); }
      }
      measurement.wallSeconds=(performance.now()-begin)/1000;
      measurement.run=finished;
      await save(report);
      console.log(JSON.stringify({id,wallSeconds:measurement.wallSeconds,run:finished},null,2));
    }
  }
} finally { await client.close(); }
