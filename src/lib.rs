#![cfg(windows)]
#![deny(missing_docs)]
#![no_std]
//! This crate offers a DirectX 11 renderer for the [imgui-rs](https://docs.rs/imgui/*/imgui/) rust bindings.

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;
use core::{mem, slice};

use windows::core::{Result, PCSTR};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::{
    D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D11_SRV_DIMENSION_TEXTURE2D, D3D_PRIMITIVE_TOPOLOGY,
};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11BlendState, ID3D11Buffer, ID3D11ClassInstance, ID3D11DepthStencilState, ID3D11Device,
    ID3D11DeviceContext, ID3D11GeometryShader, ID3D11InputLayout, ID3D11PixelShader,
    ID3D11RasterizerState, ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D,
    ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_INDEX_BUFFER,
    D3D11_BIND_SHADER_RESOURCE, D3D11_BIND_VERTEX_BUFFER, D3D11_BLEND_DESC,
    D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE, D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA,
    D3D11_BUFFER_DESC, D3D11_COLOR_WRITE_ENABLE_ALL, D3D11_COMPARISON_ALWAYS,
    D3D11_CPU_ACCESS_WRITE, D3D11_CULL_NONE, D3D11_DEPTH_STENCILOP_DESC, D3D11_DEPTH_STENCIL_DESC,
    D3D11_DEPTH_WRITE_MASK_ALL, D3D11_FILL_SOLID, D3D11_FILTER_MIN_MAG_MIP_LINEAR,
    D3D11_INPUT_ELEMENT_DESC, D3D11_INPUT_PER_VERTEX_DATA, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_WRITE_DISCARD, D3D11_RASTERIZER_DESC, D3D11_RENDER_TARGET_BLEND_DESC,
    D3D11_RESOURCE_MISC_FLAG, D3D11_SAMPLER_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC,
    D3D11_STENCIL_OP_KEEP, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
    D3D11_TEXTURE_ADDRESS_WRAP, D3D11_USAGE_DEFAULT, D3D11_USAGE_DYNAMIC, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_R16_UINT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R32_UINT,
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::DXGI_ERROR_INVALID_CALL;


use imgui::internal::RawWrapper;
use imgui::{
    BackendFlags, DrawCmd, DrawCmdParams, DrawData, DrawIdx, DrawVert, TextureId, Textures,
};

const FONT_TEX_ID: usize = !0;

const VERTEX_BUF_ADD_CAPACITY: usize = 5000;
const INDEX_BUF_ADD_CAPACITY: usize = 10000;

#[repr(C)]
struct VertexConstantBuffer {
    mvp: [[f32; 4]; 4],
}

/// A DirectX 11 renderer for (Imgui-rs)[https://docs.rs/imgui/*/imgui/].
#[derive(Debug)]
pub struct Renderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    input_layout: ID3D11InputLayout,
    constant_buffer: ID3D11Buffer,
    blend_state: ID3D11BlendState,
    rasterizer_state: ID3D11RasterizerState,
    depth_stencil_state: ID3D11DepthStencilState,
    font_resource_view: ID3D11ShaderResourceView,
    font_sampler: ID3D11SamplerState,
    vertex_buffer: Buffer,
    index_buffer: Buffer,
    textures: Textures<ID3D11ShaderResourceView>,
}

impl Renderer {
    /// Creates a new renderer for the given [`ID3D11Device`].
    ///
    /// # Safety
    ///
    /// `device` must be a valid [`ID3D11Device`] pointer.
    ///
    /// [`ID3D11Device`]: https://docs.rs/winapi/0.3/x86_64-pc-windows-msvc/winapi/um/d3d11/struct.ID3D11Device.html
    pub unsafe fn new(im_ctx: &mut imgui::Context, device: &ID3D11Device) -> Result<Self> {
        let (vertex_shader, input_layout, constant_buffer) = Self::create_vertex_shader(device)?;
        let pixel_shader = Self::create_pixel_shader(device)?;
        let (blend_state, rasterizer_state, depth_stencil_state) =
            Self::create_device_objects(device)?;
        let (font_resource_view, font_sampler) = Self::create_font_texture(im_ctx.fonts(), device)?;
        let vertex_buffer = Self::create_vertex_buffer(device, 0)?;
        let index_buffer = Self::create_index_buffer(device, 0)?;

        let mut context = None;
        device.GetImmediateContext(&mut context);

        im_ctx.io_mut().backend_flags |= BackendFlags::RENDERER_HAS_VTX_OFFSET;
        let renderer_name = concat!("imgui_dx11_renderer@", env!("CARGO_PKG_VERSION"));
        im_ctx.set_renderer_name(Some(renderer_name.parse().unwrap()));

        Ok(Renderer {
            device: device.clone(),
            context: context.unwrap(),
            vertex_shader,
            pixel_shader,
            input_layout,
            constant_buffer,
            blend_state,
            rasterizer_state,
            depth_stencil_state,
            font_resource_view,
            font_sampler,
            vertex_buffer,
            index_buffer,
            textures: Textures::new(),
        })
    }

    /// The textures registry of this renderer.
    ///
    /// The texture slot at !0 is reserved for the font texture, therefore the
    /// renderer will ignore any texture inserted into said slot.
    #[inline]
    pub fn textures_mut(&mut self) -> &mut Textures<ID3D11ShaderResourceView> {
        &mut self.textures
    }

    /// The textures registry of this renderer.
    #[inline]
    pub fn textures(&self) -> &Textures<ID3D11ShaderResourceView> {
        &self.textures
    }

    /// Renders the given [`Ui`] with this renderer.
    ///
    /// Should the [`DrawData`] contain an invalid texture index the renderer
    /// will return `DXGI_ERROR_INVALID_CALL` and immediately stop rendering.
    ///
    /// [`Ui`]: https://docs.rs/imgui/*/imgui/struct.Ui.html
    pub fn render(&mut self, draw_data: &DrawData) -> Result<()> {
        if draw_data.display_size[0] <= 0.0 || draw_data.display_size[1] <= 0.0 {
            return Ok(());
        }
        unsafe {
            if self.vertex_buffer.len() < draw_data.total_vtx_count as usize {
                self.vertex_buffer =
                    Self::create_vertex_buffer(&self.device, draw_data.total_vtx_count as usize)?;
            }
            if self.index_buffer.len() < draw_data.total_idx_count as usize {
                self.index_buffer =
                    Self::create_index_buffer(&self.device, draw_data.total_idx_count as usize)?;
            }
            let _state_guard = StateBackup::backup(Some(self.context.clone()));

            self.write_buffers(draw_data)?;
            self.setup_render_state(draw_data);
            self.render_impl(draw_data)?;
            _state_guard.restore();
        }
        Ok(())
    }

    unsafe fn render_impl(&self, draw_data: &DrawData) -> Result<()> {
        let clip_off = draw_data.display_pos;
        let clip_scale = draw_data.framebuffer_scale;
        let mut vertex_offset = 0;
        let mut index_offset = 0;
        let mut last_tex = TextureId::from(FONT_TEX_ID);
        let context = &self.context;
        context.PSSetShaderResources(0, Some(&[Some(self.font_resource_view.clone())]));
        for draw_list in draw_data.draw_lists() {
            for cmd in draw_list.commands() {
                match cmd {
                    DrawCmd::Elements {
                        count,
                        cmd_params: DrawCmdParams { clip_rect, texture_id, .. },
                    } => {
                        if texture_id != last_tex {
                            let texture = if texture_id.id() == FONT_TEX_ID {
                                self.font_resource_view.clone()
                            } else {
                                self.textures
                                    .get(texture_id)
                                    .ok_or(DXGI_ERROR_INVALID_CALL)?
                                    .clone()
                            };
                            context.PSSetShaderResources(0, Some(&[Some(texture)]));
                            last_tex = texture_id;
                        }

                        let r = RECT {
                            left: ((clip_rect[0] - clip_off[0]) * clip_scale[0]) as i32,
                            top: ((clip_rect[1] - clip_off[1]) * clip_scale[1]) as i32,
                            right: ((clip_rect[2] - clip_off[0]) * clip_scale[0]) as i32,
                            bottom: ((clip_rect[3] - clip_off[1]) * clip_scale[1]) as i32,
                        };
                        context.RSSetScissorRects(Some(&[r]));
                        context.DrawIndexed(
                            count as u32,
                            index_offset as u32,
                            vertex_offset as i32,
                        );
                        index_offset += count;
                    },
                    DrawCmd::ResetRenderState => self.setup_render_state(draw_data),
                    DrawCmd::RawCallback { callback, raw_cmd } => {
                        callback(draw_list.raw(), raw_cmd)
                    },
                }
            }
            vertex_offset += draw_list.vtx_buffer().len();
        }
        Ok(())
    }

    unsafe fn setup_render_state(&self, draw_data: &DrawData) {
        let ctx = &self.context;
        let vp = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: draw_data.display_size[0],
            Height: draw_data.display_size[1],
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let draw_fmt = if mem::size_of::<DrawIdx>() == 2 {
            DXGI_FORMAT_R16_UINT
        } else {
            DXGI_FORMAT_R32_UINT
        };
        let stride = mem::size_of::<DrawVert>() as u32;
        let blend_factor = 0.0;

        ctx.RSSetViewports(Some(&[vp]));
        ctx.IASetInputLayout(&self.input_layout);
        ctx.IASetVertexBuffers(
            0,
            1,
            Some(&Some(self.vertex_buffer.get_buf().clone())),
            Some(&stride),
            Some(&0),
        );
        ctx.IASetIndexBuffer(self.index_buffer.get_buf(), draw_fmt, 0);
        ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        ctx.VSSetShader(&self.vertex_shader, Some(&[]));
        ctx.VSSetConstantBuffers(0, Some(&[Some(self.constant_buffer.clone())]));
        ctx.PSSetShader(&self.pixel_shader, Some(&[]));
        ctx.PSSetSamplers(0, Some(&[Some(self.font_sampler.clone())]));
        ctx.GSSetShader(None, Some(&[]));
        ctx.HSSetShader(None, Some(&[]));
        ctx.DSSetShader(None, Some(&[]));
        ctx.CSSetShader(None, Some(&[]));
        ctx.OMSetBlendState(&self.blend_state, Some(&blend_factor), 0xFFFFFFFF);
        ctx.OMSetDepthStencilState(&self.depth_stencil_state, 0);
        ctx.RSSetState(&self.rasterizer_state);
    }

    unsafe fn create_vertex_buffer(device: &ID3D11Device, vtx_count: usize) -> Result<Buffer> {
        let len = vtx_count + VERTEX_BUF_ADD_CAPACITY;
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: (len * mem::size_of::<DrawVert>()) as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_VERTEX_BUFFER,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
            MiscFlags: D3D11_RESOURCE_MISC_FLAG(0),
            StructureByteStride: 0,
        };

        device.CreateBuffer(&desc, None).map(|buf| Buffer(buf, len))
    }

    unsafe fn create_index_buffer(device: &ID3D11Device, idx_count: usize) -> Result<Buffer> {
        let len = idx_count + INDEX_BUF_ADD_CAPACITY;
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: (len * mem::size_of::<DrawIdx>()) as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_INDEX_BUFFER,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
            MiscFlags: D3D11_RESOURCE_MISC_FLAG(0),
            StructureByteStride: 0,
        };

        device.CreateBuffer(&desc, None).map(|buf| Buffer(buf, len))
    }

    unsafe fn write_buffers(&self, draw_data: &DrawData) -> Result<()> {
        let vtx_resource: D3D11_MAPPED_SUBRESOURCE =
            self.context.Map(self.vertex_buffer.get_buf(), 0, D3D11_MAP_WRITE_DISCARD, 0)?;
        let idx_resource: D3D11_MAPPED_SUBRESOURCE =
            self.context.Map(self.index_buffer.get_buf(), 0, D3D11_MAP_WRITE_DISCARD, 0)?;

        let mut vtx_dst = slice::from_raw_parts_mut(
            vtx_resource.pData.cast::<DrawVert>(),
            draw_data.total_vtx_count as usize,
        );
        let mut idx_dst = slice::from_raw_parts_mut(
            idx_resource.pData.cast::<DrawIdx>(),
            draw_data.total_idx_count as usize,
        );

        for (vbuf, ibuf) in
            draw_data.draw_lists().map(|draw_list| (draw_list.vtx_buffer(), draw_list.idx_buffer()))
        {
            vtx_dst[..vbuf.len()].copy_from_slice(vbuf);
            idx_dst[..ibuf.len()].copy_from_slice(ibuf);
            vtx_dst = &mut vtx_dst[vbuf.len()..];
            idx_dst = &mut idx_dst[ibuf.len()..];
        }

        self.context.Unmap(self.vertex_buffer.get_buf(), 0);
        self.context.Unmap(self.index_buffer.get_buf(), 0);

        let mapped_resource: D3D11_MAPPED_SUBRESOURCE =
            self.context.Map(&self.constant_buffer, 0, D3D11_MAP_WRITE_DISCARD, 0)?;
        let l = draw_data.display_pos[0];
        let r = draw_data.display_pos[0] + draw_data.display_size[0];
        let t = draw_data.display_pos[1];
        let b = draw_data.display_pos[1] + draw_data.display_size[1];
        let mvp = [
            [2.0 / (r - l), 0.0, 0.0, 0.0],
            [0.0, 2.0 / (t - b), 0.0, 0.0],
            [0.0, 0.0, 0.5, 0.0],
            [(r + l) / (l - r), (t + b) / (b - t), 0.5, 1.0],
        ];
        *mapped_resource.pData.cast::<VertexConstantBuffer>() = VertexConstantBuffer { mvp };
        self.context.Unmap(&self.constant_buffer, 0);

        Ok(())
    }

    unsafe fn create_font_texture(
        fonts: &mut imgui::FontAtlas,
        device: &ID3D11Device,
    ) -> Result<(ID3D11ShaderResourceView, ID3D11SamplerState)> {
        let fa_tex = fonts.build_rgba32_texture();

        let desc = D3D11_TEXTURE2D_DESC {
            Width: fa_tex.width,
            Height: fa_tex.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE,
            ..Default::default()
        };
        let sub_resource = D3D11_SUBRESOURCE_DATA {
            pSysMem: fa_tex.data.as_ptr().cast(),
            SysMemPitch: desc.Width * 4,
            SysMemSlicePitch: 0,
        };

        let texture: ID3D11Texture2D = device.CreateTexture2D(&desc, Some(&sub_resource))?;
        let mut srv_desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            ViewDimension: D3D11_SRV_DIMENSION_TEXTURE2D,
            ..Default::default()
        };
        srv_desc.Anonymous.Texture2D.MipLevels = desc.MipLevels;
        srv_desc.Anonymous.Texture2D.MostDetailedMip = 0;
        let font_texture_view = device.CreateShaderResourceView(&texture, Some(&srv_desc))?;

        fonts.tex_id = TextureId::from(FONT_TEX_ID);

        let desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D11_TEXTURE_ADDRESS_WRAP,
            AddressV: D3D11_TEXTURE_ADDRESS_WRAP,
            AddressW: D3D11_TEXTURE_ADDRESS_WRAP,
            MipLODBias: 0.0,
            ComparisonFunc: D3D11_COMPARISON_ALWAYS,
            MinLOD: 0.0,
            MaxLOD: 0.0,
            ..Default::default()
        };
        let font_sampler = device.CreateSamplerState(&desc)?;
        Ok((font_texture_view, font_sampler))
    }

    unsafe fn create_vertex_shader(
        device: &ID3D11Device,
    ) -> Result<(ID3D11VertexShader, ID3D11InputLayout, ID3D11Buffer)> {
        const VERTEX_SHADER: &[u8] =
            include_bytes!(concat!(env!("OUT_DIR"), "/vertex_shader.vs_4_0"));
        let vs_shader = device.CreateVertexShader(VERTEX_SHADER, None)?;

        let local_layout = [
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR(b"POSITION\0".as_ptr()),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 0,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR(b"TEXCOORD\0".as_ptr()),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 8,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR(b"COLOR\0".as_ptr()),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                InputSlot: 0,
                AlignedByteOffset: 16,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
        ];

        let input_layout = device.CreateInputLayout(&local_layout, VERTEX_SHADER)?;

        let desc = D3D11_BUFFER_DESC {
            ByteWidth: mem::size_of::<VertexConstantBuffer>() as _,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
            MiscFlags: D3D11_RESOURCE_MISC_FLAG(0),
            StructureByteStride: 0,
        };
        let vertex_constant_buffer = device.CreateBuffer(&desc, None)?;
        Ok((vs_shader, input_layout, vertex_constant_buffer))
    }

    unsafe fn create_pixel_shader(device: &ID3D11Device) -> Result<ID3D11PixelShader> {
        const PIXEL_SHADER: &[u8] =
            include_bytes!(concat!(env!("OUT_DIR"), "/pixel_shader.ps_4_0"));
        device.CreatePixelShader(PIXEL_SHADER, None)
    }

    unsafe fn create_device_objects(
        device: &ID3D11Device,
    ) -> Result<(ID3D11BlendState, ID3D11RasterizerState, ID3D11DepthStencilState)> {
        let desc = D3D11_BLEND_DESC {
            AlphaToCoverageEnable: false.into(),
            IndependentBlendEnable: true.into(),
            RenderTarget: [D3D11_RENDER_TARGET_BLEND_DESC {
                BlendEnable: true.into(),
                SrcBlend: D3D11_BLEND_SRC_ALPHA,
                DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOp: D3D11_BLEND_OP_ADD,
                SrcBlendAlpha: D3D11_BLEND_ONE,
                DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOpAlpha: D3D11_BLEND_OP_ADD,
                RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
            }; 8],
        };
        let blend_state = device.CreateBlendState(&desc)?;

        let desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            DepthClipEnable: true.into(),
            ScissorEnable: true.into(),
            ..Default::default()
        };
        let rasterizer_state = device.CreateRasterizerState(&desc)?;

        let stencil_op_desc = D3D11_DEPTH_STENCILOP_DESC {
            StencilFailOp: D3D11_STENCIL_OP_KEEP,
            StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
            StencilPassOp: D3D11_STENCIL_OP_KEEP,
            StencilFunc: D3D11_COMPARISON_ALWAYS,
        };
        let desc = D3D11_DEPTH_STENCIL_DESC {
            DepthEnable: false.into(),
            DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL,
            DepthFunc: D3D11_COMPARISON_ALWAYS,
            StencilEnable: false.into(),
            StencilReadMask: 0,
            StencilWriteMask: 0,
            FrontFace: stencil_op_desc,
            BackFace: stencil_op_desc,
        };
        let depth_stencil_state = device.CreateDepthStencilState(&desc)?;
        Ok((blend_state, rasterizer_state, depth_stencil_state))
    }
}

#[derive(Debug)]
struct Buffer(ID3D11Buffer, usize);

impl Buffer {
    #[inline]
    fn len(&self) -> usize {
        self.1
    }
    #[inline]
    fn get_buf(&self) -> &ID3D11Buffer {
        &self.0
    }
}

#[derive(Debug, Default)]
struct StateBackup {
    context: Option<ID3D11DeviceContext>,
    scissor_rects: RECT,
    viewports: D3D11_VIEWPORT,
    rasterizer_state: Option<ID3D11RasterizerState>,
    blend_state: Option<ID3D11BlendState>,
    blend_factor: f32,
    sample_mask: u32,
    depth_stencil_state: Option<ID3D11DepthStencilState>,
    stencil_ref: u32,
    shader_resource: Vec<Option<ID3D11ShaderResourceView>>,
    sampler: Vec<Option<ID3D11SamplerState>>,
    ps_shader: Option<ID3D11PixelShader>,
    ps_instances: Option<ID3D11ClassInstance>,
    vs_shader: Option<ID3D11VertexShader>,
    vs_instances: Option<ID3D11ClassInstance>,
    constant_buffer: Vec<Option<ID3D11Buffer>>,
    gs_shader: Option<ID3D11GeometryShader>,
    gs_instances: Option<ID3D11ClassInstance>,
    index_buffer: Option<ID3D11Buffer>,
    index_buffer_offset: u32,
    index_buffer_format: DXGI_FORMAT,
    vertex_buffer: Option<ID3D11Buffer>,
    vertex_buffer_offset: u32,
    vertex_buffer_stride: u32,
    topology: D3D_PRIMITIVE_TOPOLOGY,
    input_layout: Option<ID3D11InputLayout>,
}

impl StateBackup {
    unsafe fn backup(context: Option<ID3D11DeviceContext>) -> Self {
        let mut result = Self::default();

        let ctx = context.as_ref().unwrap();
        ctx.RSGetScissorRects(&mut 16, Some(&mut result.scissor_rects));
        ctx.RSGetViewports(&mut 16, Some(&mut result.viewports));
        ctx.RSGetState(Some(&mut result.rasterizer_state));
        ctx.OMGetBlendState(
            Some(&mut result.blend_state),
            Some(&mut result.blend_factor),
            Some(&mut result.sample_mask),
        );
        ctx.OMGetDepthStencilState(
            Some(&mut result.depth_stencil_state),
            Some(&mut result.stencil_ref),
        );
        ctx.PSGetShaderResources(0, Some(&mut result.shader_resource));
        ctx.PSGetSamplers(0, Some(&mut result.sampler));
        ctx.PSGetShader(
            Some(&mut result.ps_shader),
            Some(&mut result.ps_instances),
            Some(&mut 256),
        );
        ctx.VSGetShader(
            Some(&mut result.vs_shader),
            Some(&mut result.vs_instances),
            Some(&mut 256),
        );
        ctx.VSGetConstantBuffers(0, Some(&mut result.constant_buffer));
        ctx.GSGetShader(
            Some(&mut result.gs_shader),
            Some(&mut result.gs_instances),
            Some(&mut 256),
        );
        ctx.IAGetPrimitiveTopology(&mut result.topology);
        ctx.IAGetIndexBuffer(
            Some(&mut result.index_buffer),
            Some(&mut result.index_buffer_format),
            Some(&mut result.index_buffer_offset),
        );
        ctx.IAGetVertexBuffers(
            0,
            1,
            Some(&mut result.vertex_buffer),
            Some(&mut result.vertex_buffer_stride),
            Some(&mut result.vertex_buffer_offset),
        );
        ctx.IAGetInputLayout(Some(&mut result.input_layout));
        result.context = context;
        result
    }
    pub fn restore(mut self) {
        unsafe {
            let ctx = self.context.as_ref().unwrap();
            let inst =
                if self.ps_instances.is_some() { vec![self.ps_instances.take()] } else { vec![] };
            let vinst =
                if self.vs_instances.is_some() { vec![self.vs_instances.take()] } else { vec![] };

            ctx.RSSetScissorRects(Some(&[self.scissor_rects]));
            ctx.RSSetViewports(Some(&[self.viewports]));
            if let Some(rasterizer_state) = self.rasterizer_state {
                ctx.RSSetState(&rasterizer_state)
            }
            if let Some(blend_state) = self.blend_state {
                ctx.OMSetBlendState(&blend_state, Some(&self.blend_factor), 0xFFFFFFFF);
            }
            if let Some(depth_stencil_state) = self.depth_stencil_state {
                ctx.OMSetDepthStencilState(&depth_stencil_state, self.stencil_ref);
            }
            ctx.PSSetShaderResources(0, Some(&self.shader_resource));
            ctx.PSSetSamplers(0, Some(&self.sampler));
            if let Some(ps_shader) = self.ps_shader {
                ctx.PSSetShader(&ps_shader, Some(&inst));
            }
            if let Some(vs_shader) = self.vs_shader {
                ctx.VSSetShader(&vs_shader, Some(&vinst));
            }
            ctx.VSSetConstantBuffers(0, Some(&self.constant_buffer));
            if let Some(gs_shader) = self.gs_shader {
                ctx.GSSetShader(&gs_shader, Some(&[]));
            }
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            if let Some(index_buffer) = self.index_buffer {
                ctx.IASetIndexBuffer(
                    &index_buffer,
                    self.index_buffer_format,
                    self.index_buffer_offset,
                );
            }
            ctx.IASetVertexBuffers(
                0,
                1,
                Some(&self.vertex_buffer),
                Some(&self.vertex_buffer_stride),
                Some(&self.vertex_buffer_offset),
            );
            if let Some(input_layout) = self.input_layout {
                ctx.IASetInputLayout(&input_layout);
            }
        }
    }
}
