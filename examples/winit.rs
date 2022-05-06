use std::mem::transmute;
use std::time::Instant;

use imgui::{Context, FontConfig, FontSource};
use imgui_winit_support::{HiDpiMode, WinitPlatform};
use windows::core::Interface;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use winit::dpi::LogicalSize;
use winit::event::{Event, WindowEvent};
use winit::event_loop::EventLoop;
use winit::platform::windows::*;
use winit::window::WindowBuilder;

use imgui_dx11_renderer::Renderer;

const WINDOW_WIDTH: f64 = 760.0;
const WINDOW_HEIGHT: f64 = 760.0;

type Result<T> = std::result::Result<T, windows::core::Error>;

fn create_device_with_type(drive_type: D3D_DRIVER_TYPE) -> Result<ID3D11Device> {
    let mut flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;

    if cfg!(debug_assertions) {
        flags |= D3D11_CREATE_DEVICE_DEBUG;
    }

    let mut device = None;
    let feature_levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_10_0];
    let mut fl = D3D_FEATURE_LEVEL_11_1;
    unsafe {
        D3D11CreateDevice(
            None,
            drive_type,
            HINSTANCE::default(),
            flags,
            &feature_levels,
            D3D11_SDK_VERSION,
            &mut device,
            &mut fl,
            &mut None,
        )
        .map(|()| device.unwrap())
    }
}

fn create_device() -> Result<ID3D11Device> {
    create_device_with_type(D3D_DRIVER_TYPE_HARDWARE)
}

fn create_swapchain(device: &ID3D11Device, window: HWND) -> Result<IDXGISwapChain> {
    let factory = get_dxgi_factory(device)?;

    let sc_desc = DXGI_SWAP_CHAIN_DESC {
        BufferDesc: DXGI_MODE_DESC {
            Width: 0,
            Height: 0,
            RefreshRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            ..Default::default()
        },
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 3,
        OutputWindow: window,
        Windowed: true.into(),
        SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
        Flags: DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH.0 as u32,
    };

    unsafe { factory.CreateSwapChain(device, &sc_desc) }
}

fn get_dxgi_factory(device: &ID3D11Device) -> Result<IDXGIFactory2> {
    let dxdevice = device.cast::<IDXGIDevice>()?;
    unsafe { dxdevice.GetAdapter()?.GetParent() }
}

fn create_render_target(
    swapchain: &IDXGISwapChain,
    device: &ID3D11Device,
) -> Result<ID3D11RenderTargetView> {
    unsafe {
        let backbuffer: ID3D11Resource = swapchain.GetBuffer(0)?;
        device.CreateRenderTargetView(&backbuffer, 0 as _)
    }
}

fn main() -> Result<()> {
    let event_loop = EventLoop::new();
    let mut device_ctx = None;
    let window = WindowBuilder::new()
        .with_title("imgui_dx11_renderer winit example")
        .with_inner_size(LogicalSize { width: WINDOW_WIDTH, height: WINDOW_HEIGHT })
        .build(&event_loop)
        .unwrap();

    let device = create_device()?;
    let swapchain = unsafe { create_swapchain(&device, transmute(window.hwnd()))? };
    unsafe {
        device.GetImmediateContext(&mut device_ctx);
    }
    let mut target = Some(create_render_target(&swapchain, &device)?);

    let mut imgui = Context::create();
    let mut platform = WinitPlatform::init(&mut imgui);
    imgui.set_ini_filename(None);
    platform.attach_window(imgui.io_mut(), &window, HiDpiMode::Rounded);

    let hidpi_factor = platform.hidpi_factor();
    let font_size = (13.0 * hidpi_factor) as f32;
    imgui.fonts().add_font(&[FontSource::DefaultFontData {
        config: Some(FontConfig { size_pixels: font_size, ..FontConfig::default() }),
    }]);

    let mut renderer = unsafe { Renderer::new(&mut imgui, &device)? };
    let mut last_frame = Instant::now();

    event_loop.run(move |event, _, control_flow| match event {
        Event::NewEvents(_) => {
            let now = Instant::now();
            imgui.io_mut().update_delta_time(now - last_frame);
            last_frame = now;
        },
        Event::MainEventsCleared => {
            let io = imgui.io_mut();
            platform.prepare_frame(io, &window).expect("Failed to start frame");
            window.request_redraw();
        },
        Event::RedrawRequested(_) => {
            unsafe {
                if let Some(ref context) = device_ctx {
                    context.OMSetRenderTargets(&[target.clone()], None);
                    context.ClearRenderTargetView(target.as_ref().unwrap(), &0.6);
                }
            }
            let ui = imgui.frame();
            imgui::Window::new("Hello world")
                .size([300.0, 100.0], imgui::Condition::FirstUseEver)
                .build(&ui, || {
                    ui.text("Hello world!");
                    ui.text("This...is...imgui-rs!");
                    ui.separator();
                    let mouse_pos = ui.io().mouse_pos;
                    ui.text(format!("Mouse Position: ({:.1},{:.1})", mouse_pos[0], mouse_pos[1]));
                });
            ui.show_demo_window(&mut true);

            platform.prepare_render(&ui, &window);
            renderer.render(ui.render()).unwrap();
            unsafe {
                swapchain.Present(1, 0).unwrap();
            }
        },
        Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
            *control_flow = winit::event_loop::ControlFlow::Exit
        },
        Event::WindowEvent {
            event: WindowEvent::Resized(winit::dpi::PhysicalSize { height, width }),
            ..
        } => {
            target = None;
            unsafe {
                swapchain.ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, 0).unwrap();
            }
            let rtv = create_render_target(&swapchain, &device).unwrap();
            target = Some(rtv);
            platform.handle_event(imgui.io_mut(), &window, &event);
        },
        Event::LoopDestroyed => (),
        event => platform.handle_event(imgui.io_mut(), &window, &event),
    })
}
