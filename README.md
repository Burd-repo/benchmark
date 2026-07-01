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
