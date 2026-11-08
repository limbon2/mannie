//! The example RSA key of RFC 7515, appendix A.2, and ID tokens signed with
//! it, for the tests of the ChatGPT sign-in. The key is a published example,
//! so no private key of Mannie's own is in the repository.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::rsa::{KeyPairComponents, PublicKeyComponents};
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use serde_json::{Value, json};

/// The key's modulus, exponents, primes and CRT values, base64url-encoded.
const N: &str = "ofgWCuLjybRlzo0tZWJjNiuSfb4p4fAkd_wWJcyQoTbji9k0l8W26mPddxHmfHQp-Vaw-4qPCJrcS2mJPMEzP1Pt0Bm4d4QlL-yRT-SFd2lZS-pCgNMsD1W_YpRPEwOWvG6b32690r2jZ47soMZo9wGzjb_7OMg0LOL-bSf63kpaSHSXndS5z5rexMdbBYUsLA9e-KXBdQOS-UTo7WTBEMa2R2CapHg665xsmtdVMTBQY4uDZlxvb3qCo5ZwKh9kG4LT6_I5IhlJH7aGhyxXFvUK-DWNmoudF8NAco9_h9iaGNj8q2ethFkMLs91kzk2PAcDTW9gb54h4FRWyuXpoQ";
const E: &str = "AQAB";
const D: &str = "Eq5xpGnNCivDflJsRQBXHx1hdR1k6Ulwe2JZD50LpXyWPEAeP88vLNO97IjlA7_GQ5sLKMgvfTeXZx9SE-7YwVol2NXOoAJe46sui395IW_GO-pWJ1O0BkTGoVEn2bKVRUCgu-GjBVaYLU6f3l9kJfFNS3E0QbVdxzubSu3Mkqzjkn439X0M_V51gfpRLI9JYanrC4D4qAdGcopV_0ZHHzQlBjudU2QvXt4ehNYTCBr6XCLQUShb1juUO1ZdiYoFaFQT5Tw8bGUl_x_jTj3ccPDVZFD9pIuhLhBOneufuBiB4cS98l2SR_RQyGWSeWjnczT0QU91p1DhOVRuOopznQ";
const P: &str = "4BzEEOtIpmVdVEZNCqS7baC4crd0pqnRH_5IB3jw3bcxGn6QLvnEtfdUdiYrqBdss1l58BQ3KhooKeQTa9AB0Hw_Py5PJdTJNPY8cQn7ouZ2KKDcmnPGBY5t7yLc1QlQ5xHdwW1VhvKn-nXqhJTBgIPgtldC-KDV5z-y2XDwGUc";
const Q: &str = "uQPEfgmVtjL0Uyyx88GZFF1fOunH3-7cepKmtH4pxhtCoHqpWmT8YAmZxaewHgHAjLYsp1ZSe7zFYHj7C6ul7TjeLQeZD_YwD66t62wDmpe_HlB-TnBA-njbglfIsRLtXlnDzQkv5dTltRJ11BKBBypeeF6689rjcJIDEz9RWdc";
const DP: &str = "BwKfV3Akq5_MFZDFZCnW-wzl-CCo83WoZvnLQwCTeDv8uzluRSnm71I3QCLdhrqE2e9YkxvuxdBfpT_PI7Yz-FOKnu1R6HsJeDCjn12Sk3vmAktV2zb34MCdy7cpdTh_YVr7tss2u6vneTwrA86rZtu5Mbr1C1XsmvkxHQAdYo0";
const DQ: &str = "h_96-mK1R_7glhsum81dZxjTnYynPbZpHziZjeeHcXYsXaaMwkOlODsWa7I9xXDoRwbKgB719rrmI2oKr6N3Do9U0ajaHF-NKJnwgjMd2w9cjz3_-kyNlxAr2v4IKhGNpmM5iIgOS1VZnOZ68m6_pbLBSp3nssTdlqvd0tIiTHU";
const QI: &str = "IYd7DHOhrWvxkwPQsRM2tOgrjbcrfvtQJipd-DlcxyVuuM9sQLdgjVk2oy26F0EmpScGLq2MowX7fhd_QJQ3ydy5cY7YIBi87w93IKLEdfnbJtoOPLUW0ITrJReOgo1cq9SbsxYawBgfp_gh6A5603k2-ZQwVK0JKSHuLFkuQ3U";

/// The issuer that the tests' tokens name.
#[cfg(test)]
pub(crate) const ISSUER: &str = "https://auth.example.test";
/// The time of the tests: 2026-11-08, in milliseconds.
#[cfg(test)]
pub(crate) const NOW_MILLIS: i64 = 1_794_096_000_000;

/// A base64url constant of the key, decoded.
fn decoded(text: &str) -> Vec<u8> {
    URL_SAFE_NO_PAD
        .decode(text)
        .expect("the example key's values are base64url")
}

/// The key set that publishes the example key under `kid`.
pub(crate) fn key_set(kid: &str) -> Value {
    json!({ "keys": [{ "kty": "RSA", "use": "sig", "alg": "RS256", "kid": kid, "n": N, "e": E }] })
}

/// An ID token with `claims`, signed with the example key under `kid`.
///
/// # Panics
///
/// Never: the example key is a valid RSA key of 2048 bits.
pub(crate) fn signed_token(kid: &str, claims: &Value) -> String {
    let header =
        URL_SAFE_NO_PAD.encode(json!({ "alg": "RS256", "typ": "JWT", "kid": kid }).to_string());
    let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
    let signed = format!("{header}.{payload}");
    let key = RsaKeyPair::from_components(&KeyPairComponents {
        public_key: PublicKeyComponents {
            n: decoded(N),
            e: decoded(E),
        },
        d: decoded(D),
        p: decoded(P),
        q: decoded(Q),
        dP: decoded(DP),
        dQ: decoded(DQ),
        qInv: decoded(QI),
    })
    .expect("the example key is a valid RSA key");
    let mut signature = vec![0_u8; key.public().modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        signed.as_bytes(),
        &mut signature,
    )
    .expect("the example key signs");
    format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature))
}

/// The claims of a valid token for `client` and `nonce`.
#[cfg(test)]
pub(crate) fn valid_claims(client: &str, nonce: &str) -> Value {
    let now = NOW_MILLIS / 1_000;
    json!({
        "iss": ISSUER,
        "aud": client,
        "sub": "user-123",
        "email": "owner@example.com",
        "nonce": nonce,
        "iat": now - 10,
        "exp": now + 3_600,
        "sid": "session-1",
    })
}
