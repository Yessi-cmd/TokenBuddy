use tokenbuddy_domain::NormalizedUsage;

/// A provider/model-specific price card, expressed in USD per million tokens.
///
/// This table is intentionally exact-match by provider and model family. A
/// model name alone is not enough: a relay can expose the same name while
/// charging a different amount, so unknown or third-party routes stay
/// unavailable instead of receiving an official vendor price.
#[derive(Debug, Clone, Copy)]
struct PriceRule {
    input: f64,
    cache_read: Option<f64>,
    cache_write: CacheWriteRule,
    output: f64,
}

#[derive(Debug, Clone, Copy)]
enum CacheWriteRule {
    /// The provider does not expose a separately billable cache-write term
    /// for the usage shape represented by this source.
    None,
    /// The estimate is only valid when the source reports cache-write tokens.
    Required(f64),
    /// The provider publishes a cache-write price, but this source may omit
    /// the write count. Add it when reported without turning an absent count
    /// into zero.
    IfReported(f64),
}

/// Calculate an API-equivalent estimate when every billable component required
/// by the matched price card is present.
pub(super) fn estimate_cost(
    provider_id: Option<&str>,
    provider_upstream_url: Option<&str>,
    model: Option<&str>,
    usage: &NormalizedUsage,
) -> Option<f64> {
    let rule = price_rule(provider_id?, provider_upstream_url, model?)?;
    let uncached_input = usage.input_tokens_uncached?;
    let output = usage.output_tokens_total?;

    let mut cost =
        per_million(uncached_input, rule.input).checked_add(per_million(output, rule.output))?;
    if let Some(price) = rule.cache_read {
        cost = cost.checked_add(per_million(usage.cache_read_tokens?, price))?;
    }
    match rule.cache_write {
        CacheWriteRule::None => {}
        CacheWriteRule::Required(price) => {
            cost = cost.checked_add(per_million(usage.cache_write_tokens?, price))?;
        }
        CacheWriteRule::IfReported(price) => {
            if let Some(tokens) = usage.cache_write_tokens {
                cost = cost.checked_add(per_million(tokens, price))?;
            }
        }
    }
    cost.is_finite().then_some(cost)
}

fn price_rule(
    provider_id: &str,
    provider_upstream_url: Option<&str>,
    model: &str,
) -> Option<PriceRule> {
    let provider = provider_id.trim().to_ascii_lowercase();
    let model = model.trim().to_ascii_lowercase();
    match provider.as_str() {
        // OpenAI's GPT-5-Codex model page
        // (https://developers.openai.com/api/docs/models/gpt-5-codex): $1.25
        // input, $0.125 cached input, and $10 output per million tokens.
        // OpenAI does not publish a separate cache-write price for this model,
        // so no unknown write term is invented here.
        "openai" if model == "gpt-5-codex" => Some(PriceRule {
            input: 1.25,
            cache_read: Some(0.125),
            cache_write: CacheWriteRule::None,
            output: 10.0,
        }),
        // OpenAI's current GPT-5.6 model pages:
        // (https://developers.openai.com/api/docs/models/gpt-5.6-sol),
        // (https://developers.openai.com/api/docs/models/gpt-5.6-terra), and
        // (https://developers.openai.com/api/docs/models/gpt-5.6-luna). The
        // pages list input/cached-input/output as Sol $5/$0.50/$30, Terra
        // $2/$0.20/$12, and Luna $0.20/$0.02/$1.20 per million. Cache writes
        // are 1.25x uncached input. Codex session usage does not consistently
        // expose a separate write count, so include it only when reported.
        "openai" if model == "gpt-5.6" || is_model_variant(&model, "gpt-5.6-sol") => {
            Some(PriceRule {
                input: 5.0,
                cache_read: Some(0.50),
                cache_write: CacheWriteRule::IfReported(6.25),
                output: 30.0,
            })
        }
        "openai" if is_model_variant(&model, "gpt-5.6-terra") => Some(PriceRule {
            input: 2.0,
            cache_read: Some(0.20),
            cache_write: CacheWriteRule::IfReported(2.50),
            output: 12.0,
        }),
        "openai" if is_model_variant(&model, "gpt-5.6-luna") => Some(PriceRule {
            input: 0.20,
            cache_read: Some(0.02),
            cache_write: CacheWriteRule::IfReported(0.25),
            output: 1.20,
        }),
        // Anthropic's current Claude Opus 5 and Fable 5 cards
        // (https://platform.claude.com/docs/en/about-claude/pricing): Opus 5
        // is $5 input, $6.25 five-minute cache write, $0.50 cache hit, and
        // $25 output; Fable 5 is $10, $12.50, $1, and $50 per million.
        "anthropic" if is_model_variant(&model, "claude-opus-5") => Some(PriceRule {
            input: 5.0,
            cache_read: Some(0.50),
            cache_write: CacheWriteRule::Required(6.25),
            output: 25.0,
        }),
        "anthropic" if is_model_variant(&model, "claude-fable-5") => Some(PriceRule {
            input: 10.0,
            cache_read: Some(1.0),
            cache_write: CacheWriteRule::Required(12.50),
            output: 50.0,
        }),
        // Anthropic's standard Claude 3.7 Sonnet card
        // (https://docs.anthropic.com/en/docs/about-claude/pricing): $3 input,
        // $0.30 cache reads, $3.75 five-minute cache writes, and $15 output per
        // million.
        "anthropic" if model.starts_with("claude-3-7-sonnet") => Some(PriceRule {
            input: 3.0,
            cache_read: Some(0.30),
            cache_write: CacheWriteRule::Required(3.75),
            output: 15.0,
        }),
        // OpenCode Go's official rate card
        // (https://opencode.ai/docs/go/) lists DeepSeek V4 Flash at $0.14
        // uncached input, $0.0028 cached read, and $0.28 output per million
        // tokens. Go does not publish a separately billable cache-write term.
        // CC Switch gives relays installation-local IDs, so bind this rule to
        // OpenCode's official Go endpoint rather than trusting a display name.
        _ if is_opencode_go(provider_upstream_url) && model == "deepseek-v4-flash" => {
            Some(PriceRule {
                input: 0.14,
                cache_read: Some(0.0028),
                cache_write: CacheWriteRule::None,
                output: 0.28,
            })
        }
        // DeepSeek's official API price card
        // (https://api-docs.deepseek.com/quick_start/pricing) lists V4 Flash
        // at $0.14 cache-miss input, $0.0028 cache-hit input, and $0.28 output
        // per million tokens. Context-cache storage has no separate write fee.
        // Bind the rule to the official API endpoint so a relay exposing the
        // same model name never inherits DeepSeek's price accidentally.
        _ if is_official_deepseek(provider_upstream_url) && model == "deepseek-v4-flash" => {
            Some(PriceRule {
                input: 0.14,
                cache_read: Some(0.0028),
                cache_write: CacheWriteRule::None,
                output: 0.28,
            })
        }
        _ => None,
    }
}

fn is_opencode_go(upstream_url: Option<&str>) -> bool {
    upstream_url.is_some_and(|url| {
        let url = url.trim().trim_end_matches('/').to_ascii_lowercase();
        url == "https://opencode.ai/zen/go" || url.starts_with("https://opencode.ai/zen/go/")
    })
}

fn is_official_deepseek(upstream_url: Option<&str>) -> bool {
    upstream_url.is_some_and(|url| {
        let url = url.trim().trim_end_matches('/').to_ascii_lowercase();
        url == "https://api.deepseek.com" || url.starts_with("https://api.deepseek.com/")
    })
}

fn is_model_variant(model: &str, family: &str) -> bool {
    model == family || model.starts_with(&format!("{family}-"))
}

fn per_million(tokens: u64, price: f64) -> f64 {
    (tokens as f64 / 1_000_000.0) * price
}

trait CheckedFloatAdd {
    fn checked_add(self, rhs: f64) -> Option<f64>;
}

impl CheckedFloatAdd for f64 {
    fn checked_add(self, rhs: f64) -> Option<f64> {
        let value = self + rhs;
        value.is_finite().then_some(value)
    }
}

#[cfg(test)]
mod tests {
    use tokenbuddy_domain::NormalizedUsage;

    use super::estimate_cost;

    #[test]
    fn prices_openai_input_cache_and_output_separately() {
        let usage = NormalizedUsage {
            input_tokens_uncached: Some(80),
            cache_read_tokens: Some(20),
            output_tokens_total: Some(40),
            ..Default::default()
        };

        let cost =
            estimate_cost(Some("openai"), None, Some("gpt-5-codex"), &usage).expect("known price");
        assert!((cost - 0.0005025).abs() < f64::EPSILON);
    }

    #[test]
    fn prices_each_gpt_5_6_tier_and_optional_cache_writes() {
        let usage = NormalizedUsage {
            input_tokens_uncached: Some(80),
            cache_read_tokens: Some(20),
            output_tokens_total: Some(40),
            ..Default::default()
        };
        let cases = [
            ("gpt-5.6", 0.00161),
            ("gpt-5.6-sol", 0.00161),
            ("gpt-5.6-terra", 0.000644),
            ("gpt-5.6-luna", 0.0000644),
        ];
        for (model, expected) in cases {
            let cost =
                estimate_cost(Some("openai"), None, Some(model), &usage).expect("known price");
            assert!((cost - expected).abs() < f64::EPSILON, "{model}: {cost}");
        }

        let with_cache_write = NormalizedUsage {
            cache_write_tokens: Some(10),
            ..usage
        };
        let cost = estimate_cost(Some("openai"), None, Some("gpt-5.6-sol"), &with_cache_write)
            .expect("known price");
        assert!((cost - 0.0016725).abs() < f64::EPSILON);
    }

    #[test]
    fn prices_anthropic_cache_writes_without_treating_missing_fields_as_zero() {
        let usage = NormalizedUsage {
            input_tokens_uncached: Some(100),
            cache_read_tokens: Some(30),
            cache_write_tokens: Some(20),
            output_tokens_total: Some(40),
            ..Default::default()
        };
        let cost = estimate_cost(Some("anthropic"), None, Some("claude-3-7-sonnet"), &usage)
            .expect("known price");
        assert!((cost - 0.000984).abs() < f64::EPSILON);

        let missing_cache_write = NormalizedUsage {
            cache_write_tokens: None,
            ..usage
        };
        assert_eq!(
            estimate_cost(
                Some("anthropic"),
                None,
                Some("claude-3-7-sonnet"),
                &missing_cache_write
            ),
            None
        );
    }

    #[test]
    fn prices_current_claude_opus_and_fable_cards() {
        let usage = NormalizedUsage {
            input_tokens_uncached: Some(100),
            cache_read_tokens: Some(30),
            cache_write_tokens: Some(20),
            output_tokens_total: Some(40),
            ..Default::default()
        };
        let opus = estimate_cost(Some("anthropic"), None, Some("claude-opus-5"), &usage)
            .expect("known Opus price");
        assert!((opus - 0.00164).abs() < f64::EPSILON);
        let fable = estimate_cost(Some("anthropic"), None, Some("claude-fable-5"), &usage)
            .expect("known Fable price");
        assert!((fable - 0.00328).abs() < f64::EPSILON);
    }

    #[test]
    fn never_prices_a_model_without_an_authoritative_provider_match() {
        let usage = NormalizedUsage {
            input_tokens_uncached: Some(100),
            cache_read_tokens: Some(20),
            output_tokens_total: Some(40),
            ..Default::default()
        };
        assert_eq!(
            estimate_cost(Some("cc-switch:relay"), None, Some("gpt-5-codex"), &usage),
            None
        );
        assert_eq!(estimate_cost(Some("openai"), None, None, &usage), None);
    }

    #[test]
    fn prices_deepseek_v4_flash_only_for_the_official_opencode_go_route() {
        let usage = NormalizedUsage {
            input_tokens_uncached: Some(225),
            cache_read_tokens: Some(53_888),
            cache_write_tokens: Some(0),
            output_tokens_total: Some(481),
            ..Default::default()
        };
        let cost = estimate_cost(
            Some("cc-switch:claude:local-id"),
            Some("https://opencode.ai/zen/go"),
            Some("deepseek-v4-flash"),
            &usage,
        )
        .expect("official OpenCode Go rate");
        assert!((cost - 0.0003170664).abs() < f64::EPSILON);
        assert_eq!(
            estimate_cost(
                Some("cc-switch:claude:other"),
                Some("https://relay.example/v1"),
                Some("deepseek-v4-flash"),
                &usage,
            ),
            None
        );
    }

    #[test]
    fn prices_deepseek_v4_flash_for_the_official_deepseek_api() {
        let usage = NormalizedUsage {
            input_tokens_uncached: Some(225),
            cache_read_tokens: Some(53_888),
            cache_write_tokens: Some(0),
            output_tokens_total: Some(481),
            ..Default::default()
        };
        for endpoint in [
            "https://api.deepseek.com",
            "https://api.deepseek.com/anthropic",
            "https://api.deepseek.com/v1",
        ] {
            let cost = estimate_cost(
                Some("cc-switch:claude:deepseek-official"),
                Some(endpoint),
                Some("deepseek-v4-flash"),
                &usage,
            )
            .expect("official DeepSeek rate");
            assert!((cost - 0.0003170664).abs() < f64::EPSILON);
        }
        assert_eq!(
            estimate_cost(Some("deepseek"), None, Some("deepseek-v4-flash"), &usage,),
            None,
            "a model-derived provider id is not proof of the official route"
        );
    }
}
