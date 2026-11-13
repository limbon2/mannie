//! Validates a WebView2 server leaf using a private, in-memory Windows chain.

use std::ffi::{CStr, c_void};
use std::ptr::{NonNull, null_mut};

use windows::Win32::Foundation::{E_INVALIDARG, FILETIME, HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    AUTHTYPE_SERVER, CERT_ALT_NAME_INFO, CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL, CERT_CHAIN_CONTEXT,
    CERT_CHAIN_DISABLE_AIA, CERT_CHAIN_DISABLE_AUTH_ROOT_AUTO_UPDATE, CERT_CHAIN_ENGINE_CONFIG,
    CERT_CHAIN_PARA, CERT_CHAIN_POLICY_PARA, CERT_CHAIN_POLICY_SSL, CERT_CHAIN_POLICY_STATUS,
    CERT_CONTEXT, CERT_INFO, CERT_OPEN_STORE_FLAGS, CERT_SIGNED_CONTENT_INFO,
    CERT_STORE_ADD_ALWAYS, CERT_STORE_PROV_MEMORY, CERT_STRONG_SIGN_OID_INFO_CHOICE,
    CERT_STRONG_SIGN_PARA, CERT_STRONG_SIGN_PARA_0, CERT_USAGE_MATCH, CRYPT_ALGORITHM_IDENTIFIER,
    CRYPT_DECODE_ALLOC_FLAG, CRYPT_INTEGER_BLOB, CRYPT_MASK_GEN_ALGORITHM,
    CRYPT_RSA_SSA_PSS_PARAMETERS, CRYPT_STRING, CRYPT_STRING_BASE64HEADER, CRYPT_STRING_STRICT,
    CRYPT_VERIFY_CERT_SIGN_DISABLE_MD2_MD4_FLAG, CRYPT_VERIFY_CERT_SIGN_ISSUER_CERT,
    CRYPT_VERIFY_CERT_SIGN_SUBJECT_CERT, CTL_USAGE, CertAddCertificateContextToStore,
    CertCloseStore, CertCompareCertificateName, CertCreateCertificateChainEngine,
    CertCreateCertificateContext, CertFindExtension, CertFreeCertificateChain,
    CertFreeCertificateChainEngine, CertFreeCertificateContext, CertGetCertificateChain,
    CertIsStrongHashToSign, CertOpenStore, CertVerifyCertificateChainPolicy, CryptDecodeObjectEx,
    CryptStringToBinaryW, CryptVerifyCertificateSignatureEx, HCERTCHAINENGINE, HCERTSTORE,
    HTTPSPolicyCallbackData, HTTPSPolicyCallbackData_0, PKCS_RSA_SSA_PSS_TRAILER_FIELD_BC,
    USAGE_MATCH_TYPE_AND, X509_ALTERNATE_NAME, X509_ASN_ENCODING, X509_CERT,
    szOID_CERT_STRONG_SIGN_OS_1, szOID_PKIX_KP_SERVER_AUTH, szOID_RSA_SSA_PSS,
    szOID_SUBJECT_ALT_NAME2,
};
use windows::core::{PCSTR, PCWSTR, PSTR, PWSTR, w};

// CERT_ALT_NAME_ENTRY tags from WinCrypt.h, omitted by the Rust bindings.
const CERT_ALT_NAME_DNS_NAME: u32 = 3;
const CERT_ALT_NAME_IP_ADDRESS: u32 = 8;

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
    if length == 0 {
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

/// The DER lives through validation. None uses the system time.
pub(super) fn verified_leaf(der: &[u8], hostname: &str, at: Option<&FILETIME>) -> Option<FILETIME> {
    // SAFETY: the DER slice is readable for the call.
    let leaf = CertificateContext(NonNull::new(unsafe {
        CertCreateCertificateContext(X509_ASN_ENCODING, der)
    })?);
    // SAFETY: this owns the context returned by CertCreateCertificateContext.
    let context = unsafe { leaf.0.as_ref() };
    // SAFETY: the native certificate context owns a parsed CERT_INFO.
    let info = unsafe { context.pCertInfo.as_ref() }?;
    if !self_signed_and_strong(&leaf, info, der) || !has_server_name(info, hostname) {
        return None;
    }
    let store = CertificateStore::new(&leaf).ok()?;
    let engine = ChainEngine::new(&store).ok()?;
    let chain = engine.build(&leaf, at).ok()?;
    if !chain.is_single_trusted_leaf() || !chain.verifies_name(hostname) {
        return None;
    }
    Some(info.NotAfter)
}

fn self_signed_and_strong(leaf: &CertificateContext, info: &CERT_INFO, der: &[u8]) -> bool {
    // SAFETY: the parsed certificate owns both encoded names.
    if !unsafe {
        CertCompareCertificateName(
            X509_ASN_ENCODING,
            &raw const info.Issuer,
            &raw const info.Subject,
        )
    }
    .as_bool()
    {
        return false;
    }
    let Some(signed) = Decoded::<CERT_SIGNED_CONTENT_INFO>::new(X509_CERT, der) else {
        return false;
    };
    let algorithm = &signed.get().SignatureAlgorithm;
    if !same_algorithm(algorithm, &info.SignatureAlgorithm) {
        return false;
    }
    let Some(hash) = signature_hash(algorithm) else {
        return false;
    };
    let strong = CERT_STRONG_SIGN_PARA {
        cbSize: structure_size::<CERT_STRONG_SIGN_PARA>(),
        dwInfoChoice: CERT_STRONG_SIGN_OID_INFO_CHOICE,
        Anonymous: CERT_STRONG_SIGN_PARA_0 {
            pszOID: PSTR(szOID_CERT_STRONG_SIGN_OS_1.0.cast_mut()),
        },
    };
    // SAFETY: the parameters, OID, hash and certificate are live inputs.
    if unsafe { CertIsStrongHashToSign(&raw const strong, hash, Some(leaf.0.as_ptr())) }.is_err() {
        return false;
    }
    let certificate = leaf.0.as_ptr().cast::<c_void>();
    // SAFETY: both typed arguments point to the same live certificate context.
    unsafe {
        CryptVerifyCertificateSignatureEx(
            None,
            X509_ASN_ENCODING,
            CRYPT_VERIFY_CERT_SIGN_SUBJECT_CERT,
            certificate,
            CRYPT_VERIFY_CERT_SIGN_ISSUER_CERT,
            Some(certificate),
            CRYPT_VERIFY_CERT_SIGN_DISABLE_MD2_MD4_FLAG,
            None,
        )
    }
    .is_ok()
}

fn same_algorithm(left: &CRYPT_ALGORITHM_IDENTIFIER, right: &CRYPT_ALGORITHM_IDENTIFIER) -> bool {
    oid(left).is_some_and(|left| Some(left) == oid(right))
        && blob(&left.Parameters) == blob(&right.Parameters)
}

fn oid(algorithm: &CRYPT_ALGORITHM_IDENTIFIER) -> Option<&[u8]> {
    let value = algorithm.pszObjId;
    if value.is_null() {
        return None;
    }
    // SAFETY: WinCrypt decoded the OID as a NUL-terminated string owned by
    // the allocation containing the algorithm identifier.
    Some(unsafe { CStr::from_ptr(value.0.cast()) }.to_bytes())
}

fn blob(value: &CRYPT_INTEGER_BLOB) -> &[u8] {
    if value.cbData == 0 || value.pbData.is_null() {
        return &[];
    }
    // SAFETY: the WinCrypt-decoded blob owns this many readable bytes.
    unsafe { std::slice::from_raw_parts(value.pbData, value.cbData as usize) }
}

fn signature_hash(algorithm: &CRYPT_ALGORITHM_IDENTIFIER) -> Option<PCWSTR> {
    match oid(algorithm)? {
        b"1.2.840.113549.1.1.11" | b"1.2.840.10045.4.3.2" => Some(w!("SHA256")),
        b"1.2.840.113549.1.1.12" | b"1.2.840.10045.4.3.3" => Some(w!("SHA384")),
        b"1.2.840.113549.1.1.13" | b"1.2.840.10045.4.3.4" => Some(w!("SHA512")),
        b"1.2.840.113549.1.1.10" => pss_hash(algorithm),
        _ => None,
    }
}

fn oid_mask(mask: &CRYPT_MASK_GEN_ALGORITHM) -> Option<&[u8]> {
    let value = mask.pszObjId;
    if value.is_null() {
        return None;
    }
    // SAFETY: the decoded PSS parameters own a NUL-terminated OID string.
    Some(unsafe { CStr::from_ptr(value.0.cast()) }.to_bytes())
}

fn pss_hash(algorithm: &CRYPT_ALGORITHM_IDENTIFIER) -> Option<PCWSTR> {
    let parameters = Decoded::<CRYPT_RSA_SSA_PSS_PARAMETERS>::new(
        szOID_RSA_SSA_PSS,
        blob(&algorithm.Parameters),
    )?;
    let parameters = parameters.get();
    if oid_mask(&parameters.MaskGenAlgorithm)? != b"1.2.840.113549.1.1.8"
        || !same_algorithm(
            &parameters.HashAlgorithm,
            &parameters.MaskGenAlgorithm.HashAlgorithm,
        )
        || parameters.dwTrailerField != PKCS_RSA_SSA_PSS_TRAILER_FIELD_BC
    {
        return None;
    }
    match (oid(&parameters.HashAlgorithm)?, parameters.dwSaltLength) {
        (b"2.16.840.1.101.3.4.2.1", 32) => Some(w!("SHA256")),
        (b"2.16.840.1.101.3.4.2.2", 48) => Some(w!("SHA384")),
        (b"2.16.840.1.101.3.4.2.3", 64) => Some(w!("SHA512")),
        _ => None,
    }
}

fn has_server_name(info: &CERT_INFO, hostname: &str) -> bool {
    if info.cExtension == 0 || info.rgExtension.is_null() {
        return false;
    }
    // SAFETY: this parsed certificate owns a live array of cExtension entries.
    let extensions =
        unsafe { std::slice::from_raw_parts(info.rgExtension, info.cExtension as usize) };
    // SAFETY: both the OID and the input slice are live.
    let extension = unsafe { CertFindExtension(szOID_SUBJECT_ALT_NAME2, extensions) };
    // SAFETY: a non-null result points to an element in the live extensions array.
    let Some(extension) = (unsafe { extension.as_ref() }) else {
        return false;
    };
    let Some(names) =
        Decoded::<CERT_ALT_NAME_INFO>::new(X509_ALTERNATE_NAME, blob(&extension.Value))
    else {
        return false;
    };
    let names = names.get();
    if names.cAltEntry == 0 || names.rgAltEntry.is_null() {
        return false;
    }
    let choice = if hostname.parse::<std::net::IpAddr>().is_ok() {
        CERT_ALT_NAME_IP_ADDRESS
    } else {
        CERT_ALT_NAME_DNS_NAME
    };
    // SAFETY: Windows owns this array until the decoded names are freed.
    let entries = unsafe { std::slice::from_raw_parts(names.rgAltEntry, names.cAltEntry as usize) };
    entries.iter().any(|entry| entry.dwAltNameChoice == choice)
}

fn structure_size<T>() -> u32 {
    u32::try_from(size_of::<T>()).expect("WinCrypt structures fit in a DWORD")
}

/// Owns a decoded structure. Callers pair each type with its `CryptDecodeObjectEx` OID.
struct Decoded<T>(NonNull<T>);

impl<T> Decoded<T> {
    fn new(kind: PCSTR, bytes: &[u8]) -> Option<Self> {
        let mut decoded: *mut T = null_mut();
        let mut length = 0;
        // SAFETY: the input is readable and ALLOC writes one pointer to the
        // decoded structure. The default allocator is LocalAlloc.
        unsafe {
            CryptDecodeObjectEx(
                X509_ASN_ENCODING,
                kind,
                bytes,
                CRYPT_DECODE_ALLOC_FLAG,
                None,
                Some((&raw mut decoded).cast()),
                &raw mut length,
            )
        }
        .ok()?;
        let decoded = Self(NonNull::new(decoded)?);
        (length >= structure_size::<T>()).then_some(decoded)
    }

    fn get(&self) -> &T {
        // SAFETY: Windows allocated and filled a matching structure.
        unsafe { self.0.as_ref() }
    }
}

impl<T> Drop for Decoded<T> {
    fn drop(&mut self) {
        // SAFETY: the default WinCrypt decoder allocator uses LocalAlloc.
        let _freed = unsafe { LocalFree(Some(HLOCAL(self.0.as_ptr().cast()))) };
    }
}

struct CertificateContext(NonNull<CERT_CONTEXT>);

impl Drop for CertificateContext {
    fn drop(&mut self) {
        // SAFETY: this context was returned by CertCreateCertificateContext.
        let _freed = unsafe { CertFreeCertificateContext(Some(self.0.as_ptr())) };
    }
}

struct CertificateStore(HCERTSTORE);

impl CertificateStore {
    fn new(leaf: &CertificateContext) -> windows::core::Result<Self> {
        // SAFETY: the memory provider takes no extra parameters.
        let store = Self(unsafe {
            CertOpenStore(
                CERT_STORE_PROV_MEMORY,
                X509_ASN_ENCODING,
                None,
                CERT_OPEN_STORE_FLAGS::default(),
                None,
            )
        }?);
        // SAFETY: the store and the certificate are live for the call.
        unsafe {
            CertAddCertificateContextToStore(
                Some(store.0),
                leaf.0.as_ptr(),
                CERT_STORE_ADD_ALWAYS,
                None,
            )
        }?;
        Ok(store)
    }
}

impl Drop for CertificateStore {
    fn drop(&mut self) {
        // SAFETY: this owns the handle returned by CertOpenStore.
        let _closed = unsafe { CertCloseStore(Some(self.0), 0) };
    }
}

struct ChainEngine(HCERTCHAINENGINE);

impl ChainEngine {
    fn new(store: &CertificateStore) -> windows::core::Result<Self> {
        let configuration = CERT_CHAIN_ENGINE_CONFIG {
            cbSize: structure_size::<CERT_CHAIN_ENGINE_CONFIG>(),
            hExclusiveRoot: store.0,
            ..Default::default()
        };
        let mut engine = HCERTCHAINENGINE::default();
        // SAFETY: configuration and its store are live and the handle output is writable.
        unsafe { CertCreateCertificateChainEngine(&raw const configuration, &raw mut engine) }?;
        Ok(Self(engine))
    }

    fn build(
        &self,
        leaf: &CertificateContext,
        at: Option<&FILETIME>,
    ) -> windows::core::Result<Chain> {
        let mut server_auth = PSTR(szOID_PKIX_KP_SERVER_AUTH.0.cast_mut());
        let parameters = CERT_CHAIN_PARA {
            cbSize: structure_size::<CERT_CHAIN_PARA>(),
            RequestedUsage: CERT_USAGE_MATCH {
                dwType: USAGE_MATCH_TYPE_AND,
                Usage: CTL_USAGE {
                    cUsageIdentifier: 1,
                    rgpszUsageIdentifier: &raw mut server_auth,
                },
            },
            ..Default::default()
        };
        let mut chain = null_mut();
        // SAFETY: the engine, certificate and parameter buffers are live.
        // The chain uses the only supplied anchor and no network retrieval.
        unsafe {
            CertGetCertificateChain(
                Some(self.0),
                leaf.0.as_ptr(),
                at.map(std::ptr::from_ref),
                None,
                &raw const parameters,
                CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL
                    | CERT_CHAIN_DISABLE_AIA
                    | CERT_CHAIN_DISABLE_AUTH_ROOT_AUTO_UPDATE,
                None,
                &raw mut chain,
            )
        }?;
        NonNull::new(chain)
            .map(Chain)
            .ok_or_else(windows::core::Error::from_thread)
    }
}

impl Drop for ChainEngine {
    fn drop(&mut self) {
        // SAFETY: this owns the handle returned by CertCreateCertificateChainEngine.
        unsafe { CertFreeCertificateChainEngine(Some(self.0)) };
    }
}

struct Chain(NonNull<CERT_CHAIN_CONTEXT>);

impl Chain {
    fn is_single_trusted_leaf(&self) -> bool {
        // SAFETY: this owns the context and its allocated chain arrays.
        let chain = unsafe { self.0.as_ref() };
        if chain.TrustStatus.dwErrorStatus != 0 || chain.cChain != 1 || chain.rgpChain.is_null() {
            return false;
        }
        // SAFETY: there is exactly one entry in the live array.
        let simple = unsafe { *chain.rgpChain };
        // SAFETY: any non-null entry is a chain allocated by this context.
        unsafe { simple.as_ref() }.is_some_and(|chain| chain.cElement == 1)
    }

    fn verifies_name(&self, hostname: &str) -> bool {
        let mut hostname: Vec<u16> = hostname.encode_utf16().chain([0]).collect();
        let mut ssl = HTTPSPolicyCallbackData {
            Anonymous: HTTPSPolicyCallbackData_0 {
                cbSize: structure_size::<HTTPSPolicyCallbackData>(),
            },
            dwAuthType: AUTHTYPE_SERVER,
            fdwChecks: 0,
            pwszServerName: PWSTR(hostname.as_mut_ptr()),
        };
        let parameters = CERT_CHAIN_POLICY_PARA {
            cbSize: structure_size::<CERT_CHAIN_POLICY_PARA>(),
            pvExtraPolicyPara: (&raw mut ssl).cast(),
            ..Default::default()
        };
        let mut status = CERT_CHAIN_POLICY_STATUS {
            cbSize: structure_size::<CERT_CHAIN_POLICY_STATUS>(),
            ..Default::default()
        };
        // SAFETY: the chain and the parameters (including hostname) are live,
        // and the policy output is writable.
        let checked = unsafe {
            CertVerifyCertificateChainPolicy(
                CERT_CHAIN_POLICY_SSL,
                self.0.as_ptr(),
                &raw const parameters,
                &raw mut status,
            )
        };
        checked.as_bool() && status.dwError == 0
    }
}

impl Drop for Chain {
    fn drop(&mut self) {
        // SAFETY: this owns the context returned by CertGetCertificateChain.
        unsafe { CertFreeCertificateChain(self.0.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_certificate_signatures_are_ineligible() {
        let sha1_with_rsa = windows::core::s!("1.2.840.113549.1.1.5");
        let algorithm = CRYPT_ALGORITHM_IDENTIFIER {
            pszObjId: PSTR(sha1_with_rsa.0.cast_mut()),
            ..Default::default()
        };
        assert!(signature_hash(&algorithm).is_none());
    }
}
