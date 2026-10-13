use super::{model::*, oracle};
use oracle::{CHILD, FILE, OS, ROOT};
use std::collections::BTreeMap;
use std::time::Instant;

fn fixture(witness: usize) -> (Context, Snapshot) {
    let source = Source {
        source_id: "synthetic-source".into(),
        volume_id: "synthetic-volume".into(),
        provider_id: "synthetic-provider".into(),
        platform: Platform::Windows,
        provider: Provider::Service,
        source_epoch: 1,
        volume_epoch: 1,
        provider_epoch: 1,
        mount_epoch: 1,
        permission_epoch: 1,
    };
    let mut s = Snapshot {
        source: source.clone(),
        revision: 1,
        known_folder_version: 1,
        mode: Mode::SmartLean,
        roots: vec![Root {
            path: "C:/Users/owner/Projects".into(),
            identity: ROOT,
        }],
        rules: vec![],
        protected: vec![OS],
    };
    let mut c = Context {
        source,
        policy_revision: 1,
        health: SourceHealth::Accounted,
        identity: FILE,
        path: "C:/Users/owner/Projects/app/src/main.rs".into(),
        unicode_path: None,
        parents: vec![
            Node {
                id: ROOT,
                parent: None,
            },
            Node {
                id: FILE,
                parent: Some(ROOT),
            },
        ],
        aliases: vec![],
        permission: true,
        metadata_complete: true,
        classification_complete: true,
        performance_evidence: None,
        performance_path: Some("C:/Users/owner/Projects/app".into()),
        anchor: None,
        rename_epoch: 1,
    };
    match witness {
        0 => (),
        1 => {
            c.performance_evidence = Some(Evidence::Dependency {
                project: ROOT,
                manifest: "package-lock.json".into(),
            })
        }
        2 => {
            c.performance_evidence = Some(Evidence::Generated {
                project: ROOT,
                signature: "compiler-output".into(),
            })
        }
        3 => {
            c.performance_evidence = Some(Evidence::Cache {
                owner: ROOT,
                signature: "ephemeral-cache".into(),
            })
        }
        4 => {
            s.rules = vec![
                rule("C:/Users/owner/Projects/app", false, "outer-exclude"),
                rule("C:/Users/owner/Projects/app/src", true, "nested-include"),
            ];
            c.performance_evidence = Some(Evidence::Generated {
                project: ROOT,
                signature: "compiler-output".into(),
            });
        }
        5 => {
            s.rules = vec![
                rule("C:/Users/owner/Projects/app", true, "equal-include"),
                rule("C:/Users/owner/Projects/app", false, "equal-exclude"),
            ]
        }
        6 => {
            c.parents.push(Node {
                id: OS,
                parent: None,
            });
            c.aliases = vec![alias(&c, AliasKind::Symlink, Some(OS))];
            s.rules = vec![rule("C:/Users/owner/Projects", true, "user-include")];
        }
        7 => {
            c.parents.push(Node {
                id: CHILD,
                parent: Some(FILE),
            });
            c.anchor = Some(AnchorNeed {
                need: "included-child".into(),
                descendant_identity: CHILD,
                included_snapshot: Box::new(s.clone()),
            });
        }
        8 => c.classification_complete = false,
        9 => c.parents.retain(|n| n.id != ROOT),
        10 => c.aliases = vec![alias(&c, AliasKind::Reparse, Some(FILE))],
        11 => c.aliases = vec![alias(&c, AliasKind::Junction, None)],
        12 => c.health = SourceHealth::Busy,
        13 => c.permission = false,
        14 => c.policy_revision = 0,
        15 => c.source.provider_epoch = 0,
        16 => c.parents.push(c.parents[0].clone()),
        17 => c.metadata_complete = false,
        _ => panic!("unknown fixture witness"),
    }
    (c, s)
}
fn alias(c: &Context, kind: AliasKind, target: Option<Identity>) -> AliasEdge {
    AliasEdge {
        kind,
        target,
        source: c.source.clone(),
        permission: true,
    }
}
fn rule(path: &str, include: bool, label: &str) -> Rule {
    Rule {
        path: path.into(),
        include,
        label: label.into(),
    }
}
fn check(witness: usize) -> (Context, Snapshot) {
    let (c, s) = fixture(witness);
    assert_eq!(
        policy_evaluate(&c, &s),
        oracle::expected(witness),
        "witness {witness}"
    );
    (c, s)
}
fn assert_unknown(c: &Context, s: &Snapshot, reason: Reason, missing: &str) {
    assert_eq!(
        policy_evaluate(c, s),
        Decision::Undetermined {
            reason,
            missing_evidence: missing.into()
        }
    );
}

#[test]
fn t01_modes_are_synthetic_and_broad_compatibility_is_inert() {
    let (c, mut s) = fixture(1);
    for mode in [Mode::SmartLean, Mode::Custom] {
        s.mode = mode;
        assert_eq!(policy_evaluate(&c, &s), oracle::expected(1));
    }
    s.mode = Mode::Broad;
    assert_eq!(policy_evaluate(&c, &s), oracle::expected(0));
    let (protected, mut broad) = fixture(6);
    broad.mode = Mode::Broad;
    assert_eq!(policy_evaluate(&protected, &broad), oracle::expected(6));
    // No function changes any mode or default: immutable before/after witness.
    let before = (c.clone(), s.clone());
    policy_evaluate(&c, &s);
    assert_eq!((c, s), before);
}

#[test]
fn t02_same_named_user_directories_are_not_system_boundaries() {
    let (mut c, s) = fixture(0);
    for relative in [
        "Projects/Windows/report.txt",
        "Design/build/draft.psd",
        "Configs/AppData/config.toml",
        "Program Files2/notes.txt",
        "Windows/System32/user.c",
    ] {
        c.path = format!("C:/Users/owner/Projects/{relative}");
        assert_eq!(policy_evaluate(&c, &s), oracle::expected(0), "{relative}");
    }
}

#[test]
fn t03_value_paths_and_unknown_conservatism() {
    let (mut c, s) = fixture(0);
    for name in [
        "docs/guide.md",
        "src/lib.rs",
        "README.md",
        "Cargo.toml",
        "package.json",
        ".vscode/settings.json",
    ] {
        c.path = format!("C:/Users/owner/Projects/app/{name}");
        assert_eq!(policy_evaluate(&c, &s), oracle::expected(0));
        c.classification_complete = false;
        assert_eq!(policy_evaluate(&c, &s), oracle::expected(8));
        c.classification_complete = true;
    }
    c.path = "missing-parent/README.md".into();
    assert_unknown(&c, &s, Reason::Path, "credible absolute component path");
    check(17);
}

#[test]
fn t04_exclusions_require_context_not_names() {
    for witness in 1..=3 {
        check(witness);
    }
    let (mut c, s) = fixture(0);
    for name in ["node_modules", "target", "build", ".cache"] {
        c.path = format!("C:/Users/owner/Projects/app/{name}/user.txt");
        assert_eq!(policy_evaluate(&c, &s), oracle::expected(0));
    }
    let (mut misplaced, snap) = fixture(1);
    misplaced.performance_path = Some("C:/Users/owner/Projects/app/node_modules".into());
    assert_eq!(policy_evaluate(&misplaced, &snap), oracle::expected(8));
    misplaced.performance_path = None;
    assert_eq!(policy_evaluate(&misplaced, &snap), oracle::expected(8));
    for invalid in [
        Evidence::Dependency {
            project: OS,
            manifest: "Cargo.toml".into(),
        },
        Evidence::Generated {
            project: ROOT,
            signature: String::new(),
        },
        Evidence::Cache {
            owner: OS,
            signature: "ephemeral-cache".into(),
        },
        Evidence::ProtectedIdentity(OS),
    ] {
        c.performance_evidence = Some(invalid);
        assert_eq!(policy_evaluate(&c, &s), oracle::expected(8));
    }
}

#[test]
fn t05_explicit_specificity_conflicts_and_safety_precedence() {
    check(4);
    check(5);
    check(6);
    let (dependency, mut chosen) = fixture(1);
    chosen
        .rules
        .push(rule("C:/Users/owner/Projects", true, "broad-choice"));
    assert_eq!(policy_evaluate(&dependency, &chosen), oracle::expected(1));
    chosen.rules.push(rule(
        "C:/Users/owner/Projects/app/src",
        true,
        "nested-include",
    ));
    assert_eq!(policy_evaluate(&dependency, &chosen), oracle::expected(4));
    let (c, mut s) = fixture(4);
    s.rules.push(rule(
        "C:/Users/owner/Projects/app/src/main.rs",
        false,
        "equal-exclude",
    ));
    assert_eq!(policy_evaluate(&c, &s), oracle::expected(5));
    s.rules.reverse();
    assert_eq!(policy_evaluate(&c, &s), oracle::expected(5));
    let (mut denied, mut include) = fixture(13);
    include
        .rules
        .push(rule("C:/Users/owner/Projects", true, "user-include"));
    assert_eq!(policy_evaluate(&denied, &include), oracle::expected(13));
    denied.permission = true;
    denied.path = "C:/Other/private.txt".into();
    assert_unknown(
        &denied,
        &include,
        Reason::Scope,
        "verified lawful scope root path and identity",
    );
}

#[test]
fn t06_known_folder_registry_is_source_bound() {
    let (mut c, mut s) = fixture(0);
    c.path = "C:/Users/owner/Projects/Windows/System32/notes.txt".into();
    assert_eq!(policy_evaluate(&c, &s), oracle::expected(0));
    c.parents.push(Node {
        id: OS,
        parent: None,
    });
    c.parents.iter_mut().find(|n| n.id == ROOT).unwrap().parent = Some(OS);
    s.rules
        .push(rule("C:/Users/owner/Projects", true, "user-include"));
    assert_eq!(policy_evaluate(&c, &s), oracle::expected(6));
    c.source.volume_epoch += 1;
    assert_eq!(policy_evaluate(&c, &s), oracle::expected(15));
    c.source = s.source.clone();
    s.known_folder_version = 0;
    assert_unknown(&c, &s, Reason::SourceIdentity, "known-folder rule version");
}

#[test]
fn t07_anchors_and_zero_results_do_not_prove_source_health() {
    let (c, s) = check(7);
    let result = policy_evaluate(&c, &s);
    let mut explicitly_included = s.clone();
    explicitly_included
        .rules
        .push(rule("C:/Users/owner/Projects", true, "user-include"));
    let mut current_anchor = c.clone();
    *current_anchor.anchor.as_mut().unwrap().included_snapshot = explicitly_included.clone();
    assert_eq!(
        policy_evaluate(&current_anchor, &explicitly_included),
        oracle::expected(7)
    );
    assert_eq!(
        policy_evaluate(&c, &explicitly_included),
        oracle::expected(14)
    );
    let mut stage = StageAccounting {
        evaluated_fingerprint: fingerprint(&s),
        enumerated: 1,
        evaluated: 1,
        unresolved: 0,
        native_stages_verified: true,
        source: s.source.clone(),
        policy_revision: s.revision,
    };
    assert_eq!(
        coverage(&s, SourceHealth::Accounted, &stage, &[result]),
        CoverageProjection::Accounted {
            included: 0,
            anchors: 1,
            excluded: 0
        }
    );
    let excluded = oracle::expected(1);
    assert_eq!(
        coverage(
            &s,
            SourceHealth::Accounted,
            &stage,
            std::slice::from_ref(&excluded)
        ),
        CoverageProjection::Accounted {
            included: 0,
            anchors: 0,
            excluded: 1
        }
    );
    for health in [
        SourceHealth::Unavailable,
        SourceHealth::Busy,
        SourceHealth::PermissionRequired,
        SourceHealth::Partial,
        SourceHealth::CheckpointGap,
    ] {
        assert_eq!(
            coverage(&s, health, &stage, std::slice::from_ref(&excluded)),
            CoverageProjection::Partial
        );
    }
    stage.enumerated = 0;
    stage.evaluated = 0;
    stage.native_stages_verified = false;
    assert_eq!(
        coverage(&s, SourceHealth::Accounted, &stage, &[]),
        CoverageProjection::Partial
    );
    stage.native_stages_verified = true;
    assert_eq!(
        coverage(&s, SourceHealth::Accounted, &stage, &[]),
        CoverageProjection::Accounted {
            included: 0,
            anchors: 0,
            excluded: 0
        }
    );
    stage.unresolved = 1;
    assert_eq!(
        coverage(&s, SourceHealth::Accounted, &stage, &[]),
        CoverageProjection::Partial
    );
    let mut invalid_anchor = c;
    invalid_anchor.anchor = Some(AnchorNeed {
        need: "invented-child".into(),
        descendant_identity: OS,
        included_snapshot: Box::new(s.clone()),
    });
    assert_unknown(
        &invalid_anchor,
        &s,
        Reason::MissingParent,
        "verified included descendant identity",
    );
}

#[test]
fn t08_source_health_and_coverage_failures_remain_distinct() {
    check(9);
    check(12);
    check(13);
    let (mut c, s) = fixture(1);
    for (health, reason) in [
        (SourceHealth::Unavailable, Reason::SourceUnavailable),
        (SourceHealth::Busy, Reason::SourceBusy),
        (SourceHealth::Partial, Reason::SourcePartial),
        (SourceHealth::PermissionRequired, Reason::Permission),
        (SourceHealth::CheckpointGap, Reason::CheckpointGap),
    ] {
        c.health = health;
        assert_unknown(&c, &s, reason, "coverage-critical source accounting");
        assert_eq!(c.health, health);
    }
}

#[test]
fn t09_windows_transport_components_and_unicode_identity_fallback() {
    let (mut c, mut s) = fixture(0);
    for path in [
        r"C:\USERS\OWNER\PROJECTS\app\README.md",
        r"\\?\C:\Users\owner\Projects\app\src.rs",
    ] {
        c.path = path.into();
        assert_eq!(policy_evaluate(&c, &s), oracle::expected(0));
    }
    for path in [
        r"\\server\share\Projects\app.txt",
        r"\\?\UNC\SERVER\SHARE\Projects\app.txt",
    ] {
        s.roots[0].path = r"\\server\share\Projects".into();
        c.path = path.into();
        assert_eq!(policy_evaluate(&c, &s), oracle::expected(0));
    }
    c.path = r"\\server\share\Projects2\app.txt".into();
    assert_unknown(
        &c,
        &s,
        Reason::Scope,
        "verified lawful scope root path and identity",
    );
    c.path = r"\\server\share\Projects\..\private.txt".into();
    assert_unknown(&c, &s, Reason::Path, "credible absolute component path");
    c.source.platform = Platform::Macos;
    s.source.platform = Platform::Macos;
    s.roots[0].path = "/Users/owner/café".into();
    c.path = "/Users/owner/cafe\u{301}/文件.txt".into();
    // Missing normalization provenance is not silently guessed.
    assert_unknown(
        &c,
        &s,
        Reason::Scope,
        "verified lawful scope root path and identity",
    );
    c.unicode_path = Some("/Users/owner/café/文件.txt".into());
    assert_eq!(policy_evaluate(&c, &s), oracle::expected(0));
    c.identity.birth = 2; // Canonical spelling alone cannot repair reused inode.
    assert_eq!(policy_evaluate(&c, &s), oracle::expected(9));
}

#[test]
fn t10_aliases_hardlink_visibility_and_bounded_cycles() {
    check(6);
    check(10);
    check(11);
    let (mut c, mut s) = fixture(0);
    let mut cache = EvaluationCache::default();
    s.rules.push(rule(
        "C:/Users/owner/Projects/private",
        false,
        "equal-exclude",
    ));
    c.path = "C:/Users/owner/Projects/private/secret.txt".into();
    assert_eq!(cache.evaluate(&c, &s), oracle::expected(5));
    // Same physical identity, distinct hardlink path, distinct visibility.
    c.path = "C:/Users/owner/Projects/public/secret.txt".into();
    assert_eq!(cache.evaluate(&c, &s), oracle::expected(0));
    assert_eq!(cache.evaluations, 2);
    c.parents.push(Node {
        id: CHILD,
        parent: Some(ROOT),
    });
    for kind in [AliasKind::Symlink, AliasKind::Junction, AliasKind::Reparse] {
        c.aliases = vec![alias(&c, kind, Some(CHILD))];
        assert_eq!(policy_evaluate(&c, &s), oracle::expected(0));
        c.aliases.push(alias(&c, kind, Some(FILE)));
        assert_eq!(policy_evaluate(&c, &s), oracle::expected(10));
    }
    c.aliases = vec![alias(&c, AliasKind::Junction, Some(CHILD))];
    c.aliases[0].permission = false;
    assert_eq!(policy_evaluate(&c, &s), oracle::expected(13));
    c.aliases[0].permission = true;
    c.aliases[0].source.volume_epoch += 1;
    assert_eq!(policy_evaluate(&c, &s), oracle::expected(15));
    c.aliases.clear();
    c.parents[0].parent = Some(FILE);
    assert_unknown(
        &c,
        &s,
        Reason::Cycle,
        "complete unique bounded parent identity graph",
    );
    c.parents[0].parent = None;
    c.aliases = vec![alias(&c, AliasKind::Junction, Some(CHILD)); GRAPH_LIMIT + 1];
    assert_unknown(&c, &s, Reason::GraphBound, "bounded alias traversal");
    let mut deep = fixture(0).0;
    deep.parents.clear();
    for number in 2..=(GRAPH_LIMIT as u64 + 3) {
        let id = Identity { number, birth: 1 };
        deep.parents.push(Node {
            id,
            parent: Some(Identity {
                number: number + 1,
                birth: 1,
            }),
        });
    }
    assert_unknown(
        &deep,
        &s,
        Reason::GraphBound,
        "complete unique bounded parent identity graph",
    );
}

#[test]
fn t11_rename_move_and_policy_transitions_only_emit_hints() {
    let (mut c, mut s) = fixture(0);
    s.rules.push(rule(
        "C:/Users/owner/Projects/private",
        false,
        "equal-exclude",
    ));
    let old = policy_evaluate(&c, &s);
    c.path = "C:/Users/owner/Projects/private/main.rs".into();
    c.rename_epoch += 1;
    let new = policy_evaluate(&c, &s);
    assert_eq!(new, oracle::expected(5));
    assert_eq!(transition_hint(&old, &new, FILE), Hint::Reconcile(FILE));
    assert_eq!(transition_hint(&new, &old, FILE), Hint::Backfill(FILE));
    let mut changed = s.clone();
    changed.revision += 1;
    changed.rules.clear();
    c.policy_revision = changed.revision;
    assert_eq!(policy_evaluate(&c, &changed), oracle::expected(0));
    assert_eq!(
        transition_hint(&new, &policy_evaluate(&c, &changed), FILE),
        Hint::Backfill(FILE)
    );
    c.parents[1].parent = Some(OS);
    assert_eq!(
        transition_hint(&old, &policy_evaluate(&c, &changed), FILE),
        Hint::Reconcile(FILE)
    );
}

#[test]
fn t12_canonical_fingerprint_cache_and_generation_replay() {
    let (c, mut s) = fixture(5);
    let mut cache = EvaluationCache::default();
    assert_eq!(cache.evaluate(&c, &s), oracle::expected(5));
    s.rules.reverse();
    assert_eq!(cache.evaluate(&c, &s), oracle::expected(5));
    assert_eq!((cache.evaluations, cache.hits), (1, 1));
    let before = fingerprint(&s);
    for field in 0..10 {
        let mut next = s.clone();
        match field {
            0 => next.source.source_epoch += 1,
            1 => next.source.volume_epoch += 1,
            2 => next.source.provider_epoch += 1,
            3 => next.source.mount_epoch += 1,
            4 => next.source.permission_epoch += 1,
            5 => next.revision += 1,
            6 => next.known_folder_version += 1,
            7 => next.roots[0].identity.birth += 1,
            8 => next.rules[0].include = !next.rules[0].include,
            9 => next.protected.push(FILE),
            _ => unreachable!(),
        }
        assert_ne!(fingerprint(&next), before);
        assert_eq!(cache.evaluate(&c, &next), policy_evaluate(&c, &next));
        if field < 5 {
            assert_eq!(policy_evaluate(&c, &next), oracle::expected(15));
        }
        if field == 5 {
            assert_eq!(policy_evaluate(&c, &next), oracle::expected(14));
        }
    }
    assert_eq!(cache.evaluations, 11);
    let mut contexts = Vec::new();
    for field in 0..7 {
        let mut next = c.clone();
        match field {
            0 => next.rename_epoch += 1,
            1 => next.permission = false,
            2 => next.parents[1].parent = None,
            3 => next.identity.birth += 1,
            4 => next.metadata_complete = false,
            5 => next.health = SourceHealth::CheckpointGap,
            6 => next.unicode_path = Some("C:/Users/owner/Projects/elsewhere".into()),
            _ => unreachable!(),
        }
        contexts.push(next);
    }
    for next in contexts {
        assert_eq!(cache.evaluate(&next, &s), policy_evaluate(&next, &s));
    }
    assert_eq!(cache.evaluations, 18);
    for identity in 0..3 {
        let mut unrelated = s.clone();
        match identity {
            0 => unrelated.source.source_id = "other-source-same-epoch".into(),
            1 => unrelated.source.volume_id = "other-volume-same-epoch".into(),
            2 => unrelated.source.provider_id = "other-provider-same-epoch".into(),
            _ => unreachable!(),
        }
        assert_ne!(fingerprint(&unrelated), fingerprint(&s));
        assert_eq!(cache.evaluate(&c, &unrelated), oracle::expected(15));
    }
    let (mut empty_identity, mut empty_snapshot) = fixture(0);
    empty_identity.source.source_id.clear();
    empty_snapshot.source.source_id.clear();
    assert_eq!(
        policy_evaluate(&empty_identity, &empty_snapshot),
        oracle::expected(15)
    );
    let (mut old_anchor, mut new_snapshot) = fixture(7);
    new_snapshot.revision += 1;
    old_anchor.policy_revision = new_snapshot.revision;
    assert_eq!(
        policy_evaluate(&old_anchor, &new_snapshot),
        oracle::expected(14)
    );
    let (stale, snapshot) = fixture(14);
    let result = policy_evaluate(&stale, &snapshot);
    assert_eq!(
        transition_hint(&oracle::expected(0), &result, FILE),
        Hint::RejectGeneration
    );
    let stage = StageAccounting {
        evaluated_fingerprint: fingerprint(&snapshot),
        enumerated: 0,
        evaluated: 0,
        unresolved: 0,
        native_stages_verified: true,
        source: snapshot.source.clone(),
        policy_revision: 0,
    };
    assert_eq!(
        coverage(&snapshot, SourceHealth::Accounted, &stage, &[]),
        CoverageProjection::Obsolete
    );
    let mut advanced = snapshot.clone();
    advanced.revision += 1;
    let old_stage = StageAccounting {
        policy_revision: snapshot.revision,
        ..stage
    };
    assert_eq!(
        coverage(&advanced, SourceHealth::Accounted, &old_stage, &[]),
        CoverageProjection::Obsolete
    );
    for variant in 0..3 {
        let mut same_revision = snapshot.clone();
        match variant {
            0 => same_revision.known_folder_version += 1,
            1 => same_revision
                .rules
                .push(rule("C:/Users/owner/Projects", false, "changed-rule")),
            2 => same_revision.roots[0].identity.birth += 1,
            _ => unreachable!(),
        }
        assert_eq!(
            coverage(&same_revision, SourceHealth::Accounted, &old_stage, &[]),
            CoverageProjection::Obsolete
        );
    }
}

#[test]
fn t13_abstract_platform_provider_parity_is_not_native_evidence() {
    for witness in 0..oracle::WITNESSES {
        for (platform, provider) in [
            (Platform::Windows, Provider::Service),
            (Platform::Windows, Provider::Direct),
            (Platform::Macos, Provider::Spotlight),
        ] {
            let (mut c, mut s) = fixture(witness);
            c.source.platform = platform;
            s.source.platform = platform;
            c.source.provider = provider;
            s.source.provider = provider;
            for edge in &mut c.aliases {
                edge.source.platform = platform;
                edge.source.provider = provider;
            }
            if platform == Platform::Macos {
                c.path = c.path.replace("C:/", "/");
                c.performance_path = c.performance_path.map(|p| p.replace("C:/", "/"));
                for root in &mut s.roots {
                    root.path = root.path.replace("C:/", "/");
                }
                for rule in &mut s.rules {
                    rule.path = rule.path.replace("C:/", "/");
                }
            }
            if let Some(anchor) = &mut c.anchor {
                *anchor.included_snapshot = s.clone();
            }
            assert_eq!(policy_evaluate(&c, &s), oracle::expected(witness));
        }
    }
}

#[test]
fn t14_mocked_authorities_and_user_corrections_are_untouched() {
    // Sentinels represent independent fixture authorities, never production IO.
    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Authorities {
        legacy_global: Vec<(u64, String)>,
        managed_roots: Vec<String>,
        ai_eligible: bool,
        content_consent: bool,
        corrections: Vec<String>,
        history: Vec<String>,
        physical_files: Vec<String>,
        schema: u32,
        ipc: u32,
    }
    let authority = Authorities {
        legacy_global: vec![(1, "kept".into())],
        managed_roots: vec!["C:/Managed".into()],
        ai_eligible: false,
        content_consent: false,
        corrections: vec!["owner correction".into()],
        history: vec!["history reference".into()],
        physical_files: vec!["user file".into()],
        schema: 37,
        ipc: 3,
    };
    let before = authority.clone();
    for mode in [Mode::Broad, Mode::SmartLean, Mode::Custom] {
        for witness in 0..oracle::WITNESSES {
            let (c, mut s) = fixture(witness);
            s.mode = mode;
            let frozen = (c.clone(), s.clone());
            let old = policy_evaluate(&c, &s);
            s.revision += 1;
            s.rules
                .push(rule("C:/Users/owner/Projects", false, "policy-change"));
            let mut current = c.clone();
            current.policy_revision = s.revision;
            let new = policy_evaluate(&current, &s);
            transition_hint(&old, &new, FILE);
            assert_eq!(c, frozen.0);
            assert_eq!(authority, before);
        }
    }
}

#[test]
fn t15_parent_order_duplicate_gaps_reuse_and_checkpoints() {
    let (mut c, s) = check(0);
    c.parents.reverse();
    assert_eq!(policy_evaluate(&c, &s), oracle::expected(0));
    check(16);
    check(9);
    c.parents
        .iter_mut()
        .find(|n| n.id == ROOT)
        .unwrap()
        .id
        .birth += 1;
    assert_eq!(policy_evaluate(&c, &s), oracle::expected(9));
    c = fixture(0).0;
    c.parents
        .iter_mut()
        .find(|n| n.id == FILE)
        .unwrap()
        .id
        .birth += 1;
    assert_eq!(policy_evaluate(&c, &s), oracle::expected(9));
    c = fixture(0).0;
    c.health = SourceHealth::CheckpointGap;
    assert_unknown(
        &c,
        &s,
        Reason::CheckpointGap,
        "coverage-critical source accounting",
    );
}

fn next_random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}
#[test]
fn t16_deterministic_small_corpus_oracle_reshuffle_cache_and_counters() {
    const SEEDS: [u64; 2] = [0x4953_4702, 0x0061_0781_6377];
    for count in [1_000, 10_000] {
        for seed in SEEDS {
            let started = Instant::now();
            let mut random = seed;
            let mut samples = Vec::with_capacity(count);
            let mut histogram = BTreeMap::new();
            for index in 0..count {
                let witness = (next_random(&mut random) as usize) % oracle::WITNESSES;
                let (mut c, s) = fixture(witness);
                // Unique path occurrences test hardlink-sensitive caching.
                c.path = format!("{}/sample-{index}", c.path);
                let mut s = s;
                if next_random(&mut random) & 1 == 1 {
                    c.parents.reverse();
                    s.rules.reverse();
                }
                samples.push((witness, c, s));
                *histogram.entry(witness).or_insert(0_usize) += 1;
            }
            assert_eq!(histogram.len(), oracle::WITNESSES);
            let mut cache = EvaluationCache::default();
            for (witness, c, s) in &samples {
                let before = (c.clone(), s.clone());
                let expected = oracle::expected(*witness);
                assert_eq!(
                    policy_evaluate(c, s),
                    expected,
                    "seed={seed} witness={witness}"
                );
                assert_eq!(cache.evaluate(c, s), expected);
                assert_eq!((c.clone(), s.clone()), before);
            }
            for i in (1..samples.len()).rev() {
                let j = (next_random(&mut random) as usize) % (i + 1);
                samples.swap(i, j);
            }
            for (witness, c, s) in &samples {
                assert_eq!(cache.evaluate(c, s), oracle::expected(*witness));
                assert_eq!(policy_evaluate(c, s), oracle::expected(*witness));
            }
            assert_eq!(cache.evaluations, count);
            assert_eq!(cache.hits, count);
            eprintln!("ISG-02 samples={count} seed={seed:#x} uncached={} evaluations={} hits={} elapsed_ms={} witnesses={histogram:?}",
                2 * count, cache.evaluations, cache.hits, started.elapsed().as_millis());
        }
    }
}
