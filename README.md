<div align="left">

<a href="https://burd.ia">
  <img src="./public/burd-logo.svg" alt="Logo da Burd" title="Burd Benchmark" height="48" />
</a>

<br />

<p>
  Produto local de validaÃ§Ã£o da Burd para detectar hardware, executar benchmarks, calcular score, gerar relatÃ³rios assinados e preparar evidÃªncias locais para providers de compute.
</p>

[![status](https://img.shields.io/badge/status-active-2C5E8A)](https://github.com/Burd-repo/benchmark)
[![license](https://img.shields.io/badge/license-MIT-green)](./LICENSE)
[![agent](https://img.shields.io/badge/agent-burd--agent-blue)](https://github.com/Burd-repo/benchmark)
[![validation](https://img.shields.io/badge/provider-validation-lightgrey)](https://github.com/Burd-repo/benchmark)

</div>

---

## SumÃ¡rio

* [VisÃ£o geral](#visÃ£o-geral)
* [InÃ­cio rÃ¡pido](#inÃ­cio-rÃ¡pido)
* [Build](#build)
* [Comandos principais](#comandos-principais)
* [API local](#api-local)
* [Identidade do Provider](#identidade-do-provider)
* [RelatÃ³rios assinados](#relatÃ³rios-assinados)
* [Challenge local](#challenge-local)
* [Readiness](#readiness)
* [Score](#score)
* [AI Performance Metrics](#ai-performance-metrics)
* [Network score](#network-score)
* [Reliability e uptime](#reliability-e-uptime)
* [HistÃ³rico](#histÃ³rico)
* [Payload de registro](#payload-de-registro)
* [Regras de seguranÃ§a](#regras-de-seguranÃ§a)
* [Diretrizes para Pull Request](#diretrizes-para-pull-request)
* [Checklist de Pull Request](#checklist-de-pull-request)
* [ConvenÃ§Ã£o de commits](#convenÃ§Ã£o-de-commits)
* [NÃ£o commitar](#nÃ£o-commitar)
* [Notas para mantenedores](#notas-para-mantenedores)
* [LicenÃ§a](#licenÃ§a)

---

## VisÃ£o geral

O **Burd Benchmark** gera o binÃ¡rio `burd-agent`, responsÃ¡vel pela validaÃ§Ã£o local de mÃ¡quinas que desejam atuar como providers.

O agent Ã© responsÃ¡vel por:

* detectar hardware local;
* identificar GPU, VRAM, CPU, RAM, disco e drivers;
* estimar compatibilidade com workloads de IA;
* executar benchmarks locais;
* calcular o Burd Compute Score;
* gerar relatÃ³rios assinados;
* verificar relatÃ³rios;
* criar e validar challenges locais;
* registrar histÃ³rico local;
* calcular readiness;
* calcular reliability local a partir do historico de heartbeat;
* calcular network score local a partir de benchmark finito;
* expor uma API local para integraÃ§Ã£o com interfaces.

Este repositÃ³rio nÃ£o Ã© uma landing page institucional.
O foco aqui Ã© validaÃ§Ã£o local, contratos de dados, score, evidÃªncias e API do provider.

---

## InÃ­cio rÃ¡pido

```bash
git clone https://github.com/Burd-repo/benchmark.git
cd benchmark
cargo build
```

No Windows PowerShell:

```powershell
.\target\debug\burd-agent.exe --help
```

ValidaÃ§Ã£o local rÃ¡pida:

```powershell
.\scripts\test-local.ps1
```

Esse script executa verificaÃ§Ãµes seguras e rÃ¡pidas. Ele nÃ£o inicia o servidor local, nÃ£o roda loops de heartbeat e nÃ£o executa benchmarks pesados.

---

## Build

### Build padrÃ£o

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

### FormataÃ§Ã£o

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

### RelatÃ³rios

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

### SessÃ£o local

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
burd-agent registration-payload --json
```

### HistÃ³rico e logs

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

Todos os comandos com `--json` devem escrever JSON vÃ¡lido em `stdout`, sem misturar logs no mesmo output.

---

## API local

Para iniciar a API local:

```powershell
.\target\debug\burd-agent.exe serve --host 127.0.0.1 --port 8787
```

A API fica disponÃ­vel em:

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

A identidade local Ã© usada para assinar relatÃ³rios e comprovar a origem das evidÃªncias geradas pela mÃ¡quina.

Comandos:

```bash
burd-agent identity init
burd-agent identity show --json
```

O agent grava configuraÃ§Ã£o pÃºblica e mantÃ©m a chave privada separada.

Regras:

* relatÃ³rios nÃ£o devem expor chave privada;
* raw data nÃ£o deve expor chave privada;
* payloads pÃºblicos nÃ£o devem incluir secrets;
* migraÃ§Ãµes devem preservar evidÃªncias vÃ¡lidas;
* mudanÃ§as de identidade devem ser explÃ­citas.

---

## RelatÃ³rios assinados

Para gerar um relatÃ³rio assinado completo:

```bash
burd-agent report --run-all --signed --json
```

Um relatÃ³rio assinado pode conter:

* hash canÃ´nico do relatÃ³rio;
* assinatura;
* chave pÃºblica;
* algoritmo da chave;
* timestamp de assinatura;
* resultado de verificaÃ§Ã£o local;
* versÃ£o de canonicalizaÃ§Ã£o;
* resumo de hardware;
* score;
* evidÃªncias do benchmark.

Regras:

* `report --run-all` deve registrar histÃ³rico local;
* `report --run-all --signed` deve registrar histÃ³rico local;
* relatÃ³rios expirados nÃ£o devem contar como evidÃªncia vÃ¡lida;
* assinatura invÃ¡lida deve bloquear readiness;
* relatÃ³rio assinado nÃ£o deve conter segredo.

---

## Challenge local

O challenge local valida evidÃªncias por meio de nonce, expiraÃ§Ã£o, assinatura e relatÃ³rio assinado.

Comando recomendado:

```bash
burd-agent challenge run-local --json
```

Esse comando cria, executa, assina, verifica e persiste a evidÃªncia local do challenge sem exigir arquivo intermediÃ¡rio.

Regras:

* challenge expirado nÃ£o deve contar ponto de readiness;
* nonce deve ser validado;
* assinatura do relatÃ³rio deve ser validada;
* assinatura da resposta do challenge deve ser validada;
* evidÃªncia vÃ¡lida deve ser persistida;
* histÃ³rico sozinho nÃ£o substitui evidÃªncia de challenge vÃ¡lida.

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
* relatÃ³rio assinado;
* challenge;
* provider verification;
* histÃ³rico;
* token da API local;
* redaction de raw data;
* validade e expiraÃ§Ã£o das evidÃªncias.

Estados esperados:

```txt
ready_locally
partial
failed
not_verified
uninitialized
```

`ready_locally` significa que os checks locais passaram.
NÃ£o significa aprovaÃ§Ã£o externa, auditoria, listagem em marketplace ou garantia de receita.

---

## Score

O Burd Compute Score Ã© uma pontuaÃ§Ã£o de `0` a `100`.

Pesos do MVP:

```txt
40% benchmark LLM real ou fallback estimado
20% VRAM e capacidade
15% estabilidade
10% rede
10% disco
5% sinais de verificaÃ§Ã£o
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

PreÃ§os e ganhos demonstrativos nÃ£o devem ser tratados como promessa de receita.

---

## AI Performance Metrics

O comando burd-agent ai-performance --json consolida metricas de performance de IA sem executar benchmark pesado automaticamente.

A API local expoe o mesmo contrato em GET /api/v1/ai-performance.

O relatorio separa metricas medidas (real_benchmark, signed_report, benchmark_history) de estimativas (fit_estimate) e dados ausentes (not_measured). Campos sem medicao confiavel retornam null ou origem not_measured/unavailable; estimativas de fit nunca sao tratadas como benchmark real. Evidencia expirada permanece visivel com is_expired true, warning e confianca reduzida.

Este recurso nao inicia runtime externo, nao executa Proof of Capability remoto, nao contata backend, nao aprova marketplace e nao cria scheduler, jobs, leases, billing ou payouts.

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

## HistÃ³rico

Comandos:

```bash
burd-agent history --json
burd-agent history latest --json
burd-agent history export --output history.json
```

O histÃ³rico deve armazenar resumos de benchmark e evidÃªncias pÃºblicas.

O histÃ³rico nÃ£o deve conter:

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

O payload de registro Ã© uma estrutura local para futura validaÃ§Ã£o externa.

Ele pode conter:

* identidade pÃºblica;
* hash do relatÃ³rio assinado;
* score;
* tier;
* capabilities;
* pricing demonstrativo;
* resumo de verificaÃ§Ã£o.

Ele nÃ£o deve submeter dados automaticamente.
Ele nÃ£o deve incluir segredos.

---

## Regras de seguranÃ§a

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

RÃ³tulos seguros sÃ£o permitidos:

```txt
configurado
ausente
invÃ¡lido
rotacionado
ativado
desativado
```

Arquivos pÃºblicos, payloads, logs, raw data e snapshots devem aplicar redaction quando necessÃ¡rio.

---

## Diretrizes para Pull Request

Antes de abrir um Pull Request, confirme:

* a alteraÃ§Ã£o tem um objetivo claro;
* os comandos com `--json` continuam retornando JSON vÃ¡lido;
* relatÃ³rios nÃ£o expÃµem secrets;
* raw data nÃ£o expÃµe secrets;
* readiness reflete checks reais;
* challenge vÃ¡lido Ã© persistido corretamente;
* evidÃªncias expiradas nÃ£o contam como vÃ¡lidas;
* histÃ³rico nÃ£o contÃ©m credenciais;
* mudanÃ§as de contrato JSON foram intencionais;
* testes relevantes foram executados;
* arquivos temporÃ¡rios nÃ£o foram commitados.

Rode:

```bash
cargo fmt --all --check
cargo test --workspace
cargo build
```

Se a alteraÃ§Ã£o afetar API local, rode tambÃ©m:

```powershell
.\scripts\test-api.ps1
```

Se a alteraÃ§Ã£o afetar validaÃ§Ã£o local, rode:

```powershell
.\scripts\test-local.ps1
```

---

## Checklist de Pull Request

* [ ] A alteraÃ§Ã£o tem propÃ³sito claro.
* [ ] `cargo fmt --all --check` passa.
* [ ] `cargo test --workspace` passa.
* [ ] `cargo build` passa.
* [ ] JSON de comandos com `--json` continua vÃ¡lido.
* [ ] Nenhum segredo Ã© exposto.
* [ ] Nenhum token Ã© registrado em log.
* [ ] Raw/config continuam com redaction.
* [ ] Readiness reflete checks reais.
* [ ] Challenge vÃ¡lido Ã© persistido quando necessÃ¡rio.
* [ ] EvidÃªncias expiradas sÃ£o tratadas corretamente.
* [ ] Arquivos temporÃ¡rios nÃ£o foram commitados.
* [ ] Mensagem de commit segue a convenÃ§Ã£o do projeto.

---

## ConvenÃ§Ã£o de commits

Use mensagens semÃ¢nticas curtas:

```txt
tipo: descriÃ§Ã£o curta
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
feat: adiciona persistÃªncia de challenge local
fix: corrige cÃ¡lculo de readiness
fix: preserva redaction em raw data
docs: atualiza guia do benchmark
test: adiciona contrato de relatÃ³rio assinado
chore: atualiza fixtures de snapshot
refactor: simplifica geraÃ§Ã£o de score
```

Evite mensagens genÃ©ricas como:

```txt
update
ajustes
correÃ§Ãµes
final
```

---

## NÃ£o commitar

NÃ£o commite:

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

TambÃ©m nÃ£o commite:

```txt
segredos locais
estado local
credenciais
tokens
chaves privadas
relatÃ³rios gerados locais
payloads locais de teste
arquivos temporÃ¡rios de challenge
```

---

## Notas para mantenedores

Ao revisar mudanÃ§as, preste atenÃ§Ã£o especial em:

* contratos JSON;
* validade de relatÃ³rios assinados;
* expiraÃ§Ã£o de evidÃªncias;
* persistÃªncia de challenge;
* cÃ¡lculo de readiness;
* redaction de raw/config;
* status do token local;
* compatibilidade da API local;
* efeitos em histÃ³rico e payload de registro;
* mensagens de erro de comandos CLI;
* separaÃ§Ã£o entre evidÃªncia local e aprovaÃ§Ã£o externa.

Um Pull Request que exponha segredos, quebre JSON vÃ¡lido, confunda readiness local com aprovaÃ§Ã£o externa ou altere contratos sem justificativa nÃ£o deve ser mesclado.

---

## LicenÃ§a

Este projeto Ã© licenciado sob a licenÃ§a **MIT**.

Consulte o arquivo [`LICENSE`](./LICENSE) para mais detalhes.
