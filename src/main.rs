// Step 1: can a standalone program regenerate the hub's salt instances exactly?
// Reads a family dump written by the hub (QUIP_DUMP_FAMILY) and compares instance digests.
use quip_solver_conformance::families::export_instances;
use quip_solver_core::ising_generators::{family::BaseTopology, ice::TWISTED_DIMER_V1};
fn hex(s: &str) -> Vec<u8> { (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() }
fn h(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }
fn main() {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
    println!("family {} rev {} params {}", v["family_id"], v["revision"], v["params_hex"]);
    let nodes: Vec<u32> = serde_json::from_value(v["topology"]["nodes"].clone()).unwrap();
    let edges: Vec<(u32, u32)> = serde_json::from_value(v["topology"]["edges"].clone()).unwrap();
    let native: Vec<(u32, u32)> = edges.iter().map(|&(u, v)| (nodes[u as usize], nodes[v as usize])).collect();
    let base = BaseTopology::new(nodes.clone(), &native).unwrap();
    let params = hex(v["params_hex"].as_str().unwrap());
    let tuples = v["tuples"].as_array().unwrap();
    let nonces: Vec<[u8; 32]> = tuples.iter().map(|t| hex(t["nonce"].as_str().unwrap()).try_into().unwrap()).collect();
    let t0 = std::time::Instant::now();
    let res = export_instances(&TWISTED_DIMER_V1, &params, &base, &nonces).unwrap();
    let dt = t0.elapsed();
    let mut ok = 0;
    for (t, r) in tuples.iter().zip(res) {
        match r {
            Ok(i) => {
                let same_digest = h(&i.digest()) == t["digest"].as_str().unwrap();
                let jh: Vec<i64> = i.j().iter().take(24).map(|&x| x as i64).collect();
                let want: Vec<i64> = t["j_head"].as_array().unwrap().iter().map(|x| x.as_i64().unwrap()).collect();
                println!("idx {} digest_match={} j_head_match={} j_len {} (hub {})", t["idx"], same_digest, jh == want, i.j().len(), t["j_len"]);
                if same_digest && jh == want { ok += 1; }
            }
            Err(e) => println!("idx {} generator error {:?}", t["idx"], e),
        }
    }
    println!("{}/{} exact matches; {} instances in {:?}", ok, tuples.len(), tuples.len(), dt);
}
