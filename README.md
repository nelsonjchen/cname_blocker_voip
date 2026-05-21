# Rust VoIP.ms CNAME Blocker

Small SIP endpoint that registers outbound to VoIP.ms, checks inbound caller name/CNAME, and either:

- answers blocked names, plays SIT tones plus a disconnected message, then hangs up
- rejects non-matching calls with `486 Busy Here` so VoIP.ms Call Hunting can continue to the next member

No inbound router port forwarding is required.

## VoIP.ms Setup

The blocker is designed to sit in front of your normal phone path as the first
member of a VoIP.ms Call Hunting group. It should be the first endpoint that gets
offered the call, but it should only answer calls that match your block list.

1. Create a dedicated SIP subaccount for this daemon.
   - Give it a boring internal name like `cname-blocker`.
   - Use those credentials for `VOIPMS_USER` and `VOIPMS_PASSWORD`.
   - Register it to the same VoIP.ms POP you normally use, such as
     `losangeles1.voip.ms`.
2. Create a Call Hunting entry.
   - Set Ring Order to Follow Order.
   - Disable any mode that rings all destinations at once.
   - Keep the blocker first in the ordered list.
3. Add the blocker subaccount as the first call hunting member.
   - It only needs a short ring time because it decides immediately.
   - It does not need voicemail or any public inbound port forwarding.
4. Add your real destination second.
   - This can be your normal SIP subaccount, a ring group, forwarding target, or
     whatever already receives your calls.
5. Route the DID to the Call Hunting entry.
   - In DID routing, send the number to the new hunting group instead of directly
     to your phone/ring group.

With this order, normal calls receive `486 Busy Here` from the blocker and continue to your real phone. Matching calls are answered by the blocker, so the hunt stops before your phone rings.

### Why This Works

For non-matching calls, the blocker intentionally rejects the INVITE with:

```text
486 Busy Here
```

VoIP.ms treats that as "try the next call hunting member", so the call continues
to your real phone path.

For matching calls, the blocker answers the call, plays the disconnected audio,
then hangs up. Because the call was answered, VoIP.ms does not continue to the
next hunting member.

### Suggested First Test

Before using a production block pattern, temporarily block something you can
generate yourself:

```sh
BLOCK_CNAME_PATTERNS=yourname
```

Call the DID from a number whose caller name contains that value. You should hear
the blocker audio and your real phone should not ring. Then call from a
non-matching caller name; the blocker should return `486 Busy Here` and your real
phone should ring through the next call hunting member.

After testing, switch back to your real pattern list, for example:

```sh
BLOCK_CNAME_PATTERNS=pch
```

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
BLOCK_CNAME_REGEXES=[[:alpha:]] CA$
TWILIO_API_KEY_SID=SKxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
TWILIO_API_KEY_SECRET=replace-me
TWILIO_LOOKUP_TIMEOUT_MS=1500
BLOCKER_MESSAGE_AUDIO=/path/to/message.ogg
RTP_PORT_MIN=30000
RTP_PORT_MAX=30100
NAT_KEEPALIVE_SECS=15
REGISTER_EXPIRY_SECS=60
REGISTER_RETRY_SECS=5
REGISTER_MAX_RETRY=3
RUST_LOG=info,xphone::sip::client=warn,xphone::phone=warn
LOG_ANSI=false
LOG_TIMESTAMPS=false
```

`BLOCK_CNAME_PATTERNS` is a comma-separated, case-insensitive token list. Patterns match on non-alphanumeric boundaries, so `pch` matches `PCH` and `PCH-CLAIMS` but not `Kupchak`.

`BLOCK_CNAME_REGEXES` is an optional comma-separated list of case-insensitive Rust regexes matched against caller name and SIP `From` headers. For example, `[[:alpha:]] CA$` blocks city/state CNAM values ending in a letter, a space, and `CA`, such as `UPLAND CA`, `BREA CA`, or `ONTARIO CA`.

When `TWILIO_API_KEY_SID` and `TWILIO_API_KEY_SECRET` are both set, the blocker looks up the caller name with Twilio Lookup v2 (`Fields=caller_name`) and matches against that result instead of the upstream SIP/VoIP.ms CNAME. If Twilio returns no caller name or the lookup fails, the call is allowed to cascade.

For local or live testing with your name:

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
