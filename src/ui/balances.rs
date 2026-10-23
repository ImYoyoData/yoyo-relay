use std::collections::BTreeMap;

use egui::RichText;
use egui_phosphor::regular as icons;

use crate::runtime::UiCommand;
use crate::snapshot::{BalanceView, NoticeLevel, QuotaWindowView};

use super::{theme, widgets, App};

fn quota_ratio(used: f64, limit: f64) -> f32 {
    if limit <= 0.0 {
        0.0
    } else {
        (used / limit).clamp(0.0, 1.0) as f32
    }
}

fn quota_color(ratio: f32) -> egui::Color32 {
    if ratio >= 0.9 {
        theme::RED
    } else if ratio >= 0.7 {
        theme::ORANGE
    } else {
        theme::GREEN
    }
}

fn quota_bar(
    ui: &mut egui::Ui,
    label: &str,
    used: f64,
    limit: f64,
    unit: &str,
    currency: &str,
    reset_at: i64,
) {
    let t = theme::tokens(ui);
    let ratio = quota_ratio(used, limit);
    let color = quota_color(ratio);
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(10.5).strong().color(t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format!("剩余 {:.0}%", (1.0 - ratio) * 100.0))
                    .size(9.5)
                    .color(color),
            );
        });
    });
    ui.add(
        egui::ProgressBar::new(ratio)
            .desired_height(9.0)
            .fill(color)
            .corner_radius(5)
            .text(format!("{:.0}%", ratio * 100.0)),
    );
    ui.horizontal(|ui| {
        let value = if unit == "percent" {
            format!("{used:.0}% / {limit:.0}%")
        } else {
            format!(
                "{} / {}",
                widgets::format_money(used, currency),
                widgets::format_money(limit, currency)
            )
        };
        ui.label(RichText::new(value).monospace().size(9.0).color(t.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let reset = if reset_at > 0 {
                format!("重置 {}", widgets::format_time(reset_at))
            } else {
                "不重置".to_string()
            };
            ui.label(RichText::new(reset).size(8.5).color(t.muted));
        });
    });
    ui.add_space(5.0);
}

fn quota_aggregate(ui: &mut egui::Ui, windows: &[&QuotaWindowView]) {
    let t = theme::tokens(ui);
    // Group by label + unit + currency. Percent windows average; money
    // windows sum within the same currency.
    let mut groups: Vec<(String, String, String, Vec<&QuotaWindowView>)> = Vec::new();
    for window in windows {
        let currency = if window.unit == "percent" {
            String::new()
        } else {
            window.currency.clone()
        };
        let key = (window.label.clone(), window.unit.clone(), currency);
        if let Some(group) = groups
            .iter_mut()
            .find(|(label, unit, cur, _)| label == &key.0 && unit == &key.1 && cur == &key.2)
        {
            group.3.push(*window);
        } else {
            groups.push((key.0, key.1, key.2, vec![*window]));
        }
    }
    super::kit::section_card(
        ui,
        icons::CHART_BAR,
        "额度聚合",
        "按账号汇总 · 本机请求花费估算",
        |ui| {
            super::kit::badge(ui, &format!("{} 个窗口", windows.len()), theme::BLUE_INFO);
        },
        |ui| {
            for (label, unit, currency, group) in groups {
                let (used, limit, reset_at) = if unit == "percent" {
                    let used = group.iter().map(|w| w.used).sum::<f64>() / group.len() as f64;
                    let limit = group.iter().map(|w| w.limit).sum::<f64>() / group.len() as f64;
                    (used, limit, group.first().map(|w| w.reset_at).unwrap_or(0))
                } else {
                    (
                        group.iter().map(|w| w.used).sum(),
                        group.iter().map(|w| w.limit).sum(),
                        group.first().map(|w| w.reset_at).unwrap_or(0),
                    )
                };
                quota_bar(ui, &label, used, limit, &unit, &currency, reset_at);
            }
            ui.label(
                RichText::new(
                    "订阅额度按本机记录的上游请求花费估算；上游未提供额度 API 时无法精确到官方剩余量。",
                )
                .size(9.5)
                .color(t.muted),
            );
        },
    );
}

impl App {
    pub(crate) fn page_balances(&mut self, ui: &mut egui::Ui) {
        let balances: Vec<BalanceView> = self
            .snapshot
            .balances
            .iter()
            .filter(|b| self.config.account(&b.account_id).is_some())
            .cloned()
            .collect();
        let known: Vec<&BalanceView> = balances.iter().filter(|b| b.amount.is_some()).collect();
        let mut totals: BTreeMap<String, f64> = BTreeMap::new();
        for balance in &known {
            if let Some(amount) = balance.amount {
                *totals
                    .entry(crate::config::normalize_currency(&balance.currency))
                    .or_insert(0.0) += amount;
            }
        }
        let total_text = if totals.is_empty() {
            "—".to_string()
        } else {
            totals
                .iter()
                .map(|(currency, amount)| widgets::format_money(*amount, currency))
                .collect::<Vec<_>>()
                .join(" + ")
        };
        let display_currency = self.config.ui.active_currency();
        let converted_total: f64 = totals
            .iter()
            .map(|(currency, amount)| {
                self.config
                    .ui
                    .fx_convert(*amount, currency, &display_currency)
            })
            .sum();
        let cny_sub = if totals.is_empty() {
            "暂无可用余额数据".to_string()
        } else {
            let rate = self.config.ui.fx_rate(&display_currency);
            format!(
                "折合 {} · 1 USD = {rate:.4} {display_currency}",
                widgets::format_money(converted_total, &display_currency)
            )
        };
        let low = known
            .iter()
            .filter(|b| b.amount.unwrap_or(0.0) < self.config.ui.threshold_for(&b.currency))
            .count();
        let threshold_text = if totals.is_empty() {
            "按币种分别设置".to_string()
        } else {
            totals
                .keys()
                .map(|currency| {
                    format!(
                        "{} {}",
                        currency,
                        widgets::format_money(self.config.ui.threshold_for(currency), currency)
                    )
                })
                .collect::<Vec<_>>()
                .join(" · ")
        };
        let auto_count = balances.iter().filter(|b| b.mode.contains("自动")).count();
        let manual_count = balances.iter().filter(|b| b.mode.contains("手动")).count();

        super::kit::page_header(
            ui,
            super::Page::Balances.eyebrow(),
            "余额",
            "自动查询、手动估算与低余额提醒",
            |ui| {
                if super::kit::primary_labeled(ui, icons::ARROWS_CLOCKWISE, "全部刷新").clicked()
                {
                    let _ = self.cmd_tx.send(UiCommand::RefreshBalance(None));
                }
            },
        );

        let cards: Vec<(&str, &str, String, String, egui::Color32)> = vec![
            (
                icons::WALLET,
                "已知余额合计",
                total_text,
                cny_sub,
                theme::ACCENT,
            ),
            (
                icons::WARNING_CIRCLE,
                "低余额账号",
                format!("{low} 个"),
                format!("阈值 {threshold_text}"),
                if low > 0 { theme::ORANGE } else { theme::GREEN },
            ),
            (
                icons::ARROWS_CLOCKWISE,
                "自动查询",
                format!("{auto_count} 个"),
                format!("每 {} 分钟刷新", self.config.ui.balance_refresh_mins.max(1)),
                theme::BLUE_INFO,
            ),
            (
                icons::PENCIL_SIMPLE,
                "手动估算",
                format!("{manual_count} 个"),
                "按路由价格递减".to_string(),
                theme::MUTED,
            ),
        ];
        super::kit::kpi_grid(ui, &cards);
        let quota_windows: Vec<&QuotaWindowView> = balances
            .iter()
            .flat_map(|balance| balance.quota_windows.iter())
            .collect();
        if !quota_windows.is_empty() {
            ui.add_space(10.0);
            quota_aggregate(ui, &quota_windows);
        }
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} 个账号", balances.len()))
                    .size(11.0)
                    .color(theme::muted(ui)),
            );
            if self.snapshot.balance_refreshing {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(RichText::new("刷新中…").size(10.5).color(theme::muted(ui)));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!(
                        "每 {} 分钟自动刷新",
                        self.config.ui.balance_refresh_mins
                    ))
                    .size(10.0)
                    .color(theme::muted(ui)),
                );
            });
        });
        ui.add_space(8.0);

        if balances.is_empty() {
            if super::kit::empty_state(
                ui,
                icons::WALLET,
                "还没有余额数据",
                "请先在「账号」页配置余额查询地址或手动余额。",
                icons::USERS,
                "前往账号",
            ) {
                self.page = super::Page::Accounts;
            }
            return;
        }

        let mut action: Option<BalanceAction> = None;
        for balance in &balances {
            self.balance_card(ui, balance, &mut action);
            ui.add_space(8.0);
        }
        if let Some(action) = action {
            match action {
                BalanceAction::Refresh(id) => {
                    let _ = self.cmd_tx.send(UiCommand::RefreshBalance(Some(id)));
                    self.toast(NoticeLevel::Info, "已发起余额刷新");
                }
                BalanceAction::Manual(id) => self.open_manual_balance(&id),
                BalanceAction::Setup(id) => self.open_balance_setup(&id),
            }
        }
        ui.add_space(6.0);
        super::kit::tooltip_text(
            ui,
            icons::INFO,
            "自动查询需要一个返回 JSON 的余额端点；手动模式从录入时间点起按请求花费递减，并标注「估算」。",
        );
    }

    fn balance_card(
        &self,
        ui: &mut egui::Ui,
        balance: &BalanceView,
        action: &mut Option<BalanceAction>,
    ) {
        let account = self.config.account(&balance.account_id);
        let low_amount = balance
            .amount
            .map(|a| a < self.config.ui.threshold_for(&balance.currency))
            .unwrap_or(false);
        let is_auto = account
            .map(|a| a.balance.mode == crate::config::BalanceMode::Auto)
            .unwrap_or(false);
        let has_url = account
            .and_then(|a| a.balance.url.clone())
            .map(|u| !u.trim().is_empty())
            .unwrap_or(false);

        super::kit::surface(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                super::kit::icon_tile(
                    ui,
                    icons::WALLET,
                    if low_amount {
                        theme::RED
                    } else {
                        theme::ACCENT
                    },
                    36.0,
                );
                ui.add_space(3.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&balance.account_name).size(14.5).strong());
                        super::kit::badge(ui, &balance.mode, theme::MUTED);
                        if !balance.quota_windows.is_empty() {
                            let preset = crate::config::quota_preset_label(&balance.quota_preset);
                            super::kit::badge(
                                ui,
                                if preset == "无" { "额度制" } else { preset },
                                theme::BLUE_INFO,
                            );
                        }
                        if balance.estimated {
                            super::kit::badge(ui, "估算", theme::ORANGE);
                        }
                    });
                    ui.label(
                        RichText::new(&balance.provider_name)
                            .size(10.0)
                            .color(theme::muted(ui)),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(amount) = balance.amount {
                        ui.label(
                            RichText::new(widgets::format_money(amount, &balance.currency))
                                .size(20.0)
                                .strong()
                                .color(if low_amount { theme::RED } else { theme::GREEN }),
                        );
                    } else if !balance.quota_windows.is_empty() {
                        ui.label(RichText::new("额度制").size(12.0).color(theme::BLUE_INFO));
                    } else {
                        ui.label(RichText::new("—").color(theme::muted(ui)));
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&balance.status_text).size(10.0).color(
                    if balance.amount.is_some() {
                        theme::GREEN
                    } else {
                        theme::ORANGE
                    },
                ));
                if balance.updated_at > 0 {
                    ui.label(
                        RichText::new(format!(
                            "更新于 {}",
                            widgets::time_ago(balance.updated_at, crate::config::now_ts())
                        ))
                        .size(10.0)
                        .color(theme::muted(ui)),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if super::kit::icon_button(
                        ui,
                        icons::PENCIL_SIMPLE,
                        "余额设置：查询方式、JSON 路径、公式、百分比换算",
                    )
                    .clicked()
                    {
                        *action = Some(BalanceAction::Setup(balance.account_id.clone()));
                    }
                    if super::kit::icon_button(ui, icons::CURRENCY_CIRCLE_DOLLAR, "手动录入余额")
                        .clicked()
                    {
                        *action = Some(BalanceAction::Manual(balance.account_id.clone()));
                    }
                    if super::kit::labeled_button(
                        ui,
                        icons::ARROWS_CLOCKWISE,
                        "刷新",
                        armas::ButtonVariant::Outline,
                        armas::ButtonSize::Small,
                    )
                    .clicked()
                        && is_auto
                        && has_url
                        && !self.snapshot.balance_refreshing
                    {
                        *action = Some(BalanceAction::Refresh(balance.account_id.clone()));
                    }
                });
            });
            if !balance.quota_windows.is_empty() {
                ui.add_space(8.0);
                for window in &balance.quota_windows {
                    quota_bar(
                        ui,
                        &window.label,
                        window.used,
                        window.limit,
                        &window.unit,
                        &window.currency,
                        window.reset_at,
                    );
                }
                if !balance.quota_note.is_empty() {
                    ui.label(
                        RichText::new(&balance.quota_note)
                            .size(9.0)
                            .color(theme::muted(ui)),
                    );
                }
            }
        });
    }
}

enum BalanceAction {
    Refresh(String),
    Manual(String),
    Setup(String),
}
