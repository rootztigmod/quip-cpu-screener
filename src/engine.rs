//! Generate instances from nonces and score them with the CPU screen.
use crate::{cpu_screen, proto::Family};
use quip_solver_conformance::families::export_instances;
use quip_solver_core::ising_generators::{family::BaseTopology, ice::TWISTED_DIMER_V1};

pub struct Engine { base: BaseTopology, params: Vec<u8>, model: Vec<f64> }

impl Engine {
    pub fn new(f: &Family) -> Result<Self, String> {
        let base = BaseTopology::new(f.nodes.clone(), &f.native_edges).map_err(|e| format!("topology: {e:?}"))?;
        Ok(Self { base, params: f.params.clone(), model: f.model.clone() })
    }
    /// Score of every nonce (lower is better); `None` if the generator refused the nonce.
    pub fn score(&self, nonces: &[[u8; 32]]) -> Vec<Option<f64>> {
        match export_instances(&TWISTED_DIMER_V1, &self.params, &self.base, nonces) {
            Ok(res) => res.into_iter().map(|r| r.ok().map(|inst| {
                let cy = cpu_screen::cycles(inst.edges());
                cpu_screen::score(&self.model, &cpu_screen::features(&cy, inst.j()))
            })).collect(),
            Err(_) => vec![None; nonces.len()],
        }
    }
}
