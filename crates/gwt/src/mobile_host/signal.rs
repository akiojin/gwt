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
//!
//! The Host answers on the same connection state with [`HostSignal`]s: one
//! answer after the browser's offer, then its own trickled candidates (within
//! the same bounds) and their end. A Host signal that breaks this wire rejects
//! the whole connection, in both directions.

use serde::{Deserialize, Serialize};

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
    /// A struct variant so that `deny_unknown_fields` rejects extra fields.
    EndOfCandidates {},
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

/// A signal the Host sends to a browser, encoded as `{"type":"answer","sdp"}`,
/// `{"type":"candidate","candidate","sdp_mid"?,"sdp_m_line_index"?}` or
/// `{"type":"end_of_candidates"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostSignal {
    Answer {
        sdp: String,
    },
    Candidate {
        candidate: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        sdp_mid: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sdp_m_line_index: Option<u16>,
    },
    EndOfCandidates {},
}

/// The signal state of one browser connection, in both directions.
#[derive(Debug, Default)]
pub struct SignalSequence {
    offered: bool,
    candidates: usize,
    ended: bool,
    answered: bool,
    host_candidates: usize,
    host_ended: bool,
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

    /// Encode the Host's next signal. Any failure rejects the connection, so
    /// every later payload, in either direction, is [`SignalError::Rejected`].
    pub fn send(&mut self, signal: &HostSignal) -> Result<String, SignalError> {
        if self.rejected {
            return Err(SignalError::Rejected);
        }
        let encoded = self.next_host(signal);
        if encoded.is_err() {
            self.rejected = true;
        }
        encoded
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
            BrowserSignal::EndOfCandidates {} => self.ended = true,
        }
        Ok(signal)
    }

    fn next_host(&mut self, signal: &HostSignal) -> Result<String, SignalError> {
        match signal {
            HostSignal::Answer { sdp } if sdp.is_empty() => return Err(SignalError::Invalid),
            HostSignal::Candidate {
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
        let payload = serde_json::to_string(signal).map_err(|_| SignalError::Invalid)?;
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(SignalError::TooLarge);
        }
        if self.host_ended {
            return Err(SignalError::OutOfOrder);
        }
        match signal {
            HostSignal::Answer { .. } if !self.offered || self.answered => {
                return Err(SignalError::OutOfOrder)
            }
            HostSignal::Answer { .. } => self.answered = true,
            _ if !self.answered => return Err(SignalError::OutOfOrder),
            HostSignal::Candidate { .. } if self.host_candidates >= MAX_CANDIDATES => {
                return Err(SignalError::TooManyCandidates)
            }
            HostSignal::Candidate { .. } => self.host_candidates += 1,
            HostSignal::EndOfCandidates {} => self.host_ended = true,
        }
        Ok(payload)
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
        assert_eq!(sequence.accept(&end), Ok(BrowserSignal::EndOfCandidates {}));
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
            json!({"type":"end_of_candidates","x":1}).to_string(),
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

    fn answer(sdp: &str) -> HostSignal {
        HostSignal::Answer {
            sdp: sdp.to_string(),
        }
    }

    fn host_candidate(text: &str) -> HostSignal {
        HostSignal::Candidate {
            candidate: text.to_string(),
            sdp_mid: Some("0".to_string()),
            sdp_m_line_index: Some(0),
        }
    }

    #[test]
    fn sends_answer_then_candidates_then_end_on_the_fixed_wire() {
        let mut sequence = SignalSequence::default();
        assert!(sequence.accept(&offer()).is_ok());
        let sent = |payload: Result<String, SignalError>| -> serde_json::Value {
            serde_json::from_str(&payload.unwrap()).unwrap()
        };
        assert_eq!(
            sent(sequence.send(&answer("v=0"))),
            json!({"type":"answer","sdp":"v=0"})
        );
        assert_eq!(
            sent(sequence.send(&host_candidate("c"))),
            json!({"type":"candidate","candidate":"c","sdp_mid":"0","sdp_m_line_index":0})
        );
        let bare = HostSignal::Candidate {
            candidate: "c".to_string(),
            sdp_mid: None,
            sdp_m_line_index: None,
        };
        assert_eq!(
            sent(sequence.send(&bare)),
            json!({"type":"candidate","candidate":"c"})
        );
        // The browser's candidates still flow after the Host answered.
        assert!(sequence.accept(&candidate("c")).is_ok());
        assert_eq!(
            sent(sequence.send(&HostSignal::EndOfCandidates {})),
            json!({"type":"end_of_candidates"})
        );
        // Nothing follows the Host's end of candidates.
        assert_eq!(
            sequence.send(&host_candidate("c")),
            Err(SignalError::OutOfOrder)
        );
        assert_eq!(sequence.send(&answer("v=0")), Err(SignalError::Rejected));
    }

    #[test]
    fn host_sends_in_order() {
        // No answer before the browser's offer.
        let mut sequence = SignalSequence::default();
        assert_eq!(sequence.send(&answer("v=0")), Err(SignalError::OutOfOrder));

        let mut sequence = SignalSequence::default();
        assert!(sequence.accept(&offer()).is_ok());
        assert_eq!(
            sequence.send(&host_candidate("c")),
            Err(SignalError::OutOfOrder)
        );

        let mut sequence = SignalSequence::default();
        assert!(sequence.accept(&offer()).is_ok());
        assert_eq!(
            sequence.send(&HostSignal::EndOfCandidates {}),
            Err(SignalError::OutOfOrder)
        );

        let mut sequence = SignalSequence::default();
        assert!(sequence.accept(&offer()).is_ok());
        assert!(sequence.send(&answer("v=0")).is_ok());
        assert_eq!(sequence.send(&answer("v=0")), Err(SignalError::OutOfOrder));
    }

    #[test]
    fn host_sends_within_bounds() {
        let offered = || {
            let mut sequence = SignalSequence::default();
            assert!(sequence.accept(&offer()).is_ok());
            sequence
        };
        let overhead = json!({"type":"answer","sdp":""}).to_string().len();
        let fits = offered().send(&answer(&"v".repeat(MAX_PAYLOAD_BYTES - overhead)));
        assert_eq!(fits.map(|payload| payload.len()), Ok(MAX_PAYLOAD_BYTES));
        assert_eq!(
            offered().send(&answer(&"v".repeat(MAX_PAYLOAD_BYTES - overhead + 1))),
            Err(SignalError::TooLarge)
        );
        assert_eq!(offered().send(&answer("")), Err(SignalError::Invalid));

        let answered = || {
            let mut sequence = offered();
            assert!(sequence.send(&answer("v=0")).is_ok());
            sequence
        };
        assert!(answered()
            .send(&host_candidate(&"c".repeat(MAX_CANDIDATE_BYTES)))
            .is_ok());
        assert_eq!(
            answered().send(&host_candidate(&"c".repeat(MAX_CANDIDATE_BYTES + 1))),
            Err(SignalError::Invalid)
        );
        assert_eq!(
            answered().send(&host_candidate("")),
            Err(SignalError::Invalid)
        );
        let mid = |len: usize| HostSignal::Candidate {
            candidate: "c".to_string(),
            sdp_mid: Some("m".repeat(len)),
            sdp_m_line_index: None,
        };
        assert!(answered().send(&mid(MAX_SDP_MID_BYTES)).is_ok());
        assert_eq!(
            answered().send(&mid(MAX_SDP_MID_BYTES + 1)),
            Err(SignalError::Invalid)
        );

        let mut sequence = answered();
        for _ in 0..MAX_CANDIDATES {
            assert!(sequence.send(&host_candidate("c")).is_ok());
        }
        assert_eq!(
            sequence.send(&host_candidate("c")),
            Err(SignalError::TooManyCandidates)
        );
    }

    #[test]
    fn a_host_violation_rejects_the_whole_connection() {
        let mut sequence = SignalSequence::default();
        assert!(sequence.accept(&offer()).is_ok());
        assert_eq!(
            sequence.send(&host_candidate("c")),
            Err(SignalError::OutOfOrder)
        );
        assert_eq!(sequence.accept(&candidate("c")), Err(SignalError::Rejected));
        assert_eq!(sequence.send(&answer("v=0")), Err(SignalError::Rejected));

        // And a browser violation stops the Host's signals too.
        let mut sequence = SignalSequence::default();
        assert_eq!(sequence.accept("offer"), Err(SignalError::Invalid));
        assert_eq!(sequence.send(&answer("v=0")), Err(SignalError::Rejected));
    }
}
