// Hosted (binary plugin) project capture lives on the experimental host branch. On this line no module needs a
// rendezvous with a companion process, so saves always use the ordinary configured snapshot.
pub(crate) struct HostedSave;

impl OpenPatch {
    fn hosted_save_pending(&self) -> bool {
        false
    }

    fn begin_hosted_save(&mut self, _name: &str, _new_file: bool) -> Result<bool, String> {
        Ok(false)
    }

    fn service_hosted_save(&mut self) {}
}
