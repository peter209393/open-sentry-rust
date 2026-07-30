use std::io::Read;

use flate2::read::{GzDecoder, ZlibDecoder};
use serde::Deserialize;
use serde_json::Value;

use crate::error::{AppError, Result};

pub const MAX_COMPRESSED_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_DECOMPRESSED_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_ITEMS: usize = 100;
pub const MAX_EVENT_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub struct EnvelopeItem {
    pub item_type: String,
    pub payload: Vec<u8>,
}

#[derive(Deserialize)]
struct ItemHeader {
    #[serde(rename = "type")]
    item_type: String,
    length: Option<usize>,
}

pub fn decode(content_encoding: Option<&str>, input: &[u8]) -> Result<Vec<u8>> {
    match content_encoding.unwrap_or("identity") {
        "identity" => ensure_size(input.to_vec()),
        "gzip" => read_limited(GzDecoder::new(input)),
        "deflate" | "zlib" => read_limited(ZlibDecoder::new(input)),
        other => Err(AppError::BadRequest(format!(
            "unsupported content encoding: {other}"
        ))),
    }
}

fn read_limited(reader: impl Read) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    reader
        .take((MAX_DECOMPRESSED_BYTES + 1) as u64)
        .read_to_end(&mut output)
        .map_err(|error| AppError::BadRequest(format!("invalid compressed payload: {error}")))?;
    ensure_size(output)
}

fn ensure_size(bytes: Vec<u8>) -> Result<Vec<u8>> {
    if bytes.len() > MAX_DECOMPRESSED_BYTES {
        Err(AppError::PayloadTooLarge(
            "decompressed payload exceeds 20 MiB".into(),
        ))
    } else {
        Ok(bytes)
    }
}

pub fn parse(input: &[u8]) -> Result<Vec<EnvelopeItem>> {
    let (envelope_header, mut cursor) = line(input, 0)?;
    serde_json::from_slice::<Value>(envelope_header)
        .map_err(|error| AppError::BadRequest(format!("invalid envelope header: {error}")))?;

    let mut items = Vec::new();
    while cursor < input.len() {
        if input[cursor] == b'\n' {
            cursor += 1;
            continue;
        }
        if items.len() >= MAX_ITEMS {
            return Err(AppError::PayloadTooLarge(
                "envelope contains more than 100 items".into(),
            ));
        }
        let (header_bytes, after_header) = line(input, cursor)?;
        let header: ItemHeader = serde_json::from_slice(header_bytes)
            .map_err(|error| AppError::BadRequest(format!("invalid item header: {error}")))?;
        cursor = after_header;
        let payload = if let Some(length) = header.length {
            let end = cursor
                .checked_add(length)
                .filter(|end| *end <= input.len())
                .ok_or_else(|| AppError::BadRequest("truncated envelope item".into()))?;
            let payload = input[cursor..end].to_vec();
            cursor = end;
            if cursor < input.len() && input[cursor] == b'\n' {
                cursor += 1;
            }
            payload
        } else {
            let (payload, next) = line(input, cursor)?;
            cursor = next;
            payload.to_vec()
        };
        if matches!(header.item_type.as_str(), "event" | "transaction")
            && payload.len() > MAX_EVENT_BYTES
        {
            return Err(AppError::PayloadTooLarge("event item exceeds 1 MiB".into()));
        }
        items.push(EnvelopeItem {
            item_type: header.item_type,
            payload,
        });
    }
    Ok(items)
}

fn line(input: &[u8], start: usize) -> Result<(&[u8], usize)> {
    match input[start..].iter().position(|byte| *byte == b'\n') {
        Some(relative_end) => {
            let end = start + relative_end;
            Ok((&input[start..end], end + 1))
        }
        // 容忍最后一行无换行结尾（浏览器 Sentry SDK 的 envelope 不带尾换行）
        None => Ok((&input[start..], input.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_length_delimited_item() {
        let input = br#"{"event_id":"abc"}
{"type":"event","length":7}
{"x":1}
"#;
        let items = parse(input).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].payload, br#"{"x":1}"#);
    }

    #[test]
    fn rejects_too_many_items() {
        let mut input = b"{}\n".to_vec();
        for _ in 0..=MAX_ITEMS {
            input.extend_from_slice(b"{\"type\":\"attachment\",\"length\":0}\n\n");
        }
        assert!(matches!(parse(&input), Err(AppError::PayloadTooLarge(_))));
    }
}
