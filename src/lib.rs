#![cfg(windows)]
#![forbid(rust_2018_idioms)]
#![deny(missing_docs)]
//! This crate offers a DirectX 11 renderer for the [imgui-rs](https://docs.rs/imgui/*/imgui/) rust bindings.

use imgui::internal::RawWrapper;
use imgui::{
    BackendFlags, Context, DrawCmd, DrawCmdParams, DrawData, DrawIdx, DrawVert, ImString,
    TextureId, Textures,
};

use winapi::shared::minwindef::{FALSE, TRUE};
use winapi::shared::winerror::S_OK;
use winapi::Interface as _;

use winapi::shared::dxgi::*;
use winapi::shared::dxgiformat::*;
use winapi::shared::dxgitype::*;

use winapi::um::d3d11::*;
use winapi::um::d3dcommon::*;
use winapi::um::d3dcompiler::*;

use core::fmt;
use core::mem;
use core::ptr::{self, NonNull};
use core::slice;

const FONT_TEX_ID: usize = !0;

const VERTEX_BUF_ADD_CAPACITY: usize = 5000;
const INDEX_BUF_ADD_CAPACITY: usize = 10000;

#[repr(C)]
struct VertexConstantBuffer {
    mvp: [[f32; 4]; 4],
}

type Result<T> = core::result::Result<T, RendererError>;

/// The error type returned by the renderer.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum RendererError {
    /// The directx device ran out of memory
    OutOfMemory,
    /// The renderer received an invalid texture id
    InvalidTexture(TextureId),
    ///
    FactoryAquisition,
}

impl fmt::Display for RendererError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            RendererError::OutOfMemory => write!(f, "device ran out of memory"),
            RendererError::InvalidTexture(id) => {
                write!(f, "failed to find texture with id {:?}", id)
            },
            RendererError::FactoryAquisition => write!(f, "unable to acquire IDXGIFactory"),
        }
    }
}

impl std::error::Error for RendererError {}

/// A DirectX 11 renderer for (Imgui-rs)[https://docs.rs/imgui/*/imgui/].
#[derive(Debug)]
pub struct Renderer {
    device: NonNull<ID3D11Device>,
    context: NonNull<ID3D11DeviceContext>,
    factory: NonNull<IDXGIFactory>,
    vertex_shader: NonNull<ID3D11VertexShader>,
    pixel_shader: NonNull<ID3D11PixelShader>,
    input_layout: NonNull<ID3D11InputLayout>,
    constant_buffer: NonNull<ID3D11Buffer>,
    blend_state: NonNull<ID3D11BlendState>,
    rasterizer_state: NonNull<ID3D11RasterizerState>,
    depth_stencil_state: NonNull<ID3D11DepthStencilState>,
    font_resource_view: NonNull<ID3D11ShaderResourceView>,
    font_sampler: NonNull<ID3D11SamplerState>,
    vertex_buffer: Buffer,
    index_buffer: Buffer,
    textures: Textures<NonNull<ID3D11ShaderResourceView>>,
}

impl Renderer {
    /// Creates a new renderer for the given [`ID3D11Device`] and
    /// [`ID3D11DeviceContext`].
    ///
    /// Internally the renderer will then add a reference through the
    /// COM api via [`IUnknown::AddRef`] and release it via
    /// [`IUnknown::Release`] once dropped.
    ///
    /// [`ID3D11Device`]: https://docs.rs/winapi/0.3/x86_64-pc-windows-msvc/winapi/um/d3d11/struct.ID3D11Device.html
    /// [`ID3D11DeviceContext`]: https://docs.rs/winapi/0.3/x86_64-pc-windows-msvc/winapi/um/d3d11/struct.ID3D11DeviceContext.html
    /// [`IUnknown::AddRef`]: https://docs.rs/winapi/0.3/x86_64-pc-windows-msvc/winapi/um/unknwnbase/struct.IUnknown.html#method.AddRef
    /// [`IUnknown::Release`]: https://docs.rs/winapi/0.3/x86_64-pc-windows-msvc/winapi/um/unknwnbase/struct.IUnknown.html#method.Release
    pub fn new(
        ctx: &mut Context,
        mut device: NonNull<ID3D11Device>,
        mut context: NonNull<ID3D11DeviceContext>,
    ) -> Result<Self> {
        unsafe {
            ctx.io_mut().backend_flags |= BackendFlags::RENDERER_HAS_VTX_OFFSET;
            ctx.set_renderer_name(ImString::new(concat!(
                "imgui_dx11_renderer@",
                env!("CARGO_PKG_VERSION")
            )));

            Self::acquire_factory(device).and_then(|factory| {
                let (vertex_shader, input_layout, constant_buffer) =
                    Self::create_vertex_shader(device)?;
                let pixel_shader = Self::create_pixel_shader(device)?;
                let (blend_state, rasterizer_state, depth_stencil_state) =
                    Self::create_device_objects(device);
                let (font_resource_view, font_sampler) =
                    Self::create_font_texture(ctx.fonts(), device)?;
                (device.as_mut()).AddRef();
                (context.as_mut()).AddRef();
                Ok(Renderer {
                    device,
                    context,
                    factory,
                    vertex_shader,
                    pixel_shader,
                    input_layout,
                    constant_buffer,
                    blend_state,
                    rasterizer_state,
                    depth_stencil_state,
                    font_resource_view,
                    font_sampler,
                    vertex_buffer: Self::create_vertex_buffer(device, 0)?,
                    index_buffer: Self::create_index_buffer(device, 0)?,
                    textures: Textures::new(),
                })
            })
        }
    }

    unsafe fn acquire_factory(device: NonNull<ID3D11Device>) -> Result<NonNull<IDXGIFactory>> {
        let mut dxgi_device = ptr::null_mut::<IDXGIDevice>();
        let mut dxgi_adapter = ptr::null_mut::<IDXGIAdapter>();
        let mut dxgi_factory = ptr::null_mut::<IDXGIFactory>();

        if device.as_ref().QueryInterface(
            &IDXGIDevice::uuidof(),
            &mut dxgi_device as *mut *mut _ as *mut *mut _,
        ) == S_OK
        {
            if (&*dxgi_device).GetParent(
                &IDXGIAdapter::uuidof(),
                &mut dxgi_adapter as *mut *mut _ as *mut *mut _,
            ) == S_OK
            {
                if (&*dxgi_adapter).GetParent(
                    &IDXGIFactory::uuidof(),
                    &mut dxgi_factory as *mut *mut _ as *mut *mut _,
                ) == S_OK
                {
                    return NonNull::new(dxgi_factory).ok_or(RendererError::FactoryAquisition);
                }
                (&*dxgi_adapter).Release();
            }
            (&*dxgi_device).Release();
        }
        Err(RendererError::FactoryAquisition)
    }

    /// The textures registry of this renderer.
    ///
    /// The texture slot at !0 is reserved for the font texture, therefore the
    /// renderer will ignore any texture inserted into said slot.
    ///
    /// # Safety
    ///
    /// Mutable access is unsafe since the renderer assumes that the texture
    /// handles inside of it are valid until they are removed manually.
    /// Failure to keep this invariant in check will cause UB.
    #[inline]
    pub unsafe fn textures_mut(&mut self) -> &mut Textures<NonNull<ID3D11ShaderResourceView>> {
        &mut self.textures
    }

    /// The textures registry of this renderer.
    #[inline]
    pub fn textures(&mut self) -> &mut Textures<NonNull<ID3D11ShaderResourceView>> {
        &mut self.textures
    }

    /// Renders the given [`Ui`] with this renderer.
    ///
    /// [`Ui`]: https://docs.rs/imgui/*/imgui/struct.Ui.html
    pub fn render(&mut self, draw_data: &DrawData) -> Result<()> {
        if draw_data.display_size[0] <= 0.0 || draw_data.display_size[1] <= 0.0 {
            return Ok(());
        }

        unsafe {
            if self.vertex_buffer.len() < draw_data.total_vtx_count as usize {
                self.vertex_buffer =
                    Self::create_vertex_buffer(self.device, draw_data.total_vtx_count as usize)?;
            }
            if self.index_buffer.len() < draw_data.total_idx_count as usize {
                self.index_buffer =
                    Self::create_index_buffer(self.device, draw_data.total_idx_count as usize)?;
            }

            //let _state_backup = StateBackup::backup(self.device.as_ptr())?;

            self.write_buffers(draw_data)?;
            self.setup_render_state(draw_data);
            self.render_impl(draw_data)?;
        }
        Ok(())
    }

    unsafe fn render_impl(&mut self, draw_data: &DrawData) -> Result<()> {
        let clip_off = draw_data.display_pos;
        let clip_scale = draw_data.framebuffer_scale;
        let mut vertex_offset = 0;
        let mut index_offset = 0;
        let mut last_tex = TextureId::from(FONT_TEX_ID);
        (self.context.as_mut()).PSSetShaderResources(0, 1, &self.font_resource_view.as_ptr());
        for draw_list in draw_data.draw_lists() {
            for cmd in draw_list.commands() {
                match cmd {
                    DrawCmd::Elements {
                        count,
                        cmd_params:
                            DrawCmdParams {
                                clip_rect,
                                texture_id,
                                ..
                            },
                    } => {
                        if texture_id != last_tex {
                            let texture = if texture_id.id() == FONT_TEX_ID {
                                self.font_resource_view.as_ptr()
                            } else {
                                self.textures
                                    .get(texture_id)
                                    .ok_or(RendererError::InvalidTexture(texture_id))?
                                    .as_ptr()
                            };
                            (self.context.as_mut()).PSSetShaderResources(0, 1, &texture);
                            last_tex = texture_id;
                        }

                        let r = D3D11_RECT {
                            left: ((clip_rect[0] - clip_off[0]) * clip_scale[0]) as i32,
                            top: ((clip_rect[1] - clip_off[1]) * clip_scale[1]) as i32,
                            right: ((clip_rect[2] - clip_off[0]) * clip_scale[0]) as i32,
                            bottom: ((clip_rect[3] - clip_off[1]) * clip_scale[1]) as i32,
                        };
                        (self.context.as_mut()).RSSetScissorRects(1, &r);
                        (self.context.as_mut()).DrawIndexed(
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

    unsafe fn setup_render_state(&mut self, draw_data: &DrawData) {
        let context = self.context.as_ref();
        let vp = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: draw_data.display_size[0],
            Height: draw_data.display_size[1],
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        context.RSSetViewports(1, &vp);

        let stride = mem::size_of::<DrawVert>() as u32;
        context.IASetInputLayout(self.input_layout.as_ptr());
        context.IASetVertexBuffers(0, 1, &self.vertex_buffer.as_ptr(), &stride, &0);
        context.IASetIndexBuffer(
            self.index_buffer.as_ptr(),
            if mem::size_of::<DrawIdx>() == 2 {
                DXGI_FORMAT_R16_UINT
            } else {
                DXGI_FORMAT_R32_UINT
            },
            0,
        );
        context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        context.VSSetShader(self.vertex_shader.as_ptr(), ptr::null(), 0);
        context.VSSetConstantBuffers(0, 1, &self.constant_buffer.as_ptr());
        context.PSSetShader(self.pixel_shader.as_ptr(), ptr::null(), 0);
        context.PSSetSamplers(0, 1, &self.font_sampler.as_ptr());
        context.GSSetShader(ptr::null_mut(), ptr::null(), 0);
        context.HSSetShader(ptr::null_mut(), ptr::null(), 0);
        context.DSSetShader(ptr::null_mut(), ptr::null(), 0);
        context.CSSetShader(ptr::null_mut(), ptr::null(), 0);

        let blend_factor = [0.0; 4];
        context.OMSetBlendState(self.blend_state.as_ptr(), &blend_factor, 0xFFFFFFFF);
        context.OMSetDepthStencilState(self.depth_stencil_state.as_ptr(), 0);
        context.RSSetState(self.rasterizer_state.as_ptr());
    }

    unsafe fn create_vertex_buffer(
        device: NonNull<ID3D11Device>,
        vtx_count: usize,
    ) -> Result<Buffer> {
        let mut vertex_buffer = ptr::null_mut();
        let len = vtx_count + VERTEX_BUF_ADD_CAPACITY;
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: (len * mem::size_of::<DrawVert>()) as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_VERTEX_BUFFER,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        if device
            .as_ref()
            .CreateBuffer(&desc, ptr::null(), &mut vertex_buffer)
            < 0
        {
            Err(RendererError::OutOfMemory)
        } else {
            NonNull::new(vertex_buffer)
                .map(|ptr| Buffer(ptr, len))
                .ok_or(RendererError::OutOfMemory)
        }
    }

    unsafe fn create_index_buffer(
        device: NonNull<ID3D11Device>,
        idx_count: usize,
    ) -> Result<Buffer> {
        let mut index_buffer = ptr::null_mut();
        let len = idx_count + INDEX_BUF_ADD_CAPACITY;
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: (len * mem::size_of::<DrawIdx>()) as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_INDEX_BUFFER,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        if device
            .as_ref()
            .CreateBuffer(&desc, ptr::null(), &mut index_buffer)
            < 0
        {
            Err(RendererError::OutOfMemory)
        } else {
            NonNull::new(index_buffer)
                .map(|ptr| Buffer(ptr, len))
                .ok_or(RendererError::OutOfMemory)
        }
    }

    unsafe fn write_buffers(&mut self, draw_data: &DrawData) -> Result<()> {
        let mut vtx_resource = mem::zeroed();
        let mut idx_resource = mem::zeroed();
        if self.context.as_ref().Map(
            self.vertex_buffer.as_ptr().cast(),
            0,
            D3D11_MAP_WRITE_DISCARD,
            0,
            &mut vtx_resource,
        ) != S_OK
        {
            panic!();
        }
        if self.context.as_ref().Map(
            self.index_buffer.as_ptr().cast(),
            0,
            D3D11_MAP_WRITE_DISCARD,
            0,
            &mut idx_resource,
        ) != S_OK
        {
            panic!();
        }

        let mut vtx_dst = slice::from_raw_parts_mut(
            vtx_resource.pData.cast::<DrawVert>(),
            draw_data.total_vtx_count as usize,
        );
        let mut idx_dst = slice::from_raw_parts_mut(
            idx_resource.pData.cast::<DrawIdx>(),
            draw_data.total_idx_count as usize,
        );
        for draw_list in draw_data.draw_lists() {
            for (&vertex, vtx_dst) in draw_list.vtx_buffer().iter().zip(vtx_dst.iter_mut()) {
                *vtx_dst = vertex;
            }
            idx_dst[..draw_list.idx_buffer().len()].copy_from_slice(draw_list.idx_buffer());
            vtx_dst = &mut vtx_dst[draw_list.vtx_buffer().len()..];
            idx_dst = &mut idx_dst[draw_list.idx_buffer().len()..];
        }

        self.context
            .as_ref()
            .Unmap(self.vertex_buffer.as_ptr().cast(), 0);
        self.context
            .as_ref()
            .Unmap(self.index_buffer.as_ptr().cast(), 0);

        // constant buffer
        let mut mapped_resource = mem::zeroed();
        if self.context.as_ref().Map(
            self.constant_buffer.as_ptr().cast(),
            0,
            D3D11_MAP_WRITE_DISCARD,
            0,
            &mut mapped_resource,
        ) != S_OK
        {
            panic!()
        }

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
        self.context
            .as_ref()
            .Unmap(self.constant_buffer.as_ptr().cast(), 0);
        Ok(())
    }

    unsafe fn create_font_texture(
        mut fonts: imgui::FontAtlasRefMut<'_>,
        device: NonNull<ID3D11Device>,
    ) -> Result<(
        NonNull<ID3D11ShaderResourceView>,
        NonNull<ID3D11SamplerState>,
    )> {
        let fa_tex = fonts.build_rgba32_texture();

        let desc = D3D11_TEXTURE2D_DESC {
            Width: fa_tex.width,
            Height: fa_tex.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = ptr::null_mut();
        let sub_resource = D3D11_SUBRESOURCE_DATA {
            pSysMem: fa_tex.data.as_ptr().cast(),
            SysMemPitch: desc.Width * 4,
            SysMemSlicePitch: 0,
        };
        device
            .as_ref()
            .CreateTexture2D(&desc, &sub_resource, &mut texture);

        let mut desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            ViewDimension: D3D11_SRV_DIMENSION_TEXTURE2D,
            u: mem::zeroed(),
        };
        *desc.u.Texture2D_mut() = D3D11_TEX2D_SRV {
            MostDetailedMip: 0,
            MipLevels: 1,
        };
        let mut font_texture_view = ptr::null_mut();
        device
            .as_ref()
            .CreateShaderResourceView(texture.cast(), &desc, &mut font_texture_view);
        (&*texture).Release();

        fonts.tex_id = TextureId::from(FONT_TEX_ID);

        let desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D11_TEXTURE_ADDRESS_WRAP,
            AddressV: D3D11_TEXTURE_ADDRESS_WRAP,
            AddressW: D3D11_TEXTURE_ADDRESS_WRAP,
            MipLODBias: 0.0,
            MaxAnisotropy: 0,
            ComparisonFunc: D3D11_COMPARISON_ALWAYS,
            BorderColor: [0.0; 4],
            MinLOD: 0.0,
            MaxLOD: 0.0,
        };
        let mut font_sampler = ptr::null_mut();
        device.as_ref().CreateSamplerState(&desc, &mut font_sampler);
        Ok((
            NonNull::new_unchecked(font_texture_view),
            NonNull::new_unchecked(font_sampler),
        ))
    }

    unsafe fn create_vertex_shader(
        device: NonNull<ID3D11Device>,
    ) -> Result<(
        NonNull<ID3D11VertexShader>,
        NonNull<ID3D11InputLayout>,
        NonNull<ID3D11Buffer>,
    )> {
        static VERTEX_SHADER: &str = include_str!("vertex_shader.vs_4_0");
        let mut vs_blob = ptr::null_mut::<ID3DBlob>();
        if D3DCompile(
            VERTEX_SHADER.as_ptr().cast(),
            VERTEX_SHADER.len(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            "main\0".as_ptr().cast(),
            "vs_4_0\0".as_ptr().cast(),
            0,
            0,
            &mut vs_blob,
            ptr::null_mut(),
        ) != S_OK
        {
            (&*vs_blob).Release();
            panic!();
        }
        let mut vs_shader = ptr::null_mut();
        if device.as_ref().CreateVertexShader(
            (&*vs_blob).GetBufferPointer(),
            (&*vs_blob).GetBufferSize(),
            ptr::null_mut(),
            &mut vs_shader,
        ) != S_OK
        {
            (&*vs_blob).Release();
            panic!()
        }

        let local_layout = [
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: "POSITION\0".as_ptr().cast(),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 0,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: "TEXCOORD\0".as_ptr().cast(),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 8,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: "COLOR\0".as_ptr().cast(),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                InputSlot: 0,
                AlignedByteOffset: 16,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
        ];

        let mut input_layout = ptr::null_mut();
        if device.as_ref().CreateInputLayout(
            local_layout.as_ptr(),
            local_layout.len() as _,
            (&*vs_blob).GetBufferPointer(),
            (&*vs_blob).GetBufferSize(),
            &mut input_layout,
        ) != S_OK
        {
            (&*vs_blob).Release();
            panic!();
        }
        (&*vs_blob).Release();

        let desc = D3D11_BUFFER_DESC {
            ByteWidth: mem::size_of::<VertexConstantBuffer>() as _,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let mut vertex_constant_buffer = ptr::null_mut();
        device
            .as_ref()
            .CreateBuffer(&desc, ptr::null_mut(), &mut vertex_constant_buffer);
        Ok((
            NonNull::new_unchecked(vs_shader),
            NonNull::new_unchecked(input_layout),
            NonNull::new_unchecked(vertex_constant_buffer),
        ))
    }

    unsafe fn create_pixel_shader(
        device: NonNull<ID3D11Device>,
    ) -> Result<NonNull<ID3D11PixelShader>> {
        static PIXEL_SHADER: &str = include_str!("pixel_shader.ps_4_0");

        let mut ps_blob = ptr::null_mut::<ID3DBlob>();
        if D3DCompile(
            PIXEL_SHADER.as_ptr().cast(),
            PIXEL_SHADER.len(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            "main\0".as_ptr().cast(),
            "ps_4_0\0".as_ptr().cast(),
            0,
            0,
            &mut ps_blob,
            ptr::null_mut(),
        ) != S_OK
        {
            (&*ps_blob).Release();
            panic!();
        }
        let mut ps_shader = ptr::null_mut();
        if device.as_ref().CreatePixelShader(
            (&*ps_blob).GetBufferPointer(),
            (&*ps_blob).GetBufferSize(),
            ptr::null_mut(),
            &mut ps_shader,
        ) != S_OK
        {
            (&*ps_blob).Release();
            panic!()
        }
        (&*ps_blob).Release();
        Ok(NonNull::new_unchecked(ps_shader))
    }

    unsafe fn create_device_objects(
        device: NonNull<ID3D11Device>,
    ) -> (
        NonNull<ID3D11BlendState>,
        NonNull<ID3D11RasterizerState>,
        NonNull<ID3D11DepthStencilState>,
    ) {
        let mut desc = D3D11_BLEND_DESC {
            AlphaToCoverageEnable: TRUE,
            IndependentBlendEnable: FALSE,
            RenderTarget: std::mem::zeroed(),
        };
        desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: TRUE,
            SrcBlend: D3D11_BLEND_SRC_ALPHA,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
            DestBlendAlpha: D3D11_BLEND_ZERO,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL as u8,
        };
        let mut blend_state = ptr::null_mut();
        device.as_ref().CreateBlendState(&desc, &mut blend_state);

        let desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            FrontCounterClockwise: 0,
            DepthBias: 0,
            DepthBiasClamp: 0.0,
            SlopeScaledDepthBias: 0.0,
            DepthClipEnable: TRUE,
            ScissorEnable: TRUE,
            MultisampleEnable: 0,
            AntialiasedLineEnable: 0,
        };
        let mut rasterizer_state = ptr::null_mut();
        device
            .as_ref()
            .CreateRasterizerState(&desc, &mut rasterizer_state);

        let stencil_op_desc = D3D11_DEPTH_STENCILOP_DESC {
            StencilFailOp: D3D11_STENCIL_OP_KEEP,
            StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
            StencilPassOp: D3D11_STENCIL_OP_KEEP,
            StencilFunc: D3D11_COMPARISON_ALWAYS,
        };
        let desc = D3D11_DEPTH_STENCIL_DESC {
            DepthEnable: FALSE,
            DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL,
            DepthFunc: D3D11_COMPARISON_ALWAYS,
            StencilEnable: FALSE,
            StencilReadMask: 0,
            StencilWriteMask: 0,
            FrontFace: stencil_op_desc,
            BackFace: stencil_op_desc,
        };
        let mut depth_stencil_state = ptr::null_mut();
        device
            .as_ref()
            .CreateDepthStencilState(&desc, &mut depth_stencil_state);
        (
            NonNull::new_unchecked(blend_state),
            NonNull::new_unchecked(rasterizer_state),
            NonNull::new_unchecked(depth_stencil_state),
        )
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { self.factory.as_ref().Release() };
        unsafe { self.device.as_ref().Release() };
        unsafe { self.context.as_ref().Release() };
    }
}

#[derive(Debug)]
struct Buffer(NonNull<ID3D11Buffer>, usize);

impl Buffer {
    #[inline]
    fn len(&self) -> usize {
        self.1
    }

    #[inline]
    fn as_ptr(&mut self) -> *mut ID3D11Buffer {
        self.0.as_ptr()
    }
}

impl Drop for Buffer {
    #[inline]
    fn drop(&mut self) {
        unsafe { (*self.as_ptr()).Release() };
    }
}
