//! Small Markdown renderer for the in-app documentation.
//!
//! The API guide only needs a focused subset (headings, paragraphs, lists,
//! fenced code, tables, quotes and inline emphasis), so this stays dependency
//! free and renders with the app's own theme instead of a generic HTML look.

use std::sync::OnceLock;

use egui::text::{LayoutJob, TextFormat};
use egui::{FontId, RichText, Stroke};

use super::theme;

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph(String),
    Code {
        language: String,
        code: String,
    },
    List {
        ordered: bool,
        items: Vec<String>,
    },
    Quote(String),
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Rule,
}

#[derive(Clone, Debug, PartialEq)]
enum Inline {
    Text(String),
    Bold(String),
    Italic(String),
    Code(String),
    Link { text: String, url: String },
}

/// Parse the documentation subset. Kept public for unit tests and future
/// markdown surfaces (release notes, plugin descriptions).
pub fn parse(markdown: &str) -> Vec<Block> {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            index += 1;
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("```") {
            let language = rest.trim().to_string();
            let mut code = String::new();
            index += 1;
            while index < lines.len() && !lines[index].trim_start().starts_with("```") {
                code.push_str(lines[index]);
                code.push('\n');
                index += 1;
            }
            if index < lines.len() {
                index += 1;
            }
            blocks.push(Block::Code { language, code });
            continue;
        }

        if let Some((level, text)) = heading(trimmed) {
            blocks.push(Block::Heading { level, text });
            index += 1;
            continue;
        }

        if is_rule(trimmed) {
            blocks.push(Block::Rule);
            index += 1;
            continue;
        }

        if trimmed.starts_with('>') {
            let mut quote = Vec::new();
            while index < lines.len() {
                let current = lines[index].trim_start();
                let Some(rest) = current.strip_prefix('>') else {
                    break;
                };
                quote.push(rest.trim_start().to_string());
                index += 1;
            }
            blocks.push(Block::Quote(quote.join("\n")));
            continue;
        }

        if let Some(ordered) = list_marker(trimmed) {
            let mut items = Vec::new();
            while index < lines.len() {
                let current = lines[index].trim_start();
                let Some(current_ordered) = list_marker(current) else {
                    break;
                };
                if current_ordered != ordered {
                    break;
                }
                let content = if ordered {
                    let dot = current.find(". ").unwrap_or(0);
                    current[dot + 2..].trim().to_string()
                } else {
                    current[2..].trim().to_string()
                };
                items.push(content);
                index += 1;
            }
            blocks.push(Block::List { ordered, items });
            continue;
        }

        if trimmed.starts_with('|')
            && index + 1 < lines.len()
            && is_table_separator(lines[index + 1])
        {
            let header = table_cells(trimmed);
            index += 2;
            let mut rows = Vec::new();
            while index < lines.len() {
                let current = lines[index].trim();
                if !current.starts_with('|') {
                    break;
                }
                rows.push(table_cells(current));
                index += 1;
            }
            blocks.push(Block::Table { header, rows });
            continue;
        }

        let mut paragraph = Vec::new();
        while index < lines.len() {
            let current = lines[index].trim();
            if current.is_empty()
                || current.starts_with("```")
                || heading(current).is_some()
                || is_rule(current)
                || current.starts_with('>')
                || list_marker(current).is_some()
                || (current.starts_with('|')
                    && index + 1 < lines.len()
                    && is_table_separator(lines[index + 1]))
            {
                break;
            }
            paragraph.push(current.to_string());
            index += 1;
        }
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(paragraph.join(" ")));
        }
    }
    blocks
}

fn heading(line: &str) -> Option<(u8, String)> {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if hashes == 0 || hashes > 4 {
        return None;
    }
    let rest = line[hashes..].trim_start();
    if rest.is_empty() {
        return None;
    }
    Some((hashes as u8, rest.to_string()))
}

fn is_rule(line: &str) -> bool {
    let trimmed = line.trim();
    (trimmed.starts_with("---") || trimmed.starts_with("***"))
        && trimmed.chars().all(|ch| matches!(ch, '-' | '*' | ' '))
        && trimmed.chars().filter(|ch| *ch != ' ').count() >= 3
}

fn list_marker(line: &str) -> Option<bool> {
    if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ ") {
        return Some(false);
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && line[digits..].starts_with(". ") {
        return Some(true);
    }
    None
}

fn is_table_separator(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') || !trimmed.contains('-') {
        return false;
    }
    trimmed
        .chars()
        .all(|ch| matches!(ch, '|' | '-' | ':' | ' '))
}

fn table_cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

/// Render the bundled plugin API guide. Parsing is cached because the modal is
/// repainted every frame while it is open.
pub fn api_doc(ui: &mut egui::Ui) {
    static BLOCKS: OnceLock<Vec<Block>> = OnceLock::new();
    let blocks = BLOCKS.get_or_init(|| parse(crate::plugins::docs::API_DOC));
    render(ui, blocks);
}

pub fn render(ui: &mut egui::Ui, blocks: &[Block]) {
    for (index, block) in blocks.iter().enumerate() {
        ui.push_id(("markdown_block", index), |ui| match block {
            Block::Heading { level, text } => {
                let size = match level {
                    1 => 20.0,
                    2 => 16.0,
                    3 => 13.5,
                    _ => 12.0,
                };
                ui.add_space(if *level <= 2 { 8.0 } else { 5.0 });
                ui.label(RichText::new(plain_inline(text)).size(size).strong().color(
                    if *level <= 2 {
                        theme::ACCENT
                    } else {
                        theme::text(ui)
                    },
                ));
                if *level == 2 {
                    ui.add_space(2.0);
                    ui.separator();
                }
                ui.add_space(3.0);
            }
            Block::Paragraph(text) => {
                render_inline(ui, text, ui.available_width());
                ui.add_space(5.0);
            }
            Block::Code { language, code } => {
                egui::Frame::new()
                    .fill(theme::subtle_fill(ui))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if language.is_empty() {
                                ui.label(RichText::new("code").size(9.5).color(theme::muted(ui)));
                            } else {
                                ui.label(
                                    RichText::new(language.as_str())
                                        .monospace()
                                        .size(9.5)
                                        .color(theme::muted(ui)),
                                );
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if super::kit::ghost_button(ui, "", "复制").clicked() {
                                        ui.ctx().copy_text(code.clone());
                                    }
                                },
                            );
                        });
                        ui.add_space(2.0);
                        ui.add(
                            egui::Label::new(RichText::new(code.trim_end()).monospace().size(10.0))
                                .wrap(),
                        );
                    });
                ui.add_space(6.0);
            }
            Block::List { ordered, items } => {
                for (item_index, item) in items.iter().enumerate() {
                    ui.horizontal_top(|ui| {
                        ui.add_sized(
                            [18.0, 0.0],
                            egui::Label::new(
                                RichText::new(if *ordered {
                                    format!("{}.", item_index + 1)
                                } else {
                                    "•".to_string()
                                })
                                .size(10.5)
                                .color(theme::muted(ui)),
                            ),
                        );
                        let width = (ui.available_width() - 4.0).max(80.0);
                        ui.vertical(|ui| {
                            render_inline(ui, item, width);
                        });
                    });
                    ui.add_space(2.0);
                }
                ui.add_space(4.0);
            }
            Block::Quote(text) => {
                egui::Frame::new()
                    .fill(theme::subtle_fill(ui))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(10, 7))
                    .show(ui, |ui| {
                        render_inline_colored(ui, text, theme::muted(ui), ui.available_width());
                    });
                ui.add_space(5.0);
            }
            Block::Table { header, rows } => {
                egui::ScrollArea::horizontal()
                    .id_salt(("markdown_table", index))
                    .show(ui, |ui| {
                        egui::Grid::new(("markdown_grid", index))
                            .striped(true)
                            .spacing([12.0, 5.0])
                            .show(ui, |ui| {
                                for cell in header {
                                    ui.label(
                                        RichText::new(plain_inline(cell))
                                            .size(10.0)
                                            .strong()
                                            .color(theme::text(ui)),
                                    );
                                }
                                ui.end_row();
                                for row in rows {
                                    for column in 0..header.len() {
                                        let cell = row.get(column).cloned().unwrap_or_default();
                                        ui.label(
                                            RichText::new(plain_inline(&cell))
                                                .size(9.8)
                                                .color(theme::text(ui)),
                                        );
                                    }
                                    ui.end_row();
                                }
                            });
                    });
                ui.add_space(5.0);
            }
            Block::Rule => {
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
            }
        });
    }
}

fn render_inline(ui: &mut egui::Ui, text: &str, max_width: f32) {
    render_inline_colored(ui, text, theme::text(ui), max_width);
}

fn render_inline_colored(ui: &mut egui::Ui, text: &str, color: egui::Color32, max_width: f32) {
    let font = FontId::proportional(11.3);
    let code_font = FontId::monospace(10.2);
    let mut job = LayoutJob::default();
    job.wrap.max_width = max_width.max(80.0);
    for span in inline_spans(text) {
        let mut format = TextFormat {
            font_id: font.clone(),
            color,
            ..Default::default()
        };
        let content = match span {
            Inline::Text(value) => value,
            Inline::Bold(value) => {
                format.color = ui.visuals().strong_text_color();
                format.extra_letter_spacing = 0.1;
                value
            }
            Inline::Italic(value) => {
                format.italics = true;
                value
            }
            Inline::Code(value) => {
                format.font_id = code_font.clone();
                format.background = theme::subtle_fill(ui);
                value
            }
            Inline::Link { text, .. } => {
                format.color = theme::ACCENT;
                format.underline = Stroke::new(1.0_f32, theme::ACCENT);
                text
            }
        };
        job.append(&content, 0.0, format);
    }
    ui.add(egui::Label::new(job).wrap());
}

fn inline_spans(text: &str) -> Vec<Inline> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut plain = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '*' && chars.get(index + 1) == Some(&'*') {
            if let Some(end) = find_sequence(&chars, index + 2, "**") {
                if !plain.is_empty() {
                    spans.push(Inline::Text(std::mem::take(&mut plain)));
                }
                spans.push(Inline::Bold(chars[index + 2..end].iter().collect()));
                index = end + 2;
                continue;
            }
        }
        if chars[index] == '`' {
            if let Some(end) = find_char(&chars, index + 1, '`') {
                if !plain.is_empty() {
                    spans.push(Inline::Text(std::mem::take(&mut plain)));
                }
                spans.push(Inline::Code(chars[index + 1..end].iter().collect()));
                index = end + 1;
                continue;
            }
        }
        if chars[index] == '[' {
            if let Some(link_end) = find_char(&chars, index + 1, ']') {
                if chars.get(link_end + 1) == Some(&'(') {
                    if let Some(url_end) = find_char(&chars, link_end + 2, ')') {
                        if !plain.is_empty() {
                            spans.push(Inline::Text(std::mem::take(&mut plain)));
                        }
                        spans.push(Inline::Link {
                            text: chars[index + 1..link_end].iter().collect(),
                            url: chars[link_end + 2..url_end].iter().collect(),
                        });
                        index = url_end + 1;
                        continue;
                    }
                }
            }
        }
        if chars[index] == '*' {
            if let Some(end) = find_char(&chars, index + 1, '*') {
                if !plain.is_empty() {
                    spans.push(Inline::Text(std::mem::take(&mut plain)));
                }
                spans.push(Inline::Italic(chars[index + 1..end].iter().collect()));
                index = end + 1;
                continue;
            }
        }
        plain.push(chars[index]);
        index += 1;
    }
    if !plain.is_empty() {
        spans.push(Inline::Text(plain));
    }
    spans
}

fn find_char(chars: &[char], from: usize, target: char) -> Option<usize> {
    (from..chars.len()).find(|index| chars[*index] == target)
}

fn find_sequence(chars: &[char], from: usize, sequence: &str) -> Option<usize> {
    let target: Vec<char> = sequence.chars().collect();
    if target.is_empty() {
        return None;
    }
    (from..chars.len().saturating_sub(target.len() - 1)).find(|index| {
        chars
            .get(*index..*index + target.len())
            .map(|slice| slice == target.as_slice())
            .unwrap_or(false)
    })
}

fn plain_inline(text: &str) -> String {
    inline_spans(text)
        .into_iter()
        .map(|span| match span {
            Inline::Text(value)
            | Inline::Bold(value)
            | Inline::Italic(value)
            | Inline::Code(value) => value,
            Inline::Link { text, .. } => text,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headings_code_lists_and_tables() {
        let blocks = parse(
            "# 标题\n\n一段 **粗体** 和 `代码` 文本。\n\n- 甲\n- 乙\n\n```js\nconst a = 1;\n```\n\n| A | B |\n|:--|:--|\n| 1 | 2 |\n",
        );
        assert_eq!(
            blocks[0],
            Block::Heading {
                level: 1,
                text: "标题".into()
            }
        );
        assert!(matches!(&blocks[1], Block::Paragraph(text) if text.contains("**粗体**")));
        assert_eq!(
            blocks[2],
            Block::List {
                ordered: false,
                items: vec!["甲".into(), "乙".into()]
            }
        );
        assert_eq!(
            blocks[3],
            Block::Code {
                language: "js".into(),
                code: "const a = 1;\n".into()
            }
        );
        assert_eq!(
            blocks[4],
            Block::Table {
                header: vec!["A".into(), "B".into()],
                rows: vec![vec!["1".into(), "2".into()]]
            }
        );
    }

    #[test]
    fn inline_spans_support_bold_code_and_links() {
        let spans = inline_spans("a **b** `c` [d](https://example.com) *e*");
        assert_eq!(spans[0], Inline::Text("a ".into()));
        assert_eq!(spans[1], Inline::Bold("b".into()));
        assert_eq!(spans[2], Inline::Text(" ".into()));
        assert_eq!(spans[3], Inline::Code("c".into()));
        assert_eq!(
            spans[5],
            Inline::Link {
                text: "d".into(),
                url: "https://example.com".into()
            }
        );
        assert_eq!(plain_inline("a **b** `c`"), "a b c");
    }

    #[test]
    fn ordered_lists_and_quotes_are_recognized() {
        let blocks = parse("1. 甲\n2. 乙\n\n> 引用\n");
        assert_eq!(
            blocks[0],
            Block::List {
                ordered: true,
                items: vec!["甲".into(), "乙".into()]
            }
        );
        assert_eq!(blocks[1], Block::Quote("引用".into()));
    }
}
