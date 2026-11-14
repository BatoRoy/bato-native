struct VertexInput {
    @builtin(vertex_index) vertex_idx: u32,
    @location(0) pos: vec2<i32>,
    @location(1) dim: u32,
    @location(2) uv: u32,
    @location(3) color: u32,
    @location(4) content_type_with_srgb: u32,
    @location(5) depth: f32,
}

struct VertexOutput {
    @invariant @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) content_type: u32,
};

struct Params {
    screen_resolution: vec2<u32>,
    _pad: vec2<u32>,
};

@group(0) @binding(0)
var color_atlas_texture: texture_2d<f32>;

@group(0) @binding(1)
var mask_atlas_texture: texture_2d<f32>;

@group(0) @binding(2)
var atlas_sampler: sampler;

@group(1) @binding(0)
var<uniform> params: Params;

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    } else {
        return pow((c + 0.055) / 1.055, 2.4);
    }
}

// bato patch: Skia's text "preblend" (SkMaskGamma) as Chromium on Linux
// configures it: SK_GAMMA_EXPONENT 1.2, SK_GAMMA_CONTRAST 0.2, text luminance
// quantized to 3 bits (SkTMaskGamma<3,3,3>). It corrects glyph coverage
// depending on the text colour so the result matches Chromium's grayscale AA
// (e.g. light text on dark gets coverage^(1/1.2)).
fn sk_luminance(c: vec3<f32>) -> f32 {
    let l = dot(pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.2)), vec3<f32>(0.2126, 0.7152, 0.0722));
    let byte = round(pow(l, 1.0 / 1.2) * 255.0);
    let i = floor(byte / 32.0);
    // sk_t_scale255<3>: (i << 5) | (i << 2) | (i >> 1)
    return (i * 32.0 + i * 4.0 + floor(i / 2.0)) / 255.0;
}

fn sk_preblend(a: f32, src: f32) -> f32 {
    let contrast = 0.2;
    let gamma = 1.2;
    let dst = 1.0 - src;
    let lin_dst = pow(dst, gamma);
    let adj = contrast * lin_dst;
    let srca = a + (1.0 - a) * adj * a;
    if (abs(src - dst) < 1.0 / 256.0) {
        return srca;
    }
    let lin_src = pow(src, gamma);
    let lin_out = lin_src * srca + (1.0 - srca) * lin_dst;
    let o = pow(lin_out, 1.0 / gamma);
    return clamp((o - dst) / (src - dst), 0.0, 1.0);
}

@vertex
fn vs_main(in_vert: VertexInput) -> VertexOutput {
    var pos = in_vert.pos;
    let width = in_vert.dim & 0xffffu;
    let height = (in_vert.dim & 0xffff0000u) >> 16u;
    let color = in_vert.color;
    var uv = vec2<u32>(in_vert.uv & 0xffffu, (in_vert.uv & 0xffff0000u) >> 16u);
    let v = in_vert.vertex_idx;

    let corner_position = vec2<u32>(
        in_vert.vertex_idx & 1u,
        (in_vert.vertex_idx >> 1u) & 1u,
    );

    let corner_offset = vec2<u32>(width, height) * corner_position;

    uv = uv + corner_offset;
    pos = pos + vec2<i32>(corner_offset);

    var vert_output: VertexOutput;

    vert_output.position = vec4<f32>(
        2.0 * vec2<f32>(pos) / vec2<f32>(params.screen_resolution) - 1.0,
        in_vert.depth,
        1.0,
    );

    vert_output.position.y *= -1.0;

    let content_type = in_vert.content_type_with_srgb & 0xffffu;
    let srgb = (in_vert.content_type_with_srgb & 0xffff0000u) >> 16u;

    switch srgb {
        case 0u: {
            vert_output.color = vec4<f32>(
                f32((color & 0x00ff0000u) >> 16u) / 255.0,
                f32((color & 0x0000ff00u) >> 8u) / 255.0,
                f32(color & 0x000000ffu) / 255.0,
                f32((color & 0xff000000u) >> 24u) / 255.0,
            );
        }
        case 1u: {
            vert_output.color = vec4<f32>(
                srgb_to_linear(f32((color & 0x00ff0000u) >> 16u) / 255.0),
                srgb_to_linear(f32((color & 0x0000ff00u) >> 8u) / 255.0),
                srgb_to_linear(f32(color & 0x000000ffu) / 255.0),
                f32((color & 0xff000000u) >> 24u) / 255.0,
            );
        }
        default: {}
    }

    var dim: vec2<u32> = vec2(0u);
    switch content_type {
        case 0u: {
            dim = textureDimensions(color_atlas_texture);
            break;
        }
        case 1u: {
            dim = textureDimensions(mask_atlas_texture);
            break;
        }
        default: {}
    }

    vert_output.content_type = content_type;

    vert_output.uv = vec2<f32>(uv) / vec2<f32>(dim);

    return vert_output;
}

@fragment
fn fs_main(in_frag: VertexOutput) -> @location(0) vec4<f32> {
    switch in_frag.content_type {
        case 0u: {
            return textureSampleLevel(color_atlas_texture, atlas_sampler, in_frag.uv, 0.0);
        }
        case 1u: {
            let cov = textureSampleLevel(mask_atlas_texture, atlas_sampler, in_frag.uv, 0.0).x;
            // Skia applies the table to the 8-bit coverage.
            let a = sk_preblend(round(cov * 255.0) / 255.0, sk_luminance(in_frag.color.rgb));
            return vec4<f32>(in_frag.color.rgb, in_frag.color.a * a);
        }
        default: {
            return vec4<f32>(0.0);
        }
    }
}
