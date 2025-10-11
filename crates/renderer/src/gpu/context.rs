use anyhow::{anyhow, Context as AnyhowContext, Result};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use wgpu::TextureFormatFeatureFlags;
use winit::dpi::PhysicalSize;

use crate::types::{
    AdapterProfile, Antialiasing, ColorSpaceMode, GpuMemoryMode, GpuPowerPreference, VsyncMode,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SurfaceColorSpace {
    Gamma,
    Linear,
}

pub(crate) struct GpuContext {
    pub _instance: wgpu::Instance,
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub size: PhysicalSize<u32>,
    pub sample_count: u32,
    pub surface_format: wgpu::TextureFormat,
    pub color_space: SurfaceColorSpace,
    pub adapter_profile: AdapterProfile,
    pub _surface_supports_copy: bool,
    #[allow(dead_code)]
    vsync_mode: VsyncMode,
    surface_caps: wgpu::SurfaceCapabilities,
}

impl GpuContext {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new<T>(
        target: &T,
        initial_size: PhysicalSize<u32>,
        antialiasing: Antialiasing,
        color_space: ColorSpaceMode,
        gpu_power: GpuPowerPreference,
        gpu_memory: GpuMemoryMode,
        gpu_latency: u32,
        vsync_mode: VsyncMode,
    ) -> Result<Self>
    where
        T: HasDisplayHandle + HasWindowHandle,
    {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
        });

        let window_handle = target
            .window_handle()
            .map_err(|err| anyhow!("failed to acquire window handle: {err}"))?;
        let display_handle = target
            .display_handle()
            .map_err(|err| anyhow!("failed to acquire display handle: {err}"))?;

        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: display_handle.as_raw(),
                raw_window_handle: window_handle.as_raw(),
            })
        }
        .context("failed to create rendering surface")?;

        let power_preference = match gpu_power {
            GpuPowerPreference::Low => wgpu::PowerPreference::LowPower,
            GpuPowerPreference::High => wgpu::PowerPreference::HighPerformance,
        };
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .context("failed to find a suitable GPU adapter")?;

        let adapter_info = adapter.get_info();
        let limits = adapter.limits();
        let adapter_profile = AdapterProfile::from_wgpu(&adapter_info, &limits);
        let is_software = adapter_profile.is_software();
        tracing::debug!(
            name = %adapter_profile.name,
            backend = ?adapter_profile.backend,
            device_type = ?adapter_profile.device_type,
            is_software,
            "selected GPU adapter"
        );

        let max_dimension = limits.max_texture_dimension_2d;
        let requested_width = initial_size.width.max(1);
        let requested_height = initial_size.height.max(1);
        if requested_width > max_dimension || requested_height > max_dimension {
            anyhow::bail!(
                "GPU max texture dimension is {max_dimension}, requested surface is {width}x{height}",
                max_dimension = max_dimension,
                width = requested_width,
                height = requested_height
            );
        }

        let surface_caps = surface.get_capabilities(&adapter);
        let color_space = match color_space {
            ColorSpaceMode::Auto | ColorSpaceMode::Gamma => SurfaceColorSpace::Gamma,
            ColorSpaceMode::Linear => SurfaceColorSpace::Linear,
        };

        let surface_format = match color_space {
            SurfaceColorSpace::Linear => surface_caps
                .formats
                .iter()
                .copied()
                .find(|format| format.is_srgb())
                .unwrap_or_else(|| {
                    let fallback = surface_caps.formats[0];
                    if !fallback.is_srgb() {
                        tracing::warn!(
                            ?fallback,
                            "no sRGB surface format available; falling back to {:?}",
                            fallback
                        );
                    }
                    fallback
                }),
            SurfaceColorSpace::Gamma => surface_caps
                .formats
                .iter()
                .copied()
                .find(|format| !format.is_srgb())
                .unwrap_or_else(|| {
                    let fallback = surface_caps.formats[0];
                    if fallback.is_srgb() {
                        tracing::warn!(
                            ?fallback,
                            "no linear (non-sRGB) surface format available; falling back to {:?}",
                            fallback
                        );
                    }
                    fallback
                }),
        };

        let format_features = adapter.get_texture_format_features(surface_format);
        let mut supported_samples = format_features.flags.supported_sample_counts();
        if !supported_samples.contains(&1) {
            supported_samples.push(1);
        }
        supported_samples.sort_unstable();
        supported_samples.dedup();

        let mut sample_count = match antialiasing {
            Antialiasing::Auto => *supported_samples.last().unwrap_or(&1),
            Antialiasing::Off => 1,
            Antialiasing::Samples(requested) => {
                if supported_samples.contains(&requested) {
                    requested
                } else {
                    let fallback = supported_samples
                        .iter()
                        .copied()
                        .filter(|&count| count <= requested)
                        .max()
                        .unwrap_or(*supported_samples.first().unwrap_or(&1));
                    tracing::warn!(
                        requested,
                        fallback,
                        ?supported_samples,
                        "requested MSAA sample count not supported; falling back"
                    );
                    fallback
                }
            }
        };

        if sample_count > 1
            && !format_features
                .flags
                .contains(TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
        {
            tracing::warn!(
                ?surface_format,
                "surface format does not support MSAA resolve; disabling MSAA"
            );
            sample_count = 1;
        }

        if is_software && sample_count > 1 {
            tracing::warn!(
                sample_count,
                "software rasterizer detected; disabling MSAA for performance"
            );
            sample_count = 1;
        }

        let mut required_features = wgpu::Features::empty();
        if sample_count > 4 {
            required_features |= wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        }

        let memory_hints = match gpu_memory {
            GpuMemoryMode::Balanced => wgpu::MemoryHints::MemoryUsage,
            GpuMemoryMode::Performance => wgpu::MemoryHints::Performance,
        };

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("wax11 device"),
            required_features,
            required_limits: limits.clone(),
            memory_hints,
            trace: wgpu::Trace::default(),
        }))
        .context("failed to create GPU device")?;

        let desired_maximum_frame_latency = gpu_latency.clamp(1, 3);
        if desired_maximum_frame_latency != gpu_latency {
            tracing::warn!(
                requested = gpu_latency,
                clamped = desired_maximum_frame_latency,
                "GPU frame latency clamped to valid range (1-3)"
            );
        }

        let size = PhysicalSize::new(requested_width, requested_height);
        let surface_supports_copy = surface_caps.usages.contains(wgpu::TextureUsages::COPY_SRC);
        let mut surface_usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        if surface_supports_copy {
            surface_usage |= wgpu::TextureUsages::COPY_SRC;
        } else {
            tracing::warn!(
                "surface does not advertise COPY_SRC; still-export will fall back to presenting only"
            );
        }

        let present_mode = surface_caps
            .present_modes
            .iter()
            .copied()
            .find(|mode| *mode == wgpu::PresentMode::Fifo)
            .unwrap_or_else(|| surface_caps.present_modes[0]);

        // Apply initial vsync setting based on mode
        let present_mode = match vsync_mode {
            VsyncMode::Never => present_mode, // Keep Fifo (vsync on)
            VsyncMode::Always => {
                // Prefer Immediate (no vsync), fallback to Mailbox, then Fifo
                surface_caps
                    .present_modes
                    .iter()
                    .copied()
                    .find(|mode| *mode == wgpu::PresentMode::Immediate)
                    .or_else(|| {
                        surface_caps
                            .present_modes
                            .iter()
                            .copied()
                            .find(|mode| *mode == wgpu::PresentMode::Mailbox)
                    })
                    .unwrap_or(present_mode)
            }
            VsyncMode::Crossfade => present_mode, // Start with vsync, will toggle dynamically
        };

        tracing::debug!(?present_mode, ?vsync_mode, "using present mode");

        let config = wgpu::SurfaceConfiguration {
            usage: surface_usage,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency,
        };
        surface.configure(&device, &config);

        Ok(Self {
            _instance: instance,
            surface,
            device,
            queue,
            config,
            size: initial_size,
            sample_count,
            surface_format,
            color_space,
            adapter_profile,
            _surface_supports_copy: surface_supports_copy,
            vsync_mode,
            surface_caps,
        })
    }

    pub(crate) fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }

        self.size = new_size;
        self.config.width = new_size.width.max(1);
        self.config.height = new_size.height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    /// Enable or disable VSync by reconfiguring the surface present mode.
    /// When `enabled` is false, prefers Immediate mode (no vsync) for lowest latency.
    pub(crate) fn set_vsync(&mut self, enabled: bool) {
        let target_mode = if enabled {
            // Prefer Fifo (vsync) for tear-free presentation
            self.surface_caps
                .present_modes
                .iter()
                .copied()
                .find(|mode| *mode == wgpu::PresentMode::Fifo)
                .unwrap_or(self.config.present_mode)
        } else {
            // Prefer Immediate (no vsync), fallback to Mailbox, then current
            self.surface_caps
                .present_modes
                .iter()
                .copied()
                .find(|mode| *mode == wgpu::PresentMode::Immediate)
                .or_else(|| {
                    self.surface_caps
                        .present_modes
                        .iter()
                        .copied()
                        .find(|mode| *mode == wgpu::PresentMode::Mailbox)
                })
                .unwrap_or(self.config.present_mode)
        };

        if target_mode != self.config.present_mode {
            self.config.present_mode = target_mode;
            self.surface.configure(&self.device, &self.config);
            tracing::debug!(
                ?target_mode,
                vsync_enabled = enabled,
                "reconfigured surface present mode"
            );
        }
    }
}
