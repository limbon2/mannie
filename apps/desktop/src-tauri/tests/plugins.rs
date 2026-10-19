//! The plugins that `task gen` found and the binary bundles (ADR-0035,
//! decisions 3 and 4): every manifest is one that the host accepts, the list
//! comes in the order of the plugins' starts, and the host builds its list
//! from it, which refuses two plugins with one ID or with prefixes that nest.
//! With the feature `validation`, the list also holds the fixture plugins.

use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugins::PluginList;

#[test]
fn the_bundled_plugins_have_valid_manifests_in_the_order_of_their_starts() {
    let manifests: Vec<Manifest> = mannie_plugin_list::plugins()
        .iter()
        .map(|entry| Manifest::parse(entry.manifest).unwrap())
        .collect();
    let mut ordered = manifests.clone();
    ordered.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));

    let list = PluginList::new(mannie_plugin_list::plugins());

    assert_ne!(manifests, Vec::<Manifest>::new());
    assert_eq!(manifests, ordered);
    assert!(list.is_ok(), "{:?}", list.err());
}
