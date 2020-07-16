use imgui::{im_str, FontConfig, FontSource};
use imgui_winit_support::{HiDpiMode, WinitPlatform};
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

use winapi::shared::minwindef::TRUE;
use winapi::shared::windef::HWND;
use winapi::shared::winerror::S_OK;
use winapi::Interface as _;

use winapi::shared::dxgi::*;
use winapi::shared::dxgiformat::*;
use winapi::shared::dxgitype::*;

use winapi::um::d3d11::*;
use winapi::um::d3dcommon::*;

use winit::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::EventLoop,
    window::WindowBuilder,
};

use wio::com::ComPtr;

use std::ptr;
use std::time::Instant;

const WINDOW_WIDTH: f64 = 760.0;
const WINDOW_HEIGHT: f64 = 760.0;

unsafe fn create_device(
    hwnd: HWND,
) -> Option<(
    ComPtr<IDXGISwapChain>,
    ComPtr<ID3D11Device>,
    ComPtr<ID3D11DeviceContext>,
)> {
    let sc_desc = DXGI_SWAP_CHAIN_DESC {
        BufferDesc: DXGI_MODE_DESC {
            Width: 0,
            Height: 0,
            RefreshRate: DXGI_RATIONAL {
                Numerator: 60,
                Denominator: 1,
            },
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            ScanlineOrdering: 0,
            Scaling: 0,
        },
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        OutputWindow: hwnd,
        Windowed: TRUE,
        SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
        Flags: DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH,
    };

    let mut swapchain = ptr::null_mut();
    let mut device = ptr::null_mut();
    let mut context = ptr::null_mut();

    let mut feature_level = 0;
    let feature_levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_10_0];
    if D3D11CreateDeviceAndSwapChain(
        ptr::null_mut(),
        D3D_DRIVER_TYPE_HARDWARE,
        ptr::null_mut(),
        0,
        feature_levels.as_ptr(),
        feature_levels.len() as u32,
        D3D11_SDK_VERSION,
        &sc_desc,
        &mut swapchain,
        &mut device,
        &mut feature_level,
        &mut context,
    ) != S_OK
    {
        None
    } else {
        Some((
            ComPtr::from_raw(swapchain),
            ComPtr::from_raw(device),
            ComPtr::from_raw(context),
        ))
    }
}

unsafe fn create_render_target(
    swapchain: &ComPtr<IDXGISwapChain>,
    device: &ComPtr<ID3D11Device>,
) -> ComPtr<ID3D11RenderTargetView> {
    let mut back_buffer = ptr::null_mut::<ID3D11Texture2D>();
    let mut main_rtv = ptr::null_mut();
    swapchain.GetBuffer(
        0,
        &ID3D11Resource::uuidof(),
        &mut back_buffer as *mut *mut _ as *mut *mut _,
    );
    device.CreateRenderTargetView(back_buffer.cast(), ptr::null_mut(), &mut main_rtv);
    (&*back_buffer).Release();
    ComPtr::from_raw(main_rtv)
}

fn main() {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("imgui_dx11_renderer winit example")
        .with_inner_size(LogicalSize {
            width: WINDOW_WIDTH,
            height: WINDOW_HEIGHT,
        })
        .build(&event_loop)
        .unwrap();
    let hwnd = if let RawWindowHandle::Windows(handle) = window.raw_window_handle() {
        handle.hwnd
    } else {
        unreachable!()
    };
    let (swapchain, device, context) = unsafe { create_device(hwnd.cast()) }.unwrap();
    let mut main_rtv = unsafe { create_render_target(&swapchain, &device) };
    let mut imgui = imgui::Context::create();

    let mut renderer =
        imgui_dx11_renderer::Renderer::new(&mut imgui, device.clone(), context.clone()).unwrap();

    let mut platform = WinitPlatform::init(&mut imgui);
    platform.attach_window(imgui.io_mut(), &window, HiDpiMode::Rounded);

    let hidpi_factor = platform.hidpi_factor();
    let font_size = (13.0 * hidpi_factor) as f32;
    imgui.fonts().add_font(&[FontSource::DefaultFontData {
        config: Some(FontConfig {
            size_pixels: font_size,
            ..FontConfig::default()
        }),
    }]);

    imgui.io_mut().font_global_scale = (1.0 / hidpi_factor) as f32;

    imgui.set_ini_filename(None);
    let clear_color = [0.45, 0.55, 0.60, 1.00];

    let mut last_frame = Instant::now();

    event_loop.run(move |event, _, control_flow| match event {
        Event::NewEvents(_) => last_frame = imgui.io_mut().update_delta_time(last_frame),
        Event::MainEventsCleared => {
            let io = imgui.io_mut();
            platform
                .prepare_frame(io, &window)
                .expect("Failed to start frame");
            last_frame = io.update_delta_time(last_frame);
            window.request_redraw();
        },
        Event::RedrawRequested(_) => {
            unsafe {
                context.OMSetRenderTargets(1, &main_rtv.as_raw(), ptr::null_mut());
                context.ClearRenderTargetView(main_rtv.as_raw(), &clear_color);
            }
            let ui = imgui.frame();
            imgui::Window::new(im_str!("Hello world"))
                .size([300.0, 100.0], imgui::Condition::FirstUseEver)
                .build(&ui, || {
                    ui.text(im_str!("Hello world!"));
                    ui.text(im_str!("This...is...imgui-rs!"));
                    ui.separator();
                    let mouse_pos = ui.io().mouse_pos;
                    ui.text(im_str!(
                        "Mouse Position: ({:.1},{:.1})",
                        mouse_pos[0],
                        mouse_pos[1]
                    ));
                });
            ui.show_demo_window(&mut true);
            platform.prepare_render(&ui, &window);
            renderer.render(ui.render()).unwrap();
            unsafe {
                swapchain.Present(1, 0);
            }
        },
        Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } => *control_flow = winit::event_loop::ControlFlow::Exit,
        Event::WindowEvent {
            event: WindowEvent::Resized(winit::dpi::PhysicalSize { height, width }),
            ..
        } => unsafe {
            swapchain.ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, 0);
            main_rtv = create_render_target(&swapchain, &device);
        },
        Event::LoopDestroyed => (),
        event => {
            platform.handle_event(imgui.io_mut(), &window, &event);
        },
    });
}
