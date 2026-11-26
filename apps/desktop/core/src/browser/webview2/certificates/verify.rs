//! Decodes a WebView2 server leaf PEM and parses bounded X.509 metadata in
//! pure Rust without enforcing CA chains, self-signing, SANs or expiry
//! (ADR-0082).

use std::net::{Ipv4Addr, Ipv6Addr};

use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Security::Cryptography::{
    CRYPT_STRING, CRYPT_STRING_BASE64HEADER, CRYPT_STRING_STRICT, CryptStringToBinaryW,
};

/// Maximum leaf certificate DER size accepted from WebView2.
pub(super) const MAX_CERT_DER_BYTES: usize = 64 * 1024;
/// Maximum formatted Subject or Issuer string length exposed to the pane.
const MAX_NAME_CHARS: usize = 256;
/// Maximum SAN entries extracted from a leaf certificate.
const MAX_SANS: usize = 32;
/// Maximum characters per SAN entry.
const MAX_SAN_CHARS: usize = 128;
/// OID 2.5.29.17 (subjectAltName) DER value bytes.
const OID_SAN: &[u8] = &[0x55, 0x1d, 0x11];

/// Sanitized X.509 leaf certificate metadata for diagnostics and owner
/// inspection (ADR-0082).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParsedLeafCertificate {
    pub(super) subject: String,
    pub(super) issuer: String,
    pub(super) sans: Vec<String>,
    pub(super) not_before_unix: i64,
    pub(super) not_after_unix: i64,
    pub(super) not_before_iso: String,
    pub(super) not_after_iso: String,
    pub(super) self_signed: bool,
}

impl ParsedLeafCertificate {
    pub(super) fn is_expired(&self, now_unix: i64) -> bool {
        now_unix < self.not_before_unix || now_unix > self.not_after_unix
    }
}

/// Decodes the single leaf PEM returned by WebView2.
pub(super) fn decode_pem(pem: &str) -> windows::core::Result<Vec<u8>> {
    let pem = pem.trim();
    if !pem.starts_with("-----BEGIN CERTIFICATE-----")
        || !pem.ends_with("-----END CERTIFICATE-----")
    {
        return Err(windows::core::Error::from_hresult(E_INVALIDARG));
    }
    let pem: Vec<u16> = pem.encode_utf16().collect();
    let flags = CRYPT_STRING(CRYPT_STRING_BASE64HEADER.0 | CRYPT_STRING_STRICT.0);
    let mut length = 0;
    // SAFETY: a null output asks Windows for the required size.
    unsafe { CryptStringToBinaryW(&pem, flags, None, &raw mut length, None, None) }?;
    if length == 0 || length as usize > MAX_CERT_DER_BYTES {
        return Err(windows::core::Error::from_hresult(E_INVALIDARG));
    }
    let mut der = vec![0; length as usize];
    // SAFETY: the output is writable and has the capacity Windows reported.
    unsafe {
        CryptStringToBinaryW(
            &pem,
            flags,
            Some(der.as_mut_ptr()),
            &raw mut length,
            None,
            None,
        )
    }?;
    der.truncate(length as usize);
    Ok(der)
}

/// Parses structural X.509 metadata from a leaf DER certificate.
///
/// Validates the outer ASN.1 certificate envelope and extracts Subject,
/// Issuer, Validity and SANs. Does not require `Issuer == Subject`, a
/// matching SAN, specific signature algorithms or current validity.
pub(super) fn parse_leaf(der: &[u8]) -> Option<ParsedLeafCertificate> {
    if der.is_empty() || der.len() > MAX_CERT_DER_BYTES {
        return None;
    }

    let mut outer = DerReader::new(der);
    let cert_body = outer.expect_tag(0x30)?;
    if !outer.is_empty() {
        return None;
    }

    let mut cert = DerReader::new(cert_body);
    let tbs_body = cert.expect_tag(0x30)?;
    let _sig_alg = cert.expect_tag(0x30)?;
    let _sig_bits = cert.expect_tag(0x03)?;
    if !cert.is_empty() {
        return None;
    }

    let mut tbs = DerReader::new(tbs_body);
    // Optional explicit [0] version.
    if tbs.peek_tag() == Some(0xa0) {
        let _version = tbs.read_tlv()?;
    }

    // serialNumber (INTEGER)
    let _serial = tbs.expect_tag(0x02)?;
    // signature (AlgorithmIdentifier SEQUENCE)
    let _tbs_sig_alg = tbs.expect_tag(0x30)?;
    // issuer (Name SEQUENCE)
    let issuer_der = tbs.expect_tag(0x30)?;
    // validity (Validity SEQUENCE)
    let validity_der = tbs.expect_tag(0x30)?;
    // subject (Name SEQUENCE)
    let subject_der = tbs.expect_tag(0x30)?;
    // subjectPublicKeyInfo (SEQUENCE)
    let _spki = tbs.expect_tag(0x30)?;

    let issuer = parse_name(issuer_der).unwrap_or_default();
    let subject = parse_name(subject_der).unwrap_or_default();
    let self_signed = issuer_der == subject_der;
    let (not_before_unix, not_before_iso, not_after_unix, not_after_iso) =
        parse_validity(validity_der)?;

    let mut sans = Vec::new();
    while !tbs.is_empty() {
        let (tag, content) = tbs.read_tlv()?;
        // Explicit [3] extensions.
        if tag == 0xa3 {
            sans = parse_extensions_for_san(content);
            break;
        }
    }

    Some(ParsedLeafCertificate {
        subject,
        issuer,
        sans,
        not_before_unix,
        not_after_unix,
        not_before_iso,
        not_after_iso,
        self_signed,
    })
}

struct DerReader<'a> {
    data: &'a [u8],
}

impl<'a> DerReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn peek_tag(&self) -> Option<u8> {
        self.data.first().copied()
    }

    fn expect_tag(&mut self, expected: u8) -> Option<&'a [u8]> {
        let (tag, content) = self.read_tlv()?;
        (tag == expected).then_some(content)
    }

    fn read_tlv(&mut self) -> Option<(u8, &'a [u8])> {
        let tag = *self.data.first()?;
        let rest = &self.data[1..];
        let first_len = *rest.first()?;
        let (len, header_len) = if first_len < 0x80 {
            (usize::from(first_len), 1usize)
        } else {
            let num_bytes = usize::from(first_len & 0x7f);
            if num_bytes == 0 || num_bytes > 4 || rest.len() < 1 + num_bytes {
                return None;
            }
            let mut length = 0usize;
            for &byte in &rest[1..=num_bytes] {
                length = (length << 8) | usize::from(byte);
            }
            (length, 1 + num_bytes)
        };

        let body_end = header_len.checked_add(len)?;
        if body_end > rest.len() {
            return None;
        }

        let content = &rest[header_len..body_end];
        self.data = &rest[body_end..];
        Some((tag, content))
    }
}

fn sanitize_text(text: &str, max_chars: usize) -> String {
    text.chars()
        .filter(|ch| !ch.is_control())
        .take(max_chars)
        .collect()
}

fn decode_directory_string(tag: u8, bytes: &[u8]) -> String {
    // Tag 0x1e is ASN.1 BMPString (big-endian UTF-16).
    if tag == 0x1e && bytes.len().is_multiple_of(2) {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_be_bytes(*pair))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn parse_name(name_der: &[u8]) -> Option<String> {
    let mut reader = DerReader::new(name_der);
    let mut parts = Vec::new();

    while !reader.is_empty() {
        let rdn_set = reader.expect_tag(0x31)?;
        let mut rdn = DerReader::new(rdn_set);
        while !rdn.is_empty() {
            let atv = rdn.expect_tag(0x30)?;
            let mut fields = DerReader::new(atv);
            let oid = fields.expect_tag(0x06)?;
            let (val_tag, val_bytes) = fields.read_tlv()?;
            let key = oid_short_name(oid);
            let decoded = decode_directory_string(val_tag, val_bytes);
            let value = sanitize_text(&decoded, MAX_NAME_CHARS);
            if !value.is_empty() {
                parts.push(format!("{key}={value}"));
            }
        }
    }

    Some(sanitize_text(&parts.join(", "), MAX_NAME_CHARS))
}

fn oid_short_name(oid: &[u8]) -> &'static str {
    match oid {
        [0x55, 0x04, 0x03] => "CN",
        [0x55, 0x04, 0x06] => "C",
        [0x55, 0x04, 0x07] => "L",
        [0x55, 0x04, 0x08] => "ST",
        [0x55, 0x04, 0x0a] => "O",
        [0x55, 0x04, 0x0b] => "OU",
        _ => "OID",
    }
}

fn parse_validity(validity_der: &[u8]) -> Option<(i64, String, i64, String)> {
    let mut reader = DerReader::new(validity_der);
    let (tag_before, bytes_before) = reader.read_tlv()?;
    let (tag_after, bytes_after) = reader.read_tlv()?;
    let (before_unix, before_iso) = parse_asn1_time(tag_before, bytes_before)?;
    let (after_unix, after_iso) = parse_asn1_time(tag_after, bytes_after)?;
    Some((before_unix, before_iso, after_unix, after_iso))
}

fn parse_asn1_time(tag: u8, bytes: &[u8]) -> Option<(i64, String)> {
    let text = std::str::from_utf8(bytes).ok()?;
    let (year, rest) = match tag {
        // UTCTime: YYMMDDHHMMSSZ (RFC 5280: 00..=49 -> 2000..=2049, 50..=99 -> 1950..=1999)
        0x17 => {
            if text.len() < 13 || !text.ends_with('Z') {
                return None;
            }
            let yy: i32 = text[0..2].parse().ok()?;
            let year = if yy >= 50 { 1900 + yy } else { 2000 + yy };
            (year, &text[2..text.len() - 1])
        }
        // GeneralizedTime: YYYYMMDDHHMMSS(.fff)?Z
        0x18 => {
            if text.len() < 15 || !text.ends_with('Z') {
                return None;
            }
            let year: i32 = text[0..4].parse().ok()?;
            (year, &text[4..text.len() - 1])
        }
        _ => return None,
    };

    if rest.len() < 10 {
        return None;
    }

    let month: u32 = rest[0..2].parse().ok()?;
    let day: u32 = rest[2..4].parse().ok()?;
    let hour: u32 = rest[4..6].parse().ok()?;
    let minute: u32 = rest[6..8].parse().ok()?;
    let second: u32 = rest[8..10].parse().ok()?;

    let date_ok = (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour <= 23
        && minute <= 59
        && second <= 60;
    if !date_ok {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let unix = days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second);
    let iso = format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z");
    Some((unix, iso))
}

/// Howard Hinnant's civil-from-days formula: days since 1970-01-01.
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn parse_extensions_for_san(ext_wrapper: &[u8]) -> Vec<String> {
    let mut wrapper = DerReader::new(ext_wrapper);
    let Some(seq_bytes) = wrapper.expect_tag(0x30) else {
        return Vec::new();
    };
    let mut seq = DerReader::new(seq_bytes);

    while !seq.is_empty() {
        let Some(ext_bytes) = seq.expect_tag(0x30) else {
            break;
        };
        let mut ext = DerReader::new(ext_bytes);
        let Some(oid) = ext.expect_tag(0x06) else {
            continue;
        };
        // Optional critical BOOLEAN (0x01).
        if ext.peek_tag() == Some(0x01) {
            let _critical = ext.read_tlv();
        }
        let Some(extn_value) = ext.expect_tag(0x04) else {
            continue;
        };
        if oid == OID_SAN {
            return parse_san_sequence(extn_value);
        }
    }

    Vec::new()
}

fn parse_san_sequence(san_der: &[u8]) -> Vec<String> {
    let mut outer = DerReader::new(san_der);
    let Some(seq_bytes) = outer.expect_tag(0x30) else {
        return Vec::new();
    };
    let mut seq = DerReader::new(seq_bytes);
    let mut sans = Vec::new();

    while !seq.is_empty() && sans.len() < MAX_SANS {
        let Some((tag, value)) = seq.read_tlv() else {
            break;
        };
        match tag {
            // dNSName [2] IMPLICIT IA5String
            0x82 => {
                if let Ok(name) = std::str::from_utf8(value) {
                    let clean = sanitize_text(name, MAX_SAN_CHARS);
                    if !clean.is_empty() {
                        sans.push(format!("DNS:{clean}"));
                    }
                }
            }
            // iPAddress [7] IMPLICIT OCTET STRING
            0x87 => match value.len() {
                4 => {
                    let ip = Ipv4Addr::new(value[0], value[1], value[2], value[3]);
                    sans.push(format!("IP:{ip}"));
                }
                16 => {
                    if let Ok(bytes) = <[u8; 16]>::try_from(value) {
                        sans.push(format!("IP:{}", Ipv6Addr::from(bytes)));
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    sans
}
