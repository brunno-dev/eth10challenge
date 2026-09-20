# Words Breaker (Ethereum)

A command-line tool that attempts to recover a BIP-39 mnemonic seed phrase by testing permutations of 12 known words against a target Ethereum address.

## MCP para LLMs

O servidor local em [`mcp/`](mcp/README.md) conecta Codex, Claude Desktop e outros
clientes MCP ao aplicativo. Permite consultar pistas, verificar cobertura,
iniciar/pausar/retomar buscas e acompanhar resultados usando o mesmo histórico
automático da interface. Veja o guia para os arquivos de configuração prontos.

## Aplicativo Windows — ETH Search Studio

### Pistas do vídeo e política de checksum

**Pistas novas do vídeo** oferece quatro grupos: moedas (5), tela (109), fala
(110) e união (192 palavras distintas). **Adicionar ao vídeo** preserva o post,
as posições e as repetições existentes, sem adicionar a mesma candidata novamente.
São hipóteses do commit `740c58b`, não palavras confirmadas nem novos filtros.
Veja [origem e atribuição](history/research/README.md).

A política aparece no formulário, histórico, monitor e fila. **Preparar esta
busca sem checksum**, disponível para execuções concluídas ou cobertas, prepara
uma nova execução com o mesmo domínio. O usuário precisa clicar em **Iniciar**.
Em buscas de arquivo, o atalho se refere à linha selecionada. RO1 é desativado
nesse modo; o histórico automático só reaproveita a mesma política, e registros
manuais da execução anterior são removidos na preparação. O modo normal continua
como padrão. A mudança do oracle externo já era suportada pelo motor local.

### Importar listas de hipóteses

A aba **Importar arquivo** aceita `.txt`, `.doc` (Word 97–2003) e `.docx`,
até 10 MB e 100.000 linhas preenchidas. Use exatamente 12 palavras BIP-39 em inglês
por linha; no Word, separe listas com Enter. Espaços, tabulações, vírgulas e
ponto e vírgula separam as palavras. TXT aceita UTF-8 e UTF-16 com BOM.

Esse modo fixa **dutch@1, fog@5 e parrot@12 no backend**. As palavras devem
estar presentes em cada linha; o importador remove uma ocorrência de cada uma
do conjunto e permuta as outras nove, preservando repetições. Não divide a lista
artificialmente em post/vídeo, nem infere a origem pela ordem no arquivo.
Essas posições são a regra solicitada para a importação, não prova adicional
das pistas. Outras hipóteses continuam disponíveis nos modos manuais.

A prévia mostra erros e duplicatas (mesmo multiconjunto em outra ordem).
**Iniciar fila** executa as linhas válidas em sequência, reaproveita o histórico
e ignora linhas totalmente cobertas. Limites são retomados automaticamente até
esgotar a linha; pausa, erro e acerto param a fila. A fila é persistida em
`%LOCALAPPDATA%/dev.ethsearch.studio/file-queue.json` e reabre pausada.
Use **Continuar fila** para retomar ou **Encerrar fila** para liberar novas buscas
sem apagar os checkpoints e negativos. A fila também reserva o motor entre linhas
para evitar competição com comandos manuais/MCP.

Exemplo de formato: [`desktop/exemplo-lista.txt`](desktop/exemplo-lista.txt).
Documentos protegidos, corrompidos ou de formatos antigos não reconhecidos devem
ser salvos como TXT. A leitura Word usa [rwml](https://docs.rs/rwml/0.1.4/rwml/),
sem exigir instalação do Word ou LibreOffice.

A interface Tauri está em `desktop/`. Depois de gerar o pacote, abra
`desktop/release/ETH Search Studio.exe` com dois cliques, mantendo a pasta
`engine` ao lado do executável. Para copiar para outro computador, extraia
todo o conteúdo de `desktop/ETH-Search-Studio-Windows.zip`.

Na tela você pode configurar palavras e posições ou listas do post/vídeo,
escolher CPU ou GPU automática, iniciar, pausar e continuar uma execução.
O monitor mostra contadores, velocidade, mensagens e métricas do motor.
Os controles avançados incluem limites, idioma, checksum e exclusão de
tentativas anteriores. O exemplo usa um endereço zero e palavras sintéticas.

Novas buscas abrem em Post + vídeo com o endereço do desafio Guntis Vitolins
(`0x9c2f44efad0c1e852a09df9939e6daf061140caf`), post `dutch@1 fiber fork`
e vídeo `fog@5 parrot@12`. Complete as listas com pelo menos seis palavras
por fonte antes de iniciar. `fiber` e `fork` não têm posição confirmada;
palavras sem `@` são candidatas, não uma obrigação de inclusão em conjuntos
com palavras extras. `fog` é a interpretação adotada no README do desafio;
o arquivo de posições também registra `cloud` como alternativa da pista 5.
Fonte: [pistas e análise do desafio](https://github.com/floflo777/open-crypto-puzzles/tree/main/1-big-prizes/guntis-vitolins-metamask-8-6eth).

O **histórico automático** fica em `%LOCALAPPDATA%/dev.ethsearch.studio/history`.
Cada término negativo confirmado (conclusão, pausa ou limite) gera um registro
compacto da cobertura. A parte ainda não processada nunca é marcada como testada.
Antes de uma nova busca, o app importa checkpoints antigos com término negativo
confirmado e verifica a cobertura: bloqueia buscas integralmente conhecidas e
filtra as sequências já avaliadas nas buscas com sobreposição parcial. Liberar
uma posição fixa amplia a busca, mas preserva a exclusão das sequências antigas.
Alterar apenas a ordem de digitação das candidatas não torna a busca nova.

Os registros são separados por endereço, idioma, checksum e política de derivação.
Não são listas de palavras proibidas: a mesma palavra continua disponível em
sequências ainda não cobertas. Resultados encontrados, falhas e sessões de resultado
incerto não são promovidos automaticamente a evidência negativa. Na retomada,
o checkpoint mantém os filtros originais e também recebe novos negativos
compatíveis. RO1 fica selecionado por padrão nas novas buscas do desafio; os
outros relatos do catálogo não viram exclusões sem um modelo validado.

Validação ponta a ponta do histórico (motor já compilado):

```powershell
./scripts/test-auto-history.ps1
```

As execuções ficam em `%LOCALAPPDATA%/dev.ethsearch.studio/runs`, com
configuração, checkpoint e métricas. A pausa aguarda o lote atual terminar;
fechar a janela durante uma busca também solicita essa pausa e aguarda o
salvamento. Retomar usa a configuração original, e o limite de candidatos
se aplica a cada retomada. O aplicativo usa WebView2, normalmente já presente
no Windows; o pacote portátil requer esse runtime instalado.

Para compilar, instale Node.js, Rust, Visual Studio C++ Build Tools e prepare
CUDA conforme as instruções abaixo. Na raiz do projeto:

```powershell
./scripts/build-desktop.ps1 -Check
```

O script valida o código, compila o motor e a interface e gera a pasta portátil
e o ZIP. `-SkipEngine` reutiliza `desktop/src-tauri/engine/words-breaker.exe`.
Para desenvolvimento da tela, use `npm run dev` em `desktop`; a prévia no
navegador não executa buscas. Para desenvolvimento nativo, use
`npm run tauri -- dev` com a prévia ativa e o motor preparado.

## Use Case

If you have 12 BIP-39 mnemonic words but don't remember the correct order, this tool will brute-force permutations to find the combination that derives to your known Ethereum address.

Addresses are derived at **`m/44'/60'/0'/0/0`** — the first account of the default
Ethereum wallet (MetaMask, Ledger Live, Trust, Rabby, ...) — with no BIP-39
passphrase. If your wallet used a passphrase or a non-default account index,
this tool will not find it as-is.

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (current stable; validated with 1.98)
- For the default CUDA build: an **NVIDIA GPU** and the **CUDA toolkit** (`nvcc`).
  CPU-only builds use `--no-default-features` and need neither. The default build
  compiles `src/cuda/kernels.cu` to PTX. Failure to initialize CUDA falls back to
  CPU; a failure during the search is reported without restarting it.

### CUDA library path

`cust` locates the CUDA driver library via `CUDA_LIBRARY_PATH`, which must point at
a CUDA root containing `lib64/` (Windows: `lib/x64/`) and `include/cuda.h`. On a distro-packaged CUDA
install (`nvcc` in `/usr/bin`) that is `/usr/lib/cuda`, and `.cargo/config.toml`
already sets it. For a standard toolkit install, export your root, e.g.:

```bash
export CUDA_LIBRARY_PATH=/usr/local/cuda
```

## Building

### Windows (CUDA or CPU)

Install Visual Studio 2022 C++ Build Tools and an NVIDIA driver. The following
scripts download a pinned CUDA 12.8 compiler/header package from NVIDIA, verify
its SHA-256, and build using MSVC. The toolkit stays in the ignored `.cuda/`
directory; the scripts do not replace the driver or change system settings.

```powershell
./scripts/setup-cuda.ps1
./scripts/build-windows.ps1 -Selftest
```

Alternatively, build with an installed toolkit from an x64 Native Tools command
prompt, setting `CUDA_PATH`, `CUDA_LIBRARY_PATH` and `NVCC` to that installation.
Field arithmetic uses explicit 64-bit carries and CUDA `__umul64hi`, so native
Windows builds no longer need the host compiler's `unsigned __int128` extension.

For CPU only:

```powershell
cargo build --release --no-default-features
```

The binary will be located at `target\release\words-breaker.exe`.

`-Check` also runs Clippy and unit tests in CUDA and CPU builds. The first GPU
run can spend a few minutes compiling PTX in the driver; the scripts use a local
cache in `.cuda/jit-cache` for subsequent runs.

### Linux / WSL (CUDA)

```bash
cargo build --release
```

The binary will be located at `target/release/words-breaker`.

For CPU-only Linux/macOS, use `cargo build --release --no-default-features`.
The CUDA architecture defaults to `sm_86`; set `CUDA_ARCH` for another supported
architecture and `NVCC` to override the compiler executable.

## Usage

The 2026-09-06 upstream update (`855fe1e`) is integrated; see
[UPSTREAM.md](UPSTREAM.md) for the source revision and local adaptations.

```
words-breaker <TARGET_ADDRESS> <WORD1> <WORD2> ... <WORD12> [OPTIONS]
```

### Arguments

| Argument | Description |
|----------|-------------|
| `TARGET_ADDRESS` | Target Ethereum address, 40 hex characters with an optional `0x` prefix. Mixed-case input is verified against its EIP-55 checksum, so a typo is rejected up front instead of after an exhaustive search. |
| `WORD1..WORDN` | 10, 11, or 12 BIP-39 words in any order. With 10 or 11 words, the missing word(s) are completed from the 2048-word BIP-39 list. |

### Options

| Option | Default | Description |
|--------|---------|-------------|
| `--pattern` | | 12 space-separated slots, `?` for an unknown one. Non-`?` slots are pinned to that position. Replaces the positional word list. |
| `--pool` | | Words that fill the `?` slots, each used at most once. Fewer pool words than `?` slots means the leftovers are drawn from the fill set. |
| `--fill` | whole wordlist | Restricts what a leftover `?` slot may hold. Takes literal words and `prefix*` patterns, e.g. `--fill "d*,f*"`. Each leftover slot multiplies the search by this set's size, so this is the strongest lever available. |
| `-l, --language` | `english` | BIP-39 wordlist language |
| `-t, --threads` | `0` (all cores) | CPU threads (CPU path only) |
| `--cpu` | off | Force the CPU (rayon) search instead of the GPU |
| `--post` | | Six words from the post: `word@N` pins a word at a 1-based position; bare words are candidates for that origin's remaining slots. Requires `--video`; excludes positional words, `--pattern`, `--pool` and `--fill`. |
| `--video` | | Six words from the video, using the same syntax as `--post`. |
| `--exclude-record-dir` | | Load all compatible complete/partial negative JSON records in a directory; corrupt records cause an error. |
| `--record-progress` | | Atomically publish/update confirmed negative coverage after normal termination, including limited/paused searches and searches using history. |
| `--preflight-history` | | Report whether the requested domain is already covered, without checkpoint changes or GPU initialization. Proofs are bounded; small domains also get an exact membership inspection. |
| `--additional-record-dir` | | On resume only: apply newer compatible negatives while retaining the checkpoint's original filter fingerprint. |
| `--export-checkpoint-record` | | Export an existing checkpoint after its negative outcome has been independently confirmed; the desktop uses this for migration. A checkpoint alone is not evidence of a negative outcome. |
| `--no-checksum` | off | Derive every original phrase, even with an invalid BIP-39 checksum (~16x more derivations). GPU batches are capped at 65,536 in this mode. |
| `--selftest` | | Verify GPU primitives and the actual search pipeline against CPU references, then exit |
| `--batch-size` | `1048576` | Candidates per GPU batch; CPU caps batches at 4096 |
| `--block-size` | `64` | CUDA threads per block: 32, 64, 128 or 256 |
| `--max-candidates` | unlimited within a process counter | Stop after this many additional candidates; report an incomplete search if candidates remain |
| `--checkpoint` | | File for completed progress, saved every 10 seconds and at normal termination |
| `--resume` | off | Resume that checkpoint with identical search inputs |
| `-h, --help` | | Print help |
| `-V, --version` | | Print version |

The search runs on the **GPU by default**, streaming candidates in batches. Each
batch is filtered by BIP-39 checksum on the GPU (a cheap pass that keeps ~1/16 of
candidates), then only the survivors run the full seed/derivation/address
pipeline. Use `--selftest` to confirm the GPU primitives (SHA-256/512, Keccak-256,
HMAC, PBKDF2, secp256k1, BIP32) match the reference CPU crates bit-for-bit.

**Supported languages:** `english`, `portuguese`, `spanish`, `french`, `italian`, `czech`, `korean`, `japanese`, `chinese-simplified`, `chinese-traditional`

### Examples

**Windows:**
```powershell
.\target\release\words-breaker.exe 0xc01B0dEFB2D8767F9C9A59EB464437e62428A31d scene spell mask private regret soda spike coconut any little december bronze
```

**Linux / macOS:**
```bash
./target/release/words-breaker 0xc01B0dEFB2D8767F9C9A59EB464437e62428A31d scene spell mask private regret soda spike coconut any little december bronze
```

That example is a throwaway mnemonic generated for testing, with its first two
words swapped. It resolves to `spell scene mask ...` at candidate index
39,916,800 (= 11!, the lexicographic rank of a single leading transposition).

**Known positions plus a word pool:**
```bash
./target/release/words-breaker 0x… \
  --pattern "dutch ? ? ? fog ? ? ? ? ? ? parrot" \
  --pool "fork fiber forest dinner goat seed key lake"
```

Three words are pinned to positions 1, 5 and 12; the eight pool words permute
over the nine open slots, and the ninth slot — whichever it turns out to be — is
drawn from the full 2048-word list. The distinct count is `2040 x 9! + 8 x (9!/2!) = 741,726,720`: the
eight fill words already in the pool each produce a repeated-word multiset. The tool prints this count before starting, so a miscounted pool is
caught in a second rather than an hour in.

**Two batches of six (post + video):**

This example tests the **hypothesis** `fiber@4`. Hint 4 identifies a word,
not its seed position; `fiber` must also be considered in other positions.

```bash
./target/release/words-breaker <address> \
  --post "dutch@1 fiber@4 forest dinner rib roast fresh cattle" \
  --video "fog@5 parrot@12 expect easy there will lake"
```

Pins count toward their origin's six words. Here, four holes use four of the
six unpinned post candidates, and four use four of the five video candidates.
The eight holes can be assigned to the origins in any order, giving
`C(8,4) * P(6,4) * P(5,4) = 3,024,000` distinct phrases. There is no fill fallback;
each origin needs enough unpinned candidates. Extra candidates are allowed.
Repeated pool entries permit repeated words, and words shared by the pools are
supported without repeating the same final phrase. All selectors use NFKD
normalization, including accented `word@N` entries.

`--no-checksum` works with either search mode. It derives the exact candidate
words, without repairing their checksum. This is useful only when testing the
hypothesis that a generator produced non-conforming phrases; it does not reduce
the cost of a normal BIP-39 search.

**Previous search coverage (optional):**

`--list-history` prints the archived research catalog. `--coverage-report`
checks overlap with RO1 without PBKDF2 or GPU initialization; it examines at
most 1,000,000 original candidates by default, or `--max-candidates N`.
A bounded prefix is explicitly reported as incomplete, never extrapolated.

```bash
./target/release/words-breaker 0x9c2f44efad0c1e852a09df9939e6daf061140caf \
  --pattern "dutch ? ? ? fog ? ? ? ? ? ? parrot" \
  --pool "update winter cattle lake also forest wood fiber fork" \
  --coverage-report
```

To skip the matching domain during an actual search, replace
`--coverage-report` with `--exclude-tested RO1`. This is explicit trust in the
archived **external** negative report, not a locally repeated wallet search.
The exact membership filter runs before CPU/GPU cryptography; it preserves
source order and cross-source repeated words. Other catalog entries are
reference-only. No words or unrestricted permutations are blacklisted.

RO1 exclusions require the challenge target, English, checksum checking,
empty passphrase and `m/44'/60'/0'/0/0`. Incompatible requests fail before
searching. Original enumeration still determines limits, candidate indices
and checkpoints. Excluded arrangements are counted separately; this count
includes checksum-invalid rows and is **not** a count of saved derivations.
Version-3 checkpoints bind the exclusion model and evidence fingerprint;
resuming with exclusions changed or removed is rejected. Existing searches
without this option keep their previous behavior.

See [history/README.md](history/README.md) for provenance, validation and scope.

**Reuse your own completed searches:**

Add `--record-search completed.json` to a search to save its full domain after
exhaustion without a match. Partial runs and matches do not produce negative
records. Continue a partial run with the same checkpoint and `--resume`.
Recording requires a search without exclusions, so local evidence cannot
silently inherit an external negative.

Use `--exclude-record completed.json` on a later search with the same target,
language and checksum policy. Multiple records are allowed by repeating the
option, and can be combined with `--exclude-tested RO1`. Matching phrases are
excluded before cryptography, including in broader pools. `--coverage-report
--exclude-record completed.json` previews the overlap without searching.

When a record proves that the entire requested domain is covered, a run without
`--checkpoint` skips enumeration and GPU initialization altogether. The proof
handles reordered or smaller pools, additional pins, restricted fills and
two-source quotas, including comparisons between template and batch searches.
It uses dynamic programming over word counts, not permutations. Existing pins
in the record must remain fixed, and added pins consume the recorded capacity.
The proof respects required words and repetitions; comparing word sets alone
would be insufficient. If no single record suffices, a bounded proof combines
the selected records by splitting the query into exhaustive subdomains.
Every subdomain must be covered; overlapping sources are never double-counted.
The whole-domain proof allows at most 256 nodes and 4,096 record checks, with
a 250 ms time budget checked between operations.

During CPU and GPU searches, local records also enable automatic prefix pruning:
the enumerator skips a subtree only when its complete remainder is proven covered.
Compilation examines at most the first three original open positions, constructs
at most 256 variant plans and performs at most 4,096 record checks, with a separate
soft 250 ms budget checked between operations. Overlapping post/video origins
must all be covered before their shared prefix can be skipped. This is a local
record optimization; RO1 keeps its exact per-phrase filter. Unproven branches,
including those left by a depleted budget, retain normal generation and filtering.

`--no-prune-history` disables prefix pruning while retaining per-phrase exclusions.
It can be toggled on resume: pruning preserves the checkpoint schema, original
cursors, candidate indices and raw `--max-candidates` limits. The `Pruned ...
without generation this run` counter includes only skipped subtrees; phrases
already generated and then excluded remain part of the total history exclusions.
See [the pruning benchmark](history/PRUNING-BENCHMARK.md) for a reproducible
comparison using identical records with pruning enabled and disabled.

Covered-subtree counts are also reused in a lazy in-memory cache of at most 1,024
states, shared by clones of the same search domain. New states replace the oldest
entries as the search advances. The key retains the complete multiset of
choices already assigned, including its length; it does not rely on hash equality.
Equivalent prefixes therefore reuse the same exact residual count. Ordinary
unpruned enumeration does not initialize the cache. `--no-count-cache` disables
this optimization for comparison, without changing checkpoints or candidate order.

With history exclusions, the GPU producer now accumulates up to `--batch-size`
remaining candidates across excluded regions. Device-buffer capacity stays the
same, while a batch may account for more original candidates. A raw-work limit
of 64 times the batch size and a soft 250 ms producer budget allow partially
filled batches to report progress. The raw candidate limit and original hit
indices are preserved; only fully processed batches advance the checkpoint.
`--no-pack-history` restores batching by original candidates. Both switches can
be changed when resuming the same checkpoint. These batches precede the existing
GPU checksum filter.

See [the packing and count-cache benchmarks](history/PACKING-BENCHMARK.md) for
reproducible measurements and their scope.

Local-history membership now intersects word/position bitmasks before checking
the possible records exactly. Partial-record frontiers and absent words are
rejected before rebuilding multiplicities. The index is shared, bounded to
6 MiB and does not change evidence fingerprints or checkpoint formats. Partial
negatives can also prove whole prefixes when both their original-order boundary
and full structural coverage permit it. See the
[host-filter profile and before/after benchmark](history/HOST-OPTIMIZATION.md).

`--adaptive-batch` opts into GPU batch control based on completed processing
latency. It starts at 16,384 candidates (or the lower `--batch-size` ceiling),
grows after two full batches below 125 ms and halves the request above 500 ms,
down to 256 or the lower ceiling. Queued batches with an old request do not
change the current setting. This manages latency rather than guaranteeing the
highest throughput; fixed batching remains the default. Candidate buffers are
still allocated for the ceiling, so this is not an automatic VRAM tuner.

`--metrics` prints completed-work counters and stage timings; `--metrics-json
report.json` writes them to a new JSON file after normal termination. CPU and GPU
report original candidates, exclusions, candidates retained for checking and
derivation candidates. GPU timings separate host generation/history filtering,
queue waiting, transfers, checksum filtering and derivation. CPU reports checksum
and derivation together. Timings use host clocks and overlap across producer/GPU
work; they must not be added as if all stages were sequential.

Metrics are per invocation, including on resume, and exclude prefetched work and
the batch containing a hit. Requested batch sizes and adjustment counts are
also reported. A whole-domain proof reports backend `coverage-proof` with no
completed batches. Reports do not contain candidate words or the target address.
See [adaptive batching and metrics](history/ADAPTIVE-METRICS.md) for usage and
reproducible validation.

**Verify the GPU implementation:**
```bash
./target/release/words-breaker --selftest
```

**With Portuguese wordlist:**
```bash
./target/release/words-breaker <TARGET_ADDRESS> bexiga bonde curativo nevoeiro mundial vareta urubu megafone cozinha livro surpresa senador -l portuguese
```

## How It Works

Every mode uses 12 slots, each pinned or open. The template mode fills holes
from one pool, with fill fallback when needed. The post/video mode uses two
pools and exact six-word quotas, counting pins. Passing 12 loose words is the
case of 12 open slots and one 12-word pool. All modes share the allocation-free
traversal in `src/candidates.rs`.

For distinct pool words with no overlap between pool and fill, with `h` open
slots, a pool of `p` and a fill set of `f`, the space is
`p!/(p-h)!` when `p >= h` (which pool words, in which order), and
`h!/(h-p)! x f^(h-p)` when `p < h` (place the pool, then draw the rest from the
fill set).

With repeated pool words or overlap, those expressions overcount. The program
now enumerates each distinct phrase once and computes the exact count from word
multiplicities. Repeated words remain permitted; a fill word already in the pool
is not removed. Counts beyond `u128` are rejected before searching.

That last exponent is where searches live or die. As an assignment-count
illustration (before deduplication), two unknown slots over the whole list give
`2048^2` = 4.2M times the placement count; restricting them with
`--fill "d*,f*"` gives `218^2` = 47.5K times — an 88x cut. If you
know anything at all about the missing words, spend it here.

1. Streams distinct candidates as compact 12-index arrays without allocations per candidate
2. On the GPU, each candidate is checksum-filtered (unless `--no-checksum`), then survivors are derived:
   PBKDF2-HMAC-SHA512 seed → BIP32 `m/44'/60'/0'/0/0` → secp256k1 public key →
   `keccak256(X ‖ Y)`
3. The low 20 bytes of that digest are the address, compared against the target
4. Stops and outputs the correct phrase when a match is found

Note that BIP32's master-key HMAC uses the literal string `"Bitcoin seed"` for
every coin — that constant is fixed by BIP32 itself, not by Bitcoin. The only
thing separating an Ethereum account from a BIP-44 Bitcoin one here is the coin
type (`60'` vs `0'`) and the final hash.

## Performance Notes

- 12 distinct words have 479,001,600 (12!) possible permutations; supplying 10 or 11 words
  multiplies this by up to 2048 per missing word
- The whole space is streamed and searched (there is no fixed permutation cap)
- Invalid BIP-39 checksums are filtered out cheaply before the expensive work,
  which removes 15/16 of candidates for the cost of one SHA-256

Historical CUDA baseline, before the current host-side changes: on an RTX 3050
the full 12! space took 197 s (measured, exhaustive run), at
roughly 2.42M permutations/s (~151k full derivations/s after checksum
filtering).

The cost per surviving candidate is dominated by PBKDF2-HMAC-SHA512, whose 2048
iterations are fixed by the BIP-39 spec — measured at ~92% of GPU time, with all
of BIP32 + secp256k1 + the address hash making up the rest. The address hash is
the smallest part of that: one Keccak-f[1600] permutation against PBKDF2's 4096
SHA-512 compressions. Three things matter most:

1. **PBKDF2 runs entirely in registers.** With `dkLen == 64` every one of the
   4096 SHA-512 compressions per candidate is a single block of fixed layout
   (`W[8] = 0x80…`, `W[9..14] = 0`, `W[15] = 1536`), so the loop carries its
   state and message as `u64` registers instead of streaming bytes through a
   context struct in local memory. This was worth ~6.5x on its own; a generic
   streaming SHA-512 spends more time on local-memory traffic than on hashing.
2. **All three scalar multiplications are fixed-base.** Both non-hardened BIP32
   levels and the final public key are `k*G`, so a precomputed table of multiples
   of G (4-bit windows, built once at startup by `k_init_gtable`) replaces
   double-and-add: 64 point additions and no doublings.
3. **Field inversion uses an addition chain** (255 squarings + 15 multiplies)
   rather than a full exponentiation by `p-2`.

Candidate batches are generated on a producer thread so the CPU-side permutation
stream overlaps with the GPU work rather than running between launches.
PBKDF2 and address derivation now run in separate CUDA kernels so curve arithmetic
does not raise the seed kernel's register requirements. The intermediate seed
buffer stays on the GPU and grows to accommodate actual checksum survivors.
Native RTX 3060 Laptop measurements showed a modest +2.33% median full-search
gain at the default settings over the already ported combined kernel; see
[BENCHMARKS.md](BENCHMARKS.md) for inputs, raw results and thermal limitations.

## Bounded runs, checkpoints and comparison

Keep the same target, language, pattern, pool (including its order), fill, both
origin pools and checksum policy when resuming. Block/batch sizes, CPU/GPU
backend and thread count may change. Existing version-1 template checkpoints
remain readable; changing `--no-checksum` on resume is rejected. The new modes
write version-2 checkpoints to prevent older binaries from ignoring their extra
constraints. A checkpoint stores the cursor after completed work, not queued GPU work;
resume does not walk through all previous candidates. After an abrupt stop, at
most the work since the last checkpoint needs to be repeated. Candidate order in
fill searches has changed; old indices must not be used as new checkpoints.
Checkpoints contain the search inputs; keep them with your local challenge files.

```bash
./target/release/words-breaker <address> <words...> --max-candidates 1000000 --checkpoint run.json
./target/release/words-breaker <address> <words...> --checkpoint run.json --resume
```

For comparable throughput measurements, choose a bounded space/target known to
have no match so early success does not skew timings. Repeat after warm-up with
identical inputs; compare `--block-size 64`, `128` and `256` and different batch
sizes. The reported overall rate includes backend initialization and checkpoint
I/O. A candidate limit is explicitly reported as incomplete, not exhausted.

```bash
cargo test --no-default-features
cargo bench --no-default-features --bench candidates
```

On Windows, `./scripts/benchmark-gpu.ps1 -OutputCsv results.csv` compares block
and batch sizes after warm-up. It refuses to benchmark an automatic CPU fallback.
Use `-BaselineBinary ./baseline.exe -DefaultOnly -Repeats 5` for an alternating
comparison of two executables at the default launch settings.

The enumeration benchmark includes the historical implementation only as a test
baseline. See [BENCHMARKS.md](BENCHMARKS.md) for the measured changes and limits.
For CUDA builds, also run `words-breaker --selftest` before long searches. It now
covers all languages, dense survivors, block sizes and batch boundaries, in
addition to the primitive tests. Every GPU hit is independently verified on CPU.

## License

MIT
