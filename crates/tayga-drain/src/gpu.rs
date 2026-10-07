//! wgpu compute backend (sub-project 4 spec §3.3): WGSL, so Metal on macOS and Vulkan on Linux.
//! Small batches, batches past the device limits and any GPU error run on the CPU instead.

use crate::fingerprint::{
    BatchFingerprinter, BodyBatch, Fingerprint, ScalarFingerprinter, from_lanes,
};
use std::time::Duration;
use wgpu::util::DeviceExt;

/// Below this many bodies a batch runs on the CPU: the dispatch and readback alone took about
/// 0.6 ms in the spike (0.28 ms in the `fingerprint_gpu_dispatch` bench), and the GPU passed
/// one CPU thread only between 2,048 and 5,000 bodies (`docs/perf/sp4-performance.md`).
pub const GPU_MIN_BATCH: usize = 2048;
const WORKGROUP: usize = 64;
const SHADER: &str = include_str!("fingerprint.wgsl");
/// Longest wait for one batch; past it the batch runs on the CPU.
const POLL_TIMEOUT: Duration = Duration::from_secs(5);

/// Error scopes for every [`wgpu::ErrorFilter`]. In wgpu 30 an error outside any scope goes to
/// the default uncaptured-error handler, which panics; inside these scopes it is reported by
/// [`failed`](Self::failed) and the batch falls back to scalar. Scopes are thread-local and
/// must be popped in reverse order: `failed` and `Drop` (an early return) both do that.
struct ErrorScopes(Vec<wgpu::ErrorScopeGuard>);

impl ErrorScopes {
    fn push(device: &wgpu::Device) -> Self {
        Self(
            [
                wgpu::ErrorFilter::OutOfMemory,
                wgpu::ErrorFilter::Internal,
                wgpu::ErrorFilter::Validation,
            ]
            .into_iter()
            .map(|f| device.push_error_scope(f))
            .collect(),
        )
    }

    /// Pops every scope, innermost first; true when any of them captured an error.
    fn failed(mut self) -> bool {
        let mut failed = false;
        while let Some(scope) = self.0.pop() {
            failed |= pollster::block_on(scope.pop()).is_some();
        }
        failed
    }
}

impl Drop for ErrorScopes {
    fn drop(&mut self) {
        while let Some(scope) = self.0.pop() {
            drop(scope);
        }
    }
}

pub struct GpuFingerprinter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    adapter: String,
}

impl GpuFingerprinter {
    /// `None` without an adapter, without compute shader support, or when the device or the
    /// shader cannot be created.
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()?;
        if !adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
        {
            return None;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("tayga-fingerprint"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .ok()?;
        let scopes = ErrorScopes::push(&device);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tayga-fingerprint"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("tayga-fingerprint"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        if scopes.failed() {
            return None;
        }
        Some(Self {
            device,
            queue,
            pipeline,
            adapter: adapter.get_info().name,
        })
    }

    /// The adapter's name, e.g. `Apple M3 Max`.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// The batch on the GPU; `None` when it exceeds a device limit, the GPU reports an error, or
    /// the batch takes longer than [`POLL_TIMEOUT`].
    fn run(&self, batch: &BodyBatch, keep_http_status: bool) -> Option<Vec<Option<Fingerprint>>> {
        let n = batch.len();
        let limits = self.device.limits();
        let mut data = batch.bytes().to_vec();
        data.resize(data.len().div_ceil(4).max(1) * 4, 0);
        let lanes_size = (n * 16) as u64;
        // `lanes` (16 bytes per body) is the largest buffer but for `bytes`. Its binding check
        // bounds `n` to `max_binding / 16`, below `u32::MAX` for any `u64` limit up to 64 GiB,
        // and the workgroup-count check bounds it to `65,535 * 64` on default limits; so the
        // `n as u32` casts below are exact.
        let max_binding = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        if data.len() as u64 > max_binding
            || lanes_size > max_binding
            || n.div_ceil(WORKGROUP) > limits.max_compute_workgroups_per_dimension as usize
        {
            return None;
        }
        let scopes = ErrorScopes::push(&self.device);
        let storage = |contents: &[u8]| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents,
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        let bytes = storage(&data);
        let offsets = storage(bytemuck::cast_slice(batch.offsets()));
        let params = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&[n as u32, u32::from(keep_http_status), 0, 0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let output = |size: u64, usage: wgpu::BufferUsages| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let out_usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        let read_usage = wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ;
        let valid_size = (n * 4) as u64;
        let lanes = output(lanes_size, out_usage);
        let valid = output(valid_size, out_usage);
        let lanes_read = output(lanes_size, read_usage);
        let valid_read = output(valid_size, read_usage);
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: bytes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: offsets.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: lanes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: valid.as_entire_binding(),
                },
            ],
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(n.div_ceil(WORKGROUP) as u32, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&lanes, 0, &lanes_read, 0, lanes_size);
        encoder.copy_buffer_to_buffer(&valid, 0, &valid_read, 0, valid_size);
        let submitted = self.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        for buffer in [&lanes_read, &valid_read] {
            let tx = tx.clone();
            buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r.is_ok());
            });
        }
        drop(tx);
        // A timeout or a lost device is an `Err`: the batch runs on the CPU.
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submitted),
                timeout: Some(POLL_TIMEOUT),
            })
            .ok()?;
        // The poll ran the map callbacks; `try_iter` never blocks on one that did not run.
        let mapped = rx.try_iter().filter(|&ok| ok).count();
        if scopes.failed() || mapped != 2 {
            return None;
        }
        let lanes: Vec<u32> = bytemuck::allocation::pod_collect_to_vec(
            &lanes_read.slice(..).get_mapped_range().ok()?,
        );
        let valid: Vec<u32> = bytemuck::allocation::pod_collect_to_vec(
            &valid_read.slice(..).get_mapped_range().ok()?,
        );
        Some(
            (0..n)
                .map(|i| {
                    (valid[i] == 1).then(|| {
                        from_lanes([
                            lanes[4 * i],
                            lanes[4 * i + 1],
                            lanes[4 * i + 2],
                            lanes[4 * i + 3],
                        ])
                    })
                })
                .collect(),
        )
    }
}

impl BatchFingerprinter for GpuFingerprinter {
    fn name(&self) -> &'static str {
        "gpu"
    }

    fn fingerprint(
        &self,
        batch: &BodyBatch,
        keep_http_status: bool,
        out: &mut Vec<Option<Fingerprint>>,
    ) {
        if batch.len() >= GPU_MIN_BATCH
            && let Some(v) = self.run(batch, keep_http_status)
        {
            *out = v;
            return;
        }
        ScalarFingerprinter.fingerprint(batch, keep_http_status, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fingerprint::fingerprint_body;

    /// Dropping the scopes (an early return in `run`) pops them in reverse order: wgpu panics
    /// on any other order. A captured validation error is reported, not raised.
    #[test]
    fn error_scopes_capture_errors_and_drop_in_order() {
        let Some(g) = GpuFingerprinter::new() else {
            assert!(
                std::env::var("TAYGA_REQUIRE_GPU").as_deref() != Ok("1"),
                "TAYGA_REQUIRE_GPU=1 but no usable GPU adapter"
            );
            eprintln!("no GPU adapter: skipped");
            return;
        };
        drop(ErrorScopes::push(&g.device));
        assert!(!ErrorScopes::push(&g.device).failed());
        let scopes = ErrorScopes::push(&g.device);
        // `MAP_READ | MAP_WRITE` without `MAPPABLE_PRIMARY_BUFFERS` is a validation error.
        let _invalid = g.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 3,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::MAP_WRITE,
            mapped_at_creation: false,
        });
        assert!(scopes.failed());
    }

    /// The kernel itself, below `GPU_MIN_BATCH`: workgroup edges and every rejection rule.
    #[test]
    fn the_kernel_equals_fingerprint_body_on_small_batches() {
        let Some(g) = GpuFingerprinter::new() else {
            assert!(
                std::env::var("TAYGA_REQUIRE_GPU").as_deref() != Ok("1"),
                "TAYGA_REQUIRE_GPU=1 but no usable GPU adapter"
            );
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let edge = [
            "",
            " \t\n ",
            "a\0b",
            "\0",
            "x \0",
            "zaż\u{f3}łć 1",
            "\u{2026}",
            "Found 10 products from database",
            "deadbeef DEADBEEFCAFE deadbee _deadbeef",
            "a<*>b <* <empty>",
            "a\x0Bb c",
            r#""POST /x HTTP/2" 200 - 5"#,
            "upstream replied HTTP/1.1 503",
            "HTTP/1.10 503 HTTP/1.1\" 404",
        ];
        let long = "word ".repeat(100);
        for n in [1, 2, 63, 64, 65, 129] {
            let mut batch = BodyBatch::new();
            for body in edge.iter().copied().chain([long.as_str()]).cycle().take(n) {
                assert!(batch.push(body));
            }
            for keep in [true, false] {
                let want: Vec<_> = (0..n)
                    .map(|i| fingerprint_body(batch.body(i), keep))
                    .collect();
                assert_eq!(g.run(&batch, keep), Some(want), "n={n} keep={keep}");
            }
        }
    }
}
