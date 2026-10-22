use crate::config::types::*;

pub struct ProviderPreset {
    pub key: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Grouping used by the preset gallery in the provider editor.
    pub group: &'static str,
}

pub const PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        key: "openai",
        name: "OpenAI",
        description: "官方直连 · openai-chat",
        group: "国际官方",
    },
    ProviderPreset {
        key: "anthropic",
        name: "Anthropic",
        description: "官方直连 · anthropic-messages",
        group: "国际官方",
    },
    ProviderPreset {
        key: "zhipu",
        name: "智谱 AI",
        description: "GLM 系列 · OpenAI 兼容",
        group: "国内官方",
    },
    ProviderPreset {
        key: "xiaomi",
        name: "小米 MiMo",
        description: "MiMo 系列 · OpenAI 兼容",
        group: "国内官方",
    },
    ProviderPreset {
        key: "minimax",
        name: "MiniMax",
        description: "MiniMax 系列 · OpenAI 兼容",
        group: "国内官方",
    },
    ProviderPreset {
        key: "doubao",
        name: "豆包 · 火山方舟",
        description: "豆包模型 · Ark v3 兼容端点",
        group: "国内官方",
    },
    ProviderPreset {
        key: "qwen",
        name: "通义千问",
        description: "Qwen 系列 · DashScope 兼容模式",
        group: "国内官方",
    },
    ProviderPreset {
        key: "hunyuan",
        name: "腾讯混元",
        description: "Hunyuan 系列 · OpenAI 兼容",
        group: "国内官方",
    },
    ProviderPreset {
        key: "stepfun",
        name: "阶跃星辰",
        description: "Step 系列 · OpenAI 兼容",
        group: "国内官方",
    },
    ProviderPreset {
        key: "command_code",
        name: "Command Code",
        description: "聚合平台 · 按模型前缀解析协议",
        group: "聚合平台",
    },
    ProviderPreset {
        key: "opencode_zen",
        name: "OpenCode Zen",
        description: "聚合平台 · 按模型前缀解析协议",
        group: "聚合平台",
    },
    ProviderPreset {
        key: "opencode_go",
        name: "OpenCode Go",
        description: "订阅套餐 · 与 Zen 共用兼容网关（可改）",
        group: "聚合平台",
    },
    ProviderPreset {
        key: "custom",
        name: "自定义端点",
        description: "任意 OpenAI / Anthropic 兼容服务",
        group: "自定义",
    },
];

pub fn preset(key: &str) -> Option<&'static ProviderPreset> {
    PRESETS.iter().find(|preset| preset.key == key)
}

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
        "zhipu" => {
            p.name = "智谱 AI".into();
            p.kind = ProviderKind::Official;
            p.base_url = "https://open.bigmodel.cn/api/paas/v4".into();
            p.auth = AuthConfig::default();
            p.static_protocol = Protocol::OpenAiChat;
            p.test_model = Some("glm-4-flash".into());
        }
        "xiaomi" => {
            p.name = "小米 MiMo".into();
            p.kind = ProviderKind::Official;
            p.base_url = "https://api.xiaomimimo.com/v1".into();
            p.auth = AuthConfig::default();
            p.static_protocol = Protocol::OpenAiChat;
            p.test_model = Some("mimo-v2-flash".into());
        }
        "minimax" => {
            p.name = "MiniMax".into();
            p.kind = ProviderKind::Official;
            p.base_url = "https://api.minimax.chat/v1".into();
            p.auth = AuthConfig::default();
            p.static_protocol = Protocol::OpenAiChat;
            p.test_model = Some("MiniMax-M2".into());
        }
        "doubao" => {
            p.name = "豆包 · 火山方舟".into();
            p.kind = ProviderKind::Official;
            p.base_url = "https://ark.cn-beijing.volces.com/api/v3".into();
            p.auth = AuthConfig::default();
            p.static_protocol = Protocol::OpenAiChat;
            p.test_model = Some("doubao-seed-1-6-250615".into());
        }
        "qwen" => {
            p.name = "通义千问".into();
            p.kind = ProviderKind::Official;
            p.base_url = "https://dashscope.aliyuncs.com/compatible-mode/v1".into();
            p.auth = AuthConfig::default();
            p.static_protocol = Protocol::OpenAiChat;
            p.test_model = Some("qwen-plus".into());
        }
        "hunyuan" => {
            p.name = "腾讯混元".into();
            p.kind = ProviderKind::Official;
            p.base_url = "https://api.hunyuan.cloud.tencent.com/v1".into();
            p.auth = AuthConfig::default();
            p.static_protocol = Protocol::OpenAiChat;
            p.test_model = Some("hunyuan-turbos-latest".into());
        }
        "stepfun" => {
            p.name = "阶跃星辰".into();
            p.kind = ProviderKind::Official;
            p.base_url = "https://api.stepfun.com/v1".into();
            p.auth = AuthConfig::default();
            p.static_protocol = Protocol::OpenAiChat;
            p.test_model = Some("step-2-16k".into());
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
        "opencode_go" => {
            p.name = "OpenCode Go".into();
            p.kind = ProviderKind::Aggregator;
            // OpenCode Go subscriptions are served through the same compatible
            // gateway as Zen; users can override the URL in the editor.
            p.base_url = "https://opencode.ai/zen/v1".into();
            p.auth = AuthConfig::default();
            p.static_protocol = Protocol::OpenAiChat;
            p.protocol_strategy = ProtocolStrategy {
                mode: ProtocolStrategyMode::ModelPrefix,
                prefix_rules: aggregator_prefix_rules(),
            };
            p.test_model = Some("claude-sonnet-4-5".into());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_unique_and_buildable() {
        let mut keys = std::collections::HashSet::new();
        for preset in PRESETS {
            assert!(keys.insert(preset.key), "duplicate preset {}", preset.key);
            let provider = provider_from_preset(preset.key);
            assert!(
                provider.base_url.starts_with("http"),
                "preset {} has invalid url",
                preset.key
            );
            assert!(!provider.name.is_empty());
        }
    }
}
