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
- requires the operator to select `RUN`;
- targets `[self-hosted, Windows, X64, burd-hardware]`;
- uses the protected `real-hardware` environment;
- grants only `contents: read`;
- disables persisted checkout credentials;
- isolates `BURD_AGENT_HOME` and `BURD_AGENT_CONFIG` under `runner.temp`;
- removes temporary Burd state after the job.

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
