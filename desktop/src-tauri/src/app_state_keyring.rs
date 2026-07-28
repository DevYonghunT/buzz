use crate::product::{KEYRING_SERVICE, KEYRING_SERVICE_DEV};

/// Prefix a scoped standalone dev service must carry to be accepted, e.g.
/// `schoolx-desktop-dev.my-worktree`.
fn dev_scope_prefix() -> String {
    format!("{KEYRING_SERVICE_DEV}.")
}

/// Service name for the desktop OS keyring. Debug builds default to a distinct
/// service, while standalone worktree launches may request a scoped dev service.
fn dev_keyring_service(configured: Option<String>) -> String {
    configured
        .filter(|service| service.starts_with(&dev_scope_prefix()))
        .unwrap_or_else(|| KEYRING_SERVICE_DEV.to_string())
}

pub(crate) fn keyring_service() -> &'static str {
    if cfg!(debug_assertions) {
        static DEV_SERVICE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        DEV_SERVICE
            .get_or_init(|| dev_keyring_service(std::env::var("BUZZ_DEV_KEYRING_SERVICE").ok()))
            .as_str()
    } else {
        KEYRING_SERVICE
    }
}

pub(super) fn migration_marker_name(service: &str, default_name: &str) -> String {
    if service == KEYRING_SERVICE || service == KEYRING_SERVICE_DEV {
        default_name.to_string()
    } else {
        format!("identity.{service}.migrated")
    }
}

#[cfg(test)]
mod tests {
    use super::{dev_keyring_service, migration_marker_name, KEYRING_SERVICE, KEYRING_SERVICE_DEV};

    #[test]
    fn standalone_scope_must_remain_under_dev_service() {
        let scoped = format!("{KEYRING_SERVICE_DEV}.example");
        assert_eq!(dev_keyring_service(Some(scoped.clone())), scoped);
        assert_eq!(
            dev_keyring_service(Some(KEYRING_SERVICE.to_string())),
            KEYRING_SERVICE_DEV
        );
    }

    /// A scope that names *Buzz's* dev service must not be honoured: it would
    /// point a SchoolX build at a co-installed Buzz's keychain entries.
    #[test]
    fn buzz_scoped_service_is_rejected() {
        assert_eq!(
            dev_keyring_service(Some("buzz-desktop-dev.example".to_string())),
            KEYRING_SERVICE_DEV
        );
    }

    #[test]
    fn standalone_scope_uses_its_own_migration_marker() {
        assert_eq!(
            migration_marker_name(KEYRING_SERVICE, "identity.migrated"),
            "identity.migrated"
        );
        assert_eq!(
            migration_marker_name(KEYRING_SERVICE_DEV, "identity.migrated"),
            "identity.migrated"
        );
        assert_eq!(
            migration_marker_name(
                &format!("{KEYRING_SERVICE_DEV}.example"),
                "identity.migrated"
            ),
            format!("identity.{KEYRING_SERVICE_DEV}.example.migrated")
        );
    }
}
