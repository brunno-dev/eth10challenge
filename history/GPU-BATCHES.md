# Lotes da GPU — 20/09/2026

Depois de otimizar as consultas ao histórico, a derivação na GPU passou a
dominar os cenários medidos. A interface e o MCP usavam lotes de 65.536
candidatos, enquanto o CLI já usava 1.048.576. Com checksum, aproximadamente
1/16 de cada lote chega à derivação cara; lotes pequenos deixam pouco trabalho
para distribuir pela placa.

## Comparação na máquina local

RTX 3060 Laptop, blocos CUDA de 64 threads, mesmo motor e mesmo histórico
congelado do [benchmark anterior](HOST-OPTIMIZATION.md). Primeiros 5.000.000
candidatos de cada cenário, três repetições por configuração, ordem alternada
e aquecimento prévio. Nenhuma compilação ou outra busca foi executada em paralelo.
Medianas do tempo de processo, incluindo inicialização e gravação do progresso:

| Configuração | Cinco moedas + antigas | Tela + fala + antigas |
| --- | ---: | ---: |
| Fixo: 65.536 | 2,532 s | 2,637 s |
| Fixo: 262.144 | 1,861 s | 1,923 s |
| Fixo: 1.048.576 | **1,813 s** | **1,812 s** |
| Adaptativo, teto 1.048.576 | 1,890 s | 1,976 s |

O novo padrão aumenta a taxa de processamento em **39,7%** e **45,6%**,
respectivamente, sobre o motor que já tinha o histórico otimizado. Isso
equivale a **28,4%** e **31,3% menos tempo**. As taxas de cerca de 2,76 milhões
de candidatos/s incluem exclusões do histórico e rejeições por checksum;
não representam esse número de carteiras derivadas por segundo.

Os resultados são amostras curtas desta máquina e destes dois domínios.
Não medem uma busca contínua durante dias, outras GPUs, o modo sem checksum
ou outras distribuições de filtros. O adaptativo permanece disponível,
mas não foi escolhido como padrão porque ficou mais lento nestas amostras.

## Mudança aplicada

- Novas buscas na interface e no MCP usam lote máximo de 1.048.576.
- O limite por rodada continua em 100.000 por padrão. O motor respeita esse
  limite mesmo quando ele é menor que o tamanho do lote.
- Configurações explícitas e retomadas mantêm os valores informados/salvos.
- CPU continua limitada internamente a lotes de até 4.096; CUDA sem checksum,
  a até 65.536. Não houve alteração nos kernels nem na ordem dos candidatos.

## Verificações

As 24 execuções tiveram resultados negativos. Em cada cenário, todas as
configurações produziram checkpoints completos idênticos, o mesmo digest
do registro negativo e as mesmas contagens:

| Cenário | Candidatos originais | Excluídos | Retidos | Sobreviventes do checksum |
| --- | ---: | ---: | ---: | ---: |
| Moedas | 5.000.000 | 1.195.600 | 3.804.400 | 237.551 |
| União | 5.000.000 | 1.000.084 | 3.999.916 | 249.993 |

Os lotes enviados ao dispositivo caíram de 59 para 4 em moedas e de 62 para
4 na união. Essa alteração agrupa mais trabalho em cada envio; não pula
candidatos nem flexibiliza a verificação. Os testes MCP também verificam
que a configuração explícita anterior continua aceita e o limite não muda.

Validação da entrega: 16 testes MCP e 24 testes Tauri passaram; Clippy sem
avisos e build de produção concluídos. O motor empacotado, com lote 1.048.576,
encontrou corretamente o endereço Ethereum conhecido do vetor BIP-39 público
com onze `abandon` e `about`. Os hashes dos executáveis no ZIP são iguais aos
da pasta portátil atualizada.

Motor medido (sem recompilação entre configurações), SHA-256:
`5220d7f7ea42915b2b461c3aed095da2eb7711c17b85ba97e8f07bc81ec92a39`.

## Reprodução

```powershell
./scripts/benchmark-gpu-batches.ps1 -Binary <motor.exe> -History <copia-do-historico> -OutputDirectory <pasta-nova> -Trials 3
```

O script exige CUDA, aquece o driver, alterna a ordem, salva métricas e
interrompe em caso de erro, match ou divergência de cobertura/resultados.
Ele repete trabalho deliberadamente para medir desempenho e guarda os
negativos somente na pasta de saída, sem modificar o histórico do app.
Nesta comparação, os trechos já estavam cobertos pelo histórico real.

Resultados locais completos:
`output/import-check/host-optimization/gpu-batches-20260920/` (ignorado pelo Git).
