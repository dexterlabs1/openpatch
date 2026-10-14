//! CLI bridge: MCP clients keep using stdio while the rack owns the session.

pub fn run(args: &[String]) -> Result<(), String> {
    let (session, workspace) = selection(args)?;
    bridge(session, workspace)
}

fn selection(args: &[String]) -> Result<(Option<&str>, Option<&std::path::Path>), String> {
    match args {
        [] => Ok((None, None)),
        [flag, id] if flag == "--session" && !id.is_empty() => Ok((Some(id), None)),
        [flag, path] if flag == "--workspace" && !path.is_empty() => {
            Ok((None, Some(std::path::Path::new(path))))
        }
        _ => Err("usage: openpatch attach [--session ID | --workspace DIRECTORY]".into()),
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn list() -> Result<(), String> {
    let sessions = openpatch::local_mcp::sessions()?;
    let values: Vec<_> = sessions.iter().map(|session| session.to_json()).collect();
    println!("{}", serde_json::to_string_pretty(&values).unwrap());
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn bridge(session: Option<&str>, workspace: Option<&std::path::Path>) -> Result<(), String> {
    use std::io::{self, Read, Write};
    use std::net::Shutdown;
    use std::time::Duration;

    let mut socket = openpatch::local_mcp::connect(session, workspace)?;
    socket
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| e.to_string())?;
    let mut writer = socket.try_clone().map_err(|e| e.to_string())?;
    // The main thread drains responses. A server disconnect must exit this
    // process even when the MCP client's stdin remains open. Do not join the
    // input thread: it may be blocked on stdin, which has no portable timeout.
    std::thread::Builder::new()
        .name("openpatch-attach-input".into())
        .spawn(move || {
            let result = io::copy(&mut io::stdin().lock(), &mut writer);
            let how = if result.is_ok() {
                Shutdown::Write
            } else {
                Shutdown::Both
            };
            let _ = writer.shutdown(how);
            if let Err(error) = result {
                eprintln!("openpatch attach input: {error}");
            }
        })
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let mut stdout = io::stdout().lock();
        let mut bytes = [0_u8; 8192];
        loop {
            let count = socket.read(&mut bytes)?;
            if count == 0 {
                return Ok::<(), io::Error>(());
            }
            stdout.write_all(&bytes[..count])?;
            stdout.flush()?;
        }
    })();
    let _ = socket.shutdown(Shutdown::Both);
    result.map_err(|e| e.to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn list() -> Result<(), String> {
    Err("Local rack attachment is currently supported on macOS and Linux. Use openpatch-desktop --mcp for a live stdio session on this platform.".into())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn bridge(_session: Option<&str>, _workspace: Option<&std::path::Path>) -> Result<(), String> {
    list()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_never_guesses_between_explicit_selectors() {
        assert_eq!(selection(&[]).unwrap(), (None, None));
        assert_eq!(
            selection(&["--session".into(), "abc".into()]).unwrap(),
            (Some("abc"), None)
        );
        assert!(selection(&["--session".into()]).is_err());
        assert!(selection(&["--session".into(), "".into()]).is_err());
        assert!(selection(&[
            "--session".into(),
            "abc".into(),
            "--workspace".into(),
            "x".into()
        ])
        .is_err());
    }
}
