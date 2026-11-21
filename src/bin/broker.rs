//! Broker: sits next to the hub. Screeners dial it (listen address, e.g. the Tailscale address); the hub dials it too
//! (hub address, e.g. the docker host gateway). The hub sends the salt family once, asks for a free screener, then sends a
//! batch of nonces and receives the best candidates back. The broker spot-checks a few scores of every reply itself.
//!
//! usage: QUIP_SCREENER_TOKEN=... broker --screeners ADDR:PORT --hub ADDR:PORT
//!   or (test): broker --sim --screeners ADDR:PORT --family family_dump.json --model model.json [--batches N]
use quip_cpu_screener::{engine::Engine, proto::*};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn hexb(s: &str) -> Vec<u8> { (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() }
fn splitmix(x: &mut u64) -> u64 { *x = x.wrapping_add(0x9e3779b97f4a7c15); let mut z = *x; z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9); z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb); z ^ (z >> 31) }
fn tok32(s: &str) -> [u8; 32] { let mut t = [0u8; 32]; let b = s.as_bytes(); t[..b.len().min(32)].copy_from_slice(&b[..b.len().min(32)]); t }
fn same(a: &[u8], b: &[u8]) -> bool { let mut d = a.len() ^ b.len(); for (x, y) in a.iter().zip(b.iter()) { d |= (x ^ y) as usize; } d == 0 }

struct Screener { name: String, threads: u32, stream: Mutex<TcpStream>, busy: AtomicBool, dead: AtomicBool, rate: Mutex<f64>, salts: AtomicU64 }
struct State { family: Mutex<Option<(Arc<Family>, Arc<Engine>)>>, screeners: Mutex<Vec<Arc<Screener>>>, token: [u8; 32], salts: AtomicU64 }

fn handle_screener(mut s: TcpStream, st: Arc<State>) -> Result<(), String> {
    s.set_nodelay(true).ok();
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let (k, body) = get(&mut s).map_err(|e| e.to_string())?;
    if k != K_HELLO { return Err("expected HELLO".into()); }
    let mut r = Rd::new(&body);
    if r.u32()? != MAGIC { return Err("bad magic".into()); }
    let _ver = r.u32()?; let tok = r.take(32)?; let threads = r.u32()?; let nl = r.u8()? as usize; let name = String::from_utf8_lossy(r.take(nl)?).to_string();
    if !same(tok, &st.token) { let _ = put(&mut s, K_WELCOME, &[1]); return Err(format!("{name}: bad token")); }
    let fam = match st.family.lock().unwrap().as_ref() { Some((f, _)) => f.clone(), None => { let _ = put(&mut s, K_WELCOME, &[2]); return Err(format!("{name}: no family yet")); } };
    put(&mut s, K_WELCOME, &enc_welcome(&fam)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(120))).ok();
    let sc = Arc::new(Screener { name: name.clone(), threads, stream: Mutex::new(s), busy: AtomicBool::new(false), dead: AtomicBool::new(false), rate: Mutex::new(20.0 * threads as f64), salts: AtomicU64::new(0) });
    st.screeners.lock().unwrap().push(sc.clone());
    eprintln!("screener '{name}' joined ({threads} threads, epoch {})", fam.epoch);
    // The screener is driven by hub requests; this thread only watches for it going away or the family changing.
    loop {
        std::thread::sleep(Duration::from_secs(5));
        if sc.dead.load(Relaxed) { break; }
        let cur = st.family.lock().unwrap().as_ref().map(|(f, _)| f.epoch);
        if cur != Some(fam.epoch) { sc.dead.store(true, Relaxed); break; } // family changed: drop so it reconnects with the new one
    }
    st.screeners.lock().unwrap().retain(|x| !Arc::ptr_eq(x, &sc));
    eprintln!("screener '{name}' left ({} salts)", sc.salts.load(Relaxed));
    Ok(())
}

fn reserve(st: &State, wait: Duration) -> Option<Arc<Screener>> {
    let t0 = Instant::now();
    loop {
        {
            let g = st.screeners.lock().unwrap();
            let mut best: Option<&Arc<Screener>> = None;
            for s in g.iter().filter(|s| !s.dead.load(Relaxed) && !s.busy.load(Relaxed)) {
                if best.map_or(true, |b| *s.rate.lock().unwrap() > *b.rate.lock().unwrap()) { best = Some(s); }
            }
            if let Some(s) = best { if s.busy.compare_exchange(false, true, Relaxed, Relaxed).is_ok() { return Some(s.clone()); } }
        }
        if t0.elapsed() >= wait { return None; }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn handle_hub(mut h: TcpStream, st: Arc<State>) -> Result<(), String> {
    h.set_nodelay(true).ok();
    h.set_read_timeout(Some(Duration::from_secs(120))).ok();
    let (k, body) = get(&mut h).map_err(|e| e.to_string())?;
    if k != K_HUB_HELLO { return Err("expected HUB_HELLO".into()); }
    let mut r = Rd::new(&body);
    if r.u32()? != MAGIC || !same(r.take(32)?, &st.token) { return Err("hub: bad hello".into()); }
    eprintln!("hub connected");
    let mut held: Option<Arc<Screener>> = None;
    let res = (|| -> Result<(), String> { loop {
        let (k, body) = get(&mut h).map_err(|e| e.to_string())?;
        match k {
            K_FAMILY => {
                let fam = dec_welcome(&body)?;
                let cur = st.family.lock().unwrap().as_ref().map(|(f, _)| f.epoch);
                if cur != Some(fam.epoch) {
                    let eng = Arc::new(Engine::new(&fam)?);
                    eprintln!("family epoch {} ({} nodes, {} edges)", fam.epoch, fam.nodes.len(), fam.native_edges.len());
                    *st.family.lock().unwrap() = Some((Arc::new(fam), eng));
                }
            }
            K_SLOT_REQ => {
                let ms = Rd::new(&body).u32()? as u64;
                if let Some(old) = held.take() { old.busy.store(false, Relaxed); }
                match reserve(&st, Duration::from_millis(ms.min(5000))) {
                    Some(s) => { let n = ((*s.rate.lock().unwrap() * 3.0) as u32).clamp(16, 8192); held = Some(s); put(&mut h, K_SLOT_OK, &n.to_le_bytes()).map_err(|e| e.to_string())?; }
                    None => put(&mut h, K_SLOT_NONE, &[]).map_err(|e| e.to_string())?,
                }
            }
            K_SCREEN => {
                let mut r = Rd::new(&body);
                let id = r.u64()?; let keep = r.f32()?; let n = r.u32()? as usize;
                let nonce_bytes = r.take(n * 32)?.to_vec();
                let Some(sc) = held.take() else { put(&mut h, K_SCREEN_REPLY, &enc_screen_reply(id, 2, &[])).map_err(|e| e.to_string())?; continue };
                let (fam, eng) = match st.family.lock().unwrap().clone() { Some(x) => x, None => { sc.busy.store(false, Relaxed); put(&mut h, K_SCREEN_REPLY, &enc_screen_reply(id, 3, &[])).map_err(|e| e.to_string())?; continue } };
                let mut b = Vec::with_capacity(24 + nonce_bytes.len());
                b.extend_from_slice(&id.to_le_bytes()); b.extend_from_slice(&fam.epoch.to_le_bytes()); b.extend_from_slice(&keep.to_le_bytes()); b.extend_from_slice(&(n as u32).to_le_bytes()); b.extend_from_slice(&nonce_bytes);
                let t0 = Instant::now();
                let outcome: Result<Vec<(u32, f64)>, String> = (|| {
                    let mut s = sc.stream.lock().unwrap();
                    put(&mut *s, K_BATCH, &b).map_err(|e| e.to_string())?;
                    let (k, body) = get(&mut *s).map_err(|e| e.to_string())?;
                    if k != K_RESULT { return Err("expected RESULT".into()); }
                    let mut r = Rd::new(&body); let rid = r.u64()?; let _ms = r.u32()?; let kk = r.u32()? as usize;
                    if rid != id { return Err("batch id mismatch".into()); }
                    let mut v = Vec::with_capacity(kk);
                    for _ in 0..kk { let i = r.u32()?; let sc_ = r.f64()?; if i as usize >= n { return Err("index out of range".into()); } v.push((i, sc_)); }
                    Ok(v)
                })();
                match outcome {
                    Ok(passes) => {
                        // spot-check: recompute the first two reported scores ourselves
                        let mut ok = true;
                        for &(i, s) in passes.iter().take(2) {
                            let nonce: [u8; 32] = nonce_bytes[i as usize * 32..i as usize * 32 + 32].try_into().unwrap();
                            if eng.score(&[nonce])[0].map_or(true, |m| (m - s).abs() > 1e-6) { ok = false; }
                        }
                        let el = t0.elapsed().as_secs_f64();
                        if !ok { eprintln!("screener '{}' failed a spot-check; dropped", sc.name); sc.dead.store(true, Relaxed); put(&mut h, K_SCREEN_REPLY, &enc_screen_reply(id, 4, &[])).map_err(|e| e.to_string())?; continue; }
                        { let mut r = sc.rate.lock().unwrap(); *r = 0.5 * *r + 0.5 * (n as f64 / el); }
                        sc.salts.fetch_add(n as u64, Relaxed); st.salts.fetch_add(n as u64, Relaxed);
                        sc.busy.store(false, Relaxed);
                        put(&mut h, K_SCREEN_REPLY, &enc_screen_reply(id, 0, &passes)).map_err(|e| e.to_string())?;
                    }
                    Err(e) => { eprintln!("screener '{}' failed: {e}", sc.name); sc.dead.store(true, Relaxed); put(&mut h, K_SCREEN_REPLY, &enc_screen_reply(id, 1, &[])).map_err(|e| e.to_string())?; }
                }
            }
            _ => return Err(format!("unexpected frame {k} from hub")),
        }
    } })();
    if let Some(s) = held { s.busy.store(false, Relaxed); }
    res
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let arg = |k: &str| a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned();
    let token = tok32(&std::env::var("QUIP_SCREENER_TOKEN").expect("QUIP_SCREENER_TOKEN"));
    let st = Arc::new(State { family: Mutex::new(None), screeners: Mutex::new(Vec::new()), token, salts: AtomicU64::new(0) });
    if a.iter().any(|x| x == "--sim") {
        // test mode: load a family from a file so screeners can join without a hub
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(arg("--family").expect("--family")).unwrap()).unwrap();
        let nodes: Vec<u32> = serde_json::from_value(v["topology"]["nodes"].clone()).unwrap();
        let edges: Vec<(u32, u32)> = serde_json::from_value(v["topology"]["edges"].clone()).unwrap();
        let native: Vec<(u32, u32)> = edges.iter().map(|&(u, w)| (nodes[u as usize], nodes[w as usize])).collect();
        let model: Vec<f64> = serde_json::from_slice(&std::fs::read(arg("--model").expect("--model")).unwrap()).unwrap();
        let fam = Family { epoch: 1, params: hexb(v["params_hex"].as_str().unwrap()), nodes, native_edges: native, model };
        let eng = Arc::new(Engine::new(&fam).unwrap());
        *st.family.lock().unwrap() = Some((Arc::new(fam), eng));
        let lst = TcpListener::bind(arg("--screeners").unwrap_or("127.0.0.1:7300".into())).unwrap();
        eprintln!("broker (sim) accepting screeners");
        let st2 = st.clone();
        std::thread::spawn(move || { // sim driver: act as the hub with one fake connection per screener
            let mut seed = 0x1234u64;
            loop {
                let Some(sc) = reserve(&st2, Duration::from_secs(5)) else { continue };
                let (fam, eng) = st2.family.lock().unwrap().clone().unwrap();
                let n = ((*sc.rate.lock().unwrap() * 3.0) as usize).clamp(16, 8192);
                let nonces: Vec<[u8; 32]> = (0..n).map(|_| { let mut x = [0u8; 32]; for c in 0..4 { x[c * 8..c * 8 + 8].copy_from_slice(&splitmix(&mut seed).to_le_bytes()); } x }).collect();
                let mut b = Vec::new(); b.extend_from_slice(&0u64.to_le_bytes()); b.extend_from_slice(&fam.epoch.to_le_bytes()); b.extend_from_slice(&0.05f32.to_le_bytes()); b.extend_from_slice(&(n as u32).to_le_bytes()); for x in &nonces { b.extend_from_slice(x); }
                let t0 = Instant::now();
                let mut s = sc.stream.lock().unwrap();
                if put(&mut *s, K_BATCH, &b).is_err() { sc.dead.store(true, Relaxed); continue; }
                let Ok((k, body)) = get(&mut *s) else { sc.dead.store(true, Relaxed); continue };
                if k != K_RESULT { sc.dead.store(true, Relaxed); continue; }
                let mut r = Rd::new(&body); let _ = r.u64(); let _ = r.u32(); let kk = r.u32().unwrap() as usize;
                let mut bad = 0; let mut best = f64::MAX;
                for j in 0..kk { let i = r.u32().unwrap() as usize; let sco = r.f64().unwrap(); best = best.min(sco); if j < 2 && eng.score(&nonces[i..i + 1])[0].map_or(true, |m| (m - sco).abs() > 1e-6) { bad += 1; } }
                let el = t0.elapsed().as_secs_f64();
                { let mut rr = sc.rate.lock().unwrap(); *rr = 0.5 * *rr + 0.5 * (n as f64 / el); }
                eprintln!("'{}' {} salts in {el:.1}s = {:.0}/s, {kk} kept, best {best:.1}, spot-check failures {bad}", sc.name, n, n as f64 / el);
                sc.busy.store(false, Relaxed);
            }
        });
        for c in lst.incoming().flatten() { let st = st.clone(); std::thread::spawn(move || { if let Err(e) = handle_screener(c, st) { eprintln!("screener dropped: {e}"); } }); }
        return;
    }
    let sl = TcpListener::bind(arg("--screeners").expect("--screeners ADDR:PORT")).unwrap();
    let hl = TcpListener::bind(arg("--hub").expect("--hub ADDR:PORT")).unwrap();
    eprintln!("broker: screeners on {:?}, hub on {:?}", sl.local_addr().unwrap(), hl.local_addr().unwrap());
    { let st = st.clone(); std::thread::spawn(move || loop { std::thread::sleep(Duration::from_secs(30)); eprintln!("screeners {} | {} salts screened", st.screeners.lock().unwrap().len(), st.salts.load(Relaxed)); }); }
    { let st = st.clone(); std::thread::spawn(move || { for c in sl.incoming().flatten() { let st = st.clone(); std::thread::spawn(move || { if let Err(e) = handle_screener(c, st) { eprintln!("screener dropped: {e}"); } }); } }); }
    for c in hl.incoming().flatten() { let st = st.clone(); std::thread::spawn(move || { if let Err(e) = handle_hub(c, st) { eprintln!("hub connection ended: {e}"); } }); }
}
