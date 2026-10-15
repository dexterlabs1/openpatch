# MCP session controls

Click **MCP** in the native toolbar to start or reuse this window's local MCP
bridge and open its connection panel. The panel shows whether the bridge is
ready, the number of attached transports, and the current session ID. A ready
bridge with zero clients is waiting for a client connection.

**Start bridge** and **Stop bridge** control the local listener. Stop pauses a
running construction plan before disconnecting attached clients. Committed edits
stay in the rack and remain undoable. It does not change playback or the patch.
The bridge has a cancellation fence for requests that were already parsed or
performing preflight when Stop took effect. A separately enabled stdio connection
remains independent.

The panel provides **Copy attach command**, **Copy Claude JSON** and
**Copy Codex TOML**. They contain the actual bundled `openpatch` executable and
this window's live session ID. Restarting the bridge creates a new ID, so copy
fresh connection details afterward. The panel does not edit client configuration,
launch another app, collect API keys or create an assistant conversation.

For Codex, the copied snippet uses a `[mcp_servers.openpatch]` table with
`command` and `args`, as described in the
[official MCP configuration documentation](https://learn.chatgpt.com/docs/extend/mcp?surface=cli).
Claude's copied JSON uses the local stdio `mcpServers` format described in the
[MCP local-server setup guide](https://modelcontextprotocol.io/docs/develop/connect-local-servers).
Add the copied configuration in your chosen client's MCP settings, then connect
to the running window. External clients control the same live patch.

The toolbar starts the bridge if it is stopped. Existing macOS/Linux startup
attachment remains available; opening an already-ready bridge is idempotent.
If startup fails, the panel retains the error and offers a retry. Copy controls
are disabled when the bundled attachment executable is unavailable. The footer
also opens connection details.

The local attachment bridge is supported on macOS and Linux. Windows retains
the existing client-launched stdio mode; the panel identifies that scope and
provides no nonfunctional local-listener control.

## Validation

Headless tests exercise real local socket startup, MCP handshake, disconnect,
restart, construction pause and busy-operation retry. They also check copied
configuration escaping, clipboard actions and focus. App tests keep rack gestures,
keyboard performance and global edit shortcuts out of the connection window.
The existing same-user socket ownership and discovery rules still apply; see
[MCP transport and tools](MCP.md).
