//! Broker. Today it runs in --sim mode: it plays the hub (issues random salt batches, verifies a sample of what screeners
//! return) so the protocol can be tested without touching a live node.
//! usage: QUIP_SCREENER_TOKEN=... broker --listen ADDR:PORT --family family_dump.json --model model.json [--batches N]
use quip_cpu_screener::{engine::Engine, proto::*};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn hexb(s: &str) -> Vec<u8> { (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() }
fn splitmix(x: &mut u64) -> u64 { *x = x.wrapping_add(0x9e3779b97f4a7c15); let mut z = *x; z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9); z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb); z ^ (z >> 31) }

fn serve(mut s: TcpStream, fam: Arc<Family>, eng: Arc<Engine>, token: [u8; 32], salts: Arc<AtomicU64>, max_batches: u64) -> Result<(), String> {
    s.set_nodelay(true).ok();
    let (k, body) = get(&mut s).map_err(|e| e.to_string())?;
    if k != K_HELLO { return Err("expected HELLO".into()); }
    let mut r = Rd::new(&body);
    if r.u32()? != MAGIC { return Err("bad magic".into()); }
    let _ver = r.u32()?; let tok = r.take(32)?; let threads = r.u32()?; let nl = r.u8()? as usize; let name = String::from_utf8_lossy(r.take(nl)?).to_string();
    let mut diff = 0u8; for (a, b) in tok.iter().zip(token.iter()) { diff |= a ^ b; }
    if diff != 0 { let _ = put(&mut s, K_WELCOME, &[1]); return Err(format!("{name}: bad token")); }
    put(&mut s, K_WELCOME, &enc_welcome(&fam)).map_err(|e| e.to_string())?;
    eprintln!("screener '{name}' connected ({threads} threads)");
    let mut seed = 0x1234_5678_9abc_def0u64 ^ (Instant::now().elapsed().as_nanos() as u64) ^ (name.len() as u64 * 7919);
    let mut rate = 20.0 * threads as f64; // salts/s estimate, adapted from measured batch times
    let (mut id, mut checked, mut bad) = (0u64, 0u64, 0u64);
    while id < max_batches {
        let n = ((rate * 3.0) as usize).clamp(16, 8192); // aim for ~3 s per batch
        let keep = 0.05f32;
        let nonces: Vec<[u8; 32]> = (0..n).map(|_| { let mut x = [0u8; 32]; for c in 0..4 { x[c * 8..c * 8 + 8].copy_from_slice(&splitmix(&mut seed).to_le_bytes()); } x }).collect();
        let mut o = Vec::new();
        o.extend_from_slice(&id.to_le_bytes()); o.extend_from_slice(&fam.epoch.to_le_bytes()); o.extend_from_slice(&keep.to_le_bytes()); o.extend_from_slice(&(n as u32).to_le_bytes());
        for x in &nonces { o.extend_from_slice(x); }
        let t0 = Instant::now();
        put(&mut s, K_BATCH, &o).map_err(|e| e.to_string())?;
        let (k, body) = get(&mut s).map_err(|e| e.to_string())?;
        if k != K_RESULT { return Err("expected RESULT".into()); }
        let el = t0.elapsed().as_secs_f64();
        let mut r = Rd::new(&body);
        let rid = r.u64()?; let _ms = r.u32()?; let kk = r.u32()? as usize;
        if rid != id { return Err("batch id mismatch".into()); }
        let mut passes = Vec::with_capacity(kk);
        for _ in 0..kk { let i = r.u32()? as usize; let sc = r.f64()?; if i >= n { return Err("index out of range".into()); } passes.push((i, sc)); }
        // spot-check: recompute the first two passes ourselves; a dishonest or broken screener gets dropped.
        for &(i, sc) in passes.iter().take(2) {
            checked += 1;
            let mine = eng.score(&nonces[i..i + 1])[0];
            if mine.map_or(true, |m| (m - sc).abs() > 1e-6) { bad += 1; }
        }
        if bad > 0 { return Err(format!("{name}: {bad} of {checked} spot-checks failed")); }
        salts.fetch_add(n as u64, Relaxed);
        rate = 0.5 * rate + 0.5 * (n as f64 / el);
        eprintln!("'{name}' batch {id}: {n} salts in {el:.1}s = {:.0}/s, {} kept, best score {:.1}", n as f64 / el, passes.len(), passes.iter().map(|p| p.1).fold(f64::MAX, f64::min));
        id += 1;
    }
    Ok(())
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let arg = |k: &str| a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned();
    let listen = arg("--listen").unwrap_or("127.0.0.1:7300".into());
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(arg("--family").expect("--family")).unwrap()).unwrap();
    let nodes: Vec<u32> = serde_json::from_value(v["topology"]["nodes"].clone()).unwrap();
    let edges: Vec<(u32, u32)> = serde_json::from_value(v["topology"]["edges"].clone()).unwrap();
    let native: Vec<(u32, u32)> = edges.iter().map(|&(u, w)| (nodes[u as usize], nodes[w as usize])).collect();
    let model: Vec<f64> = serde_json::from_slice(&std::fs::read(arg("--model").expect("--model")).unwrap()).unwrap();
    let fam = Arc::new(Family { epoch: 1, params: hexb(v["params_hex"].as_str().unwrap()), nodes, native_edges: native, model });
    let eng = Arc::new(Engine::new(&fam).unwrap());
    let tok = std::env::var("QUIP_SCREENER_TOKEN").expect("QUIP_SCREENER_TOKEN");
    let mut token = [0u8; 32]; let tb = tok.as_bytes(); token[..tb.len().min(32)].copy_from_slice(&tb[..tb.len().min(32)]);
    let max_batches: u64 = arg("--batches").and_then(|v| v.parse().ok()).unwrap_or(u64::MAX);
    let salts = Arc::new(AtomicU64::new(0));
    let l = TcpListener::bind(&listen).unwrap();
    eprintln!("broker (sim) listening on {listen}");
    let t0 = Instant::now();
    { let salts = salts.clone(); std::thread::spawn(move || loop { std::thread::sleep(Duration::from_secs(10)); eprintln!("total {} salts, {:.0}/s", salts.load(Relaxed), salts.load(Relaxed) as f64 / t0.elapsed().as_secs_f64()); }); }
    for c in l.incoming().flatten() {
        let (fam, eng, salts) = (fam.clone(), eng.clone(), salts.clone());
        std::thread::spawn(move || { if let Err(e) = serve(c, fam, eng, token, salts, max_batches) { eprintln!("screener dropped: {e}"); } });
    }
}
