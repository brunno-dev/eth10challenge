// Run through playwright-cli run-code --filename against the desktop WebView2.
async (page) => {
  const fixture='C:/Users/desen/Downloads/codigos2/ETH/eth10challenge/output/import-check/negative.txt';
  await page.locator('#word-file-input').setInputFiles(fixture);
  await page.waitForFunction(()=>document.querySelector('#file-summary').textContent.includes('2 válidas'));
  await page.locator('#run-name').fill('Arquivo · teste automático');
  await page.locator('#target').fill('0x0000000000000000000000000000000000000000');
  await page.locator('#backend').selectOption('cpu');
  if (!await page.locator('#max-candidates').isVisible()) await page.locator('.advanced-details>summary').click();
  await page.locator('#exclude-ro1').uncheck();
  await page.locator('#max-candidates').fill('1');
  const previousId=await page.evaluate(async()=> (await window.__TAURI__.core.invoke('get_state')).queue?.id);
  await page.getByRole('button',{name:'Iniciar fila · 2 linhas',exact:true}).click();
  let actual;
  for(let attempt=0;attempt<100;attempt++) {
    actual=await page.evaluate(()=>window.__TAURI__.core.invoke('get_state'));
    if(actual.queue?.id!==previousId&&['completed','failed'].includes(actual.queue?.status))break;
    await page.waitForTimeout(100);
  }
  if(actual.queue.status!=='completed')throw new Error(JSON.stringify(actual.queue));
  const statuses=actual.queue.rows.map(row=>row.status);
  if(!['completed','covered'].includes(statuses[0])||statuses[1]!=='duplicate'||!['completed','covered'].includes(statuses[2])||statuses[3]!=='invalid'||statuses[4]!=='invalid')throw new Error(JSON.stringify(statuses));
  const runs=actual.queue.rows.filter(r=>r.runId).map(row=>actual.runs.find(r=>r.id===row.runId));
  if(runs.some(r=>r.config.pattern!=='dutch ? ? ? fog ? ? ? ? ? ? parrot'||r.config.mode!=='template'))throw new Error('Anchors were not fixed.');
  console.log(JSON.stringify({queue:actual.queue.status,statuses,runs:runs.map(r=>({checked:r.checked,total:r.total,status:r.status}))}));
}
