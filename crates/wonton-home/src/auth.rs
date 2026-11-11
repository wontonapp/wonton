use crate::{
    service::{ApiError, App},
    store::{hash, now},
};
use axum::http::{HeaderMap, Method, Uri};
use iroh::{EndpointId, Signature};

pub fn canonical(method: &str, path: &str, timestamp: &str, nonce: &str, body: &[u8]) -> String {
    format!(
        "wonton-home-v1\n{method}\n{path}\n{timestamp}\n{nonce}\n{}",
        hash(body)
    )
}

fn header<'a>(headers: &'a HeaderMap, key: &str) -> Result<&'a str, ApiError> {
    let mut values = headers.get_all(key).iter();
    let value = values
        .next()
        .and_then(|v| v.to_str().ok())
        .ok_or_else(ApiError::unauthorized)?;
    if values.next().is_some() {
        return Err(ApiError::unauthorized());
    }
    Ok(value)
}

fn decode<const N: usize>(text: &str) -> Result<[u8; N], ApiError> {
    if text.len() != N * 2
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(ApiError::unauthorized());
    }
    let mut bytes = [0; N];
    let (pairs, _) = text.as_bytes().as_chunks::<2>();
    for (byte, pair) in bytes.iter_mut().zip(pairs) {
        let digit = |b: u8| if b <= b'9' { b - b'0' } else { b - b'a' + 10 };
        *byte = digit(pair[0]) * 16 + digit(pair[1]);
    }
    Ok(bytes)
}

pub async fn authenticate(
    app: &App,
    method: Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<String, ApiError> {
    if uri.query().is_some() {
        return Err(ApiError::bad("query strings are forbidden"));
    }
    let endpoint = header(headers, "x-wonton-endpoint")?;
    let key: EndpointId = endpoint.parse().map_err(|_| ApiError::unauthorized())?;
    if key.to_string() != endpoint {
        return Err(ApiError::unauthorized());
    }
    let timestamp = header(headers, "x-wonton-timestamp")?;
    let seconds: u64 = timestamp.parse().map_err(|_| ApiError::unauthorized())?;
    if seconds.to_string() != timestamp || now().abs_diff(seconds) > 300 {
        return Err(ApiError::unauthorized());
    }
    let nonce = header(headers, "x-wonton-nonce")?;
    decode::<16>(nonce)?;
    let signature = Signature::from_bytes(&decode::<64>(header(headers, "x-wonton-signature")?)?);
    key.verify(
        canonical(method.as_str(), uri.path(), timestamp, nonce, body).as_bytes(),
        &signature,
    )
    .map_err(|_| ApiError::unauthorized())?;
    let mut store = app.store.lock().await;
    let nonce_key = format!("{endpoint}:{nonce}");
    let clock = now();
    store
        .data
        .nonces
        .retain(|_, expiration| *expiration >= clock);
    if store.data.nonces.contains_key(&nonce_key) {
        return Err(ApiError::unauthorized());
    }
    store
        .data
        .nonces
        .insert(nonce_key, seconds.saturating_add(301));
    store.save()?;
    Ok(endpoint.to_owned())
}
