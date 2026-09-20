# Histórico: perfil e otimização — 19/09/2026

O produtor da GPU consultava cada registro local para cada candidato. A consulta
reconstruía multiplicidades antes de rejeitar palavras ausentes ou frases além
da fronteira de uma execução parcial. A medição separada mostrou que o custo
dominante dessa preparação era o histórico, não a enumeração.

## Alterações

- Rejeição antecipada por fronteira parcial e por palavra ausente no domínio.
  Uma frase anterior à fronteira ainda precisa passar pelas cotas de origem,
  multiplicidades e posições fixas.
- Índice por palavra/posição com máscaras de 64 registros. A interseção seleciona
  apenas registros possíveis; `Domain::contains` continua sendo a decisão exata.
  O índice é descartável, compartilhado por `Arc`, não muda o fingerprint, usa
  no máximo 6 MiB e tem fallback linear para menos de 8 ou mais de 2.048 registros.
- Prova conservadora de subárvores cobertas por registros parciais. Exige prefixo
  fixado estritamente anterior à fronteira na ordem original **e** cobertura
  estrutural integral. Uma posição anterior aberta deixa a prova inconclusiva.

Não mudaram a ordem de enumeração, os formatos persistidos, as políticas de
checksum, a derivação da carteira ou a regra externa RO1. A mesma otimização
de associação ao histórico atende os motores CPU e CUDA. Não foi necessário
adicionar threads de geração para obter os ganhos medidos.

## Perfil isolado da CPU

Benchmark `benches/history_filter.rs`, sem derivação de carteiras e sem gravação
de negativos. Geração e consultas são cronometradas separadamente, com os
candidatos já materializados durante a medição do filtro. Isso mede o custo
das etapas, não simula a memória usada pelo motor, que continua trabalhando em
lotes. Uma passagem de aquecimento e três amostras por cenário, mediana abaixo.

Histórico congelado: 125 arquivos do teste anterior, 114 compatíveis com inglês,
alvo do desafio e checksum obrigatório, mais RO1. Nenhum filtro foi removido.

Post: `dutch@1 fiber fork dinner cloud live`. Vídeo: `fog@5 parrot@12`, quatro
antigas (`fiber wood winter rib`) e o grupo indicado. Primeiros 3.000.000 de
candidatos de cada domínio, incluindo trabalho coberto e não coberto.

| Grupo | Geração antes/depois | Histórico antes/depois | Ganho só no filtro | Excluídos, idênticos |
| --- | ---: | ---: | ---: | ---: |
| Cinco moedas + antigas | 0,148 / 0,150 s | 8,108 / 0,127 s | 63,7× | 1.092.088 |
| Tela + fala + antigas | 0,084 / 0,085 s | 7,729 / 0,093 s | 83,0× | 1.000.042 |

Além dos totais, o SHA-256 de **todas as decisões ordenadas** de exclusão foi
idêntico antes/depois: `31e74ccb111d7fa342bf0c8a5d838ca884b7f0755fc1777556752575798c49c5`
(moedas) e `7ca1c68ad80a9e797cc550b2ed0d03251cfd0685c1519625addaa180e730d9f2`
(união). O fingerprint das evidências permaneceu
`coverage-v1:23abb3bb8db33933319505734b2d68efd79a3b302c63ac825187e7524dba3dd1`.

Esses multiplicadores são do filtro isolado, não da busca inteira nem uma
promessa para outros históricos. Registros muito sobrepostos podem deixar
mais verificações exatas após o índice.

## Busca completa com CUDA

RTX 3060 Laptop, batch 65.536, mesma cópia dos filtros e primeiros 5.000.000 de
candidatos. Três execuções de cada versão, ordem antes/depois alternada. Mediana
de tempo de processo, incluindo inicialização, checkpoints e registro negativo:

| Grupo | Antes | Depois | Ganho total | Candidatos/s depois |
| --- | ---: | ---: | ---: | ---: |
| Cinco moedas + antigas | 42,584 s | 2,712 s | 15,70× | 1.843.438 |
| Tela + fala + antigas | 12,743 s | 2,607 s | 4,89× | 1.918.239 |

Ambas as versões terminaram as amostras sem match, com checkpoints completos
idênticos e digests iguais nos registros negativos. Contagens conservadas:
1.195.600 excluídos / 237.551 sobreviventes do checksum (moedas) e
1.000.084 excluídos / 249.993 sobreviventes (união).

No motor otimizado, a preparação dos lotes consumiu cerca de 0,4 s (moedas) e
0,24 s (união), enquanto a derivação CUDA consumiu cerca de 2,1 s. Essas etapas
se sobrepõem; não se somam ao tempo de processo. A alimentação da GPU deixou de
ser o gargalo dominante nessas amostras. Nenhuma subárvore foi podada nessas
execuções: o ganho medido veio da consulta ao histórico; a extensão de poda
parcial foi verificada separadamente por enumeração exaustiva de domínios pequenos.

Esta é uma medição curta, específica da máquina, configuração e distribuição
dos filtros. Não comprova uma taxa sustentada por dias, não permite extrapolar
o multiplicador de moedas para a união, nem garante conclusão dentro de um
orçamento de nuvem. Os domínios grandes não foram esgotados.

SHA-256 dos executáveis medidos:

- Antes: `6be0547f20025347223e948d730aa600c57929fc74476c984ef87073f1f7e8b2`.
- Depois: `5220d7f7ea42915b2b461c3aed095da2eb7711c17b85ba97e8f07bc81ec92a39`.

## Validação

- Clippy com `-D warnings`, incluindo os benchmarks.
- 67 testes passando no build CUDA e 64 no CPU; a reconstrução integral RO1
  de 167 milhões de arranjos permanece explicitamente ignorada em ambos.
- Todos os selftests físicos da GPU passaram, incluindo primitivas
  criptográficas, resultado conhecido de sucesso, exclusões, pausa e retomada.
- Retomada real: checkpoint de 5.000.000 criado pelo executável antigo,
  continuado por mais 12.345 candidatos em cada versão. Ambos produziram
  exatamente o mesmo checkpoint de 5.012.345 candidatos.
- Novos oráculos de associação ao histórico cobrem modos Template/Fill/Batches,
  repetições, fontes compartilhadas, posições fixas e todos os lados de limites
  de 64 registros. Fronteiras parciais continuam exclusivas.
- Os dados e digests existentes continuam válidos; não há migração de histórico.

O negativo confirmado dos primeiros 5.000.000 da união foi incorporado ao
histórico automático do aplicativo, com digest
`ab0d7d3a7dfec4ff1ef822ad68352a23db14d0dd7c2bcab1b67b3594b566e842`.
O grupo de moedas já tinha cobertura integral no histórico. Benchmarks repetem
trabalho deliberadamente para comparar versões; as buscas normais continuam
reaproveitando a cobertura confirmada.

## Reprodução

```powershell
cargo bench --no-default-features --bench history_filter -- --history <copia-do-historico> --case coins --limit 3000000 --trials 3
```

Para comparar dois executáveis CUDA usando o mesmo histórico congelado:

```powershell
./scripts/benchmark-history.ps1 -Before <motor-antigo.exe> -After <motor-novo.exe> -History <copia-do-historico> -OutputDirectory <pasta-nova>
```

O segundo script alterna a ordem dos executáveis, grava métricas/checkpoints e
negativos isolados, verifica igualdade dos checkpoints e dos digests dos
negativos, exige CUDA e interrompe se houver match/erro. Não altera o histórico
do aplicativo automaticamente. Não execute simultaneamente com outras buscas
ou compilações ao medir desempenho.

Resultados locais detalhados e cópias dos executáveis ficam em
`output/import-check/host-optimization/` (ignorado pelo Git).
