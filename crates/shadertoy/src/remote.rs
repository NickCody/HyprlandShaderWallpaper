use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::manifest::{
    InputSource, PassInput, PassKind, ShaderPackManifest, ShaderPass, SurfaceAlpha,
};
use crate::pack::LocalPack;

#[derive(Debug, Clone)]
pub struct ShadertoyConfig {
    pub api_key: String,
    pub api_base: Url,
    pub media_base: Url,
}

impl ShadertoyConfig {
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            bail!("Shadertoy API key must not be empty");
        }
        Ok(Self {
            api_key,
            api_base: Url::parse("https://www.shadertoy.com/api/v1/")?,
            media_base: Url::parse("https://www.shadertoy.com/")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ShadertoyClient {
    http: Client,
    config: ShadertoyConfig,
}

impl ShadertoyClient {
    pub fn new(config: ShadertoyConfig) -> Result<Self> {
        let http = Client::builder().build()?;
        Ok(Self { http, config })
    }

    pub fn fetch_shader(&self, shader_id: &str) -> Result<ShaderPayload> {
        if shader_id.trim().is_empty() {
            bail!("shader id must not be empty");
        }
        let mut url = self.config.api_base.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow!("invalid Shadertoy API base url"))?;
            segments.push("shaders");
            segments.push(shader_id);
        }
        url.set_query(Some(&format!("key={}", self.config.api_key)));
        let response = self
            .http
            .get(url.clone())
            .send()
            .with_context(|| format!("requesting {url}"))?
            .error_for_status()
            .context("Shadertoy API returned an error status")?;
        let body = response.text()?;
        // Try to decode the happy path first.
        if let Ok(payload) = serde_json::from_str::<ShaderApiResponse>(&body) {
            return Ok(payload.shader);
        }
        // Try to decode common error shapes from the API.
        if let Ok(err) = serde_json::from_str::<ApiError>(&body) {
            bail!(
                "Shadertoy API error: {} (while requesting {url})",
                err.error
            );
        }
        let snippet = body.chars().take(200).collect::<String>();
        bail!(
            "unexpected Shadertoy API response; could not find 'Shader' field. First 200 bytes: {}",
            snippet
        )
    }

    pub fn fetch_and_cache(&self, shader_id: &str, cache_dir: &Path) -> Result<LocalPack> {
        let payload = self.fetch_shader(shader_id)?;
        materialize_shader(&payload, cache_dir, |url, destination| {
            self.download_asset(url, destination)
        })
    }

    pub fn download_asset(&self, src: &str, destination: &Path) -> Result<()> {
        let url = self
            .resolve_media_url(src)
            .with_context(|| format!("resolving asset url '{src}'"))?;
        debug!(%url, path = %destination.display(), "downloading shadertoy asset");
        let response = self
            .http
            .get(url.clone())
            .send()
            .with_context(|| format!("requesting asset {url}"))?
            .error_for_status()
            .context("Shadertoy asset request failed")?;
        let bytes = response.bytes()?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(destination, &bytes)?;
        Ok(())
    }

    fn resolve_media_url(&self, src: &str) -> Result<Url> {
        if src.starts_with("http://") || src.starts_with("https://") {
            return Ok(Url::parse(src)?);
        }
        if src.starts_with("//") {
            return Ok(Url::parse(&format!("https:{}", src))?);
        }
        let trimmed = src.trim_start_matches('/');
        self.config
            .media_base
            .join(trimmed)
            .context("joining media url")
    }
}

#[derive(Debug, Deserialize)]
struct ShaderApiResponse {
    #[serde(rename = "Shader")]
    shader: ShaderPayload,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    #[serde(alias = "Error")]
    error: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ShaderPayload {
    pub info: ShaderInfo,
    #[serde(rename = "renderpass")]
    pub render_passes: Vec<RenderPass>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ShaderInfo {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RenderPass {
    pub name: String,
    #[serde(rename = "type")]
    pub pass_type: String,
    pub code: String,
    #[serde(default)]
    pub inputs: Vec<RenderInput>,
    #[serde(default)]
    pub outputs: Vec<RenderOutput>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RenderInput {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub channel: u8,
    #[serde(default)]
    pub src: Option<String>,
    #[serde(rename = "ctype")]
    pub channel_type: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RenderOutput {
    pub id: i64,
    #[serde(default)]
    pub channel: u8,
}

#[derive(Debug)]
struct PassArtifact {
    source_rel: PathBuf,
    source_code: String,
}

#[derive(Debug)]
struct AssetArtifact {
    url: String,
    destination_rel: PathBuf,
}

#[derive(Debug)]
struct CachePlan {
    manifest: ShaderPackManifest,
    pass_artifacts: Vec<PassArtifact>,
    assets: Vec<AssetArtifact>,
}

pub fn materialize_shader<F>(
    payload: &ShaderPayload,
    cache_dir: &Path,
    mut fetch_asset: F,
) -> Result<LocalPack>
where
    F: FnMut(&str, &Path) -> Result<()>,
{
    fs::create_dir_all(cache_dir)?;
    let plan = build_cache_plan(payload)?;

    for pass in &plan.pass_artifacts {
        let full_path = cache_dir.join(&pass.source_rel);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&full_path, pass.source_code.as_bytes())?;
    }

    for asset in &plan.assets {
        let dest = cache_dir.join(&asset.destination_rel);
        fetch_asset(&asset.url, &dest)?;
    }

    let manifest_path = cache_dir.join("shader.toml");
    let manifest_toml = toml::to_string_pretty(&plan.manifest)?;
    fs::write(&manifest_path, manifest_toml)?;

    LocalPack::load(cache_dir).map_err(|err| anyhow!(err))
}

fn build_cache_plan(payload: &ShaderPayload) -> Result<CachePlan> {
    if payload.render_passes.is_empty() {
        bail!("shader payload contains no render passes");
    }

    let mut used_names = HashSet::new();
    let mut pass_names = Vec::new();
    let mut output_to_pass: HashMap<i64, String> = HashMap::new();
    let mut name_lookup: HashMap<String, String> = HashMap::new();

    for (index, pass) in payload.render_passes.iter().enumerate() {
        let base_name = sanitize_label(&pass.name).unwrap_or_else(|| format!("pass{index}"));
        let unique_name = make_unique_name(base_name, &mut used_names);
        name_lookup.insert(pass.name.to_lowercase(), unique_name.clone());
        for output in &pass.outputs {
            output_to_pass.insert(output.id, unique_name.clone());
        }
        pass_names.push(unique_name);
    }

    let entry = payload
        .render_passes
        .iter()
        .zip(pass_names.iter())
        .find(|(pass, _)| pass.pass_type.eq_ignore_ascii_case("image"))
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| pass_names[0].clone());

    let mut pass_artifacts = Vec::new();
    let mut assets = Vec::new();
    let mut manifest_passes = Vec::new();

    // Collect any 'Common' pass code so it can be injected into other passes.
    let mut common_code = String::new();
    for pass in &payload.render_passes {
        if pass.pass_type.eq_ignore_ascii_case("common") {
            common_code.push_str(&pass.code);
            if !common_code.ends_with('\n') {
                common_code.push('\n');
            }
        }
    }

    for (pass, pass_name) in payload.render_passes.iter().zip(pass_names.iter()) {
        if pass.pass_type.eq_ignore_ascii_case("common") {
            // Don't emit a manifest pass for 'Common'; its code is injected into other passes.
            continue;
        }
        let source_rel = PathBuf::from(format!("{pass_name}.glsl"));
        let mut inputs = Vec::new();
        for input in &pass.inputs {
            let channel = input.channel;
            if channel > 3 {
                bail!(
                    "pass '{}' references channel {} which exceeds supported range",
                    pass.name,
                    channel
                );
            }
            let ctype = input.channel_type.to_ascii_lowercase();
            match ctype.as_str() {
                "buffer" => {
                    let buffer_name = input
                        .id
                        .and_then(|id| output_to_pass.get(&id).cloned())
                        .or_else(|| {
                            input
                                .src
                                .as_ref()
                                .and_then(|s| name_lookup.get(&s.to_lowercase()).cloned())
                        })
                        .ok_or_else(|| {
                            anyhow!(
                                "pass '{}' references buffer input with unknown target",
                                pass.name
                            )
                        })?;
                    inputs.push(PassInput {
                        channel,
                        source: InputSource::Buffer { name: buffer_name },
                    });
                }
                "texture" => {
                    let src = input.src.as_ref().ok_or_else(|| {
                        anyhow!("texture input missing src in pass '{}'", pass.name)
                    })?;
                    let filename = derive_filename(src, &format!("{pass_name}_ch{channel}"), "png");
                    let dest_rel = PathBuf::from("textures").join(&filename);
                    if !assets
                        .iter()
                        .any(|asset: &AssetArtifact| asset.destination_rel == dest_rel)
                    {
                        assets.push(AssetArtifact {
                            url: src.clone(),
                            destination_rel: dest_rel.clone(),
                        });
                    }
                    inputs.push(PassInput {
                        channel,
                        source: InputSource::Texture { path: dest_rel },
                    });
                }
                "cubemap" => {
                    let src = input.src.as_ref().ok_or_else(|| {
                        anyhow!("cubemap input missing src in pass '{}'", pass.name)
                    })?;
                    let filename = derive_filename(src, &format!("{pass_name}_cube"), "png");
                    let base_dir_name = Path::new(&filename)
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().to_string())
                        .and_then(|stem| sanitize_label(&stem))
                        .unwrap_or_else(|| "cubemap".to_string());
                    let dest_dir = PathBuf::from("cubemaps").join(&base_dir_name);
                    let dest_rel = dest_dir.join(&filename);
                    if !assets
                        .iter()
                        .any(|asset: &AssetArtifact| asset.destination_rel == dest_rel)
                    {
                        assets.push(AssetArtifact {
                            url: src.clone(),
                            destination_rel: dest_rel.clone(),
                        });
                    }
                    inputs.push(PassInput {
                        channel,
                        source: InputSource::Cubemap {
                            directory: dest_dir,
                        },
                    });
                }
                "music" | "sound" | "musicstream" => {
                    let src = input.src.as_ref().ok_or_else(|| {
                        anyhow!("audio input missing src in pass '{}'", pass.name)
                    })?;
                    let filename = derive_filename(src, &format!("{pass_name}_ch{channel}"), "mp3");
                    let dest_rel = PathBuf::from("audio").join(&filename);
                    if !assets
                        .iter()
                        .any(|asset: &AssetArtifact| asset.destination_rel == dest_rel)
                    {
                        assets.push(AssetArtifact {
                            url: src.clone(),
                            destination_rel: dest_rel.clone(),
                        });
                    }
                    inputs.push(PassInput {
                        channel,
                        source: InputSource::Audio { path: dest_rel },
                    });
                }
                other => {
                    // Gracefully ignore unsupported channel types (e.g. 'keyboard', 'webcam').
                    // The renderer will bind a placeholder texture/sampler, which is sufficient
                    // for most shaders to keep running.
                    tracing::warn!(
                        pass = %pass.name,
                        channel,
                        channel_type = %other,
                        "ignoring unsupported channel type"
                    );
                    // Intentionally do not push a PassInput; the channel remains unbound in the manifest.
                }
            }
        }

        let kind = map_pass_kind(&pass.pass_type)?;
        manifest_passes.push(ShaderPass {
            name: pass_name.clone(),
            kind,
            source: source_rel.clone(),
            inputs,
        });
        // Prepend Common code to the pass source (if any was present)
        let source_code = if common_code.is_empty() {
            pass.code.clone()
        } else {
            let mut s = String::with_capacity(common_code.len() + pass.code.len() + 1);
            s.push_str(&common_code);
            s.push_str(&pass.code);
            s
        };
        pass_artifacts.push(PassArtifact {
            source_rel,
            source_code,
        });
    }

    let manifest = ShaderPackManifest {
        name: (!payload.info.name.trim().is_empty()).then(|| payload.info.name.clone()),
        entry,
        surface_alpha: SurfaceAlpha::Opaque,
        description: payload.info.description.clone(),
        tags: payload.info.tags.clone(),
        passes: manifest_passes,
    };

    let issues = manifest.validate();
    if !issues.is_empty() {
        bail!("generated manifest invalid: {:?}", issues);
    }

    Ok(CachePlan {
        manifest,
        pass_artifacts,
        assets,
    })
}

fn map_pass_kind(kind: &str) -> Result<PassKind> {
    match kind.to_ascii_lowercase().as_str() {
        "image" => Ok(PassKind::Image),
        "buffer" | "compute" => Ok(PassKind::Buffer),
        "sound" | "music" => Ok(PassKind::Sound),
        "cubemap" => Ok(PassKind::Cubemap),
        other => bail!("unsupported render pass type '{}'", other),
    }
}

fn sanitize_label(input: &str) -> Option<String> {
    let mut result = String::new();
    let mut prev_underscore = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if (ch.is_ascii_whitespace() || ch == '-' || ch == '_')
            && !result.is_empty()
            && !prev_underscore
        {
            result.push('_');
            prev_underscore = true;
        }
    }
    while result.ends_with('_') {
        result.pop();
    }
    if result.is_empty() {
        return None;
    }
    if result.starts_with(|c: char| c.is_ascii_digit()) {
        result.insert(0, 'p');
    }
    Some(result)
}

fn make_unique_name(base: String, used: &mut HashSet<String>) -> String {
    if used.insert(base.clone()) {
        return base;
    }
    let base_copy = base;
    let mut counter = 2;
    loop {
        let candidate = format!("{base_copy}_{counter}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        counter += 1;
    }
}

fn derive_filename(src: &str, fallback_base: &str, default_ext: &str) -> String {
    let trimmed = src.split(&['?', '#'][..]).next().unwrap_or(src);
    let path_owned = if let Ok(url) = Url::parse(trimmed) {
        url.path().to_string()
    } else {
        trimmed.to_string()
    };
    if let Some(name) = Path::new(&path_owned).file_name() {
        let name = name.to_string_lossy().to_string();
        if !name.is_empty() {
            return name;
        }
    }
    let mut base = sanitize_label(fallback_base).unwrap_or_else(|| "asset".to_string());
    if base.is_empty() {
        base = "asset".to_string();
    }
    let ext = default_ext.trim_start_matches('.');
    if ext.is_empty() {
        base
    } else {
        format!("{base}.{ext}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::InputSource;

    fn sample_payload() -> ShaderPayload {
        let json = r#"
        {
            "Shader": {
                "info": {
                    "name": "Sample Shader",
                    "description": "Demo",
                    "tags": ["demo", "test"]
                },
                "renderpass": [
                    {
                        "name": "Buffer A",
                        "type": "buffer",
                        "code": "void mainImage(out vec4 fragColor, in vec2 fragCoord) { fragColor = vec4(0.0); }",
                        "inputs": [
                            {
                                "channel": 0,
                                "ctype": "texture",
                                "src": "/media/foo/tex0.png"
                            }
                        ],
                        "outputs": [{ "id": 12, "channel": 0 }]
                    },
                    {
                        "name": "Image",
                        "type": "image",
                        "code": "void mainImage(out vec4 fragColor, in vec2 fragCoord) { fragColor = vec4(1.0); }",
                        "inputs": [
                            { "channel": 0, "ctype": "buffer", "id": 12 },
                            { "channel": 1, "ctype": "texture", "src": "https://www.shadertoy.com/media/prev.png" }
                        ],
                        "outputs": [{ "id": 15, "channel": 0 }]
                    }
                ]
            }
        }
        "#;
        serde_json::from_str::<ShaderApiResponse>(json)
            .expect("valid sample payload")
            .shader
    }

    #[test]
    fn materializes_payload_into_cache() {
        let payload = sample_payload();
        let temp = tempfile::tempdir().unwrap();
        let mut fetched_assets = Vec::new();
        let pack = materialize_shader(&payload, temp.path(), |url, destination| {
            fetched_assets.push((url.to_string(), destination.to_path_buf()));
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(destination, b"stub").unwrap();
            Ok(())
        })
        .expect("materialize shader");

        assert_eq!(pack.manifest().entry, "image");
        assert_eq!(pack.manifest().passes.len(), 2);

        let image_pass = pack
            .manifest()
            .passes
            .iter()
            .find(|p| p.name == "image")
            .expect("image pass");
        assert_eq!(image_pass.inputs.len(), 2);
        assert!(matches!(
            image_pass.inputs[0].source,
            InputSource::Buffer { ref name } if name == "buffer_a"
        ));
        assert!(matches!(
            image_pass.inputs[1].source,
            InputSource::Texture { ref path } if path == Path::new("textures/prev.png")
        ));

        assert!(pack.pass_source_path("buffer_a").unwrap().exists());
        assert!(pack.pass_source_path("image").unwrap().exists());

        assert!(fetched_assets
            .iter()
            .any(|(url, dest)| url.contains("tex0.png") && dest.ends_with("textures/tex0.png")));
    }

    #[test]
    fn sanitize_label_handles_edge_cases() {
        assert_eq!(sanitize_label("Buffer A"), Some("buffer_a".to_string()));
        assert_eq!(sanitize_label("  "), None);
        assert_eq!(sanitize_label("123name"), Some("p123name".to_string()));
    }
}
