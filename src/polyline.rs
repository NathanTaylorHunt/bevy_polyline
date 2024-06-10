use crate::material::PolylineMaterial;
use bevy::{
    core::cast_slice,
    ecs::{
        query::ROQueryItem,
        system::{
            lifetimeless::{Read, SRes},
            SystemParamItem,
        },
    },
    prelude::*,
    reflect::TypePath,
    render::{
        extract_component::{ComponentUniforms, DynamicUniformIndex, UniformComponentPlugin},
        render_asset::{RenderAsset, RenderAssetPlugin, RenderAssetUsages, RenderAssets},
        render_phase::{PhaseItem, RenderCommand, RenderCommandResult, TrackedRenderPass},
        render_resource::*,
        renderer::RenderDevice,
        texture::BevyDefault,
        view::{ViewUniform, ViewUniforms},
        Extract, Render, RenderApp, RenderSet,
    },
};

pub struct PolylineBasePlugin;

impl Plugin for PolylineBasePlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<Polyline>()
            .add_plugins(RenderAssetPlugin::<Polyline>::default());
    }
}

pub struct PolylineRenderPlugin;
impl Plugin for PolylineRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(UniformComponentPlugin::<PolylineUniform>::default());
    }

    fn finish(&self, app: &mut App) {
        app.sub_app_mut(RenderApp)
            .init_resource::<PolylinePipeline>()
            .add_systems(ExtractSchedule, extract_polylines)
            .add_systems(
                Render,
                (
                    prepare_polyline_bind_group.in_set(RenderSet::PrepareBindGroups),
                    prepare_polyline_view_bind_groups.in_set(RenderSet::PrepareBindGroups),
                ),
            );
    }
}

#[derive(Bundle, Default)]
pub struct PolylineBundle {
    pub polyline: Handle<Polyline>,
    pub material: Handle<PolylineMaterial>,
    pub transform: Transform,
    pub global_transform: GlobalTransform,
    /// User indication of whether an entity is visible
    pub visibility: Visibility,
    /// Algorithmically-computed indication of whether an entity is visible and should be extracted for rendering
    pub inherited_visibility: InheritedVisibility,
    pub view_visibility: ViewVisibility,
}

#[derive(Debug, Copy, Clone)]
pub struct IndexRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Default, Asset, Clone, TypePath)]
// #[uuid = "c76af88a-8afe-405c-9a64-0a7d845d2546"]
pub struct Polyline {
    pub vertices: Vec<Vec3>,
    pub current_vertex_index: usize,
    pub index_ranges: Vec<IndexRange>,
    pub current_index_index: usize,
}

impl Polyline {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            vertices: vec![Vec3::ZERO; capacity],
            index_ranges: vec![IndexRange { start: 0, end: 0 }; capacity],  // todo add extra param for this capacity
            current_vertex_index: 0,
            current_index_index: 0,
        }
    }

    pub fn add_vertex(&mut self, vertex: Vec3, connected: bool) {
        // add vertex to buffer
        self.vertices[self.current_vertex_index] = vertex;
        
        // update index ranges
        if connected {
            let index_range = &mut self.index_ranges[self.current_index_index];
            index_range.end = self.current_vertex_index as u32 + 1;  // todo clamp
        } else {
            self.current_index_index = (self.current_index_index + 1) % self.index_ranges.capacity();
            let index_range = &mut self.index_ranges[self.current_index_index];
            index_range.start = self.current_vertex_index as u32;
            index_range.end = self.current_vertex_index as u32 + 1;  // todo clamp
        }

        // update ring indices
        self.current_vertex_index = (self.current_vertex_index + 1) % self.vertices.capacity() - 1;
    }
}

impl RenderAsset for Polyline {
    type PreparedAsset = GpuPolyline;

    type Param = SRes<RenderDevice>;

    fn asset_usage(&self) -> bevy::render::render_asset::RenderAssetUsages {
        RenderAssetUsages::default()
    }

    fn prepare_asset(
        self,
        render_device: &mut SystemParamItem<Self::Param>,
    ) -> Result<Self::PreparedAsset, bevy::render::render_asset::PrepareAssetError<Self>> {
        let vertex_buffer_data = cast_slice(self.vertices.as_slice());
        let vertex_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
            usage: BufferUsages::VERTEX,
            label: Some("Polyline Vertex Buffer"),
            contents: vertex_buffer_data,
        });

        Ok(GpuPolyline {
            vertex_buffer,
            index_ranges: self.index_ranges.clone(),  // !!!
            // vertex_count: self.vertices.len() as u32,
        })
    }
}

#[derive(Component, Clone, ShaderType)]
pub struct PolylineUniform {
    pub transform: Mat4,
    //pub inverse_transpose_model: Mat4,
}

/// The GPU-representation of a [`Polyline`]
#[derive(Debug, Clone)]
pub struct GpuPolyline {
    pub vertex_buffer: Buffer,
    pub index_ranges: Vec<IndexRange>,
    // pub vertex_count: u32,
}

pub fn extract_polylines(
    mut commands: Commands,
    mut previous_len: Local<usize>,
    query: Extract<
        Query<(
            Entity,
            &InheritedVisibility,
            &ViewVisibility,
            &GlobalTransform,
            &Handle<Polyline>,
        )>,
    >,
) {
    let mut values = Vec::with_capacity(*previous_len);
    for (entity, inherited_visibility, view_visibility, transform, handle) in query.iter() {
        if !inherited_visibility.get() || !view_visibility.get() {
            continue;
        }
        let transform = transform.compute_matrix();
        values.push((
            entity,
            (
                handle.clone_weak(),
                PolylineUniform {
                    transform,
                    //inverse_transpose_model: transform.inverse().transpose(),
                },
            ),
        ));
    }
    *previous_len = values.len();
    commands.insert_or_spawn_batch(values);
}

#[derive(Clone, Resource)]
pub struct PolylinePipeline {
    pub view_layout: BindGroupLayout,
    pub polyline_layout: BindGroupLayout,
    pub shader: Handle<Shader>,
}

impl FromWorld for PolylinePipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.get_resource::<RenderDevice>().unwrap();
        let view_layout = render_device.create_bind_group_layout(
            Some("polyline_view_layout"),
            &[
                // View
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: BufferSize::new(ViewUniform::min_size().into()),
                    },
                    count: None,
                },
            ],
        );

        let polyline_layout = render_device.create_bind_group_layout(
            Some("polyline_layout"),
            &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: BufferSize::new(PolylineUniform::min_size().into()),
                },
                count: None,
            }],
        );

        PolylinePipeline {
            view_layout,
            polyline_layout,
            shader: crate::SHADER_HANDLE,
        }
    }
}

impl SpecializedRenderPipeline for PolylinePipeline {
    type Key = PolylinePipelineKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        let vertex_attributes = vec![
            VertexAttribute {
                format: VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            VertexAttribute {
                format: VertexFormat::Float32x3,
                offset: 12,
                shader_location: 1,
            },
        ];
        let shader_defs = Vec::new();
        let (label, blend, depth_write_enabled);

        if key.contains(PolylinePipelineKey::TRANSPARENT_MAIN_PASS) {
            label = "transparent_polyline_pipeline".into();
            blend = Some(BlendState::ALPHA_BLENDING);
            // For the transparent pass, fragments that are closer will be alpha blended
            // but their depth is not written to the depth buffer
            depth_write_enabled = false;
        } else if key.contains(PolylinePipelineKey::PERSPECTIVE) {
            // We need to use transparent pass with perspective to support thin line fading.
            label = "transparent_polyline_pipeline".into();
            blend = Some(BlendState::ALPHA_BLENDING);
            // Because we are expecting an opaque matl we should enable depth writes, as we don't
            // need to blend most lines.
            depth_write_enabled = true;
        } else {
            label = "opaque_polyline_pipeline".into();
            blend = Some(BlendState::REPLACE);
            // For the opaque and alpha mask passes, fragments that are closer will replace
            // the current fragment value in the output and the depth is written to the
            // depth buffer
            depth_write_enabled = true;
        }

        let format = match key.contains(PolylinePipelineKey::HDR) {
            true => bevy::render::view::ViewTarget::TEXTURE_FORMAT_HDR,
            false => TextureFormat::bevy_default(),
        };

        RenderPipelineDescriptor {
            vertex: VertexState {
                shader: self.shader.clone(),
                entry_point: "vertex".into(),
                shader_defs: shader_defs.clone(),
                buffers: vec![VertexBufferLayout {
                    array_stride: 12,
                    step_mode: VertexStepMode::Instance,
                    attributes: vertex_attributes,
                }],
            },
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs,
                entry_point: "fragment".into(),
                targets: vec![Some(ColorTargetState {
                    format,
                    blend,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            layout: vec![], // This is set in `PolylineMaterialPipeline::specialize()`
            primitive: PrimitiveState {
                front_face: FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
                topology: PrimitiveTopology::TriangleList,
                strip_index_format: None,
            },
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled,
                depth_compare: CompareFunction::Greater,
                stencil: StencilState {
                    front: StencilFaceState::IGNORE,
                    back: StencilFaceState::IGNORE,
                    read_mask: 0,
                    write_mask: 0,
                },
                bias: DepthBiasState {
                    constant: 0,
                    slope_scale: 0.0,
                    clamp: 0.0,
                },
            }),
            multisample: MultisampleState {
                count: key.msaa_samples(),
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            label: Some(label),
            push_constant_ranges: vec![],
        }
    }
}

bitflags::bitflags! {
    #[repr(transparent)]
    #[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Clone, Copy)]
    // NOTE: Apparently quadro drivers support up to 64x MSAA.
    // MSAA uses the highest 3 bits for the MSAA log2(sample count) to support up to 128x MSAA.
    pub struct PolylinePipelineKey: u32 {
        const NONE = 0;
        const PERSPECTIVE = (1 << 0);
        const TRANSPARENT_MAIN_PASS = (1 << 1);
        const HDR = (1 << 2);
        const MSAA_RESERVED_BITS = Self::MSAA_MASK_BITS << Self::MSAA_SHIFT_BITS;
    }
}

impl PolylinePipelineKey {
    const MSAA_MASK_BITS: u32 = 0b111;
    const MSAA_SHIFT_BITS: u32 = 32 - Self::MSAA_MASK_BITS.count_ones();

    pub fn from_msaa_samples(msaa_samples: u32) -> Self {
        let msaa_bits =
            (msaa_samples.trailing_zeros() & Self::MSAA_MASK_BITS) << Self::MSAA_SHIFT_BITS;
        Self::from_bits_retain(msaa_bits)
    }

    pub fn msaa_samples(&self) -> u32 {
        1 << ((self.bits() >> Self::MSAA_SHIFT_BITS) & Self::MSAA_MASK_BITS)
    }

    pub fn from_hdr(hdr: bool) -> Self {
        if hdr {
            PolylinePipelineKey::HDR
        } else {
            PolylinePipelineKey::NONE
        }
    }
}

#[derive(Resource)]
pub struct PolylineBindGroup {
    pub value: BindGroup,
}

pub fn prepare_polyline_bind_group(
    mut commands: Commands,
    polyline_pipeline: Res<PolylinePipeline>,
    render_device: Res<RenderDevice>,
    polyline_uniforms: Res<ComponentUniforms<PolylineUniform>>,
) {
    if let Some(binding) = polyline_uniforms.uniforms().binding() {
        commands.insert_resource(PolylineBindGroup {
            value: render_device.create_bind_group(
                Some("polyline_bind_group"),
                &polyline_pipeline.polyline_layout,
                &[BindGroupEntry {
                    binding: 0,
                    resource: binding,
                }],
            ),
        });
    }
}

#[derive(Component)]
pub struct PolylineViewBindGroup {
    pub value: BindGroup,
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_polyline_view_bind_groups(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    polyline_pipeline: Res<PolylinePipeline>,
    view_uniforms: Res<ViewUniforms>,
    views: Query<Entity, With<bevy::render::view::ExtractedView>>,
) {
    if let Some(view_binding) = view_uniforms.uniforms.binding() {
        for entity in views.iter() {
            let view_bind_group = render_device.create_bind_group(
                Some("polyline_view_bind_group"),
                &polyline_pipeline.view_layout,
                &[BindGroupEntry {
                    binding: 0,
                    resource: view_binding.clone(),
                }],
            );

            commands.entity(entity).insert(PolylineViewBindGroup {
                value: view_bind_group,
            });
        }
    }
}

pub struct SetPolylineBindGroup<const I: usize>;
impl<const I: usize, P: PhaseItem> RenderCommand<P> for SetPolylineBindGroup<I> {
    type ViewQuery = ();
    type ItemQuery = Read<DynamicUniformIndex<PolylineUniform>>;
    type Param = SRes<PolylineBindGroup>;

    #[inline]
    fn render<'w>(
        _item: &P,
        _view: ROQueryItem<'w, Self::ViewQuery>,
        polyline_index: Option<ROQueryItem<'w, Self::ItemQuery>>,
        bind_group: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        pass.set_bind_group(I, &bind_group.into_inner().value, &[polyline_index.unwrap().index()]);
        RenderCommandResult::Success
    }
}

pub struct DrawPolyline;
impl<P: PhaseItem> RenderCommand<P> for DrawPolyline {
    type ViewQuery = ();
    type ItemQuery = Read<Handle<Polyline>>;
    type Param = SRes<RenderAssets<Polyline>>;

    #[inline]
    fn render<'w>(
        _item: &P,
        _view: ROQueryItem<'w, Self::ViewQuery>,
        pl_handle: Option<ROQueryItem<'w, Self::ItemQuery>>,
        polylines: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        if let Some(gpu_polyline) = polylines.into_inner().get(pl_handle.unwrap()) {
            pass.set_vertex_buffer(0, gpu_polyline.vertex_buffer.slice(..));
            for range in &gpu_polyline.index_ranges {
                if range.start != 0 && range.end != 0 {
                    pass.draw(0..6, range.start..range.end);
                }
            }
            RenderCommandResult::Success
        } else {
            RenderCommandResult::Failure
        }
    }
}
