# llmfit Adaptation

## Strategy

The chosen strategy is adapter integration.

The Burd repository keeps its own product crates and depends on
`third_party/llmfit/llmfit-core` through a Rust path dependency. This preserves
the mature llmfit core while letting Burd add provider scoring, reports,
antifraud protocol, local identity, local API, and branded UI without turning
the whole repository into a direct fork.

## What Came From llmfit

The following capabilities are reused through `llmfit-core`:

- hardware detection via `SystemSpecs::detect`;
- CPU, RAM, GPU, VRAM and backend detection;
- CUDA, ROCm, Metal, Vulkan, SYCL, CPU and Ascend backend modeling;
- model database and model metadata;
- fit analysis via `ModelFit`;
- dynamic quantization selection;
- estimated tokens per second;
- run modes such as GPU, CPU+GPU, CPU, MoE offload and tensor parallel;
- Ollama, vLLM and MLX benchmark helpers;
- provider/runtime concepts.

## What Burd Adds

Burd-specific crates add:

- Burd-shaped JSON reports;
- provider capability summaries;
- workload classification for marketplace use;
- benchmark profiles by VRAM tier;
- stability, network and disk benchmarks;
- Burd Compute Score;
- demonstrative BRL/hour pricing;
- local identity config;
- challenge and signed report structures;
- local API endpoints;
- benchmark UI following `SKILL.md`.

## License Handling

The llmfit project is MIT licensed. The original license is preserved at
`third_party/llmfit/LICENSE`, and credits are recorded in `NOTICE.md`.

Any future code copied directly from llmfit into Burd-owned crates should keep
the original copyright notice in the adapted file or in a nearby module-level
comment.

## Future Upstream Maintenance

The adapter strategy keeps the Burd layer small. Future llmfit updates can be
pulled into `third_party/llmfit`, then validated with:

```sh
cargo fmt
cargo test
cargo build
```

Breaking changes in `llmfit-core` should be isolated to `crates/burd-llmfit`.
