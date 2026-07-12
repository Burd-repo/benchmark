# Benchmark Profiles

Profiles map detected VRAM to suggested benchmark behavior and marketplace
workload expectations.

## profile_8gb

- Minimum VRAM: 8 GB
- Suggested model: `llama3.2:1b`
- Runtime: Ollama
- Workloads: light LLMs, embeddings, Whisper, agents simples
- Not recommended: SDXL, fine-tuning, enterprise batch inference

## profile_12gb

- Minimum VRAM: 12 GB
- Suggested model: `llama3.2:3b`
- Runtime: Ollama
- Workloads: small quantized LLMs, basic Stable Diffusion, Whisper, embeddings,
  lightweight bots and agents

## profile_16gb

- Minimum VRAM: 16 GB
- Suggested model: `qwen2.5:7b`
- Runtime: Ollama
- Workloads: medium quantized LLMs, ComfyUI basic workflows, batch inference
  pequeno

## profile_24gb

- Minimum VRAM: 24 GB
- Suggested model: `qwen2.5:14b`
- Runtime: Ollama or vLLM
- Workloads: SDXL, ComfyUI, medium quantized LLMs, fast agents, small batch
  inference

## profile_48gb

- Minimum VRAM: 48 GB
- Suggested model: `qwen2.5:32b`
- Runtime: vLLM
- Workloads: larger LLM inference, multi-user inference, batch inference,
  advanced ComfyUI

## profile_80gb

- Minimum VRAM: 80 GB
- Suggested model: `Qwen/Qwen2.5-72B-Instruct`
- Runtime: vLLM
- Workloads: fine-tuning, enterprise inference, larger models and high-throughput
  batch jobs

Profiles are MVP guidance. Final marketplace profiles should be tuned with real
Burd production measurements.

## Backend Profile Registry

The VRAM-tier profiles above are local MVP guidance. BN-10 adds the backend-owned Benchmark Profiles v2 registry in the control plane:

- admins/control-plane policy create versioned profiles through `POST /v1/benchmark-profiles`;
- providers submit signed benchmark results through `POST /v1/sessions/{session_id}/benchmark-results`;
- backend verification binds result hash, active device key, remote session, hardware fingerprint, GPU UUID, image digest, optional model hash, and thresholds;
- accepted benchmark result history is an input for future policy and scheduler work, not marketplace approval by itself.

See [`docs/bn-10-benchmark-profiles-v2.md`](bn-10-benchmark-profiles-v2.md).