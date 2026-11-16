//! The cross-MCP trigger layer (DESIGN §D4), rung 1.
//!
//! Rules are data (`triggers.toml`), listed in the Fleet panel;
//! a rule that is not listed does not exist. The engine is a
//! bounded predicate over named event fields — comparisons
//! only, no expression language, no eval. Firings are keyed by a
//! hash of (rule id, event payload) so a duplicate event never
//! re-fires, and every evaluation outcome appends an audit line
//! carrying the rule's birth metadata (`origin_lane` +
//! `content_label`, BLOCH lane law D-1030).
//!
//! Rung-1 scope, stated honestly: the engine evaluates, audits,
//! and records firings into a sink the app owns. The live MCP
//! dispatch to bloch is **not** wired in this rung — the Fleet
//! panel and the member report say so; the sink is fake-tested.

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};

use serde::Deserialize;
use serde_json::{Value, json};

/// Audit lines kept in memory, newest last.
pub const AUDIT_LINES_MAX: usize = 200;

/// One declared trigger rule.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Rule {
    /// Stable rule id (also part of the idempotency key).
    pub id: String,
    /// BLOCH birth metadata: origin lane, stamped from birth.
    pub origin_lane: String,
    /// BLOCH birth metadata: content label (`sfw` here).
    pub content_label: String,
    /// The event name that evaluates this rule.
    pub event: String,
    /// The bounded `where`: when true, the rule additionally
    /// requires the event's `all_gates_green` field to be true.
    #[serde(default)]
    pub require_all_gates_green: bool,
    /// Target MCP server name.
    pub target_mcp: String,
    /// Target tool on that server.
    pub target_tool: String,
    /// `auto` or `plan` (kept as data; rung 1 records firings).
    pub mode: String,
    /// Disabled rules never fire; toggling takes effect at once.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct RulesFile {
    #[serde(default)]
    rule: Vec<Rule>,
}

/// The one rule this member ships in rung 1: a pipeline that
/// completes with every certification gate green produces a
/// bloch lesson candidate, born `qompass` / `sfw`.
pub fn default_rules() -> Vec<Rule> {
    vec![Rule {
        id: "slipway-pipeline-to-bloch-lesson".to_string(),
        origin_lane: "qompass".to_string(),
        content_label: "sfw".to_string(),
        event: "pipeline.completed".to_string(),
        require_all_gates_green: true,
        target_mcp: "bloch".to_string(),
        target_tool: "lesson_candidate".to_string(),
        mode: "auto".to_string(),
        enabled: true,
    }]
}

/// Load rules from a `triggers.toml` document. A malformed file
/// is an error the caller surfaces — never a silent fallback
/// that would hide a rule the operator thinks exists.
pub fn load_rules(text: &str) -> Result<Vec<Rule>, String> {
    let parsed: RulesFile =
        toml::from_str(text).map_err(|e| format!("triggers.toml is malformed: {e}"))?;
    Ok(parsed.rule)
}

/// A recorded firing: what would be dispatched to the target.
#[derive(Debug, Clone, PartialEq)]
pub struct Firing {
    /// The rule that fired.
    pub rule_id: String,
    /// Target MCP server.
    pub target_mcp: String,
    /// Target tool.
    pub target_tool: String,
    /// The call arguments, carrying birth metadata.
    pub args: Value,
}

/// The rule engine: rules + idempotency keys + audit + sink.
#[derive(Debug)]
pub struct TriggerEngine {
    rules: Vec<Rule>,
    fired_keys: HashSet<u64>,
    audit: Vec<String>,
    firings: Vec<Firing>,
}

impl TriggerEngine {
    /// Build an engine over declared rules.
    pub fn new(rules: Vec<Rule>) -> TriggerEngine {
        TriggerEngine {
            rules,
            fired_keys: HashSet::new(),
            audit: Vec::new(),
            firings: Vec::new(),
        }
    }

    /// The declared rules (the Fleet panel lists exactly these).
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Enable or disable a rule by id, effective immediately.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> bool {
        if let Some(rule) = self.rules.iter_mut().find(|r| r.id == id) {
            rule.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// Every firing recorded so far.
    pub fn firings(&self) -> &[Firing] {
        &self.firings
    }

    /// The audit tail (oldest first, bounded).
    pub fn audit(&self) -> &[String] {
        &self.audit
    }

    fn push_audit(&mut self, line: String) {
        if self.audit.len() >= AUDIT_LINES_MAX {
            self.audit.remove(0);
        }
        self.audit.push(line);
    }

    /// Evaluate one event. Returns the firings it produced
    /// (usually zero or one). Duplicate events are dropped and
    /// audited; non-matching evaluations are audited too.
    pub fn evaluate(&mut self, event: &str, all_gates_green: bool, payload: &str) -> Vec<Firing> {
        let mut produced = Vec::new();
        // Collect matches first so the audit/sink writes below
        // borrow nothing from the rule list.
        let matching: Vec<Rule> = self
            .rules
            .iter()
            .filter(|r| r.enabled && r.event == event)
            .cloned()
            .collect();
        for rule in matching {
            if rule.require_all_gates_green && !all_gates_green {
                self.push_audit(
                    json!({"rule": rule.id, "event": event, "outcome": "no-match",
                           "reason": "all_gates_green is false",
                           "origin_lane": rule.origin_lane,
                           "content_label": rule.content_label})
                    .to_string(),
                );
                continue;
            }
            let mut hasher = DefaultHasher::new();
            rule.id.hash(&mut hasher);
            payload.hash(&mut hasher);
            let key = hasher.finish();
            if !self.fired_keys.insert(key) {
                self.push_audit(
                    json!({"rule": rule.id, "event": event, "outcome": "dropped",
                           "reason": "duplicate event (idempotency key seen)",
                           "origin_lane": rule.origin_lane,
                           "content_label": rule.content_label})
                    .to_string(),
                );
                continue;
            }
            let firing = Firing {
                rule_id: rule.id.clone(),
                target_mcp: rule.target_mcp.clone(),
                target_tool: rule.target_tool.clone(),
                args: json!({
                    "summary": payload,
                    "origin_lane": rule.origin_lane,
                    "content_label": rule.content_label,
                }),
            };
            self.push_audit(
                json!({"rule": rule.id, "event": event, "outcome": "fired",
                       "target": format!("{}.{}", rule.target_mcp, rule.target_tool),
                       "mode": rule.mode,
                       "origin_lane": rule.origin_lane,
                       "content_label": rule.content_label})
                .to_string(),
            );
            self.firings.push(firing.clone());
            produced.push(firing);
        }
        produced
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn green_completion_fires_bloch_candidate_with_birth_metadata() {
        let mut engine = TriggerEngine::new(default_rules());
        let fired = engine.evaluate("pipeline.completed", true, "run 1: all steps done");
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].target_mcp, "bloch");
        assert_eq!(fired[0].target_tool, "lesson_candidate");
        assert_eq!(fired[0].args["origin_lane"], json!("qompass"));
        assert_eq!(fired[0].args["content_label"], json!("sfw"));
    }

    #[test]
    fn non_green_completion_does_not_fire() {
        let mut engine = TriggerEngine::new(default_rules());
        let fired = engine.evaluate("pipeline.completed", false, "run 2");
        assert!(fired.is_empty());
        assert!(engine.audit().iter().any(|l| l.contains("no-match")));
    }

    #[test]
    fn duplicate_event_never_refires() {
        let mut engine = TriggerEngine::new(default_rules());
        assert_eq!(engine.evaluate("pipeline.completed", true, "same").len(), 1);
        assert_eq!(engine.evaluate("pipeline.completed", true, "same").len(), 0);
        assert!(engine.audit().iter().any(|l| l.contains("dropped")));
    }

    #[test]
    fn disabled_rule_does_not_fire() {
        let mut engine = TriggerEngine::new(default_rules());
        assert!(engine.set_enabled("slipway-pipeline-to-bloch-lesson", false));
        assert!(
            engine
                .evaluate("pipeline.completed", true, "run 3")
                .is_empty()
        );
    }

    #[test]
    fn shipped_triggers_toml_parses_to_the_default_rule() {
        let text = include_str!("../triggers.toml");
        let rules = load_rules(text).expect("parse shipped triggers.toml");
        assert_eq!(rules, default_rules());
    }

    #[test]
    fn malformed_toml_is_an_error_not_a_silent_empty() {
        assert!(load_rules("[[rule]]\nid = 7").is_err());
    }
}
