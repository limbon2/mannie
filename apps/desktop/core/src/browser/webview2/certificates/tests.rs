use super::Certificates;
use super::verify::{decode_pem, verified_leaf};
use crate::browser::certificate::CertificatePin;
use mannie_tools::browser::ChromeError;
use windows::Win32::Foundation::FILETIME;

const SERVER: &str = include_str!("server.pem");
const VALID_TIME: FILETIME = filetime(134_116_992_000_000_000); // 2026-01-01 UTC

#[test]
fn temporary_pins_take_precedence_and_only_exact_live_pins_need_revocation() {
    let saved_pin = CertificatePin {
        hostname: "localhost".into(),
        fingerprint: [1; 32],
    };
    let temporary = CertificatePin {
        fingerprint: [2; 32],
        ..saved_pin.clone()
    };
    let mut certificates = Certificates::default();
    let saved = [(saved_pin.hostname.clone(), saved_pin.fingerprint)]
        .into_iter()
        .collect();

    assert_eq!(
        certificates.fingerprint("localhost", Some(&saved)),
        Some(saved_pin.fingerprint)
    );
    assert!(!certificates.could_use(&saved_pin));
    certificates.add(temporary.clone()).unwrap();
    assert_eq!(
        certificates.fingerprint("localhost", Some(&saved)),
        Some(temporary.fingerprint)
    );
    assert!(certificates.could_use(&temporary));
    assert!(!certificates.could_use(&saved_pin));

    let mut certificates = Certificates::default();
    certificates.allowed.insert(saved_pin.clone());
    assert!(certificates.could_use(&saved_pin));
    assert_eq!(
        certificates.add(temporary),
        Err(ChromeError::CertificateConflict)
    );
}

const fn filetime(ticks: u64) -> FILETIME {
    let [low_0, low_1, low_2, low_3, high_0, high_1, high_2, high_3] = ticks.to_le_bytes();
    FILETIME {
        dwLowDateTime: u32::from_le_bytes([low_0, low_1, low_2, low_3]),
        dwHighDateTime: u32::from_le_bytes([high_0, high_1, high_2, high_3]),
    }
}

#[test]
fn certificate_verifier_accepts_strong_self_signed_leafs_with_matching_dns_or_ip() {
    for pem in [SERVER, include_str!("rsa-pss.pem")] {
        let der = decode_pem(pem).unwrap();
        for host in ["localhost", "127.0.0.1", "::1"] {
            assert!(
                verified_leaf(&der, host, Some(&VALID_TIME)).is_some(),
                "{host} must match a valid server certificate"
            );
        }
        for host in ["other.local", "127.0.0.2", "::2"] {
            assert!(verified_leaf(&der, host, Some(&VALID_TIME)).is_none());
        }
    }
}

#[test]
fn certificate_verifier_refuses_certificates_outside_their_validity() {
    let der = decode_pem(SERVER).unwrap();
    let too_early = filetime(133_485_408_000_000_000); // 2024-01-01 UTC
    let too_late = filetime(134_747_712_000_000_000); // 2028-01-01 UTC
    assert!(verified_leaf(&der, "localhost", Some(&too_early)).is_none());
    assert!(verified_leaf(&der, "localhost", Some(&too_late)).is_none());
}

#[test]
fn certificate_verifier_refuses_weak_inapplicable_or_non_self_signed_leafs() {
    for (reason, pem) in [
        ("weak key", include_str!("weak-key.pem")),
        (
            "client authentication only",
            include_str!("client-only.pem"),
        ),
        (
            "matching common name without a SAN",
            include_str!("common-name-only.pem"),
        ),
        (
            "matching issuer and subject but signed with another key",
            include_str!("different-signing-key.pem"),
        ),
    ] {
        let der = decode_pem(pem).unwrap();
        assert!(
            verified_leaf(&der, "localhost", Some(&VALID_TIME)).is_none(),
            "{reason} must be refused"
        );
    }
    let mut tampered = decode_pem(SERVER).unwrap();
    *tampered.last_mut().unwrap() ^= 1;
    assert!(verified_leaf(&tampered, "localhost", Some(&VALID_TIME)).is_none());
}

#[test]
fn session_certificate_pins_are_idempotent_and_cannot_be_replaced() {
    let mut certificates = Certificates::default();
    let pin = CertificatePin {
        hostname: "localhost".into(),
        fingerprint: [1; 32],
    };
    certificates.add(pin.clone()).unwrap();
    certificates.add(pin).unwrap();
    let replacement = CertificatePin {
        hostname: "localhost".into(),
        fingerprint: [2; 32],
    };
    assert!(matches!(
        certificates.add(replacement),
        Err(ChromeError::CertificateConflict)
    ));
    assert_eq!(certificates.pins.get("localhost"), Some(&[1; 32]));
}
