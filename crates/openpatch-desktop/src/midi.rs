//! UI-thread MIDI port discovery and connection lifetime. No audio devices are
//! opened here. Call `poll` periodically to detect unplugged hardware.
use crate::audio::{HardwareMidiSender, MidiReset};
use midir::{Ignore, MidiInput, MidiInputConnection};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiPortInfo {
    /// Backend port identity, rather than a potentially duplicated display name.
    /// Some backends may change this identity after unplugging/restarting.
    pub id: String,
    pub name: String,
}

// midir returns its MidiInput but not callback data on connection failure.
// Recover the sole producer when callback storage is destroyed (also on failed
// connect). This mutex is used ONLY at connection creation/destruction; message
// delivery and the audio callback never acquire it.
struct RecoverSender {
    sender: Option<HardwareMidiSender>,
    returned: Arc<Mutex<Option<HardwareMidiSender>>>,
}
impl Drop for RecoverSender {
    fn drop(&mut self) {
        *self.returned.lock().unwrap_or_else(|e| e.into_inner()) = self.sender.take();
    }
}

pub struct MidiInputManager {
    available: Arc<Mutex<Option<HardwareMidiSender>>>,
    connection: Option<MidiInputConnection<RecoverSender>>,
    connected: Option<String>,
    reset: MidiReset,
    last_poll: Instant,
}
impl MidiInputManager {
    pub fn new(sender: HardwareMidiSender) -> Self {
        let reset = sender.reset_handle();
        Self {
            available: Arc::new(Mutex::new(Some(sender))),
            connection: None,
            connected: None,
            reset,
            last_poll: Instant::now(),
        }
    }
    pub fn refresh(&mut self) -> Result<Vec<MidiPortInfo>, String> {
        let input = MidiInput::new("OpenPatch port discovery").map_err(|e| e.to_string())?;
        let ports: Vec<_> = input
            .ports()
            .iter()
            .filter_map(|port| {
                input.port_name(port).ok().map(|name| MidiPortInfo {
                    id: port.id(),
                    name,
                })
            })
            .collect();
        if self
            .connected
            .as_ref()
            .is_some_and(|id| !ports.iter().any(|p| &p.id == id))
        {
            self.disconnect();
        }
        Ok(ports)
    }
    pub fn connect(&mut self, id: &str) -> Result<(), String> {
        self.disconnect();
        let mut input = MidiInput::new("OpenPatch MIDI").map_err(|e| e.to_string())?;
        input.ignore(Ignore::None); // Include timing clock and transport.
        let port = input
            .find_port_by_id(id)
            .ok_or("The MIDI input is no longer available")?;
        let sender = self
            .available
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or("MIDI producer is unavailable")?;
        let data = RecoverSender {
            sender: Some(sender),
            returned: self.available.clone(),
        };
        let connection = input
            .connect(
                &port,
                "OpenPatch input",
                |timestamp, bytes, data| {
                    if let Some(sender) = data.sender.as_mut() {
                        sender.send_bytes(timestamp, bytes);
                    }
                },
                data,
            )
            .map_err(|e| format!("Could not connect MIDI input: {e}"))?;
        self.connection = Some(connection);
        self.connected = Some(id.to_owned());
        self.last_poll = Instant::now();
        Ok(())
    }
    pub fn disconnect(&mut self) {
        // Close first: wait for in-flight callbacks, recover producer, then reset
        // both queues so callbacks cannot publish new notes after the reset.
        if let Some(connection) = self.connection.take() {
            let (_, sender) = connection.close();
            drop(sender);
        }
        self.connected = None;
        self.reset.panic();
    }
    pub fn connected_id(&self) -> Option<&str> {
        self.connected.as_deref()
    }
    /// Lightweight on most UI frames; enumeration is at most once per second.
    /// Disappearance resets notes and reports an error without auto-reconnecting.
    pub fn poll(&mut self) -> Option<String> {
        if self.connected.is_none() || self.last_poll.elapsed() < Duration::from_secs(1) {
            return None;
        }
        self.last_poll = Instant::now();
        match self.refresh() {
            Ok(_) if self.connected.is_none() => {
                Some("MIDI input disconnected; held notes were cleared.".into())
            }
            Ok(_) => None,
            Err(error) => {
                self.disconnect();
                Some(error)
            }
        }
    }
}
impl Drop for MidiInputManager {
    fn drop(&mut self) {
        self.disconnect();
    }
}
