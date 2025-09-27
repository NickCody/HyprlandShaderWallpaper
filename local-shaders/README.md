# Local Shader Packs

Place user-provided shader packs in subdirectories of this folder. Each pack mirrors
ShaderToy's render pass structure and must include a `shader.toml` manifest that
declares the passes and channel bindings.

```
local-shaders/
  cloudy-evening/
    shader.toml
    image.glsl
    bufferA.glsl
    textures/
      noise.png
    cubemaps/
      skybox/
        posx.png
        negx.png
        posy.png
        negy.png
        posz.png
        negz.png
    audio/
      track0.ogg
```

## Manifest Format

```toml
name = "Cloudy Evening"
entry = "image"

tags = ["atmosphere", "demo"]

[[passes]]
name = "image"
kind = "image"
source = "image.glsl"

[[passes.inputs]]
channel = 0
type = "texture"
path = "textures/noise.png"

[[passes]]
name = "bufferA"
kind = "buffer"
source = "bufferA.glsl"

[[passes.inputs]]
channel = 0
type = "buffer"
name = "bufferA"
```

The runtime validates manifests on load. Channels must be in the `0..=3` range and
referenced buffer passes must be declared in the same manifest. Asset paths are
resolved relative to the pack root.
