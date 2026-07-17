//! Renderable trait and simple flex containers.

use ratatui::{layout::Rect, Frame};

pub(crate) mod highlight;
pub(crate) mod markdown;
pub(crate) mod table_detect;

pub trait Renderable {
    fn render(&mut self, frame: &mut Frame, area: Rect);

    fn desired_height(&self, width: u16) -> u16;

    fn cursor_pos(&self, _area: Rect) -> Option<(u16, u16)> {
        None
    }
}

pub struct Column<'a> {
    children: Vec<Box<dyn Renderable + 'a>>,
    fill_index: Option<usize>,
}

impl<'a> Column<'a> {
    pub fn with_fill(children: Vec<Box<dyn Renderable + 'a>>, fill_index: usize) -> Self {
        assert!(
            fill_index < children.len(),
            "Column fill index must point to an existing child"
        );
        Self {
            children,
            fill_index: Some(fill_index),
        }
    }

    pub fn child_areas(&self, area: Rect) -> Vec<Rect> {
        let heights = self.layout_heights(area);
        let mut y = area.y;
        heights
            .into_iter()
            .map(|height| {
                let rect = Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height,
                };
                y = y.saturating_add(height);
                rect
            })
            .collect()
    }

    fn layout_heights(&self, area: Rect) -> Vec<u16> {
        let desired = self
            .children
            .iter()
            .map(|child| child.desired_height(area.width))
            .collect::<Vec<_>>();
        if let Some(fill_index) = self.fill_index {
            return layout_heights_with_fill(area.height, desired, fill_index);
        }

        let total = desired
            .iter()
            .copied()
            .fold(0u16, |sum, height| sum.saturating_add(height));
        if total <= area.height {
            return desired;
        }

        let mut scaled = desired
            .iter()
            .map(|height| {
                let value = u32::from(*height) * u32::from(area.height) / u32::from(total.max(1));
                u16::try_from(value).unwrap_or(0)
            })
            .collect::<Vec<_>>();
        let allocated = scaled
            .iter()
            .copied()
            .fold(0u16, |sum, height| sum.saturating_add(height));
        if let Some(last) = scaled.last_mut() {
            *last = last.saturating_add(area.height.saturating_sub(allocated));
        }
        scaled
    }
}

impl Renderable for Column<'_> {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let areas = self.child_areas(area);
        for (child, area) in self.children.iter_mut().zip(areas) {
            child.render(frame, area);
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.children
            .iter()
            .map(|child| child.desired_height(width))
            .fold(0u16, |sum, height| sum.saturating_add(height))
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.children
            .iter()
            .zip(self.child_areas(area))
            .find_map(|(child, area)| child.cursor_pos(area))
    }
}

fn layout_heights_with_fill(height: u16, desired: Vec<u16>, fill_index: usize) -> Vec<u16> {
    let fixed_total = desired
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != fill_index)
        .map(|(_, height)| *height)
        .fold(0u16, |sum, height| sum.saturating_add(height));

    if fixed_total <= height {
        let mut heights = desired;
        if let Some(fill_height) = heights.get_mut(fill_index) {
            *fill_height = height.saturating_sub(fixed_total);
        }
        return heights;
    }

    let mut heights = vec![0; desired.len()];
    let mut remaining = height;
    for (index, desired_height) in desired.into_iter().enumerate() {
        if index == fill_index {
            continue;
        }
        let value = desired_height.min(remaining);
        heights[index] = value;
        remaining = remaining.saturating_sub(value);
    }

    heights
}

pub struct Row<'a> {
    children: Vec<Box<dyn Renderable + 'a>>,
    ratios: Vec<u16>,
}

impl<'a> Row<'a> {
    pub fn new(children: Vec<Box<dyn Renderable + 'a>>, ratios: Vec<u16>) -> Self {
        assert_eq!(
            children.len(),
            ratios.len(),
            "Row children and ratios must have the same length"
        );
        Self { children, ratios }
    }

    pub fn areas_for(area: Rect, ratios: &[u16]) -> Vec<Rect> {
        let widths = layout_widths(area.width, ratios);
        let mut x = area.x;
        widths
            .into_iter()
            .map(|width| {
                let rect = Rect {
                    x,
                    y: area.y,
                    width,
                    height: area.height,
                };
                x = x.saturating_add(width);
                rect
            })
            .collect()
    }

    pub fn child_areas(&self, area: Rect) -> Vec<Rect> {
        Self::areas_for(area, &self.ratios)
    }

    fn layout_widths(&self, width: u16) -> Vec<u16> {
        layout_widths(width, &self.ratios)
    }
}

impl Renderable for Row<'_> {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let areas = self.child_areas(area);
        for (child, area) in self.children.iter_mut().zip(areas) {
            child.render(frame, area);
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        let widths = self.layout_widths(width);
        self.children
            .iter()
            .zip(widths)
            .map(|(child, width)| child.desired_height(width))
            .max()
            .unwrap_or(0)
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.children
            .iter()
            .zip(self.child_areas(area))
            .find_map(|(child, area)| child.cursor_pos(area))
    }
}

fn layout_widths(width: u16, ratios: &[u16]) -> Vec<u16> {
    let ratio_sum = ratios
        .iter()
        .copied()
        .fold(0u16, |sum, ratio| sum.saturating_add(ratio));
    if ratio_sum == 0 {
        return vec![0; ratios.len()];
    }

    let mut allocated = 0u16;
    ratios
        .iter()
        .enumerate()
        .map(|(index, ratio)| {
            if index + 1 == ratios.len() {
                return width.saturating_sub(allocated);
            }
            let item_width = u32::from(width) * u32::from(*ratio) / u32::from(ratio_sum.max(1));
            let item_width = u16::try_from(item_width).unwrap_or(0);
            allocated = allocated.saturating_add(item_width);
            item_width
        })
        .collect()
}
