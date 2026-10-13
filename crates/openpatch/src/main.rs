mod control;
mod mcp;
mod visualizer;

use control::Application;
use openpatch_core::{demo_patch, render, write_wav, Patch};
use std::{fs, path::Path};

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("mcp") => {
            let workspace = match args.as_slice() {
                [_, flag, path] if flag == "--workspace" => path,
                _ => return Err("usage: openpatch mcp --workspace DIRECTORY".into()),
            };
            mcp::serve(Application::new(Path::new(workspace))?)
        }
        Some("demo") if args.len() == 2 => {
            let patch = demo_patch();
            let samples = render(&patch, 3.0)?;
            write_wav(Path::new(&args[1]), &samples, patch.sample_rate)
                .map_err(|e| e.to_string())?;
            println!("Rendered three seconds to {}", args[1]);
            Ok(())
        }
        Some("visualizer") if args.len() == 2 => {
            visualizer::export(Path::new(&args[1])).map_err(|e| e.to_string())?;
            println!("Open {} in a browser and select a local audio file.", args[1]);
            Ok(())
        }
        Some("render") if args.len() == 4 => {
            let patch: Patch =
                serde_json::from_slice(&fs::read(&args[1]).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let seconds = args[3].parse().map_err(|_| "invalid seconds")?;
            let samples = render(&patch, seconds)?;
            write_wav(Path::new(&args[2]), &samples, patch.sample_rate)
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        Some("catalog") if args.len() == 1 => {
            println!(
                "{}",
                serde_json::to_string_pretty(&openpatch_core::catalog()).unwrap()
            );
            Ok(())
        }
        Some("--version") => {
            println!("OpenPatch {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        None | Some("--help") | Some("help") => {
            println!("OpenPatch — modular synthesis with native MCP\n\nCommands:\n  mcp --workspace DIRECTORY\n  demo OUTPUT.wav\n  render PATCH.json OUTPUT.wav SECONDS\n  visualizer OUTPUT.html\n  catalog\n  --version\n\nMCP uses stdio. Files are never overwritten. This prototype renders offline.");
            Ok(())
        }
        _ => Err("unknown command or arguments; run openpatch --help".into()),
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("openpatch: {e}");
        std::process::exit(1);
    }
}
