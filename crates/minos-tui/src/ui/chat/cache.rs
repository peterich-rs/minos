use crate::translation::ChatItem;

use super::{build_segment_visual_lines, VisualLine};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Default)]
pub struct RenderCache {
    indexed_thread_id: Option<String>,
    item_starts: Vec<usize>,
    segments: Vec<CachedSegment>,
    total_lines: usize,
    indexed_version: u64,
    indexed_width: u16,
}

#[derive(Clone)]
struct CachedSegment {
    fingerprint: u64,
    start: usize,
    visual_lines: Vec<VisualLine>,
}

#[cfg(test)]
pub struct VisibleWindow<'a> {
    pub items: &'a [ChatItem],
    pub start_item_index: usize,
    pub line_offset_within_first_segment: usize,
}

impl RenderCache {
    pub fn rebuild_if_stale(
        &mut self,
        thread_id: &str,
        items: &[ChatItem],
        version: u64,
        width: u16,
    ) {
        if self.is_valid(thread_id, version, width) {
            return;
        }
        let can_reuse_segments =
            self.indexed_thread_id.as_deref() == Some(thread_id) && self.indexed_width == width;
        self.rebuild(items, width, can_reuse_segments);
        self.indexed_version = version;
        self.indexed_width = width;
        self.indexed_thread_id = Some(thread_id.to_owned());
    }

    pub fn total_lines(&self) -> usize {
        self.total_lines
    }

    pub(crate) fn is_valid(&self, thread_id: &str, version: u64, width: u16) -> bool {
        self.indexed_thread_id.as_deref() == Some(thread_id)
            && self.indexed_version == version
            && self.indexed_width == width
    }

    #[cfg(test)]
    pub(crate) fn item_starts(&self) -> &[usize] {
        self.item_starts.as_slice()
    }

    fn rebuild(&mut self, items: &[ChatItem], width: u16, can_reuse_segments: bool) {
        let previous = if can_reuse_segments {
            std::mem::take(&mut self.segments)
        } else {
            Vec::new()
        };
        let mut item_starts = Vec::with_capacity(items.len());
        let mut segments = Vec::with_capacity(items.len());
        let mut current_start = 0usize;

        for (idx, item) in items.iter().enumerate() {
            item_starts.push(current_start);
            let fingerprint = item_fingerprint(item);
            let mut segment = previous
                .get(idx)
                .filter(|segment| segment.fingerprint == fingerprint)
                .cloned()
                .unwrap_or_else(|| CachedSegment {
                    fingerprint,
                    start: current_start,
                    visual_lines: build_segment_visual_lines(idx, item, width),
                });
            segment.start = current_start;
            current_start += segment.visual_lines.len();
            segments.push(segment);
        }

        self.item_starts = item_starts;
        self.segments = segments;
        self.total_lines = current_start;
    }

    #[cfg(test)]
    pub fn visible_window<'a>(
        &self,
        items: &'a [ChatItem],
        base_row: usize,
        height: usize,
    ) -> VisibleWindow<'a> {
        if self.item_starts.is_empty() || items.is_empty() {
            return VisibleWindow {
                items: &[],
                start_item_index: 0,
                line_offset_within_first_segment: 0,
            };
        }

        let end_row = base_row + height;
        let start_item_index = self.item_starts.partition_point(|&start| start <= base_row);
        let start_item_index = start_item_index.saturating_sub(1);
        let end_item_index = self
            .item_starts
            .partition_point(|&start| start < end_row)
            .min(self.item_starts.len());

        let item_count = end_item_index.saturating_sub(start_item_index).max(1);
        let item_count = item_count.min(items.len().saturating_sub(start_item_index));
        let item_start_abs = self.item_starts[start_item_index];

        VisibleWindow {
            items: &items[start_item_index..start_item_index + item_count],
            start_item_index,
            line_offset_within_first_segment: base_row.saturating_sub(item_start_abs),
        }
    }

    pub(super) fn visible_visual_lines(&self, base_row: usize, height: usize) -> Vec<VisualLine> {
        if self.segments.is_empty() || height == 0 {
            return Vec::new();
        }

        let end_row = base_row.saturating_add(height);
        let start_index = self.item_starts.partition_point(|&start| start <= base_row);
        let start_index = start_index.saturating_sub(1);
        let end_index = self
            .item_starts
            .partition_point(|&start| start < end_row)
            .min(self.segments.len());
        let end_index = end_index.max(start_index + 1).min(self.segments.len());

        let mut out = Vec::with_capacity(height);
        for segment in &self.segments[start_index..end_index] {
            let segment_start = segment.start;
            for (line_index, line) in segment.visual_lines.iter().enumerate() {
                let absolute_row = segment_start + line_index;
                if absolute_row < base_row {
                    continue;
                }
                if absolute_row >= end_row {
                    return out;
                }
                out.push(line.clone());
                if out.len() >= height {
                    return out;
                }
            }
        }
        out
    }
}

fn item_fingerprint(item: &ChatItem) -> u64 {
    let mut hasher = DefaultHasher::new();
    item.hash(&mut hasher);
    hasher.finish()
}
