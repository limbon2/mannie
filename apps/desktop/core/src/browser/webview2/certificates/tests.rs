use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT,
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED,
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID,
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED, COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
};

use super::verify::{MAX_CERT_DER_BYTES, decode_pem, parse_leaf};
use super::{Certificates, classify_status};
use crate::browser::certificate::CertificatePin;

const SERVER: &str = include_str!("server.pem");
const VALID_UNIX: i64 = 1_767_225_600; // 2026-01-01T00:00:00Z
const AFTER_EXPIRY_UNIX: i64 = 1_830_297_600; // 2028-01-01T00:00:00Z

fn der_tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    if body.len() < 0x80 {
        out.push(u8::try_from(body.len()).unwrap());
    } else if body.len() <= 0xff {
        out.push(0x81);
        out.push(u8::try_from(body.len()).unwrap());
    } else {
        out.push(0x82);
        let len = u16::try_from(body.len()).unwrap();
        out.extend_from_slice(&len.to_be_bytes());
    }
    out.extend_from_slice(body);
    out
}

fn der_name(cn: &str) -> Vec<u8> {
    // SEQUENCE { SET { SEQUENCE { OID 2.5.4.3 (CN), UTF8String } } }
    let oid = der_tlv(0x06, &[0x55, 0x04, 0x03]);
    let val = der_tlv(0x0c, cn.as_bytes());
    let mut atv = oid;
    atv.extend_from_slice(&val);
    der_tlv(0x30, &der_tlv(0x31, &der_tlv(0x30, &atv)))
}

fn synthetic_cert(
    issuer_cn: &str,
    subject_cn: &str,
    sig_oid: &[u8],
    san_entries: &[Vec<u8>],
) -> Vec<u8> {
    let version = der_tlv(0xa0, &der_tlv(0x02, &[2]));
    let serial = der_tlv(0x02, &[1]);
    let sig_alg = der_tlv(0x30, &der_tlv(0x06, sig_oid));
    let issuer = der_name(issuer_cn);
    let mut validity = der_tlv(0x17, b"240101000000Z");
    validity.extend_from_slice(&der_tlv(0x18, b"20250101000000Z"));
    let validity = der_tlv(0x30, &validity);
    let subject = der_name(subject_cn);
    let spki = der_tlv(0x30, &[]);

    let mut tbs = Vec::new();
    tbs.extend_from_slice(&version);
    tbs.extend_from_slice(&serial);
    tbs.extend_from_slice(&sig_alg);
    tbs.extend_from_slice(&issuer);
    tbs.extend_from_slice(&validity);
    tbs.extend_from_slice(&subject);
    tbs.extend_from_slice(&spki);

    if !san_entries.is_empty() {
        let mut san_body = Vec::new();
        for entry in san_entries {
            san_body.extend_from_slice(entry);
        }
        let san_ext_val = der_tlv(0x04, &der_tlv(0x30, &san_body));
        let mut ext = der_tlv(0x06, &[0x55, 0x1d, 0x11]);
        ext.extend_from_slice(&san_ext_val);
        let extensions = der_tlv(0xa3, &der_tlv(0x30, &der_tlv(0x30, &ext)));
        tbs.extend_from_slice(&extensions);
    }

    let mut cert = der_tlv(0x30, &tbs);
    cert.extend_from_slice(&sig_alg);
    cert.extend_from_slice(&der_tlv(0x03, &[0x00, 0xaa, 0xbb]));
    der_tlv(0x30, &cert)
}

#[test]
fn only_exact_pins_allowed_by_the_session_need_revocation() {
    let saved_pin = CertificatePin {
        hostname: "localhost".into(),
        fingerprint: [1; 32],
    };
    let replacement = CertificatePin {
        fingerprint: [2; 32],
        ..saved_pin.clone()
    };
    let mut certificates = Certificates::default();
    assert!(!certificates.allowed(&saved_pin));
    certificates.allowed.insert(saved_pin.clone());
    assert!(certificates.allowed(&saved_pin));
    assert!(!certificates.allowed(&replacement));
}

#[test]
fn status_classification_allows_invalid_mismatch_and_expired_but_refuses_revoked() {
    assert_eq!(
        classify_status(COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID),
        ("untrusted_authority", true)
    );
    assert_eq!(
        classify_status(COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT),
        ("hostname_mismatch", true)
    );
    assert_eq!(
        classify_status(COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED),
        ("expired", true)
    );
    assert_eq!(
        classify_status(COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED),
        ("revoked", false)
    );
    assert_eq!(
        classify_status(COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN),
        ("invalid", false)
    );
}

#[test]
fn leaf_parser_accepts_self_signed_weak_cn_only_and_cross_signed_fixtures() {
    for (name, pem) in [
        ("server", SERVER),
        ("rsa-pss", include_str!("rsa-pss.pem")),
        ("weak key", include_str!("weak-key.pem")),
        ("client-only", include_str!("client-only.pem")),
        ("common-name-only", include_str!("common-name-only.pem")),
        (
            "different signing key",
            include_str!("different-signing-key.pem"),
        ),
    ] {
        let der = decode_pem(pem).unwrap();
        let parsed = parse_leaf(&der).unwrap_or_else(|| panic!("{name} must parse"));
        assert!(!parsed.subject.is_empty(), "{name} has a subject");
        assert!(!parsed.not_before_iso.is_empty(), "{name} has not_before");
        assert!(!parsed.not_after_iso.is_empty(), "{name} has not_after");
    }

    let server = parse_leaf(&decode_pem(SERVER).unwrap()).unwrap();
    assert!(server.self_signed);
    assert!(!server.is_expired(VALID_UNIX));
    assert!(server.is_expired(AFTER_EXPIRY_UNIX));
    assert!(server.sans.iter().any(|san| san == "DNS:localhost"));
    assert!(server.sans.iter().any(|san| san == "IP:127.0.0.1"));
}

#[test]
fn leaf_parser_accepts_private_ca_ed25519_expired_and_sanitizes_fields() {
    // OID 1.3.101.112 (Ed25519)
    let ed25519_oid = [0x2b, 0x65, 0x70];
    let dns_san = der_tlv(0x82, b"pve\x07.lan");
    let ipv4_san = der_tlv(0x87, &[192, 168, 1, 50]);
    let mut ipv6 = [0u8; 16];
    ipv6[15] = 1;
    let ipv6_san = der_tlv(0x87, &ipv6);

    let der = synthetic_cert(
        "PVE Cluster CA",
        "pve-node-1\r\n",
        &ed25519_oid,
        &[dns_san, ipv4_san, ipv6_san],
    );
    let parsed = parse_leaf(&der).expect("private-CA Ed25519 leaf must parse");
    assert!(!parsed.self_signed);
    assert_eq!(parsed.issuer, "CN=PVE Cluster CA");
    assert_eq!(parsed.subject, "CN=pve-node-1");
    assert_eq!(parsed.not_before_iso, "2024-01-01T00:00:00Z");
    assert_eq!(parsed.not_after_iso, "2025-01-01T00:00:00Z");
    assert!(parsed.is_expired(VALID_UNIX));
    assert_eq!(
        parsed.sans,
        vec!["DNS:pve.lan", "IP:192.168.1.50", "IP:::1"]
    );
}

#[test]
fn leaf_parser_rejects_malformed_truncated_and_oversized_der() {
    assert!(parse_leaf(&[]).is_none());
    assert!(parse_leaf(&[0x30, 0x03, 0x02, 0x01]).is_none());
    let oversized = vec![0x30; MAX_CERT_DER_BYTES + 1];
    assert!(parse_leaf(&oversized).is_none());

    let mut with_trailing = decode_pem(SERVER).unwrap();
    with_trailing.push(0x00);
    assert!(parse_leaf(&with_trailing).is_none());
}
