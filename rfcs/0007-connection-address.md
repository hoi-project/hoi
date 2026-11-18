# RFC-0007: Show the address a connection exposes

Issue: [#114](https://github.com/hoi-project/hoi/issues/114)

Status: proposed

## Decision

A node returns the IP it observes to the remote endpoint of that same connection. Each
accepted sync hello can carry an optional `oa` text field containing an IPv4 or IPv6
literal without port or zone. The node omits it when it has no direct socket observation.
A new hello replaces the prior observation, including when it omits the field. Clients
keep one value per active link and discard it when the link closes. A peer can misreport its
observation. It never authorizes a dial, proves a VPN is active or changes node trust.

A home node may include `he` only after the recipient authenticates as its paired device.
It lists up to 16 distinct strings of the form `transport IP` (`quic`, `webrtc` or
`websocket`). Each address is the home node's own source as reported on an active onward
connection. It supplies no other peer's IP, identifier, or community. The device displays
this list separately from its own connection to home, refreshes it on replacement hellos,
and drops it when the home connection closes. Ordinary peers never receive this list.

Before joining, a person can explicitly check a selected node. The screen says that the
check reveals the source IP to the destination. The client opens a fresh connection to that
route and sends no sync hello, identity, pairing secret or community. A browser uses the
invite's pinned DTLS endpoint in client mode or its WebSocket endpoint. A server accepting
members recognizes an initial binary frame containing the ASCII bytes `hoi/address-check/1`.
It replies once, on that connection, with `hoi/address/1 ` followed by an IP literal or
nothing if its direct observation is unavailable. A check cannot become a sync session.
The usual WebRTC frame length, encrypted channel, WebSocket origin checks and connection
limits apply. Home nodes that do not accept strangers remain silent before pairing.

The native client may also check the UDP path to an invited node's RTC port through its
existing RFC 8489 STUN service. It checks the responder address and transaction ID and labels
the result as UDP. This observes the route without verifying the node's DTLS fingerprint.
When the pinned browser RTC probe fails, the browser can ask the same numeric socket
for a STUN binding using a fresh peer connection. Only a numeric server-reflexive UDP
candidate from that check is displayed. A missing or redacted candidate leaves the
peer result unavailable.
For a numeric direct QUIC route, the native client creates a fresh temporary endpoint,
pins its socket source to that route and authenticates the endpoint key in the invitation.
The ALPN `hoi/address/1` accepts one bidirectional stream containing the same fixed request,
terminated by FIN. It returns the same response, terminated by FIN, without opening sync.
The client uses no default relays, discovery providers, UPnP, captive-portal or HTTPS probes.
The listener applies its usual admission, per-source and connection limits.
When an interface is required, native checks use numeric destinations or loopback. Linux
can also resolve a hostname through systemd-resolved on that interface when the response
identifies the selected link. Without per-interface DNS, the check asks for a numeric route. Displays label
loopback, local and other special-use observations, and an ordinary unicast report does not
establish public reachability.
The native probe follows the selected VPN and home-network policy. A reverse proxy's own
upstream address and an iroh packet relay's address are never substituted for a client IP.

## Alternatives and effects

A separate user-initiated **Check public IP** can contact the named HTTPS observer
`api64.ipify.org` without an invitation or active peer. The UI identifies the service
before contact and labels the returned value as the browser’s HTTPS exit IP, with the
time of observation. A peer check, a failed peer check, and opening the panel send no
request to this service. HTTPS and peer traffic can follow different routes. Browser ICE
statistics can supply direct evidence for some active connections, but can omit a reflexive
candidate. Both known addresses and unavailable states remain local. Observations never
enter signed records, operator logs, peer discovery, storage or shared profile data.
