use std::collections::BTreeMap;

use armas::components::{Card, CardVariant};
use egui::RichText;
use egui_phosphor::regular as icons;

use crate::runtime::UiCommand;
use crate::snapshot::{BalanceView, NoticeLevel};

use super::{theme, widgets, App};

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
                if known.is_empty() {
                    "暂无可用余额数据".to_string()
                } else {
                    "按币种分别统计，不做汇率相加".to_string()
                },
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
                "每 {} 分钟刷新".to_string(),
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

        Card::new()
            .variant(CardVariant::Outlined)
            .inner_margin(16.0)
            .show(ui, |ui| {
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
                        if super::kit::icon_button(ui, icons::PENCIL_SIMPLE, "手动录入/修改余额")
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
            });
    }
}

enum BalanceAction {
    Refresh(String),
    Manual(String),
}
