use std::fmt;
use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

/// Hard limit for one app-server JSONL message.
///
/// The initial bridge exchanges small control messages. Keeping a finite cap
/// prevents a broken or hostile child from growing the desktop process without
/// bound. Streaming command output arrives as separate notifications.
pub(crate) const MAX_JSONL_LINE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug)]
pub(crate) enum JsonLineError {
    Io(io::Error),
    TooLong { limit: usize },
    InvalidJson(serde_json::Error),
}

impl fmt::Display for JsonLineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "app-server stream error: {error}"),
            Self::TooLong { limit } => {
                write!(formatter, "app-server message exceeded {limit} bytes")
            }
            Self::InvalidJson(error) => {
                write!(formatter, "app-server sent invalid JSON: {error}")
            }
        }
    }
}

impl std::error::Error for JsonLineError {}

impl From<io::Error> for JsonLineError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Read one non-empty newline-delimited JSON value without allocating beyond
/// [`MAX_JSONL_LINE_BYTES`]. `BufRead::read_line` cannot enforce a cap while it
/// grows, so framing is implemented over `fill_buf`/`consume`.
pub(crate) fn read_json_line<R: BufRead>(
    reader: &mut R,
    line: &mut Vec<u8>,
) -> Result<Option<Value>, JsonLineError> {
    loop {
        line.clear();
        let mut reached_eof = false;

        loop {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                reached_eof = true;
                break;
            }

            let newline = available.iter().position(|byte| *byte == b'\n');
            let take = newline.map_or(available.len(), |index| index + 1);
            if line.len().saturating_add(take) > MAX_JSONL_LINE_BYTES {
                reader.consume(take);
                return Err(JsonLineError::TooLong {
                    limit: MAX_JSONL_LINE_BYTES,
                });
            }
            line.extend_from_slice(&available[..take]);
            reader.consume(take);
            if newline.is_some() {
                break;
            }
        }

        while matches!(line.last(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        if line.is_empty() {
            if reached_eof {
                return Ok(None);
            }
            continue;
        }

        return serde_json::from_slice(line)
            .map(Some)
            .map_err(JsonLineError::InvalidJson);
    }
}

pub(crate) fn write_value<W: Write>(writer: &mut W, value: &Value) -> Result<(), String> {
    let encoded = encode_value(value)?;
    writer
        .write_all(&encoded)
        .and_then(|_| writer.write_all(b"\n"))
        .and_then(|_| writer.flush())
        .map_err(|error| format!("failed to write app-server message: {error}"))
}

pub(crate) fn validate_value_size(value: &Value) -> Result<(), String> {
    encode_value(value).map(|_| ())
}

fn encode_value(value: &Value) -> Result<Vec<u8>, String> {
    let encoded = serde_json::to_vec(value)
        .map_err(|error| format!("failed to encode app-server message: {error}"))?;
    if encoded.len().saturating_add(1) > MAX_JSONL_LINE_BYTES {
        return Err(format!(
            "app-server message exceeded {} bytes",
            MAX_JSONL_LINE_BYTES
        ));
    }
    Ok(encoded)
}

pub(crate) fn request(id: u64, method: &str, params: Value) -> Value {
    // app-server intentionally omits the JSON-RPC 2.0 header on the wire.
    json!({ "id": id, "method": method, "params": params })
}

pub(crate) fn notification(method: &str) -> Value {
    json!({ "method": method })
}

pub(crate) fn method_not_found(id: Value, method: &str) -> Value {
    error_response(
        id,
        -32601,
        format!("SchoolX Code does not support server request `{method}`"),
    )
}

pub(crate) fn response(id: Value, result: Value) -> Value {
    json!({ "id": id, "result": result })
}

pub(crate) fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "id": id,
        "error": {
            "code": code,
            "message": message.into()
        }
    })
}

#[derive(Debug)]
pub(crate) enum IncomingMessage {
    Response {
        id: Value,
        result: Option<Value>,
        error: Option<RpcError>,
    },
    Request {
        id: Value,
        method: String,
        params: Option<Value>,
    },
    Notification {
        method: String,
        params: Option<Value>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct RpcError {
    pub code: i64,
    pub message: String,
}

pub(crate) fn classify(value: Value) -> Result<IncomingMessage, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "app-server message must be an object".to_string())?;
    let id = object.get("id").cloned();
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);
    let params = object.get("params").cloned();

    match (id, method) {
        (Some(id), Some(method)) => Ok(IncomingMessage::Request { id, method, params }),
        (None, Some(method)) => Ok(IncomingMessage::Notification { method, params }),
        (Some(id), None) => {
            let result = object.get("result").cloned();
            let error = object.get("error").map(parse_rpc_error).transpose()?;
            if result.is_none() && error.is_none() {
                return Err("app-server response has neither result nor error".to_string());
            }
            Ok(IncomingMessage::Response { id, result, error })
        }
        (None, None) => Err("app-server message has neither id nor method".to_string()),
    }
}

fn parse_rpc_error(value: &Value) -> Result<RpcError, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "app-server response error must be an object".to_string())?;
    let code = object
        .get("code")
        .and_then(Value::as_i64)
        .ok_or_else(|| "app-server response error is missing code".to_string())?;
    let message = object
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(|| "app-server response error is missing message".to_string())?;
    Ok(RpcError {
        code,
        message: message.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use serde_json::json;

    use super::*;

    #[test]
    fn reads_lf_and_crlf_messages_and_skips_blank_lines() {
        let input = b"\n{\"method\":\"one\"}\r\n{\"id\":2,\"result\":{}}\n";
        let mut reader = BufReader::new(Cursor::new(input));
        let mut line = Vec::new();

        assert_eq!(
            read_json_line(&mut reader, &mut line).ok().flatten(),
            Some(json!({ "method": "one" }))
        );
        assert_eq!(
            read_json_line(&mut reader, &mut line).ok().flatten(),
            Some(json!({ "id": 2, "result": {} }))
        );
        assert!(read_json_line(&mut reader, &mut line)
            .ok()
            .flatten()
            .is_none());
    }

    #[test]
    fn rejects_an_oversized_line_before_unbounded_growth() {
        let input = vec![b'x'; MAX_JSONL_LINE_BYTES + 1];
        let mut reader = BufReader::new(Cursor::new(input));
        let mut line = Vec::new();

        let error = read_json_line(&mut reader, &mut line).err();
        assert!(matches!(error, Some(JsonLineError::TooLong { .. })));
        assert!(line.len() <= MAX_JSONL_LINE_BYTES);
    }

    #[test]
    fn request_omits_jsonrpc_header() {
        let value = request(7, "initialize", json!({ "clientInfo": {} }));
        assert_eq!(value.get("id"), Some(&json!(7)));
        assert!(value.get("jsonrpc").is_none());
    }

    #[test]
    fn rejects_an_oversized_outbound_message() {
        let value = json!({ "payload": "x".repeat(MAX_JSONL_LINE_BYTES) });
        assert!(validate_value_size(&value).is_err());
    }

    #[test]
    fn classifies_server_requests_separately_from_notifications() {
        let request_message = classify(json!({
            "id": "approval-1",
            "method": "item/commandExecution/requestApproval",
            "params": { "turnId": "turn-1" }
        }));
        assert!(matches!(
            request_message,
            Ok(IncomingMessage::Request { method, .. })
                if method == "item/commandExecution/requestApproval"
        ));

        let notification_message = classify(json!({
            "method": "thread/started",
            "params": { "thread": {} }
        }));
        assert!(matches!(
            notification_message,
            Ok(IncomingMessage::Notification { method, .. }) if method == "thread/started"
        ));
    }

    #[test]
    fn parses_error_responses() {
        let message = classify(json!({
            "id": 4,
            "error": { "code": -32602, "message": "bad params" }
        }));
        assert!(matches!(
            message,
            Ok(IncomingMessage::Response {
                error: Some(RpcError { code: -32602, message }),
                ..
            }) if message == "bad params"
        ));
    }
}
