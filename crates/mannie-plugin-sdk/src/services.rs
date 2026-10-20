//! The services of the SDK (ADR-0034, decision 5). Each module holds one
//! service's marker type, its API and the values that the API takes and
//! returns. The core module that provides a service lists its provider, and
//! the composition root hands every list to the host once.

pub mod credentials;
pub mod executor_host;
pub mod file_dialog;
pub mod processes;
pub mod secrets;
pub mod store;

use crate::service::ServiceInfo;

/// Every service of the SDK, by name. A manifest declares services from this
/// list alone, each at its major version, and the host binds only what the
/// manifest declares.
pub const SERVICES: &[ServiceInfo] = &[
    ServiceInfo::of::<credentials::Credentials>(),
    ServiceInfo::of::<executor_host::ExecutorHost>(),
    ServiceInfo::of::<file_dialog::FileDialog>(),
    ServiceInfo::of::<processes::Processes>(),
    ServiceInfo::of::<secrets::Secrets>(),
    ServiceInfo::of::<store::Store>(),
];

/// The service with this name.
pub fn service_info(name: &str) -> Option<ServiceInfo> {
    SERVICES
        .iter()
        .copied()
        .find(|service| service.name == name)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{SERVICES, service_info};
    use crate::service::{Service, ServiceInfo};
    use crate::services::credentials::Credentials;
    use crate::services::executor_host::ExecutorHost;
    use crate::services::file_dialog::FileDialog;
    use crate::services::processes::Processes;
    use crate::services::secrets::Secrets;
    use crate::services::store::Store;

    #[test]
    fn every_service_has_a_name_of_its_own_its_major_and_its_sensitivity() {
        let names: BTreeSet<&str> = SERVICES.iter().map(|service| service.name).collect();
        let listed: Vec<(&str, u16, bool)> = SERVICES
            .iter()
            .map(|service| (service.name, service.major, service.sensitive))
            .collect();

        assert_eq!(names.len(), SERVICES.len());
        assert_eq!(
            listed,
            [
                ("credentials", 1, true),
                ("executor-host", 1, false),
                ("file-dialog", 1, false),
                ("processes", 1, true),
                ("secrets", 1, true),
                ("store", 1, false)
            ]
        );
    }

    #[test]
    fn the_list_names_every_marker_with_what_it_declares() {
        for marker in [
            ServiceInfo::of::<Credentials>(),
            ServiceInfo::of::<ExecutorHost>(),
            ServiceInfo::of::<FileDialog>(),
            ServiceInfo::of::<Processes>(),
            ServiceInfo::of::<Secrets>(),
            ServiceInfo::of::<Store>(),
        ] {
            assert_eq!(service_info(marker.name), Some(marker), "{}", marker.name);
        }
        assert_eq!(Credentials::NAME, "credentials");
        assert_eq!(ExecutorHost::NAME, "executor-host");
        assert_eq!(FileDialog::NAME, "file-dialog");
        assert_eq!(Processes::NAME, "processes");
        assert_eq!(Secrets::NAME, "secrets");
        assert_eq!(Store::NAME, "store");
        assert_eq!(service_info("teleport"), None);
        assert_eq!(service_info("Executor-Host"), None);
    }

    #[test]
    fn service_names_are_lower_case_words_joined_by_hyphens() {
        for service in SERVICES {
            let words_are_valid = service.name.split('-').all(|word| {
                !word.is_empty() && word.chars().all(|character| character.is_ascii_lowercase())
            });
            assert!(words_are_valid, "{}", service.name);
        }
    }
}
