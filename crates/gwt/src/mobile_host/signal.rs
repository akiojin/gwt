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
//!
//! A connection that claimed an invite ([`SignalSequence::claimed`]) may also
//! pair once, after the Host answered (so both DTLS fingerprints are fixed):
//! the browser's `{"type":"pair_request","name","public_key","project"}`, the
//! Host's `{"type":"pair_challenge","request_id","signing_payload"}` and the
//! browser's `{"type":"pair_proof","request_id","signature"}`, in that order.
//! The key (65-byte raw P-256 point) and signature (64-byte `r‖s`) travel as
//! unpadded base64url; the signing payload is the Host's exact JSON text. The
//! end of candidates stops only candidates.
//!
//! A connection that reconnects an approved device
//! ([`SignalSequence::reconnected`]) instead proves that device's key once,
//! after the Host answered: the Host's
//! `{"type":"auth_challenge","challenge_id","signing_payload"}`, then the
//! browser's `{"type":"auth_proof","challenge_id","signature"}`. It never
//! pairs, and a claimed connection is never challenged; a connection of
//! neither origin does neither. No request or proof is checked here beyond its
//! shape: the Host authority verifies them.

use serde::{Deserialize, Serialize};

/// Bound of one relayed payload, and so of the SDP it carries.
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
/// Trickled candidates accepted per connection.
pub const MAX_CANDIDATES: usize = 128;
/// Bound of one candidate line.
pub const MAX_CANDIDATE_BYTES: usize = 1024;
/// Bound of a candidate's media stream identification tag.
pub const MAX_SDP_MID_BYTES: usize = 64;
/// Bound of the device name a pairing request carries.
pub const MAX_PAIR_NAME_BYTES: usize = 256;
/// Bound of the Project a pairing request names.
pub const MAX_PAIR_PROJECT_BYTES: usize = 4096;
/// Bound of the pairing request or device challenge ID the Host sends.
pub const MAX_REQUEST_ID_BYTES: usize = 64;
/// A raw uncompressed P-256 public key.
const PUBLIC_KEY_BYTES: usize = 65;
/// A fixed-size P-256 ECDSA signature (`r‖s`).
const SIGNATURE_BYTES: usize = 64;

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
    PairRequest {
        name: String,
        #[serde(deserialize_with = "base64url")]
        public_key: Vec<u8>,
        project: String,
    },
    PairProof {
        request_id: String,
        #[serde(deserialize_with = "base64url")]
        signature: Vec<u8>,
    },
    AuthProof {
        challenge_id: String,
        #[serde(deserialize_with = "base64url")]
        signature: Vec<u8>,
    },
}

fn base64url<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    use base64::Engine as _;
    let text = String::deserialize(deserializer)?;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text)
        .map_err(serde::de::Error::custom)
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
/// `{"type":"end_of_candidates"}`, a claimed connection's
/// `{"type":"pair_challenge","request_id","signing_payload"}` or a reconnected
/// connection's `{"type":"auth_challenge","challenge_id","signing_payload"}`.
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
    PairChallenge {
        request_id: String,
        signing_payload: String,
    },
    AuthChallenge {
        challenge_id: String,
        signing_payload: String,
    },
}

/// How far a connection's one key proof has gone: a pairing for a claimed
/// connection, a device key proof for a reconnected one.
#[derive(Debug, Default)]
enum Proof {
    /// A connection of unknown origin: neither.
    #[default]
    Refused,
    PairOpen,
    PairRequested,
    PairChallenged(String),
    AuthOpen,
    AuthChallenged(String),
    Done,
}

/// The signal state of one browser connection, in both directions. The
/// default neither pairs nor proves a device key; see [`Self::claimed`] and
/// [`Self::reconnected`].
#[derive(Debug, Default)]
pub struct SignalSequence {
    proof: Proof,
    offered: bool,
    candidates: usize,
    ended: bool,
    answered: bool,
    host_candidates: usize,
    host_ended: bool,
    rejected: bool,
}

impl SignalSequence {
    /// The state of a connection that claimed an invite: it may pair once.
    pub fn claimed() -> Self {
        Self {
            proof: Proof::PairOpen,
            ..Self::default()
        }
    }

    /// The state of a connection that reconnects an approved device: it may
    /// prove that device's key once.
    pub fn reconnected() -> Self {
        Self {
            proof: Proof::AuthOpen,
            ..Self::default()
        }
    }

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
            BrowserSignal::PairRequest {
                name,
                public_key,
                project,
            } if name.is_empty()
                || name.len() > MAX_PAIR_NAME_BYTES
                || project.is_empty()
                || project.len() > MAX_PAIR_PROJECT_BYTES
                || public_key.len() != PUBLIC_KEY_BYTES =>
            {
                return Err(SignalError::Invalid)
            }
            BrowserSignal::PairProof { signature, .. }
            | BrowserSignal::AuthProof { signature, .. }
                if signature.len() != SIGNATURE_BYTES =>
            {
                return Err(SignalError::Invalid)
            }
            _ => {}
        }
        match &signal {
            BrowserSignal::PairRequest { .. } => {
                if !self.answered || !matches!(self.proof, Proof::PairOpen) {
                    return Err(SignalError::OutOfOrder);
                }
                self.proof = Proof::PairRequested;
            }
            BrowserSignal::PairProof { request_id, .. } => match &self.proof {
                Proof::PairChallenged(challenged) if challenged == request_id => {
                    self.proof = Proof::Done
                }
                Proof::PairChallenged(_) => return Err(SignalError::Invalid),
                _ => return Err(SignalError::OutOfOrder),
            },
            BrowserSignal::AuthProof { challenge_id, .. } => match &self.proof {
                Proof::AuthChallenged(challenged) if challenged == challenge_id => {
                    self.proof = Proof::Done
                }
                Proof::AuthChallenged(_) => return Err(SignalError::Invalid),
                _ => return Err(SignalError::OutOfOrder),
            },
            _ if self.ended => return Err(SignalError::OutOfOrder),
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
            HostSignal::PairChallenge {
                request_id: id,
                signing_payload,
            }
            | HostSignal::AuthChallenge {
                challenge_id: id,
                signing_payload,
            } if id.is_empty() || id.len() > MAX_REQUEST_ID_BYTES || signing_payload.is_empty() => {
                return Err(SignalError::Invalid)
            }
            _ => {}
        }
        let payload = serde_json::to_string(signal).map_err(|_| SignalError::Invalid)?;
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(SignalError::TooLarge);
        }
        match signal {
            HostSignal::PairChallenge { request_id, .. } => {
                if !matches!(self.proof, Proof::PairRequested) {
                    return Err(SignalError::OutOfOrder);
                }
                self.proof = Proof::PairChallenged(request_id.clone());
            }
            HostSignal::AuthChallenge { challenge_id, .. } => {
                if !self.answered || !matches!(self.proof, Proof::AuthOpen) {
                    return Err(SignalError::OutOfOrder);
                }
                self.proof = Proof::AuthChallenged(challenge_id.clone());
            }
            _ if self.host_ended => return Err(SignalError::OutOfOrder),
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

    fn b64(bytes: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    fn pair_request(public_key: &[u8]) -> String {
        json!({"type":"pair_request","name":"Phone","public_key":b64(public_key),"project":"p"})
            .to_string()
    }

    fn pair_proof(request_id: &str, signature: &[u8]) -> String {
        json!({"type":"pair_proof","request_id":request_id,"signature":b64(signature)}).to_string()
    }

    fn pair_challenge(request_id: &str) -> HostSignal {
        HostSignal::PairChallenge {
            request_id: request_id.to_string(),
            signing_payload: r#"["gwt-mobile-pair-v1"]"#.to_string(),
        }
    }

    /// A claimed connection after the Host answered: both fingerprints are fixed.
    fn answered_claim() -> SignalSequence {
        let mut sequence = SignalSequence::claimed();
        assert!(sequence.accept(&offer()).is_ok());
        assert!(sequence.send(&answer("v=0")).is_ok());
        sequence
    }

    #[test]
    fn a_claimed_connection_pairs_once_in_order_on_the_fixed_wire() {
        let mut sequence = answered_claim();
        // The end of candidates stops only candidates, never the pairing.
        let end = json!({"type":"end_of_candidates"}).to_string();
        assert!(sequence.accept(&end).is_ok());
        assert!(sequence.send(&HostSignal::EndOfCandidates {}).is_ok());
        assert_eq!(
            sequence.accept(&pair_request(&[4; 65])),
            Ok(BrowserSignal::PairRequest {
                name: "Phone".to_string(),
                public_key: vec![4; 65],
                project: "p".to_string(),
            })
        );
        let sent: serde_json::Value =
            serde_json::from_str(&sequence.send(&pair_challenge("r1")).unwrap()).unwrap();
        assert_eq!(
            sent,
            json!({"type":"pair_challenge","request_id":"r1",
                   "signing_payload":r#"["gwt-mobile-pair-v1"]"#})
        );
        assert_eq!(
            sequence.accept(&pair_proof("r1", &[7; 64])),
            Ok(BrowserSignal::PairProof {
                request_id: "r1".to_string(),
                signature: vec![7; 64],
            })
        );
        // One proof per connection.
        assert_eq!(
            sequence.accept(&pair_proof("r1", &[7; 64])),
            Err(SignalError::OutOfOrder)
        );
    }

    #[test]
    fn pairing_follows_its_order() {
        // No pairing request before the Host answered.
        let mut sequence = SignalSequence::claimed();
        assert!(sequence.accept(&offer()).is_ok());
        assert_eq!(
            sequence.accept(&pair_request(&[4; 65])),
            Err(SignalError::OutOfOrder)
        );

        // One pairing request per connection.
        let mut sequence = answered_claim();
        assert!(sequence.accept(&pair_request(&[4; 65])).is_ok());
        assert_eq!(
            sequence.accept(&pair_request(&[4; 65])),
            Err(SignalError::OutOfOrder)
        );

        // No challenge before the request, and one challenge per request.
        let mut sequence = answered_claim();
        assert_eq!(
            sequence.send(&pair_challenge("r1")),
            Err(SignalError::OutOfOrder)
        );
        let mut sequence = answered_claim();
        assert!(sequence.accept(&pair_request(&[4; 65])).is_ok());
        assert!(sequence.send(&pair_challenge("r1")).is_ok());
        assert_eq!(
            sequence.send(&pair_challenge("r1")),
            Err(SignalError::OutOfOrder)
        );

        // No proof before the challenge.
        let mut sequence = answered_claim();
        assert!(sequence.accept(&pair_request(&[4; 65])).is_ok());
        assert_eq!(
            sequence.accept(&pair_proof("r1", &[7; 64])),
            Err(SignalError::OutOfOrder)
        );

        // A reconnected (not claimed) connection never pairs, nor does one of
        // unknown origin.
        for mut sequence in [SignalSequence::reconnected(), SignalSequence::default()] {
            assert!(sequence.accept(&offer()).is_ok());
            assert!(sequence.send(&answer("v=0")).is_ok());
            assert_eq!(
                sequence.accept(&pair_request(&[4; 65])),
                Err(SignalError::OutOfOrder)
            );
            assert_eq!(sequence.accept(&offer()), Err(SignalError::Rejected));
        }
    }

    fn auth_challenge(challenge_id: &str) -> HostSignal {
        HostSignal::AuthChallenge {
            challenge_id: challenge_id.to_string(),
            signing_payload: r#"["gwt-mobile-auth-v2"]"#.to_string(),
        }
    }

    fn auth_proof(challenge_id: &str, signature: &[u8]) -> String {
        json!({"type":"auth_proof","challenge_id":challenge_id,"signature":b64(signature)})
            .to_string()
    }

    /// A reconnected connection after the Host answered.
    fn answered_reconnect() -> SignalSequence {
        let mut sequence = SignalSequence::reconnected();
        assert!(sequence.accept(&offer()).is_ok());
        assert!(sequence.send(&answer("v=0")).is_ok());
        sequence
    }

    #[test]
    fn a_reconnected_connection_proves_its_key_once_in_order_on_the_fixed_wire() {
        let mut sequence = answered_reconnect();
        // The end of candidates stops only candidates, never the proof.
        let end = json!({"type":"end_of_candidates"}).to_string();
        assert!(sequence.accept(&end).is_ok());
        assert!(sequence.send(&HostSignal::EndOfCandidates {}).is_ok());
        let sent: serde_json::Value =
            serde_json::from_str(&sequence.send(&auth_challenge("c1")).unwrap()).unwrap();
        assert_eq!(
            sent,
            json!({"type":"auth_challenge","challenge_id":"c1",
                   "signing_payload":r#"["gwt-mobile-auth-v2"]"#})
        );
        assert_eq!(
            sequence.accept(&auth_proof("c1", &[7; 64])),
            Ok(BrowserSignal::AuthProof {
                challenge_id: "c1".to_string(),
                signature: vec![7; 64],
            })
        );
        // One proof per connection.
        assert_eq!(
            sequence.accept(&auth_proof("c1", &[7; 64])),
            Err(SignalError::OutOfOrder)
        );
    }

    #[test]
    fn key_proof_follows_its_order() {
        // No challenge before the Host answered.
        let mut sequence = SignalSequence::reconnected();
        assert!(sequence.accept(&offer()).is_ok());
        assert_eq!(
            sequence.send(&auth_challenge("c1")),
            Err(SignalError::OutOfOrder)
        );

        // One challenge per connection.
        let mut sequence = answered_reconnect();
        assert!(sequence.send(&auth_challenge("c1")).is_ok());
        assert_eq!(
            sequence.send(&auth_challenge("c1")),
            Err(SignalError::OutOfOrder)
        );

        // No proof before the challenge.
        let mut sequence = answered_reconnect();
        assert_eq!(
            sequence.accept(&auth_proof("c1", &[7; 64])),
            Err(SignalError::OutOfOrder)
        );

        // A claimed connection, or one of unknown origin, is never challenged
        // and never proves a device key.
        for make in [SignalSequence::claimed, SignalSequence::default] {
            let mut sequence = make();
            assert!(sequence.accept(&offer()).is_ok());
            assert!(sequence.send(&answer("v=0")).is_ok());
            assert_eq!(
                sequence.send(&auth_challenge("c1")),
                Err(SignalError::OutOfOrder)
            );
            let mut sequence = make();
            assert!(sequence.accept(&offer()).is_ok());
            assert!(sequence.send(&answer("v=0")).is_ok());
            assert_eq!(
                sequence.accept(&auth_proof("c1", &[7; 64])),
                Err(SignalError::OutOfOrder)
            );
        }
    }

    #[test]
    fn key_proof_messages_are_bounded() {
        let challenged = || {
            let mut sequence = answered_reconnect();
            assert!(sequence.send(&auth_challenge("c1")).is_ok());
            sequence
        };
        for payload in [
            auth_proof("c2", &[7; 64]),
            auth_proof("c1", &[7; 63]),
            auth_proof("c1", &[7; 65]),
            json!({"type":"auth_proof","challenge_id":"c1","signature":"!!"}).to_string(),
            json!({"type":"auth_proof","challenge_id":"c1","signature":b64(&[7; 64]),"x":1})
                .to_string(),
        ] {
            let mut sequence = challenged();
            assert_eq!(
                sequence.accept(&payload),
                Err(SignalError::Invalid),
                "{payload}"
            );
            assert_eq!(
                sequence.accept(&auth_proof("c1", &[7; 64])),
                Err(SignalError::Rejected)
            );
        }

        assert_eq!(
            answered_reconnect().send(&auth_challenge("")),
            Err(SignalError::Invalid)
        );
        assert!(answered_reconnect()
            .send(&auth_challenge(&"c".repeat(MAX_REQUEST_ID_BYTES)))
            .is_ok());
        assert_eq!(
            answered_reconnect().send(&auth_challenge(&"c".repeat(MAX_REQUEST_ID_BYTES + 1))),
            Err(SignalError::Invalid)
        );
        let empty = HostSignal::AuthChallenge {
            challenge_id: "c1".to_string(),
            signing_payload: String::new(),
        };
        let mut sequence = answered_reconnect();
        assert_eq!(sequence.send(&empty), Err(SignalError::Invalid));
        assert_eq!(
            sequence.accept(&auth_proof("c1", &[7; 64])),
            Err(SignalError::Rejected)
        );
    }

    #[test]
    fn pairing_messages_are_bounded() {
        let request = |fields: serde_json::Value| {
            let mut message = json!({"type":"pair_request","name":"Phone",
                                     "public_key":b64(&[4; 65]),"project":"p"});
            for (key, value) in fields.as_object().unwrap() {
                message[key] = value.clone();
            }
            message.to_string()
        };
        for payload in [
            pair_request(&[4; 64]),
            pair_request(&[4; 66]),
            request(json!({"public_key":"!!"})),
            request(json!({"public_key":base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD, [4; 65])})),
            request(json!({"name":""})),
            request(json!({"name":"n".repeat(MAX_PAIR_NAME_BYTES + 1)})),
            request(json!({"project":""})),
            request(json!({"project":"p".repeat(MAX_PAIR_PROJECT_BYTES + 1)})),
            request(json!({"extra":1})),
        ] {
            let mut sequence = answered_claim();
            assert_eq!(
                sequence.accept(&payload),
                Err(SignalError::Invalid),
                "{payload}"
            );
            assert_eq!(
                sequence.accept(&pair_request(&[4; 65])),
                Err(SignalError::Rejected)
            );
        }
        let at_bounds = request(json!({"name":"n".repeat(MAX_PAIR_NAME_BYTES),
                                       "project":"p".repeat(MAX_PAIR_PROJECT_BYTES)}));
        assert!(answered_claim().accept(&at_bounds).is_ok());

        let challenged = || {
            let mut sequence = answered_claim();
            assert!(sequence.accept(&pair_request(&[4; 65])).is_ok());
            assert!(sequence.send(&pair_challenge("r1")).is_ok());
            sequence
        };
        for payload in [
            pair_proof("r2", &[7; 64]),
            pair_proof("r1", &[7; 63]),
            pair_proof("r1", &[7; 65]),
            json!({"type":"pair_proof","request_id":"r1","signature":"!!"}).to_string(),
            json!({"type":"pair_proof","request_id":"r1","signature":b64(&[7; 64]),"x":1})
                .to_string(),
        ] {
            assert_eq!(
                challenged().accept(&payload),
                Err(SignalError::Invalid),
                "{payload}"
            );
        }

        let requested = || {
            let mut sequence = answered_claim();
            assert!(sequence.accept(&pair_request(&[4; 65])).is_ok());
            sequence
        };
        assert_eq!(
            requested().send(&pair_challenge("")),
            Err(SignalError::Invalid)
        );
        assert!(requested()
            .send(&pair_challenge(&"r".repeat(MAX_REQUEST_ID_BYTES)))
            .is_ok());
        assert_eq!(
            requested().send(&pair_challenge(&"r".repeat(MAX_REQUEST_ID_BYTES + 1))),
            Err(SignalError::Invalid)
        );
        let empty = HostSignal::PairChallenge {
            request_id: "r1".to_string(),
            signing_payload: String::new(),
        };
        let mut sequence = requested();
        assert_eq!(sequence.send(&empty), Err(SignalError::Invalid));
        // A Host violation rejects the browser's side too.
        assert_eq!(
            sequence.accept(&pair_proof("r1", &[7; 64])),
            Err(SignalError::Rejected)
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
