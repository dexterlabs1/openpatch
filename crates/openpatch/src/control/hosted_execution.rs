//! Ephemeral, explicit delayed-host permission. Never a saved playback fallback.
use super::*;
use crate::binary_host::execution::HostedExecutionGrant;

impl Session {
    /// Request preparation of the complete hosted graph. This is not an active
    /// playback receipt, does not change Exact/Partial, and is not persisted.
    /// The application controller must still prepare/admit the full request.
    pub fn request_delayed_hosted_execution(
        &mut self,
        expected_revision: Option<u64>,
    ) -> Result<u64, String> {
        self.check_open()?;
        self.check_revision(expected_revision)?;
        let grant = HostedExecutionGrant::new(&self.document, self.project_save.generation())?;
        // Permission is specific to an actually compilable full graph, not
        // merely a document whose omitted native carrier happened to compile.
        grant
            .prepare(&self.document, self.project_save.generation())?
            .compile(&self.prepared_wave_banks)?;
        let revision = self.revision.checked_add(1).ok_or("revision exhausted")?;
        self.host_lifecycle.supersede_execution();
        self.hosted_execution_grant = Some(grant);
        self.document_execution = true;
        self.revision = revision;
        self.sync_live_intents(&BTreeMap::new(), &[]);
        Ok(revision)
    }

    pub fn revoke_delayed_hosted_execution(
        &mut self,
        expected_revision: Option<u64>,
    ) -> Result<u64, String> {
        self.check_open()?;
        self.check_revision(expected_revision)?;
        if self.hosted_execution_grant.is_none() {
            return Ok(self.revision);
        }
        let revision = self.revision.checked_add(1).ok_or("revision exhausted")?;
        self.host_lifecycle.supersede_execution();
        self.hosted_execution_grant = None;
        self.revision = revision;
        self.sync_live_intents(&BTreeMap::new(), &[]);
        Ok(revision)
    }

    pub(super) fn grant_after_edit(
        &self,
        document: &ProjectDocument,
        effects: &EditEffects,
    ) -> Option<HostedExecutionGrant> {
        // Same-ID recreation changes the incarnation even if its serialized
        // topology is identical. Revocation is permanent, including after undo.
        if effects.next_document_generation.is_some()
            || effects.playback_mode.is_some()
            || !effects.retired.is_empty()
        {
            return None;
        }
        self.grant_after_history(document, &[])
    }

    pub(super) fn grant_after_history(
        &self,
        document: &ProjectDocument,
        retired: &[String],
    ) -> Option<HostedExecutionGrant> {
        if !retired.is_empty() {
            return None;
        }
        self.hosted_execution_grant.as_ref().and_then(|grant| {
            grant
                .matches_document(document, self.project_save.generation())
                .unwrap_or(false)
                .then(|| grant.clone())
        })
    }
}
