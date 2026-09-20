# Pistas do vídeo — revisão de 19/09/2026

O arquivo `video-onscreen-words.json` é uma cópia sem alterações do
[commit 740c58b](https://github.com/floflo777/open-crypto-puzzles/commit/740c58b027061090136b9145b6c079734ed58602),
publicado em 07/09/2026. `video-catalog.json` registra origem, licença e SHA-256.
Créditos: cjmcdaniel e colaboradores de floflo777/open-crypto-puzzles;
CC BY 4.0, conforme [licença arquivada](../ro1/LICENSE).

São 109 palavras observadas na tela e 110 nas legendas automáticas: 192
distintas na união, com 27 compartilhadas. As cinco candidatas destacadas são
`atom link basic token dash`. São ocorrências no vídeo, não palavras confirmadas
da frase. A pesquisa foi relatada na [issue 18](https://github.com/floflo777/open-crypto-puzzles/issues/18).
Até a revisão, não havia resultado negativo concluído para a extensão proposta.

O aplicativo oferece quatro grupos e adiciona suas palavras ao campo do vídeo,
sem duplicar novas entradas nem remover posições e multiplicidades existentes.
Cada palavra adicionada é opcional quando sobram candidatas. O usuário ainda
define a lista do post; a aplicação não inventa as palavras que faltam.
Esses atalhos usam o modo post/vídeo existente, não reproduzem a proposta de
geração em ordem de leitura com exatamente uma ou duas palavras de moedas.
Nenhuma busca é disparada ao aplicar um grupo. Nada disso altera o filtro RO1
ou passa a ser uma exclusão negativa.

Em 09/09, o [commit 789119f](https://github.com/floflo777/open-crypto-puzzles/commit/789119f46265ea5342092dd3af3f6dee1b9e4ab1)
alterou o oracle para derivar frases sem checksum válido. Nosso motor já tinha
`--no-checksum`; esta integração torna a política visível no formulário,
histórico, monitor e fila. O botão de preparação cria uma nova configuração
com o mesmo domínio, desativa RO1 e remove registros manuais da política antiga.
Não altera execuções ou checkpoints anteriores. O histórico automático continua
selecionando apenas registros com a mesma política de checksum.

A pesquisa não prova que a solução Guntis tenha checksum inválido. O modo normal
continua como padrão. RO1 é desativado no frontend, na validação nativa e no MCP
quando o modo ampliado é solicitado. As listas e a ressalva também estão
disponíveis no MCP por `eth_get_challenge`.

Validação: 24 testes nativos, Clippy sem avisos e 16 testes do MCP/dados passaram.
O teste de interface em `scripts/test-video-research-ui.js` preservou posições
e multiplicidades ao aplicar os grupos, confirmou a união de 192 palavras e
executou uma frase sintética de checksum inválido: a busca normal esgotou sem
match; a preparação ampliada encontrou o endereço de referência independente.
O resultado original foi preservado e o hit não foi exportado como negativo.
