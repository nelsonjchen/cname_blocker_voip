# Rust VoIP.ms CNAME Blocker

Small SIP endpoint that registers outbound to VoIP.ms, checks inbound caller name/CNAME, and either:

- answers blocked names, plays SIT tones plus a disconnected message, then hangs up
- rejects non-matching calls with `486 Busy Here` so VoIP.ms Call Hunting can continue to the next member

No inbound router port forwarding is required.

## VoIP.ms Setup

1. Create a dedicated SIP subaccount for this daemon.
2. Create a Call Hunting entry with Ring Order set to Follow Order.
3. Add the blocker subaccount first.
4. Add your real phone, existing subaccount, or existing ring group second.
5. Route the DID to that Call Hunting entry.

With this order, normal calls receive `486 Busy Here` from the blocker and continue to your real phone. Matching calls are answered by the blocker, so the hunt stops before your phone rings.

## Configuration

Required environment variables:

```sh
VOIPMS_USER=123456_blocker
VOIPMS_PASSWORD=your-subaccount-password
VOIPMS_HOST=losangeles1.voip.ms
```

For local testing, copy the template, edit it, and run:

```sh
cp .env.example .env
cargo run
```

Useful optional variables:

```sh
VOIPMS_PORT=5060
BLOCK_CNAME_PATTERNS=pch
BLOCKER_MESSAGE_AUDIO=/path/to/message.ogg
RTP_PORT_MIN=30000
RTP_PORT_MAX=30100
NAT_KEEPALIVE_SECS=15
REGISTER_EXPIRY_SECS=60
REGISTER_RETRY_SECS=5
REGISTER_MAX_RETRY=3
RUST_LOG=info,xphone::sip::client=warn,xphone::phone=warn
```

`BLOCK_CNAME_PATTERNS` is a comma-separated, case-insensitive substring list. For local or live testing with your name:

```sh
BLOCK_CNAME_PATTERNS=nelson
```

The committed `assets/disconnected.ogg` is decoded locally, resampled to 8 kHz mono PCM if needed, and streamed through the SIP media codec negotiated by `xphone`.

## Run

```sh
cargo run --release
```

Stop with Ctrl-C.

## Docker

```sh
docker build -t cname-blocker-voip .
docker run --rm \
  -e VOIPMS_USER=123456_blocker \
  -e VOIPMS_PASSWORD=your-subaccount-password \
  -e VOIPMS_HOST=losangeles1.voip.ms \
  -e BLOCK_CNAME_PATTERNS=pch \
  cname-blocker-voip
```

If you set a fixed RTP range, expose the same UDP range from the container according to your runtime/network mode.

## Local Tests

The tests do not require VoIP.ms credentials:

```sh
cargo test
```

The integration test registers the daemon against a loopback fake registrar, sends an INVITE with `From: "Nelson" <sip:...>`, verifies a `200 OK`, receives RTP audio, and confirms the registrar sees BYE. It also sends a non-matching CNAME and verifies `486 Busy Here`.
