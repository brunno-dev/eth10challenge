# Benchmark do cache de contagens e agrupamento para GPU

Medição em 06/09/2026, Windows, release, NVIDIA GeForce RTX 3060 Laptop GPU.
Três repetições após aquecimento, sem compilação ou selftests concorrentes.
Dados completos em [`packing-results.csv`](packing-results.csv).

| Configuração | Mediana do processo |
| --- | ---: |
| Sem agrupamento e sem cache | 2,653430 s |
| Cache, sem agrupamento | 2,639232 s |
| Agrupamento e cache | 0,582405 s |

O agrupamento reduziu o tempo em aproximadamente 78%, ou 4,56 vezes mais rápido
neste cenário de exclusões intercaladas e lotes pequenos. Todos os casos
percorreram os mesmos 151.200 arranjos e excluíram exatamente 136.080. A diferença
pequena do controle `cache-only` é variação de execução: este cenário não aciona
o cache de contagens. Não se deve extrapolar o ganho para o lote padrão ou buscas
sem histórico.

O microbenchmark [`../benches/pruning_counts.rs`](../benches/pruning_counts.rs)
isola a enumeração com contagens residuais, sem CUDA, leitura de registros,
checksum ou PBKDF2. Usa doze palavras distintas e cobertura sintética a partir da
segunda posição aberta. Cada execução avança 20 milhões de candidatos originais;
os dois modos precisam produzir contadores idênticos de passos e frases geradas.
O cache começa vazio em cada execução. Medianas de cinco repetições alternadas,
após aquecimento, disponíveis em [`count-cache-results.csv`](count-cache-results.csv):

| Limite bruto por lote | Sem cache | Cache de 1.024 estados | Razão dos tempos |
| --- | ---: | ---: | ---: |
| 4.096 | 0,086148 s | 0,017430 s | 4,94× |
| 65.536 | 0,008410 s | 0,004442 s | 1,89× |

Esse ganho se refere somente ao microbenchmark de enumeração e contagem; não é
um multiplicador adicional do ganho da GPU. O cache usa substituição FIFO para
continuar admitindo estados novos sem ultrapassar 1.024 entradas. A chave contém
o prefixo como multiconjunto completo e sua profundidade; clones do mesmo domínio
compartilham o cache, que não é serializado no checkpoint.

Reprodução do microbenchmark: `cargo bench --no-default-features --bench pruning_counts --locked`.
Esse comando também pode substituir o executável de busca por um build CPU no
mesmo diretório de saída. Execute `./scripts/build-windows.ps1` depois para
restaurar o executável CUDA, ou use um `CARGO_TARGET_DIR` separado para o benchmark.

Validação: 44 testes no build CUDA, 41 no build CPU e 20 grupos do selftest GPU.
A reconstrução completa RO1 continua ignorada por padrão. O script
`scripts/test-history-packing.ps1` também verifica retomada CPU → GPU → GPU com
as otimizações desligadas: cursor final idêntico à execução contínua, 120
candidatos originais e 42 exclusões únicas de dois registros sobrepostos.

O script compara três configurações do mesmo executável, com a poda de prefixos
ativa em todas. Exige CUDA; uma queda para CPU interrompe a medição.

```powershell
./scripts/benchmark-history-packing.ps1 -Binary ../gpu-target/release/words-breaker.exe -Repeats 3 -OutputPath packing-results.csv
```

| Caso | Opções adicionais | Recursos ativos |
|---|---|---|
| `previous-behavior` | `--no-pack-history --no-count-cache` | Poda, sem cache nem novo agrupamento |
| `cache-only` | `--no-pack-history` | Poda e cache de contagens |
| `packing-and-cache` | Nenhuma | Poda, cache e agrupamento para GPU |

`previous-behavior` representa os recursos desativados neste executável, não uma
medição de um binário histórico. O cache reutiliza contagens de subárvores; o
agrupamento reúne trabalho remanescente para a GPU após as exclusões. O lote
de 1.024 candidatos foi escolhido para evidenciar o custo de lotes pequenos;
não representa o tamanho padrão nem garante o mesmo ganho em outros tamanhos.

As entradas são sintéticas públicas, com alvo zero, seis posições fixadas em
`abandon` e seis posições abertas. O pool contém dez palavras distintas:
`abandon ability able about above absent absorb abstract absurd abuse`.
São `10! / 4! = 151.200` arranjos. Nove buscas independentes, sem exclusões,
fixam a última posição (12ª) em cada uma das primeiras nove palavras e produzem os
registros usados em todos os casos. Cada registro cobre 15.120 arranjos;
juntos cobrem exatamente 136.080 (90%), deixando 15.120 para processamento normal.

Fixar a última posição mantém as exclusões intercaladas ao longo do domínio.
A prova limitada às três primeiras posições abertas não cobre esses prefixos;
o filtro por frase realiza as exclusões. Assim, este cenário isola o efeito do
agrupamento de candidatos remanescentes em lotes para GPU. `cache-only` permanece
como controle, sem expectativa de ganho: não há contagens de subárvores podadas
para reutilizar aqui. O cache de contagens requer uma medição separada.

Cada execução usa os mesmos nove registros e um checkpoint temporário novo,
sem retomada, para impedir o atalho de cobertura integral. O script verifica
exaustão, 151.200 candidatos originais, 136.080 exclusões na execução e acumuladas,
além de ausência de poda de subárvores neste cenário. O contador de poda conta
somente subárvores puladas sem geração individual, não as frases geradas e
excluídas. Erros ou contadores inesperados interrompem a medição.

A preparação dos registros fica fora do cronômetro. Uma passagem inicial por
todos os casos aquece o driver e é descartada; seguem três repetições por padrão,
com ordem alternada. `Seconds` mede o processo completo, incluindo inicialização,
planejamento, leitura dos registros e gravação do checkpoint. `SearchSeconds`
preserva o tempo arredondado do resumo do executável; não é tempo exclusivo dos
kernels. O CSV também registra backend, tamanho de lote e contadores. Segundos
usam ponto decimal independentemente da configuração regional.

Compare as medianas de cada caso e conserve as repetições individuais para
avaliar variação. A taxa de candidatos originais inclui o espaço excluído e
checksums inválidos, portanto não mede derivações criptográficas por segundo.
Não é uma busca real do desafio.

Registros e checkpoints temporários próprios são removidos ao final. O CSV
solicitado e o cache persistente de compilação CUDA são preservados.
