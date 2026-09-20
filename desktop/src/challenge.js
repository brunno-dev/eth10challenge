import words from '../../history/research/video-onscreen-words.json' with { type: 'json' };
import provenance from '../../history/research/video-catalog.json' with { type: 'json' };

export const videoResearch = { ...provenance, words };
export const videoGroups = [
  { id: 'coins', label: 'Moedas na tela', words: words.coin_words_from_the_portfolio_table, detail: 'Cinco candidatas destacadas na pesquisa. Comece por este grupo menor.' },
  { id: 'onscreen', label: 'Vistas na tela', words: words.onscreen_not_in_pool, detail: '109 palavras ausentes do pool escrito anterior. Incluem textos da interface do navegador.' },
  { id: 'spoken', label: 'Faladas no vídeo', words: words.spoken_not_in_pool, detail: '110 palavras das legendas automáticas, ausentes do pool escrito anterior.' },
  { id: 'combined', label: 'Tela + fala', words: [...new Set([...words.onscreen_not_in_pool, ...words.spoken_not_in_pool])].sort(), detail: '192 palavras distintas. A união remove as 27 palavras presentes nos dois canais.' },
];

// Preserve deliberate multiplicities and pins in the user's input. Only skip
// new additions whose word already appears, pinned or unpinned.
export function appendVideoWords(input, additions) {
  const tokens = input.trim().split(/\s+/).filter(Boolean);
  const seen = new Set(tokens.map(token => token.toLowerCase().split('@')[0]));
  const added = [];
  for (const word of additions) {
    if (!seen.has(word)) { seen.add(word); added.push(word); }
  }
  return { text: [...tokens, ...added].join(' '), added: added.length };
}

export function expandedSearch(config) {
  return { ...config, name: `${(config.name || 'Busca').slice(0, 70)} · sem checksum`, noChecksum: true, excludeRo1: false, excludeRecords: [] };
}

export const checksumLabel = config => config?.noChecksum ? 'Todas as frases · sem filtro de checksum' : 'Somente checksum válido';
