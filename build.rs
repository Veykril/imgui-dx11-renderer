use std::error::Error;
use std::{env, fs, slice, str};

use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::ID3DBlob;
use windows::s;

fn main() -> Result<(), Box<dyn Error + 'static>> {
    static VERTEX_SHADER: &str = include_str!("src/vertex_shader.vs_4_0");
    static PIXEL_SHADER: &str = include_str!("src/pixel_shader.ps_4_0");

    let mut err = None; // Never used, but left in-case inspection later is needed
    let mut vs_blob = None;
    let mut ps_blob = None;

    unsafe {
        D3DCompile(
            VERTEX_SHADER.as_ptr() as _,
            VERTEX_SHADER.len(),
            None,
            None,
            None,
            s!("main"),
            s!("vs_4_0"),
            0,
            0,
            &mut vs_blob,
            Some(&mut err),
        )?;
        if let Some(vs_blob) = vs_blob.as_ref() {
            write_blob("vertex_shader.vs_4_0", vs_blob)?;
        }

        D3DCompile(
            PIXEL_SHADER.as_ptr() as _,
            PIXEL_SHADER.len(),
            None,
            None,
            None,
            s!("main"),
            s!("ps_4_0"),
            0,
            0,
            &mut ps_blob,
            Some(&mut err),
        )?;
        if let Some(ps_blob) = ps_blob.as_ref() {
            write_blob("pixel_shader.ps_4_0", ps_blob)?;
        }
    }
    Ok(())
}

unsafe fn write_blob(shader_name: &str, blob: &ID3DBlob) -> Result<(), Box<dyn Error + 'static>> {
    let out_dir = env::var("OUT_DIR")?;
    let data = slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize());
    fs::write(&format!("{}/{}", out_dir, shader_name), data).map_err(Into::into)
}
