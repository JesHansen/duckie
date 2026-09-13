# Performance notes

## V1 budget

On 13 September 2026, the user changed the v1 memory requirement to **below 500 MB**. Use 500,000,000 bytes as the conservative ceiling. The proposal's 80 MiB figure is superseded for v1. Account for the desktop and any active test worker when validating workload peaks.

## Initial Windows smoke measurements

Windows 11 Pro 10.0.26200; Ryzen 7 5800X with 16 logical processors; NVIDIA GTX 1080 Ti, driver 32.0.15.8180; 125% display scaling. Release builds outside a debugger, empty scratch workspace, no response or test worker.

| Renderer | Sampled idle private memory | Result |
| --- | ---: | --- |
| wgpu | 374.3 MiB | Under revised ceiling; higher baseline |
| glow / OpenGL | 146.3 MiB | Selected for the development preview |

The glow sample was 153,415,680 private bytes and approximately 119.7 MiB working set.

Release binary sizes at the end of 13 September 2026, built **without** the development screenshot feature: desktop 13,111,808 bytes, worker 1,401,856 bytes. The packaged ZIP (`scripts/package.ps1`, which also emits `THIRD_PARTY_NOTICES.md`) is 6,303,547 bytes. The earlier 13,130,752-byte desktop figure included the screenshot feature; packaging now refuses a binary that still contains it.

The idle memory samples above predate that day's editor, storage and import changes and have not been retaken. Re-measure before treating them as current.

Raw idle samples are in `artifacts/idle-start.json`, `artifacts/idle-cpu-start.json`, and `artifacts/idle-result.json`. Test processes were stopped when the user paused the session.

These are initial samples, not release acceptance. Still measure startup p95, restored collections, active-worker overhead, 50 MiB decoded/encoded response handling and GUI peak memory, input latency, GPU allocations, and representative corporate-network behavior. Follow the protocol in ARCHITECTURE.md with the user-approved memory ceiling.
