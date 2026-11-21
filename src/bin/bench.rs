// Step 2: per-salt CPU cost, split into generation and screening, single thread and multi-thread.
#[path = "../cpu_screen.rs"]
mod cpu_screen;
use quip_solver_conformance::families::export_instances;
use quip_solver_core::ising_generators::{family::BaseTopology, ice::TWISTED_DIMER_V1};
use std::time::Instant;
fn hex(s: &str) -> Vec<u8> { (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() }
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&a[1]).unwrap()).unwrap();
    let model: Vec<f64> = serde_json::from_slice(&std::fs::read(&a[2]).unwrap()).unwrap();
    let n: usize = a[3].parse().unwrap(); let threads: usize = a[4].parse().unwrap();
    let nodes: Vec<u32> = serde_json::from_value(v["topology"]["nodes"].clone()).unwrap();
    let edges: Vec<(u32, u32)> = serde_json::from_value(v["topology"]["edges"].clone()).unwrap();
    let native: Vec<(u32, u32)> = edges.iter().map(|&(u, v)| (nodes[u as usize], nodes[v as usize])).collect();
    let params = hex(v["params_hex"].as_str().unwrap());
    let t0 = Instant::now();
    let total = std::sync::atomic::AtomicU64::new(0);
    let (gen_ns, scr_ns) = (std::sync::atomic::AtomicU64::new(0), std::sync::atomic::AtomicU64::new(0));
    std::thread::scope(|sc| {
        for th in 0..threads {
            let (nodes, native, params, model, total, gen_ns, scr_ns) = (&nodes, &native, &params, &model, &total, &gen_ns, &scr_ns);
            sc.spawn(move || {
                let base = BaseTopology::new(nodes.clone(), native).unwrap();
                let mut seed = 0x9e3779b97f4a7c15u64 ^ (th as u64 * 7919);
                let mut x = |s: &mut u64| { *s = s.wrapping_add(0x9e3779b97f4a7c15); let mut z = *s; z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9); z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb); z ^ (z >> 31) };
                let per = n / threads; let batch = 8;
                let mut done = 0;
                while done < per {
                    let nonces: Vec<[u8; 32]> = (0..batch).map(|_| { let mut nn = [0u8; 32]; for c in 0..4 { nn[c * 8..c * 8 + 8].copy_from_slice(&x(&mut seed).to_le_bytes()); } nn }).collect();
                    let g0 = Instant::now();
                    let res = export_instances(&TWISTED_DIMER_V1, params, &base, &nonces).unwrap();
                    gen_ns.fetch_add(g0.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
                    for r in res {
                        let inst = r.unwrap();
                        let s0 = Instant::now();
                        let cy = cpu_screen::cycles(inst.edges());
                        let f = cpu_screen::features(&cy, inst.j());
                        let sc_ = cpu_screen::score(model, &f);
                        std::hint::black_box(sc_);
                        scr_ns.fetch_add(s0.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
                        total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    done += batch;
                }
            });
        }
    });
    let el = t0.elapsed().as_secs_f64(); let t = total.load(std::sync::atomic::Ordering::Relaxed) as f64;
    println!("threads {threads}: {t} salts in {el:.2}s = {:.0} salts/s total, {:.1} per thread; per-salt generate {:.1} ms, screen {:.1} ms", t / el, t / el / threads as f64, gen_ns.load(std::sync::atomic::Ordering::Relaxed) as f64 / t / 1e6, scr_ns.load(std::sync::atomic::Ordering::Relaxed) as f64 / t / 1e6);
}
