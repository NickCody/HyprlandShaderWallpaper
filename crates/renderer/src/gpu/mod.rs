//! GPU orchestration rebuilt for the lightweight renderer.
//!
//! The new architecture keeps the public surface (`GpuState`) stable while
//! dramatically simplifying the path from uniforms to pixels:
//! - `context` owns wgpu instance/device/surface wiring and knows how to
//!   rebuild swapchain state when the window resizes.
//! - `channels` materialises ShaderToy channel resources (textures, cubemaps,
//!   keyboard) and exposes their resolutions for uniforms.
//! - `pipeline` compiles wrapped GLSL into render pipelines with a single
//!   bind group layout.
//! - `uniforms` mirrors the injected ShaderToy macros and writes changes
//!   straight through the queue each frame.
//! - `timeline` tracks pending shader swaps, warmup frames, and crossfade
//!   envelopes using wall-clock `Instant`s plus user-selectable easing shapes.
//! - `state` glues everything together and exposes the `GpuState` API used by
//!   `window` and `wallpaper`.
//!
//! Phases still to come (tracked for follow-up work):
//! - Reintroduce still-frame exports.
//! - Restore fill-method experimentation and GPU power/latency knobs.
//! - Async pipeline warmup once animation smoothness is nailed down.

mod channels;
mod context;
mod pipeline;
mod state;
mod timeline;
mod uniforms;

pub(crate) use state::{FileExportTarget, GpuState, RenderExportError};
