//! Wire format shared by the screener and the broker (see DESIGN.md).
use std::io::{Read, Write};
pub const MAGIC: u32 = 0x5153_5231;
pub const K_HELLO: u8 = 1;
pub const K_WELCOME: u8 = 2;
pub const K_BATCH: u8 = 3;
pub const K_RESULT: u8 = 4;
pub const K_CANCEL: u8 = 5;
pub const MAX_FRAME: usize = 64 << 20;

pub fn put(w: &mut impl Write, kind: u8, body: &[u8]) -> std::io::Result<()> {
    let mut b = Vec::with_capacity(5 + body.len());
    b.push(kind);
    b.extend_from_slice(&(body.len() as u32).to_le_bytes());
    b.extend_from_slice(body);
    w.write_all(&b)?;
    w.flush()
}
pub fn get(r: &mut impl Read) -> std::io::Result<(u8, Vec<u8>)> {
    let mut h = [0u8; 5];
    r.read_exact(&mut h)?;
    let n = u32::from_le_bytes([h[1], h[2], h[3], h[4]]) as usize;
    if n > MAX_FRAME { return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large")); }
    let mut body = vec![0u8; n];
    r.read_exact(&mut body)?;
    Ok((h[0], body))
}

pub struct Rd<'a> { pub b: &'a [u8], pub at: usize }
impl<'a> Rd<'a> {
    pub fn new(b: &'a [u8]) -> Self { Self { b, at: 0 } }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.at + n > self.b.len() { return Err("short frame".into()); }
        let s = &self.b[self.at..self.at + n]; self.at += n; Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8, String> { Ok(self.take(1)?[0]) }
    pub fn u32(&mut self) -> Result<u32, String> { Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    pub fn u64(&mut self) -> Result<u64, String> { Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap())) }
    pub fn f32(&mut self) -> Result<f32, String> { Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    pub fn f64(&mut self) -> Result<f64, String> { Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap())) }
}

/// Everything a screener needs to generate and score instances.
#[derive(Clone)]
pub struct Family { pub epoch: u64, pub params: Vec<u8>, pub nodes: Vec<u32>, pub native_edges: Vec<(u32, u32)>, pub model: Vec<f64> }

pub fn enc_welcome(f: &Family) -> Vec<u8> {
    let mut o = vec![0u8];
    o.extend_from_slice(&f.epoch.to_le_bytes());
    o.extend_from_slice(&(f.params.len() as u32).to_le_bytes()); o.extend_from_slice(&f.params);
    o.extend_from_slice(&(f.nodes.len() as u32).to_le_bytes()); for n in &f.nodes { o.extend_from_slice(&n.to_le_bytes()); }
    o.extend_from_slice(&(f.native_edges.len() as u32).to_le_bytes()); for (u, v) in &f.native_edges { o.extend_from_slice(&u.to_le_bytes()); o.extend_from_slice(&v.to_le_bytes()); }
    for m in &f.model { o.extend_from_slice(&m.to_le_bytes()); }
    o
}
pub fn dec_welcome(b: &[u8]) -> Result<Family, String> {
    let mut r = Rd::new(b);
    let status = r.u8()?; if status != 0 { return Err(format!("welcome refused (status {status})")); }
    let epoch = r.u64()?;
    let pl = r.u32()? as usize; let params = r.take(pl)?.to_vec();
    let nn = r.u32()? as usize; let mut nodes = Vec::with_capacity(nn); for _ in 0..nn { nodes.push(r.u32()?); }
    let ne = r.u32()? as usize; let mut native_edges = Vec::with_capacity(ne); for _ in 0..ne { let u = r.u32()?; let v = r.u32()?; native_edges.push((u, v)); }
    let mut model = Vec::with_capacity(61); for _ in 0..61 { model.push(r.f64()?); }
    Ok(Family { epoch, params, nodes, native_edges, model })
}

// ---- hub <-> broker frames ----
pub const K_HUB_HELLO: u8 = 14; // magic u32, token 32 B
pub const K_FAMILY: u8 = 8;     // same body as WELCOME (status 0 + family)
pub const K_SLOT_REQ: u8 = 9;   // timeout ms u32
pub const K_SLOT_OK: u8 = 10;   // suggested batch size u32
pub const K_SLOT_NONE: u8 = 11; // no idle screener
pub const K_SCREEN: u8 = 12;    // id u64, keep f32, n u32, n * 32 B nonces
pub const K_SCREEN_REPLY: u8 = 13; // id u64, status u8 (0 ok), k u32, k * (index u32, score f64)

pub fn enc_screen_reply(id: u64, status: u8, passes: &[(u32, f64)]) -> Vec<u8> {
    let mut o = Vec::with_capacity(13 + passes.len() * 12);
    o.extend_from_slice(&id.to_le_bytes()); o.push(status); o.extend_from_slice(&(passes.len() as u32).to_le_bytes());
    for (i, s) in passes { o.extend_from_slice(&i.to_le_bytes()); o.extend_from_slice(&s.to_le_bytes()); }
    o
}
pub fn dec_screen_reply(b: &[u8]) -> Result<(u64, u8, Vec<(u32, f64)>), String> {
    let mut r = Rd::new(b); let id = r.u64()?; let st = r.u8()?; let k = r.u32()? as usize;
    let mut v = Vec::with_capacity(k); for _ in 0..k { let i = r.u32()?; let s = r.f64()?; v.push((i, s)); }
    Ok((id, st, v))
}
