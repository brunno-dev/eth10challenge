# Investigação da cobertura C1 — 2026-09-06

Resultado: **ainda não habilitar exclusão automática**.

Fontes consultadas:

- [PR #13](https://github.com/floflo777/open-crypto-puzzles/pull/13), incluindo
  descrição, comentários e relação de arquivos alterados.
- [Commit do fork submetido ao PR](https://github.com/salikkhann/open-crypto-puzzles/tree/478b5fe1e201c1ee01f278d43b76a0f4ae9b1a1e).
- Árvore do repositório principal e o relato arquivado em `ro1/tested.md`.
- [Notas das pistas](https://github.com/floflo777/open-crypto-puzzles/blob/4b7d48a110d4bfce0dea711f8e7935cd76b202a4/1-big-prizes/guntis-vitolins-metamask-8-6eth/analysis/leads.md).

O PR altera documentação. A árvore examinada do fork não contém um gerador
C1 nem um arquivo com suas listas completas por fonte. O kernel criptográfico
`engines/bip39_passphrase_engine.cu` não define quais candidatos foram enviados.
O comentário do mantenedor trata da integração do relato, sem acrescentar dados.

O relato fornece âncoras, divisão 6/6, exigência de palavras distintas e totais,
mas as palavras de ligação aparecem como exemplos de uma lista maior.
As notas de pistas contêm estimativas anteriores e até descrevem a pista como
aberta; não substituem o manifesto da execução C1 posteriormente concluída.

Para habilitar a exclusão precisamos das duas listas completas usadas na
execução, das regras do gerador e da identificação da versão corrigida do
checksum. Uma reconstrução deve conferir os conjuntos efetivamente cobertos,
não apenas coincidir com o total de 822.640 pares. Diferentes listas podem ter
o mesmo total. Não inferimos cobertura a partir do tamanho ou de exemplos.

Enquanto esses dados não estiverem disponíveis, C1 continua como referência.
Nenhuma mensagem foi enviada aos autores nesta investigação.
