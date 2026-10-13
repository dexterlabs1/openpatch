# MCP integration

OpenPatch exposes tools over newline-delimited JSON-RPC on stdio, with protocol
versions `2025-11-25`, `2025-06-18`, and `2025-03-26`. The server negotiates the
version during initialization. Stdout contains protocol messages only; errors
from startup go to stderr. It does not open a network port or call a model API.

Build `cargo build --release --workspace` first. The examples below launch
`target/release/openpatch-desktop --mcp --workspace DIRECTORY`. Replace paths
with absolute paths on your machine. On Windows use `openpatch-desktop.exe`.

The MCP client launches a native window and controls that window's live patch.
GUI controls and MCP tools share the same session and revision counter. Playback
starts paused: press **Play** in the window (or Space when a text field is not
focused) to hear the patch. MCP can edit a playing patch, but has no tool to start
or stop speaker playback. `render_audio` exports a WAV independently of playback.

A window launched manually is a separate session. This stdio integration does
not attach to an already running window. If Claude and Codex each launch the
executable, each gets its own window and patch. Sharing a workspace only shares
saved files. When stdin reaches EOF, the MCP thread exits and the window remains
open; a client that terminates its child process can still close the application.

## Codex

```sh
codex mcp add openpatch -- /absolute/path/openpatch/target/release/openpatch-desktop --mcp --workspace /absolute/path/patches
```

Or add a table to your Codex configuration:

```toml
[mcp_servers.openpatch]
command = "/absolute/path/openpatch/target/release/openpatch-desktop"
args = ["--mcp", "--workspace", "/absolute/path/patches"]
```

On Windows, TOML literal strings avoid backslash escapes:

```toml
[mcp_servers.openpatch]
command = 'C:\OpenPatch\target\release\openpatch-desktop.exe'
args = ['--mcp', '--workspace', 'C:\OpenPatch\patches']
```

See [official Codex MCP configuration](https://developers.openai.com/codex/mcp/).

## Claude Code

```sh
claude mcp add --transport stdio openpatch -- /absolute/path/openpatch/target/release/openpatch-desktop --mcp --workspace /absolute/path/patches
```

See [official Claude Code MCP documentation](https://code.claude.com/docs/en/mcp).

## Claude Desktop

Merge this server into `claude_desktop_config.json`; preserve any existing servers:

```json
{
  "mcpServers": {
    "openpatch": {
      "command": "/absolute/path/openpatch/target/release/openpatch-desktop",
      "args": ["--mcp", "--workspace", "/absolute/path/patches"]
    }
  }
}
```

Use escaped backslashes in Windows JSON paths, e.g.
`C:\\OpenPatch\\target\\release\\openpatch-desktop.exe`.
Client support and setup locations may vary by version. These examples configure
the same local stdio server, not a remote HTTP connector. No existing user client
configuration is changed by this repository.

## Building a patch

1. Call `catalog` to discover modules, port names, parameters, and limits.
2. Call `inspect_patch` for the patch and current revision.
3. Use `replace_patch` for an atomic graph change, or edit individual modules and cables.
4. Select the audible node with `set_output`. Without an output the patch is silent.
5. Press Play locally to audition the live patch, or use `render_audio` with a new
   `.wav` filename and `seconds` between 0.001 and 30 to export it.
6. Save the graph with `save_patch` and a new `.json` filename.

Include `expected_revision` on edits when coordinating multiple operations. A
stale revision or invalid edit leaves both the patch and revision unchanged.
Tool failures return `isError: true`; malformed protocol calls use JSON-RPC errors.

The desktop starts with the demo patch. Call `inspect_patch` before editing;
include `expected_revision` to detect edits made in the window since your read.
Exchange saved JSON patches to move between separate client processes.

## Headless offline mode

The original CLI remains available without a display or audio device:

```sh
/absolute/path/openpatch/target/release/openpatch mcp --workspace /absolute/path/patches
```

For a headless client configuration, use `openpatch` as the command and
`["mcp", "--workspace", "/absolute/path/patches"]` as its arguments. It starts
with an empty patch, renders offline, and has no native window or speaker
playback. It does not share state with desktop processes.

## Execution boundaries

Compilation, JSON parsing, file access, MCP dispatch, and rendering orchestration
run on control threads. The native GUI observes committed revisions, compiles a
device-rate engine, and transfers it to the audio callback through bounded
queues. The callback never locks the session or dispatches MCP commands. Edits
crossfade to fresh engines; oscillator and filter state are reset by graph edits.

Offline rendering and benchmarks use a coherent patch snapshot and release the
session mutex before processing. MCP requests on that connection remain
sequential while the operation runs, but GUI edits and live audio can continue.
Every offline render starts oscillator/filter state from reset and does not
change the live patch or revision. The device sample rate can differ from the
stored patch sample rate used for WAV export.

Requests and patch files are capped at 1 MiB. Patches are bounded to 256 nodes
and 1,024 cables. File arguments accept simple filenames, not paths; symlinks
are rejected and writes use exclusive creation. Use a workspace owned by you,
not a directory concurrently modified by untrusted users. This is a local tool,
not an OS sandbox or a hardened multi-user file service.
