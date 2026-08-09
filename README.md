<div align="left">

<a href="https://burd.ia">
  <img src="./public/burd-logo.svg" alt="Logo da Burd" title="Burd Benchmark" height="48" />
</a>

<br />

<p>
  Produto local de validação da Burd para detectar hardware, executar benchmarks, calcular score, gerar relatórios assinados e preparar evidências locais para providers de compute.
</p>

[![status](https://img.shields.io/badge/status-active-2C5E8A)](https://github.com/Burd-repo/benchmark)
[![license](https://img.shields.io/badge/license-MIT-green)](./LICENSE)
[![agent](https://img.shields.io/badge/agent-burd--agent-blue)](https://github.com/Burd-repo/benchmark)
[![validation](https://img.shields.io/badge/provider-validation-lightgrey)](https://github.com/Burd-repo/benchmark)

</div>

---

## Sumário

* [Visão geral](#visão-geral)
* [Início rápido](#início-rápido)
* [Build](#build)
* [Comandos principais](#comandos-principais)
* [API local](#api-local)
* [Identidade do Provider](#identidade-do-provider)
* [Relatórios assinados](#relatórios-assinados)
* [Challenge local](#challenge-local)
* [Readiness](#readiness)
* [Score](#score)
* [Provider Trust Layer](#provider-trust-layer)
* [Hardware fingerprint e marketplace policy](#hardware-fingerprint-e-marketplace-policy)
* [Sessoes e heartbeat](#sessoes-e-heartbeat)
* [AI Performance Metrics](#ai-performance-metrics)
* [Network score](#network-score)
* [Reliability e uptime](#reliability-e-uptime)
* [Trust score e workload eligibility](#trust-score-e-workload-eligibility)
* [Runtime seguro](#runtime-seguro)
* [Histórico](#histórico)
* [Payload de registro](#payload-de-registro)
* [Regras de segurança](#regras-de-segurança)
* [Diretrizes para Pull Request](#diretrizes-para-pull-request)
* [Checklist de Pull Request](#checklist-de-pull-request)
* [Convenção de commits](#convenção-de-commits)
* [Não commitar](#não-commitar)
* [Notas para mantenedores](#notas-para-mantenedores)
* [Licença](#licença)
---

## Visão geral

O **Burd Benchmark** gera o binário `burd-agent`, responsável pela validação local de máquinas que desejam atuar como providers.

O agent é responsável por:

* detectar hardware local;
* identificar GPU, VRAM, CPU, RAM, disco e drivers;
* estimar compatibilidade com workloads de IA;
* executar benchmarks locais;
* calcular o Burd Compute Score;
* gerar relatórios assinados;
* verificar relatórios;
* criar e validar challenges locais;
* registrar histórico local;
* calcular readiness;
* calcular reliability local a partir do historico de heartbeat;
* calcular network score local a partir de benchmark finito;
* expor uma API local para integração com interfaces.

Este repositório não é uma landing page institucional.
O foco aqui é validação local, contratos de dados, score, evidências e API do provider.

---

## Início rápido

```bash
git clone https://github.com/Burd-repo/benchmark.git
cd benchmark
cargo build
```

No Windows PowerShell:

```powershell
.\target\debug\burd-agent.exe --help
```

Validação local rápida:

```powershell
.\scripts\test-local.ps1
```

Esse script executa verificações seguras e rápidas. Ele não inicia o servidor local, não roda loops de heartbeat e não executa benchmarks pesados.

---

## Build

### Build padrão

```bash
cargo build
```

### Build release

```bash
cargo build --release
```

### Testes

```bash
cargo test --workspace
```

### Formatação

```bash
cargo fmt --all --check
```

### Checklist local recomendado

```powershell
cargo fmt --all --check
cargo test --workspace
cargo build --release
.\scripts\test-local.ps1
```

---

## Comandos principais

### Sistema

```bash
burd-agent system --json
burd-agent fingerprint --json
burd-agent raw --json
```

### Fit e benchmark

```bash
burd-agent fit --json
burd-agent bench llm --provider ollama --model llama3.2:1b --runs 3 --json
burd-agent bench stability --minutes 10 --json
burd-agent bench network --json
burd-agent bench disk --json
```

### Score

```bash
burd-agent score --json
burd-agent network-score --json
```

### Relatórios

```bash
burd-agent report --json
burd-agent report --run-all --json
burd-agent report --run-all --signed --json
burd-agent verify-report --file docs/examples/signed-report.json --json
```

### Identidade

```bash
burd-agent identity init
burd-agent identity show --json
burd-agent identity rotate-key --confirm
burd-agent identity migrate --confirm
```

### Enrollment remoto

```powershell
burd-agent identity init
$env:BURD_ENROLLMENT_TOKEN = "<one-time-token>"
burd-agent enrollment enroll --control-plane-url http://127.0.0.1:8080
burd-agent enrollment status --json
burd-agent enrollment refresh-credential --json
```

O fluxo remoto prova posse da chave Ed25519 sem transmitir a chave privada. Veja
[`docs/bn-02-provider-enrollment.md`](docs/bn-02-provider-enrollment.md).

### Sessao remota

```powershell
burd-agent remote-session connect
burd-agent remote-session connect --telemetry --telemetry-batch-samples 8
burd-agent remote-session connect --proofs --telemetry-batch-samples 8
burd-agent remote-session status --json
burd-agent remote-session lifecycle --json
```

O agente mantem uma conexao WebSocket de saida autenticada, com heartbeat
sequenciado, retomada e backoff. Um lock exclusivo por diretorio de estado
serializa `remote-session connect` e mudancas criticas de identity, enrollment e
API token; comandos de status e diagnostico continuam disponiveis.
`remote-session status` consulta o estado autoritativo do backend, enquanto
`remote-session lifecycle` le o ciclo local do processo foreground (`starting`,
`connecting`, `online`, `degraded`, `stopping`, `terminal_failure` ou
`stopped`). Readiness local so e verdadeira em `online`; um lock de liveness
mantido pelo processo impede que um snapshot obsoleto apos crash seja exposto
como ativo. Estados JSON canonicos usam substituicao atomica por arquivo; isso
evita JSON parcial, mas nao forma uma transacao entre arquivos nem impede toda
atualizacao concorrente. Veja
[`docs/bn-03-remote-session.md`](docs/bn-03-remote-session.md).
O contrato de lifecycle agora falha fechado para estado de sessao corrompido,
impede resume token de atravessar Control Planes e invalida a sessao local apos
novo enrollment. Preparacao de conexao e refresh de credencial recebem
cancelamento cooperativo e um grace period de cinco segundos no supervisor,
assim como o Proof of Capability. Chamadas nativas/HTTP bloqueantes que ja
estiverem em andamento nao sao forcadamente interrompidas; o Agent continua
foreground, sem daemon ou limite global de saida do processo. Veja
[`docs/hardening/agent-service-lifecycle-contract.md`](docs/hardening/agent-service-lifecycle-contract.md),
[`docs/hardening/agent-lifecycle-readiness.md`](docs/hardening/agent-lifecycle-readiness.md)
e
[`docs/hardening/agent-cooperative-proof-shutdown.md`](docs/hardening/agent-cooperative-proof-shutdown.md).
O processo foreground tambem emite o evento redigido `burd.agent.exit.v1` e
usa codigos estaveis para parada solicitada (`0`), argumentos semanticos
invalidos (`2`), estado local (`10`), credencial (`11`), revogacao (`12`),
rejeicao remota (`13`), contrato remoto (`14`) e falha interna (`15`). Outage
recuperavel nao encerra o Agent: permanece `degraded` com retry. Outros comandos
ainda usam o codigo legado `1`; erros sintaticos do Clap continuam no formato
nativo com codigo `2`. Veja
[`docs/hardening/agent-exit-status-contract.md`](docs/hardening/agent-exit-status-contract.md).
Uma matriz fisica sanitizada em Windows/AMD/Vulkan/Ollama/Docker confirma
diagnostico local, assinatura, challenge local, readiness e bloqueio correto do
marketplace sem alegar cobertura NVIDIA/CUDA; veja
[`docs/hardening/windows-physical-compatibility-matrix.md`](docs/hardening/windows-physical-compatibility-matrix.md).
O BN-04 adiciona telemetria GPU assinada no control plane; veja [`docs/bn-04-gpu-telemetry.md`](docs/bn-04-gpu-telemetry.md).
O BN-05 adiciona registry remoto de evidencias assinadas no control plane; veja
[`docs/bn-05-remote-evidence-registry.md`](docs/bn-05-remote-evidence-registry.md).
O BN-06 adiciona o protocolo backend-issued e o runner foreground do Agent para
Proof of Capability CUDA/VRAM/GEMM/LLM. `--proofs` implica telemetria assinada e
exige CUDA/Ollama conforme o perfil. O processo foreground supervisiona o worker
e persiste somente metadados limitados e redigidos de tentativas para impedir
reexecucao do mesmo challenge apos reinicio; isso nao e um daemon ou servico do
sistema operacional. Veja
[`docs/bn-06-active-proof-of-capability.md`](docs/bn-06-active-proof-of-capability.md).
O BN-07 adiciona estado backend-owned para verificacao recorrente e baseada em risco; veja
[`docs/bn-07-recurring-risk-verification.md`](docs/bn-07-recurring-risk-verification.md).
O sweep recorrente fica desabilitado ate o Control Plane receber um perfil completo
com digest Ollama exato e thresholds positivos; ele nao usa artifact mock como fallback.
O BN-08 adiciona registry backend-owned de probes regionais de rede e score remoto; veja
[`docs/bn-08-regional-network-probes.md`](docs/bn-08-regional-network-probes.md).
O BN-09 adiciona trust global e antifraude backend-owned no control plane; veja
[`docs/bn-09-global-trust-antifraud.md`](docs/bn-09-global-trust-antifraud.md).
O BN-10 adiciona registry backend-owned de Benchmark Profiles v2 e resultados assinados; veja
[`docs/bn-10-benchmark-profiles-v2.md`](docs/bn-10-benchmark-profiles-v2.md).

### Token local da API

```bash
burd-agent api-token create --json
burd-agent api-token rotate --json
burd-agent api-token show --json
```

### Challenge

```bash
burd-agent challenge create-mock --json
burd-agent challenge run-local --json
burd-agent challenge run --file docs/examples/challenge.json --json
burd-agent challenge verify --file signed-response.json --json
```

### Sessão local

```bash
burd-agent session start --json
burd-agent session status --json
burd-agent session stop --json
```

### Provider

```bash
burd-agent provider --json
burd-agent verify-provider --json
burd-agent readiness --json
burd-agent uptime --json
burd-agent reliability --json
burd-agent trust-score --json
burd-agent capability-spot --json
burd-agent workload-eligibility --json
burd-agent registration-payload --json
```

### Histórico e logs

```bash
burd-agent history --json
burd-agent history latest --json
burd-agent history export --output history.json
burd-agent logs --json
burd-agent logs --tail 50 --json
burd-agent actions --json
```

### API local

```bash
burd-agent serve --host 127.0.0.1 --port 8787
```

Todos os comandos com `--json` devem escrever JSON válido em `stdout`, sem misturar logs no mesmo output.

---

## API local

Para iniciar a API local:

```powershell
.\target\debug\burd-agent.exe serve --host 127.0.0.1 --port 8787
```

A API fica disponível em:

```txt
http://127.0.0.1:8787
```

Endpoints principais:

```txt
GET  /health
GET  /api/v1/system
GET  /api/v1/fit
GET  /api/v1/score
GET  /api/v1/report
POST /api/v1/report/signed
POST /api/v1/report/verify
GET  /api/v1/provider
GET  /api/v1/readiness
GET  /api/v1/verification
GET  /api/v1/uptime
GET  /api/v1/reliability
GET  /api/v1/network-score

GET  /api/v1/ai-performance

GET  /api/v1/trust-score
GET  /api/v1/capability-spot
GET  /api/v1/workload-eligibility

GET  /api/v1/history
GET  /api/v1/registration-payload
GET  /api/v1/pricing
GET  /api/v1/earnings
GET  /api/v1/actions
GET  /api/v1/logs
GET  /api/v1/raw
GET  /api/v1/config
POST /api/v1/benchmark/run
GET  /api/v1/benchmark/status
POST /api/v1/challenge/create-mock
POST /api/v1/challenge/run
POST /api/v1/challenge/verify
POST /api/v1/provider/verify
```

Para testar a API sem deixar o servidor rodando:

```powershell
.\scripts\test-api.ps1
```

Para parar um processo local preso:

```powershell
Get-Process burd-agent -ErrorAction SilentlyContinue | Stop-Process -Force
```

---

## Identidade do Provider

A identidade local é usada para assinar relatórios e comprovar a origem das evidências geradas pela máquina.

Comandos:

```bash
burd-agent identity init
burd-agent identity show --json
```

O agent grava configuração pública e mantém a chave privada separada.

Regras:

* relatórios não devem expor chave privada;
* raw data não deve expor chave privada;
* payloads públicos não devem incluir secrets;
* migrações devem preservar evidências válidas;
* mudanças de identidade devem ser explícitas.

---

## Relatórios assinados

Para gerar um relatório assinado completo:

```bash
burd-agent report --run-all --signed --json
```

Um relatório assinado pode conter:

* hash canônico do relatório;
* assinatura;
* chave pública;
* algoritmo da chave;
* timestamp de assinatura;
* resultado de verificação local;
* versão de canonicalização;
* resumo de hardware;
* score;
* evidências do benchmark.

Regras:

* `report --run-all` deve registrar histórico local;
* `report --run-all --signed` deve registrar histórico local;
* relatórios expirados não devem contar como evidência válida;
* assinatura inválida deve bloquear readiness;
* relatório assinado não deve conter segredo.

---

## Challenge local

O challenge local valida evidências por meio de nonce, expiração, assinatura e relatório assinado.

Comando recomendado:

```bash
burd-agent challenge run-local --json
```

Esse comando cria, executa, assina, verifica e persiste a evidência local do challenge sem exigir arquivo intermediário.

Regras:

* challenge expirado não deve contar ponto de readiness;
* nonce deve ser validado;
* assinatura do relatório deve ser validada;
* assinatura da resposta do challenge deve ser validada;
* evidência válida deve ser persistida;
* histórico sozinho não substitui evidência de challenge válida.

---

## Readiness

O readiness consolida o estado local do provider.

Comandos:

```bash
burd-agent readiness
burd-agent readiness --json
```

O readiness considera:

* identidade;
* relatório assinado;
* challenge;
* provider verification;
* histórico;
* token da API local;
* redaction de raw data;
* validade e expiração das evidências.

Estados esperados:

```txt
ready_locally
partial
failed
not_verified
uninitialized
```

`ready_locally` significa que os checks locais passaram.
Não significa aprovação externa, auditoria, listagem em marketplace ou garantia de receita.

---

## Score

O Burd Compute Score é uma pontuação de `0` a `100`.

Pesos do MVP:

```txt
40% benchmark LLM real ou fallback estimado
20% VRAM e capacidade
15% estabilidade
10% rede
10% disco
5% sinais de verificação
```

Tiers:

```txt
0-39    Not Eligible
40-59   Burd Basic
60-74   Burd Plus
75-89   Burd Pro
90-96   Burd Max
97-100  Burd Enterprise
```

Preços e ganhos demonstrativos não devem ser tratados como promessa de receita.

---

## Provider Trust Layer

A Provider Trust Layer da Burd organiza os sinais locais para a tese de produto:

```txt
Verified AI Compute. Not just listed. Proven.
```

Ela separa conceitos que nao devem ser misturados:

* Readiness: contratos locais validos agora, sem aprovar marketplace.
* Compute Score: capacidade computacional da maquina.
* Network Score: qualidade de conexao para perfis de workload.
* Reliability Score: estabilidade local de sessao e heartbeat.
* Trust Score: confianca heuristica local baseada em evidencias, historico e estabilidade.
* Capability Spot: verificacao local/mock de capacidade de IA.
* Workload Eligibility: decisao local por tipo de workload e futura candidatura de marketplace.

Documentacao detalhada:

* [`docs/provider-trust-layer.md`](docs/provider-trust-layer.md)
* [`docs/bn-00-architecture-freeze.md`](docs/bn-00-architecture-freeze.md)
* [`docs/bn-01-backend-foundation.md`](docs/bn-01-backend-foundation.md)
* [`docs/bn-05-remote-evidence-registry.md`](docs/bn-05-remote-evidence-registry.md)
* [`docs/bn-06-active-proof-of-capability.md`](docs/bn-06-active-proof-of-capability.md)
* [`docs/bn-07-recurring-risk-verification.md`](docs/bn-07-recurring-risk-verification.md)
* [`docs/bn-08-regional-network-probes.md`](docs/bn-08-regional-network-probes.md)
* [`docs/bn-09-global-trust-antifraud.md`](docs/bn-09-global-trust-antifraud.md)
* [`docs/bn-11-workload-eligibility-v2.md`](docs/bn-11-workload-eligibility-v2.md)
* [`docs/bn-12-secure-provider-runtime.md`](docs/bn-12-secure-provider-runtime.md)
* [`docs/bn-13-job-api-data-plane.md`](docs/bn-13-job-api-data-plane.md)
* [`docs/bn-14-scheduler-leases.md`](docs/bn-14-scheduler-leases.md)
* [`docs/bn-15-metering-usage-ledger.md`](docs/bn-15-metering-usage-ledger.md)
* [`docs/bn-16-marketplace-registry-listings.md`](docs/bn-16-marketplace-registry-listings.md)
* [`docs/bn-17-customer-accounts-reservations.md`](docs/bn-17-customer-accounts-reservations.md)
* [`docs/bn-18-billing-pix-payouts.md`](docs/bn-18-billing-pix-payouts.md)
* [`docs/bn-19-observability-sre.md`](docs/bn-19-observability-sre.md)
* [`docs/bn-20-security-hardening-attestation.md`](docs/bn-20-security-hardening-attestation.md)
* [`docs/bn-21-multi-gpu-foundation.md`](docs/bn-21-multi-gpu-foundation.md)
* [`docs/remote-protocol-v1.md`](docs/remote-protocol-v1.md)
* [`docs/remote-authority-matrix.md`](docs/remote-authority-matrix.md)
* [`docs/threat-model.md`](docs/threat-model.md)
* [`docs/reliability-score.md`](docs/reliability-score.md)
* [`docs/network-score.md`](docs/network-score.md)
* [`docs/trust-score.md`](docs/trust-score.md)
* [`docs/spot-verification.md`](docs/spot-verification.md)
* [`docs/workload-eligibility.md`](docs/workload-eligibility.md)

A camada local nao implementa marketplace real, jobs, leases, scheduler, billing, Pix ou payouts. O control plane agora possui registry/listings backend-owned no BN-16, contas/reservas de cliente no BN-17, primitives financeiros BN-18 para price book, Pix intents, invoices, ledger financeiro e payouts administrados, observabilidade operacional BN-19, registry de security posture/attestation BN-20 e inventory multi-GPU backend-owned no BN-21.
O BN-01 inicia o backend real em `crates/burd-control-plane`; BN-11 ja registra policies remotas e eligibility backend-derived a partir de benchmark/trust/network/verification. O BN-12 adiciona planejamento local de runtime seguro Docker/NVIDIA para imagens digest-pinned e allowlisted. O BN-13 adiciona Job API e data-plane grants no control plane. O BN-14 adiciona scheduler pass admin-triggered e leases para jobs ja criados. O BN-15 adiciona usage ledger append-only e recibos hash-backed para jobs terminais. O BN-16 adiciona marketplace registry/listings backend-owned a partir de trust, eligibility, proof, benchmark, network e leases. O BN-17 adiciona contas de cliente, projetos, API keys, quotas, credit ledger nao financeiro, reservas e usage views. O BN-18 adiciona price book, Pix intents confirmaveis, ledger financeiro double-entry append-only, invoices e payout accounts/payouts administrados, ainda sem gateway Pix real ou checkout UI. O BN-19 adiciona logs estruturados, correlation IDs, metrics Prometheus, snapshot admin e SLO status para operacao inicial do control plane. O BN-20 adiciona security posture assinada, registry imutavel, policy backend-owned e metadados de attestation/hardening, ainda sem TPM/HSM/OS keychain ou verifier de attestation produtivo.

## Hardware fingerprint e marketplace policy

A Burd gera um fingerprint tecnico versionado para vincular evidencias assinadas ao hardware atual. Mudancas relevantes de GPU, VRAM, backend, CUDA/ROCm/Vulkan, driver critico, CPU, RAM, OS ou arquitetura alteram o fingerprint e devem invalidar evidencias antigas para readiness/session/trust.

A politica local `nvidia_cuda_only_mvp` define a direcao do marketplace pago MVP:

* NVIDIA RTX 30xx+ ou datacenter NVIDIA compativel;
* CUDA disponivel e backend CUDA;
* VRAM presente, com fonte e confianca `detected`;
* evidencias assinadas, challenge, readiness, trust, sessao e heartbeat suficientes em camadas superiores.

AMD, Intel GPU, Apple Silicon, ROCm, Vulkan-only e CPU-only podem continuar em diagnostico local quando suportados, mas devem permanecer fora do marketplace pago MVP.

Documentacao:

* [`docs/hardware-fingerprint.md`](docs/hardware-fingerprint.md)
* [`docs/marketplace-gpu-policy.md`](docs/marketplace-gpu-policy.md)

## Sessoes e heartbeat

Provider Session representa a tentativa local de ficar disponivel agora. Heartbeat e uma verificacao local de uma execucao, sem loop continuo.

```bash
burd-agent session start --json
burd-agent session status --json
burd-agent session stop --json
burd-agent heartbeat --once --json
```

A sessao guarda fingerprint, readiness snapshot, report hash, challenge id, expiracao e snapshot da marketplace policy. Heartbeat atualiza `last_heartbeat_at`, incrementa historico local e invalida a sessao se o fingerprint atual divergir.

Documentacao:

* [`docs/provider-session.md`](docs/provider-session.md)
* [`docs/heartbeat.md`](docs/heartbeat.md)
* [`docs/evidence-expiration.md`](docs/evidence-expiration.md)

## AI Performance Metrics

O comando burd-agent ai-performance --json consolida metricas de performance de IA sem executar benchmark pesado automaticamente.

A API local expoe o mesmo contrato em GET /api/v1/ai-performance.

O relatorio separa metricas medidas (real_benchmark, signed_report, benchmark_history) de estimativas (fit_estimate) e dados ausentes (not_measured). Campos sem medicao confiavel retornam null ou origem not_measured/unavailable; estimativas de fit nunca sao tratadas como benchmark real. Evidencia expirada permanece visivel com is_expired true, warning e confianca reduzida.

Este recurso local nao inicia runtime externo, nao executa o workload remoto de Proof of Capability, nao submete automaticamente resultados BN-10, nao aprova marketplace e nao cria scheduler, jobs, leases, billing ou payouts.
## Network score

O network score local usa a ultima amostra finita de `bench network --json` ou a secao `network` do ultimo `report --run-all` salvo em `~/.burd/latest-report.json`.
Ele nao executa medicao continua, nao abre porta, nao prova disponibilidade publica e nao representa aprovacao de marketplace.

Comandos:

```bash
burd-agent bench network --json
burd-agent network-score --json
```

Campos principais:

```txt
network_score   score 0-100 ponderado por latencia, jitter, perda e DNS
status          no_benchmark, failed, constrained, usable, strong ou excellent
level           No Data, Poor, Constrained, Usable, Strong ou Excellent
```

---

## Reliability e uptime

O score de reliability local usa apenas o historico de heartbeat em `~/.burd/uptime.json`.
Ele nao altera o Burd Compute Score e nao representa disponibilidade de backend, auditoria externa, listagem em marketplace ou promessa de receita.

Comandos:

```bash
burd-agent uptime --json
burd-agent reliability --json
```

Campos principais:

```txt
uptime_score        score 0-100 ponderado por uptime 1d, 7d e 30d
reliability_score   score 0-100 com uptime, cobertura de amostras, status recente e penalidade de falhas consecutivas
status              no_history, warming_up, reliable, degraded ou offline
```

---

## Trust score e workload eligibility

Trust Score local e um score heuristico do agente. Ele combina integridade de verificacao, frescor de evidencias, reliability, network score e profundidade de historico. Ele nao e reputacao global, aprovacao de backend ou elegibilidade de payout. O trust global backend-owned com antifraude inicial fica no BN-09 do control plane.

Workload Eligibility local usa fit, capability spot, trust, verification, reliability, compute score e marketplace policy para indicar se cada workload esta `eligible_locally`, `diagnostic_only`, `not_recommended`, `blocked`, `marketplace_candidate` ou `marketplace_blocked`. O BN-11 adiciona eligibility remota backend-owned no control plane com `eligible`, `limited`, `ineligible`, `verification_required`, `temporarily_unavailable` e `blocked`.

Comandos:

```bash
burd-agent trust-score --json
burd-agent capability-spot --json
burd-agent workload-eligibility --json
```

APIs:

```txt
GET /api/v1/trust-score
GET /api/v1/capability-spot
GET /api/v1/workload-eligibility
```

Esses contratos sao locais e nao criam jobs, leases, scheduler assignment, marketplace admission, billing, Pix ou payouts.

## Runtime seguro

O BN-12 adiciona planejamento local de runtime seguro para o futuro data plane de jobs. Ele inspeciona Docker/NVIDIA, valida template aprovado, imagem `@sha256`, allowlist, GPU UUID e perfil de isolamento. Ele nao executa job de cliente.

Comandos:

```bash
burd-agent runtime check --json
burd-agent runtime plan --image-ref ghcr.io/burd/runtime/llm@sha256:<digest> --allow-image-ref ghcr.io/burd/runtime/llm@sha256:<digest> --gpu-uuid GPU-... --json
```

`docker_args` so aparece quando o plano esta `ready`. Em Windows/macOS, o esperado para readiness e `unsupported_host`, porque o runtime seguro com GPU comeca em Linux.

Documento: [`docs/bn-12-secure-provider-runtime.md`](docs/bn-12-secure-provider-runtime.md).

## Job API e data plane

O BN-13 adiciona no control plane a primeira API de jobs: criacao admin com idempotencia, pull pelo provider via sessao remota autenticada, accept, eventos sequenciados, resultado final, cancelamento e grants de data plane com credencial separada. Desde o BN-14, o pull depende de um lease oferecido pelo scheduler. O Control Plane agora tambem retorna uma especificacao versionada que vincula job, lease, identidade, GPU, imagem digest-pinned, timeout e politica de runtime. Depois do accept exato, o worker consulta um endpoint de controle autenticado pelo `job_id + lease_id`: cancelamento administrativo, perda de autoridade ou silencio excessivo do Control Plane interrompem data plane/executor e fazem cleanup sem enviar um resultado `failed` contraditorio. O provider worker e os executores continuam desconectados para validacao; a conexao remota de producao ainda nao os ativa.

Documento: [`docs/bn-13-job-api-data-plane.md`](docs/bn-13-job-api-data-plane.md).
Contrato adicional: [`docs/provider-job-runner-contract.md`](docs/provider-job-runner-contract.md).

## Scheduler e leases

O BN-14 adiciona `POST /v1/scheduler/run`, `job_leases`, listagem de leases por job/provider e prevencao de dupla reserva por job ou por GPU ativa. O scheduler consome sessao remota, provider/device state e workload eligibility backend-owned para oferecer leases curtos a jobs ja criados e ja vinculados a provider/device/session/GPU. A assignment persiste seu `assignment_lease_id`, e o accept exige esse ID exato; acknowledgements antigos retornam `409` sem alterar uma assignment mais nova.

Documento: [`docs/bn-14-scheduler-leases.md`](docs/bn-14-scheduler-leases.md).

## Metering e usage ledger

O BN-15 adiciona `usage_ledger_entries`, recibos de uso por job terminal, hash canonico do recibo, listagem por job/provider e finalize idempotente para backfill admin. Ele mede GPU-seconds, bytes de artefatos, retries e classificacao inicial de falha, mas nao cobra cliente, nao paga provider e nao cria ledger financeiro.

Documento: [`docs/bn-15-metering-usage-ledger.md`](docs/bn-15-metering-usage-ledger.md).

## Marketplace registry e listings

O BN-16 adiciona `marketplace_listings`, sweep admin em `POST /v1/marketplace/listings/sweep`, listagem global publicada em `GET /v1/marketplace/listings` e inspecao por provider em `GET /v1/providers/{provider_id}/marketplace-listings`. Listings so publicam GPU/VRAM como verificados quando proof backend e benchmark sucedido estao vinculados ao GPU UUID observado. Preco fica explicitamente `not_configured_bn16`.

Documento: [`docs/bn-16-marketplace-registry-listings.md`](docs/bn-16-marketplace-registry-listings.md).

O BN-17 adiciona `organizations`, `organization_users`, `projects`, `project_quotas`, `customer_api_keys`, `customer_credit_ledger_entries`, `marketplace_reservations` e `customer_audit_events`. Admin cria usuarios, organizacoes, projetos, quotas, creditos e API keys; customers usam API key para criar/listar/cancelar reservas e consultar usage. Reservas exigem listing backend-published, quota de projeto e `Idempotency-Key`. Billing real com gateway externo ainda nao e executado no BN-17; o BN-18 adiciona a camada financeira backend-owned.

Documento: [`docs/bn-17-customer-accounts-reservations.md`](docs/bn-17-customer-accounts-reservations.md).

## Billing, Pix e payouts

O BN-18 adiciona `marketplace_listing_prices`, `pix_payment_intents`, `financial_ledger_lines`, `billing_invoices`, `provider_payout_accounts` e `provider_payouts`. A cobranca usa usage ledger BN-15 + reservation BN-17 + price book BN-18 + saldo confirmado do projeto; Pix intents so alteram saldo quando confirmados por admin/adapter; provider payouts exigem payable balance, KYC/tax verificados, minimum payout e hold policy. Ainda nao ha chamada a gateway Pix real, webhook assinado, checkout UI ou execucao bancaria automatica.

Documento: [`docs/bn-18-billing-pix-payouts.md`](docs/bn-18-billing-pix-payouts.md).

## Observability e SRE

O BN-19 adiciona logs JSON estruturados, propagation de `x-burd-correlation-id`, metricas Prometheus em `GET /metrics`, snapshot operacional admin em `GET /v1/observability/snapshot`, contadores de erros de tarefas de fundo e SLO status configuravel para disponibilidade HTTP e p95 de latencia recente. Ele nao substitui auditoria de negocio, nao exporta OpenTelemetry ainda e nao automatiza backup/restore.

Documento: [`docs/bn-19-observability-sre.md`](docs/bn-19-observability-sre.md).

## Security hardening e attestation

O BN-20 adiciona `GET /v1/security/policy`, `POST /v1/sessions/{session_id}/security-posture` e `GET /v1/providers/{provider_id}/security-postures`. O agent submete uma postura assinada por chave ativa e vinculada a provider, device, sessao, fingerprint e hash canonico; o backend verifica assinatura, binding e policy, persiste historico imutavel e classifica como `verified` ou `needs_hardening`.

O BN-20 ainda nao implementa TPM/HSM/OS keychain real, verifier remoto de quote, signed updater, secret manager, geracao de SBOM, vulnerability scanner ou supply-chain scanning externo.

Documento: [`docs/bn-20-security-hardening-attestation.md`](docs/bn-20-security-hardening-attestation.md).

## BN-21 - Multi-GPU Inventory Foundation

- `burd-protocol` define signed device GPU inventory payloads, per-GPU inventory rows, canonical inventory hashing, and signature-message binding.
- PostgreSQL migrations `0019_multi_gpu_inventory` and `0029_gpu_inventory_authoritative_snapshots` persist immutable signed snapshot envelopes with zero to 32 per-GPU rows, append-only enforcement, and backend ingestion ordering.
- The control plane exposes `POST /v1/sessions/{session_id}/gpu-inventory` and `GET /v1/providers/{provider_id}/gpu-inventory`.
- Job creation, scheduler selection, assignment and acceptance require the requested GPU UUID in the latest snapshot by backend `ingest_seq`; signed `gpus=[]` removes all historical GPUs from current supply.
- BN-21 does not implement distributed placement, cluster orchestration, or GPU reservation across multiple providers.

Documento: [`docs/bn-21-multi-gpu-foundation.md`](docs/bn-21-multi-gpu-foundation.md).

## Histórico

Comandos:

```bash
burd-agent history --json
burd-agent history latest --json
burd-agent history export --output history.json
```

O histórico deve armazenar resumos de benchmark e evidências públicas.

O histórico não deve conter:

```txt
private keys
api tokens
raw credentials
secrets
```

---

## Payload de registro

Comandos:

```bash
burd-agent registration-payload --json
burd-agent registration-payload --output registration.json
```

O payload de registro é uma estrutura local para futura validação externa.

Ele pode conter:

* identidade pública;
* hash do relatório assinado;
* score;
* tier;
* capabilities;
* pricing demonstrativo;
* resumo de verificação.

Ele não deve submeter dados automaticamente.
Ele não deve incluir segredos.

---

## Regras de segurança

Nunca exponha, registre em log ou commite:

```txt
private_key
private_key_path
secret_key_base64
api_token
api_token_hash
Authorization header
credentials
password
valor bruto de token
```

Rótulos seguros são permitidos:

```txt
configurado
ausente
inválido
rotacionado
ativado
desativado
```

Arquivos públicos, payloads, logs, raw data e snapshots devem aplicar redaction quando necessário.

---

## Diretrizes para Pull Request

Antes de abrir um Pull Request, confirme:

* a alteração tem um objetivo claro;
* os comandos com `--json` continuam retornando JSON válido;
* relatórios não expõem secrets;
* raw data não expõe secrets;
* readiness reflete checks reais;
* challenge válido é persistido corretamente;
* evidências expiradas não contam como válidas;
* histórico não contém credenciais;
* mudanças de contrato JSON foram intencionais;
* testes relevantes foram executados;
* arquivos temporários não foram commitados.

Rode:

```bash
cargo fmt --all --check
cargo test --workspace
cargo build
```

Se a alteração afetar API local, rode também:

```powershell
.\scripts\test-api.ps1
```

Se a alteração afetar validação local, rode:

```powershell
.\scripts\test-local.ps1
```

---

## Checklist de Pull Request

* [ ] A alteração tem propósito claro.
* [ ] `cargo fmt --all --check` passa.
* [ ] `cargo test --workspace` passa.
* [ ] `cargo build` passa.
* [ ] JSON de comandos com `--json` continua válido.
* [ ] Nenhum segredo é exposto.
* [ ] Nenhum token é registrado em log.
* [ ] Raw/config continuam com redaction.
* [ ] Readiness reflete checks reais.
* [ ] Challenge válido é persistido quando necessário.
* [ ] Evidências expiradas são tratadas corretamente.
* [ ] Arquivos temporários não foram commitados.
* [ ] Mensagem de commit segue a convenção do projeto.

---

## Convenção de commits

Use mensagens semânticas curtas:

```txt
tipo: descrição curta
```

Tipos aceitos:

```txt
feat
fix
docs
style
chore
test
perf
refactor
```

Exemplos:

```txt
feat: adiciona persistência de challenge local
fix: corrige cálculo de readiness
fix: preserva redaction em raw data
docs: atualiza guia do benchmark
test: adiciona contrato de relatório assinado
chore: atualiza fixtures de snapshot
refactor: simplifica geração de score
```

Evite mensagens genéricas como:

```txt
update
ajustes
correções
final
```

---

## Não commitar

Não commite:

```txt
target/
tmp/
logs/
.env
.env.*
*.log
challenge.json
signed-response.json
registration.json
history.json
latest-challenge-response.json
benchmark-history.json
agent.json
agent.key
```

Também não commite:

```txt
segredos locais
estado local
credenciais
tokens
chaves privadas
relatórios gerados locais
payloads locais de teste
arquivos temporários de challenge
```

---

## Notas para mantenedores

Ao revisar mudanças, preste atenção especial em:

* contratos JSON;
* validade de relatórios assinados;
* expiração de evidências;
* persistência de challenge;
* cálculo de readiness;
* redaction de raw/config;
* status do token local;
* compatibilidade da API local;
* efeitos em histórico e payload de registro;
* mensagens de erro de comandos CLI;
* separação entre evidência local e aprovação externa.

Um Pull Request que exponha segredos, quebre JSON válido, confunda readiness local com aprovação externa ou altere contratos sem justificativa não deve ser mesclado.

---

## Licença

Este projeto é licenciado sob a licença **MIT**.

Consulte o arquivo [`LICENSE`](./LICENSE) para mais detalhes.
