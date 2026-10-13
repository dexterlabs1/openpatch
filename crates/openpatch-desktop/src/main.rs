#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod audio;
mod chrome;
mod document_state;
mod keyboard;
mod library;
mod midi;
mod project;
#[cfg(test)]
mod project_acceptance_tests;
mod rack;
mod scope;
mod visuals;

use openpatch::control::{Session, SharedSession};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

fn main() {
    if let Err(error) = run() {
        eprintln!("OpenPatch: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let mut mcp = false;
    let mut smoke_test = None;
    let mut workspace = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("OpenPatch");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mcp" => mcp = true,
            "--smoke-test" => {
                smoke_test = Some(PathBuf::from(
                    args.next().ok_or("--smoke-test needs a PNG path")?,
                ))
            }
            "--workspace" => {
                workspace = PathBuf::from(args.next().ok_or("--workspace needs a directory")?)
            }
            "--help" => {
                println!("OpenPatch native desktop\n  --mcp                  Serve MCP over stdio, controlling this window\n  --workspace DIRECTORY  Patch files (default ~/OpenPatch)");
                return Ok(());
            }
            _ => return Err(format!("Unknown argument: {arg}")),
        }
    }
    let mut session = Session::new(&workspace)?;
    let mut initial_patch = openpatch_core::analog_demo_patch();
    initial_patch.version = 2;
    session.replace_patch(initial_patch, None)?;
    let shared: SharedSession = Arc::new(Mutex::new(session));
    if mcp {
        let shared = shared.clone();
        std::thread::Builder::new()
            .name("openpatch-mcp".into())
            .spawn(move || {
                if let Err(err) = openpatch::mcp::serve_shared(shared) {
                    eprintln!("MCP: {err}");
                }
            })
            .map_err(|e| e.to_string())?;
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/openpatch.png"))
                    .expect("bundled OpenPatch icon must be valid PNG"),
            )
            .with_inner_size([1320.0, 880.0])
            .with_min_inner_size([940.0, 680.0])
            .with_app_id("org.openpatch.desktop"),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "OpenPatch",
        options,
        Box::new(move |cc| Ok(Box::new(app::OpenPatch::new(cc, shared, mcp, smoke_test)))),
    )
    .map_err(|e| e.to_string())
}
