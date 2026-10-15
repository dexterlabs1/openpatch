#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod audio;
mod cable_animation;
mod cable_palette;
mod cable_palette_ui;
mod chrome;
mod construction_motion;
mod construction_ui;
mod document_state;
mod expression;
mod keyboard;
#[cfg(test)]
mod keyboard_acceptance_tests;
mod library;
mod lighting;
mod mcp_session_ui;
mod midi;
mod panel_skin;
mod project;
#[cfg(test)]
mod project_acceptance_tests;
mod rack;
mod scope;
mod scope_transport;
mod startup;
mod visuals;

use openpatch::control::SharedSession;
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
    let mut explicit_patch = None;
    let mut workspace = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("OpenPatch");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--third-party-licenses" => {
                println!(
                    "{}\n{}",
                    include_str!("../assets/licenses/THIRD_PARTY.txt"),
                    include_str!("../assets/licenses/triple_buffer-MPL-2.0.txt")
                );
                return Ok(());
            }
            "--font-license" => {
                println!("{}", include_str!("../assets/fonts/OFL-Barlow.txt"));
                return Ok(());
            }
            "--mcp" => mcp = true,
            "--patch" => {
                explicit_patch = Some(args.next().ok_or("--patch needs a project filename")?)
            }
            "--smoke-test" => {
                smoke_test = Some(PathBuf::from(
                    args.next().ok_or("--smoke-test needs a PNG path")?,
                ))
            }
            "--workspace" => {
                workspace = PathBuf::from(args.next().ok_or("--workspace needs a directory")?)
            }
            "--help" => {
                println!("OpenPatch native desktop\n  --mcp                  Serve MCP over stdio, controlling this window\n  --workspace DIRECTORY  Patch files (default ~/OpenPatch)\n  --patch FILENAME       Open a workspace project (.openpatch or legacy .json)\n  --font-license         Print the bundled panel font license\n  --third-party-licenses Print bundled dependency notices and licenses");
                return Ok(());
            }
            _ => return Err(format!("Unknown argument: {arg}")),
        }
    }
    let (session, startup) =
        startup::prepare(&workspace, explicit_patch.as_deref(), smoke_test.is_some())?;
    let shared: SharedSession = Arc::new(Mutex::new(session));
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let local_mcp = if smoke_test.is_none() {
        Some(openpatch::local_mcp::LocalServer::start(
            shared.clone(),
            &workspace,
        ))
    } else {
        None
    };
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
        Box::new(move |cc| {
            let app = app::OpenPatch::new(cc, shared, mcp, smoke_test, startup);
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            let app = {
                let mut app = app;
                if let Some(server) = local_mcp {
                    app.set_local_mcp(server);
                }
                app
            };
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| e.to_string())
}
