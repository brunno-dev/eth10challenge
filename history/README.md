# Histórico de buscas

Primeira integração: RO1 do projeto
[floflo777/open-crypto-puzzles](https://github.com/floflo777/open-crypto-puzzles/tree/4b7d48a110d4bfce0dea711f8e7935cd76b202a4/1-big-prizes/guntis-vitolins-metamask-8-6eth).
O catálogo fixa o commit `4b7d48a110d4bfce0dea711f8e7935cd76b202a4` e o SHA-256
de cada evidência. Os arquivos em `ro1/` são cópias sem alteração. Textos e
dados de floflo777 e colaboradores: CC BY 4.0, conforme `ro1/LICENSE`;
script: MIT, conforme `ro1/LICENSE-CODE`. Direitos de citações do autor do
desafio permanecem com seus titulares. Catálogo, documentação e filtro local
foram adicionados nesta integração; não são resultados publicados pelos autores.

## O que pode ser excluído

Somente RO1, mediante `--exclude-tested RO1`. A opção aceita explicitamente
o resultado negativo externo. P1, G1, R1, R2-minus-R1, S1, R1b, RO2 e C1
ficam como referências; seus domínios não foram reconstruídos aqui.
O registro de busca interrompida nunca gera exclusão. A lista completa de
relatos, incluindo buscas menores e testes estatísticos, está em `ro1/tested.md`.
As contagens externas não devem ser somadas como se fossem conjuntos disjuntos.

A regra RO1 exige:

- Endereço `0x9c2f44efad0c1e852a09df9939e6daf061140caf`, inglês, 12 palavras,
  passphrase vazia, caminho `m/44'/60'/0'/0/0`, checksum obrigatório.
- `dutch` na posição 1, `fog` na 5, `parrot` na 12.
- Seis palavras por fonte. Vídeo: duas ou três palavras antes de `fog`,
  respectivamente duas ou uma entre `fog` e `parrot`, em ordem estrita.
- Post: `dutch`, `fiber`, três outras palavras em ordem do post e `fork`
  em qualquer posição restante do post. `fiber` não está fixada na posição 4.
- Listas e ordens exatas de `ro1/reading-order-pool.json`.
  Repetições entre fontes são permitidas; repetições dentro da mesma fonte não.

O filtro RO1 faz uma decisão exata por frase, sem hashes probabilísticos ou Bloom
filter. Não elimina palavras nem todas as permutações de um conjunto. Seu custo
é pequeno, mas uma busca com pouca sobreposição pode ficar mais lenta: o ganho
depende das derivações efetivamente evitadas. A geração das frases continua
existindo nos casos sem prova de cobertura integral; o atalho de histórico local
descrito abaixo pode dispensar a enumeração de todo um domínio.

## Uso e retomada

```powershell
words-breaker --list-history
words-breaker 0x9c2f44efad0c1e852a09df9939e6daf061140caf --pattern "dutch ? ? ? fog ? ? ? ? ? ? parrot" --pool "update winter cattle lake also forest wood fiber fork" --coverage-report
words-breaker 0x9c2f44efad0c1e852a09df9939e6daf061140caf --pattern "dutch ? ? ? fog ? ? ? ? ? ? parrot" --pool "update winter cattle lake also forest wood fiber fork" --exclude-tested RO1 --checkpoint ro1-search.json
```

O relatório não deriva endereços nem inicializa CUDA. Por padrão inspeciona
até 1 milhão de candidatos; `--max-candidates N` muda o limite. Só declara
inspeção completa quando o enumerador termina. Um prefixo não estima o restante.
O exemplo acima contém 362.880 frases, portanto pode ser inspecionado inteiro.

Na busca, `--max-candidates`, índices e cursor continuam contando candidatos
originais, inclusive os excluídos. Lotes totalmente excluídos avançam e salvam
o cursor sem executar kernels. A contagem de exclusões inclui frases de
checksum inválido; não representa o número de PBKDF2 evitados. Em caso de hit,
somente os lotes anteriores inteiramente concluídos ficam no checkpoint.

Checkpoints com exclusão usam versão 3 e vinculam a regra e as evidências por
fingerprint. Mudar/remover a exclusão ou atualizar a evidência impede retomar
o mesmo checkpoint. Os formatos anteriores continuam aceitos sem exclusão.

## Validação local

Em 2026-09-06 reconstruímos integralmente o modelo, sem PBKDF2:

| Forma do vídeo | Arranjos | Checksums válidos |
|---|---:|---:|
| 2 antes / 2 depois de fog | 55.080.000 | 3.445.910 |
| 3 antes / 1 depois de fog | 112.608.000 | 7.039.009 |
| Total | 167.688.000 | 10.484.919 |

As contagens coincidem com o relato externo e todos os arranjos reconstruídos
foram aceitos pelo filtro local. Isso valida o domínio e o checksum, **não**
reproduz o resultado negativo das derivações, nem certifica unicidade global.
O teste também pode contar múltiplas atribuições de fonte da mesma frase.
Um oráculo independente testa todas as partições de fonte em exemplos,
trocas de posição e 10.000 mutações determinísticas para detectar exclusões
indevidas, incluindo palavras compartilhadas e repetidas.

```powershell
cargo test --release --no-default-features --locked
cargo test --release --no-default-features --locked audit_full_ro1 -- --ignored --nocapture
words-breaker --selftest
```

A auditoria integral levou 84,61 s nesta máquina. Não é benchmark do filtro:
inclui geração, alocações de teste e SHA-256 de todos os arranjos. O teste
integral é opt-in; os testes comuns verificam regras, evidências e checkpoints.
O selftest CUDA verifica compactação, índices originais, limites, lotes vazios
após exclusão, cursor e propagação de erros usando alvos sintéticos.

Verificação da integração: 21 testes comuns passaram em builds CPU e CUDA,
Clippy sem avisos e 18 grupos do selftest na RTX 3060 Laptop passaram.
Uma execução limitada na CPU foi retomada na GPU com versão 3, dois candidatos
originais e uma exclusão acumulada. Endereço e política de checksum incompatíveis
foram rejeitados pela CLI. O exemplo de relatório acima identificou exatamente
225 arranjos RO1 entre 362.880 permutações em 0,025 s; é uma inspeção de cobertura,
não uma medição de ganho da busca criptográfica.

O script Python arquivado serve como evidência e não é executado pelo sistema.
Ele preserva caminhos relativos do repositório de origem; use-o no checkout
original para executar seus comandos. Atualizações futuras devem revisar o
domínio e renovar catálogo, hashes e versão da regra antes de habilitar exclusões.

## Buscas locais reutilizáveis

`--record-search arquivo.json` registra uma busca **completamente esgotada e sem
match**. O arquivo inclui o plano de geração, multiplicidades, âncoras, cotas,
idioma, endereço, caminho, passphrase vazia, política de checksum e contagem exata.
Não é um arquivo de frases: seu tamanho depende das listas de entrada.
Um SHA-256 detecta alterações acidentais; ele não é assinatura nem prova de que
um arquivo recebido de terceiros representa uma execução real.

O registro só é publicado após a conclusão e não sobrescreve arquivos existentes.
Publicação usa hard link atômico no mesmo diretório; o filesystem precisa suportar
esse recurso (validado em NTFS). Não há registro negativo em buscas parciais,
erros ou hits. Também não é permitido gerar registros a partir de buscas que
tenham usado exclusões: esta versão mantém os resultados locais independentes.
Uma execução limitada pode terminar depois via checkpoint e então gerar o registro.
Falhas na derivação CPU são propagadas, impedindo certificar erro como negativo.

```powershell
words-breaker ENDERECO --pattern "SEU PADRAO DE 12 POSICOES" --pool "SUAS PALAVRAS" --checkpoint andamento.json --record-search concluida.json
words-breaker ENDERECO --pattern "SEU PADRAO DE 12 POSICOES" --pool "POOL AMPLIADO" --coverage-report --exclude-record concluida.json
words-breaker ENDERECO --pattern "SEU PADRAO DE 12 POSICOES" --pool "POOL AMPLIADO" --exclude-record concluida.json
```

Repita `--exclude-record` para selecionar mais registros. Registros idênticos
são deduplicados; sobreposição entre registros é contada uma única vez por frase.
Cada registro deve corresponder ao alvo, idioma e política de derivação atual.
É possível combinar registros locais e RO1 explicitamente; C1 segue indisponível
para exclusão pelos motivos documentados em [C1-AUDIT.md](C1-AUDIT.md).

A verificação local preserva as regras do enumerador: pool obrigatório antes do
fill quando faltam palavras, repetições, pools maiores que o número de posições,
duas fontes sobrepostas e suas cotas. Não impõe a restrição de palavras distintas
do C1 a outras buscas. Os testes confrontam o filtro com a enumeração exaustiva
em espaços pequenos, incluindo casos sem candidatos.

Além de planos equivalentes e frases totalmente fixadas, o programa prova
inclusão de buscas mais restritas em um registro anterior: pools menores,
posições adicionais fixadas, fills restritos e mudanças entre template e duas
fontes. A prova considera quantidades e cotas, sem enumerar permutações.
As posições que estavam fixadas no registro devem permanecer fixadas. Palavras
fixadas agora em posições antes livres consomem a capacidade do registro.
Sem checkpoint, cobertura integral encerra antes de enumerar ou inicializar a GPU.
Nesse caso `--max-candidates` não limita a prova: nenhum candidato precisa ser
processado. Com checkpoint, o fluxo normal continua contando posições originais
para preservar o cursor. Quando nenhum registro basta sozinho, uma prova
limitada combina a cobertura de vários registros (detalhes abaixo). Quando
não consegue fechar a cobertura integral, o filtro por frase continua disponível.
O enumerador ainda não poda ramos parcialmente cobertos durante a busca.

O fingerprint do checkpoint incorpora os registros selecionados. Trocar um
registro por outro incompatível impede a retomada; a ordem dos arquivos e
duplicatas não alteram o fingerprint. Checkpoints anteriores apenas com RO1
mantêm seu fingerprint. Registros locais de buscas antigas não são inferidos
automaticamente de descrições ou checkpoints incompletos.

Verificação desta etapa: 23 testes comuns passaram em builds CPU e CUDA,
Clippy sem avisos e 18 grupos do selftest CUDA passaram. A CLI também confirmou:
busca parcial sem registro; conclusão por retomada gerando registro; reutilização
com retomada CPU → GPU (24 candidatos, 6 excluídos); e hit sem registro negativo.

## Medição do reaproveitamento local

Benchmark sintético em 2026-09-06, RTX 3060 Laptop, alvo zero e palavras públicas
de teste. Não é uma busca do desafio. Seis posições fixas e seis abertas;
registro de um pool de 10 palavras (151.200 frases), busca ampliada com 11
palavras (332.640 frases). Sobreposição exata: 151.200, ou 45,45% dos arranjos.
Esse percentual inclui checksums inválidos, portanto não equivale a derivações
economizadas. Uma passagem de aquecimento, três repetições e ordem alternada.

| Caso | Mediana de tempo total |
|---|---:|
| Busca ampliada sem exclusões | 0,494586 s |
| Busca ampliada com registro parcial | 0,427780 s |
| Pool original sem exclusões | 0,391998 s |
| Pool original reordenado, cobertura integral comprovada | 0,011747 s |

A redução na busca ampliada foi 13,5% neste exemplo. Tempos incluem iniciar o
processo e, quando usada, inicializar CUDA. Casos abaixo de um segundo têm
variação relevante; não extrapolar esse ganho para outros pools ou máquinas.
O atalho integral economiza também inicialização e enumeração, por isso deve
ser comparado ao pool original, não ao ampliado.

Dados brutos: [local-benchmark.csv](local-benchmark.csv). Reprodução:

```powershell
./scripts/benchmark-history.ps1 -Binary CAMINHO_DO_EXECUTAVEL -Repeats 3 -OutputPath resultado.csv
```

O script cria e remove seu próprio registro temporário, verifica as contagens
esperadas e identifica o backend efetivamente usado. `-Cpu` permite a mesma
comparação na CPU. O cache CUDA deve estar configurado como nas outras medições
do projeto para não misturar recompilação JIT com o custo do filtro.

## Prova de inclusão entre buscas — etapa seguinte

A prova calcula o pior caso de três condições do registro anterior: violação
de limites/obrigatoriedade de cada palavra, contribuição mínima exigida do post
e contribuição mínima exigida do vídeo. Programação dinâmica maximiza cada
condição sobre as quantidades que a busca nova pode selecionar. Só declara
cobertura integral se nenhuma frase possível puder violar o registro.

O cálculo tem até 49 estados de contagem por palavra no modo de duas fontes
(de 0 a 6 palavras selecionadas por fonte). O custo depende do vocabulário e
dessas pequenas cotas, sem crescer com o número de permutações. Palavras
compartilhadas pelas fontes admitem todas as atribuições válidas. No template
com fill, todas as palavras obrigatórias do pool precisam estar presentes.

Exemplo de cuidado necessário: uma busca antiga permite uma ocorrência de uma
palavra. Fixá-la numa posição nova não autoriza continuar selecionando outra
ocorrência do pool; as frases com duas ocorrências podem estar fora da cobertura.
O relatório distingue essa sobreposição parcial e não pula a busca inteira.

Validação: 8.000 pares de planos comparados com conjuntos enumerados integralmente,
além de casos específicos de cotas, repetição entre fontes, palavras obrigatórias
ausentes e consumo de capacidade por posições fixadas. Com âncoras compatíveis,
a prova coincidiu com a inclusão exata dos conjuntos nesses testes. Os 25 testes
comuns passaram em builds CPU e CUDA, com Clippy sem avisos. A retomada de um
subconjunto na CPU → GPU preservou limite e contadores (6 candidatos, 6 excluídos).
Não foi necessário alterar os kernels criptográficos ou o formato dos registros.

Benchmark sintético na mesma RTX 3060 Laptop, alvo zero e registro anterior de
151.200 frases. Uma passagem de aquecimento e três repetições em ordem alternada;
medianas de tempo total, incluindo o processo e a inicialização CUDA quando usada:

| Nova busca coberta | Frases | Sem histórico | Com prova de inclusão |
|---|---:|---:|---:|
| Pool menor | 60.480 | 0,324618 s | 0,010200 s |
| Mais uma posição fixa | 15.120 | 0,334307 s | 0,010526 s |
| Divisão em post e vídeo | 720 | 0,325806 s | 0,010670 s |

A prova dispensou a enumeração e a inicialização GPU nos três casos. Esses
tempos curtos são dominados pela inicialização; não representam aceleração dos
kernels ou ganho aplicável a buscas sem cobertura integral.
Dados: [subset-benchmark.csv](subset-benchmark.csv). Reprodução:

```powershell
./scripts/benchmark-history-subsets.ps1 -Binary CAMINHO_DO_EXECUTAVEL -Repeats 3 -OutputPath resultado.csv
```

Nenhuma opção nova é necessária: `--exclude-record` e `--coverage-report` já
usam a prova de inclusão automaticamente. Registros e checkpoints anteriores
continuam com os mesmos fingerprints, pois o conjunto excluído não mudou.

## Cobertura pela união de registros

Repita `--exclude-record` para que buscas anteriores possam se complementar:

```powershell
words-breaker ENDERECO --pattern "SEU PADRAO DE 12 POSICOES" --pool "SUAS PALAVRAS" --exclude-record parte1.json --exclude-record parte2.json
```

O sistema tenta primeiro provar a cobertura por um registro. Se isso não basta,
escolhe uma posição livre e cria subdomínios fixando suas possíveis palavras.
Prioriza posições fixadas nos registros, pois elas costumam separar diretamente
as partes já executadas. Cada filho precisa ser coberto por algum registro ou
ter sua própria divisão integralmente coberta; nenhum filho é ignorado.

No template, uma palavra ainda presente no pool consome uma ocorrência dele,
mesmo que também pertença ao fill. Em duas fontes, uma palavra compartilhada
pode gerar dois filhos, consumindo cotas distintas. A união dos filhos reproduz
o conjunto do pai, mas pode conter sobreposição. Por isso suas contagens não
são somadas como candidatos distintos. Os totais exibidos vêm do plano original.

A análise tem limites globais: 256 nós (incluindo raiz e filhos pendentes),
4.096 verificações de registros e meta de 250 ms, verificada entre operações.
Uma operação já iniciada pode ultrapassar a meta; não é um prazo de tempo real.
Uma expansão maior que o orçamento é rejeitada antes de copiar seus planos.
Esgotar qualquer limite produz **cobertura não provada**, nunca autorização
para descartar frases. O programa então usa o fluxo normal e seu filtro exato.

Quando a prova é completa e não há checkpoint, a busca encerra sem construir
o fluxo de candidatos nem inicializar a GPU. A prova visita até 256 subdomínios,
que em casos pequenos podem chegar a frases totalmente fixadas. Com checkpoint,
continuam valendo a enumeração, os limites e os cursores originais. Não houve
alteração no formato dos registros, nos fingerprints ou nos kernels.

Um agente fez revisão matemática independente e implementou testes da união
dos filhos contra a enumeração completa dos pais, em todas as posições livres.
Os testes cobrem fill, repetições, origens compartilhadas, cotas, vários níveis,
orçamento insuficiente/exato, planos inválidos e remoção de um trecho da cobertura.
Passaram 31 testes comuns nas compilações CPU e CUDA, e Clippy sem avisos.

Benchmark sintético na RTX 3060 Laptop: dez buscas locais disjuntas, cada uma
com a sétima posição fixada em uma palavra diferente, cobrem as 151.200 frases
do pool original. Cada registro tem 15.120 frases. Alvo zero, uma passagem de
aquecimento, três repetições alternadas; tempo total inclui inicialização:

| Caso | Mediana | Comportamento |
|---|---:|---|
| Sem histórico | 0,404128 s | Busca CUDA completa |
| Dez registros, união completa | 0,012905 s | Fluxo e GPU dispensados |
| Um registro removido | 0,346038 s | 136.080 excluídos; restante processado |

O ganho no caso integral inclui evitar inicialização e geração; não é aceleração
dos cálculos criptográficos e não se aplica a uma união incompleta. Dados:
[union-benchmark.csv](union-benchmark.csv). Reprodução:

```powershell
./scripts/benchmark-history-union.ps1 -Binary CAMINHO_DO_EXECUTAVEL -Repeats 3 -OutputPath resultado.csv
```
