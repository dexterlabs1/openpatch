//! Decode and prepare on a detached control snapshot so UI/MCP readers and Stop
//! can acquire the live Session while an imported table is being prepared.
use super::*;
use std::sync::atomic::AtomicBool;

pub(super) fn execute(
    session: &SharedSession,
    args: Value,
    authorization: Option<&AtomicBool>,
) -> Result<Value, String> {
    let (mut snapshot, base_revision) = {
        let mut live = session.lock().map_err(|_| "session lock poisoned")?;
        check_transport_authorization(authorization)?;
        live.check_open()?;
        live.reconcile_midi_cc_learn()?;
        validate_arguments(&tools(), "import_wavetable", &args)?;
        live.check_revision(expected_revision(&args)?)?;
        (live.detached_snapshot(), live.revision)
    };

    // One file read and one preparation, with the shared admission cache. The
    // snapshot owns the prepared ticket until live.commit has resolved its banks.
    let mut receipt = snapshot.execute("import_wavetable", args)?;
    let changed = receipt["changed"]
        .as_bool()
        .ok_or("wavetable import did not return a change receipt")?;

    #[cfg(test)]
    transport_authorization_tests::pause_if_requested(
        session,
        transport_authorization_tests::Boundary::WavetableImportPrepared,
    );

    let mut live = session.lock().map_err(|_| "session lock poisoned")?;
    check_transport_authorization(authorization)?;
    live.check_open()?;
    live.check_revision(Some(base_revision))?;
    if changed {
        live.commit(
            snapshot.patch.clone(),
            Some(base_revision),
            None,
            &EditEffects::default(),
        )?;
    }
    receipt["revision"] = json!(live.revision);
    Ok(receipt)
}
