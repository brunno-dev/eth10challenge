# Lotes adaptativos e métricas de execução

Medição em 06/09/2026, Windows, release e NVIDIA GeForce RTX 3060 Laptop GPU.
Após aquecimento, três repetições alternadas produziram estas medianas:

| Modo | Tempo do processo | Lotes enviados | Tamanhos solicitados |
| --- | ---: | ---: | --- |
| Fixo | 0,347248 s | 3 | 65.536 |
| Adaptativo | 0,392026 s | 7 | 16.384 a 65.536 |

O modo adaptativo foi aproximadamente 12,9% mais lento nesta busca curta.
Começar com lotes menores adicionou lançamentos antes de alcançar o teto;
ocorreram duas mudanças de tamanho. Por isso o lote fixo continua como padrão.
Os dois modos concluíram os mesmos 151.200 candidatos e 9.409 derivações.
O controlador regula latência, sem provar que a configuração resultante maximiza
vazão. Os dados completos estão em [`adaptive-results.csv`](adaptive-results.csv).

As métricas separaram, no modo fixo, aproximadamente 3,4 ms de produção dos
candidatos e 87,1 ms na fase de derivação (medianas). O tempo total também inclui
inicialização, alocações e outras operações fora dessas fases. Não se devem somar
os intervalos sobrepostos para reconstruir esse total.

Validação: 54 testes no build CUDA, 51 no build CPU e 21 grupos do selftest GPU
passaram; a reconstrução completa RO1 permanece ignorada por padrão. O teste CLI
de retomada confirma que os relatórios de CPU e GPU somam os mesmos contadores
da execução contínua. Também foram verificados o índice de um resultado conhecido,
a exclusão do lote com resultado das métricas, os limites e a preservação de
arquivos de relatório existentes.

`--adaptive-batch` habilita o ajuste de lotes na GPU. A opção é voluntária; sem
ela, o tamanho solicitado permanece fixo. `--batch-size` define o teto, inclusive
das alocações limitadas por esse tamanho. O controlador começa em
`min(16384, teto)` e mantém piso `min(256, teto)`. Ajustar o lote não aumenta o
limite de memória definido pelo teto.

O alvo de latência é 250 ms, com uma faixa de tolerância:

- Dois lotes completos consecutivos abaixo de 125 ms permitem dobrar o tamanho,
  respeitando o teto.
- Um lote acima de 500 ms reduz o tamanho pela metade, respeitando o piso. Um
  lote parcial lento também permite redução.
- Lotes parciais rápidos não autorizam crescimento. Amostras vazias, de duração
  zero ou de lotes antecipados que solicitaram um tamanho já substituído são
  ignoradas pelo controlador.

O ajuste responde ao tempo observado; não escolhe antecipadamente a melhor
configuração para cada placa. Execuções curtas podem ficar mais lentas porque
começam com lotes menores. Não há garantia de ganho nem exigência de que o
controlador altere o tamanho em toda execução.

## Consultar métricas

```powershell
words-breaker ENDERECO --pattern "PADRAO DE 12 POSICOES" --pool "PALAVRAS" --adaptive-batch --metrics --metrics-json nova-execucao.json
```

`--metrics` apresenta um resumo no console. `--metrics-json` grava as métricas
na terminação normal e exige um caminho novo; um arquivo existente não é
sobrescrito. O relatório contém somente agregados, sem frases, palavras do pool
ou endereço alvo. Erros ou encerramento abrupto não garantem um relatório final.

Os contadores representam apenas lotes concluídos. Um lote contendo um resultado
positivo fica fora desses agregados, seguindo o limite de trabalho confirmado
pelo checkpoint. Ao retomar uma busca, as métricas contam somente a nova
execução; não são os totais acumulados do checkpoint. Um encerramento por prova
de cobertura integral registra o backend `coverage-proof`, com zero trabalho
executado e sem inicializar CPU/GPU para a busca.

| Campo | Interpretação |
|---|---|
| `completed_raw` | Candidatos originais de lotes concluídos, incluindo exclusões |
| `excluded`, `retained` | Candidatos excluídos pelo histórico e restantes; somam `completed_raw` |
| `pruned` | Parte das exclusões pulada por subárvores sem geração individual |
| `checksum_survivors` | Candidatos que passaram pela política de checksum |
| `completed_batches`, `device_batches` | Lotes concluídos e lotes efetivamente enviados ao dispositivo |
| `producer_seconds`, `wait_seconds` | Tempo do produtor e espera por trabalho |
| `transfer_seconds`, `filter_seconds`, `derive_seconds` | Intervalos observados nas transferências, filtro e derivação |
| `checkpoint_seconds` | Tempo de atualização e gravação do progresso |
| `min_batch_size`, `max_batch_size` | Menor e maior tamanho solicitado dos lotes concluídos; podem ser nulos sem lotes |
| `adaptive_changes` | Alterações de tamanho realizadas pelo controlador |

Os tempos são medidos pelo relógio do host e incluem sincronização onde
necessária; não são tempos exclusivos de kernels medidos por eventos CUDA.
O produtor trabalha em paralelo com a GPU, portanto os intervalos podem se
sobrepor e não devem ser somados para reconstruir o tempo total. Na CPU, o
intervalo de derivação inclui checksum e derivação juntos. Com `--no-checksum`,
o contador de sobreviventes segue a política de aceitar os candidatos retidos.

## Benchmark reproduzível

```powershell
./scripts/benchmark-adaptive-batches.ps1 -Binary ../gpu-target/release/words-breaker.exe -Repeats 3 -OutputPath adaptive-results.csv
```

O script compara lote fixo de 65.536 com modo adaptativo sob o mesmo teto,
começando em 16.384. Usa alvo zero, seis posições fixadas em `abandon`, seis
abertas e o pool público de dez palavras
`abandon ability able about above absent absorb abstract absurd abuse`.
São 151.200 candidatos originais, sem histórico, exclusões ou checkpoints.

Uma passagem por ambos os modos aquece o driver e é descartada; seguem três
repetições por padrão, em ordem alternada. CUDA é obrigatório: fallback para
CPU interrompe o benchmark. O script valida 151.200 candidatos concluídos e
retidos, zero exclusões, o mesmo número de sobreviventes em todas as execuções
e lotes que não excedam 65.536. Não exige alterações adaptativas, pois o tempo
do hardware e a duração da busca determinam se elas ocorrem.

O CSV registra o tempo do processo completo em `Seconds`, além dos contadores
e intervalos do JSON. Valores de tempo usam ponto decimal independentemente da
configuração regional. Compare medianas sem descartar a variação das repetições.
Os JSONs temporários próprios são removidos ao final; o CSV solicitado e o
cache persistente de compilação CUDA permanecem.
