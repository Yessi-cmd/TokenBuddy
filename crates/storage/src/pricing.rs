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
    cache_write: Option<f64>,
    output: f64,
}

/// Calculate an API-equivalent estimate when every billable component required
/// by the matched price card is present.
pub(super) fn estimate_cost(
    provider_id: Option<&str>,
    model: Option<&str>,
    usage: &NormalizedUsage,
) -> Option<f64> {
    let rule = price_rule(provider_id?, model?)?;
    let uncached_input = usage.input_tokens_uncached?;
    let output = usage.output_tokens_total?;

    let mut cost =
        per_million(uncached_input, rule.input).checked_add(per_million(output, rule.output))?;
    if let Some(price) = rule.cache_read {
        cost = cost.checked_add(per_million(usage.cache_read_tokens?, price))?;
    }
    if let Some(price) = rule.cache_write {
        cost = cost.checked_add(per_million(usage.cache_write_tokens?, price))?;
    }
    cost.is_finite().then_some(cost)
}

fn price_rule(provider_id: &str, model: &str) -> Option<PriceRule> {
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
            cache_write: None,
            output: 10.0,
        }),
        // Anthropic's standard Claude 3.7 Sonnet card
        // (https://docs.anthropic.com/en/docs/about-claude/pricing): $3 input,
        // $0.30 cache reads, $3.75 five-minute cache writes, and $15 output per
        // million.
        "anthropic" if model.starts_with("claude-3-7-sonnet") => Some(PriceRule {
            input: 3.0,
            cache_read: Some(0.30),
            cache_write: Some(3.75),
            output: 15.0,
        }),
        _ => None,
    }
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

        let cost = estimate_cost(Some("openai"), Some("gpt-5-codex"), &usage).expect("known price");
        assert!((cost - 0.0005025).abs() < f64::EPSILON);
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
        let cost = estimate_cost(Some("anthropic"), Some("claude-3-7-sonnet"), &usage)
            .expect("known price");
        assert!((cost - 0.000984).abs() < f64::EPSILON);

        let missing_cache_write = NormalizedUsage {
            cache_write_tokens: None,
            ..usage
        };
        assert_eq!(
            estimate_cost(
                Some("anthropic"),
                Some("claude-3-7-sonnet"),
                &missing_cache_write
            ),
            None
        );
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
            estimate_cost(Some("cc-switch:relay"), Some("gpt-5-codex"), &usage),
            None
        );
        assert_eq!(estimate_cost(Some("openai"), None, &usage), None);
    }
}
