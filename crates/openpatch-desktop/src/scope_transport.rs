//! Bounded SPSC scope publication. Only readers lock or allocate after setup.
//! Producer endpoints must be retired with their graph off the audio thread.
use openpatch_core::ScopeSnapshot;
use std::sync::{Arc, Mutex};
use triple_buffer::{Input, Output};

#[derive(Clone)]
struct Packet {
    token: u64,
    revision: u64,
    frame: u64,
    sequence: u64,
    snapshot: Box<ScopeSnapshot>,
}

pub(super) struct ScopeProducer {
    input: Input<Packet>,
    sequence: u64,
}

/// A stable, coherent capture owned by the non-real-time reader.
#[derive(Clone, Debug)]
pub(super) struct ScopeFrame {
    pub token: u64,
    pub revision: u64,
    pub frame: u64,
    pub sequence: u64,
    pub snapshot: Arc<ScopeSnapshot>,
}

struct Consumer {
    output: Output<Packet>,
    cached: Option<ScopeFrame>,
}

/// Clones share one consumer; UI and MCP serialize only with each other.
#[derive(Clone)]
pub(super) struct ScopeReader(Arc<Mutex<Consumer>>);

pub(super) fn channel(token: u64, revision: u64) -> (ScopeProducer, ScopeReader) {
    let initial = Packet {
        token,
        revision,
        frame: 0,
        sequence: 0,
        snapshot: Box::new(ScopeSnapshot::default()),
    };
    let (input, output) = triple_buffer::triple_buffer(&initial);
    (
        ScopeProducer { input, sequence: 0 },
        ScopeReader(Arc::new(Mutex::new(Consumer {
            output,
            cached: None,
        }))),
    )
}

impl ScopeProducer {
    /// Copy into an existing packet and publish with a bounded atomic exchange.
    /// The caller owns publication cadence and per-callback work limits.
    pub(super) fn publish(&mut self, snapshot: &ScopeSnapshot, frame: u64) {
        self.sequence = self.sequence.wrapping_add(1).max(1);
        let packet = self.input.input_buffer_mut();
        packet.snapshot.as_mut().clone_from(snapshot);
        packet.frame = frame;
        packet.sequence = self.sequence;
        self.input.publish();
    }
}

impl ScopeReader {
    /// Exact identity/revision matching prevents exposing a retired graph.
    /// Before first publication, or after mutex poisoning, return unavailable.
    pub(super) fn read(&self, token: u64, revision: u64) -> Option<ScopeFrame> {
        let mut consumer = self.0.lock().ok()?;
        consumer.output.update();
        let packet = consumer.output.output_buffer();
        if packet.sequence == 0 || packet.token != token || packet.revision != revision {
            return None;
        }
        if consumer.cached.as_ref().map(|cached| cached.sequence) != Some(packet.sequence) {
            let frame = ScopeFrame {
                token: packet.token,
                revision: packet.revision,
                frame: packet.frame,
                sequence: packet.sequence,
                snapshot: Arc::from(packet.snapshot.clone()),
            };
            consumer.cached = Some(frame);
        }
        consumer.cached.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openpatch_core::ScopePoint;

    #[test]
    fn publication_requires_exact_identity_and_reuses_cached_capture() {
        let (mut producer, reader) = channel(7, 12);
        assert!(reader.read(7, 12).is_none());
        let mut capture = ScopeSnapshot::default();
        capture.x[0][0] = ScopePoint {
            min: -2.0,
            max: 3.0,
        };
        producer.publish(&capture, 400);
        assert!(reader.read(8, 12).is_none());
        assert!(reader.read(7, 11).is_none());
        assert!(reader.read(7, 13).is_none());
        let first = reader.read(7, 12).unwrap();
        let repeated = reader.clone().read(7, 12).unwrap();
        assert!(Arc::ptr_eq(&first.snapshot, &repeated.snapshot));
        assert_eq!(
            (first.token, first.revision, first.frame, first.sequence),
            (7, 12, 400, 1)
        );
        capture.x[0][0].max = 9.0;
        producer.publish(&capture, 401);
        let second = reader.read(7, 12).unwrap();
        assert!(!Arc::ptr_eq(&first.snapshot, &second.snapshot));
        assert_eq!(first.snapshot.x[0][0].max, 3.0);
        assert_eq!(second.snapshot.x[0][0].max, 9.0);
    }

    #[test]
    fn concurrent_publications_are_coherent_and_latest_survives_overwrites() {
        let (mut producer, reader) = channel(31, 42);
        let writer = std::thread::spawn(move || {
            let mut capture = ScopeSnapshot::default();
            for sequence in 1..=1_000u32 {
                let point = ScopePoint {
                    min: -(sequence as f32),
                    max: sequence as f32,
                };
                capture.x.fill([point; 256]);
                capture.y.fill([point; 256]);
                capture.sample_index = sequence;
                producer.publish(&capture, u64::from(sequence));
            }
        });
        let other_reader = reader.clone();
        let observer = std::thread::spawn(move || {
            for _ in 0..100 {
                if let Some(frame) = other_reader.read(31, 42) {
                    assert_coherent(&frame);
                }
                std::thread::yield_now();
            }
        });
        while !writer.is_finished() {
            if let Some(frame) = reader.read(31, 42) {
                assert_coherent(&frame);
            }
            std::thread::yield_now();
        }
        writer.join().unwrap();
        observer.join().unwrap();
        let final_frame = reader.read(31, 42).unwrap();
        assert_coherent(&final_frame);
        assert_eq!(final_frame.sequence, 1_000);
    }

    fn assert_coherent(frame: &ScopeFrame) {
        assert_eq!(frame.frame, frame.sequence);
        assert_eq!(u64::from(frame.snapshot.sample_index), frame.sequence);
        for axis in [&frame.snapshot.x, &frame.snapshot.y] {
            for point in axis.iter().flatten() {
                assert_eq!(point.min, -(frame.frame as f32));
                assert_eq!(point.max, frame.frame as f32);
            }
        }
    }
}
