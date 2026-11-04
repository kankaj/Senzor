//! Broker passwords never leave Senzor through MCP. Outputs show [`REDACTED`] instead; an input
//! that carries the marker back keeps the password the project already has.

use senzor_core::project::{BrokerSpec, ProjectFile};

/// Stands in for a broker password in everything the server returns.
pub const REDACTED: &str = "<redacted>";

/// A copy of `file` with the broker password replaced by [`REDACTED`].
pub(crate) fn redacted(file: &ProjectFile) -> ProjectFile {
    let mut file = file.clone();
    if file.broker.password.is_some() {
        file.broker.password = Some(REDACTED.to_string());
    }
    file
}

/// Puts the current password back where `incoming` carries the marker.
pub(crate) fn restore_password(incoming: &mut BrokerSpec, current: &BrokerSpec) {
    if incoming.password.as_deref() == Some(REDACTED) {
        incoming.password = current.password.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_password(password: Option<&str>) -> ProjectFile {
        let mut file = ProjectFile::default();
        file.broker.password = password.map(Into::into);
        file
    }

    #[test]
    fn the_password_is_replaced_and_nothing_else_changes() {
        let file = with_password(Some("s3cret"));
        let out = redacted(&file);
        assert_eq!(out.broker.password.as_deref(), Some(REDACTED));
        assert!(!out.to_toml_string().contains("s3cret"));
        assert_eq!(redacted(&with_password(None)), with_password(None));
    }

    #[test]
    fn the_marker_keeps_the_current_password() {
        let current = with_password(Some("s3cret")).broker;
        let mut incoming = redacted(&with_password(Some("s3cret"))).broker;
        incoming.port = 1884;
        restore_password(&mut incoming, &current);
        assert_eq!(incoming.password.as_deref(), Some("s3cret"));
        assert_eq!(incoming.port, 1884);
    }

    #[test]
    fn a_new_password_or_none_is_taken_as_given() {
        let current = with_password(Some("s3cret")).broker;
        let mut changed = with_password(Some("other")).broker;
        restore_password(&mut changed, &current);
        assert_eq!(changed.password.as_deref(), Some("other"));
        let mut removed = with_password(None).broker;
        restore_password(&mut removed, &current);
        assert_eq!(removed.password, None);
    }
}
