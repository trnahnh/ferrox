// Bridges the exchange's UDP multicast feed to a single unicast destination.
//
// AWS VPCs are unicast-only — there is no L2 multicast domain to join from a
// cloud subscriber (see docs/SYSTEM_DESIGN.md §13.2). This relay runs on the
// same network segment as the matching engine, joins the multicast group as
// an ordinary subscriber, and forwards every raw ExecutionReport datagram
// unmodified to a remote unicast address (e.g. a cloud NLB's UDP listener).
//
// It does not decode, validate, or repair anything — a dropped multicast
// packet upstream produces the same gap downstream. It converts reach, not
// reliability.

use std::env;
use std::net::{Ipv4Addr, UdpSocket};

use ferrox::protocol::EXECUTION_REPORT_SIZE;

fn main() {
    let multicast_group: Ipv4Addr = env::var("MULTICAST_GROUP")
        .unwrap_or_else(|_| "239.1.1.1".to_string())
        .parse()
        .expect("MULTICAST_GROUP must be an IPv4 address");
    let listen_port: u16 = env::var("LISTEN_PORT")
        .unwrap_or_else(|_| "9001".to_string())
        .parse()
        .expect("LISTEN_PORT must be a u16");
    let relay_to = env::var("RELAY_TO").expect("RELAY_TO must be set (e.g. 203.0.113.10:9001)");

    let listen_socket = UdpSocket::bind(("0.0.0.0", listen_port)).expect("failed to bind listen socket");
    listen_socket
        .join_multicast_v4(&multicast_group, &Ipv4Addr::UNSPECIFIED)
        .expect("failed to join multicast group");

    let send_socket = UdpSocket::bind("0.0.0.0:0").expect("failed to bind send socket");
    send_socket.connect(&relay_to).expect("failed to connect to relay target");

    eprintln!("relay: {multicast_group}:{listen_port} -> {relay_to} (unicast)");

    let mut buf = [0u8; EXECUTION_REPORT_SIZE];
    let mut forwarded: u64 = 0;
    loop {
        let (n, src) = match listen_socket.recv_from(&mut buf) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("relay: recv error: {e}");
                continue;
            }
        };

        if n < EXECUTION_REPORT_SIZE {
            eprintln!("relay: short packet ({n} bytes) from {src}, dropping");
            continue;
        }

        if let Err(e) = send_socket.send(&buf[..n]) {
            eprintln!("relay: send error: {e}");
            continue;
        }

        forwarded += 1;
        if forwarded % 100_000 == 0 {
            eprintln!("relay: forwarded {forwarded} packets");
        }
    }
}
