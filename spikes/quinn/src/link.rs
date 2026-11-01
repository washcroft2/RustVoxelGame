//! Userspace link shaper: a UDP proxy that adds one-way delay, jitter, random
//! loss, a rate cap and a drop-tail queue, shared by every connection through
//! it. Stands in for `tc netem` so the spike needs no root.
//!
//! Delivery is strictly in order per direction. Jitter changes delay but never
//! reorders; independent per-packet jitter at these packet rates reorders
//! heavily, which loss-based congestion control reads as loss.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::time::sleep_until;

#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub one_way: Duration,
    pub jitter: Duration,
    /// Independent drop probability per packet.
    pub loss: f64,
    pub rate_bps: f64,
    /// Longest the bottleneck queue may hold a packet before it is dropped.
    pub queue_limit: Duration,
}

struct Direction {
    link: Link,
    next_free: Instant,
    last_deliver: Instant,
    rng: u64,
    dropped_queue: u64,
}

impl Direction {
    fn rand(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    /// When this packet should be delivered, or `None` if the link drops it.
    fn admit(&mut self, bytes: usize) -> Option<Instant> {
        if self.rand() < self.link.loss {
            return None;
        }
        let now = Instant::now();
        let start = self.next_free.max(now);
        if start - now > self.link.queue_limit {
            self.dropped_queue += 1;
            return None;
        }
        self.next_free = start + Duration::from_secs_f64(bytes as f64 * 8.0 / self.link.rate_bps);
        let jitter = self.link.jitter.as_secs_f64() * (self.rand() * 2.0 - 1.0);
        let delay = (self.link.one_way.as_secs_f64() + jitter).max(0.0);
        let at = (self.next_free + Duration::from_secs_f64(delay)).max(self.last_deliver);
        self.last_deliver = at;
        Some(at)
    }
}

enum Dest {
    /// Towards the server, from a per-client socket.
    Back(Arc<UdpSocket>),
    /// Towards a client, from the shared front socket.
    Front(Arc<UdpSocket>, SocketAddr),
}

struct Delivery {
    at: Instant,
    data: Vec<u8>,
    dest: Dest,
}

/// One task per direction sleeps until each packet's time in turn, so order is kept.
fn spawn_delivery() -> UnboundedSender<Delivery> {
    let (tx, mut rx) = unbounded_channel::<Delivery>();
    tokio::spawn(async move {
        while let Some(d) = rx.recv().await {
            sleep_until(d.at.into()).await;
            let _ = match d.dest {
                Dest::Back(s) => s.send(&d.data).await,
                Dest::Front(s, to) => s.send_to(&d.data, to).await,
            };
        }
    });
    tx
}

pub struct Proxy {
    pub addr: SocketAddr,
    up: Arc<Mutex<Direction>>,
    down: Arc<Mutex<Direction>>,
}

impl Proxy {
    /// Starts a proxy in front of `server`. `up` shapes client to server, `down` the reverse.
    pub async fn start(
        server: SocketAddr,
        up: Link,
        down: Link,
        seed: u64,
    ) -> anyhow::Result<Self> {
        let front = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
        let addr = front.local_addr()?;
        let mk = |link, s| {
            Arc::new(Mutex::new(Direction {
                link,
                next_free: Instant::now(),
                last_deliver: Instant::now(),
                rng: seed ^ s,
                dropped_queue: 0,
            }))
        };
        let (up_dir, down_dir) = (
            mk(up, 0x1234_5678_9ABC_DEF1),
            mk(down, 0x0FED_CBA9_8765_4321),
        );
        let (up_tx, down_tx) = (spawn_delivery(), spawn_delivery());
        tokio::spawn(client_side(
            front,
            server,
            up_dir.clone(),
            down_dir.clone(),
            up_tx,
            down_tx,
        ));
        Ok(Self {
            addr,
            up: up_dir,
            down: down_dir,
        })
    }

    /// Packets dropped at the full queue, as `(client to server, server to client)`.
    pub fn queue_drops(&self) -> (u64, u64) {
        let d = |m: &Arc<Mutex<Direction>>| m.lock().expect("lock").dropped_queue;
        (d(&self.up), d(&self.down))
    }
}

async fn client_side(
    front: Arc<UdpSocket>,
    server: SocketAddr,
    up: Arc<Mutex<Direction>>,
    down: Arc<Mutex<Direction>>,
    up_tx: UnboundedSender<Delivery>,
    down_tx: UnboundedSender<Delivery>,
) {
    let mut backs: HashMap<SocketAddr, Arc<UdpSocket>> = HashMap::new();
    let mut buf = vec![0u8; 2048];
    loop {
        let Ok((n, client)) = front.recv_from(&mut buf).await else {
            return;
        };
        let back = match backs.get(&client) {
            Some(b) => b.clone(),
            None => {
                let Ok(b) = UdpSocket::bind("127.0.0.1:0").await else {
                    return;
                };
                if b.connect(server).await.is_err() {
                    return;
                }
                let b = Arc::new(b);
                tokio::spawn(server_side(
                    b.clone(),
                    front.clone(),
                    client,
                    down.clone(),
                    down_tx.clone(),
                ));
                backs.insert(client, b.clone());
                b
            }
        };
        let at = up.lock().expect("lock").admit(n);
        if let Some(at) = at {
            let _ = up_tx.send(Delivery {
                at,
                data: buf[..n].to_vec(),
                dest: Dest::Back(back),
            });
        }
    }
}

async fn server_side(
    back: Arc<UdpSocket>,
    front: Arc<UdpSocket>,
    client: SocketAddr,
    down: Arc<Mutex<Direction>>,
    down_tx: UnboundedSender<Delivery>,
) {
    let mut buf = vec![0u8; 2048];
    loop {
        let Ok(n) = back.recv(&mut buf).await else {
            return;
        };
        let at = down.lock().expect("lock").admit(n);
        if let Some(at) = at {
            let dest = Dest::Front(front.clone(), client);
            let _ = down_tx.send(Delivery {
                at,
                data: buf[..n].to_vec(),
                dest,
            });
        }
    }
}
