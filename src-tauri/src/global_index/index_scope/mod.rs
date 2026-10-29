//! Index Scope only. This module grants neither source access nor AI/mutation authority.
pub mod command;
mod native;
mod repository;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

pub const POLICY_VERSION: u32 = 1;
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoverageMode {
    #[default]
    SmartLean,
    BroadCoverage,
    Custom,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CoverageRule {
    pub source_id: String,
    pub stable_volume_id: String,
    pub path: String,
    pub directory_identity: String,
    pub include: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CoveragePolicy {
    pub version: u32,
    pub revision: u64,
    pub mode: CoverageMode,
    pub rules: Vec<CoverageRule>,
}
impl Default for CoveragePolicy {
    fn default() -> Self {
        Self {
            version: POLICY_VERSION,
            revision: 1,
            mode: CoverageMode::SmartLean,
            rules: Vec::new(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Include,
    MinimalAncestor(&'static str),
    Exclude(&'static str),
    Undetermined(&'static str),
}
impl Decision {
    pub fn visible(&self) -> bool {
        matches!(self, Self::Include | Self::Undetermined(_))
    }
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Include => "include",
            Self::MinimalAncestor(r) | Self::Exclude(r) | Self::Undetermined(r) => r,
        }
    }
}
/// Facts are supplied by a source-bound native observer, never inferred from a basename alone.
#[derive(Debug, Default)]
pub struct Evidence {
    pub protected: bool,
    pub uncertain: Option<&'static str>,
    pub performance_reason: Option<&'static str>,
    pub explicit_rule: Option<bool>,
    pub directory: bool,
}
pub fn evaluate(policy: &CoveragePolicy, evidence: &Evidence) -> Decision {
    if evidence.protected {
        return Decision::Exclude("protected_boundary");
    }
    if let Some(reason) = evidence.uncertain {
        return Decision::Undetermined(reason);
    }
    if policy.mode == CoverageMode::Custom {
        if let Some(include) = evidence.explicit_rule {
            return if include {
                Decision::Include
            } else if evidence.directory {
                Decision::MinimalAncestor("custom_exclude")
            } else {
                Decision::Exclude("custom_exclude")
            };
        }
    }
    if policy.mode != CoverageMode::BroadCoverage {
        if let Some(reason) = evidence.performance_reason {
            return if evidence.directory {
                Decision::MinimalAncestor(reason)
            } else {
                Decision::Exclude(reason)
            };
        }
    }
    Decision::Include
}

pub use native::bind_rule;
pub(crate) use native::{observe_batch, source_identity_current};
pub(crate) use repository::{load_policy, record_decisions};
pub use repository::{CoverageSnapshot, CoverageSourceProgress};
