// Real stdio MCP -> authenticated desktop -> engine integration. Synthetic target only.
import assert from 'node:assert/strict';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import { fileURLToPath } from 'node:url';
import { randomBytes } from 'node:crypto';

const client = new Client({name:'eth-mcp-desktop-test',version:'1.0.0'});
const transport = new StdioClientTransport({command:process.execPath,args:[fileURLToPath(new URL('./server.js',import.meta.url))],stderr:'pipe'});
let diagnostics = '';
transport.stderr?.on('data',chunk=>{diagnostics+=chunk.toString();});
await client.connect(transport);
const call = async (name,args={}) => {
  const r = await client.callTool({name,arguments:args},undefined,{timeout:150000});
  if(r.isError) throw new Error(r.content.map(c=>c.text||'').join('\n'));
  return r.structuredContent;
};
const completed = async id => {
  for(let i=0;i<6;i++) {
    const r=await call('eth_wait_run',{id,seconds:10});
    if(r.terminal)return r.run;
  }
  throw new Error('Engine did not reach a terminal state.');
};
try {
  const catalog = await call('eth_get_challenge');
  assert.deepEqual(catalog.externalHistory.filter(e=>e.filterImplemented).map(e=>e.id),['RO1']);
  const state = await call('eth_get_state');
  assert.equal(state.active,null,'An existing user search must not be interrupted.');
  // Independent random synthetic address prevents previous smoke runs covering this one.
  const target = '0x'+randomBytes(20).toString('hex');
  const common = {target,mode:'template',backend:'cpu',excludeRo1:false,maxCandidates:1,batchSize:32};
  const fixed = {...common,name:'MCP · teste de registro parcial',pattern:'zero zero zero zero zero zero zero zero zero ability ? ?',pool:'able about'};
  const before = await call('eth_preflight',{search:fixed});
  assert.equal(before.fullyCovered,false);
  assert.equal(before.total,'2');
  const started=await call('eth_start_search',{search:fixed});
  const limited=await completed(started.id);
  assert.equal(limited.status,'limited');
  assert.equal(limited.checked,'1');
  await call('eth_resume_search',{id:started.id});
  const resumed=await completed(started.id);
  assert.equal(resumed.status,'completed');
  assert.equal(resumed.checked,'2');
  const free={...common,maxCandidates:100,name:'MCP · teste de posições livres',pattern:'zero zero zero zero zero zero zero zero zero ? ? ?',pool:'ability able about'};
  const expanded=await completed((await call('eth_start_search',{search:free})).id);
  assert.equal(expanded.status,'completed');
  assert.equal(expanded.checked,'6');
  assert.equal(expanded.excluded,'2');
  const after=await call('eth_preflight',{search:{...free,pool:'about ability able'}});
  assert.equal(after.fullyCovered,true);
  const blocked=await client.callTool({name:'eth_start_search',arguments:{search:{...free,name:'MCP · bloqueio de duplicata'}}},undefined,{timeout:150000});
  assert.equal(blocked.isError,true);
  assert.match(blocked.content[0].text,/já foram testadas/);
  const latest=await call('eth_get_state');
  assert.equal(latest.active,null);
  assert.equal(latest.runs[0].status,'covered');
  assert.equal(latest.runs[0].checked,'0');
  const long={...common,maxCandidates:1000000,batchSize:1024,name:'MCP · teste de pausa',pattern:'zero zero zero zero zero zero ? ? ? ? ? ?',pool:'abandon ability able about above absent absorb abstract absurd abuse access accident account accuse achieve acid acoustic acquire across act'};
  const longRun=await call('eth_start_search',{search:long});
  await call('eth_pause_search');
  const paused=await completed(longRun.id);
  assert.equal(paused.status,'paused');
  console.log(JSON.stringify({mcp:'stdio',partial:limited.checked,resumed:resumed.checked,expanded:{checked:expanded.checked,excluded:expanded.excluded},duplicateBlocked:true,pause:paused.status,runIds:[started.id,expanded.id,longRun.id]},null,2));
  assert.equal(diagnostics,'','MCP stderr must be clean.');
} finally { await client.close(); }
