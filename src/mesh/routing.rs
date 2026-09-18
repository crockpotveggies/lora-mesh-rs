//! Signed link-state graph. Immediate authenticated peers, not origins, establish neighbors.
use crate::radio::protocol::invalid;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
#[derive(Clone, Debug)]
pub struct Advertisement {
    pub links: BTreeMap<u16, u32>,
    pub expires: u64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Route {
    pub next: u16,
    pub cost: u64,
}
pub struct Router {
    pub node: u16,
    pub topology: BTreeMap<u16, Advertisement>,
    pub routes: BTreeMap<u16, Route>,
    pub recomputations: u64,
}
impl Router {
    pub fn new(node: u16) -> Self {
        Self {
            node,
            topology: BTreeMap::new(),
            routes: BTreeMap::new(),
            recomputations: 0,
        }
    }
    pub fn update(&mut self, node: u16, links: BTreeMap<u16, u32>, expires: u64) -> bool {
        let changed = self
            .topology
            .get(&node)
            .map(|a| a.links != links)
            .unwrap_or(true);
        self.topology.insert(node, Advertisement { links, expires });
        changed
    }
    pub fn expire(&mut self, now: u64) -> bool {
        let old = self.topology.len();
        self.topology
            .retain(|node, a| *node == self.node || a.expires > now);
        old != self.topology.len()
    }
    fn graph(&self) -> BTreeMap<u16, BTreeMap<u16, u32>> {
        self.topology
            .iter()
            .map(|(&origin, a)| {
                let edges = a
                    .links
                    .iter()
                    .filter(|(peer, _)| {
                        self.topology
                            .get(peer)
                            .map(|p| p.links.contains_key(&origin))
                            .unwrap_or(false)
                    })
                    .map(|(p, c)| (*p, *c))
                    .collect();
                (origin, edges)
            })
            .collect()
    }
    fn shortest(
        graph: &BTreeMap<u16, BTreeMap<u16, u32>>,
        start: u16,
        avoid: Option<u16>,
    ) -> BTreeMap<u16, (u64, u16)> {
        let mut distances = BTreeMap::from([(start, (0u64, start))]);
        let mut visited = BTreeSet::new();
        while let Some((node, (cost, first))) = distances
            .iter()
            .filter(|(n, _)| !visited.contains(*n))
            .min_by_key(|(n, (c, _))| (*c, **n))
            .map(|(n, v)| (*n, *v))
        {
            visited.insert(node);
            if let Some(edges) = graph.get(&node) {
                for (&next, &edge) in edges {
                    if Some(next) == avoid || visited.contains(&next) {
                        continue;
                    }
                    let candidate = (
                        cost + u64::from(edge),
                        if node == start { next } else { first },
                    );
                    if distances
                        .get(&next)
                        .map(|old| candidate < *old)
                        .unwrap_or(true)
                    {
                        distances.insert(next, candidate);
                    }
                }
            }
        }
        distances
    }
    pub fn rebuild(&mut self) {
        self.recomputations += 1;
        let graph = self.graph();
        let shortest = Self::shortest(&graph, self.node, None);
        let mut routes = BTreeMap::new();
        // Cache the alternate-path searches once per old first-hop, not once per destination.
        let mut alternatives = BTreeMap::new();
        for (&destination, &(cost, next)) in &shortest {
            if destination == self.node {
                continue;
            }
            let mut chosen = Route { next, cost };
            if let Some(old) = self.routes.get(&destination) {
                if old.next != next {
                    if let Some(edge) = graph.get(&self.node).and_then(|e| e.get(&old.next)) {
                        let paths = alternatives
                            .entry(old.next)
                            .or_insert_with(|| Self::shortest(&graph, old.next, Some(self.node)));
                        if let Some((remaining, _)) = paths.get(&destination) {
                            let old_cost = u64::from(*edge) + remaining;
                            if cost * 100 >= old_cost * 80 {
                                chosen = Route {
                                    next: old.next,
                                    cost: old_cost,
                                };
                            }
                        }
                    }
                }
            }
            routes.insert(destination, chosen);
        }
        self.routes = routes;
    }
}
pub fn encode_links(links: &BTreeMap<u16, u32>) -> io::Result<Vec<u8>> {
    if links.len() > 32 {
        return Err(invalid("neighbor cap"));
    }
    let mut bytes = vec![links.len() as u8];
    for (peer, cost) in links {
        if *peer == 0 || *cost == 0 {
            return Err(invalid("invalid link cost"));
        }
        bytes.extend_from_slice(&peer.to_be_bytes());
        bytes.extend_from_slice(&cost.to_be_bytes());
    }
    Ok(bytes)
}
pub fn decode_links(
    bytes: &[u8],
    origin: u16,
    members: &BTreeSet<u16>,
) -> io::Result<BTreeMap<u16, u32>> {
    if bytes.is_empty() || bytes[0] > 32 || bytes.len() != 1 + usize::from(bytes[0]) * 6 {
        return Err(invalid("invalid announcement length"));
    }
    let mut out = BTreeMap::new();
    for item in bytes[1..].chunks_exact(6) {
        let peer = u16::from_be_bytes(item[..2].try_into().unwrap());
        let cost = u32::from_be_bytes(item[2..].try_into().unwrap());
        if peer == origin
            || !members.contains(&peer)
            || cost == 0
            || cost > 1_000_000_000
            || out.insert(peer, cost).is_some()
        {
            return Err(invalid("invalid announcement edge"));
        }
    }
    Ok(out)
}
