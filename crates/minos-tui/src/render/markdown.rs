use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

use super::highlight::highlight_to_lines;

/// Markdown features used by AgentDetail transcript rendering.
const MARKDOWN_OPTIONS: Options = Options::ENABLE_TABLES
    .union(Options::ENABLE_STRIKETHROUGH)
    .union(Options::ENABLE_TASKLISTS);

#[derive(Clone, Copy)]
pub(crate) struct MarkdownStyles {
    pub text: Style,
    pub heading: Style,
    pub bold: Style,
    pub italic: Style,
    pub code_inline: Style,
    pub code_block: Style,
    pub code_block_border: Style,
    pub quote: Style,
    pub link: Style,
    pub list_marker: Style,
    pub diff_add: Style,
    pub diff_del: Style,
    pub diff_hunk: Style,
    pub diff_gutter: Style,
}

pub(crate) fn render_markdown(text: &str, styles: MarkdownStyles) -> Vec<Line<'static>> {
    let mut renderer = Renderer::new(styles);
    for event in Parser::new_ext(text, MARKDOWN_OPTIONS) {
        renderer.event(event);
    }
    renderer.finish()
}

/// Whether `text` looks like a unified/patch diff (for tool detail routing).
pub(crate) fn looks_like_diff(text: &str) -> bool {
    is_diff_block("", text)
}

pub(crate) fn render_code_block(
    lang: &str,
    code: &str,
    styles: MarkdownStyles,
) -> Vec<Line<'static>> {
    let label = lang.trim();
    let label = if label.is_empty() { "code" } else { label };
    let code = code.trim_end_matches(['\n', '\r']);
    let mut lines = vec![Line::from(Span::styled(
        format!("┌─ {label} ─"),
        styles.code_block_border,
    ))];

    if is_diff_block(label, code) {
        // Assistant fenced ```diff keeps bordered dual-gutter chrome.
        lines.extend(render_diff_body(code, styles, DiffSurface::Fenced));
    } else if let Some(highlighted) = highlight_to_lines(code, label) {
        lines.extend(highlighted.into_iter().map(|spans| {
            let mut prefixed = vec![Span::styled("│ ", styles.code_block_border)];
            prefixed.extend(spans);
            Line::from(prefixed)
        }));
    } else {
        lines.extend(code.split('\n').map(|line| {
            Line::from(vec![
                Span::styled("│ ", styles.code_block_border),
                Span::styled(line.to_owned(), styles.code_block),
            ])
        }));
    }

    lines.push(Line::from(Span::styled("└──", styles.code_block_border)));
    lines
}

/// Grok edit-style tool diff: 2-space indent, single new-line gutter, no box border.
pub(crate) fn render_tool_diff(code: &str, styles: MarkdownStyles) -> Vec<Line<'static>> {
    let code = code.trim_end_matches(['\n', '\r']);
    render_diff_body(code, styles, DiffSurface::ToolEdit)
}

/// Unbordered preformatted tool body (stdout / generic multi-line).
pub(crate) fn render_tool_preformatted(
    code: &str,
    styles: MarkdownStyles,
    first: usize,
    last: usize,
) -> Vec<Line<'static>> {
    let code = code.trim_end_matches(['\n', '\r']);
    let raw_lines: Vec<&str> = code.split('\n').collect();
    let total = raw_lines.len();
    let selected: Vec<&str> = if first + last < total {
        let mut out = Vec::with_capacity(first + last + 1);
        out.extend_from_slice(&raw_lines[..first]);
        out.push("…");
        out.extend_from_slice(&raw_lines[total - last..]);
        out
    } else {
        raw_lines
    };

    selected
        .into_iter()
        .map(|line| {
            Line::from(vec![
                Span::raw("  "),
                Span::styled(line.to_owned(), styles.code_block),
            ])
        })
        .collect()
}

/// Read-tool body: line-number gutter + optional syntax highlight, first/last truncate.
pub(crate) fn render_tool_read_body(
    code: &str,
    path_or_lang: &str,
    styles: MarkdownStyles,
    first: usize,
    last: usize,
) -> Vec<Line<'static>> {
    let code = code.trim_end_matches(['\n', '\r']);
    let lang = path_extension_lang(path_or_lang);
    let highlighted = highlight_to_lines(code, &lang);
    let raw_lines: Vec<&str> = code.split('\n').collect();
    let total = raw_lines.len();
    let gutter_w = digit_count(total.max(1));

    let indices: Vec<usize> = if first + last < total {
        let mut idx: Vec<usize> = (0..first).collect();
        idx.push(usize::MAX); // ellipsis marker
        idx.extend(total - last..total);
        idx
    } else {
        (0..total).collect()
    };

    indices
        .into_iter()
        .map(|i| {
            if i == usize::MAX {
                return Line::from(vec![
                    Span::raw("  "),
                    Span::styled("…".to_owned(), styles.diff_gutter),
                ]);
            }
            let line_no = i + 1;
            let gutter = format!("  {line_no:>gutter_w$}  ");
            let mut spans = vec![Span::styled(gutter, styles.diff_gutter)];
            if let Some(ref hl) = highlighted {
                if let Some(line_spans) = hl.get(i) {
                    spans.extend(line_spans.iter().cloned());
                } else {
                    spans.push(Span::styled(
                        raw_lines.get(i).unwrap_or(&"").to_string(),
                        styles.code_block,
                    ));
                }
            } else {
                spans.push(Span::styled(
                    raw_lines.get(i).unwrap_or(&"").to_string(),
                    styles.code_block,
                ));
            }
            Line::from(spans)
        })
        .collect()
}

fn path_extension_lang(path_or_lang: &str) -> String {
    let name = path_or_lang
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path_or_lang);
    if let Some(ext) = name.rsplit_once('.').map(|(_, e)| e) {
        if !ext.is_empty() && ext.len() <= 12 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
            return ext.to_owned();
        }
    }
    path_or_lang.to_owned()
}

fn digit_count(n: usize) -> usize {
    n.checked_ilog10().map_or(1, |d| d as usize + 1)
}

#[derive(Clone, Copy)]
enum DiffSurface {
    /// Assistant ```diff fence: dual gutters + border prefix on each line.
    Fenced,
    /// Tool edit expand: indent + single new-line gutter + hunk separators.
    ToolEdit,
}

struct Renderer {
    styles: MarkdownStyles,
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    style_stack: Vec<Style>,
    list_stack: Vec<ListKind>,
    item_stack: Vec<ItemPrefix>,
    quote_depth: usize,
    code_block: Option<CodeBlock>,
    link_stack: Vec<String>,
    table: Option<TableBuilder>,
}

struct CodeBlock {
    lang: String,
    code: String,
}

struct TableBuilder {
    rows: Vec<Vec<String>>,
    current_row: Vec<String>,
    current_cell: String,
    in_cell: bool,
    /// Number of header rows (usually 1). Used for separator placement.
    header_rows: usize,
    in_head: bool,
}

struct ItemPrefix {
    marker: String,
    continuation: String,
    marker_pending: bool,
}

enum ListKind {
    Unordered,
    Ordered { next: u64 },
}

impl Renderer {
    fn new(styles: MarkdownStyles) -> Self {
        Self {
            styles,
            lines: Vec::new(),
            current: Vec::new(),
            style_stack: Vec::new(),
            list_stack: Vec::new(),
            item_stack: Vec::new(),
            quote_depth: 0,
            code_block: None,
            link_stack: Vec::new(),
            table: None,
        }
    }

    fn event(&mut self, event: Event<'_>) {
        // Table cells collect plain text; inline events still route here.
        if let Some(table) = self.table.as_mut() {
            if table.in_cell {
                match &event {
                    Event::Text(text) | Event::Code(text) => {
                        table.current_cell.push_str(text);
                        return;
                    }
                    Event::SoftBreak | Event::HardBreak => {
                        table.current_cell.push(' ');
                        return;
                    }
                    Event::Start(Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. })
                    | Event::End(
                        TagEnd::Emphasis
                        | TagEnd::Strong
                        | TagEnd::Strikethrough
                        | TagEnd::Link,
                    ) => {
                        // Skip style wrappers inside cells; keep text only.
                        return;
                    }
                    _ => {}
                }
            }
        }

        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.push_text(&text),
            Event::Code(code) => self.push_span(code.to_string(), self.styles.code_inline),
            Event::SoftBreak | Event::HardBreak => self.flush_current(),
            Event::Rule => {
                self.flush_current();
                self.lines.push(Line::from(Span::styled(
                    "───".to_owned(),
                    self.styles.code_block_border,
                )));
            }
            Event::Html(html) | Event::InlineHtml(html) => self.push_text(&html),
            Event::FootnoteReference(reference) => {
                self.push_span(format!("[{reference}]"), self.styles.text);
            }
            Event::TaskListMarker(checked) => {
                let marker = if checked { "[x] " } else { "[ ] " };
                self.push_span(marker.to_owned(), self.styles.list_marker);
            }
            Event::InlineMath(math) | Event::DisplayMath(math) => self.push_text(&math),
        }
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush_current();
                self.push_style(heading_style(level, self.styles.heading));
            }
            Tag::BlockQuote(_) => {
                self.flush_current();
                self.quote_depth += 1;
            }
            Tag::CodeBlock(kind) => {
                self.flush_current();
                let lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code_block = Some(CodeBlock {
                    lang,
                    code: String::new(),
                });
            }
            Tag::List(start) => self.start_list(start),
            Tag::Item => self.start_item(),
            Tag::Emphasis => self.push_style(self.styles.italic),
            Tag::Strong => self.push_style(self.styles.bold),
            Tag::Link { dest_url, .. } => {
                self.link_stack.push(dest_url.to_string());
                self.push_style(self.styles.link);
            }
            Tag::Strikethrough => {
                let mut style = Style::new();
                style.add_modifier |= Modifier::CROSSED_OUT;
                self.push_style(style);
            }
            Tag::Table(_) => {
                self.flush_current();
                self.table = Some(TableBuilder {
                    rows: Vec::new(),
                    current_row: Vec::new(),
                    current_cell: String::new(),
                    in_cell: false,
                    header_rows: 0,
                    in_head: false,
                });
            }
            Tag::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.in_head = true;
                }
            }
            Tag::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.current_row.clear();
                }
            }
            Tag::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.in_cell = true;
                    table.current_cell.clear();
                }
            }
            Tag::Superscript | Tag::Subscript => {}
            Tag::HtmlBlock
            | Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Image { .. }
            | Tag::MetadataBlock(_) => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush_current(),
            TagEnd::Heading(_) => {
                self.flush_current();
                self.pop_style();
            }
            TagEnd::BlockQuote(_) => {
                self.flush_current();
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            TagEnd::CodeBlock => {
                if let Some(block) = self.code_block.take() {
                    self.lines
                        .extend(render_code_block(&block.lang, &block.code, self.styles));
                }
            }
            TagEnd::List(_) => {
                self.flush_current();
                self.list_stack.pop();
            }
            TagEnd::Item => {
                self.flush_current();
                self.item_stack.pop();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_style(),
            TagEnd::Link => {
                self.pop_style();
                if let Some(dest) = self.link_stack.pop() {
                    self.push_span(format!(" ({dest})"), self.styles.link);
                }
            }
            TagEnd::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.in_cell = false;
                    let cell = std::mem::take(&mut table.current_cell);
                    table.current_row.push(cell.trim().to_owned());
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    let row = std::mem::take(&mut table.current_row);
                    if !row.is_empty() {
                        table.rows.push(row);
                        if table.in_head {
                            table.header_rows = table.header_rows.saturating_add(1);
                        }
                    }
                }
            }
            TagEnd::TableHead => {
                // pulldown-cmark emits header cells under TableHead without a
                // TableRow wrapper — flush the accumulated header row here.
                if let Some(table) = self.table.as_mut() {
                    let row = std::mem::take(&mut table.current_row);
                    if !row.is_empty() {
                        table.rows.push(row);
                        table.header_rows = table.header_rows.saturating_add(1);
                    }
                    table.in_head = false;
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.lines
                        .extend(render_table(table.rows, table.header_rows, self.styles));
                }
            }
            TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::HtmlBlock
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::Image
            | TagEnd::FootnoteDefinition
            | TagEnd::MetadataBlock(_) => {}
        }
    }

    fn start_list(&mut self, start: Option<u64>) {
        self.flush_current();
        self.list_stack.push(match start {
            Some(start) => ListKind::Ordered { next: start },
            None => ListKind::Unordered,
        });
    }

    fn start_item(&mut self) {
        self.flush_current();
        let marker = match self.list_stack.last_mut() {
            Some(ListKind::Ordered { next }) => {
                let marker = format!("{next}. ");
                *next = next.saturating_add(1);
                marker
            }
            Some(ListKind::Unordered) | None => "• ".to_owned(),
        };
        let continuation = " ".repeat(marker.len());
        self.item_stack.push(ItemPrefix {
            marker,
            continuation,
            marker_pending: true,
        });
    }

    fn push_text(&mut self, text: &str) {
        if let Some(block) = self.code_block.as_mut() {
            block.code.push_str(text);
            return;
        }

        for (index, chunk) in text.split('\n').enumerate() {
            if index > 0 {
                self.flush_current();
            }
            if !chunk.is_empty() {
                let style = if self.can_style_plain_diff_line() && is_diff_line(chunk.trim_start())
                {
                    diff_style(chunk.trim_start(), self.styles)
                } else {
                    self.current_style()
                };
                self.push_span(chunk.to_owned(), style);
            }
        }
    }

    fn push_span(&mut self, text: String, style: Style) {
        self.ensure_prefix();
        self.current.push(Span::styled(text, style));
    }

    fn push_style(&mut self, style: Style) {
        self.style_stack.push(style);
    }

    fn pop_style(&mut self) {
        self.style_stack.pop();
    }

    fn current_style(&self) -> Style {
        self.style_stack
            .iter()
            .fold(self.styles.text, |style, overlay| {
                merge_style(style, *overlay)
            })
    }

    fn can_style_plain_diff_line(&self) -> bool {
        self.style_stack.is_empty()
            && self.item_stack.is_empty()
            && self.quote_depth == 0
            && self.table.is_none()
    }

    fn ensure_prefix(&mut self) {
        if !self.current.is_empty() {
            return;
        }
        // Build into a local buffer so we can mutably take from `item_stack`
        // without fighting `self.current`'s borrow.
        let mut spans = Vec::new();
        for _ in 0..self.quote_depth {
            spans.push(Span::styled("│ ", self.styles.quote));
        }
        let list_marker = self.styles.list_marker;
        for prefix in &mut self.item_stack {
            if prefix.marker_pending {
                // Marker is only rendered once; move it out instead of cloning.
                let marker = std::mem::take(&mut prefix.marker);
                prefix.marker_pending = false;
                spans.push(Span::styled(marker, list_marker));
            } else {
                // Continuation indent is reused on every subsequent line.
                spans.push(Span::raw(prefix.continuation.clone()));
            }
        }
        self.current = spans;
    }

    fn flush_current(&mut self) {
        if self.code_block.is_some() {
            if let Some(block) = self.code_block.as_mut() {
                block.code.push('\n');
            }
            return;
        }

        if !self.current.is_empty() {
            self.lines
                .push(Line::from(std::mem::take(&mut self.current)));
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_current();
        self.lines
    }
}

fn render_table(
    rows: Vec<Vec<String>>,
    header_rows: usize,
    styles: MarkdownStyles,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return Vec::new();
    }

    let col_count = rows.iter().map(Vec::len).max().unwrap_or(0);
    if col_count == 0 {
        return Vec::new();
    }

    let mut widths = vec![0usize; col_count];
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(UnicodeWidthStr::width(cell.as_str()));
        }
    }

    let mut out = Vec::with_capacity(rows.len().saturating_add(2));
    for (row_idx, row) in rows.iter().enumerate() {
        let mut spans = Vec::with_capacity(col_count.saturating_mul(2).saturating_add(1));
        spans.push(Span::styled("│ ", styles.code_block_border));
        for (i, width) in widths.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" │ ", styles.code_block_border));
            }
            let cell = row.get(i).map(String::as_str).unwrap_or("");
            let pad = width.saturating_sub(UnicodeWidthStr::width(cell));
            let style = if row_idx < header_rows {
                styles.heading
            } else {
                styles.text
            };
            spans.push(Span::styled(format!("{cell}{}", " ".repeat(pad)), style));
        }
        spans.push(Span::styled(" │", styles.code_block_border));
        out.push(Line::from(spans));

        if header_rows > 0 && row_idx + 1 == header_rows {
            let mut sep = Vec::with_capacity(col_count.saturating_mul(2).saturating_add(1));
            sep.push(Span::styled("├─", styles.code_block_border));
            for (i, width) in widths.iter().enumerate() {
                if i > 0 {
                    sep.push(Span::styled("─┼─", styles.code_block_border));
                }
                sep.push(Span::styled(
                    "─".repeat((*width).max(1)),
                    styles.code_block_border,
                ));
            }
            sep.push(Span::styled("─┤", styles.code_block_border));
            out.push(Line::from(sep));
        }
    }
    out
}

fn heading_style(level: HeadingLevel, base: Style) -> Style {
    let mut style = base;
    if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) {
        style.add_modifier |= Modifier::BOLD;
    }
    style
}

fn merge_style(mut base: Style, overlay: Style) -> Style {
    if overlay.fg.is_some() {
        base.fg = overlay.fg;
    }
    if overlay.bg.is_some() {
        base.bg = overlay.bg;
    }
    if overlay.underline_color.is_some() {
        base.underline_color = overlay.underline_color;
    }
    base.add_modifier |= overlay.add_modifier;
    base.sub_modifier |= overlay.sub_modifier;
    base
}

fn is_diff_block(lang: &str, code: &str) -> bool {
    let lang = lang.to_ascii_lowercase();
    lang.contains("diff")
        || lang.contains("patch")
        || code.contains("diff --git")
        || code.contains("\n@@")
        || code.starts_with("@@")
        || code.contains("*** Begin Patch")
        || code.contains("*** Update File:")
        || code.contains("*** Add File:")
        || code.contains("*** Delete File:")
        || code
            .lines()
            .any(|line| line.starts_with("+++ ") || line.starts_with("--- "))
}

fn is_diff_line(line: &str) -> bool {
    line.starts_with('+')
        || line.starts_with('-')
        || line.starts_with("@@")
        || line.starts_with("diff --git")
        || line.starts_with("*** ")
}

fn diff_style(line: &str, styles: MarkdownStyles) -> Style {
    if line.starts_with('+') && !line.starts_with("+++") {
        styles.diff_add
    } else if line.starts_with('-') && !line.starts_with("---") {
        styles.diff_del
    } else if line.starts_with("@@")
        || line.starts_with("diff --git")
        || line.starts_with("*** ")
    {
        styles.diff_hunk
    } else {
        styles.code_block
    }
}

/// Unified-diff body. Fenced = dual gutters + border; ToolEdit = Grok single gutter.
fn render_diff_body(
    code: &str,
    styles: MarkdownStyles,
    surface: DiffSurface,
) -> Vec<Line<'static>> {
    let mut old_line: Option<u32> = None;
    let mut new_line: Option<u32> = None;
    let mut out = Vec::new();
    let mut saw_content = false;

    for raw in code.split('\n') {
        if matches!(surface, DiffSurface::ToolEdit) && raw.starts_with("@@") && saw_content {
            out.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("…".to_owned(), styles.diff_gutter),
            ]));
        }

        let style = diff_style(raw, styles);
        let (old_num, new_num) = if let Some((old_start, new_start)) = parse_hunk_header(raw) {
            old_line = Some(old_start);
            new_line = Some(new_start);
            (None, None)
        } else if raw.starts_with('+') && !raw.starts_with("+++") {
            let n = new_line;
            if let Some(v) = new_line.as_mut() {
                *v = v.saturating_add(1);
            }
            saw_content = true;
            (None, n)
        } else if raw.starts_with('-') && !raw.starts_with("---") {
            let o = old_line;
            if let Some(v) = old_line.as_mut() {
                *v = v.saturating_add(1);
            }
            saw_content = true;
            (o, None)
        } else if raw.starts_with(' ')
            || (old_line.is_some()
                && !raw.starts_with("diff ")
                && !raw.starts_with("*** ")
                && !raw.starts_with("---")
                && !raw.starts_with("+++")
                && !raw.starts_with("index ")
                && !raw.starts_with("@@"))
        {
            // Context lines (leading space) or unprefixed body once a hunk is active.
            let o = old_line;
            let n = new_line;
            if let Some(v) = old_line.as_mut() {
                *v = v.saturating_add(1);
            }
            if let Some(v) = new_line.as_mut() {
                *v = v.saturating_add(1);
            }
            saw_content = true;
            (o, n)
        } else {
            // File headers / patch meta — no line numbers.
            (None, None)
        };

        match surface {
            DiffSurface::Fenced => {
                out.push(Line::from(vec![
                    Span::styled("│ ", styles.code_block_border),
                    Span::styled(format_gutter(old_num), styles.diff_gutter),
                    Span::styled(" ", styles.diff_gutter),
                    Span::styled(format_gutter(new_num), styles.diff_gutter),
                    Span::styled(" │ ", styles.code_block_border),
                    Span::styled(raw.to_owned(), style),
                ]));
            }
            DiffSurface::ToolEdit => {
                // Grok default: single new-file line number + content (no dual gutter).
                let gutter = if raw.starts_with("@@")
                    || raw.starts_with("diff ")
                    || raw.starts_with("*** ")
                    || raw.starts_with("---")
                    || raw.starts_with("+++")
                    || raw.starts_with("index ")
                {
                    "    ".to_owned()
                } else {
                    format_gutter(new_num.or(old_num))
                };
                out.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(gutter, styles.diff_gutter),
                    Span::raw(" "),
                    Span::styled(raw.to_owned(), style),
                ]));
            }
        }
    }
    out
}

fn format_gutter(num: Option<u32>) -> String {
    match num {
        Some(n) => format!("{n:>4}"),
        None => "    ".to_owned(),
    }
}

/// Parse `@@ -old_start,old_count +new_start,new_count @@` (counts optional).
fn parse_hunk_header(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("@@")?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('-')?;
    let (old_part, rest) = rest.split_once('+')?;
    let old_start = old_part
        .split(',')
        .next()?
        .trim()
        .parse::<u32>()
        .ok()?;
    let new_start = rest
        .split_whitespace()
        .next()?
        .split(',')
        .next()?
        .trim()
        .parse::<u32>()
        .ok()?;
    Some((old_start, new_start))
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Style};

    use super::{
        looks_like_diff, render_code_block, render_markdown, render_tool_diff, MarkdownStyles,
    };

    fn styles() -> MarkdownStyles {
        MarkdownStyles {
            text: Style::new(),
            heading: Style::new().fg(Color::White),
            bold: Style::new().add_modifier(ratatui::style::Modifier::BOLD),
            italic: Style::new().add_modifier(ratatui::style::Modifier::ITALIC),
            code_inline: Style::new().fg(Color::Yellow),
            code_block: Style::new().fg(Color::Yellow),
            code_block_border: Style::new().fg(Color::DarkGray),
            quote: Style::new().fg(Color::DarkGray),
            link: Style::new().fg(Color::Cyan),
            list_marker: Style::new().fg(Color::White),
            diff_add: Style::new().fg(Color::Green),
            diff_del: Style::new().fg(Color::Red),
            diff_hunk: Style::new().fg(Color::Cyan),
            diff_gutter: Style::new().fg(Color::DarkGray),
        }
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    use ratatui::text::Line;

    #[test]
    fn renders_basic_markdown() {
        let lines = render_markdown("# Plan\n- run `cargo test`", styles());
        let rendered = lines.iter().map(text).collect::<Vec<_>>();

        assert_eq!(rendered[0], "Plan");
        assert!(rendered.iter().any(|line| line.contains("• run ")));
    }

    #[test]
    fn renders_code_block_with_border() {
        let lines = render_markdown("```rust\nfn main() {}\n```", styles());
        let rendered = lines.iter().map(text).collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line == "┌─ rust ─"));
        assert!(rendered.iter().any(|line| line.contains("fn main() {}")));
    }

    #[test]
    fn renders_pipe_table() {
        let md = "| Name | Role |\n| --- | --- |\n| Alice | Eng |\n";
        let lines = render_markdown(md, styles());
        let rendered = lines.iter().map(text).collect::<Vec<_>>();

        assert!(
            rendered.iter().any(|line| line.contains("Name") && line.contains("Role")),
            "header row missing: {rendered:?}"
        );
        assert!(
            rendered.iter().any(|line| line.contains("Alice") && line.contains("Eng")),
            "body row missing: {rendered:?}"
        );
        assert!(
            rendered.iter().any(|line| line.contains('┼') || line.contains('─')),
            "separator missing: {rendered:?}"
        );
    }

    #[test]
    fn renders_task_list_and_strikethrough() {
        let lines = render_markdown("- [x] done\n- [ ] todo\n~~old~~", styles());
        let rendered = lines.iter().map(text).collect::<Vec<_>>();
        assert!(rendered.iter().any(|line| line.contains("[x]")));
        assert!(rendered.iter().any(|line| line.contains("[ ]")));
        assert!(rendered.iter().any(|line| line.contains("old")));
    }

    #[test]
    fn looks_like_diff_detects_unified_and_apply_patch() {
        assert!(looks_like_diff("diff --git a/x b/x\n@@ -1 +1 @@\n-a\n+b\n"));
        assert!(looks_like_diff("*** Begin Patch\n*** Update File: a.rs\n"));
        assert!(!looks_like_diff("- just a markdown bullet list item"));
    }

    #[test]
    fn render_code_block_colors_diff_lines() {
        let lines = render_code_block("diff", "@@ -1 +1 @@\n-old\n+new\n", styles());
        let added = lines
            .iter()
            .find(|line| text(line).contains("+new"))
            .expect("added");
        // border, old gutter, space, new gutter, separator, content
        assert_eq!(added.spans.last().unwrap().style, styles().diff_add);
    }

    #[test]
    fn diff_gutter_shows_hunk_line_numbers() {
        let lines = render_code_block(
            "diff",
            "@@ -10,2 +20,2 @@\n context\n-old line\n+new line\n",
            styles(),
        );
        let rendered = lines.iter().map(text).collect::<Vec<_>>();
        // old 10 / new 20 on context (after hunk header sets counters)
        let context = rendered
            .iter()
            .find(|line| line.contains("context"))
            .expect("context");
        assert!(
            context.contains("  10") && context.contains("  20"),
            "context gutters missing: {context:?}"
        );
        let deleted = rendered
            .iter()
            .find(|line| line.contains("-old line"))
            .expect("deleted");
        assert!(
            deleted.contains("  11") && deleted.contains("    "),
            "delete gutter missing: {deleted:?}"
        );
        let added = rendered
            .iter()
            .find(|line| line.contains("+new line"))
            .expect("added");
        assert!(
            added.contains("  21"),
            "add gutter missing: {added:?}"
        );
    }

    #[test]
    fn tool_diff_is_unbordered_single_gutter() {
        let lines = render_tool_diff(
            "@@ -1 +1 @@\n-old\n+new\n",
            styles(),
        );
        let rendered = lines.iter().map(text).collect::<Vec<_>>();
        assert!(!rendered.iter().any(|line| line.contains("┌─")));
        let added = rendered
            .iter()
            .find(|line| line.contains("+new"))
            .expect("+new");
        assert!(
            added.starts_with("  "),
            "expected indent: {added:?}"
        );
        // single gutter only (no dual old|new pair with border bars)
        assert!(
            !added.contains('│'),
            "tool diff should not use fence border: {added:?}"
        );
    }
}
