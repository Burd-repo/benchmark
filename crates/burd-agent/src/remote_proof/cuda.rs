use super::{ProofExecution, ProofExecutionRequest};
use burd_hardware::collect_nvidia_telemetry;
use burd_protocol::{GpuTelemetrySample, ProofCapabilityChallenge, ProofCapabilityMetrics};
use cudarc::cublas::{CudaBlas, Gemm, GemmConfig};
use cudarc::driver::CudaContext;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Instant;

const CUDA_RESIDENCY_TARGET_MIB: usize = 64;
const CUDA_RESIDENCY_MINIMUM_MIB: usize = 16;
const GEMM_DIMENSION: usize = 2048;
const GEMM_ITERATIONS: usize = 6;

pub(crate) fn execute_remote_proof(
    request: ProofExecutionRequest,
) -> Result<ProofExecution, String> {
    match catch_unwind(AssertUnwindSafe(|| execute_remote_proof_inner(request))) {
        Ok(result) => result,
        Err(_) => Err(
            "CUDA proof runtime failed while loading or invoking a required shared library"
                .to_string(),
        ),
    }
}

fn execute_remote_proof_inner(
    mut request: ProofExecutionRequest,
) -> Result<ProofExecution, String> {
    ensure_cuda_libraries(&request.challenge)?;
    let baseline = collect_nvidia_telemetry(1)?;
    let (context, sample) = select_cuda_device(
        &baseline.samples,
        request.challenge.required_gpu_uuid.as_deref(),
    )?;
    let gpu_uuid = sample.gpu_uuid.clone();
    let cuda_runtime_version = format_cuda_version(
        cudarc::runtime::result::version::get_runtime_version()
            .map_err(|error| format!("CUDA runtime version query failed: {error}"))?,
    )?;
    let cuda_driver_version = format_cuda_version(
        cudarc::runtime::result::version::get_driver_version()
            .map_err(|error| format!("CUDA driver version query failed: {error}"))?,
    )?;
    let stream = context.default_stream();
    let (free_bytes, _) = context
        .mem_get_info()
        .map_err(|error| format!("CUDA memory query failed: {error}"))?;
    let free_mib = free_bytes / (1024 * 1024);
    let allocation_mib = CUDA_RESIDENCY_TARGET_MIB
        .min(free_mib / 4)
        .max(CUDA_RESIDENCY_MINIMUM_MIB);
    if free_mib < allocation_mib.saturating_mul(2) {
        return Err(format!(
            "insufficient free VRAM for capability proof: {free_mib} MiB available"
        ));
    }
    let residency = stream
        .alloc_zeros::<u8>(allocation_mib * 1024 * 1024)
        .map_err(|error| format!("CUDA VRAM allocation proof failed: {error}"))?;
    context
        .synchronize()
        .map_err(|error| format!("CUDA VRAM residency synchronization failed: {error}"))?;

    request.hold_residency_for_telemetry(gpu_uuid.clone())?;

    let gemm_gflops = if requires(&request.challenge, "tensor_gemm_microbenchmark") {
        Some(run_gemm_microbenchmark(&context)?)
    } else {
        None
    };
    let (tokens_per_second, ttft_ms) = if requires(&request.challenge, "llm_short_inference")
        || request.challenge.min_tokens_per_second > 0.0
        || request.challenge.max_ttft_ms > 0
    {
        let result = super::ollama::run_inference(&request.challenge)?;
        (Some(result.tokens_per_second), Some(result.ttft_ms))
    } else {
        (None, None)
    };
    let contention_detected = sample_has_contention(&sample);
    drop(residency);

    let mut backend_proofs = vec!["cuda-driver", "cudart", "vram"];
    if gemm_gflops.is_some() {
        backend_proofs.push("cublas-sgemm");
    }
    if tokens_per_second.is_some() {
        backend_proofs.push("ollama-digest-v1");
    }

    Ok(ProofExecution {
        gpu_uuid,
        driver_version: sample.driver_version,
        cuda_driver_version: Some(cuda_driver_version),
        cuda_runtime_version: Some(cuda_runtime_version),
        metrics: ProofCapabilityMetrics {
            tokens_per_second,
            ttft_ms,
            vram_allocated_mib: Some(allocation_mib as u64),
            vram_resident_mib: Some(allocation_mib as u64),
            gemm_gflops,
            cuda_runtime_detected: true,
            backend_proof: backend_proofs.join("+"),
            contention_detected,
        },
    })
}

fn ensure_cuda_libraries(challenge: &ProofCapabilityChallenge) -> Result<(), String> {
    let driver_present = unsafe { cudarc::driver::sys::is_culib_present() };
    if !driver_present {
        return Err("NVIDIA CUDA driver shared library is unavailable".to_string());
    }
    let runtime_present = unsafe { cudarc::runtime::sys::is_culib_present() };
    if !runtime_present {
        return Err("CUDA runtime shared library is unavailable".to_string());
    }
    if requires(challenge, "tensor_gemm_microbenchmark") {
        let cublas_present = unsafe { cudarc::cublas::sys::is_culib_present() };
        if !cublas_present {
            return Err("cuBLAS shared library is unavailable".to_string());
        }
    }
    Ok(())
}

fn select_cuda_device(
    samples: &[GpuTelemetrySample],
    required_gpu_uuid: Option<&str>,
) -> Result<(Arc<CudaContext>, GpuTelemetrySample), String> {
    let device_count = CudaContext::device_count()
        .map_err(|error| format!("CUDA device count failed: {error}"))?;
    if device_count <= 0 {
        return Err("CUDA reported no available devices".to_string());
    }
    for ordinal in 0..device_count as usize {
        let context = CudaContext::new(ordinal)
            .map_err(|error| format!("CUDA context creation failed for GPU {ordinal}: {error}"))?;
        let raw_uuid = context
            .uuid()
            .map_err(|error| format!("CUDA UUID query failed for GPU {ordinal}: {error}"))?;
        let normalized = normalize_gpu_uuid_bytes(&raw_uuid.bytes);
        let sample = samples.iter().find(|sample| {
            normalize_gpu_uuid(&sample.gpu_uuid) == normalized
                && required_gpu_uuid
                    .is_none_or(|required| sample.gpu_uuid.eq_ignore_ascii_case(required))
        });
        if let Some(sample) = sample {
            return Ok((context, sample.clone()));
        }
    }
    match required_gpu_uuid {
        Some(required) => Err(format!(
            "required GPU {required} is not visible through both CUDA and NVIDIA telemetry"
        )),
        None => Err("no GPU is visible through both CUDA and NVIDIA telemetry".to_string()),
    }
}

fn run_gemm_microbenchmark(context: &Arc<CudaContext>) -> Result<f64, String> {
    let stream = context.default_stream();
    let blas = CudaBlas::new(stream.clone())
        .map_err(|error| format!("cuBLAS initialization failed: {error}"))?;
    let elements = GEMM_DIMENSION * GEMM_DIMENSION;
    let input = vec![1.0_f32; elements];
    let a = stream
        .clone_htod(&input)
        .map_err(|error| format!("GEMM input A upload failed: {error}"))?;
    let b = stream
        .clone_htod(&input)
        .map_err(|error| format!("GEMM input B upload failed: {error}"))?;
    let mut c = stream
        .alloc_zeros::<f32>(elements)
        .map_err(|error| format!("GEMM output allocation failed: {error}"))?;
    let config = GemmConfig {
        transa: cudarc::cublas::sys::cublasOperation_t::CUBLAS_OP_N,
        transb: cudarc::cublas::sys::cublasOperation_t::CUBLAS_OP_N,
        m: GEMM_DIMENSION as i32,
        n: GEMM_DIMENSION as i32,
        k: GEMM_DIMENSION as i32,
        alpha: 1.0_f32,
        lda: GEMM_DIMENSION as i32,
        ldb: GEMM_DIMENSION as i32,
        beta: 0.0_f32,
        ldc: GEMM_DIMENSION as i32,
    };
    unsafe { blas.gemm(config, &a, &b, &mut c) }
        .map_err(|error| format!("cuBLAS GEMM warmup failed: {error}"))?;
    context
        .synchronize()
        .map_err(|error| format!("cuBLAS GEMM warmup synchronization failed: {error}"))?;
    let started = Instant::now();
    for _ in 0..GEMM_ITERATIONS {
        unsafe { blas.gemm(config, &a, &b, &mut c) }
            .map_err(|error| format!("cuBLAS GEMM execution failed: {error}"))?;
    }
    context
        .synchronize()
        .map_err(|error| format!("cuBLAS GEMM synchronization failed: {error}"))?;
    let elapsed = started.elapsed().as_secs_f64();
    if elapsed <= 0.0 {
        return Err("cuBLAS GEMM elapsed time is not measurable".to_string());
    }
    let operations = 2.0 * (GEMM_DIMENSION as f64).powi(3) * GEMM_ITERATIONS as f64;
    Ok(operations / elapsed / 1_000_000_000.0)
}

pub(super) fn sample_has_contention(sample: &GpuTelemetrySample) -> bool {
    let current_pid = std::process::id();
    let foreign_process = sample.processes.iter().any(|process| {
        process.pid != current_pid && !is_expected_ollama_process(&process.process_name)
    });
    foreign_process
        || sample
            .gpu_utilization_percent
            .is_some_and(|utilization| utilization > 20.0 && sample.processes.is_empty())
}

fn is_expected_ollama_process(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "ollama" | "ollama.exe" | "ollama_llama_server" | "ollama_llama_server.exe"
    )
}

fn requires(challenge: &ProofCapabilityChallenge, proof: &str) -> bool {
    challenge.required_proofs.iter().any(|item| item == proof)
}

fn normalize_gpu_uuid(value: &str) -> String {
    value
        .trim()
        .strip_prefix("GPU-")
        .unwrap_or(value.trim())
        .chars()
        .filter(|character| character.is_ascii_hexdigit())
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalize_gpu_uuid_bytes(bytes: &[std::ffi::c_char; 16]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{:02x}", *byte as u8))
        .collect()
}

fn format_cuda_version(raw: i32) -> Result<String, String> {
    if raw <= 0 {
        return Err(format!("CUDA returned invalid version {raw}"));
    }
    let major = raw / 1000;
    let minor = (raw % 1000) / 10;
    Ok(format!("{major}.{minor}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuda_versions_use_runtime_integer_encoding() {
        assert_eq!(format_cuda_version(12090).unwrap(), "12.9");
        assert_eq!(format_cuda_version(13000).unwrap(), "13.0");
        assert!(format_cuda_version(0).is_err());
    }

    #[test]
    fn cuda_and_nvidia_uuid_formats_normalize_to_same_value() {
        let raw = [
            0x00_i8, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, -120, -103, -86, -69, -52, -35, -18,
            -1,
        ];
        assert_eq!(
            normalize_gpu_uuid_bytes(&raw),
            normalize_gpu_uuid("GPU-00112233-4455-6677-8899-aabbccddeeff")
        );
    }
}
