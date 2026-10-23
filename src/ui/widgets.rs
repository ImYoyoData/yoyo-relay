use egui::{Color32, RichText, Sense, Ui, Vec2};

use crate::config::fmt_duration_secs;
use crate::probe::TestLevel;
use crate::scheduler::AccountState as SchedState;
use crate::snapshot::UiSnapshot;

use super::theme;

pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

pub fn state_color(state: &SchedState) -> Color32 {
    match state {
        SchedState::Healthy => theme::GREEN,
        SchedState::Cooling { .. } => theme::ORANGE,
        SchedState::HalfOpen => theme::BLUE_INFO,
        SchedState::Exhausted { .. } => theme::RED,
        SchedState::Disabled => theme::MUTED,
    }
}

pub fn state_text(state: &SchedState, now: i64) -> String {
    match state {
        SchedState::Healthy => "健康".into(),
        SchedState::Cooling {
            until,
            reason,
            fail_streak,
        } => format!(
            "冷却中 · {reason} · {} · 连续{}次",
            fmt_duration_secs(until - now),
            fail_streak
        ),
        SchedState::HalfOpen => "探测中".into(),
        SchedState::Exhausted { reset_at } => {
            format!("额度用尽 · {} 后重置", fmt_duration_secs(reset_at - now))
        }
        SchedState::Disabled => "已禁用".into(),
    }
}

pub fn state_short(state: &SchedState, now: i64) -> String {
    match state {
        SchedState::Healthy => "健康".into(),
        SchedState::Cooling { until, .. } => format!("冷却 {}", fmt_duration_secs(until - now)),
        SchedState::HalfOpen => "探测中".into(),
        SchedState::Exhausted { reset_at } => {
            format!("额度用尽 {}", fmt_duration_secs(reset_at - now))
        }
        SchedState::Disabled => "已禁用".into(),
    }
}

pub fn level_color(level: TestLevel) -> Color32 {
    match level {
        TestLevel::Success => theme::GREEN,
        TestLevel::Warning | TestLevel::RateLimited => theme::ORANGE,
        TestLevel::AuthFailed | TestLevel::Error | TestLevel::Unreachable => theme::RED,
    }
}

pub fn test_badge(ui: &mut Ui, snapshot: &UiSnapshot, key: &str) {
    if snapshot.running_tests.contains(key) {
        ui.spinner();
        ui.label(RichText::new("测试中").size(10.0).color(theme::muted(ui)));
    } else if let Some(result) = snapshot.tests.get(key) {
        super::kit::badge(ui, result.level.label(), level_color(result.level));
    }
}

pub fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    options: &[(T, &str)],
    selected: T,
) -> Option<T> {
    let t = theme::tokens(ui);
    let mut changed = None;
    ui.horizontal(|ui| {
        for (value, label) in options {
            let active = *value == selected;
            let text = if active {
                RichText::new(*label).color(Color32::WHITE).size(11.0)
            } else {
                RichText::new(*label).size(11.0).color(t.text)
            };
            let button = egui::Button::new(text)
                .fill(if active { theme::ACCENT } else { t.card })
                .stroke(egui::Stroke::new(
                    1.0_f32,
                    if active { theme::ACCENT } else { t.card_border },
                ))
                .corner_radius(7);
            if ui.add(button).clicked() && !active {
                changed = Some(*value);
            }
        }
    });
    changed
}

pub fn quota_bar(
    ui: &mut Ui,
    used_tokens: u64,
    limit_tokens: Option<u64>,
    used_requests: u64,
    limit_requests: Option<u64>,
    reset_at: i64,
    now: i64,
) {
    let t = theme::tokens(ui);
    let (used, limit) = if let Some(limit) = limit_tokens {
        (used_tokens, limit)
    } else if let Some(limit) = limit_requests {
        (used_requests, limit)
    } else {
        (0, 1)
    };
    let frac = if limit == 0 {
        0.0
    } else {
        (used as f32 / limit as f32).clamp(0.0, 1.0)
    };
    let remaining = 1.0 - frac;
    let color = if remaining > 0.5 {
        t.green
    } else if remaining > 0.2 {
        t.amber
    } else {
        t.red
    };
    let width = 110.0_f32.min(ui.available_width().max(60.0));
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 6.0), Sense::hover());
    ui.painter().rect_filled(rect, 3.0, t.hover);
    let fill = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * frac, rect.height()));
    ui.painter().rect_filled(fill, 3.0, color);
    if let Some(limit) = limit_tokens {
        ui.label(
            RichText::new(format!(
                "{} / {}",
                format_count(used_tokens),
                format_count(limit)
            ))
            .size(9.5)
            .color(t.muted),
        );
    } else if let Some(limit) = limit_requests {
        ui.label(
            RichText::new(format!("{} / {} 次", used_requests, limit))
                .size(9.5)
                .color(t.muted),
        );
    }
    if reset_at > now {
        ui.label(
            RichText::new(format!("重置 {}", fmt_duration_secs(reset_at - now)))
                .size(9.0)
                .color(t.muted),
        );
    }
}

pub fn format_count(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.2}B", value as f64 / 1_000_000_000.0)
    } else if value >= 1_000_000 {
        format!("{:.2}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

pub fn format_cost(cost_micros: u64) -> String {
    let usd = cost_micros as f64 / 1_000_000.0;
    if usd >= 100.0 {
        format!("${usd:.1}")
    } else if usd >= 1.0 {
        format!("${usd:.2}")
    } else {
        format!("${usd:.4}")
    }
}

pub fn format_money(amount: f64, currency: &str) -> String {
    match crate::config::normalize_currency(currency).as_str() {
        "CNY" => format!("¥{amount:.2}"),
        "USD" => format!("${amount:.2}"),
        other => format!("{amount:.2} {other}"),
    }
}

/// Join per-currency totals without adding unlike currencies together.
pub fn format_currency_totals(totals: &[crate::snapshot::CurrencyTotal]) -> String {
    if totals.is_empty() {
        return "—".to_string();
    }
    totals
        .iter()
        .map(|total| format_money(total.amount, &total.currency))
        .collect::<Vec<_>>()
        .join(" + ")
}

pub fn format_time(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "-".into())
}

/// Absolute call time: `14:22:31` today, `23日 14:22:31` this month,
/// `10月23日 14:22:31` this year, full date otherwise.
pub fn format_call_time(ts: i64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    let Some(datetime) = Local.timestamp_opt(ts, 0).single() else {
        return "—".to_string();
    };
    let now = Local::now();
    if datetime.date_naive() == now.date_naive() {
        datetime.format("%H:%M:%S").to_string()
    } else if datetime.year() == now.year() && datetime.month() == now.month() {
        datetime.format("%d日 %H:%M:%S").to_string()
    } else if datetime.year() == now.year() {
        datetime.format("%m月%d日 %H:%M:%S").to_string()
    } else {
        datetime.format("%Y-%m-%d %H:%M:%S").to_string()
    }
}

pub fn time_ago(ts: i64, now: i64) -> String {
    let diff = now - ts;
    if diff < 5 {
        "刚刚".into()
    } else if diff < 60 {
        format!("{diff} 秒前")
    } else if diff < 3600 {
        format!("{} 分钟前", diff / 60)
    } else if diff < 86_400 {
        format!("{} 小时前", diff / 3600)
    } else {
        format!("{} 天前", diff / 86_400)
    }
}

pub fn format_day(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%m-%d").to_string())
        .unwrap_or_else(|| "—".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn formats_absolute_call_times() {
        let today = format_call_time(crate::config::now_ts());
        assert_eq!(today.len(), 8, "today should be HH:MM:SS: {today}");
        let old = chrono::Local
            .with_ymd_and_hms(2020, 1, 2, 3, 4, 5)
            .single()
            .unwrap()
            .timestamp();
        assert_eq!(format_call_time(old), "2020-01-02 03:04:05");
    }
}
