# imgui-dx11-renderer

[![Documentation](https://docs.rs/imgui-dx11-renderer/badge.svg)](https://docs.rs/imgui-dx11-renderer)
[![Version](https://img.shields.io/crates/v/imgui-dx11-renderer.svg)](https://crates.io/crates/imgui-dx11-renderer)

DirectX 11 renderer for [imgui-rs](https://github.com/Gekkio/imgui-rs).

## Usage

```rust
let device: ID3D11Device = ...;
let imgui: imgui::Context = ...;
let mut renderer = imgui_dx11_renderer::Renderer::new(&mut imgui, &device).expect("imgui dx11 renderer creation failed");

// rendering loop

let ui = imgui.frame();

// build your window via ui here
...

// then to render call
renderer.render(ui.render()).expect("imgui rendering failed");
```

The renderer backs up and reapplies the majority of the d3d11 rendering state when invoked.

## Documentation

The crate is documented but imgui-rs doesn't currently build on docs.rs
for the windows target. Due to this one has to either build it
themselves or look into the source itself.

## License

Licensed under the MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
