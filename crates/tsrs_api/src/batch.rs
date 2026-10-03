// session.go handleBatchRequests / paginateBatchResponses / handleBatchRequest and proto.go
// BatchRequestsResponse.MarshalJSONTo. Responses are kept as encoded JSON text so pages are cut on exact
// byte lengths like Go.

use tsrs_core::json::{self, Value};

use crate::handler::{ApiError, ApiResult, Response};
use crate::session::Session;
use crate::wire::Params;

/// Go `DefaultMaxResponseBytesPerPage`.
pub const DEFAULT_MAX_RESPONSE_BYTES_PER_PAGE: usize = 300_000_000;

const SOURCE_FILE_RESPONSE_METHODS: &[&str] =
    &["createSourceFile", "createSourceFileFromFile", "getSourceFile", "getCachedSourceFile", "getConfigSourceFile", "typeToTypeNode", "signatureToSignatureDeclaration"];

pub fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

fn encode_batch_response(method: &str, result: &str, error: Option<&str>) -> String {
    let mut out = String::from("{\"method\":");
    out.push_str(&json::marshal_string(method));
    out.push_str(",\"result\":");
    out.push_str(result);
    if let Some(e) = error {
        if !e.is_empty() {
            out.push_str(",\"error\":");
            out.push_str(&json::marshal_string(e));
        }
    }
    out.push('}');
    out
}

impl Session {
    /// Go `handleBatchRequest`.
    fn handle_batch_request(&self, request: &Value) -> String {
        let p = Params(request);
        let method = match p.get("method") {
            Value::String(m) => m.clone(),
            _ => String::new(),
        };
        if method == "batchRequests" {
            return encode_batch_response(&method, "null", Some("api: invalid request: batchRequests cannot be nested"));
        }
        let params = match p.get("params") {
            Value::Null => String::new(),
            v => json::marshal(v).unwrap_or_default(),
        };
        // `handle_request` already turns panics into errors.
        match crate::handler::Handler::handle_request(self, &method, params.as_bytes()) {
            Ok(Response::Json(text)) => encode_batch_response(&method, &text, None),
            Ok(Response::Binary(data)) => {
                if SOURCE_FILE_RESPONSE_METHODS.contains(&method.as_str()) {
                    if data.is_empty() {
                        encode_batch_response(&method, "null", None)
                    } else {
                        let r = format!("{{\"data\":{}}}", json::marshal_string(&base64_encode(&data)));
                        encode_batch_response(&method, &r, None)
                    }
                } else {
                    // Go marshals other RawBinary values as a JSON byte slice (base64 string).
                    encode_batch_response(&method, &json::marshal_string(&base64_encode(&data)), None)
                }
            }
            Err(e) => encode_batch_response(&method, "null", Some(&e.to_string())),
        }
    }

    pub(crate) fn handle_batch_requests(&self, p: Params) -> ApiResult<String> {
        let max = match p.opt_u64("maxResponseBytesPerPage")? {
            Some(n) if n > 0 => n as usize,
            _ => DEFAULT_MAX_RESPONSE_BYTES_PER_PAGE,
        };
        let token = p.str("continuationToken")?;
        let encoded = if !token.is_empty() {
            self.batch_pages.lock().unwrap().remove(token).ok_or_else(|| ApiError::client("invalid batch continuation token"))?
        } else {
            p.array("requests")?.iter().map(|r| self.handle_batch_request(r)).collect()
        };
        Ok(self.paginate_batch_responses(encoded, max))
    }

    /// Go `paginateBatchResponses`.
    fn paginate_batch_responses(&self, encoded: Vec<String>, max: usize) -> String {
        let mut encoded_length = "{\"responses\":[]}".len();
        let mut page_length = 0;
        for e in &encoded {
            let additional = e.len() + usize::from(page_length > 0);
            if page_length > 0 && encoded_length + additional > max {
                break;
            }
            encoded_length += additional;
            page_length += 1;
        }
        let mut token = None;
        if page_length != encoded.len() {
            let t = format!("{}-{}", self.id(), self.next_batch_page_id());
            let continuation_length = ",\"continuationToken\":\"\"".len() + t.len();
            while page_length > 1 && encoded_length + continuation_length > max {
                encoded_length -= encoded[page_length - 1].len() + 1;
                page_length -= 1;
            }
            token = Some(t);
        }
        let mut encoded = encoded;
        let rest = encoded.split_off(page_length);
        let mut out = String::from("{\"responses\":[");
        out.push_str(&encoded.join(","));
        out.push(']');
        if let Some(t) = token {
            out.push_str(",\"continuationToken\":");
            out.push_str(&json::marshal_string(&t));
            self.batch_pages.lock().unwrap().insert(t, rest);
        }
        out.push('}');
        out
    }
}
