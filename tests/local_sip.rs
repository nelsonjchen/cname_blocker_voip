use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use cname_blocker_voip::{AppConfig, CnameBlocker, DisconnectAudio, config::lookup_from_map};
use fakepbx::{FakePBX, sdp, with_auth};
use xphone::Phone;

fn phone_config(pbx: &FakePBX) -> AppConfig {
    let (host, port) = split_host_port(pbx.addr());
    AppConfig::from_lookup(lookup_from_map(std::collections::HashMap::from([
        ("VOIPMS_USER", "1001"),
        ("VOIPMS_PASSWORD", "test"),
        ("VOIPMS_HOST", host),
        ("VOIPMS_PORT", port.as_str()),
        ("BLOCK_CNAME_PATTERNS", "nelson"),
        ("REGISTER_RETRY_SECS", "1"),
        ("REGISTER_MAX_RETRY", "3"),
        ("RTP_PORT_MIN", "30000"),
        ("RTP_PORT_MAX", "30100"),
    ])))
    .unwrap()
}

fn connect_blocker(pbx: &FakePBX) -> Phone {
    let config = phone_config(pbx);
    let audio = DisconnectAudio::load(None).unwrap();
    let blocker = CnameBlocker::new(config.block_patterns.clone(), audio);
    let phone = Phone::new(config.xphone_config());

    let (registered_tx, registered_rx) = crossbeam_channel::bounded(1);
    phone.on_registered(move || {
        let _ = registered_tx.try_send(());
    });
    blocker.install_on(&phone);
    phone.connect().unwrap();
    registered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("blocker registration timed out");
    phone
}

#[test]
fn nelson_cname_is_answered_media_is_played_and_hunt_stops() {
    let pbx = FakePBX::new(&[with_auth("1001", "test")]);
    let phone = connect_blocker(&pbx);
    assert!(pbx.wait_for_register(1, Duration::from_secs(2)));

    let contact_uri = registered_contact(&pbx);
    let rtp = UdpSocket::bind("127.0.0.1:0").unwrap();
    rtp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let rtp_port = rtp.local_addr().unwrap().port();

    let real_phone_invocations = Arc::new(AtomicUsize::new(0));
    let outcome = send_invite_with_cname(&contact_uri, "Nelson", rtp_port);
    assert_eq!(outcome.final_status, 200);
    assert!(
        pbx.wait_for_bye(1, Duration::from_secs(8)),
        "blocked call should be hung up through the SIP registrar"
    );
    assert_eq!(
        real_phone_invocations.load(Ordering::SeqCst),
        0,
        "answered blocked call should stop the simulated hunt before the real phone"
    );

    let mut packet = [0_u8; 1500];
    let (len, _) = rtp
        .recv_from(&mut packet)
        .expect("expected RTP audio from blocked call");
    assert!(len >= 12, "RTP packet should include a header");

    phone.disconnect().unwrap();
}

#[test]
fn non_matching_cname_returns_busy_for_call_hunting_cascade() {
    let pbx = FakePBX::new(&[with_auth("1001", "test")]);
    let phone = connect_blocker(&pbx);
    assert!(pbx.wait_for_register(1, Duration::from_secs(2)));

    let contact_uri = registered_contact(&pbx);
    let outcome = send_invite_with_cname(&contact_uri, "Friendly Caller", 31000);
    assert_eq!(outcome.final_status, 486);
    assert_eq!(pbx.bye_count(), 0);

    let real_phone_invocations = AtomicUsize::new(0);
    if outcome.final_status == 486 {
        real_phone_invocations.fetch_add(1, Ordering::SeqCst);
    }
    assert_eq!(
        real_phone_invocations.load(Ordering::SeqCst),
        1,
        "486 from member 1 represents VoIP.ms advancing to the next hunt member"
    );

    phone.disconnect().unwrap();
}

#[derive(Debug)]
struct InviteOutcome {
    final_status: u16,
}

fn send_invite_with_cname(target_uri: &str, cname: &str, rtp_port: u16) -> InviteOutcome {
    let target_addr = sip_uri_addr(target_uri);
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let local_addr = socket.local_addr().unwrap();
    let branch = format!("z9hG4bK-local-{}", local_addr.port());
    let call_id = format!("local-test-{}@127.0.0.1", local_addr.port());
    let from = format!(
        "\"{cname}\" <sip:caller@127.0.0.1>;tag=from{}",
        local_addr.port()
    );
    let to = format!("<{target_uri}>");
    let contact = format!("<sip:caller@{local_addr}>");
    let sdp = sdp::sdp("127.0.0.1", rtp_port, &[sdp::PCMU]);
    let invite = format!(
        "INVITE {target_uri} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {local_addr};branch={branch}\r\n\
         Max-Forwards: 70\r\n\
         From: {from}\r\n\
         To: {to}\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: 1 INVITE\r\n\
         Contact: {contact}\r\n\
         Content-Type: application/sdp\r\n\
         Content-Length: {}\r\n\
         \r\n\
         {sdp}",
        sdp.len()
    );

    socket.send_to(invite.as_bytes(), &target_addr).unwrap();

    let mut buf = [0_u8; 4096];
    let (final_status, final_response) = loop {
        let (len, _) = socket.recv_from(&mut buf).unwrap();
        let message = String::from_utf8_lossy(&buf[..len]).to_string();
        if let Some(status) = response_status(&message)
            && status >= 200
        {
            break (status, message);
        }
    };

    if final_status == 200 {
        socket
            .set_read_timeout(Some(Duration::from_secs(12)))
            .unwrap();
        let tagged_to = header_value(&final_response, "To").unwrap_or(to);
        let ack = format!(
            "ACK {target_uri} SIP/2.0\r\n\
             Via: SIP/2.0/UDP {local_addr};branch={branch}-ack\r\n\
             Max-Forwards: 70\r\n\
             From: {from}\r\n\
             To: {tagged_to}\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 ACK\r\n\
             Contact: {contact}\r\n\
             Content-Length: 0\r\n\
             \r\n"
        );
        socket.send_to(ack.as_bytes(), &target_addr).unwrap();
    }

    InviteOutcome { final_status }
}

fn registered_contact(pbx: &FakePBX) -> String {
    pbx.last_register()
        .and_then(|record| record.request.contact())
        .expect("REGISTER should include a Contact URI")
}

fn split_host_port(addr: &str) -> (&str, String) {
    let (host, port) = addr.rsplit_once(':').unwrap();
    (host, port.to_string())
}

fn sip_uri_addr(uri: &str) -> String {
    let without_scheme = uri.strip_prefix("sip:").unwrap_or(uri);
    let host_port = without_scheme.split('@').next_back().unwrap();
    host_port
        .trim_matches(['<', '>'])
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

fn response_status(message: &str) -> Option<u16> {
    message
        .lines()
        .next()?
        .strip_prefix("SIP/2.0 ")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn header_value(message: &str, header: &str) -> Option<String> {
    message.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case(header) {
            Some(value.trim().to_string())
        } else {
            None
        }
    })
}
