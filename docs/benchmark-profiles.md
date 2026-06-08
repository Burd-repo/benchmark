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
