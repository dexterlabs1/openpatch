# MCP integration

OpenPatch exposes tools over newline-delimited JSON-RPC on stdio, with protocol
versions `2025-11-25`, `2025-06-18`, and `2025-03-26`. The server negotiates the
version during initialization. Stdout contains protocol messages only; errors
from startup go to stderr. It does not open a network port or call a model API.

Build `cargo build --release` first. Replace paths below with absolute paths on
your machine. On Windows use `openpatch.exe` and Windows paths.

## Codex

```sh
codex mcp add openpatch -- /absolute/path/openpatch mcp --workspace /absolute/path/patches
```

Or add a table to your Codex configuration:

```toml
[mcp_servers.openpatch]
command = "/absolute/path/openpatch"
args = ["mcp", "--workspace", "/absolute/path/patches"]
```

On Windows, TOML literal strings avoid backslash escapes:

```toml
[mcp_servers.openpatch]
command = 'C:\OpenPatch\openpatch.exe'
args = ['mcp', '--workspace', 'C:\OpenPatch\patches']
```

See [official Codex MCP configuration](https://developers.openai.com/codex/mcp/).

## Claude Code

```sh
claude mcp add --transport stdio openpatch -- /absolute/path/openpatch mcp --workspace /absolute/path/patches
```

See [official Claude Code MCP documentation](https://code.claude.com/docs/en/mcp).

## Claude Desktop

Merge this server into `claude_desktop_config.json`; preserve any existing servers:

```json
{
  "mcpServers": {
    "openpatch": {
      "command": "/absolute/path/openpatch",
      "args": ["mcp", "--workspace", "/absolute/path/patches"]
    }
  }
}
```

Use escaped backslashes in Windows JSON paths, e.g. `C:\\OpenPatch\\openpatch.exe`.
Client support and setup locations may vary by version. These examples configure
the same local stdio server, not a remote HTTP connector. No existing user client
configuration is changed by this repository.

## Building a patch

1. Call `catalog` to discover modules, port names, parameters, and limits.
2. Call `inspect_patch` for the patch and current revision.
3. Use `replace_patch` for an atomic graph change, or edit individual modules and cables.
4. Select the audible node with `set_output`. Without an output the patch is silent.
5. Use `render_audio` with a new `.wav` filename and `seconds` between 0.001 and 30.
6. Save the graph with `save_patch` and a new `.json` filename.

Include `expected_revision` on edits when coordinating multiple operations. A
stale revision or invalid edit leaves both the patch and revision unchanged.
Tool failures return `isError: true`; malformed protocol calls use JSON-RPC errors.

One server process means one session. Claude and Codex do not share live state
when they each start separate processes. Exchange saved JSON patches to move
between clients. A shared application service is a later milestone.

## Execution boundaries

Compilation, JSON parsing, file access, MCP dispatch, and rendering orchestration
run on the control side. The DSP processing function knows nothing about MCP.
Current rendering is offline and synchronous; there is no live audio callback to
block. Every render starts oscillator/filter state from reset. Future live audio
must use bounded commands, block-boundary updates, smoothing, and off-thread
reclamation of old graphs.

Requests and patch files are capped at 1 MiB. Patches are bounded to 256 nodes
and 1,024 cables. File arguments accept simple filenames, not paths; symlinks
are rejected and writes use exclusive creation. Use a workspace owned by you,
not a directory concurrently modified by untrusted users. This is a local tool,
not an OS sandbox or a hardened multi-user file service.
