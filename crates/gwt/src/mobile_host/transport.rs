//! One Host-side WebRTC exchange, driven by the caller's packet and timer loop.
//!
//! A verified DTLS connection establishes peer identity. The Host still owns
//! device authorization, the registration lease, and the lifetime of each
//! connection using that identity.

use std::{sync::Arc, time::Instant};

use str0m::{
    change::SdpOffer,
    crypto::{self, Fingerprint},
    Candidate, Event, IceConnectionState, Input, Output, Rtc,
};
use uuid::Uuid;

use super::signal::{MAX_CANDIDATES, MAX_PAYLOAD_BYTES};
use crate::mobile_connection::{HostBinding, PeerBinding};

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum HostWebRtcError {
    #[error("invalid mobile WebRTC offer")]
    Offer,
    #[error("invalid mobile WebRTC connection ID")]
    ConnectionId,
    #[error("mobile WebRTC transport failed")]
    Transport,
}

/// Owns the RTC so callers cannot change its certificate verification policy.
pub struct HostWebRtcPeer {
    rtc: Rtc,
    host: HostBinding,
    connection_id: Uuid,
    offered_fingerprint: Fingerprint,
    dtls_connected: bool,
}

impl HostWebRtcPeer {
    /// Accept one enabled DataChannel media line and generate the Host answer.
    /// The Host identity, connection ID, and local candidate come from the
    /// trusted Host caller.
    pub fn accept_offer(
        host: HostBinding,
        connection_id: Uuid,
        sdp: &str,
        local_candidate: Candidate,
        now: Instant,
    ) -> Result<(Self, String), HostWebRtcError> {
        if connection_id.is_nil() {
            return Err(HostWebRtcError::ConnectionId);
        }
        if sdp.len() > MAX_PAYLOAD_BYTES || !has_one_data_channel(sdp) {
            return Err(HostWebRtcError::Offer);
        }
        let offer = SdpOffer::from_sdp_string(sdp).map_err(|_| HostWebRtcError::Offer)?;
        let [media] = offer.media_lines.as_slice() else {
            return Err(HostWebRtcError::Offer);
        };
        // Negotiation can read the media ID before str0m checks consistency.
        if media.check_consistent().is_some()
            || offer
                .session
                .ice_candidates()
                .chain(media.ice_candidates())
                .take(MAX_CANDIDATES + 1)
                .count()
                > MAX_CANDIDATES
        {
            return Err(HostWebRtcError::Offer);
        }
        let offered_fingerprint = media
            .fingerprint()
            .or_else(|| offer.session.fingerprint())
            .filter(|fingerprint| sha256_fingerprint(fingerprint).is_some())
            .ok_or(HostWebRtcError::Offer)?;

        let mut rtc = Rtc::builder()
            .set_crypto_provider(Arc::new(crypto::from_feature_flags()))
            .build(now);
        rtc.add_local_candidate(local_candidate)
            .ok_or(HostWebRtcError::Transport)?;
        let answer = rtc
            .sdp_api()
            .accept_offer(offer)
            .map_err(|_| HostWebRtcError::Offer)?;
        Ok((
            Self {
                rtc,
                host,
                connection_id,
                offered_fingerprint,
                dtls_connected: false,
            },
            answer.to_sdp_string(),
        ))
    }

    pub fn handle_input(&mut self, input: Input<'_>) -> Result<(), HostWebRtcError> {
        self.rtc.handle_input(input).map_err(|_| {
            self.close();
            HostWebRtcError::Transport
        })
    }

    pub fn poll_output(&mut self) -> Result<Output, HostWebRtcError> {
        let output = self.rtc.poll_output().map_err(|_| {
            self.close();
            HostWebRtcError::Transport
        })?;
        match &output {
            Output::Event(Event::Connected) => self.dtls_connected = true,
            Output::Event(
                Event::Closed | Event::IceConnectionStateChange(IceConnectionState::Disconnected),
            ) => self.close(),
            _ => {}
        }
        Ok(output)
    }

    pub fn peer_binding(&mut self) -> Option<PeerBinding> {
        if !self.dtls_connected || !self.rtc.is_alive() {
            return None;
        }
        let dtls = self.rtc.direct_api();
        // str0m records the certificate digest while verifying it. Require a
        // successful connection event and exact agreement with the offer.
        let remote = dtls.remote_dtls_fingerprint()?;
        if remote != &self.offered_fingerprint {
            return None;
        }
        PeerBinding::new(
            self.host.clone(),
            self.connection_id,
            sha256_fingerprint(dtls.local_dtls_fingerprint())?,
            sha256_fingerprint(remote)?,
        )
        .ok()
    }

    pub fn close(&mut self) {
        self.dtls_connected = false;
        self.rtc.disconnect();
    }
}

// The SDP parser does not retain the application line's port or format and
// can ignore a non-SDP suffix. Check that envelope before negotiating.
fn has_one_data_channel(sdp: &str) -> bool {
    if sdp
        .lines()
        .any(|line| line.as_bytes().get(1) != Some(&b'='))
    {
        return false;
    }
    let mut media_lines = sdp.lines().filter_map(|line| line.strip_prefix("m="));
    let Some(media_line) = media_lines.next() else {
        return false;
    };
    let mut fields = media_line.split_ascii_whitespace();
    media_lines.next().is_none()
        && fields.next() == Some("application")
        && fields
            .next()
            .and_then(|port| port.parse::<u16>().ok())
            .is_some_and(|port| port != 0)
        && fields.next() == Some("UDP/DTLS/SCTP")
        && fields.next() == Some("webrtc-datachannel")
        && fields.next().is_none()
}

fn sha256_fingerprint(fingerprint: &Fingerprint) -> Option<[u8; 32]> {
    if fingerprint.hash_func != "sha-256" {
        return None;
    }
    fingerprint.bytes.as_slice().try_into().ok()
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use str0m::{
        change::SdpAnswer,
        media::{Direction, MediaKind},
        net::{Receive, Transmit},
        Candidate, Input, Output, Rtc,
    };
    use uuid::Uuid;

    use super::{HostWebRtcError, HostWebRtcPeer};
    use crate::mobile_connection::{HostBinding, PeerBinding};

    fn host_identity() -> HostBinding {
        HostBinding::new(
            "184abb7c-7a62-4699-a498-55a9562e6505",
            "https://example.test",
        )
        .unwrap()
    }

    fn candidate(port: u16) -> Candidate {
        Candidate::host(([127, 0, 0, 1], port).into(), "udp").unwrap()
    }

    fn receive(now: Instant, packet: &Transmit) -> Input<'_> {
        Input::Receive(
            now,
            Receive {
                proto: packet.proto,
                source: packet.source,
                destination: packet.destination,
                contents: (&packet.contents[..]).try_into().unwrap(),
            },
        )
    }

    // Drive both real protocol stacks directly. The timeout outputs supply a
    // simulated clock and packets cross without an OS socket.
    fn handshake(
        host: &mut HostWebRtcPeer,
        browser: &mut Rtc,
        mut now: Instant,
    ) -> Result<PeerBinding, HostWebRtcError> {
        for _ in 0..256 {
            if browser.direct_api().remote_dtls_fingerprint().is_some() {
                if let Some(binding) = host.peer_binding() {
                    return Ok(binding);
                }
            }
            let host_deadline = match host.poll_output()? {
                Output::Transmit(packet) => {
                    browser.handle_input(receive(now, &packet)).unwrap();
                    continue;
                }
                Output::Event(_) => continue,
                Output::Timeout(deadline) => deadline,
            };
            match browser.poll_output().unwrap() {
                Output::Transmit(packet) => host.handle_input(receive(now, &packet))?,
                Output::Event(_) => {}
                Output::Timeout(browser_deadline) => {
                    now = now.max(host_deadline.min(browser_deadline));
                    if host_deadline <= browser_deadline {
                        host.handle_input(Input::Timeout(now))?;
                    } else {
                        browser.handle_input(Input::Timeout(now)).unwrap();
                    }
                }
            }
        }
        panic!("the DTLS handshake did not finish");
    }

    #[test]
    fn binding_requires_live_dtls_with_the_offered_certificate() {
        let now = Instant::now();
        let connection_id = Uuid::new_v4();
        for valid in [true, false] {
            let mut browser = Rtc::new(now);
            browser.add_local_candidate(candidate(40000)).unwrap();
            let mut changes = browser.sdp_api();
            changes.add_channel("mobile".to_owned());
            let (offer, pending) = changes.apply().unwrap();
            let mut offer = offer.to_sdp_string();
            let browser_fingerprint = browser.direct_api().local_dtls_fingerprint().clone();
            let mut other_fingerprint = browser_fingerprint.clone();
            other_fingerprint.bytes[0] ^= 0xff;
            if valid {
                let media_start = offer.find("m=application").unwrap();
                offer.insert_str(
                    media_start,
                    &format!("a=fingerprint:{other_fingerprint}\r\n"),
                );
            } else {
                offer = offer.replace(
                    &browser_fingerprint.to_string(),
                    &other_fingerprint.to_string(),
                );
            }

            let (mut host, answer) = HostWebRtcPeer::accept_offer(
                host_identity(),
                connection_id,
                &offer,
                candidate(40001),
                now,
            )
            .unwrap();
            let answer = SdpAnswer::from_sdp_string(&answer).unwrap();
            let advertised_host_fingerprint = answer.media_lines[0]
                .fingerprint()
                .or_else(|| answer.session.fingerprint())
                .unwrap();
            browser.sdp_api().accept_answer(pending, answer).unwrap();

            assert!(host.peer_binding().is_none());
            let binding = handshake(&mut host, &mut browser, now);
            if valid {
                let actual_host_fingerprint = browser
                    .direct_api()
                    .remote_dtls_fingerprint()
                    .unwrap()
                    .clone();
                assert_eq!(actual_host_fingerprint, advertised_host_fingerprint);
                assert_eq!(
                    binding.unwrap(),
                    PeerBinding::new(
                        host_identity(),
                        connection_id,
                        actual_host_fingerprint.bytes.try_into().unwrap(),
                        browser_fingerprint.bytes.try_into().unwrap(),
                    )
                    .unwrap()
                );
            } else {
                assert_eq!(binding, Err(HostWebRtcError::Transport));
                assert!(host.peer_binding().is_none());
            }
            host.close();
            assert!(host.peer_binding().is_none());
        }
    }

    #[test]
    fn rejects_bad_and_non_data_channel_offers() {
        let now = Instant::now();
        let mut browser = Rtc::new(now);
        let mut changes = browser.sdp_api();
        changes.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
        let (offer, _) = changes.apply().unwrap();
        let media_offer = offer.to_sdp_string();
        let mut browser = Rtc::new(now);
        let mut changes = browser.sdp_api();
        changes.add_channel("mobile".to_owned());
        let (offer, _) = changes.apply().unwrap();
        let offer = offer.to_sdp_string();
        let missing_mid = offer
            .lines()
            .filter(|line| !line.starts_with("a=mid:"))
            .collect::<Vec<_>>()
            .join("\r\n");
        let trailing_garbage = format!("{offer}garbage");
        let application_line = offer.lines().find(|line| line.starts_with("m=")).unwrap();
        let unsupported_format =
            offer.replace(application_line, "m=application 9 UDP/DTLS/SCTP unknown");
        for sdp in [
            "not an offer",
            &media_offer,
            &missing_mid,
            &trailing_garbage,
            &unsupported_format,
        ] {
            assert!(matches!(
                HostWebRtcPeer::accept_offer(
                    host_identity(),
                    Uuid::new_v4(),
                    sdp,
                    candidate(40001),
                    now
                ),
                Err(HostWebRtcError::Offer)
            ));
        }
    }
}
