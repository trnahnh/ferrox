use std::env;
use std::net::{Ipv4Addr, TcpListener, UdpSocket};
use std::thread;

use ferrox::protocol::{self, EXECUTION_REPORT_SIZE};

fn main() {
    // AWS NLB UDP target groups still require a TCP (or HTTP/HTTPS) health
    // check — there's no way to health-check a bare UDP listener directly
    // (see docs/SYSTEM_DESIGN.md §13.3). This is a pure NLB-satisfying stub:
    // accept and drop, no relation to the ExecutionReport stream.
    if let Ok(health_port) = env::var("HEALTH_PORT") {
        thread::spawn(move || {
            let port = health_port.parse::<u16>().expect("HEALTH_PORT must be a u16");
            let listener = TcpListener::bind(("0.0.0.0", port)).expect("failed to bind health check listener");
            eprintln!("subscriber: health check listener bound on 0.0.0.0:{port}");
            for conn in listener.incoming() {
                match conn {
                    Ok(stream) => {
                        eprintln!("subscriber: health check connection from {:?}", stream.peer_addr());
                        drop(stream);
                    }
                    Err(e) => eprintln!("subscriber: health check accept error: {e}"),
                }
            }
        });
    }

    let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9001".to_string());
    let socket = UdpSocket::bind(&bind_addr).expect("failed to bind UDP socket");

    // On-prem deployments join the exchange's multicast group directly (default).
    // Cloud deployments (see docs/SYSTEM_DESIGN.md §13) sit behind a unicast relay
    // instead, since UDP multicast doesn't traverse a VPC — set MULTICAST_GROUP=""
    // (or leave unset with no local multicast source) to run in plain unicast mode.
    match env::var("MULTICAST_GROUP") {
        Ok(group) if !group.is_empty() => {
            let addr: Ipv4Addr = group.parse().expect("MULTICAST_GROUP must be an IPv4 address");
            socket
                .join_multicast_v4(&addr, &Ipv4Addr::UNSPECIFIED)
                .expect("failed to join multicast group");
            eprintln!("subscriber: listening for execution reports on {bind_addr}, multicast group {group}");
        }
        _ => {
            eprintln!("subscriber: listening for execution reports on {bind_addr} (unicast)");
        }
    }

    let mut buf = [0u8; EXECUTION_REPORT_SIZE];
    let mut expected_seq: u32 = 1;

    loop {
        let (n, src) = match socket.recv_from(&mut buf) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("subscriber: recv error: {e}");
                continue;
            }
        };

        if n < EXECUTION_REPORT_SIZE {
            eprintln!("subscriber: short packet ({n} bytes) from {src}");
            continue;
        }

        let report = match protocol::decode_execution_report(&buf) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("subscriber: decode error: {e}");
                continue;
            }
        };

        if report.seq_num != expected_seq {
            let gap = report.seq_num.wrapping_sub(expected_seq);
            eprintln!(
                "subscriber: GAP detected — expected seq {expected_seq}, got {}, missing {gap} report(s)",
                report.seq_num
            );
        }
        expected_seq = report.seq_num.wrapping_add(1);

        println!(
            "seq={} taker={} maker={} price={} qty={} ts={}",
            report.seq_num,
            report.taker_order_id,
            report.maker_order_id,
            report.price,
            report.quantity,
            report.timestamp,
        );
    }
}
