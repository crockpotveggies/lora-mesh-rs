use loramesh::mesh::routing::{Router, decode_links, encode_links};
use std::collections::{BTreeMap, BTreeSet};
fn edges(items: &[(u16, u32)]) -> BTreeMap<u16, u32> {
    items.iter().copied().collect()
}
#[test]
fn diamond_alternates_hysteresis_and_expiry() {
    let mut r = Router::new(1);
    r.update(1, edges(&[(2, 100), (3, 110)]), u64::MAX);
    r.update(2, edges(&[(1, 100), (4, 100)]), 1000);
    r.update(3, edges(&[(1, 110), (4, 100)]), 2000);
    r.update(4, edges(&[(2, 100), (3, 100)]), 2000);
    r.rebuild();
    assert_eq!(r.routes[&4].next, 2);
    r.update(1, edges(&[(2, 100), (3, 90)]), u64::MAX);
    r.rebuild();
    assert_eq!(r.routes[&4].next, 2);
    r.update(1, edges(&[(2, 100), (3, 10)]), u64::MAX);
    r.rebuild();
    assert_eq!(r.routes[&4].next, 3);
    r.update(1, edges(&[(2, 100), (3, 110)]), u64::MAX);
    r.rebuild();
    assert_eq!(r.routes[&4].next, 3);
    assert!(r.expire(1000));
    r.rebuild();
    assert_eq!(r.routes[&4].next, 3);
    assert!(!r.topology[&4].links.is_empty());
    r.expire(2000);
    r.rebuild();
    assert!(r.routes.is_empty());
}
#[test]
fn unreciprocated_edges_and_invalid_advertisements() {
    let mut r = Router::new(1);
    r.update(1, edges(&[(2, 1)]), u64::MAX);
    r.update(2, edges(&[(3, 1)]), 1000);
    r.rebuild();
    assert!(r.routes.is_empty());
    let members = BTreeSet::from([1, 2, 3]);
    assert_eq!(
        decode_links(&encode_links(&edges(&[(2, 123)])).unwrap(), 1, &members).unwrap(),
        edges(&[(2, 123)])
    );
    for bytes in [
        vec![],
        vec![33],
        vec![1],
        vec![1, 0, 1, 0, 0, 0, 1],
        vec![1, 0, 2, 0, 0, 0, 0],
    ] {
        assert!(decode_links(&bytes, 1, &members).is_err());
    }
}
#[test]
fn intended_next_hop_and_longer_route_cost() {
    let mut r = Router::new(1);
    for (id, e) in [
        (1, edges(&[(2, 10), (4, 100)])),
        (2, edges(&[(1, 10), (3, 10)])),
        (3, edges(&[(2, 10), (4, 10)])),
        (4, edges(&[(3, 10), (1, 100)])),
    ] {
        r.update(id, e, 1000);
    }
    r.rebuild();
    assert_eq!(r.routes[&4].next, 2);
    assert_eq!(r.routes[&4].cost, 30);
    assert!(!r.routes.contains_key(&1));
}
