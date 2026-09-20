# Benchmark da poda por prefixos

Medição em 06/09/2026, Windows, build release e NVIDIA GeForce RTX 3060 Laptop
GPU, com três repetições após aquecimento. Os dados estão em
[`pruning-results.csv`](pruning-results.csv).

| Modo | Mediana do processo | Exclusões | Arranjos não gerados |
| --- | ---: | ---: | ---: |
| Filtro por frase | 0,331109 s | 136.080 | 0 |
| Poda por prefixos | 0,323302 s | 136.080 | 136.070 |

A redução de tempo foi de aproximadamente 2,4% neste cenário curto. Evitar quase
90% da geração não reduz na mesma proporção o tempo total: ambos os modos já
evitam as mesmas derivações, e ainda há inicialização CUDA, leitura dos registros,
checkpoint e processamento dos candidatos restantes. Dez frases cobertas foram
geradas nas fronteiras dos lotes e descartadas pelo filtro normal. Esta amostra
pequena não demonstra ganho universal; o CSV permite examinar a variação entre
execuções.

Validação da implementação: 38 testes passaram em cada build (CPU e CUDA), mais
19 grupos do selftest CUDA. A auditoria completa RO1 permanece como teste
explicitamente ignorado por padrão. Uma verificação CLI adicional percorreu 13
candidatos em CPU, retomou mais 17 em GPU e terminou com a poda desligada: o
checkpoint final coincidiu com a execução contínua sem poda, incluindo o cursor,
120 candidatos originais e 24 exclusões.

O script compara a exclusão por frase com a poda de prefixos, mantendo os mesmos
registros e candidatos. Use um executável compilado com suporte à opção
`--no-prune-history`:

```powershell
./scripts/benchmark-history-pruning.ps1 -Binary ../gpu-target/release/words-breaker.exe -Repeats 3 -OutputPath pruning-results.csv
```

A poda é automática na CPU e na GPU para prefixos comprovadamente cobertos por
registros locais. Examina até as três primeiras posições originalmente abertas,
com no máximo 256 planos construídos e 4.096 verificações de registros. O limite
de tempo de 250 ms é flexível: é verificado entre operações, sem interromper uma
operação já iniciada. Prefixos com origens post/vídeo sobrepostas exigem cobertura
de todas as atribuições possíveis. RO1 continua usando o filtro exato por frase.
Se a prova não fecha ou o orçamento acaba, os ramos não comprovados seguem pela
geração e filtragem normais.

São entradas sintéticas públicas, com endereço alvo zero. As seis primeiras
posições contêm `abandon`; as seis restantes usam um pool de dez palavras
distintas. O domínio contém `10! / 4! = 151.200` arranjos.

O script conclui nove buscas independentes para criar os registros, sem exclusões.
Cada registro fixa a sétima posição em uma das primeiras nove palavras do pool
e permuta cinco das nove palavras restantes: `9! / 4! = 15.120` arranjos.
Os registros são disjuntos pela sétima posição e cobrem 136.080 arranjos (90%).
Os 15.120 restantes continuam sujeitos à verificação normal.

Os dois modos usam os mesmos nove `--exclude-record`. `phrase-filter-only`
acrescenta `--no-prune-history`; `prefix-pruning` usa a poda automática.
Cada execução recebe um checkpoint novo, impedindo o atalho de cobertura integral
e incluindo o custo de gravação no tempo medido. Não há retomada entre medições.
O formato do checkpoint, o cursor, os índices e os limites de candidatos originais
são preservados pela poda. Fora do benchmark, é possível alternar
`--no-prune-history` ao retomar o mesmo checkpoint e os mesmos registros.

A passagem inicial aquece ambos os modos e é descartada. As três repetições
padrão alternam a ordem. O cronômetro mede o processo inteiro, incluindo leitura
dos registros, planejamento, inicialização do backend e checkpoint. A preparação
dos nove registros fica fora das medições. O CSV registra o backend efetivamente
utilizado (`CUDA` ou `CPU`) e segundos com ponto decimal, independentemente da
configuração regional. Compare medianas de execuções do mesmo backend.

Cada execução precisa esgotar o domínio e informar exatamente 136.080 exclusões;
falhas interrompem o script. Exclusões incluem checksums inválidos, portanto o
percentual de cobertura não mede diretamente derivações PBKDF2 evitadas. Não há
medição de busca real do desafio nem garantia de ganho em outros domínios.

A mensagem `Pruned ... without generation this run` conta somente os arranjos
pulados como subárvores, sem geração individual. Folhas já geradas e depois
excluídas não entram nesse contador, mas entram no total `History excluded`.
Assim, o total de exclusões precisa coincidir entre os modos, enquanto o contador
de poda depende dos prefixos que puderam ser comprovados e pulados.

Os registros e checkpoints temporários próprios são removidos ao final. O CSV
solicitado e o cache persistente de compilação CUDA são preservados.
