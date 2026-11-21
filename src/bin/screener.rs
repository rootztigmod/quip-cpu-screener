//! CPU screener: connects to a broker, pulls batches of salt nonces, returns the best-scoring few.
//! usage: QUIP_SCREENER_TOKEN=... screener --hub HOST:PORT [--threads N] [--name NAME]
use quip_cpu_screener::{engine::Engine, proto::*};
use std::io::Write;
use std::net::TcpStream;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::{Duration, Instant};

fn run(addr: &str, token: &[u8; 32], threads: usize, name: &str) -> Result<(), String> {
    let mut s = TcpStream::connect(addr).map_err(|e| e.to_string())?;
    s.set_nodelay(true).ok();
    let mut hello = Vec::new();
    hello.extend_from_slice(&MAGIC.to_le_bytes()); hello.extend_from_slice(&1u32.to_le_bytes());
    hello.extend_from_slice(token); hello.extend_from_slice(&(threads as u32).to_le_bytes());
    hello.push(name.len().min(255) as u8); hello.extend_from_slice(&name.as_bytes()[..name.len().min(255)]);
    put(&mut s, K_HELLO, &hello).map_err(|e| e.to_string())?;
    let (k, body) = get(&mut s).map_err(|e| e.to_string())?;
    if k != K_WELCOME { return Err("expected WELCOME".into()); }
    let fam = dec_welcome(&body)?;
    let eng = Engine::new(&fam)?;
    eprintln!("connected to {addr}: epoch {} topology {} nodes {} edges, {threads} threads", fam.epoch, fam.nodes.len(), fam.native_edges.len());
    loop {
        let (k, body) = get(&mut s).map_err(|e| e.to_string())?;
        match k {
            K_BATCH => {
                let mut r = Rd::new(&body);
                let id = r.u64()?; let _epoch = r.u64()?; let keep = r.f32()?; let n = r.u32()? as usize;
                let mut nonces: Vec<[u8; 32]> = Vec::with_capacity(n);
                for _ in 0..n { nonces.push(r.take(32)?.try_into().unwrap()); }
                let t0 = Instant::now();
                let scores: Mutex<Vec<(u32, f64)>> = Mutex::new(Vec::with_capacity(n));
                let next = AtomicUsize::new(0);
                std::thread::scope(|sc| {
                    for _ in 0..threads {
                        sc.spawn(|| loop {
                            let a = next.fetch_add(8, Relaxed);
                            if a >= n { break; }
                            let b = (a + 8).min(n);
                            let part = eng.score(&nonces[a..b]);
                            let mut g = scores.lock().unwrap();
                            for (i, sc_) in part.into_iter().enumerate() { if let Some(x) = sc_ { g.push(((a + i) as u32, x)); } }
                        });
                    }
                });
                let mut v = scores.into_inner().unwrap();
                v.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                v.truncate(((n as f64) * keep as f64).ceil() as usize);
                let mut o = Vec::with_capacity(16 + v.len() * 12);
                o.extend_from_slice(&id.to_le_bytes()); o.extend_from_slice(&(t0.elapsed().as_millis() as u32).to_le_bytes());
                o.extend_from_slice(&(v.len() as u32).to_le_bytes());
                for (i, x) in &v { o.extend_from_slice(&i.to_le_bytes()); o.extend_from_slice(&x.to_le_bytes()); }
                put(&mut s, K_RESULT, &o).map_err(|e| e.to_string())?;
            }
            K_CANCEL => {}
            _ => return Err(format!("unexpected frame {k}")),
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let arg = |k: &str| a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned();
    let addr = arg("--hub").expect("--hub HOST:PORT");
    let threads = arg("--threads").and_then(|v| v.parse().ok()).unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()));
    let name = arg("--name").unwrap_or_else(|| "screener".into());
    let tok = std::env::var("QUIP_SCREENER_TOKEN").expect("QUIP_SCREENER_TOKEN");
    let mut token = [0u8; 32]; let tb = tok.as_bytes(); token[..tb.len().min(32)].copy_from_slice(&tb[..tb.len().min(32)]);
    let mut wait = 1;
    loop {
        match run(&addr, &token, threads, &name) { Ok(()) => {}, Err(e) => eprintln!("disconnected: {e}") }
        std::io::stderr().flush().ok();
        std::thread::sleep(Duration::from_secs(wait)); wait = (wait * 2).min(30);
    }
}
