use crate::config::types::*;

pub struct ProviderPreset {
    pub key: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

pub const PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        key: "openai",
        name: "OpenAI",
        description: "官方直连 · openai-chat",
    },
    ProviderPreset {
        key: "anthropic",
        name: "Anthropic",
        description: "官方直连 · anthropic-messages",
    },
    ProviderPreset {
        key: "command_code",
        name: "Command Code",
        description: "聚合平台 · 按模型前缀解析协议",
    },
    ProviderPreset {
        key: "opencode_zen",
        name: "OpenCode Zen",
        description: "聚合平台 · 按模型前缀解析协议",
    },
    ProviderPreset {
        key: "custom",
        name: "自定义端点",
        description: "任意 OpenAI / Anthropic 兼容服务",
    },
];

pub fn provider_from_preset(key: &str) -> Provider {
    let mut p = Provider {
        id: new_id(),
        created_at: now_ts(),
        preset: Some(key.to_string()),
        ..Provider::default()
    };
    match key {
        "openai" => {
            p.name = "OpenAI".into();
            p.kind = ProviderKind::Official;
            p.base_url = "https://api.openai.com/v1".into();
            p.auth = AuthConfig::default();
            p.static_protocol = Protocol::OpenAiChat;
            p.test_model = Some("gpt-4o-mini".into());
        }
        "anthropic" => {
            p.name = "Anthropic".into();
            p.kind = ProviderKind::Official;
            p.base_url = "https://api.anthropic.com/v1".into();
            p.auth = AuthConfig {
                mode: AuthMode::XApiKey,
                header_name: String::new(),
                scheme: String::new(),
            };
            p.static_protocol = Protocol::AnthropicMessages;
            p.test_model = Some("claude-3-5-haiku-20241022".into());
        }
        "command_code" => {
            p.name = "Command Code".into();
            p.kind = ProviderKind::Aggregator;
            p.base_url = "https://api.commandcode.ai/provider/v1".into();
            p.auth = AuthConfig::default();
            p.static_protocol = Protocol::OpenAiChat;
            p.protocol_strategy = ProtocolStrategy {
                mode: ProtocolStrategyMode::ModelPrefix,
                prefix_rules: aggregator_prefix_rules(),
            };
            p.test_model = Some("claude-3-5-haiku-20241022".into());
        }
        "opencode_zen" => {
            p.name = "OpenCode Zen".into();
            p.kind = ProviderKind::Aggregator;
            p.base_url = "https://opencode.ai/zen/v1".into();
            p.auth = AuthConfig::default();
            p.static_protocol = Protocol::OpenAiChat;
            p.protocol_strategy = ProtocolStrategy {
                mode: ProtocolStrategyMode::ModelPrefix,
                prefix_rules: aggregator_prefix_rules(),
            };
            p.test_model = Some("gpt-4o-mini".into());
        }
        _ => {
            p.name = "自定义端点".into();
            p.kind = ProviderKind::Custom;
            p.base_url = "http://127.0.0.1:11434/v1".into();
            p.auth = AuthConfig::default();
            p.static_protocol = Protocol::OpenAiChat;
            p.test_model = Some("gpt-4o-mini".into());
            p.preset = Some("custom".into());
        }
    }
    p
}

fn aggregator_prefix_rules() -> Vec<PrefixRule> {
    vec![
        PrefixRule {
            prefix: "claude".into(),
            protocol: Protocol::AnthropicMessages,
        },
        PrefixRule {
            prefix: "gpt".into(),
            protocol: Protocol::OpenAiChat,
        },
        PrefixRule {
            prefix: "o1".into(),
            protocol: Protocol::OpenAiChat,
        },
        PrefixRule {
            prefix: "o3".into(),
            protocol: Protocol::OpenAiChat,
        },
        PrefixRule {
            prefix: "o4".into(),
            protocol: Protocol::OpenAiChat,
        },
        PrefixRule {
            prefix: "gemini".into(),
            protocol: Protocol::OpenAiChat,
        },
        PrefixRule {
            prefix: "text-embedding".into(),
            protocol: Protocol::OpenAiChat,
        },
    ]
}
