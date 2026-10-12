use opencode_provider::{ChatRequest, Message, ModelInfo, ProviderRegistry};

fn create_test_registry() -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();

    registry.register(opencode_provider::anthropic::AnthropicProvider::new(
        "test-key",
    ));
    registry.register(opencode_provider::openai::OpenAIProvider::new("test-key"));
    registry.register(opencode_provider::google::GoogleProvider::new("test-key"));
    registry.register(opencode_provider::deepseek::DeepSeekProvider::new(
        "test-key",
    ));
    registry.register(opencode_provider::mistral::MistralProvider::new("test-key"));
    registry.register(opencode_provider::groq::GroqProvider::new("test-key"));
    registry.register(opencode_provider::xai::XaiProvider::new("test-key"));
    registry.register(opencode_provider::cohere::CohereProvider::new("test-key"));
    registry.register(opencode_provider::cerebras::CerebrasProvider::new(
        "test-key",
    ));
    registry.register(opencode_provider::together::TogetherProvider::new(
        "test-key",
    ));
    registry.register(opencode_provider::perplexity::PerplexityProvider::new(
        "test-key",
    ));
    registry.register(opencode_provider::openrouter::OpenRouterProvider::new(
        "test-key",
    ));

    registry
}

#[test]
fn test_registry_lists_providers() {
    let registry = create_test_registry();
    let providers = registry.list_providers();

    assert!(!providers.is_empty(), "Registry should have providers");

    let provider_ids: Vec<&str> = providers.iter().map(|p| p.id.as_str()).collect();
    assert!(
        provider_ids.contains(&"anthropic"),
        "Should have anthropic provider"
    );
    assert!(
        provider_ids.contains(&"openai"),
        "Should have openai provider"
    );
    assert!(
        provider_ids.contains(&"google"),
        "Should have google provider"
    );
}

#[test]
fn test_registry_lists_models() {
    let registry = create_test_registry();
    let models = registry.list_models();

    assert!(!models.is_empty(), "Registry should have models");

    let model_ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert!(
        model_ids.iter().any(|id| id.contains("claude")),
        "Should have Claude models"
    );
    assert!(
        model_ids.iter().any(|id| id.contains("gpt")),
        "Should have GPT models"
    );
}

#[test]
fn test_find_model_by_id() {
    let registry = create_test_registry();

    let result = registry.find_model("claude-3-5-sonnet-20241022");
    assert!(result.is_some(), "Should find claude-3-5-sonnet model");

    let (provider_id, model) = result.unwrap();
    assert_eq!(provider_id, "anthropic");
    assert!(model.supports_vision);
    assert!(model.supports_tools);
}

#[test]
fn test_provider_metadata() {
    let registry = create_test_registry();

    let anthropic = registry.get("anthropic");
    assert!(anthropic.is_some());
    let provider = anthropic.unwrap();

    assert_eq!(provider.id(), "anthropic");
    assert_eq!(provider.name(), "Anthropic");

    let models = provider.models();
    assert!(!models.is_empty());

    let claude = provider.get_model("claude-3-5-sonnet-20241022");
    assert!(claude.is_some());
}

#[test]
fn test_chat_request_builder() {
    let request = ChatRequest::new(
        "gpt-4o",
        vec![Message::system("You are helpful"), Message::user("Hello")],
    )
    .with_temperature(0.7)
    .with_max_tokens(1000)
    .with_stream(true);

    assert_eq!(request.model, "gpt-4o");
    assert_eq!(request.messages.len(), 2);
    assert_eq!(request.temperature, Some(0.7));
    assert_eq!(request.max_tokens, Some(1000));
    assert_eq!(request.stream, Some(true));
}

#[test]
fn test_model_info_clone() {
    let model = ModelInfo {
        id: "test-model".to_string(),
        name: "Test Model".to_string(),
        provider: "test".to_string(),
        context_window: 128000,
        max_output_tokens: 4096,
        supports_vision: true,
        supports_tools: true,
        cost_per_million_input: 1.0,
        cost_per_million_output: 2.0,
    };

    let cloned = model.clone();
    assert_eq!(cloned.id, model.id);
    assert_eq!(cloned.context_window, model.context_window);
}

#[test]
fn test_all_providers_have_models() {
    let registry = create_test_registry();

    for provider in registry.list() {
        let models = provider.models();
        assert!(
            !models.is_empty(),
            "Provider {} should have models",
            provider.id()
        );
    }
}
