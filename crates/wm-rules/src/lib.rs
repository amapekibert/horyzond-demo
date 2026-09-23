//! Data-only ordered window rules with explicit non-recursive evaluation.

pub use wm_types::WindowMetadata;
/// A bounded literal match predicate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Match {
    pub app_id_contains: Option<String>,
    pub title_contains: Option<String>,
}
/// One opaque action selected by a rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleAction {
    pub name: String,
    pub arguments: Vec<String>,
}
/// One priority-ordered rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rule {
    pub priority: u32,
    pub matcher: Match,
    pub actions: Vec<RuleAction>,
    pub stop: bool,
}
/// The committed outcome for one metadata evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleOutcome {
    pub actions: Vec<RuleAction>,
}
/// Immutable deterministic evaluator. Actions are returned, never executed recursively.
#[derive(Clone, Debug)]
pub struct RuleEngine {
    rules: Vec<Rule>,
}
impl RuleEngine {
    /// Sorts rules by priority while preserving source order for equal priorities.
    #[must_use]
    pub fn new(mut rules: Vec<Rule>) -> Self {
        rules.sort_by_key(|rule| rule.priority);
        Self { rules }
    }
    /// Evaluates metadata once and returns ordered actions.
    #[must_use]
    pub fn evaluate(&self, metadata: &WindowMetadata) -> RuleOutcome {
        let mut actions = Vec::new();
        for rule in &self.rules {
            if matches(&rule.matcher, metadata) {
                actions.extend(rule.actions.clone());
                if rule.stop {
                    break;
                }
            }
        }
        RuleOutcome { actions }
    }
}
fn matches(matcher: &Match, metadata: &WindowMetadata) -> bool {
    matcher
        .app_id_contains
        .as_ref()
        .is_none_or(|value| metadata.app_id.contains(value))
        && matcher
            .title_contains
            .as_ref()
            .is_none_or(|value| metadata.title.contains(value))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn action(name: &str) -> RuleAction {
        RuleAction {
            name: name.to_owned(),
            arguments: vec![],
        }
    }
    #[test]
    fn priority_and_stop_make_rule_results_deterministic() {
        let engine = RuleEngine::new(vec![
            Rule {
                priority: 20,
                matcher: Match {
                    app_id_contains: Some("term".to_owned()),
                    title_contains: None,
                },
                actions: vec![action("late")],
                stop: false,
            },
            Rule {
                priority: 10,
                matcher: Match {
                    app_id_contains: Some("term".to_owned()),
                    title_contains: None,
                },
                actions: vec![action("first")],
                stop: true,
            },
            Rule {
                priority: 30,
                matcher: Match {
                    app_id_contains: None,
                    title_contains: None,
                },
                actions: vec![action("never")],
                stop: false,
            },
        ]);
        assert_eq!(
            engine
                .evaluate(&WindowMetadata {
                    app_id: "terminal".to_owned(),
                    title: String::new()
                })
                .actions,
            vec![action("first")]
        );
    }
}
