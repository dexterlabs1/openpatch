#[test]
fn listen_meter_status_remains_coherent_and_incarnation_bound() {
    let meter = Arc::new(Meter::new());
    let writer = meter.clone();
    let done = Arc::new(AtomicBool::new(false));
    let writer_done = done.clone();
    let thread = std::thread::spawn(move || {
        for frame in 1..=10_000 {
            let connected = frame & 1 != 0;
            writer.write(
                7,
                ModuleSnapshot {
                    revision: 5,
                    frame,
                    state: None,
                    rackmcp_sequence: None,
                    listen: Some(ListenStatus {
                        frames_seen: frame,
                        dropped: frame as u32,
                        left_connected: connected,
                        right_connected: !connected,
                        capturing: connected,
                    }),
                    saved_state: None,
                    controls: None,
                    lights: None,
                    midi_learning: None,
                    parameters: [None; MAX_PUBLISHED_PARAMETERS],
                },
            );
        }
        writer_done.store(true, Ordering::Release);
    });
    let entry = Entry {
        slot: 0,
        token: 7,
        born_revision: 5,
        kind: ModuleKind::RackMcpListen,
        bypassed: false,
        action_epoch: 1,
        parameter_epochs: [0; MAX_PUBLISHED_PARAMETERS],
    };
    loop {
        if let Some(snapshot) = meter.read(entry, 5) {
            let status = snapshot.listen.unwrap();
            assert_eq!(status.frames_seen, snapshot.frame);
            assert_eq!(u64::from(status.dropped), snapshot.frame);
            assert_eq!(status.left_connected, snapshot.frame & 1 != 0);
            assert_eq!(status.right_connected, !status.left_connected);
            assert_eq!(status.capturing, status.left_connected);
        }
        if done.load(Ordering::Acquire) {
            break;
        }
        std::thread::yield_now();
    }
    thread.join().unwrap();
    assert!(meter.read(Entry { token: 8, ..entry }, 5).is_none());
    assert!(meter.read(entry, 6).is_none());
    assert!(ModuleKind::from_name("rackmcp_listen") == Some(ModuleKind::RackMcpListen));
}
