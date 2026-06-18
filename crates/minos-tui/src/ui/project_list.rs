use crate::backend::ProjectEntry;
use crate::render::Renderable;
use crate::ui::theme;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
    Frame,
};

pub struct ProjectListRenderable<'a> {
    projects: &'a [ProjectEntry],
    selected: Option<usize>,
    list_state: &'a mut ListState,
    focused: bool,
}

impl<'a> ProjectListRenderable<'a> {
    pub fn new(
        projects: &'a [ProjectEntry],
        selected: Option<usize>,
        list_state: &'a mut ListState,
        focused: bool,
    ) -> Self {
        Self {
            projects,
            selected,
            list_state,
            focused,
        }
    }
}

impl Renderable for ProjectListRenderable<'_> {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let border_style = if self.focused {
            theme::FOCUSED_BORDER
        } else {
            Style::new().fg(theme::BORDER_FG)
        };
        let block = Block::bordered()
            .title("Projects")
            .border_style(border_style);
        let items: Vec<ListItem> = self
            .projects
            .iter()
            .enumerate()
            .map(|(index, p)| {
                let prefix = if self.selected == Some(index) {
                    "> "
                } else {
                    "  "
                };
                ListItem::new(Line::from(vec![
                    Span::styled(prefix, Style::new().fg(Color::Cyan)),
                    Span::styled(format!("{:<16} ", p.name), Style::new().fg(Color::Cyan)),
                    Span::raw(p.workspace_path.to_string_lossy().to_string()),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_style(theme::HIGHLIGHTED);
        frame.render_stateful_widget(list, area, self.list_state);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        0
    }
}

pub struct ProjectSidebarRenderable<'a> {
    projects: &'a [ProjectEntry],
    selected: Option<usize>,
}

impl<'a> ProjectSidebarRenderable<'a> {
    pub fn new(projects: &'a [ProjectEntry], selected: Option<usize>) -> Self {
        Self { projects, selected }
    }
}

impl Renderable for ProjectSidebarRenderable<'_> {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered()
            .title("Project Info")
            .border_style(Style::new().fg(theme::BORDER_FG));
        let content = match self.selected.and_then(|i| self.projects.get(i)) {
            Some(project) => Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("Name: ", Style::new().fg(theme::BORDER_FG)),
                    Span::raw(&project.name),
                ]),
                Line::from(vec![
                    Span::styled("Path: ", Style::new().fg(theme::BORDER_FG)),
                    Span::raw(project.workspace_path.to_string_lossy().to_string()),
                ]),
                Line::from(vec![
                    Span::styled("Sessions: ", Style::new().fg(theme::BORDER_FG)),
                    Span::raw(project.thread_count.to_string()),
                ]),
            ])
            .block(block),
            None => Paragraph::new("Select a project").block(block),
        };
        frame.render_widget(content, area);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        0
    }
}
