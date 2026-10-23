//! Hand-authored expected-result ledger, independent of the evaluator.
//! This module imports only result vocabulary and Identity; it cannot inspect
//! Context, Snapshot, mode, basename, graph, matcher, cache or CorpusClass.
//! Fixture authors choose a numbered witness; expected results are literal.
use super::model::{Decision, Evidence, Identity, Reason};

pub const ROOT: Identity = Identity {
    number: 1,
    birth: 1,
};
pub const FILE: Identity = Identity {
    number: 2,
    birth: 1,
};
pub const OS: Identity = Identity {
    number: 9,
    birth: 1,
};
pub const CHILD: Identity = Identity {
    number: 3,
    birth: 1,
};
pub const WITNESSES: usize = 18;

pub fn expected(witness: usize) -> Decision {
    match witness {
        0 => Decision::Include {
            evidence: Evidence::LawfulScope,
        },
        1 => Decision::Exclude {
            reason: Reason::Performance,
            evidence: Evidence::Dependency {
                project: ROOT,
                manifest: "package-lock.json".into(),
            },
        },
        2 => Decision::Exclude {
            reason: Reason::Performance,
            evidence: Evidence::Generated {
                project: ROOT,
                signature: "compiler-output".into(),
            },
        },
        3 => Decision::Exclude {
            reason: Reason::Performance,
            evidence: Evidence::Cache {
                owner: ROOT,
                signature: "ephemeral-cache".into(),
            },
        },
        4 => Decision::Include {
            evidence: Evidence::ExplicitRule("nested-include".into()),
        },
        5 => Decision::Exclude {
            reason: Reason::ExplicitExclude,
            evidence: Evidence::ExplicitRule("equal-exclude".into()),
        },
        6 => Decision::Exclude {
            reason: Reason::Protected,
            evidence: Evidence::ProtectedIdentity(OS),
        },
        7 => Decision::MinimalAncestor {
            need: "included-child".into(),
            descendant_identity: CHILD,
        },
        8 => Decision::Undetermined {
            reason: Reason::Classification,
            missing_evidence: "contextual classification evidence".into(),
        },
        9 => Decision::Undetermined {
            reason: Reason::MissingParent,
            missing_evidence: "complete unique bounded parent identity graph".into(),
        },
        10 => Decision::Undetermined {
            reason: Reason::Cycle,
            missing_evidence: "acyclic alias traversal".into(),
        },
        11 => Decision::Undetermined {
            reason: Reason::AliasTarget,
            missing_evidence: "verified alias target identity".into(),
        },
        12 => Decision::Undetermined {
            reason: Reason::SourceBusy,
            missing_evidence: "coverage-critical source accounting".into(),
        },
        13 => Decision::Undetermined {
            reason: Reason::Permission,
            missing_evidence: "verified permission".into(),
        },
        14 => Decision::Undetermined {
            reason: Reason::StaleGeneration,
            missing_evidence: "current policy revision".into(),
        },
        15 => Decision::Undetermined {
            reason: Reason::SourceIdentity,
            missing_evidence: "current source/volume/provider/mount/permission identity".into(),
        },
        16 => Decision::Undetermined {
            reason: Reason::DuplicateParent,
            missing_evidence: "complete unique bounded parent identity graph".into(),
        },
        17 => Decision::Undetermined {
            reason: Reason::Metadata,
            missing_evidence: "complete metadata".into(),
        },
        _ => panic!("unregistered oracle witness {witness}"),
    }
}
