#[wgsl_rs::wgsl]
pub mod surface {
    use super::super::common::*;
    use wgsl_rs::std::*;

    #[repr(C)]
    #[derive(Clone, Copy, Wgsl)]
    pub struct SurfaceUniforms {
        pub bounds: Bounds,
        pub content_mask: ContentMask,
        pub color_format: SurfaceColorFormat,
        pub opacity: f32,
        pub padding0: u32,
        pub padding1: u32,
        // Hydra patch: rounds the corners of `bounds` (16-byte aligned, as WGSL
        // and HLSL constant buffers place a struct).
        pub corner_radii: Corners,
    }
    uniform!(group(1), binding(0), SURFACE_LOCALS: SurfaceUniforms);
    texture!(group(1), binding(1), SURFACE_TEXTURE: Texture2D<f32>);
    texture!(group(1), binding(2), SURFACE_CHROMA_TEXTURE: Texture2D<f32>);
    sampler!(group(1), binding(3), SURFACE_SAMPLER: Sampler);

    // Hydra patch: limited-range ("video") YCbCr, 8-bit levels in 0-1 units.
    /// Y's black level (16 / 255).
    pub const LIMITED_LUMA_BLACK: f32 = 0.0627451;
    /// Y's scale to full range (255 / 219).
    pub const LIMITED_LUMA_SCALE: f32 = 1.1643836;
    /// CbCr's zero (128 / 255).
    pub const CHROMA_ZERO: f32 = 0.5019608;
    /// CbCr's scale to full range (255 / 224).
    pub const LIMITED_CHROMA_SCALE: f32 = 1.1383929;
    /// BT.709 (HD) YCbCr to R'G'B': Cr into R, Cb and Cr out of G, Cb into B.
    pub const BT709_CR_TO_R: f32 = 1.5748;
    pub const BT709_CB_TO_G: f32 = 0.1873243;
    pub const BT709_CR_TO_G: f32 = 0.4681243;
    pub const BT709_CB_TO_B: f32 = 1.8556;
    /// BT.601 (SD) YCbCr to R'G'B'.
    pub const BT601_CR_TO_R: f32 = 1.402;
    pub const BT601_CB_TO_G: f32 = 0.3441363;
    pub const BT601_CR_TO_G: f32 = 0.7141363;
    pub const BT601_CB_TO_B: f32 = 1.772;

    pub const YCBCR_TO_LINEAR_RGB: Mat4x4f = mat4x4f(
        vec4f(1.0000, 1.0000, 1.0000, 0.0),
        vec4f(0.0000, -0.3441, 1.7720, 0.0),
        vec4f(1.4020, -0.7141, 0.0000, 0.0),
        vec4f(-0.7010, 0.5291, -0.8860, 1.0),
    );

    pub fn sample_yuv_surface(texture_position: Vec2f, color_format: SurfaceColorFormat) -> Vec4f {
        let luma = texture_sample_level(SURFACE_TEXTURE, SURFACE_SAMPLER, texture_position, 0.0).x;
        let chroma = texture_sample_level(
            SURFACE_CHROMA_TEXTURE,
            SURFACE_SAMPLER,
            texture_position,
            0.0,
        )
        .xy();
        if color_format == SurfaceColorFormat::Yuv {
            return YCBCR_TO_LINEAR_RGB * vec4f(luma, chroma.x, chroma.y, 1.0);
        }
        // Hydra patch: limited range (Y 16-235, CbCr 16-240), then the matrix.
        let y = (luma - LIMITED_LUMA_BLACK) * LIMITED_LUMA_SCALE;
        let cb = (chroma.x - CHROMA_ZERO) * LIMITED_CHROMA_SCALE;
        let cr = (chroma.y - CHROMA_ZERO) * LIMITED_CHROMA_SCALE;
        if color_format == SurfaceColorFormat::Yuv709Limited {
            return vec4f(
                saturate(y + BT709_CR_TO_R * cr),
                saturate(y - BT709_CB_TO_G * cb - BT709_CR_TO_G * cr),
                saturate(y + BT709_CB_TO_B * cb),
                1.0,
            );
        }
        vec4f(
            saturate(y + BT601_CR_TO_R * cr),
            saturate(y - BT601_CB_TO_G * cb - BT601_CR_TO_G * cr),
            saturate(y + BT601_CB_TO_B * cb),
            1.0,
        )
    }

    #[derive(Wgsl)]
    pub struct SurfaceVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub texture_position: Vec2f,
        #[location(3)]
        pub clip_distances: Vec4f,
    }

    #[vertex]
    pub fn vertex_surface(#[builtin(vertex_index)] vertex_id: u32) -> SurfaceVarying {
        let vertex = rectangle_vertex(vertex_id, get!(SURFACE_LOCALS).bounds);
        SurfaceVarying {
            position: vertex.clip_position,
            texture_position: vertex.unit_position,
            clip_distances: clip_distances(
                vertex.viewport_position,
                get!(SURFACE_LOCALS).content_mask.bounds,
            ),
        }
    }

    #[fragment]
    pub fn fragment_surface(input: SurfaceVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let locals = get!(SURFACE_LOCALS);
        let mut fade = ContentMask::alpha(locals.content_mask, input.position.xy());
        // Hydra patch: rounded corners, antialiased like a quad's.
        if !Corners::is_zero(locals.corner_radii) {
            let distance = rounded_rectangle_signed_distance(
                input.position.xy(),
                locals.bounds,
                locals.corner_radii,
            );
            fade *= antialiased_coverage(distance);
        }
        if locals.color_format != SurfaceColorFormat::Rgba {
            return sample_yuv_surface(input.texture_position, locals.color_format)
                * locals.opacity
                * fade;
        }
        texture_sample_level(
            SURFACE_TEXTURE,
            SURFACE_SAMPLER,
            input.texture_position,
            0.0,
        ) * locals.opacity
            * fade
    }
}

#[wgsl_rs::wgsl]
pub mod blur {
    use super::super::common::*;
    use super::super::corner_smoothing::*;
    use wgsl_rs::std::*;

    #[repr(C)]
    #[derive(Clone, Copy, Wgsl)]
    pub struct BlurUniforms {
        pub bounds: Bounds,
        pub content_mask: Bounds,
        pub corner_radii: Corners,
        pub direction: Vec2f,
        pub standard_deviation: f32,
        pub opacity: f32,
        pub sample_count: u32,
        pub sample_step: f32,
        pub composite_clip: BlurCompositeClip,
        pub downsample_mode: DownsampleMode,
        pub source_size: Vec2f,
        pub target_size: Vec2f,
        pub corner_smoothing: f32,
        pub padding0: u32,
        pub padding1: u32,
        pub padding2: u32,
    }
    uniform!(group(1), binding(0), BLUR_LOCALS: BlurUniforms);
    texture!(group(1), binding(1), BLUR_TEXTURE: Texture2D<f32>);
    sampler!(group(1), binding(2), BLUR_SAMPLER: Sampler);

    #[derive(Clone, Copy, Wgsl)]
    pub struct BlurCompositeVertexData {
        pub position: Vec4f,
        pub texture_coordinates: Vec2f,
        pub clip_distances: Vec4f,
    }

    pub fn prepare_blur_composite_vertex(vertex_id: u32) -> BlurCompositeVertexData {
        let vertex = rectangle_vertex(vertex_id, get!(BLUR_LOCALS).bounds);
        BlurCompositeVertexData {
            position: vertex.clip_position,
            texture_coordinates: vertex.unit_position,
            clip_distances: clip_distances(
                vertex.viewport_position,
                get!(BLUR_LOCALS).content_mask,
            ),
        }
    }

    /// Hydra patch: a mask group's fade ([`BlurCompositeClip::EdgeFade`]): a
    /// linear ramp from 0 at each edge of `content_mask` to 1 at the edge's
    /// length inward (left, right in `direction`, top in
    /// `standard_deviation`, bottom in `sample_step`); 1 for other composites.
    pub fn edge_fade_alpha(position: Vec2f) -> f32 {
        let uniforms = get!(BLUR_LOCALS);
        if uniforms.composite_clip != BlurCompositeClip::EdgeFade {
            return 1.0;
        }
        let bounds = uniforms.content_mask;
        let mut alpha = 1.0;
        if uniforms.direction.x > 0.0 {
            alpha *= saturate((position.x - bounds.origin.x) / uniforms.direction.x);
        }
        if uniforms.direction.y > 0.0 {
            alpha *= saturate((bounds.origin.x + bounds.size.x - position.x) / uniforms.direction.y);
        }
        if uniforms.standard_deviation > 0.0 {
            alpha *= saturate((position.y - bounds.origin.y) / uniforms.standard_deviation);
        }
        if uniforms.sample_step > 0.0 {
            alpha *= saturate((bounds.origin.y + bounds.size.y - position.y) / uniforms.sample_step);
        }
        alpha
    }

    /// Hydra patch: a transform group's composite ([`BlurCompositeClip::Transform`]): the
    /// output pixel at `position` samples the untransformed group at the inverse transform
    /// (bilinear), inside the group's source box, its edges antialiased over an output pixel.
    pub fn transform_composite_color(position: Vec2f) -> Vec4f {
        let uniforms = get!(BLUR_LOCALS);
        let m = uniforms.corner_radii;
        let source = vec2f(
            m.top_left * position.x + m.top_right * position.y + uniforms.source_size.x,
            m.bottom_right * position.x + m.bottom_left * position.y + uniforms.source_size.y,
        );
        let low = uniforms.direction;
        let high = vec2f(
            uniforms.direction.x + uniforms.standard_deviation,
            uniforms.direction.y + uniforms.sample_step,
        );
        // the distance inside the box along each source axis, in output pixels
        let x_rate = max(sqrt(m.top_left * m.top_left + m.top_right * m.top_right), 0.0001);
        let y_rate = max(
            sqrt(m.bottom_right * m.bottom_right + m.bottom_left * m.bottom_left),
            0.0001,
        );
        let inside_x = min(source.x - low.x, high.x - source.x) / x_rate;
        let inside_y = min(source.y - low.y, high.y - source.y) / y_rate;
        let coverage = saturate(min(inside_x, inside_y) + 0.5);
        let sampled = texture_sample_level(
            BLUR_TEXTURE,
            BLUR_SAMPLER,
            source / uniforms.target_size,
            0.0,
        );
        let factor = coverage * uniforms.opacity;
        vec4f(
            sampled.x * factor,
            sampled.y * factor,
            sampled.z * factor,
            sampled.w * factor,
        )
    }

    pub fn blur_composite_color(position: Vec2f, coverage: f32) -> Vec4f {
        let blurred = texture_sample_level(
            BLUR_TEXTURE,
            BLUR_SAMPLER,
            position / get!(BLUR_LOCALS).target_size,
            0.0,
        );
        let factor = coverage * get!(BLUR_LOCALS).opacity;
        vec4f(
            blurred.x * factor,
            blurred.y * factor,
            blurred.z * factor,
            blurred.w * factor,
        )
    }

    #[derive(Wgsl)]
    pub struct BlurVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub texture_coordinates: Vec2f,
        #[location(3)]
        pub clip_distances: Vec4f,
    }

    #[vertex]
    pub fn vertex_blur_fullscreen(#[builtin(vertex_index)] vertex_id: u32) -> BlurVarying {
        let vertex = fullscreen_vertex(vertex_id);
        BlurVarying {
            position: vertex.clip_position,
            texture_coordinates: vertex.texture_coordinates,
            clip_distances: unclipped_distances(),
        }
    }

    #[fragment]
    pub fn fragment_blur_downsample(input: BlurVarying) -> Vec4f {
        if get!(BLUR_LOCALS).downsample_mode == DownsampleMode::HalfResolution {
            let destination = floor(input.position.xy());
            let source_coordinates = min(
                (destination + 0.5) / get!(BLUR_LOCALS).target_size,
                (get!(BLUR_LOCALS).source_size - 0.5) / get!(BLUR_LOCALS).source_size,
            );
            return texture_sample_level(BLUR_TEXTURE, BLUR_SAMPLER, source_coordinates, 0.0);
        }
        texture_sample_level(BLUR_TEXTURE, BLUR_SAMPLER, input.texture_coordinates, 0.0)
    }

    pub fn sample_blur_texture(texture_coordinates: Vec2f) -> Vec4f {
        texture_sample_level(BLUR_TEXTURE, BLUR_SAMPLER, texture_coordinates, 0.0)
    }

    pub fn gaussian_blur(texture_coordinates: Vec2f) -> Vec4f {
        let uniforms = get!(BLUR_LOCALS);
        let center_weight = gaussian(0.0, uniforms.standard_deviation);
        let mut weighted_color = sample_blur_texture(texture_coordinates) * center_weight;
        let mut weight_sum = center_weight;
        let mut sample_index = 1u32;

        while sample_index <= uniforms.sample_count {
            let distance = sample_index as f32 * uniforms.sample_step;
            let weight = gaussian(distance, uniforms.standard_deviation);
            let coordinate_offset = uniforms.direction * distance;
            let symmetric_pair = sample_blur_texture(texture_coordinates - coordinate_offset)
                + sample_blur_texture(texture_coordinates + coordinate_offset);
            weighted_color = weighted_color + symmetric_pair * weight;
            weight_sum += 2.0 * weight;
            sample_index += 1u32;
        }
        weighted_color / max(weight_sum, MIN_NORMALIZED_WEIGHT)
    }

    #[fragment]
    pub fn fragment_blur(input: BlurVarying) -> Vec4f {
        gaussian_blur(input.texture_coordinates)
    }

    #[vertex]
    pub fn vertex_blur_composite(#[builtin(vertex_index)] vertex_id: u32) -> BlurVarying {
        let vertex = prepare_blur_composite_vertex(vertex_id);
        BlurVarying {
            position: vertex.position,
            texture_coordinates: vertex.texture_coordinates,
            clip_distances: vertex.clip_distances,
        }
    }

    #[fragment]
    pub fn fragment_blur_composite(input: BlurVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        if get!(BLUR_LOCALS).composite_clip == BlurCompositeClip::Transform {
            return transform_composite_color(input.position.xy());
        }
        let coverage = select(
            1.0,
            antialiased_coverage(rounded_rectangle_signed_distance(
                input.position.xy(),
                get!(BLUR_LOCALS).bounds,
                get!(BLUR_LOCALS).corner_radii,
            )),
            get!(BLUR_LOCALS).composite_clip == BlurCompositeClip::RoundedBounds,
        );
        blur_composite_color(
            input.position.xy(),
            coverage * edge_fade_alpha(input.position.xy()),
        )
    }

    #[derive(Wgsl)]
    pub struct SmoothedBlurVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub texture_coordinates: Vec2f,
        #[location(1)]
        #[interpolate(flat)]
        pub horizontal_corner_reaches: Vec4f,
        #[location(2)]
        #[interpolate(flat)]
        pub vertical_corner_reaches: Vec4f,
        #[location(3)]
        pub clip_distances: Vec4f,
        #[location(4)]
        #[interpolate(flat)]
        pub smoothing_factors: Vec4f,
        #[location(5)]
        #[interpolate(flat)]
        pub superellipse_power: f32,
    }

    #[vertex]
    pub fn vertex_smoothed_blur_composite(
        #[builtin(vertex_index)] vertex_id: u32,
    ) -> SmoothedBlurVarying {
        let vertex = prepare_blur_composite_vertex(vertex_id);
        let prepared = prepare_corners(
            get!(BLUR_LOCALS).bounds.size,
            get!(BLUR_LOCALS).corner_radii,
            get!(BLUR_LOCALS).corner_smoothing,
            true,
        );
        SmoothedBlurVarying {
            position: vertex.position,
            texture_coordinates: vertex.texture_coordinates,
            horizontal_corner_reaches: prepared.horizontal_reaches,
            vertical_corner_reaches: prepared.vertical_reaches,
            clip_distances: vertex.clip_distances,
            smoothing_factors: prepared.smoothing_factors,
            superellipse_power: prepared.superellipse_power,
        }
    }

    #[fragment]
    pub fn fragment_smoothed_blur_composite(input: SmoothedBlurVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let coverage = select(
            1.0,
            antialiased_coverage(prepared_corner_signed_distance(
                input.position.xy(),
                get!(BLUR_LOCALS).bounds,
                get!(BLUR_LOCALS).corner_radii,
                get!(BLUR_LOCALS).corner_smoothing,
                PreparedCorners {
                    horizontal_reaches: input.horizontal_corner_reaches,
                    vertical_reaches: input.vertical_corner_reaches,
                    smoothing_factors: input.smoothing_factors,
                    superellipse_power: input.superellipse_power,
                },
            )),
            get!(BLUR_LOCALS).composite_clip == BlurCompositeClip::RoundedBounds,
        );
        blur_composite_color(input.position.xy(), coverage)
    }
}
