//! Phase 0 quinn spike (plan sections 7 and 13). Throwaway.
//!
//! Does a stream of small unreliable "entity update" datagrams stay usable
//! while a bulk "terrain" stream saturates the same link? Compares the same
//! datagram load with no bulk traffic, unpaced bulk, paced bulk, BBR, and bulk
//! on a second connection. Run: `cargo run --release -p spike-quinn`.

mod link;

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use bytes::Bytes;
use link::{Link, Proxy};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{ClientConfig, Connection, Endpoint, ServerConfig, TransportConfig};
use rustls::pki_types::PrivatePkcs8KeyDer;
use tokio::time::{interval, sleep, sleep_until};

const RUN: Duration = Duration::from_secs(12);
const DATAGRAM_HZ: f64 = 30.0;
const DATAGRAM_LEN: usize = 200;
const BULK_CHUNK: usize = 16 * 1024;

#[derive(Clone, Copy)]
struct Scenario {
    name: &'static str,
    bulk: Bulk,
    bbr: bool,
    separate_connection: bool,
}

#[derive(Clone, Copy)]
enum Bulk {
    None,
    Unpaced,
    /// Fraction of the link rate.
    Paced(f64),
}

fn main() -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;
    rt.block_on(run_all())
}

async fn run_all() -> Result<()> {
    println!(
        "load: {DATAGRAM_HZ} Hz x {DATAGRAM_LEN} B datagrams server->client for {} s; latency is one-way",
        RUN.as_secs()
    );
    println!("link: RTT 150 ms, jitter ±3 ms, 20 Mbit/s each way, 150 ms drop-tail queue");
    let s = |name, bulk, bbr, separate_connection| Scenario {
        name,
        bulk,
        bbr,
        separate_connection,
    };
    let scenarios = [
        s("datagrams only", Bulk::None, false, false),
        s("same conn, Cubic unpaced", Bulk::Unpaced, false, false),
        s("same conn, BBR unpaced", Bulk::Unpaced, true, false),
        s("same conn, BBR paced 50%", Bulk::Paced(0.5), true, false),
        s("2nd conn, Cubic unpaced", Bulk::Unpaced, false, true),
        s("2nd conn, BBR unpaced", Bulk::Unpaced, true, true),
        s("2nd conn, BBR paced 50%", Bulk::Paced(0.5), true, true),
    ];
    for loss in [0.02, 0.0] {
        let link = Link {
            one_way: Duration::from_millis(75),
            jitter: Duration::from_millis(3),
            loss,
            rate_bps: 20e6,
            queue_limit: Duration::from_millis(150),
        };
        println!("\n== random loss {:.0}% each way", loss * 100.0);
        println!(
            "{:<28} {:>7} {:>7} {:>7} {:>7} {:>7} {:>8} {:>9} {:>11}",
            "scenario",
            "deliv%",
            "p50ms",
            "p90ms",
            "p99ms",
            "maxms",
            "gap ms",
            "bulk Mb/s",
            "q-drops d/u"
        );
        for sc in scenarios {
            let r = run(sc, link).await?;
            println!(
                "{:<28} {:>7.1} {:>7.0} {:>7.0} {:>7.0} {:>7.0} {:>8.0} {:>9.1} {:>11}",
                sc.name,
                r.delivered_pct,
                r.p50,
                r.p90,
                r.p99,
                r.max,
                r.longest_gap,
                r.bulk_mbps,
                format!("{}/{}", r.queue_drops_down, r.queue_drops_up)
            );
        }
    }
    println!(
        "\ndeliv%: datagrams received / sent. gap: longest silence between received datagrams."
    );
    println!("q-drops d/u: packets dropped at the full queue, server->client / client->server.");
    Ok(())
}

struct Report {
    delivered_pct: f64,
    p50: f64,
    p90: f64,
    p99: f64,
    max: f64,
    longest_gap: f64,
    bulk_mbps: f64,
    queue_drops_down: u64,
    queue_drops_up: u64,
}

async fn run(s: Scenario, link: Link) -> Result<Report> {
    let (server_cfg, client_cfg) = configs(s.bbr)?;
    let server = Endpoint::server(server_cfg, "127.0.0.1:0".parse()?)?;
    let proxy = Proxy::start(server.local_addr()?, link, link, 42).await?;

    let mut client_ep = Endpoint::client("127.0.0.1:0".parse()?)?;
    client_ep.set_default_client_config(client_cfg);

    let connections = if s.separate_connection { 2 } else { 1 };
    let accept = tokio::spawn({
        let server = server.clone();
        async move {
            let mut out = Vec::new();
            for _ in 0..connections {
                let incoming = server.accept().await.expect("incoming");
                out.push(incoming.await.expect("handshake"));
            }
            out
        }
    });
    let mut clients = Vec::new();
    for _ in 0..connections {
        clients.push(client_ep.connect(proxy.addr, "localhost")?.await?);
    }
    let servers = accept.await?;

    let start = Instant::now();
    let end = start + RUN;
    let epoch = start;

    // Datagram sender on connection 0.
    let dg_conn = servers[0].clone();
    let sender = tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs_f64(1.0 / DATAGRAM_HZ));
        let mut seq = 0u32;
        while Instant::now() < end {
            tick.tick().await;
            let mut p = vec![0u8; DATAGRAM_LEN];
            p[..4].copy_from_slice(&seq.to_le_bytes());
            p[4..12].copy_from_slice(&(epoch.elapsed().as_micros() as u64).to_le_bytes());
            let _ = dg_conn.send_datagram(Bytes::from(p));
            seq += 1;
        }
        seq
    });

    // Bulk sender.
    let bulk_conn = servers[if s.separate_connection { 1 } else { 0 }].clone();
    let bulk_task = match s.bulk {
        Bulk::None => None,
        Bulk::Unpaced => Some(tokio::spawn(bulk_send(bulk_conn, end, None))),
        Bulk::Paced(f) => Some(tokio::spawn(bulk_send(
            bulk_conn,
            end,
            Some(link.rate_bps / 8.0 * f),
        ))),
    };

    // Client: datagram receiver and bulk receiver.
    let dg_client = clients[0].clone();
    let receiver = tokio::spawn(async move {
        let mut got: Vec<(u32, f64, f64)> = Vec::new();
        let stop = tokio::time::Instant::from_std(end + Duration::from_secs(2));
        loop {
            tokio::select! {
                d = dg_client.read_datagram() => {
                    let Ok(d) = d else { break };
                    if d.len() < 12 { continue }
                    let now = epoch.elapsed().as_secs_f64();
                    let seq = u32::from_le_bytes(d[..4].try_into().expect("4 bytes"));
                    let sent = u64::from_le_bytes(d[4..12].try_into().expect("8 bytes")) as f64 / 1e6;
                    got.push((seq, (now - sent) * 1000.0, now));
                }
                _ = sleep_until(stop) => break,
            }
        }
        got
    });
    let bulk_client = clients[if s.separate_connection { 1 } else { 0 }].clone();
    let bulk_rx = tokio::spawn(async move {
        let mut at_end = 0u64;
        let Ok(mut rx) = bulk_client.accept_uni().await else {
            return at_end;
        };
        let mut buf = vec![0u8; 64 * 1024];
        let mut total = 0u64;
        let stop = tokio::time::Instant::from_std(end);
        loop {
            tokio::select! {
                n = rx.read(&mut buf) => match n {
                    Ok(Some(n)) => total += n as u64,
                    _ => break,
                },
                _ = sleep_until(stop) => { at_end = total; break }
            }
        }
        if at_end == 0 {
            at_end = total
        }
        at_end
    });

    let sent = sender.await?;
    if let Some(b) = bulk_task {
        let _ = b.await;
    }
    sleep(Duration::from_secs(2)).await;
    let mut got = receiver.await?;
    let bulk_bytes = if matches!(s.bulk, Bulk::None) {
        0
    } else {
        bulk_rx.await?
    };

    let (queue_drops_up, queue_drops_down) = proxy.queue_drops();
    for c in servers.iter().chain(clients.iter()) {
        c.close(0u32.into(), b"done");
    }

    got.sort_by(|a, b| a.2.total_cmp(&b.2));
    let mut lat: Vec<f64> = got.iter().map(|g| g.1).collect();
    lat.sort_by(f64::total_cmp);
    let pct = |p: f64| {
        lat.get(((lat.len() as f64 * p) as usize).min(lat.len().saturating_sub(1)))
            .copied()
            .unwrap_or(f64::NAN)
    };
    let longest_gap = got
        .windows(2)
        .map(|w| (w[1].2 - w[0].2) * 1000.0)
        .fold(0.0, f64::max);
    Ok(Report {
        delivered_pct: 100.0 * got.len() as f64 / f64::from(sent),
        p50: pct(0.5),
        p90: pct(0.9),
        p99: pct(0.99),
        max: lat.last().copied().unwrap_or(f64::NAN),
        longest_gap,
        bulk_mbps: bulk_bytes as f64 * 8.0 / RUN.as_secs_f64() / 1e6,
        queue_drops_down,
        queue_drops_up,
    })
}

/// Writes `BULK_CHUNK`-byte pieces on one uni stream until `end`.
async fn bulk_send(conn: Connection, end: Instant, bytes_per_sec: Option<f64>) -> Result<()> {
    let mut tx = conn.open_uni().await?;
    let chunk = vec![0xA5u8; BULK_CHUNK];
    let mut next = tokio::time::Instant::now();
    while Instant::now() < end {
        if tx.write_all(&chunk).await.is_err() {
            break;
        }
        if let Some(rate) = bytes_per_sec {
            next += Duration::from_secs_f64(BULK_CHUNK as f64 / rate);
            sleep_until(next).await;
        }
    }
    let _ = tx.finish();
    Ok(())
}

fn configs(bbr: bool) -> Result<(ServerConfig, ClientConfig)> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let cert_der = cert.cert.der().clone();
    let key = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());

    let server_crypto = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key.into())?;
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert_der)?;
    let client_crypto = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_root_certificates(roots)
        .with_no_client_auth();

    let mut transport = TransportConfig::default();
    if bbr {
        transport.congestion_controller_factory(Arc::new(quinn::congestion::BbrConfig::default()));
    }
    let transport = Arc::new(transport);
    let mut server =
        ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(server_crypto)?));
    server.transport_config(transport.clone());
    let mut client = ClientConfig::new(Arc::new(QuicClientConfig::try_from(client_crypto)?));
    client.transport_config(transport);
    Ok((server, client))
}
