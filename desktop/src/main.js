import './style.css';
import { videoGroups, videoResearch, appendVideoWords, expandedSearch, checksumLabel } from './challenge.js';

const icons = {
  arrow: '<svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M5 12h14m-6-6 6 6-6 6"/></svg>',
  plus: '<svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M12 5v14M5 12h14"/></svg>',
  folder: '<svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M3 7h6l2-2h9v14H3V7Z"/></svg>',
  pause: '<svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M8 6v12M16 6v12"/></svg>',
};

document.querySelector('#app').innerHTML = `
  <aside class="sidebar" aria-label="Navegação e execuções">
    <a class="brand-mark" href="#workspace" aria-label="ETH Search Studio"><svg viewBox="0 0 32 44" fill="none" aria-hidden="true"><path d="m16 2 13 21-13 8L3 23 16 2Z"/><path d="m3 28 13 14 13-14-13 8L3 28Z"/><path d="M16 2v29L3 23m13 8 13-8M16 42v-6"/></svg></a>
    <span class="sidebar-eyebrow">WORKSPACE</span>
    <button class="nav-button selected" id="new-search" type="button">${icons.plus}<span>Nova busca</span></button>
    <div class="history-heading"><span>EXECUÇÕES</span><span id="history-count">00</span></div>
    <nav id="run-history" class="run-history" aria-label="Histórico de execuções"><p class="history-empty">Seu próximo caminho<br>começa aqui.</p></nav>
    <div class="sidebar-bottom"><span class="local-dot"></span><div>Ambiente local<small>Os seus dados ficam aqui.</small></div></div>
  </aside>

  <main id="workspace">
    <header class="topbar"><span class="wordmark">ETH <span>/</span> SEARCH STUDIO</span><span class="edition">DESKTOP <span>01</span></span></header>
    <section class="intro" aria-labelledby="page-title"><div><div class="eyebrow"><span class="line-marker"></span> EXPLORAR. REDUZIR. ENCONTRAR.</div><h1 id="page-title">Encontre o<br class="desktop-break"> próximo caminho<span>.</span></h1><p>Defina as possibilidades. Deixe o motor fazer a busca.</p></div><div class="intro-coordinate" aria-hidden="true"><span>12 POSIÇÕES</span><div class="coordinate-grid"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></div><span>UM NOVO PONTO DE PARTIDA</span></div></section>
    <div id="engine-notice" class="notice" role="status" hidden></div>
    <div class="workspace-grid">
      <section class="configuration" aria-labelledby="configuration-title">
        <div class="section-heading"><h2 id="configuration-title"><span>01</span> Configure a busca</h2><button class="text-button" id="load-example" type="button">Carregar exemplo ${icons.arrow}</button></div>
        <form id="search-form" novalidate>
          <fieldset id="config-fields">
            <legend class="sr-only">Parâmetros da busca</legend>
            <div class="field-row name-row"><label for="run-name">Nome da execução <span>opcional</span></label><input id="run-name" name="name" maxlength="100" placeholder="Uma nova hipótese" autocomplete="off"></div>
            <div class="field-row"><label for="target">Endereço alvo <span>ETHEREUM</span></label><input id="target" name="target" class="mono target-input" placeholder="0x…" spellcheck="false" autocomplete="off" aria-describedby="target-error"><p class="field-error" id="target-error" hidden></p></div>
            <div class="mode-row"><span class="field-label" id="mode-label">Origem das palavras</span><div class="segmented" role="group" aria-labelledby="mode-label"><button id="mode-template" type="button" class="active" aria-pressed="true">Palavras & posições</button><button id="mode-batches" type="button" aria-pressed="false">Post + vídeo</button><button id="mode-file" type="button" aria-pressed="false">Importar arquivo</button></div></div>
            <div id="file-fields" hidden><input type="file" id="word-file-input" accept=".txt,.doc,.docx" aria-label="Arquivo de palavras" hidden>
              <div class="file-import-box"><span class="eyebrow">UMA LINHA. UMA NOVA BUSCA.</span><h3>Traga sua lista de hipóteses.</h3><p>Selecione um TXT ou documento Word com 12 palavras por linha. Cada linha será percorrida antes de passar à próxima.</p><button id="pick-word-file" class="secondary-button accent-outline" type="button">${icons.folder} Selecionar arquivo</button><small>TXT · DOC · DOCX — até 10 MB</small></div>
              <div class="file-anchors"><span>Posições fixas neste modo</span><div><code>dutch <b>01</b></code><code>fog <b>05</b></code><code>parrot <b>12</b></code></div><p>As três palavras precisam estar em cada linha, em qualquer ordem. O sistema fixa suas posições e permuta as outras nove. Não é necessário separar post e vídeo.</p></div>
              <div id="file-preview" class="file-preview" hidden><div class="label-row"><strong id="file-name"></strong><button id="clear-word-file" class="text-button" type="button">Remover</button></div><p id="file-summary" class="input-help" role="status"></p><div id="file-rows" class="file-rows" aria-label="Linhas importadas"></div><button id="more-file-rows" type="button" class="text-button" hidden>Mostrar mais linhas</button><p class="input-help">Linhas inválidas e repetidas são sinalizadas e não entram na busca. O histórico também filtra o que já foi testado.</p></div>
            </div>
            <div id="template-fields">
              <div class="field-row pattern-row"><div class="label-row"><label for="pattern">Padrão da frase</label><span class="field-hint">Use <kbd>?</kbd> para uma posição livre</span></div><textarea id="pattern" name="pattern" rows="2" class="mono" spellcheck="false" placeholder="? ? ? ? ? ? ? ? ? ? ? ?" aria-describedby="pattern-error"></textarea><p class="field-error" id="pattern-error" hidden></p><div id="slot-preview" class="slot-preview" aria-label="Prévia das 12 posições"></div><div class="pattern-caption"><span><i class="legend-dot"></i> <span id="fixed-count">0</span> fixas</span><span><i class="legend-dot open"></i> <span id="open-count">12</span> livres</span><span id="pattern-count">12 posições</span></div></div>
              <div class="field-row"><div class="label-row"><label for="pool">Palavras candidatas</label><span id="pool-count" class="field-hint">0 palavras</span></div><textarea id="pool" name="pool" rows="3" placeholder="Separe as palavras por espaços" spellcheck="false" aria-describedby="pool-help"></textarea><p class="input-help" id="pool-help">As palavras podem ocupar as posições livres. Repetições são preservadas.</p></div>
              <details class="fill-details"><summary>Completar palavras que faltam <span>OPCIONAL</span></summary><div class="details-content"><label for="fill">Palavras ou prefixos de preenchimento</label><input id="fill" name="fill" placeholder="d*, f* ou palavras específicas" spellcheck="false"><p class="input-help">Usado quando o pool não preenche todas as posições. Vazio usa a lista completa.</p></div></details>
            </div>
            <div id="batches-fields" hidden><p class="mode-description">Seis palavras do post e seis do vídeo. Use <code>palavra@N</code> para fixar uma posição.</p><div class="clue-note"><strong>Pistas do desafio</strong><p><code>dutch → 1</code> · <code>fog → 5</code> · <code>parrot → 12</code></p><p><code>fiber</code> e <code>fork</code>: palavras confirmadas, sem posição definida. Não use <code>fiber@4</code> como confirmação.</p><p>A análise do repositório favorece <code>fog</code>; a pista da posição 5 também admite <code>cloud</code>. Os campos continuam editáveis.</p><p>As pistas abaixo são um ponto de partida. Complete as listas com suas candidatas até haver pelo menos seis palavras por fonte. Palavras sem <code>@</code> entram no conjunto de candidatas, sem posição fixa nem inclusão obrigatória quando há palavras extras.</p></div><div class="field-row"><label for="post">Palavras do post</label><textarea id="post" name="post" rows="3" placeholder="dutch@1 fiber fork …" spellcheck="false" aria-describedby="batches-error"></textarea></div><div class="field-row"><label for="video">Palavras do vídeo</label><textarea id="video" name="video" rows="3" placeholder="fog@5 parrot@12 …" spellcheck="false" aria-describedby="batches-error"></textarea></div><p class="field-error" id="batches-error" hidden></p></div>
            <details id="video-research" class="research-panel"><summary>Pistas novas do vídeo <span>07 SET 2026</span></summary><div class="details-content">
              <p>Palavras identificadas na tela e na fala. São hipóteses, sem confirmação de que façam parte da solução.</p>
              <label for="video-group">Grupo de candidatas</label><select id="video-group"></select><p id="video-group-detail" class="input-help"></p>
              <div id="video-group-words" class="research-words" aria-label="Palavras do grupo"></div>
              <button id="apply-video-group" class="secondary-button accent-outline" type="button">Adicionar ao vídeo ${icons.plus}</button>
              <p id="video-group-feedback" class="input-help" role="status"></p>
              <p class="input-help">O atalho abre Post + vídeo e preserva suas listas e posições. As palavras adicionadas ficam livres e opcionais. Complete também as seis palavras do post. Grupos maiores aumentam muito o espaço de busca.</p>
              <p class="research-source">cjmcdaniel · floflo777 · CC BY 4.0<br>Fonte: open-crypto-puzzles, issue #18 · commit 740c58b<br>109 na tela + 110 na fala = 192 distintas. Estas pistas não são filtros.</p>
            </div></details>
            <div class="engine-row"><div><label for="backend">Motor de processamento</label><p>Escolha onde executar a busca.</p></div><select id="backend" name="backend"><option value="auto">Automático · GPU preferencial</option><option value="cpu">CPU</option></select></div>
            <div class="checksum-settings"><span class="subsection-title">POLÍTICA DE VERIFICAÇÃO</span><label class="checkbox-label"><input id="no-checksum" type="checkbox" aria-describedby="checksum-help"><span>Testar também frases sem checksum válido<small>Deriva todas as frases, inclusive as que falham na verificação BIP-39.</small></span></label><p id="checksum-help" class="input-help" role="status"></p></div>
            <p id="preparation-note" class="preparation-note" role="status" hidden></p>
            <p class="input-help history-auto-note"><strong>Histórico automático ativo.</strong> Sequências já verificadas são filtradas, inclusive ao mudar posições. Buscas totalmente cobertas são bloqueadas. Pausas e limites guardam só o trecho confirmado.</p>
            <details class="advanced-details"><summary>Controles avançados <span>LIMITES & HISTÓRICO</span></summary><div class="details-content advanced-grid">
              <div class="field-row"><label for="max-candidates">Limite de candidatos</label><input id="max-candidates" name="maxCandidates" inputmode="numeric" value="100000" aria-describedby="limits-help"><p class="input-help" id="limits-help">Vazio percorre todo o espaço.</p></div>
              <div class="field-row"><label for="batch-size">Tamanho máximo do lote</label><input id="batch-size" name="batchSize" type="number" min="1" max="16777216" value="1048576" aria-describedby="batch-help"><p class="input-help" id="batch-help">Agrupa o trabalho enviado à GPU. Não altera o limite de candidatos da execução.</p></div>
              <div class="field-row"><label for="threads">Threads da CPU</label><input id="threads" name="threads" type="number" min="0" max="1024" value="0"><p class="input-help">0 usa a seleção automática.</p></div>
              <div class="field-row"><label for="language">Idioma das palavras</label><select id="language" name="language"><option value="english">Inglês</option><option value="portuguese">Português</option><option value="spanish">Espanhol</option><option value="french">Francês</option><option value="italian">Italiano</option><option value="japanese">Japonês</option><option value="korean">Coreano</option><option value="czech">Tcheco</option><option value="chinese-simplified">Chinês simplificado</option><option value="chinese-traditional">Chinês tradicional</option></select></div>
              <label class="checkbox-label span-two"><input id="adaptive" type="checkbox"><span>Lotes adaptativos<small>Ajusta o tamanho dos lotes ao tempo observado na GPU.</small></span></label>
              <div class="history-settings span-two"><div class="subsection-title">REAPROVEITAR BUSCAS</div><label class="checkbox-label"><input id="exclude-ro1" type="checkbox"><span>Excluir o domínio RO1<small id="ro1-help">Aceitar o resultado negativo externo do desafio Guntis. Requer alvo, idioma e checksum compatíveis.</small></span></label><button id="pick-records" class="secondary-button" type="button">${icons.folder} Adicionar registros locais</button><ul id="record-list" class="record-list" aria-label="Registros selecionados"></ul></div>
            </div></details>
          </fieldset>
          <p id="form-error" class="form-error" role="alert" hidden></p>
          <div class="launch-area"><button class="primary-button" id="start-search" type="submit"><span>Iniciar busca</span>${icons.arrow}</button><p id="launch-help">Execução local. Você pode pausar e continuar depois.</p></div>
        </form>
        <section id="file-queue-panel" class="file-queue-panel" aria-label="Fila do arquivo" hidden><div class="label-row"><h3>Fila do arquivo</h3><span id="file-queue-status" class="queue-status"></span></div><p id="file-queue-name"></p><p id="file-queue-progress" role="status"></p><div class="progress-track"><i id="file-queue-bar"></i></div><p id="file-queue-detail" class="input-help"></p><p id="file-queue-error" class="form-error" role="alert" hidden></p><div class="queue-actions"><button id="pause-file-queue" type="button" class="secondary-button">${icons.pause} Pausar fila</button><button id="resume-file-queue" type="button" class="secondary-button accent-outline">Continuar fila ${icons.arrow}</button><button id="cancel-file-queue" type="button" class="text-button">Encerrar fila</button></div><div id="file-queue-rows" class="file-rows" aria-label="Andamento das linhas"></div></section>
      </section>

      <aside class="monitor" aria-labelledby="monitor-title"><div class="monitor-heading"><h2 id="monitor-title"><span>02</span> Monitor da execução</h2><span id="status-badge" class="status-badge idle"><i></i><span>Em espera</span></span></div>
        <div class="run-controls"><button id="pause-search" type="button" class="secondary-button" hidden>${icons.pause}<span>Pausar busca</span></button><button id="resume-search" type="button" class="secondary-button accent-outline" hidden><span>Continuar execução</span>${icons.arrow}</button><button id="open-folder" type="button" class="icon-button" title="Abrir pasta da execução" aria-label="Abrir pasta da execução" hidden>${icons.folder}</button></div>
        <div class="telemetry-visual" aria-hidden="true"><svg viewBox="0 0 320 110" fill="none"><path class="orbit" d="M61 108a99 99 0 0 1 198 0M88 108a72 72 0 0 1 144 0M114 108a46 46 0 0 1 92 0"/><path class="guide" d="M160 8v100M56 108h208M86 35l74 73 74-73"/><circle cx="160" cy="108" r="5"/><path class="signal" d="M160 36a72 72 0 0 1 57 28"/></svg><span>SEARCH ENGINE / LOCAL</span></div>
        <div class="monitor-main"><span class="eyebrow">CANDIDATOS PROCESSADOS</span><div id="checked-value" class="primary-number">—</div><p id="run-description">Pronto para explorar uma nova hipótese.</p><p id="run-policy" class="policy-badge" hidden></p><button id="prepare-expanded" class="secondary-button accent-outline" type="button" hidden>Preparar esta busca sem checksum ${icons.arrow}</button></div>
        <div class="monitor-stats"><div><span>Velocidade</span><strong id="rate-value">—</strong><small>candidatos / segundo</small></div><div><span>Excluídos pelo histórico</span><strong id="excluded-value">—</strong><small>trabalho reaproveitado</small></div><div><span>Espaço de busca</span><strong id="total-value">—</strong><small id="total-caption">aguardando configuração</small></div><div><span>Tempo de execução</span><strong id="elapsed-value">—</strong><small>tempo informado pelo motor</small></div></div>
        <div id="progress-section" class="progress-section" hidden><div><span>Espaço percorrido</span><strong id="progress-label">0%</strong></div><div id="progressbar" class="progress-track" role="progressbar" aria-label="Espaço de busca percorrido" aria-valuemin="0" aria-valuemax="100" aria-valuenow="0"><i></i></div></div>
        <div id="run-error" class="run-error" role="alert" hidden></div>
        <div id="metrics-section" class="metrics-section" hidden><div class="subsection-title">TEMPO POR ETAPA <span>SEGUNDOS</span></div><dl id="stage-metrics"></dl><p>Etapas podem se sobrepor. Seed e endereço detalham a derivação na GPU; não são tempos adicionais.</p></div>
        <section class="activity-section" aria-label="Atividade do motor"><div class="activity-heading"><h3>Atividade</h3><span id="log-state">AGUARDANDO</span></div><p id="latest-log" class="latest-log">As mensagens da execução aparecerão aqui.</p><details class="log-details"><summary>Ver log completo <span id="log-count">0 linhas</span></summary><pre id="run-logs" tabindex="0" aria-label="Log da execução">Nenhuma execução iniciada.</pre></details></section>
        <footer class="monitor-footer"><span class="local-dot"></span><span id="engine-label">Verificando motor local…</span><span class="footer-cross" aria-hidden="true">+</span></footer>
      </aside>
    </div>
    <footer class="page-footer"><span>PRECISÃO EM CADA POSSIBILIDADE.</span><span>ETH SEARCH STUDIO <i>/</i> 01</span></footer>
  </main>`;

const $ = (id) => document.getElementById(id);
const bridge = window.__TAURI__?.core?.invoke;
const state = { runs: [], active: null, selectedId: null, mode: 'template', records: [], engineAvailable: false, pending: false, pollTimer: null, closed: false, pollError: false, refreshPromise: null, historyKey: '', filePreview: null, fileRowsShown: 50, queue: null, queueKey: '' };
// Challenge sources: open-crypto-puzzles/1-big-prizes/guntis-vitolins-metamask-8-6eth/
// README.md (anchors and fog interpretation), puzzle.json (address), clues/author-posts.md (floating words).
const defaults = { name: '', target: '0x9c2f44efad0c1e852a09df9939e6daf061140caf', mode: 'batches', pattern: '? ? ? ? ? ? ? ? ? ? ? ?', pool: '', fill: '', post: 'dutch@1 fiber fork', video: 'fog@5 parrot@12', backend: 'auto', language: 'english', maxCandidates: '100000', batchSize: 1048576, threads: 0, adaptive: false, noChecksum: false, excludeRo1: true, excludeRecords: [] };
const busyStatuses = new Set(['running', 'pausing']);
const statusLabels = { running: 'Em execução', pausing: 'Pausando', paused: 'Pausada', completed: 'Concluída', limited: 'Limite atingido', found: 'Resultado encontrado', failed: 'Falha', covered: 'Já testada' };
const descriptions = { running: 'Explorando as possibilidades definidas.', pausing: 'Aguardando a confirmação do último progresso.', paused: 'Progresso salvo. Continue quando quiser.', completed: 'Espaço esgotado sem correspondência.', limited: 'Limite desta execução atingido. É possível continuar.', found: 'Correspondência encontrada. Consulte o log local.', failed: 'A execução foi interrompida por um erro.', covered: 'Busca não iniciada: todas as sequências já estão no histórico.' };
const formatNumber = (value) => {
  if (value === null || value === undefined || value === '') return '—';
  try { return BigInt(String(value)).toLocaleString('pt-BR'); } catch { return '—'; }
};
const errorText = (error) => typeof error === 'string' ? error : error?.message || 'Não foi possível concluir a operação.';
const currentRun = () => state.runs.find((run) => run.id === state.selectedId) || (state.active?.id === state.selectedId ? state.active : null);
const queueBusy = () => ['running', 'pausing'].includes(state.queue?.status);
const queueReserved = () => state.queue && !['completed','cancelled','found'].includes(state.queue.status);
const engineBusy = () => busyStatuses.has(state.active?.status) || queueBusy();
const showError = (text = '') => { $('form-error').textContent = text; $('form-error').hidden = !text; };
function notice(text = '') { $('engine-notice').textContent = text; $('engine-notice').hidden = !text; }

function setMode(mode) {
  state.mode = ['batches','file'].includes(mode) ? mode : 'template';
  for (const name of ['template', 'batches', 'file']) {
    const active = state.mode === name;
    $(`mode-${name}`).classList.toggle('active', active);
    $(`mode-${name}`).setAttribute('aria-pressed', String(active));
    $(`${name}-fields`).hidden = !active;
  }
  $('language').disabled = state.mode === 'file';
  if (state.mode === 'file') $('language').value = 'english';
  $('limits-help').textContent = state.mode === 'file' ? 'A fila retoma os trechos automaticamente até concluir cada linha.' : 'Vazio percorre todo o espaço.';
  renderFilePreview();
}

function refreshPreview() {
  const tokens = $('pattern').value.trim().split(/\s+/).filter(Boolean);
  $('slot-preview').replaceChildren(...Array.from({ length: 12 }, (_, index) => {
    const cell = document.createElement('div');
    const word = tokens[index];
    cell.className = `slot ${word && word !== '?' ? 'fixed' : 'open'}`;
    const number = document.createElement('span'); number.textContent = String(index + 1).padStart(2, '0');
    const text = document.createElement('strong'); text.textContent = word || '·'; text.title = word || 'Posição não informada';
    cell.append(number, text); return cell;
  }));
  const fixed = tokens.slice(0, 12).filter((token) => token !== '?').length;
  $('fixed-count').textContent = String(fixed);
  $('open-count').textContent = String(tokens.slice(0, 12).filter((token) => token === '?').length);
  $('pattern-count').textContent = `${tokens.length} ${tokens.length === 1 ? 'posição' : 'posições'}`;
  $('pattern-count').classList.toggle('invalid', tokens.length !== 12);
  const count = $('pool').value.trim().split(/\s+/).filter(Boolean).length;
  $('pool-count').textContent = `${count} ${count === 1 ? 'palavra' : 'palavras'}`;
}

function renderRecords() {
  $('record-list').replaceChildren(...state.records.map((path) => {
    const li = document.createElement('li');
    const label = document.createElement('span'); label.textContent = path.split(/[\\/]/).pop(); label.title = path;
    const remove = document.createElement('button'); remove.type = 'button'; remove.textContent = '×'; remove.setAttribute('aria-label', `Remover registro ${label.textContent}`);
    remove.addEventListener('click', () => { state.records = state.records.filter((item) => item !== path); renderRecords(); });
    li.append(label, remove); return li;
  }));
}

function loadConfig(config = defaults) {
  const value = { ...defaults, ...config };
  for (const [key, id] of Object.entries({ name: 'run-name', target: 'target', pattern: 'pattern', pool: 'pool', fill: 'fill', post: 'post', video: 'video', backend: 'backend', language: 'language', maxCandidates: 'max-candidates', batchSize: 'batch-size', threads: 'threads' })) $(id).value = value[key] ?? '';
  for (const [key, id] of Object.entries({ adaptive: 'adaptive', noChecksum: 'no-checksum', excludeRo1: 'exclude-ro1' })) $(id).checked = Boolean(value[key]);
  state.records = [...(value.excludeRecords || [])];
  $('preparation-note').hidden = true;
  $('video-group-feedback').textContent = '';
  updateChecksumPolicy();
  setMode(value.mode); renderRecords(); refreshPreview(); clearValidation(); showError();
}

function readConfig() {
  updateChecksumPolicy();
  return { name: $('run-name').value.trim(), target: $('target').value.trim(), mode: state.mode, pattern: $('pattern').value.trim(), pool: $('pool').value.trim(), fill: $('fill').value.trim(), post: $('post').value.trim(), video: $('video').value.trim(), backend: $('backend').value, language: $('language').value, maxCandidates: $('max-candidates').value.trim(), batchSize: Number($('batch-size').value), threads: Number($('threads').value), adaptive: $('adaptive').checked, noChecksum: $('no-checksum').checked, excludeRo1: $('exclude-ro1').checked, excludeRecords: [...state.records] };
}

function clearValidation() {
  for (const id of ['target', 'pattern', 'batches']) $(`${id}-error`).hidden = true;
  for (const id of ['target', 'pattern', 'post', 'video']) $(id).removeAttribute('aria-invalid');
}

function validate(config) {
  clearValidation();
  const errors = [];
  if (!/^0x[0-9a-fA-F]{40}$/.test(config.target)) errors.push(['target', 'Informe um endereço Ethereum com 0x e 40 caracteres hexadecimais.']);
  if (config.mode === 'template' && config.pattern.split(/\s+/).filter(Boolean).length !== 12) errors.push(['pattern', 'O padrão precisa conter exatamente 12 palavras ou posições livres (?).']);
  if (config.mode === 'batches' && [config.post, config.video].some((words) => words.split(/\s+/).filter(Boolean).length < 6)) errors.push(['batches', 'Complete as listas: são necessárias pelo menos seis palavras do post e seis do vídeo, contando as palavras com posição fixa.']);
  for (const [id, message] of errors) { $(`${id}-error`).textContent = message; $(`${id}-error`).hidden = false; $(id === 'batches' ? 'post' : id).setAttribute('aria-invalid', 'true'); }
  if (errors.length) { $(errors[0][0] === 'batches' ? 'post' : errors[0][0]).focus(); return false; }
  if (config.maxCandidates && !/^[1-9]\d*$/.test(config.maxCandidates)) { showError('O limite de candidatos deve ser um inteiro positivo, ou ficar vazio.'); $('max-candidates').closest('details').open = true; $('max-candidates').focus(); return false; }
  if (!Number.isSafeInteger(config.batchSize) || config.batchSize < 1 || config.batchSize > 16777216 || !Number.isSafeInteger(config.threads) || config.threads < 0 || config.threads > 1024) { showError('Confira o tamanho do lote (1 a 16.777.216) e as threads (0 a 1.024).'); $('batch-size').closest('details').open = true; return false; }
  if (!config.name) config.name = `Busca · ${new Date().toLocaleString('pt-BR')}`;
  return true;
}

function renderHistory() {
  const key = JSON.stringify([state.selectedId, state.runs.map((run) => [run.id, run.name, run.config?.name, run.status, run.config?.noChecksum])]);
  if (key === state.historyKey) return;
  state.historyKey = key;
  $('history-count').textContent = String(state.runs.length).padStart(2, '0');
  $('new-search').classList.toggle('selected', !state.selectedId);
  if (!state.runs.length) { const text = document.createElement('p'); text.className = 'history-empty'; text.textContent = 'Seu próximo caminho começa aqui.'; $('run-history').replaceChildren(text); return; }
  $('run-history').replaceChildren(...state.runs.map((run) => {
    const button = document.createElement('button'); button.type = 'button'; button.className = `history-item ${run.id === state.selectedId ? 'selected' : ''}`; button.setAttribute('aria-pressed', String(run.id === state.selectedId));
    const name = document.createElement('strong'); name.textContent = run.name || run.config?.name || 'Busca sem nome';
    const info = document.createElement('span'); const dot = document.createElement('i'); dot.className = `run-dot ${run.status}`;
    const text = document.createElement('span'); text.textContent = statusLabels[run.status] || run.status;
    const policy = document.createElement('small'); policy.className = 'history-policy'; policy.textContent = checksumLabel(run.config);
    info.append(dot, text); button.append(name, info, policy);
    button.addEventListener('click', () => { state.selectedId = run.id; loadConfig(run.config); render(); });
    return button;
  }));
}

function displayDuration(seconds) {
  if (!Number.isFinite(seconds) || seconds < 0) return '—';
  const whole = Math.floor(seconds); const hours = Math.floor(whole / 3600); const minutes = Math.floor((whole % 3600) / 60); const rest = whole % 60;
  return hours ? `${hours}h ${String(minutes).padStart(2, '0')}m` : `${String(minutes).padStart(2, '0')}:${String(rest).padStart(2, '0')}`;
}

function renderMetrics(metrics) {
  $('metrics-section').hidden = !metrics;
  if (!metrics) return;
  const names = { producer_seconds: 'Geração', wait_seconds: 'Espera', transfer_seconds: 'Transferência', filter_seconds: 'Checksum', derive_seconds: 'Derivação total', gpu_seed_seconds: 'Seed / PBKDF2 (GPU)', gpu_address_seconds: 'Endereço / BIP32 (GPU)', checkpoint_seconds: 'Progresso salvo' };
  $('stage-metrics').replaceChildren(...Object.entries(names).flatMap(([key, label]) => {
    if (metrics[key] == null || !Number.isFinite(Number(metrics[key]))) return [];
    const dt = document.createElement('dt'); dt.textContent = label;
    const dd = document.createElement('dd'); dd.textContent = Number(metrics[key]).toLocaleString('pt-BR', { minimumFractionDigits: 3, maximumFractionDigits: 3 });
    return [dt, dd];
  }));
}

function renderFilePreview() {
  const preview = state.filePreview;
  $('file-preview').hidden = !preview;
  if (!preview) return;
  $('file-name').textContent = preview.filename;
  $('file-summary').textContent = `${preview.validCount} válidas · ${preview.invalidCount} inválidas · ${preview.duplicateCount} repetidas`;
  $('file-rows').replaceChildren(...preview.rows.slice(0, state.fileRowsShown).map(row => {
    const div = document.createElement('div'); div.className = `file-row ${row.error ? 'invalid' : row.duplicateOf != null ? 'duplicate' : ''}`;
    const number = document.createElement('span'); number.className = 'file-line'; number.textContent = String(row.line).padStart(2,'0');
    const body = document.createElement('div');
    const phrase = document.createElement('p'); phrase.textContent = row.words.join(' ');
    const status = document.createElement('small'); status.textContent = row.error || (row.duplicateOf != null ? `Repetida: mesmas palavras da linha ${row.duplicateOf}.` : 'Pronta · 3 posições fixas, 9 palavras para permutar');
    body.append(phrase,status); div.append(number,body); return div;
  }));
  $('more-file-rows').hidden = preview.rows.length <= state.fileRowsShown;
}

function renderFileQueue() {
  const queue = state.queue;
  $('file-queue-panel').hidden = !queue;
  if (!queue) return;
  const labels = {running:'Em execução',pausing:'Pausando',paused:'Pausada',completed:'Concluída',found:'Resultado encontrado',failed:'Requer atenção',cancelled:'Encerrada',pending:'Aguardando',covered:'Já testada',invalid:'Inválida',duplicate:'Repetida'};
  $('file-queue-status').textContent = labels[queue.status] || queue.status;
  $('file-queue-name').textContent = queue.filename;
  const rows = queue.rows || [];
  const finished = rows.filter(r=>['completed','covered','duplicate','invalid','found'].includes(r.status)).length;
  $('file-queue-progress').textContent = `${finished} de ${rows.length} linhas resolvidas · ${checksumLabel(queue.config)}`;
  $('file-queue-bar').style.width = `${rows.length ? 100*finished/rows.length : 0}%`;
  const current = rows.find(r=>['running','paused','failed','found'].includes(r.status)) || rows.find(r=>r.status==='pending');
  $('file-queue-detail').textContent = queue.status==='found' ? 'Correspondência encontrada. A fila parou e não executou as linhas seguintes.' : queue.status==='cancelled' ? 'Fila encerrada. O progresso confirmado continua salvo no histórico.' : current ? `Linha ${current.line} · ${labels[current.status] || current.status}. A fila aproveita o histórico entre as linhas.` : 'Todas as linhas desta fila foram avaliadas ou sinalizadas.';
  $('file-queue-error').textContent = queue.error || ''; $('file-queue-error').hidden = !queue.error;
  $('pause-file-queue').hidden = !queueBusy();
  $('pause-file-queue').disabled = state.pending || queue.status === 'pausing';
  $('resume-file-queue').hidden = !['paused','failed'].includes(queue.status);
  $('resume-file-queue').disabled = state.pending || engineBusy();
  $('cancel-file-queue').hidden = ['completed','cancelled','found'].includes(queue.status);
  $('cancel-file-queue').disabled = state.pending;
  const key = JSON.stringify([queue.id,queue.status,rows.map(r=>[r.line,r.status,r.runId])]);
  if (key !== state.queueKey) {
    state.queueKey = key;
    const currentIndex = Math.max(0,rows.findIndex(r=>r===current));
    const first = Math.max(0,currentIndex-2);
    $('file-queue-rows').replaceChildren(...rows.slice(first,first+8).map(row=> {
      const div=document.createElement('div'); div.className=`file-row ${row.status==='invalid'||row.status==='failed'?'invalid':''}`;
      const number=document.createElement('span'); number.className='file-line'; number.textContent=String(row.line).padStart(2,'0');
      const body=document.createElement('div'); const label=document.createElement('p'); label.textContent=labels[row.status]||row.status;
      const detail=document.createElement('small'); detail.textContent=row.error||row.words?.join(' ')||''; body.append(label,detail);
      div.append(number,body);
      if (row.runId) { const button=document.createElement('button'); button.type='button'; button.className='text-button'; button.textContent='Ver'; button.addEventListener('click',()=>{state.selectedId=row.runId;render();}); div.append(button); }
      return div;
    }));
  }
}

function render() {
  const run = currentRun(); const busy = engineBusy();
  $('config-fields').disabled = busy || state.pending;
  $('load-example').disabled = busy || state.pending;
  $('new-search').disabled = busy || state.pending;
  $('apply-video-group').disabled = busy || state.pending || Boolean(queueReserved());
  $('prepare-expanded').hidden = !run || run.config?.noChecksum || !['completed','covered'].includes(run.status);
  $('prepare-expanded').disabled = busy || state.pending || Boolean(queueReserved());
  $('run-policy').hidden = !run;
  $('run-policy').textContent = run ? checksumLabel(run.config) : '';
  $('start-search').disabled = !bridge || !state.engineAvailable || busy || state.pending || queueReserved() || (state.mode === 'file' && !state.filePreview?.validCount);
  $('start-search').classList.toggle('working', state.pending);
  $('start-search').querySelector('span').textContent = busy ? 'Uma busca está em andamento' : state.pending ? 'Preparando…' : queueReserved() ? 'Continue ou encerre a fila abaixo' : state.mode === 'file' ? `Iniciar fila${state.filePreview ? ` · ${state.filePreview.validCount} linhas` : ''}` : 'Iniciar busca';
  $('launch-help').textContent = !bridge ? 'Prévia visual. Abra o aplicativo para executar uma busca.' : busy ? 'Pause a execução antes de iniciar uma nova busca.' : 'Execução local. Você pode pausar e continuar depois.';
  $('pick-records').disabled = !bridge || busy || state.pending;
  $('pick-word-file').disabled = !bridge || busy || state.pending;
  $('status-badge').className = `status-badge ${run?.status || 'idle'}`;
  $('status-badge').querySelector('span').textContent = run ? statusLabels[run.status] || run.status : 'Em espera';
  $('checked-value').textContent = run ? formatNumber(run.checked) : '—';
  $('checked-value').classList.toggle('compact-number', $('checked-value').textContent.length > 12);
  $('excluded-value').textContent = run ? formatNumber(run.excluded) : '—';
  $('rate-value').textContent = run && Number.isFinite(run.rate) ? Math.round(run.rate).toLocaleString('pt-BR') : '—';
  $('total-value').textContent = run ? formatNumber(run.total) : '—';
  $('total-caption').textContent = run ? run.total == null ? 'ainda não informado pelo motor' : 'candidatos no domínio original' : 'aguardando configuração';
  $('elapsed-value').textContent = run ? displayDuration(run.elapsedSeconds) : '—';
  const preparing = run?.backend === 'auto' && run?.checked === '0';
  $('run-description').textContent = run?.status === 'running' && preparing
    ? 'Preparando o motor. A primeira inicialização da GPU pode levar alguns minutos.'
    : run?.status === 'pausing' && preparing
      ? 'Pausa solicitada. Aguardando a inicialização do motor para salvar e encerrar.'
      : run ? descriptions[run.status] || '' : 'Pronto para explorar uma nova hipótese.';
  if (run && ['completed','covered'].includes(run.status)) {
    $('run-description').textContent = run.status === 'covered'
      ? `Já testada nesta política: ${run.config.noChecksum ? 'com e sem checksum válido.' : 'somente checksum válido. Frases inválidas não foram derivadas.'}`
      : run.config.noChecksum ? 'Espaço esgotado sem correspondência, incluindo frases com checksum inválido.' : 'Espaço esgotado sem correspondência com checksum válido. Frases inválidas não foram derivadas.';
  }
  $('progress-section').hidden = true;
  if (run?.total) {
    try {
      const total = BigInt(run.total); const checked = BigInt(run.checked || '0');
      if (total > 0n) { const percent = Math.min(100, Math.max(0, Number(checked * 10000n / total) / 100)); $('progress-section').hidden = false; $('progress-label').textContent = `${percent.toLocaleString('pt-BR', { maximumFractionDigits: 2 })}%`; $('progressbar').setAttribute('aria-valuenow', String(percent)); $('progressbar').querySelector('i').style.width = `${percent}%`; }
    } catch { /* Unknown or malformed counts do not produce estimated progress. */ }
  }
  const selectedActive = run && run.id === state.active?.id;
  $('pause-search').hidden = !queueBusy() && (!selectedActive || !busy);
  $('pause-search').disabled = state.pending || run?.status === 'pausing' || state.queue?.status === 'pausing';
  $('pause-search').querySelector('span').textContent = run?.status === 'pausing' || state.queue?.status === 'pausing' ? 'Salvando progresso…' : queueBusy() ? 'Pausar fila' : 'Pausar busca';
  const canResumeQueue = ['paused','failed'].includes(state.queue?.status);
  $('resume-search').hidden = !canResumeQueue && (!run || !['paused', 'limited'].includes(run.status) || state.queue?.rows?.some(row=>row.runId===run.id && !['completed','cancelled'].includes(state.queue.status)));
  $('resume-search').querySelector('span').textContent = canResumeQueue ? 'Continuar fila' : 'Continuar execução';
  $('resume-search').disabled = !bridge || busy || state.pending || !state.engineAvailable;
  $('open-folder').hidden = !run;
  $('open-folder').disabled = !bridge || state.pending;
  $('run-error').textContent = run?.error || ''; $('run-error').hidden = !run?.error;
  const logs = Array.isArray(run?.logs) ? run.logs.slice(-250) : [];
  $('latest-log').textContent = logs.length ? logs[logs.length - 1] : run ? 'Aguardando mensagens do motor.' : 'As mensagens da execução aparecerão aqui.';
  $('log-state').textContent = run ? busyStatuses.has(run.status) ? 'AO VIVO' : 'REGISTRADO' : 'AGUARDANDO';
  $('log-count').textContent = `${logs.length} linhas${run?.logs?.length > 250 ? ' recentes' : ''}`;
  const logView = $('run-logs'); const atBottom = logView.scrollHeight - logView.scrollTop - logView.clientHeight < 40;
  const logText = logs.length ? logs.join('\n') : 'Nenhuma mensagem disponível.';
  if (logView.textContent !== logText) { logView.textContent = logText; if (atBottom) logView.scrollTop = logView.scrollHeight; }
  renderMetrics(run?.metrics);
  $('engine-label').textContent = !bridge ? 'Prévia visual · sem motor' : !state.engineAvailable ? 'Motor indisponível' : run?.backend ? `Motor ${run.backend} · local` : 'Motor local disponível';
  renderHistory();
  renderFileQueue();
}

function mergeSnapshot(snapshot, selectActive = false) {
  const priorActive = state.active?.id;
  const priorQueue = state.queue;
  state.active = snapshot.active || null;
  if (Object.hasOwn(snapshot,'queue')) state.queue = snapshot.queue;
  state.runs = Array.isArray(snapshot.runs) ? snapshot.runs : state.runs;
  if (state.active) state.runs = [state.active, ...state.runs.filter((run) => run.id !== state.active.id)];
  if (selectActive && state.active) state.selectedId = state.active.id;
  if (queueBusy() && state.active) state.selectedId = state.active.id;
  if (state.queue?.status === 'found' && (priorQueue?.id !== state.queue.id || priorQueue?.status !== 'found')) state.selectedId = state.queue.rows.find(row=>row.status==='found')?.runId || state.selectedId;
  if (!state.selectedId && priorActive && state.runs.some((run) => run.id === priorActive)) state.selectedId = priorActive;
  render();
}

async function refresh(selectActive = false) {
  if (!state.refreshPromise) state.refreshPromise = bridge('get_state').then((snapshot) => {
    if (!state.closed) mergeSnapshot(snapshot);
  }).finally(() => { state.refreshPromise = null; });
  await state.refreshPromise;
  if (selectActive && state.active && !state.closed) { state.selectedId = state.active.id; render(); }
}

async function action(work) {
  if (state.pending) return;
  state.pending = true; showError(); render();
  try { if (state.refreshPromise) await state.refreshPromise; await work(); } catch (error) { showError(errorText(error)); }
  finally { state.pending = false; render(); }
}

function updateChecksumPolicy() {
  const expanded = $('no-checksum').checked;
  if (expanded) $('exclude-ro1').checked = false;
  $('exclude-ro1').disabled = expanded;
  $('checksum-help').textContent = expanded
    ? 'Busca ampliada: pode exigir cerca de 16 vezes mais derivações. RO1 desativado. O histórico automático usa apenas registros desta mesma política; registros manuais também precisam ser compatíveis.'
    : 'Busca com checksum obrigatório. Frases que falham nessa verificação são descartadas antes de derivar endereços. “Já testada” vale apenas para esta política.';
  $('ro1-help').textContent = expanded ? 'Desativado: RO1 não cobre as derivações de frases com checksum inválido.' : 'Aceitar o resultado negativo externo do desafio Guntis. Requer alvo, idioma e checksum compatíveis.';
}

function renderVideoGroup() {
  const group = videoGroups.find(g => g.id === $('video-group').value) || videoGroups[0];
  $('video-group-detail').textContent = group.detail;
  $('video-group-words').textContent = group.words.join(' · ');
  $('video-group-feedback').textContent = '';
}

for (const group of videoGroups) {
  const option = document.createElement('option'); option.value = group.id; option.textContent = `${group.label} · ${group.words.length} palavras`; $('video-group').append(option);
}
renderVideoGroup();
$('video-group').addEventListener('change', renderVideoGroup);
$('apply-video-group').addEventListener('click', () => {
  if (engineBusy() || state.pending || queueReserved()) return;
  const group = videoGroups.find(g => g.id === $('video-group').value);
  const merged = appendVideoWords($('video').value, group.words);
  $('video').value = merged.text;
  $('language').value = 'english';
  state.selectedId = null;
  setMode('batches'); clearValidation(); showError(); render();
  $('video-group-feedback').textContent = merged.added ? `${merged.added} candidatas adicionadas ao vídeo. Confira as duas listas antes de iniciar.` : 'Este grupo já está na lista do vídeo. Nenhuma palavra duplicada foi adicionada.';
});
$('no-checksum').addEventListener('change', updateChecksumPolicy);
$('prepare-expanded').addEventListener('click', () => {
  const run = currentRun();
  if (!run || run.config.noChecksum || engineBusy() || state.pending || queueReserved()) return;
  const manualRecords = run.config.excludeRecords?.length || 0;
  loadConfig(expandedSearch(run.config)); state.selectedId = null;
  $('preparation-note').textContent = 'Nova busca preparada com as mesmas palavras e posições, incluindo frases sem checksum válido. Clique em Iniciar busca quando quiser executar.' + (manualRecords ? ' Registros manuais da política anterior foram removidos; o histórico automático continua ativo.' : '');
  $('preparation-note').hidden = false; render(); $('run-name').focus();
});

$('mode-template').addEventListener('click', () => setMode('template'));
$('mode-batches').addEventListener('click', () => setMode('batches'));
$('mode-file').addEventListener('click', () => { setMode('file'); render(); });
$('pattern').addEventListener('input', refreshPreview);
$('pool').addEventListener('input', refreshPreview);
$('load-example').addEventListener('click', () => { state.selectedId = null; loadConfig({ ...defaults, name: 'Exploração · exemplo local', target: '0x0000000000000000000000000000000000000000', mode: 'template', post: '', video: '', excludeRo1: false, pattern: 'abandon abandon abandon abandon abandon abandon ? ? ? ? ? ?', pool: 'abandon ability able about above absent absorb abstract absurd abuse' }); render(); });
$('new-search').addEventListener('click', () => { state.selectedId = null; loadConfig(); render(); $('target').focus(); });
$('search-form').addEventListener('submit', (event) => {
  event.preventDefault(); showError();
  if (!bridge || !state.engineAvailable || engineBusy() || state.pending) return;
  const config = readConfig();
  if (state.mode === 'file') {
    config.mode = 'template'; config.pattern = 'dutch ? ? ? fog ? ? ? ? ? ? parrot'; config.pool = state.filePreview?.rows.find(r=>!r.error&&r.duplicateOf==null)?.pool || ''; config.fill=''; config.post=''; config.video=''; config.language='english';
    if (!state.filePreview?.validCount || !validate(config)) return;
    action(async () => { state.queue = await bridge('start_file_queue',{ config, importId: state.filePreview.importId }); await refresh(true); });
    return;
  }
  if (!validate(config)) return;
  action(async () => { const result = await bridge('start_search', { config }); if (result?.id) state.selectedId = result.id; await refresh(true); });
});
$('pause-search').addEventListener('click', () => action(async () => { await bridge(queueBusy() ? 'pause_file_queue' : 'pause_search'); await refresh(); }));
$('resume-search').addEventListener('click', () => {
  if (['paused','failed'].includes(state.queue?.status) && !engineBusy()) { action(async()=>{state.queue=await bridge('resume_file_queue');await refresh(true);}); return; }
  const runId = state.selectedId;
  if (!runId || engineBusy()) return;
  action(async () => { await bridge('resume_search', { runId }); await refresh(true); });
});
$('pick-records').addEventListener('click', () => action(async () => { const paths = await bridge('pick_records'); state.records = [...new Set([...state.records, ...(paths || [])])]; renderRecords(); }));
$('open-folder').addEventListener('click', () => { const runId = state.selectedId; if (runId) action(() => bridge('open_run_folder', { runId })); });
$('pick-word-file').addEventListener('click',()=>{ $('word-file-input').value=''; $('word-file-input').click(); });
$('word-file-input').addEventListener('change',()=>{ const file=$('word-file-input').files?.[0]; if (!file) return; action(async()=>{
  if(file.size>10*1024*1024) throw new Error('O arquivo deve ter até 10 MB.');
  const preview=await bridge('import_word_bytes',{filename:file.name,bytes:Array.from(new Uint8Array(await file.arrayBuffer()))});
  state.filePreview=preview;state.fileRowsShown=50;renderFilePreview();
}); });
$('clear-word-file').addEventListener('click',()=>{state.filePreview=null;renderFilePreview();render();});
$('more-file-rows').addEventListener('click',()=>{state.fileRowsShown+=50;renderFilePreview();});
for (const [button,command] of [['pause-file-queue','pause_file_queue'],['resume-file-queue','resume_file_queue'],['cancel-file-queue','cancel_file_queue']]) $(button).addEventListener('click',()=>action(async()=>{state.queue=await bridge(command);await refresh(true);}));

async function poll() {
  if (state.closed || !bridge) return;
  try { if (!state.pending) { await refresh(); if (state.pollError) { state.pollError = false; notice(); } } }
  catch (error) { state.pollError = true; notice(`Conexão com o motor interrompida: ${errorText(error)}`); }
  finally { if (!state.closed) state.pollTimer = window.setTimeout(poll, 1000); }
}

window.addEventListener('beforeunload', () => { state.closed = true; window.clearTimeout(state.pollTimer); });
async function initialize() {
loadConfig();
if (!bridge) {
  notice('Prévia visual • motor disponível no aplicativo'); render();
} else {
  try {
    const initial = await bridge('bootstrap'); state.engineAvailable = Boolean(initial.engineAvailable);
    if (!state.engineAvailable) notice('Motor de busca não encontrado. Verifique a instalação do aplicativo.');
    mergeSnapshot(initial, true);
    if (state.active?.config) loadConfig(state.active.config);
    render();
    state.pollTimer = window.setTimeout(poll, 1000);
  } catch (error) { notice(`Não foi possível carregar o ambiente: ${errorText(error)}`); render(); }
}
}
initialize();
