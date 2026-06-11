# Marketplace GPU Policy

The local marketplace policy snapshot is named `nvidia_cuda_only_mvp`.

It is a product eligibility policy for a future paid marketplace. It does not
disable local hardware detection, local benchmarks, fit analysis, or diagnostic
reports for unsupported hardware.

## Eligible Hardware

A provider is potentially marketplace eligible only when all current checks
pass:

- every detected GPU is NVIDIA;
- the GPU is RTX 30xx or newer, or a recognized compatible datacenter class;
- CUDA is available and is the detected GPU backend;
- VRAM is present;
- VRAM source is present;
- VRAM confidence is `detected`.

Recognized initial datacenter classes include T4, A10/A10G, A30, A40, A100, L4,
L40/L40S, H100, H200, and B200.

## Diagnostic-Only Hardware

AMD, Intel, Apple Silicon, ROCm, Vulkan-only, and NVIDIA hardware without a
usable CUDA backend remain available for local diagnostics where existing Burd
and llmfit behavior supports them. They receive:

```json
{
  "marketplace_eligible": false,
  "eligibility_level": "local_diagnostic_only",
  "reasons": [
    "Marketplace MVP requires NVIDIA CUDA GPUs"
  ]
}
```

CPU-only systems, unsupported/unknown NVIDIA classes, and NVIDIA systems with
missing or unreliable VRAM evidence are not marketplace eligible.

## Contract

```json
{
  "marketplace_eligible": true,
  "eligibility_level": "marketplace_eligible",
  "gpu_policy": "nvidia_cuda_only_mvp",
  "requires_nvidia": true,
  "requires_cuda": true,
  "requires_detected_vram": true,
  "minimum_class": "rtx_30xx_or_datacenter",
  "reasons": []
}
```

The snapshot appears in full/signed reports, provider details, registration
payloads, and `burd-agent fingerprint --json`.

Marketplace policy is separate from Compute Score and local Readiness. A strong
AMD GPU may keep useful local compute diagnostics while remaining ineligible
for the paid marketplace MVP.
