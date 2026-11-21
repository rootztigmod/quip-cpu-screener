// Ported verbatim from the hub solver (AGPL-3.0-or-later): the 20-number structural CPU screen.
#![allow(dead_code)]
use std::collections::{HashMap,VecDeque};
use std::sync::{Arc,Mutex};
pub struct Cycles{key:u64,tri:Vec<[u32;3]>,cyc:Vec<[u32;4]>}
static CACHE:Mutex<Option<Arc<Cycles>>>=Mutex::new(None);

fn key_of(edges:&[(u32,u32)])->u64{use std::hash::{Hash,Hasher};let mut h=std::collections::hash_map::DefaultHasher::new();edges.hash(&mut h);h.finish()}
pub fn cycles(edges:&[(u32,u32)])->Arc<Cycles>{
    let k=key_of(edges);
    {let g=CACHE.lock().unwrap_or_else(|e|e.into_inner());if let Some(c)=g.as_ref(){if c.key==k{return c.clone();}}}
    let c=Arc::new(build(edges,k));
    *CACHE.lock().unwrap_or_else(|e|e.into_inner())=Some(c.clone());c
}
fn build(edges:&[(u32,u32)],key:u64)->Cycles{
    let n=edges.iter().map(|&(u,v)|u.max(v)).max().map_or(0,|x|x as usize+1);
    let mut adj:Vec<Vec<u32>>=vec![Vec::new();n];let mut eid:HashMap<u64,u32>=HashMap::with_capacity(edges.len()*2);
    let ek=|a:u32,b:u32|((a.min(b) as u64)<<32)|(a.max(b) as u64);
    for (k,&(u,v)) in edges.iter().enumerate(){adj[u as usize].push(v);adj[v as usize].push(u);eid.insert(ek(u,v),k as u32);}
    for a in adj.iter_mut(){a.sort_unstable();}
    let mut tri=Vec::new();
    for &(u,v) in edges{let (a,b)=(u.min(v),u.max(v));let (x,y)=(&adj[a as usize],&adj[b as usize]);let (mut i,mut j)=(0,0);
        while i<x.len()&&j<y.len(){if x[i]==y[j]{let w=x[i];if w>b{tri.push([eid[&ek(a,b)],eid[&ek(a,w)],eid[&ek(b,w)]]);}i+=1;j+=1;}else if x[i]<y[j]{i+=1;}else{j+=1;}}}
    let mut wed:HashMap<u64,Vec<u32>>=HashMap::new();
    for v in 0..n{let nb=&adj[v];for i in 0..nb.len(){for j in i+1..nb.len(){wed.entry(((nb[i] as u64)<<32)|nb[j] as u64).or_default().push(v as u32);}}}
    let mut cyc=Vec::new();
    for (kk,mids) in &wed{if mids.len()<2{continue;}let (u,w)=((kk>>32) as u32,(kk&0xffff_ffff) as u32);
        for i in 0..mids.len(){for j in i+1..mids.len(){let (a,b)=(mids[i].min(mids[j]),mids[i].max(mids[j]));
            if u<a&&u<w{cyc.push([eid[&ek(u,a)],eid[&ek(a,w)],eid[&ek(w,b)],eid[&ek(b,u)]]);}}}}
    Cycles{key,tri,cyc}
}
pub fn features(c:&Cycles,j:&[i32])->[f64;20]{
    let sg=|e:u32|if j[e as usize]>0{1i32}else{-1};let st=|e:u32|j[e as usize].abs()>500;
    let mut o=[0f64;20];
    for t in &c.tri{let ns=st(t[0]) as usize+st(t[1]) as usize+st(t[2]) as usize;let f=sg(t[0])*sg(t[1])*sg(t[2])==1;o[2*ns+if f{0}else{1}]+=1.;}
    for q in &c.cyc{let (s0,s1,s2,s3)=(st(q[0]),st(q[1]),st(q[2]),st(q[3]));let n4=s0 as usize+s1 as usize+s2 as usize+s3 as usize;
        let f=sg(q[0])*sg(q[1])*sg(q[2])*sg(q[3])==-1;
        let idx=match n4{0=>8,1=>10,2=>{if (s0&&s2)||(s1&&s3){12}else{14}},3=>16,_=>18};
        o[idx+if f{0}else{1}]+=1.;}
    o
}
pub fn score(model:&[f64],x:&[f64;20])->f64{let mut s=model[60];for i in 0..20{s+=model[40+i]*(x[i]-model[i])/(model[20+i]+1e-9);}s}
