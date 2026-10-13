use std::collections::{BTreeMap, BTreeSet};

pub const GRAPH_LIMIT: usize = 32;
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Platform {
    Windows,
    Macos,
}
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Provider {
    Service,
    Direct,
    Spotlight,
}
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Mode {
    Broad,
    SmartLean,
    Custom,
}
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Source {
    pub source_id: String,
    pub volume_id: String,
    pub provider_id: String,
    pub platform: Platform,
    pub provider: Provider,
    pub source_epoch: u64,
    pub volume_epoch: u64,
    pub provider_epoch: u64,
    pub mount_epoch: u64,
    pub permission_epoch: u64,
}
/// Birth generation distinguishes reused FRN/inode from the former object.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Identity {
    pub number: u64,
    pub birth: u64,
}
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Node {
    pub id: Identity,
    pub parent: Option<Identity>,
}
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SourceHealth {
    Accounted,
    Unavailable,
    Busy,
    PermissionRequired,
    Partial,
    CheckpointGap,
}
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Evidence {
    LawfulScope,
    ExplicitRule(String),
    ProtectedIdentity(Identity),
    Dependency {
        project: Identity,
        manifest: String,
    },
    Generated {
        project: Identity,
        signature: String,
    },
    Cache {
        owner: Identity,
        signature: String,
    },
}
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Reason {
    Protected,
    Permission,
    ExplicitExclude,
    Performance,
    SourceIdentity,
    SourceUnavailable,
    SourceBusy,
    SourcePartial,
    CheckpointGap,
    MissingParent,
    DuplicateParent,
    Cycle,
    GraphBound,
    Path,
    Scope,
    Metadata,
    Classification,
    AliasTarget,
    StaleGeneration,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Decision {
    Include {
        evidence: Evidence,
    },
    MinimalAncestor {
        need: String,
        descendant_identity: Identity,
    },
    Exclude {
        reason: Reason,
        evidence: Evidence,
    },
    Undetermined {
        reason: Reason,
        missing_evidence: String,
    },
}
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Rule {
    pub path: String,
    pub include: bool,
    pub label: String,
}
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Root {
    pub path: String,
    pub identity: Identity,
}
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Snapshot {
    pub source: Source,
    pub revision: u64,
    pub known_folder_version: u64,
    pub mode: Mode,
    pub roots: Vec<Root>,
    pub rules: Vec<Rule>,
    /// Trusted platform/source registry, not basename or user-supplied flags.
    pub protected: Vec<Identity>,
}
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AliasKind {
    Symlink,
    Junction,
    Reparse,
}
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AliasEdge {
    pub kind: AliasKind,
    pub target: Option<Identity>,
    pub source: Source,
    pub permission: bool,
}
/// Synthetic receipt of an included descendant, tied to its immutable snapshot.
/// Receipt acquisition is a source fact; there is no native adapter here.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AnchorNeed {
    pub need: String,
    pub descendant_identity: Identity,
    pub included_snapshot: Box<Snapshot>,
}
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Context {
    pub source: Source,
    pub policy_revision: u64,
    pub health: SourceHealth,
    pub identity: Identity,
    pub path: String,
    /// Optional source-verified canonical Unicode spelling; never inferred.
    pub unicode_path: Option<String>,
    pub parents: Vec<Node>,
    /// Each symlink/junction/reparse target must be resolved to this source.
    /// None is unresolved. Hardlink occurrences have no traversal edge.
    pub aliases: Vec<AliasEdge>,
    pub permission: bool,
    pub metadata_complete: bool,
    pub classification_complete: bool,
    pub performance_evidence: Option<Evidence>,
    /// Source-verified subtree to which the contextual evidence applies.
    pub performance_path: Option<String>,
    /// Source-verified anchor need; anchors have no searchable payload.
    pub anchor: Option<AnchorNeed>,
    pub rename_epoch: u64,
}
fn unknown(reason: Reason, missing: &str) -> Decision {
    Decision::Undetermined {
        reason,
        missing_evidence: missing.into(),
    }
}
/// Lexical component matching only. Unicode equivalence requires source facts.
/// Extended DOS/UNC prefixes are transport spellings, never new roots.
fn components(path: &str, platform: Platform) -> Option<Vec<String>> {
    let mut text = match platform {
        Platform::Windows => path.replace('\\', "/").to_ascii_lowercase(),
        Platform::Macos => path.to_owned(),
    };
    if platform == Platform::Windows {
        if let Some(tail) = text.strip_prefix("//?/unc/") {
            text = format!("//{tail}");
        } else if let Some(tail) = text.strip_prefix("//?/") {
            text = tail.into();
        }
    }
    let prefix = if text.starts_with("//") {
        "unc"
    } else if text.starts_with('/') {
        "posix"
    } else if platform == Platform::Windows
        && text.as_bytes().get(1) == Some(&b':')
        && text.as_bytes().get(2) == Some(&b'/')
    {
        "dos"
    } else {
        return None;
    };
    let mut parts = vec![prefix.into()];
    for part in text.split('/').filter(|p| !p.is_empty()) {
        if part == "." || part == ".." || part.contains('\0') {
            return None;
        }
        parts.push(part.into());
    }
    if parts.len() < 2 || (prefix == "unc" && parts.len() < 3) {
        return None;
    }
    Some(parts)
}
fn below(path: &[String], root: &[String]) -> bool {
    path.starts_with(root)
}
fn chain(start: Identity, nodes: &[Node]) -> Result<Vec<Identity>, Reason> {
    let mut result = Vec::new();
    let mut next = Some(start);
    while let Some(id) = next {
        if result.contains(&id) {
            return Err(Reason::Cycle);
        }
        if result.len() == GRAPH_LIMIT {
            return Err(Reason::GraphBound);
        }
        let matches: Vec<_> = nodes.iter().filter(|n| n.id == id).collect();
        if matches.is_empty() {
            return Err(Reason::MissingParent);
        }
        if matches.len() != 1 {
            return Err(Reason::DuplicateParent);
        }
        result.push(id);
        next = matches[0].parent;
    }
    Ok(result)
}
/// Pure, deliberately test-only reference implementation of the frozen contract.
pub fn policy_evaluate(context: &Context, immutable_snapshot: &Snapshot) -> Decision {
    let s = immutable_snapshot;
    let c = context;
    if c.source != s.source
        || s.source.source_id.is_empty()
        || s.source.volume_id.is_empty()
        || s.source.provider_id.is_empty()
    {
        return unknown(
            Reason::SourceIdentity,
            "current source/volume/provider/mount/permission identity",
        );
    }
    if c.policy_revision != s.revision {
        return unknown(Reason::StaleGeneration, "current policy revision");
    }
    if !c.permission {
        return unknown(Reason::Permission, "verified permission");
    }
    if s.known_folder_version == 0 {
        return unknown(Reason::SourceIdentity, "known-folder rule version");
    }
    let ancestry = match chain(c.identity, &c.parents) {
        Ok(a) => a,
        Err(r) => return unknown(r, "complete unique bounded parent identity graph"),
    };
    let mut physical = ancestry.clone();
    if c.aliases.len() > GRAPH_LIMIT {
        return unknown(Reason::GraphBound, "bounded alias traversal");
    }
    let mut visited = BTreeSet::from([c.identity]);
    for edge in &c.aliases {
        if edge.source != s.source {
            return unknown(
                Reason::SourceIdentity,
                "current source/volume/provider/mount/permission identity",
            );
        }
        if !edge.permission {
            return unknown(Reason::Permission, "verified permission");
        }
        let Some(target) = &edge.target else {
            return unknown(Reason::AliasTarget, "verified alias target identity");
        };
        if !visited.insert(*target) {
            return unknown(Reason::Cycle, "acyclic alias traversal");
        }
        match chain(*target, &c.parents) {
            Ok(a) => physical.extend(a),
            Err(r) => return unknown(r, "complete unique bounded parent identity graph"),
        }
    }
    // Actual OS boundary dominates every explicit include, including aliases.
    if let Some(id) = s.protected.iter().filter(|id| physical.contains(id)).min() {
        return Decision::Exclude {
            reason: Reason::Protected,
            evidence: Evidence::ProtectedIdentity(*id),
        };
    }
    let source_problem = match c.health {
        SourceHealth::Accounted => None,
        SourceHealth::Unavailable => Some(Reason::SourceUnavailable),
        SourceHealth::Busy => Some(Reason::SourceBusy),
        SourceHealth::PermissionRequired => Some(Reason::Permission),
        SourceHealth::Partial => Some(Reason::SourcePartial),
        SourceHealth::CheckpointGap => Some(Reason::CheckpointGap),
    };
    if let Some(reason) = source_problem {
        return unknown(reason, "coverage-critical source accounting");
    }
    let Some(path) = components(
        c.unicode_path.as_deref().unwrap_or(&c.path),
        s.source.platform,
    ) else {
        return unknown(Reason::Path, "credible absolute component path");
    };
    let mut lawful = false;
    for root in &s.roots {
        let Some(root_path) = components(&root.path, s.source.platform) else {
            return unknown(Reason::Scope, "verified scope root path and identity");
        };
        if below(&path, &root_path) && ancestry.contains(&root.identity) {
            lawful = true;
        }
    }
    if !lawful {
        return unknown(
            Reason::Scope,
            "verified lawful scope root path and identity",
        );
    }
    if !c.metadata_complete {
        return unknown(Reason::Metadata, "complete metadata");
    }
    // A source-verified graph-only occurrence never becomes a search payload,
    // even when a broader user Include also matches its display path.
    if let Some(anchor) = &c.anchor {
        if anchor.included_snapshot.source != s.source {
            return unknown(
                Reason::SourceIdentity,
                "current source/volume/provider/mount/permission identity",
            );
        }
        if fingerprint(&anchor.included_snapshot) != fingerprint(s) {
            return unknown(Reason::StaleGeneration, "current policy revision");
        }
        let need = &anchor.need;
        let descendant = &anchor.descendant_identity;
        match chain(*descendant, &c.parents) {
            Ok(descendants)
                if *descendant != c.identity
                    && descendants.contains(&c.identity)
                    && !need.is_empty()
                    && !s.protected.iter().any(|id| descendants.contains(id)) =>
            {
                return Decision::MinimalAncestor {
                    need: need.clone(),
                    descendant_identity: *descendant,
                }
            }
            _ => {
                return unknown(
                    Reason::MissingParent,
                    "verified included descendant identity",
                )
            }
        }
    }
    let mut matches = Vec::new();
    for rule in &s.rules {
        let Some(rule_path) = components(&rule.path, s.source.platform) else {
            return unknown(Reason::Path, "credible explicit rule path");
        };
        if below(&path, &rule_path) {
            matches.push((rule_path.len(), rule));
        }
    }
    // Validate a performance classification against its own subtree before
    // comparing its specificity with explicit user choices.
    let performance = if s.mode == Mode::Broad {
        None
    } else if let Some(evidence) = &c.performance_evidence {
        let valid_owner = match evidence {
            Evidence::Dependency { project, manifest } => {
                ancestry.contains(project) && !manifest.is_empty()
            }
            Evidence::Generated { project, signature } => {
                ancestry.contains(project) && !signature.is_empty()
            }
            Evidence::Cache { owner, signature } => {
                ancestry.contains(owner) && !signature.is_empty()
            }
            _ => false,
        };
        let scope = c
            .performance_path
            .as_deref()
            .and_then(|p| components(p, s.source.platform));
        match scope {
            Some(scope)
                if valid_owner
                    && below(&path, &scope)
                    && s.roots.iter().any(|r| {
                        components(&r.path, s.source.platform)
                            .is_some_and(|root| below(&scope, &root))
                    }) =>
            {
                Some((scope.len(), evidence))
            }
            _ => return unknown(Reason::Classification, "contextual classification evidence"),
        }
    } else {
        None
    };
    // Specificity, then explicit exclude, then stable evidence label.
    matches.sort_by(|(da, a), (db, b)| {
        db.cmp(da)
            .then(a.include.cmp(&b.include))
            .then(a.label.cmp(&b.label))
    });
    if let Some((depth, rule)) = matches.first() {
        if !rule.include
            || performance.is_none_or(|(performance_depth, _)| *depth >= performance_depth)
        {
            return if rule.include {
                Decision::Include {
                    evidence: Evidence::ExplicitRule(rule.label.clone()),
                }
            } else {
                Decision::Exclude {
                    reason: Reason::ExplicitExclude,
                    evidence: Evidence::ExplicitRule(rule.label.clone()),
                }
            };
        }
    }
    if !c.classification_complete {
        return unknown(Reason::Classification, "contextual classification evidence");
    }
    if let Some((_, evidence)) = performance {
        return Decision::Exclude {
            reason: Reason::Performance,
            evidence: evidence.clone(),
        };
    }
    Decision::Include {
        evidence: Evidence::LawfulScope,
    }
}
/// Collision-free canonical fingerprint as structured data (no lossy hash).
/// Includes every immutable authority; collection order is non-semantic.
pub fn fingerprint(snapshot: &Snapshot) -> Snapshot {
    let mut canonical = snapshot.clone();
    canonical.roots.sort();
    canonical.roots.dedup();
    canonical.rules.sort();
    canonical.rules.dedup();
    canonical.protected.sort();
    canonical.protected.dedup();
    canonical
}
#[derive(Default)]
pub struct EvaluationCache {
    entries: BTreeMap<(Snapshot, Context), Decision>,
    pub evaluations: usize,
    pub hits: usize,
}
impl EvaluationCache {
    pub fn evaluate(&mut self, context: &Context, snapshot: &Snapshot) -> Decision {
        let key = (fingerprint(snapshot), context.clone());
        if let Some(result) = self.entries.get(&key) {
            self.hits += 1;
            return result.clone();
        }
        self.evaluations += 1;
        let result = policy_evaluate(context, snapshot);
        self.entries.insert(key, result.clone());
        result
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Hint {
    None,
    Reconcile(Identity),
    Backfill(Identity),
    RejectGeneration,
}
pub fn transition_hint(old: &Decision, new: &Decision, id: Identity) -> Hint {
    match (old, new) {
        (
            _,
            Decision::Undetermined {
                reason: Reason::StaleGeneration | Reason::SourceIdentity,
                ..
            },
        ) => Hint::RejectGeneration,
        (Decision::Exclude { .. }, Decision::Include { .. }) => Hint::Backfill(id),
        (Decision::Include { .. }, Decision::Exclude { .. }) => Hint::Reconcile(id),
        (_, Decision::Undetermined { .. }) => Hint::Reconcile(id),
        _ => Hint::None,
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoverageProjection {
    Accounted {
        included: usize,
        anchors: usize,
        excluded: usize,
    },
    Partial,
    Obsolete,
}
#[derive(Clone, Debug)]
pub struct StageAccounting {
    pub evaluated_fingerprint: Snapshot,
    pub enumerated: usize,
    pub evaluated: usize,
    pub unresolved: usize,
    pub native_stages_verified: bool,
    pub source: Source,
    pub policy_revision: u64,
}
/// Returns coverage only; never manufactures live SourceHealth or READY.
pub fn coverage(
    snapshot: &Snapshot,
    health: SourceHealth,
    stage: &StageAccounting,
    results: &[Decision],
) -> CoverageProjection {
    if stage.source != snapshot.source
        || stage.policy_revision != snapshot.revision
        || fingerprint(&stage.evaluated_fingerprint) != fingerprint(snapshot)
    {
        return CoverageProjection::Obsolete;
    }
    if health != SourceHealth::Accounted
        || !stage.native_stages_verified
        || stage.unresolved != 0
        || stage.enumerated != stage.evaluated
        || stage.evaluated != results.len()
        || results
            .iter()
            .any(|r| matches!(r, Decision::Undetermined { .. }))
    {
        return CoverageProjection::Partial;
    }
    CoverageProjection::Accounted {
        included: results
            .iter()
            .filter(|r| matches!(r, Decision::Include { .. }))
            .count(),
        anchors: results
            .iter()
            .filter(|r| matches!(r, Decision::MinimalAncestor { .. }))
            .count(),
        excluded: results
            .iter()
            .filter(|r| matches!(r, Decision::Exclude { .. }))
            .count(),
    }
}
