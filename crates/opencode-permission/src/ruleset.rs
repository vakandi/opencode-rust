use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionAction {
    #[serde(rename = "allow")]
    Allow,
    #[serde(rename = "deny")]
    Deny,
    #[serde(rename = "ask")]
    Ask,
}

impl Default for PermissionAction {
    fn default() -> Self {
        Self::Ask
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    pub permission: String,
    pub pattern: String,
    pub action: PermissionAction,
}

pub type PermissionRuleset = Vec<PermissionRule>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfigValue {
    Action(PermissionAction),
    Patterns(HashMap<String, PermissionAction>),
}

pub type ConfigPermission = HashMap<String, ConfigValue>;

fn expand(pattern: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        if pattern.starts_with("~/") {
            return format!("{}{}", home.display(), &pattern[1..]);
        }
        if pattern == "~" {
            return home.display().to_string();
        }
        if pattern.starts_with("$HOME/") {
            return format!("{}{}", home.display(), &pattern[5..]);
        }
    }
    pattern.to_string()
}

pub fn from_config(permission: &ConfigPermission) -> PermissionRuleset {
    let mut ruleset: PermissionRuleset = Vec::new();

    for (key, value) in permission.iter() {
        match value {
            ConfigValue::Action(action) => {
                ruleset.push(PermissionRule {
                    permission: key.clone(),
                    action: *action,
                    pattern: "*".to_string(),
                });
            }
            ConfigValue::Patterns(patterns) => {
                for (pattern, action) in patterns.iter() {
                    ruleset.push(PermissionRule {
                        permission: key.clone(),
                        pattern: expand(pattern),
                        action: *action,
                    });
                }
            }
        }
    }

    ruleset
}

pub fn merge(rulesets: &[PermissionRuleset]) -> PermissionRuleset {
    rulesets.iter().flat_map(|r| r.clone()).collect()
}

pub fn evaluate(permission: &str, pattern: &str, rulesets: &[PermissionRuleset]) -> PermissionRule {
    let merged = merge(rulesets);

    let matched = merged.iter().rev().find(|rule| {
        wildcard_match(permission, &rule.permission) && wildcard_match(pattern, &rule.pattern)
    });

    matched.cloned().unwrap_or(PermissionRule {
        action: PermissionAction::Ask,
        permission: permission.to_string(),
        pattern: "*".to_string(),
    })
}

const EDIT_TOOLS: &[&str] = &["edit", "write", "patch", "multiedit"];

pub fn disabled(
    tools: &[String],
    ruleset: &PermissionRuleset,
) -> std::collections::HashSet<String> {
    let mut result = std::collections::HashSet::new();

    for tool in tools {
        let permission = if EDIT_TOOLS.contains(&tool.as_str()) {
            "edit"
        } else {
            tool.as_str()
        };

        let rule = ruleset
            .iter()
            .rev()
            .find(|r| wildcard_match(permission, &r.permission));

        if let Some(rule) = rule {
            if rule.pattern == "*" && rule.action == PermissionAction::Deny {
                result.insert(tool.clone());
            }
        }
    }

    result
}

fn wildcard_match(text: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    if pattern.starts_with('*') && pattern.ends_with('*') {
        let middle = &pattern[1..pattern.len() - 1];
        return text.contains(middle);
    }

    if pattern.starts_with('*') {
        let suffix = &pattern[1..];
        return text.ends_with(suffix);
    }

    if pattern.ends_with('*') {
        let prefix = &pattern[..pattern.len() - 1];
        return text.starts_with(prefix);
    }

    text == pattern
}

pub fn default_ruleset() -> PermissionRuleset {
    let mut rules = Vec::new();

    rules.push(PermissionRule {
        permission: "*".to_string(),
        pattern: "*".to_string(),
        action: PermissionAction::Allow,
    });

    rules.push(PermissionRule {
        permission: "doom_loop".to_string(),
        pattern: "*".to_string(),
        action: PermissionAction::Ask,
    });

    rules.push(PermissionRule {
        permission: "external_directory".to_string(),
        pattern: "*".to_string(),
        action: PermissionAction::Ask,
    });

    rules.push(PermissionRule {
        permission: "question".to_string(),
        pattern: "*".to_string(),
        action: PermissionAction::Deny,
    });

    rules.push(PermissionRule {
        permission: "plan_enter".to_string(),
        pattern: "*".to_string(),
        action: PermissionAction::Deny,
    });

    rules.push(PermissionRule {
        permission: "plan_exit".to_string(),
        pattern: "*".to_string(),
        action: PermissionAction::Deny,
    });

    rules.push(PermissionRule {
        permission: "read".to_string(),
        pattern: "*.env".to_string(),
        action: PermissionAction::Ask,
    });

    rules.push(PermissionRule {
        permission: "read".to_string(),
        pattern: "*.env.*".to_string(),
        action: PermissionAction::Ask,
    });

    rules.push(PermissionRule {
        permission: "read".to_string(),
        pattern: "*.env.example".to_string(),
        action: PermissionAction::Allow,
    });

    rules
}

pub fn build_agent_ruleset(agent_name: &str, user_ruleset: &[PermissionRule]) -> PermissionRuleset {
    let defaults = default_ruleset();
    let user = user_ruleset.to_vec();

    match agent_name {
        "build" => {
            let build_specific = vec![
                PermissionRule {
                    permission: "question".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "plan_enter".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
            ];
            merge(&[defaults, build_specific, user])
        }
        "plan" => {
            let plan_specific = vec![
                PermissionRule {
                    permission: "question".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "plan_exit".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "edit".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Deny,
                },
            ];
            merge(&[defaults, plan_specific, user])
        }
        "explore" => {
            let explore_specific = vec![
                PermissionRule {
                    permission: "*".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Deny,
                },
                PermissionRule {
                    permission: "grep".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "glob".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "list".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "bash".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "webfetch".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "websearch".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "codesearch".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
                PermissionRule {
                    permission: "read".to_string(),
                    pattern: "*".to_string(),
                    action: PermissionAction::Allow,
                },
            ];
            merge(&[explore_specific, user])
        }
        _ => merge(&[defaults, user]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_config() {
        let mut config = HashMap::new();
        config.insert(
            "bash".to_string(),
            ConfigValue::Action(PermissionAction::Allow),
        );

        let ruleset = from_config(&config);
        assert_eq!(ruleset.len(), 1);
        assert_eq!(ruleset[0].permission, "bash");
        assert_eq!(ruleset[0].action, PermissionAction::Allow);
    }

    #[test]
    fn test_wildcard_match() {
        assert!(wildcard_match("foo", "*"));
        assert!(wildcard_match("foo/bar", "foo/*"));
        assert!(wildcard_match("foo/bar/baz", "*/baz"));
        assert!(wildcard_match("foo/bar/baz", "*bar*"));
        assert!(!wildcard_match("foo", "bar"));
    }

    #[test]
    fn test_disabled() {
        let ruleset = vec![PermissionRule {
            permission: "bash".to_string(),
            pattern: "*".to_string(),
            action: PermissionAction::Deny,
        }];

        let tools = vec!["bash".to_string(), "read".to_string()];
        let disabled_tools = disabled(&tools, &ruleset);

        assert!(disabled_tools.contains("bash"));
        assert!(!disabled_tools.contains("read"));
    }
}
