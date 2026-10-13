use openpatch::{control::Application, mcp, visualizer};
use openpatch_core::{
    demo_patch, render, render_audio_with_events, write_wav, write_wav_interleaved, Patch,
    TimedMidiEvent,
};
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
            println!(
                "Open {} in a browser and select a local audio file.",
                args[1]
            );
            Ok(())
        }
        Some("render") if args.len() == 4 || (args.len() == 6 && args[4] == "--midi") => {
            let patch: Patch =
                serde_json::from_slice(&fs::read(&args[1]).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let seconds = args[3].parse().map_err(|_| "invalid seconds")?;
            let events: Vec<TimedMidiEvent> = if args.len() == 6 {
                serde_json::from_slice(&fs::read(&args[5]).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("invalid MIDI trace: {e}"))?
            } else {
                Vec::new()
            };
            let rendered = render_audio_with_events(&patch, seconds, &events)?;
            write_wav_interleaved(
                Path::new(&args[2]),
                &rendered.samples,
                rendered.sample_rate,
                rendered.channels,
            )
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
            println!("OpenPatch — modular synthesis with native MCP\n\nCommands:\n  mcp --workspace DIRECTORY\n  demo OUTPUT.wav\n  render PATCH.json OUTPUT.wav SECONDS [--midi EVENTS.json]\n  visualizer OUTPUT.html\n  catalog\n  --version\n\nMCP uses stdio. Files are never overwritten. Rendering is offline; audio_stereo exports L/R, other outputs export mono.");
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
