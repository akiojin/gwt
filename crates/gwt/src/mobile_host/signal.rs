//! The fixed wire of the signals a browser relays to this Host through the
//! coordinator (Issue #6476). The coordinator relays each payload opaquely; the
//! Host accepts only the WebRTC offer, its trickled ICE candidates and their
//! end, in that order and within fixed bounds, before anything else may parse
//! them. A connection that breaks the wire once accepts nothing more: its
//! caller closes it rather than report the signal.
//!
//! The payload is a JSON object: `{"type":"offer","sdp"}`,
//! `{"type":"candidate","candidate","sdp_mid"?,"sdp_m_line_index"?}` or
//! `{"type":"end_of_candidates"}`; unknown types and fields are rejected. SDP
//! content is not interpreted here.

use serde::Deserialize;

/// Bound of one relayed payload, and so of the SDP it carries.
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
/// Trickled candidates accepted per connection.
pub const MAX_CANDIDATES: usize = 128;
/// Bound of one candidate line.
pub const MAX_CANDIDATE_BYTES: usize = 1024;
/// Bound of a candidate's media stream identification tag.
pub const MAX_SDP_MID_BYTES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserSignal {
    Offer {
        sdp: String,
    },
    Candidate {
        candidate: String,
        #[serde(default)]
        sdp_mid: Option<String>,
        #[serde(default)]
        sdp_m_line_index: Option<u16>,
    },
    EndOfCandidates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SignalError {
    #[error("the browser signal exceeds its size bound")]
    TooLarge,
    #[error("the browser signal is not a known, well-formed message")]
    Invalid,
    #[error("the browser signal arrived out of order")]
    OutOfOrder,
    #[error("the browser sent more ICE candidates than one connection may")]
    TooManyCandidates,
    #[error("the browser connection was already rejected")]
    Rejected,
}

/// The signal state of one browser connection.
#[derive(Debug, Default)]
pub struct SignalSequence {
    offered: bool,
    candidates: usize,
    ended: bool,
    rejected: bool,
}

impl SignalSequence {
    /// Parse the next relayed payload. Any failure rejects the connection, so
    /// every later payload is [`SignalError::Rejected`].
    pub fn accept(&mut self, payload: &str) -> Result<BrowserSignal, SignalError> {
        if self.rejected {
            return Err(SignalError::Rejected);
        }
        let accepted = self.next(payload);
        if accepted.is_err() {
            self.rejected = true;
        }
        accepted
    }

    /// Reject the connection for a reason outside the wire.
    pub fn reject(&mut self) {
        self.rejected = true;
    }

    fn next(&mut self, payload: &str) -> Result<BrowserSignal, SignalError> {
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(SignalError::TooLarge);
        }
        let signal: BrowserSignal =
            serde_json::from_str(payload).map_err(|_| SignalError::Invalid)?;
        match &signal {
            BrowserSignal::Offer { sdp } if sdp.is_empty() => return Err(SignalError::Invalid),
            BrowserSignal::Candidate {
                candidate, sdp_mid, ..
            } if candidate.is_empty()
                || candidate.len() > MAX_CANDIDATE_BYTES
                || sdp_mid
                    .as_ref()
                    .is_some_and(|mid| mid.len() > MAX_SDP_MID_BYTES) =>
            {
                return Err(SignalError::Invalid)
            }
            _ => {}
        }
        if self.ended {
            return Err(SignalError::OutOfOrder);
        }
        match signal {
            BrowserSignal::Offer { .. } if self.offered => return Err(SignalError::OutOfOrder),
            BrowserSignal::Offer { .. } => self.offered = true,
            _ if !self.offered => return Err(SignalError::OutOfOrder),
            BrowserSignal::Candidate { .. } if self.candidates >= MAX_CANDIDATES => {
                return Err(SignalError::TooManyCandidates)
            }
            BrowserSignal::Candidate { .. } => self.candidates += 1,
            BrowserSignal::EndOfCandidates => self.ended = true,
        }
        Ok(signal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn offer() -> String {
        json!({"type":"offer","sdp":"v=0"}).to_string()
    }

    fn candidate(text: &str) -> String {
        json!({"type":"candidate","candidate":text,"sdp_mid":"0","sdp_m_line_index":0}).to_string()
    }

    #[test]
    fn accepts_offer_then_candidates_then_end() {
        let mut sequence = SignalSequence::default();
        assert_eq!(
            sequence.accept(&offer()),
            Ok(BrowserSignal::Offer {
                sdp: "v=0".to_string()
            })
        );
        assert_eq!(
            sequence.accept(&candidate("candidate:1 1 udp 1 192.0.2.1 9 typ host")),
            Ok(BrowserSignal::Candidate {
                candidate: "candidate:1 1 udp 1 192.0.2.1 9 typ host".to_string(),
                sdp_mid: Some("0".to_string()),
                sdp_m_line_index: Some(0),
            })
        );
        let bare = json!({"type":"candidate","candidate":"c"}).to_string();
        assert_eq!(
            sequence.accept(&bare),
            Ok(BrowserSignal::Candidate {
                candidate: "c".to_string(),
                sdp_mid: None,
                sdp_m_line_index: None,
            })
        );
        let end = json!({"type":"end_of_candidates"}).to_string();
        assert_eq!(sequence.accept(&end), Ok(BrowserSignal::EndOfCandidates));
        // Nothing follows the end of candidates.
        assert_eq!(
            sequence.accept(&candidate("c")),
            Err(SignalError::OutOfOrder)
        );
        assert_eq!(sequence.accept(&offer()), Err(SignalError::Rejected));
    }

    #[test]
    fn rejects_unknown_or_malformed_messages_and_stays_rejected() {
        for payload in [
            "offer".to_string(),
            json!({"type":"answer","sdp":"v=0"}).to_string(),
            json!({"type":"snapshot"}).to_string(),
            json!({"type":"offer","sdp":"v=0","project":"p"}).to_string(),
            json!({"type":"offer","sdp":""}).to_string(),
            json!({"type":"offer"}).to_string(),
        ] {
            let mut sequence = SignalSequence::default();
            assert_eq!(
                sequence.accept(&payload),
                Err(SignalError::Invalid),
                "{payload}"
            );
            // A rejected connection accepts nothing more, even a valid offer.
            assert_eq!(sequence.accept(&offer()), Err(SignalError::Rejected));
        }
    }

    #[test]
    fn enforces_order() {
        let mut sequence = SignalSequence::default();
        assert_eq!(
            sequence.accept(&candidate("c")),
            Err(SignalError::OutOfOrder)
        );

        let mut sequence = SignalSequence::default();
        let end = json!({"type":"end_of_candidates"}).to_string();
        assert_eq!(sequence.accept(&end), Err(SignalError::OutOfOrder));

        let mut sequence = SignalSequence::default();
        assert!(sequence.accept(&offer()).is_ok());
        assert_eq!(sequence.accept(&offer()), Err(SignalError::OutOfOrder));
    }

    #[test]
    fn enforces_size_bounds_at_their_edges() {
        let wire_overhead = offer().len() - 3;
        let fits =
            json!({"type":"offer","sdp":"v".repeat(MAX_PAYLOAD_BYTES - wire_overhead)}).to_string();
        assert_eq!(fits.len(), MAX_PAYLOAD_BYTES);
        assert!(SignalSequence::default().accept(&fits).is_ok());
        let over = json!({"type":"offer","sdp":"v".repeat(MAX_PAYLOAD_BYTES - wire_overhead + 1)})
            .to_string();
        assert_eq!(
            SignalSequence::default().accept(&over),
            Err(SignalError::TooLarge)
        );

        let mut sequence = SignalSequence::default();
        assert!(sequence.accept(&offer()).is_ok());
        assert!(sequence
            .accept(&candidate(&"c".repeat(MAX_CANDIDATE_BYTES)))
            .is_ok());
        assert_eq!(
            sequence.accept(&candidate(&"c".repeat(MAX_CANDIDATE_BYTES + 1))),
            Err(SignalError::Invalid)
        );

        let mut sequence = SignalSequence::default();
        assert!(sequence.accept(&offer()).is_ok());
        assert_eq!(sequence.accept(&candidate("")), Err(SignalError::Invalid));

        let mid = |len: usize| {
            json!({"type":"candidate","candidate":"c","sdp_mid":"m".repeat(len)}).to_string()
        };
        let mut sequence = SignalSequence::default();
        assert!(sequence.accept(&offer()).is_ok());
        assert!(sequence.accept(&mid(MAX_SDP_MID_BYTES)).is_ok());
        assert_eq!(
            sequence.accept(&mid(MAX_SDP_MID_BYTES + 1)),
            Err(SignalError::Invalid)
        );
    }

    #[test]
    fn caps_candidates_per_connection() {
        let mut sequence = SignalSequence::default();
        assert!(sequence.accept(&offer()).is_ok());
        for _ in 0..MAX_CANDIDATES {
            assert!(sequence.accept(&candidate("c")).is_ok());
        }
        assert_eq!(
            sequence.accept(&candidate("c")),
            Err(SignalError::TooManyCandidates)
        );
        assert_eq!(sequence.accept(&candidate("c")), Err(SignalError::Rejected));
    }

    #[test]
    fn an_explicit_rejection_ends_the_sequence() {
        let mut sequence = SignalSequence::default();
        assert!(sequence.accept(&offer()).is_ok());
        sequence.reject();
        assert_eq!(sequence.accept(&candidate("c")), Err(SignalError::Rejected));
    }
}
