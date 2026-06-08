# Notices

This repository builds the Burd Benchmark and Burd Agent.

## llmfit

The technical foundation for hardware detection, model fit analysis, model
metadata, runtime provider probing, and real inference benchmark helpers is
adapted from or linked against the open-source `llmfit` project:

- Project: https://github.com/AlexsJones/llmfit
- Author: Alex Jones
- License: MIT
- Copyright: Copyright (c) 2026 Alex Jones

The vendored reference copy is kept under `third_party/llmfit`. The Burd crates
currently depend on `third_party/llmfit/llmfit-core` through a path dependency
and add Burd-specific scoring, reports, antifraud protocol, local identity,
local API, and benchmark UI around it.

The original MIT license text for `llmfit` is preserved at
`third_party/llmfit/LICENSE`.

## Burd additions

Burd-specific code in `crates/`, `apps/`, and `docs/` adds:

- provider validation reports;
- Burd Compute Score and marketplace-oriented tiers;
- demonstrative BRL/hour pricing;
- local identity and future challenge/signature protocol;
- local API endpoints for dashboard integration;
- Burd-branded benchmark report UI.

Prices and marketplace eligibility in this repository are demonstrative until a
real Burd backend, antifraud verification, and marketplace policy are connected.
