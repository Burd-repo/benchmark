# Physical NVIDIA Gate Images

These images exist only for the ignored physical NVIDIA integration gates. They
are not workload images and are never pulled by the Burd Agent.

Both builds require an explicitly digest-pinned Linux CUDA base image:

```bash
docker build \
  --build-arg CUDA_BASE_IMAGE='nvidia/cuda:<version>@sha256:<64-hex-digest>' \
  --build-arg BURD_GATE_MODE=report \
  -t burd-physical-nvidia-report:local \
  tools/physical-nvidia-gate

docker build \
  --build-arg CUDA_BASE_IMAGE='nvidia/cuda:<version>@sha256:<64-hex-digest>' \
  --build-arg BURD_GATE_MODE=stubborn \
  -t burd-physical-nvidia-stubborn:local \
  tools/physical-nvidia-gate
```

Push the images to the controlled registry, resolve their immutable repository
digests, and pre-pull those exact references on the dedicated runner. Configure
the `real-hardware` environment variables with repository references ending in
`@sha256:<64-hex-digest>`; mutable local tags do not pass the gate.

`report` prints `nvidia-smi -L` and exits. `stubborn` prints the same inventory,
ignores `TERM`, and remains active until the executor reaches its bounded
`KILL` path. The separate runtime-proof image remains responsible for proving a
real CUDA operation and its signed output contract.
