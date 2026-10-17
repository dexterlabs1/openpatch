use super::*;

fn region(directory: &tempfile::TempDir) -> Region {
    let region = Region::create(
        &directory.path().join("transport"),
        Fence {
            incarnation: 17,
            generation: 9,
            revision: 11,
        },
        4,
        SAMPLE_RATE,
    )
    .unwrap();
    region.atomic(STATUS).store(1, Ordering::Release);
    region
}

// Ownership-controlled simulated peer only for malformed-output/parent failure
// tests. Actual binary processing is exercised separately by the companion gate.
fn publish(endpoint: &Endpoint<'_>, sequence: u64) -> usize {
    let slot = HEADER + (sequence % SLOTS as u64) as usize * SLOT;
    assert!(endpoint
        .region
        .atomic(slot)
        .compare_exchange(INPUT_READY, 2, Ordering::Acquire, Ordering::Relaxed)
        .is_ok());
    unsafe {
        for frame in 0..QUANTUM {
            endpoint.region.write(slot + OUTPUT_COUNTS + frame, 1u8);
            endpoint.region.write(
                slot + OUTPUT_VALUES + frame * CHANNELS * 4,
                sequence as f32 + 0.5,
            );
        }
    }
    endpoint
        .region
        .atomic(slot)
        .store(OUTPUT_READY, Ordering::Release);
    slot
}

#[test]
fn complete_ring_wraps_without_replaying_old_output() {
    let directory = tempfile::tempdir().unwrap();
    let mut region = region(&directory);
    {
        let mut endpoint = region.take_endpoint().unwrap();
        let input = InputBlock::silence(4);
        let mut output = OutputBlock::default();
        for sequence in 0..32 {
            assert_eq!(endpoint.submit(&input), Ok(sequence));
            publish(&endpoint, sequence);
            assert_eq!(endpoint.receive_due(&mut output), Ok(sequence));
            assert_eq!(output.counts, [1; QUANTUM]);
            assert!(output.values.iter().all(|v| v[0] == sequence as f32 + 0.5));
            assert_eq!(endpoint.try_receive(&mut output), Ok(None));
            assert!(output.values.iter().flatten().all(|v| *v == 0.));
        }
    }
    assert!(
        region.take_endpoint().is_none(),
        "retired endpoint must not replay sequence zero"
    );
}

#[test]
fn queue_full_never_steals_a_child_slot() {
    let directory = tempfile::tempdir().unwrap();
    let mut region = region(&directory);
    let mut endpoint = region.take_endpoint().unwrap();
    let input = InputBlock::silence(4);
    for sequence in 0..SLOTS {
        assert_eq!(endpoint.submit(&input), Ok(sequence as u64));
    }
    assert_eq!(endpoint.submit(&input), Err(Fault::QueueFull));
    assert_eq!(
        endpoint.region.atomic(HEADER).load(Ordering::Acquire),
        INPUT_READY
    );
    assert_eq!(unsafe { endpoint.region.read::<u64>(HEADER + 8) }, 0);
}

#[test]
fn deadline_failure_silences_and_never_resumes_on_late_results() {
    let directory = tempfile::tempdir().unwrap();
    let mut region = region(&directory);
    let mut endpoint = region.take_endpoint().unwrap();
    let input = InputBlock::silence(4);
    let mut output = OutputBlock::default();
    output.values.fill([9.; CHANNELS]);
    endpoint.submit(&input).unwrap();
    assert_eq!(
        endpoint.receive_due(&mut output),
        Err(Fault::MissedDeadline)
    );
    publish(&endpoint, 0);
    assert_eq!(
        endpoint.receive_due(&mut output),
        Err(Fault::MissedDeadline)
    );
    assert_eq!(endpoint.submit(&input), Err(Fault::MissedDeadline));
    assert_eq!(output.counts, [0; QUANTUM]);
    assert!(output.values.iter().flatten().all(|v| *v == 0.));
}

#[test]
fn stale_incarnation_generation_revision_and_sequence_are_rejected() {
    for field in [8, 16, 24, 32] {
        let directory = tempfile::tempdir().unwrap();
        let mut region = region(&directory);
        let mut endpoint = region.take_endpoint().unwrap();
        endpoint.submit(&InputBlock::silence(4)).unwrap();
        let slot = publish(&endpoint, 0);
        // Stand in for a broken/malicious peer: mutate before parent observes.
        unsafe {
            endpoint.region.write(slot + field, 999u64);
        }
        let mut output = OutputBlock::default();
        assert_eq!(endpoint.receive_due(&mut output), Err(Fault::StaleOutput));
        assert_eq!(output.counts, [0; QUANTUM]);
        assert!(output.values.iter().flatten().all(|v| *v == 0.));
    }
}

#[test]
fn malformed_output_is_discarded_atomically() {
    for variant in 0..4 {
        let directory = tempfile::tempdir().unwrap();
        let mut region = region(&directory);
        let mut endpoint = region.take_endpoint().unwrap();
        endpoint.submit(&InputBlock::silence(4)).unwrap();
        let slot = publish(&endpoint, 0);
        unsafe {
            match variant {
                0 => endpoint.region.write(slot + OUTPUT_COUNTS + 127, 17u8),
                1 => endpoint
                    .region
                    .write(slot + OUTPUT_VALUES + 127 * CHANNELS * 4, f32::NAN),
                2 => endpoint.region.write(slot + OUTPUT_VALUES + 4, 1f32),
                _ => endpoint.region.write(slot + 44, 1u8),
            }
        }
        let mut output = OutputBlock::default();
        assert_eq!(endpoint.receive_due(&mut output), Err(Fault::InvalidOutput));
        assert!(output.values.iter().flatten().all(|v| *v == 0.));
    }
}

#[test]
fn invalid_inputs_cannot_be_partially_published() {
    for variant in 0..6 {
        let directory = tempfile::tempdir().unwrap();
        let mut region = region(&directory);
        let mut endpoint = region.take_endpoint().unwrap();
        let mut input = InputBlock::silence(4);
        match variant {
            0 => input.counts[2][3] = 0,
            1 => input.values[2][4][0] = f32::INFINITY,
            2 => input.values[0][0][0] = 1.,
            3 => input.event_count = MAX_EVENTS + 1,
            4 => {
                input.event_count = 1;
                input.events[0].parameter = 2;
            }
            _ => {
                input.event_count = 2;
                input.events[0].sample_offset = 12;
            }
        }
        assert_eq!(endpoint.submit(&input), Err(Fault::InvalidInput));
        assert_eq!(endpoint.region.atomic(HEADER).load(Ordering::Acquire), FREE);
    }
}

#[test]
fn ordered_simultaneous_events_and_per_frame_channel_counts_are_retained() {
    let directory = tempfile::tempdir().unwrap();
    let mut region = region(&directory);
    let mut endpoint = region.take_endpoint().unwrap();
    let mut input = InputBlock::silence(4);
    input.counts[2][37] = 16;
    input.values[2][37][15] = 4.;
    input.event_count = 2;
    input.events[0] = ParamEvent {
        sample_offset: 37,
        parameter: 0,
        value: 0.5,
        order: 1,
    };
    input.events[1] = ParamEvent {
        sample_offset: 37,
        parameter: 1,
        value: -0.25,
        order: 2,
    };
    assert_eq!(endpoint.submit(&input), Ok(0));
    unsafe {
        assert_eq!(
            endpoint
                .region
                .read::<u8>(HEADER + COUNTS + 2 * QUANTUM + 37),
            16
        );
        assert_eq!(
            endpoint.region.read::<f32>(
                HEADER + INPUT_VALUES + (2 * QUANTUM * CHANNELS + 37 * CHANNELS + 15) * 4
            ),
            4.
        );
        assert_eq!(endpoint.region.read::<u32>(HEADER + 40), 2);
        assert_eq!(
            endpoint.region.read::<u32>(HEADER + EVENT_VALUES + 16 + 8),
            2
        );
    }
}

#[test]
fn unsupported_configuration_and_existing_files_are_preserved() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("transport");
    let fence = Fence {
        incarnation: 1,
        generation: 1,
        revision: 1,
    };
    assert!(Region::create(&path, fence, 4, 44100).is_err());
    assert!(!path.exists());
    std::fs::write(&path, b"keep me").unwrap();
    assert!(Region::create(&path, fence, 4, SAMPLE_RATE).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"keep me");
}
