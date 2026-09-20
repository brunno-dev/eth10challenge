# ETH Search MCP

Permite que Codex, Claude Desktop ou outro cliente MCP local conduza testes no
ETH Search Studio. A LLM escolhe hipóteses e chama ferramentas; o motor continua
fazendo os cálculos na CPU/GPU. As execuções aparecem na mesma interface e usam o
mesmo histórico automático. Não precisa de chave de API para este servidor.

O servidor usa o SDK oficial MCP e transporte **stdio**. O cliente inicia
`node server.js`; uma conexão autenticada em `127.0.0.1` comunica-se com o desktop.
Não é uma URL de conector remoto. O aplicativo abre na primeira ferramenta que
precisar do motor e deve permanecer aberto. Desconectar a LLM não pausa a busca;
use `eth_pause_search` ou o botão Pausar.

## Conectar neste computador

As dependências e as configurações locais já foram preparadas. Use a versão
atualizada de `desktop/release/ETH Search Studio.exe`.

**Codex:** copie o conteúdo de `codex-config.local.toml` para seu
`%USERPROFILE%\.codex\config.toml`, preservando as outras configurações. Se já
existir `[mcp_servers.eth-search]`, atualize esse bloco em vez de duplicá-lo.
Recarregue os servidores MCP ou reabra o Codex e uma nova conversa.

**Claude Desktop:** em Settings > Developer > Edit Config, adicione a entrada
`eth-search` de `claude-config.local.json` ao objeto `mcpServers` de
`%APPDATA%\Claude\claude_desktop_config.json`. Preserve outros servidores e
configurações. Feche completamente e reabra o Claude Desktop.

Os arquivos gerados contêm caminhos absolutos desta instalação. Se mover a pasta,
execute `node config.js --write` dentro de `mcp` e atualize os clientes. Para uma
instalação nova, use Node.js 22 ou superior e `npm ci` nessa pasta antes disso.

Fontes dos formatos: [MCP no Codex](https://developers.openai.com/codex/mcp),
[servidores locais no Claude Desktop](https://modelcontextprotocol.io/docs/develop/connect-local-servers),
[SDK MCP e stdio](https://ts.sdk.modelcontextprotocol.io/server).

## Pedido pronto para a LLM

> Use o MCP eth-search. Consulte as pistas e o histórico, proponha três hipóteses
> com justificativa e execute até três rodadas de 100.000 candidatos. Verifique
> a cobertura antes de iniciar, mantenha os filtros e acompanhe cada rodada.
> Não repita buscas já cobertas. Ao terminar, informe os IDs, as combinações
> excluídas e se alguma busca ficou incompleta. Pare se encontrar correspondência.

Também existe o prompt MCP `explorar-desafio`, com objetivo e número de rodadas.
O servidor fornece ferramentas; a continuidade depende da conversa/agente do
cliente. Ele não agenda sozinho novas hipóteses quando o cliente encerra o turno.

## Ferramentas

| Ferramenta | Função |
|---|---|
| `eth_get_challenge` | Pistas, ressalvas e filtros externos implementados |
| `eth_get_state` | Busca ativa e histórico paginado |
| `eth_get_run` | Configuração, métricas e log de uma execução |
| `eth_preflight` | Validação e cobertura, sem derivação; importa checkpoints elegíveis |
| `eth_start_search` | Inicia uma rodada; retorna o ID |
| `eth_wait_run` | Aguarda até 30 segundos por um estado final ou parcial |
| `eth_pause_search` | Solicita pausa da busca ativa |
| `eth_resume_search` | Retoma o checkpoint com a configuração original |

Os recursos `eth://history/catalog`, `eth://history/reports`,
`eth://history/rules` e `eth://challenge/evidence` expõem as cópias locais das
evidências. Não atualizam o GitHub. Apenas RO1 está implementado como filtro
externo; os demais relatos permanecem referências.

Cada busca recebe `search` com nome e modo. Em `batches`, use `post` e `video`;
em `template`, use `pattern` com 12 posições e `pool`, além de `fill` se necessário.
Os campos de palavras aceitam espaços; `palavra@N` fixa posição nas duas fontes.
O alvo do desafio, inglês, checksum e GPU automática são os padrões. RO1 é padrão
somente quando compatível com alvo/idioma/checksum. O histórico local é obrigatório.

O limite padrão é 100.000 candidatos por rodada e o máximo aceito pelo adaptador
é 1 bilhão. Limites contam também exclusões. `limited`/`paused` não significam
busca completa. `completed` significa que aquele espaço foi esgotado; `covered`
significa que a busca foi bloqueada antes de executar. Contagens são strings para
preservar inteiros grandes. `fullyCovered=false` pode conter sobreposição parcial;
o filtro continua funcionando durante a execução.

Codex e Claude podem conectar simultaneamente, mas compartilham uma única busca
ativa. Consultar o estado antes de agir ajuda a coordenar as duas conversas.
Muse Spark e DeepSeek poderão usar o mesmo servidor se o aplicativo/agente que
os hospedar suportar MCP local stdio; não há integração específica com essas APIs.

## Operação e desenvolvimento

- `ETH_STUDIO_EXE`: caminho opcional para o executável.
- `ETH_STUDIO_DATA_DIR`: pasta do descriptor de conexão; padrão
  `%LOCALAPPDATA%/dev.ethsearch.studio`.
- `ETH_MCP_AUTO_LAUNCH=0`: exige que o usuário abra o aplicativo.
- `mcp-connection.json` é interno e contém um token temporário. Não é a
  configuração que se copia para os clientes.
- O MCP não permite executar comandos de shell, apagar histórico, selecionar
  arquivos arbitrários ou enviar transações. Resultados/logs consultados são
  fornecidos ao cliente LLM conectado.
- `npm test` valida schemas, transporte MCP e chamadas ao adaptador.
- `node smoke-desktop.js` testa o caminho completo pelo desktop e motor com
  endereço sintético: limite, retomada, sobreposição, bloqueio e pausa. Não o
  execute durante uma busca sua; ele cria execuções identificadas como testes.
- Recompile o desktop com `scripts/build-desktop.ps1 -SkipEngine -Check` após
  mudanças em `desktop/src-tauri/src/mcp_bridge.rs`.

Se houver timeout ao iniciar, consulte `eth_get_state` antes de repetir o pedido:
a busca pode ter sido aceita. Se uma versão antiga já estiver aberta, feche-a
normalmente e abra a nova para disponibilizar a conexão MCP.
