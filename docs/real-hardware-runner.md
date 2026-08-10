# Real Hardware Runner

The real-hardware integration workflow is intentionally separate from default
CI. This repository is public, so the runner must be dedicated, isolated, and
started only for an approved manual dispatch.

GitHub recommends self-hosted runners primarily for private repositories and
recommends ephemeral runners over persistent runners. References:

- https://docs.github.com/en/actions/how-tos/managing-self-hosted-runners/adding-self-hosted-runners
- https://docs.github.com/actions/reference/runners/self-hosted-runners
- https://docs.github.com/en/actions/hosting-your-own-runners/managing-self-hosted-runners/using-labels-with-self-hosted-runners

## Repository Configuration

The workflow:

- runs only through `workflow_dispatch`;
- refuses every ref except `refs/heads/main`;
- requires the operator to select `RUN`;
- keeps generic detection on `[self-hosted, Windows, X64, burd-hardware]`;
- targets physical gates only on dedicated `burd-nvidia-linux` or
  `burd-nvidia-windows` runners;
- uses the protected `real-hardware` environment;
- grants only `contents: read`;
- disables persisted checkout credentials;
- isolates generic detection state under `runner.temp`;
- requires digest-pinned gate images already present on NVIDIA runners;
- uploads sanitized physical-gate evidence for the evaluated commit.

In GitHub repository settings, configure the `real-hardware` environment with
required reviewers before using the runner.

## Runner Machine

Use a dedicated Windows account and machine without personal credentials,
wallets, cloud credentials, production keys, or a real `~/.burd` state.
Do not use the runner for workflows triggered by pull requests or forks.

Create a fresh repository runner registration token in:

`Settings > Actions > Runners > New self-hosted runner`

Registration and removal tokens are short-lived administrative credentials.
Never commit them, paste them into saved scripts, or expose them in logs. The
bootstrap passes the session-only environment value to GitHub's `config.cmd`
without printing it.

Set it only for the current PowerShell session, then run the bootstrap:

```powershell
$env:GITHUB_RUNNER_REGISTRATION_TOKEN = "<fresh-registration-token>"
.\scripts\configure-hardware-runner.ps1
Remove-Item Env:\GITHUB_RUNNER_REGISTRATION_TOKEN
```

The script downloads the latest official Windows x64 GitHub Actions runner,
registers the `burd-hardware` label, and configures an ephemeral runner by
default. It does not start a service or long-running process.

For a dedicated Windows/WSL2 NVIDIA gate runner, add its isolated label:

```powershell
$env:GITHUB_RUNNER_REGISTRATION_TOKEN = "<fresh-registration-token>"
.\scripts\configure-hardware-runner.ps1 `
  -RunnerRoot C:\burd-actions-runner-nvidia `
  -AdditionalLabels burd-nvidia-windows
Remove-Item Env:\GITHUB_RUNNER_REGISTRATION_TOKEN
```

Configure a Linux runner from the official GitHub Linux instructions and add
`burd-nvidia-linux`. Do not give a generic runner either NVIDIA label. Both
NVIDIA runners require Docker, the NVIDIA container runtime, at least two
physical NVIDIA GPUs, the exact pre-pulled gate images, and no personal or
production credentials.

The protected environment variables and physical promotion rule are documented
in `physical-nvidia-gates.md`. A successful workflow artifact must be retained
and reviewed; runner registration or harness availability alone does not mark a
platform verified.

Start the runner manually immediately before dispatching the workflow:

```powershell
C:\burd-actions-runner\run.cmd
```

An ephemeral runner deregisters after one job. Generate a new registration
token and run the bootstrap again for the next real-hardware validation.

## Removal

Create a fresh removal token in the runner settings, then run:

```powershell
$env:GITHUB_RUNNER_REMOVE_TOKEN = "<fresh-removal-token>"
.\scripts\remove-hardware-runner.ps1
Remove-Item Env:\GITHUB_RUNNER_REMOVE_TOKEN
```

The removal script unregisters the runner but intentionally leaves local files
for manual inspection and deletion.
