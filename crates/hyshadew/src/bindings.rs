use renderer::{ChannelBindings, SurfaceAlpha as RendererSurfaceAlpha};
use shadertoy::{InputSource, LocalPack, SurfaceAlpha as ManifestSurfaceAlpha};

pub fn channel_bindings_from_pack(pack: &LocalPack) -> ChannelBindings {
    let mut bindings = ChannelBindings::default();
    let manifest = pack.manifest();
    let entry_name = &manifest.entry;
    let entry_pass = manifest.passes.iter().find(|pass| &pass.name == entry_name);

    let Some(pass) = entry_pass else {
        tracing::warn!(entry = %entry_name, "entry pass missing; no channels bound");
        return bindings;
    };

    for input in &pass.inputs {
        match &input.source {
            InputSource::Texture { path } => {
                let resolved = if path.is_absolute() {
                    path.clone()
                } else {
                    pack.root().join(path)
                };
                let resolved_for_log = resolved.clone();
                if !resolved_for_log.exists() {
                    tracing::warn!(
                        channel = input.channel,
                        path = %resolved_for_log.display(),
                        "channel texture not found on disk"
                    );
                }
                if let Err(err) = bindings.set_texture(input.channel as usize, resolved) {
                    tracing::warn!(
                        channel = input.channel,
                        path = %resolved_for_log.display(),
                        error = %err,
                        "failed to register texture channel"
                    );
                }
            }
            InputSource::Buffer { name } => {
                tracing::warn!(
                    channel = input.channel,
                    buffer = %name,
                    "buffer channels are not supported yet"
                );
            }
            InputSource::Cubemap { directory } => {
                tracing::warn!(
                    channel = input.channel,
                    dir = %directory.display(),
                    "cubemap channels are not supported yet"
                );
            }
            InputSource::Audio { path } => {
                tracing::warn!(
                    channel = input.channel,
                    path = %path.display(),
                    "audio channels are not supported yet"
                );
            }
        }
    }

    bindings
}

pub fn map_manifest_alpha(alpha: ManifestSurfaceAlpha) -> RendererSurfaceAlpha {
    match alpha {
        ManifestSurfaceAlpha::Opaque => RendererSurfaceAlpha::Opaque,
        ManifestSurfaceAlpha::Transparent => RendererSurfaceAlpha::Transparent,
    }
}
