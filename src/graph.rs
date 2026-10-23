//! Dependency graph, cycles, explicit-root paths and exact removal impact.
//!
//! Packages are addressed by their index in [`Graph::packages`].  All adjacency
//! lists therefore contain `usize` indices and never borrow package data.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use crate::model::{Package, normalize_dependency_name, provided_version, version_satisfies};

/// A resolved hard dependency edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyEdge {
    /// Original requirement, including any version operator.
    pub requirement: String,
    /// Index of the package that satisfies the requirement.
    pub target: usize,
}

/// A dependency that could not be satisfied by any installed package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedDependency {
    pub package: usize,
    pub requirement: String,
    pub optional: bool,
}

#[derive(Debug, Clone)]
struct ProviderEntry {
    package: usize,
    version: Option<String>,
}

/// Dependency graph indexed by package position in [`Graph::packages`].
#[derive(Debug, Clone)]
pub struct Graph {
    pub packages: Vec<Package>,
    /// Hard dependencies, one list per package.
    pub dependency_edges: Vec<Vec<DependencyEdge>>,
    /// Hard dependency adjacency (`dependency_edges[*].target`).
    pub depends: Vec<Vec<usize>>,
    /// Reverse hard dependency adjacency.
    pub dependents: Vec<Vec<usize>>,
    /// Optional dependencies, kept completely separate from hard edges.
    pub opt_dependency_edges: Vec<Vec<DependencyEdge>>,
    pub opt_depends: Vec<Vec<usize>>,
    pub opt_dependents: Vec<Vec<usize>>,
    pub unresolved: Vec<UnresolvedDependency>,
    /// Strongly connected components of size > 1 plus self loops.
    pub cycles: Vec<Vec<usize>>,
    provider_index: HashMap<String, Vec<ProviderEntry>>,
}

impl Graph {
    /// Build a graph by cloning a package slice.
    pub fn build(packages: &[Package]) -> Self {
        Self::new(packages.to_vec())
    }

    /// Build a graph from a snapshot.
    pub fn from_snapshot(snapshot: &crate::model::Snapshot) -> Self {
        Self::build(&snapshot.packages)
    }

    /// Build a graph, taking ownership of the package vector.
    pub fn new(packages: Vec<Package>) -> Self {
        let count = packages.len();
        let mut provider_index: HashMap<String, Vec<ProviderEntry>> = HashMap::new();

        for (index, package) in packages.iter().enumerate() {
            add_provider(
                &mut provider_index,
                &package.name,
                ProviderEntry {
                    package: index,
                    version: Some(package.version.clone()).filter(|v| !v.is_empty()),
                },
            );
            for provide in &package.provides {
                add_provider(
                    &mut provider_index,
                    provide,
                    ProviderEntry {
                        package: index,
                        version: provided_version(provide).map(ToString::to_string),
                    },
                );
            }
        }

        let mut dependency_edges = vec![Vec::new(); count];
        let mut opt_dependency_edges = vec![Vec::new(); count];
        let mut unresolved = Vec::new();

        for (index, package) in packages.iter().enumerate() {
            let mut seen = HashSet::new();
            for requirement in &package.depends {
                if let Some(target) = resolve_with_packages(&provider_index, requirement, &packages)
                {
                    if seen.insert(target) {
                        dependency_edges[index].push(DependencyEdge {
                            requirement: requirement.clone(),
                            target,
                        });
                    }
                } else {
                    unresolved.push(UnresolvedDependency {
                        package: index,
                        requirement: requirement.clone(),
                        optional: false,
                    });
                }
            }

            let mut seen_optional = HashSet::new();
            for requirement in &package.optdepends {
                if let Some(target) = resolve_with_packages(&provider_index, requirement, &packages)
                {
                    if seen_optional.insert(target) {
                        opt_dependency_edges[index].push(DependencyEdge {
                            requirement: requirement.clone(),
                            target,
                        });
                    }
                } else {
                    unresolved.push(UnresolvedDependency {
                        package: index,
                        requirement: requirement.clone(),
                        optional: true,
                    });
                }
            }
        }

        let depends: Vec<Vec<usize>> = dependency_edges
            .iter()
            .map(|edges| edges.iter().map(|edge| edge.target).collect())
            .collect();
        let opt_depends: Vec<Vec<usize>> = opt_dependency_edges
            .iter()
            .map(|edges| edges.iter().map(|edge| edge.target).collect())
            .collect();

        let mut dependents = vec![Vec::new(); count];
        let mut opt_dependents = vec![Vec::new(); count];
        for (source, targets) in depends.iter().enumerate() {
            for target in targets {
                if let Some(list) = dependents.get_mut(*target)
                    && !list.contains(&source)
                {
                    list.push(source);
                }
            }
        }
        for (source, targets) in opt_depends.iter().enumerate() {
            for target in targets {
                if let Some(list) = opt_dependents.get_mut(*target)
                    && !list.contains(&source)
                {
                    list.push(source);
                }
            }
        }

        let mut graph = Self {
            packages,
            dependency_edges,
            depends,
            dependents,
            opt_dependency_edges,
            opt_depends,
            opt_dependents,
            unresolved,
            cycles: Vec::new(),
            provider_index,
        };
        graph.cycles = graph.find_cycles();
        graph
    }

    pub fn len(&self) -> usize {
        self.packages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    pub fn package_name(&self, index: usize) -> Option<&str> {
        self.packages
            .get(index)
            .map(|package| package.name.as_str())
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.packages
            .iter()
            .position(|package| package.name == name)
    }

    pub fn explicit_roots(&self) -> Vec<usize> {
        self.packages
            .iter()
            .enumerate()
            .filter(|(_, package)| package.explicit)
            .map(|(index, _)| index)
            .collect()
    }

    /// All packages that can be reached from an explicitly installed package.
    pub fn reachable_from_explicit(&self) -> BTreeSet<usize> {
        let mut reachable = BTreeSet::new();
        let mut queue: VecDeque<usize> = self.explicit_roots().into();

        while let Some(index) = queue.pop_front() {
            if !reachable.insert(index) {
                continue;
            }
            if let Some(targets) = self.depends.get(index) {
                for target in targets {
                    if !reachable.contains(target) {
                        queue.push_back(*target);
                    }
                }
            }
        }

        reachable
    }

    /// Resolve a requirement to all matching installed providers, best first.
    pub fn resolve_all(&self, requirement: &str) -> Vec<usize> {
        let key = normalize_dependency_name(requirement);
        let mut candidates = self
            .provider_index
            .get(&key)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|entry| match &entry.version {
                        Some(version) => version_satisfies(version, requirement),
                        None => true,
                    })
                    .map(|entry| entry.package)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        candidates.sort_by_key(|index| {
            let package_matches = self
                .packages
                .get(*index)
                .map(|package| normalize_dependency_name(&package.name) == key)
                .unwrap_or(false);
            (!package_matches, *index)
        });
        candidates.dedup();
        candidates
    }

    /// Resolve a requirement to its best provider, if any.
    pub fn resolve(&self, requirement: &str) -> Option<usize> {
        self.resolve_all(requirement).into_iter().next()
    }

    /// Names of unresolved hard dependencies.
    pub fn unresolved_hard(&self) -> Vec<&UnresolvedDependency> {
        self.unresolved
            .iter()
            .filter(|dependency| !dependency.optional)
            .collect()
    }

    /// Shortest dependency path from any explicit root to every reachable
    /// package.  Each returned path starts with the explicit root.
    pub fn shortest_paths_from_explicit(&self) -> Vec<Option<Vec<usize>>> {
        let mut paths: Vec<Option<Vec<usize>>> = vec![None; self.len()];
        let mut queue: VecDeque<usize> = VecDeque::new();

        for root in self.explicit_roots() {
            if paths.get(root).is_some_and(Option::is_none) {
                paths[root] = Some(vec![root]);
                queue.push_back(root);
            }
        }

        while let Some(index) = queue.pop_front() {
            let Some(current_path) = paths.get(index).and_then(|path| path.clone()) else {
                continue;
            };
            if let Some(targets) = self.depends.get(index) {
                for target in targets {
                    if paths.get(*target).is_some_and(Option::is_none) {
                        let mut path = current_path.clone();
                        path.push(*target);
                        paths[*target] = Some(path);
                        queue.push_back(*target);
                    }
                }
            }
        }

        paths
    }

    /// Enumerate simple dependency paths starting at each explicit root.
    /// `max_paths` bounds the result to keep unusual graphs predictable.
    pub fn paths_from_explicit(&self, max_paths: usize) -> Vec<Vec<usize>> {
        let mut out = Vec::new();
        let mut on_path = vec![false; self.len()];

        for root in self.explicit_roots() {
            let mut path = Vec::new();
            self.collect_paths(root, &mut path, &mut on_path, max_paths, &mut out);
            if out.len() >= max_paths {
                break;
            }
        }

        out
    }

    /// Enumerate simple dependency paths from one root.
    pub fn paths_from(&self, root: usize, max_paths: usize) -> Vec<Vec<usize>> {
        if root >= self.len() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut on_path = vec![false; self.len()];
        self.collect_paths(root, &mut Vec::new(), &mut on_path, max_paths, &mut out);
        out
    }

    fn collect_paths(
        &self,
        index: usize,
        path: &mut Vec<usize>,
        on_path: &mut [bool],
        max_paths: usize,
        out: &mut Vec<Vec<usize>>,
    ) {
        if out.len() >= max_paths || path.len() > 64 {
            return;
        }
        let Some(visited) = on_path.get_mut(index) else {
            return;
        };
        if *visited {
            return;
        }
        *visited = true;
        path.push(index);

        let targets = self.depends.get(index);
        if targets.is_none_or(Vec::is_empty) {
            out.push(path.clone());
        } else if let Some(targets) = targets {
            for target in targets {
                self.collect_paths(*target, path, on_path, max_paths, out);
                if out.len() >= max_paths {
                    break;
                }
            }
        }

        path.pop();
        on_path[index] = false;
    }

    /// Find cycles as strongly connected components.  Self loops are included.
    pub fn find_cycles(&self) -> Vec<Vec<usize>> {
        let mut state = CycleState {
            next_index: 0,
            indices: vec![usize::MAX; self.len()],
            lowlink: vec![0; self.len()],
            on_stack: vec![false; self.len()],
            stack: Vec::new(),
            components: Vec::new(),
        };

        for index in 0..self.len() {
            if state.indices.get(index).copied().unwrap_or(0) == usize::MAX {
                strongconnect(index, &self.depends, &mut state);
            }
        }

        let mut cycles: Vec<Vec<usize>> = state
            .components
            .into_iter()
            .filter(|component| {
                component.len() > 1
                    || component.first().is_some_and(|index| {
                        self.depends
                            .get(*index)
                            .is_some_and(|targets| targets.contains(index))
                    })
            })
            .map(|mut component| {
                component.sort_unstable();
                component
            })
            .collect();
        cycles.sort();
        cycles
    }

    /// Render cycle components as package names.
    pub fn cycle_names(&self) -> Vec<Vec<String>> {
        self.cycles
            .iter()
            .map(|cycle| {
                cycle
                    .iter()
                    .filter_map(|index| self.package_name(*index).map(ToString::to_string))
                    .collect()
            })
            .collect()
    }

    /// Compute the exact impact of removing `targets`.
    ///
    /// * Surviving consumers are installed packages that are not removed but
    ///   still hard-depend on something that is.
    /// * The exclusive dependency closure contains non-explicit dependencies
    ///   that are only required from within the removal set.
    /// * Freed bytes are the sum of `%SIZE%` for every removed package.
    pub fn removal_impact(&self, targets: &[usize]) -> RemovalImpact {
        let mut removed: BTreeSet<usize> = targets
            .iter()
            .copied()
            .filter(|index| *index < self.len())
            .collect();
        let requested: Vec<usize> = removed.iter().copied().collect();

        loop {
            let mut candidates: BTreeSet<usize> = BTreeSet::new();
            for index in &removed {
                if let Some(targets) = self.depends.get(*index) {
                    for target in targets {
                        if !removed.contains(target) {
                            candidates.insert(*target);
                        }
                    }
                }
            }

            let mut changed = false;
            for candidate in candidates {
                let explicit = self
                    .packages
                    .get(candidate)
                    .map(|package| package.explicit)
                    .unwrap_or(false);
                if explicit {
                    continue;
                }
                let needed_outside = self
                    .dependents
                    .get(candidate)
                    .is_some_and(|dependents| dependents.iter().any(|dep| !removed.contains(dep)));
                if !needed_outside {
                    removed.insert(candidate);
                    changed = true;
                }
            }

            if !changed {
                break;
            }
        }

        let mut surviving_consumers = Vec::new();
        let mut optional_consumers = Vec::new();
        for index in 0..self.len() {
            if removed.contains(&index) {
                continue;
            }
            let requirements: Vec<String> = self
                .dependency_edges
                .get(index)
                .map(|edges| {
                    edges
                        .iter()
                        .filter(|edge| removed.contains(&edge.target))
                        .map(|edge| edge.requirement.clone())
                        .collect()
                })
                .unwrap_or_default();
            if !requirements.is_empty() {
                surviving_consumers.push(RemovalConsumer {
                    package: index,
                    name: self.package_name(index).unwrap_or("").to_string(),
                    requirements,
                    optional: false,
                });
            }

            let optional_requirements: Vec<String> = self
                .opt_dependency_edges
                .get(index)
                .map(|edges| {
                    edges
                        .iter()
                        .filter(|edge| removed.contains(&edge.target))
                        .map(|edge| edge.requirement.clone())
                        .collect()
                })
                .unwrap_or_default();
            if !optional_requirements.is_empty() {
                optional_consumers.push(RemovalConsumer {
                    package: index,
                    name: self.package_name(index).unwrap_or("").to_string(),
                    requirements: optional_requirements,
                    optional: true,
                });
            }
        }

        let exclusive_closure: Vec<usize> = removed
            .iter()
            .copied()
            .filter(|index| !requested.contains(index))
            .collect();

        let freed_bytes = removed.iter().fold(0u64, |total, index| {
            total.saturating_add(
                self.packages
                    .get(*index)
                    .map(|package| package.size)
                    .unwrap_or(0),
            )
        });

        RemovalImpact {
            requested,
            removed: removed.into_iter().collect(),
            exclusive_closure,
            surviving_consumers,
            optional_consumers,
            freed_bytes,
        }
    }
}

fn add_provider(index: &mut HashMap<String, Vec<ProviderEntry>>, spec: &str, entry: ProviderEntry) {
    let key = normalize_dependency_name(spec);
    if key.is_empty() {
        return;
    }
    let entries = index.entry(key).or_default();
    if !entries
        .iter()
        .any(|existing| existing.package == entry.package && existing.version == entry.version)
    {
        entries.push(entry);
    }
}

fn resolve_with_packages(
    index: &HashMap<String, Vec<ProviderEntry>>,
    requirement: &str,
    packages: &[Package],
) -> Option<usize> {
    let key = normalize_dependency_name(requirement);
    let entries = index.get(&key)?;
    entries
        .iter()
        .filter(|entry| match &entry.version {
            Some(version) => version_satisfies(version, requirement),
            None => true,
        })
        .min_by_key(|entry| {
            let exact_name = packages
                .get(entry.package)
                .map(|package| normalize_dependency_name(&package.name) == key)
                .unwrap_or(false);
            (!exact_name, entry.package)
        })
        .map(|entry| entry.package)
}

struct CycleState {
    next_index: usize,
    indices: Vec<usize>,
    lowlink: Vec<usize>,
    on_stack: Vec<bool>,
    stack: Vec<usize>,
    components: Vec<Vec<usize>>,
}

fn strongconnect(index: usize, adjacency: &[Vec<usize>], state: &mut CycleState) {
    if let Some(slot) = state.indices.get_mut(index) {
        *slot = state.next_index;
    }
    if let Some(slot) = state.lowlink.get_mut(index) {
        *slot = state.next_index;
    }
    state.next_index += 1;
    state.stack.push(index);
    if let Some(slot) = state.on_stack.get_mut(index) {
        *slot = true;
    }

    if let Some(targets) = adjacency.get(index) {
        for target in targets {
            if state.indices.get(*target).copied().unwrap_or(0) == usize::MAX {
                strongconnect(*target, adjacency, state);
                let target_low = state.lowlink.get(*target).copied().unwrap_or(usize::MAX);
                if let Some(slot) = state.lowlink.get_mut(index) {
                    *slot = (*slot).min(target_low);
                }
            } else if state.on_stack.get(*target).copied().unwrap_or(false) {
                let target_index = state.indices.get(*target).copied().unwrap_or(usize::MAX);
                if let Some(slot) = state.lowlink.get_mut(index) {
                    *slot = (*slot).min(target_index);
                }
            }
        }
    }

    let is_root = state.indices.get(index).copied().unwrap_or(usize::MAX)
        == state.lowlink.get(index).copied().unwrap_or(usize::MAX);
    if is_root {
        let mut component = Vec::new();
        while let Some(popped) = state.stack.pop() {
            if let Some(slot) = state.on_stack.get_mut(popped) {
                *slot = false;
            }
            component.push(popped);
            if popped == index {
                break;
            }
        }
        if !component.is_empty() {
            state.components.push(component);
        }
    }
}

/// A package that survives a removal but depends on something being removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalConsumer {
    pub package: usize,
    pub name: String,
    pub requirements: Vec<String>,
    pub optional: bool,
}

/// Exact result of removing a set of packages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalImpact {
    pub requested: Vec<usize>,
    /// Requested packages plus the exclusive dependency closure.
    pub removed: Vec<usize>,
    /// Dependencies that are removed only because of the request.
    pub exclusive_closure: Vec<usize>,
    pub surviving_consumers: Vec<RemovalConsumer>,
    /// Consumers whose link is optional; they do not block the removal.
    pub optional_consumers: Vec<RemovalConsumer>,
    pub freed_bytes: u64,
}

impl RemovalImpact {
    pub fn removed_names(&self, graph: &Graph) -> Vec<String> {
        self.removed
            .iter()
            .filter_map(|index| graph.package_name(*index).map(ToString::to_string))
            .collect()
    }

    pub fn closure_names(&self, graph: &Graph) -> Vec<String> {
        self.exclusive_closure
            .iter()
            .filter_map(|index| graph.package_name(*index).map(ToString::to_string))
            .collect()
    }

    pub fn consumer_names(&self) -> Vec<&str> {
        self.surviving_consumers
            .iter()
            .map(|consumer| consumer.name.as_str())
            .collect()
    }

    pub fn is_safe(&self) -> bool {
        self.surviving_consumers.is_empty()
    }
}

/// Convenience: build a name -> index map for UI/report layers.
pub fn name_index(graph: &Graph) -> BTreeMap<&str, usize> {
    graph
        .packages
        .iter()
        .enumerate()
        .map(|(index, package)| (package.name.as_str(), index))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Package;
    use std::collections::BTreeSet;

    fn package(name: &str, version: &str, explicit: bool, size: u64) -> Package {
        Package {
            name: name.to_string(),
            version: version.to_string(),
            explicit,
            size,
            reason: Some(if explicit { "1" } else { "0" }.to_string()),
            ..Package::default()
        }
    }

    fn fixture_graph() -> Graph {
        let mut app = package("app", "1.0-1", true, 100);
        app.depends = vec!["libfoo.so=1-64".into(), "bar>=2.0".into(), "tool".into()];
        app.optdepends = vec!["extra: optional features".into()];

        let mut libfoo = package("libfoo", "1.2-1", false, 300);
        libfoo.provides = vec!["libfoo.so=1-64".into()];

        let mut bar = package("bar", "2.1-1", false, 200);
        bar.provides = vec!["bar-api=2".into()];

        let mut tool = package("tool", "1.0-1", true, 50);
        tool.depends = vec!["bar>=2.0".into()];

        let mut frontend = package("frontend", "1.0-1", true, 75);
        frontend.depends = vec!["app".into()];

        let extra = package("extra", "1.0-1", false, 10);

        let mut viewer = package("viewer", "1.0-1", true, 20);
        viewer.optdepends = vec!["app: optional integration".into()];

        Graph::new(vec![app, libfoo, bar, tool, frontend, extra, viewer])
    }

    #[test]
    fn resolves_names_provides_and_soname_versions() {
        let graph = fixture_graph();
        let app = graph.index_of("app").expect("app");
        let libfoo = graph.index_of("libfoo").expect("libfoo");
        let bar = graph.index_of("bar").expect("bar");
        let extra = graph.index_of("extra").expect("extra");

        assert_eq!(graph.resolve("libfoo.so=1-64"), Some(libfoo));
        assert_eq!(graph.resolve("bar-api=2"), Some(bar));
        assert_eq!(graph.resolve("bar>=2.0"), Some(bar));
        assert_eq!(graph.resolve("extra"), Some(extra));
        assert!(graph.resolve("does-not-exist").is_none());

        assert!(graph.depends[app].contains(&libfoo));
        assert!(graph.depends[app].contains(&bar));
        assert!(graph.opt_depends[app].contains(&extra));
        assert!(!graph.depends[app].contains(&extra));
        assert!(graph.opt_dependents[extra].contains(&app));
    }

    #[test]
    fn detects_cycles() {
        let mut a = package("a", "1", false, 1);
        let mut b = package("b", "1", false, 1);
        let mut c = package("c", "1", false, 1);
        a.depends = vec!["b".into()];
        b.depends = vec!["a".into()];
        c.depends = vec!["c".into()];

        let graph = Graph::new(vec![a, b, c]);
        assert_eq!(graph.cycles.len(), 2);
        assert_eq!(graph.cycle_names(), vec![vec!["a", "b"], vec!["c"]]);
    }

    #[test]
    fn computes_exact_removal_impact() {
        let graph = fixture_graph();
        let app = graph.index_of("app").expect("app");
        let libfoo = graph.index_of("libfoo").expect("libfoo");
        let bar = graph.index_of("bar").expect("bar");
        let tool = graph.index_of("tool").expect("tool");

        let impact = graph.removal_impact(&[app]);

        assert_eq!(impact.requested, vec![app]);
        assert!(impact.removed.contains(&app));
        assert!(impact.removed.contains(&libfoo));
        assert_eq!(impact.exclusive_closure, vec![libfoo]);
        assert!(!impact.removed.contains(&bar));
        assert!(!impact.removed.contains(&tool));
        assert_eq!(impact.freed_bytes, 400);
        assert_eq!(
            impact.consumer_names(),
            vec!["frontend"],
            "frontend survives and depends on app"
        );
        assert_eq!(impact.optional_consumers.len(), 1);
        assert_eq!(impact.optional_consumers[0].name, "viewer");
    }

    #[test]
    fn removal_closes_shared_dependencies_when_all_consumers_go() {
        let graph = fixture_graph();
        let app = graph.index_of("app").expect("app");
        let tool = graph.index_of("tool").expect("tool");
        let bar = graph.index_of("bar").expect("bar");

        let impact = graph.removal_impact(&[app, tool]);
        assert!(
            impact.removed.contains(&bar),
            "bar has no surviving consumer"
        );
        assert!(impact.removed.contains(&graph.index_of("libfoo").unwrap()));
        assert_eq!(impact.consumer_names(), vec!["frontend"]);
    }

    #[test]
    fn removal_keeps_explicit_dependencies() {
        let mut app = package("app", "1", true, 10);
        app.depends = vec!["explicit-lib".into()];
        let explicit_lib = package("explicit-lib", "1", true, 90);
        let graph = Graph::new(vec![app, explicit_lib]);
        let app = graph.index_of("app").expect("app");
        let explicit_lib = graph.index_of("explicit-lib").expect("explicit-lib");

        let impact = graph.removal_impact(&[app]);
        assert!(!impact.removed.contains(&explicit_lib));
        assert_eq!(impact.freed_bytes, 10);
    }

    #[test]
    fn roots_and_paths_are_reachable() {
        let graph = fixture_graph();
        let app = graph.index_of("app").expect("app");
        let frontend = graph.index_of("frontend").expect("frontend");
        let libfoo = graph.index_of("libfoo").expect("libfoo");

        let reachable = graph.reachable_from_explicit();
        assert!(reachable.contains(&app));
        assert!(reachable.contains(&libfoo));

        let shortest = graph.shortest_paths_from_explicit();
        assert_eq!(
            shortest.get(frontend).and_then(|path| path.as_ref()),
            Some(&vec![frontend])
        );
        assert_eq!(
            shortest.get(app).and_then(|path| path.as_ref()),
            Some(&vec![app]),
            "app is itself explicit, so the shortest root path is direct"
        );

        let paths = graph.paths_from(frontend, 10);
        assert!(paths.iter().any(|path| path.first() == Some(&frontend)));
        assert!(paths.iter().any(|path| path.contains(&app)));
    }

    #[test]
    fn unresolved_dependencies_are_reported() {
        let mut broken = package("broken", "1", true, 1);
        broken.depends = vec!["missing>=1".into()];
        broken.optdepends = vec!["missing-opt: why".into()];
        let graph = Graph::new(vec![broken]);

        assert_eq!(graph.unresolved_hard().len(), 1);
        assert_eq!(graph.unresolved_hard()[0].requirement, "missing>=1");
        let names: BTreeSet<&str> = graph
            .unresolved
            .iter()
            .map(|dependency| dependency.requirement.as_str())
            .collect();
        assert!(names.contains("missing-opt: why"));
    }
}
