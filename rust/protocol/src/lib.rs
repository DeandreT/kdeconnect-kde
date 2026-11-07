// SPDX-FileCopyrightText: 2026 Deandre Thomas
// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const MAX_PACKET_BYTES: usize = 1024 * 1024;

/// A wire envelope only; parsing does not establish device identity or trust.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Packet {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: String,
    pub body: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_size: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_transfer_info: Option<Map<String, Value>>,
}

impl Packet {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_PACKET_BYTES, "packet exceeds size limit");
        let packet: Self = serde_json::from_slice(bytes)?;
        packet.validate()?;
        Ok(packet)
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut bytes = serde_json::to_vec(self)?;
        bytes.push(b'\n');
        ensure!(bytes.len() <= MAX_PACKET_BYTES, "packet exceeds size limit");
        Ok(bytes)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.kind.starts_with("kdeconnect.")
                && self.kind.len() > "kdeconnect.".len()
                && self.kind.len() <= 256
                && self
                    .kind
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._".contains(&b)),
            "invalid packet type"
        );
        if let Some(size) = self.payload_size {
            ensure!(size >= -1, "invalid payload size");
            ensure!(
                self.payload_transfer_info.is_some(),
                "missing payload transfer information"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_round_trip() -> Result<()> {
        let raw = br#"{"id":1,"type":"kdeconnect.ping","body":{}}"#;
        let packet = Packet::decode(raw)?;
        assert_eq!(Packet::decode(&packet.encode()?)?, packet);
        Ok(())
    }

    #[test]
    fn rejects_invalid_envelopes() {
        for raw in [
            r#"{"id":1,"type":"kdeconnect.","body":{}}"#,
            r#"{"id":1,"type":"kdeconnect.ping","body":[]}"#,
            r#"{"id":1,"type":"kdeconnect.ping","body":{},"payloadSize":-2}"#,
            r#"{"id":1,"type":"kdeconnect.ping","body":{},"payloadSize":5}"#,
            r#"{"id":1,"type":"kdeconnect.ping","body":{}}garbage"#,
        ] {
            assert!(Packet::decode(raw.as_bytes()).is_err());
        }
        assert!(Packet::decode(&vec![b' '; MAX_PACKET_BYTES + 1]).is_err());
    }
}
