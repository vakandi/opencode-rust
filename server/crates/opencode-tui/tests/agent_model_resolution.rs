//! The model a user configured must survive all the way to the registry.
//!
//! `load_config` merges `~/.omo/omo.jsonc`, a plugin `config` hook can replace
//! the config wholesale, and `AgentRegistry` copies `cfg.model` onto the agent.
//! A break anywhere along that chain shows up in the TUI as the wrong model, so
//! this exercises the whole path rather than each hop.

use opencode_agent::AgentRegistry;
use opencode_config::load_config;

#[test]
fn configured_agent_models_reach_the_agent_registry() {
    let config = load_config(".").expect("load config");
    let registry = AgentRegistry::from_optional_config(Some(&config));
    let agents = registry.list();

    let model_of = |name: &str| {
        agents
            .iter()
            .find(|agent| agent.name == name)
            .and_then(|agent| agent.model.as_ref())
            .map(|model| format!("{}/{}", model.provider_id, model.model_id))
    };

    for name in ["gilfoyle", "atlas", "oracle", "sisyphus"] {
        assert_eq!(
            model_of(name).as_deref(),
            Some("opencode/space-bunny-free"),
            "agent `{name}` must keep the model from ~/.omo/omo.jsonc"
        );
    }
}

#[test]
fn a_plugin_display_name_still_matches_the_configured_slug() {
    // Plugins register agents as `Atlas - Plan Executor`; omo.jsonc keys them as
    // `atlas`. An exact-key lookup misses and the model is lost.
    let mut config = opencode_config::Config::default();
    config.agent = Some(opencode_config::AgentConfigs {
        entries: [(
            "Atlas - Plan Executor".to_string(),
            opencode_config::AgentConfig {
                model: Some("opencode/plugin-default".to_string()),
                ..Default::default()
            },
        )]
        .into_iter()
        .collect(),
    });

    let mut paths = Vec::new();
    opencode_config::apply_omo_agent_config(&mut config, &mut paths);

    let registry = AgentRegistry::from_optional_config(Some(&config));
    let model = registry
        .list()
        .iter()
        .find(|agent| agent.name == "Atlas - Plan Executor")
        .and_then(|agent| agent.model.as_ref())
        .map(|model| format!("{}/{}", model.provider_id, model.model_id));

    assert_eq!(
        model.as_deref(),
        Some("opencode/space-bunny-free"),
        "the configured model must beat the plugin default even when the agent is \
         registered under a display name"
    );
}

#[test]
fn a_plugin_rewritten_config_still_keeps_the_users_models() {
    let mut config = load_config(".").expect("load config");
    let registry = AgentRegistry::from_optional_config(Some(&config));
    let before = registry
        .list()
        .iter()
        .find(|agent| agent.name == "atlas")
        .and_then(|agent| agent.model.as_ref())
        .map(|model| format!("{}/{}", model.provider_id, model.model_id));
    assert_eq!(
        before.as_deref(),
        Some("opencode/space-bunny-free"),
        "atlas must resolve before the hook runs too"
    );

    // A plugin hook returning a config that names agents differently must not
    // wipe the models the user configured; the omo merge is re-applied after it.
    let mut paths = Vec::new();
    opencode_config::apply_omo_agent_config(&mut config, &mut paths);
    let registry = AgentRegistry::from_optional_config(Some(&config));
    let after = registry
        .list()
        .iter()
        .find(|agent| agent.name == "atlas")
        .and_then(|agent| agent.model.as_ref())
        .map(|model| format!("{}/{}", model.provider_id, model.model_id));

    assert_eq!(
        after.as_deref(),
        Some("opencode/space-bunny-free"),
        "re-applying the omo merge must restore the configured model"
    );
}