//! Grok-style layout cache: cheap height estimates for the full transcript,
//! exact markdown layout only for the visible measurement window.
//!
//! Model mirrors `xai-grok-pager` `scrollback/state/layout.rs`:
//! - bulk load → all entries **estimated**
//! - `settle_visible` → exact measure viewport + below margin (no above-margin)
//! - measuring above is avoided so the top of the viewport stays anchored
//! - bottom-pinned follow re-pins after measure; resume warms pages above

use crate::translation::ChatItem;

use super::{
    build_segment_visual_lines, build_streaming_segment_with_commit, streaming_text_source,
    StreamCommitSnapshot, VisualLine,
};
use crate::translation::{find_runs, paint_mode_with_runs, PaintMode, VerbGroupRun};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};

/// Exact-measure this many entries below the last visible row so small scrolls
/// land on already-exact heights (Grok `MEASURE_MARGIN_ENTRIES`).
const MEASURE_MARGIN_ENTRIES: usize = 8;

/// Hard cap on markdown exact-measures per `prepare_layout` call. Prevents
/// estimate→exact cascade from freezing a frame on a tall tool/diff block.
const MAX_EXACT_PER_FRAME: usize = 6;

/// Even on-screen rows are amortized when many unmeasured items enter at once
/// (e.g. PageUp into history). Remaining blanks get filled on follow-up frames.
const MAX_VISIBLE_EXACT_PER_FRAME: usize = 8;

/// Settle loop iterations (height changes can reveal one more row at the edge).
const MAX_SETTLE_ITERS: usize = 2;

/// After a bottom-pinned rebuild, exact-measure at most this many entries
/// above the viewport (cheap resume warm — not full history).
const RESUME_WARM_ENTRIES: usize = 6;

/// Cap estimated body height so a huge unmeasured tool dump does not inflate
/// the scrollbar into nonsense before it is settled.
const MAX_ESTIMATE_BODY_LINES: usize = 48;

#[derive(Default)]
pub struct RenderCache {
    indexed_thread_id: Option<String>,
    /// Absolute start row of each item (`virtual_y`).
    item_starts: Vec<usize>,
    segments: Vec<CachedSegment>,
    /// Parallel to `segments`: exact markdown layout vs cheap estimate.
    measured: Vec<bool>,
    total_lines: usize,
    indexed_version: u64,
    indexed_structure_version: u64,
    indexed_width: u16,
    viewport_height: u16,
    /// True after a bottom-pinned rebuild until warm-up runs once.
    pending_bottom_warm: bool,
    /// More exact work remains (margin/warm); main loop should schedule a frame.
    needs_followup_frame: bool,
    /// Cached `find_runs` result — keyed by thread + structure + item count + expand set.
    cached_runs: Vec<VerbGroupRun>,
    runs_cache_thread_id: Option<String>,
    runs_cache_structure_version: u64,
    runs_cache_items_len: usize,
    runs_cache_expanded_hash: u64,
    runs_cache_valid: bool,
}

#[derive(Clone)]
struct CachedSegment {
    fingerprint: u64,
    start: usize,
    /// Exact visual lines when `measured`; empty when estimated.
    visual_lines: Vec<VisualLine>,
    /// Height in rows (exact or estimate).
    height: usize,
    stream_commit: Option<StreamCommitSnapshot>,
}

#[cfg(test)]
pub struct VisibleWindow<'a> {
    pub items: &'a [ChatItem],
    pub start_item_index: usize,
    pub line_offset_within_first_segment: usize,
}

/// Inputs for one layout pass (rebuild + settle + optional warm).
pub struct LayoutPass<'a> {
    pub thread_id: &'a str,
    pub items: &'a [ChatItem],
    pub version: u64,
    pub structure_version: u64,
    pub width: u16,
    pub verb_group_expanded: &'a HashSet<String>,
    pub viewport_height: u16,
    /// When true, pin to bottom after measure (Grok follow_mode).
    pub follow_mode: bool,
    pub scroll_offset: u32,
}

impl RenderCache {
    /// Rebuild if stale, settle exact heights for the viewport window, optionally
    /// warm pages above when bottom-pinned. Returns the scroll offset to use
    /// (may re-pin on follow or clamp after height changes).
    pub fn prepare_layout(&mut self, pass: LayoutPass<'_>) -> u32 {
        self.viewport_height = pass.viewport_height;
        self.needs_followup_frame = false;
        self.rebuild_if_stale(
            pass.thread_id,
            pass.items,
            pass.version,
            pass.structure_version,
            pass.width,
            pass.verb_group_expanded,
        );

        // Follow mode must settle the *bottom* window, not whatever scroll was
        // before total_lines was known (first frame often passes offset 0).
        let mut scroll = if pass.follow_mode {
            self.follow_scroll_to_bottom()
        } else {
            pass.scroll_offset
        };
        let mut budget = MAX_EXACT_PER_FRAME;
        scroll = self.settle_visible_measurements(
            pass.items,
            pass.width,
            pass.verb_group_expanded,
            pass.follow_mode,
            scroll,
            &mut budget,
        );

        if self.pending_bottom_warm && pass.follow_mode && budget > 0 {
            self.warm_pages_above(
                pass.items,
                pass.width,
                pass.verb_group_expanded,
                &mut budget,
            );
            // One-shot warm per rebuild — remaining above stays estimated until scroll.
            self.pending_bottom_warm = false;
            scroll = self.follow_scroll_to_bottom();
        }

        scroll
    }

    /// Rebuild cached segments when content/structure/width/thread changes.
    pub fn rebuild_if_stale(
        &mut self,
        thread_id: &str,
        items: &[ChatItem],
        version: u64,
        structure_version: u64,
        width: u16,
        verb_group_expanded: &HashSet<String>,
    ) {
        if self.is_valid(thread_id, version, width) {
            return;
        }
        let same_surface = self.indexed_thread_id.as_deref() == Some(thread_id)
            && self.indexed_width == width;

        if same_surface
            && self.indexed_structure_version == structure_version
            && self.segments.len() == items.len()
        {
            self.rebuild_dirty_segments(
                thread_id,
                items,
                width,
                structure_version,
                verb_group_expanded,
            );
        } else if same_surface
            && self.indexed_structure_version == structure_version
            && items.len() > self.segments.len()
            && items.len().saturating_sub(self.segments.len()) <= 4
        {
            self.append_new_segments(
                thread_id,
                items,
                width,
                structure_version,
                verb_group_expanded,
            );
        } else {
            let can_reuse = same_surface;
            self.rebuild_all(
                thread_id,
                items,
                width,
                can_reuse,
                structure_version,
                verb_group_expanded,
            );
            // Full rebuild while following: warm history above the bottom.
            self.pending_bottom_warm = true;
        }

        self.indexed_version = version;
        self.indexed_structure_version = structure_version;
        self.indexed_width = width;
        self.indexed_thread_id = Some(thread_id.to_owned());
    }

    /// Ensure `cached_runs` matches the current transcript structure / expand set.
    ///
    /// Keyed by thread + `structure_version` + `items.len()` + stable hash of expand
    /// ids so settle / measure / warm share one O(n) scan instead of re-scanning.
    fn ensure_runs_cached(
        &mut self,
        thread_id: &str,
        items: &[ChatItem],
        structure_version: u64,
        verb_group_expanded: &HashSet<String>,
    ) {
        let expanded_hash = hash_expanded_ids(verb_group_expanded);
        if self.runs_cache_valid
            && self.runs_cache_thread_id.as_deref() == Some(thread_id)
            && self.runs_cache_structure_version == structure_version
            && self.runs_cache_items_len == items.len()
            && self.runs_cache_expanded_hash == expanded_hash
        {
            return;
        }
        self.cached_runs = find_runs(items, verb_group_expanded);
        self.runs_cache_thread_id = Some(thread_id.to_owned());
        self.runs_cache_structure_version = structure_version;
        self.runs_cache_items_len = items.len();
        self.runs_cache_expanded_hash = expanded_hash;
        self.runs_cache_valid = true;
    }

    /// Snapshot of cached runs (cheap clone of small fold-group metadata).
    fn runs_snapshot(
        &mut self,
        thread_id: &str,
        items: &[ChatItem],
        structure_version: u64,
        verb_group_expanded: &HashSet<String>,
    ) -> Vec<VerbGroupRun> {
        self.ensure_runs_cached(thread_id, items, structure_version, verb_group_expanded);
        self.cached_runs.clone()
    }

    pub fn total_lines(&self) -> usize {
        self.total_lines
    }

    pub(crate) fn is_valid(&self, thread_id: &str, version: u64, width: u16) -> bool {
        self.indexed_thread_id.as_deref() == Some(thread_id)
            && self.indexed_version == version
            && self.indexed_width == width
    }

    pub fn needs_followup_frame(&self) -> bool {
        self.needs_followup_frame || self.pending_bottom_warm
    }

    #[cfg(test)]
    pub(crate) fn item_starts(&self) -> &[usize] {
        &self.item_starts
    }

    #[cfg(test)]
    pub(crate) fn is_measured(&self, index: usize) -> bool {
        self.measured.get(index).copied().unwrap_or(false)
    }

    #[cfg(test)]
    pub(crate) fn stream_commit_stable_len(&self, index: usize) -> Option<usize> {
        self.segments
            .get(index)
            .and_then(|s| s.stream_commit.as_ref())
            .map(|c| c.stable_source.len())
    }

    #[cfg(test)]
    pub(crate) fn segment_body_line_count(&self, index: usize) -> Option<usize> {
        self.segments
            .get(index)
            .and_then(|s| s.stream_commit.as_ref())
            .map(|c| c.body_visual_lines.len())
    }

    /// Viewport + below-margin exact measure. No above-margin (keeps top anchored).
    ///
    /// Pure-scroll fast path: if the window is already measured, return without
    /// touching runs (was O(n) every wheel tick — felt like no improvement).
    fn settle_visible_measurements(
        &mut self,
        items: &[ChatItem],
        width: u16,
        verb_group_expanded: &HashSet<String>,
        follow_mode: bool,
        mut scroll_offset: u32,
        budget: &mut usize,
    ) -> u32 {
        if self.viewport_height == 0 || items.is_empty() || self.segments.is_empty() {
            return scroll_offset;
        }

        // Shared across settle iters / measure_one — not recomputed per item.
        let mut runs: Option<Vec<VerbGroupRun>> = None;

        for _ in 0..MAX_SETTLE_ITERS {
            let Some((first_vis, last_vis, end_margin)) =
                self.measurement_window_parts(scroll_offset)
            else {
                break;
            };

            // Fast path: nothing to do — pure scroll stays O(1).
            let visible_needs = (first_vis..=last_vis).any(|idx| self.needs_measure(idx));
            let margin_needs = (last_vis.saturating_add(1)..=end_margin)
                .any(|idx| self.needs_measure(idx));
            if !visible_needs && !margin_needs {
                break;
            }

            // Only pay find_runs (or cache lookup) when we actually exact-measure.
            if runs.is_none() {
                let structure_version = self.indexed_structure_version;
                let thread_id = self
                    .indexed_thread_id
                    .clone()
                    .expect("thread_id set by rebuild_if_stale before settle");
                runs = Some(self.runs_snapshot(
                    &thread_id,
                    items,
                    structure_version,
                    verb_group_expanded,
                ));
            }
            let runs = runs.as_ref().expect("runs loaded above");

            // Visible rows first — amortized so PageUp into dense history cannot
            // freeze one frame measuring dozens of large tools.
            // Follow/bottom: measure high indices first so the live tail is exact.
            let mut measured_any = false;
            if visible_needs {
                let mut vis_budget = (*budget).min(MAX_VISIBLE_EXACT_PER_FRAME).max(1);
                let before = vis_budget;
                if follow_mode {
                    measured_any |= self.measure_range_exact_rev(
                        items,
                        width,
                        runs,
                        first_vis,
                        last_vis,
                        &mut vis_budget,
                    );
                } else {
                    measured_any |= self.measure_range_exact(
                        items,
                        width,
                        runs,
                        first_vis,
                        last_vis,
                        &mut vis_budget,
                        /*required=*/ false,
                    );
                }
                let spent = before.saturating_sub(vis_budget);
                *budget = budget.saturating_sub(spent);
                if (first_vis..=last_vis).any(|idx| self.needs_measure(idx)) {
                    self.needs_followup_frame = true;
                }
            }
            // Below-margin only with remaining budget.
            if margin_needs && *budget > 0 {
                measured_any |= self.measure_range_exact(
                    items,
                    width,
                    runs,
                    last_vis.saturating_add(1),
                    end_margin,
                    budget,
                    /*required=*/ false,
                );
                if (last_vis.saturating_add(1)..=end_margin).any(|idx| self.needs_measure(idx)) {
                    self.needs_followup_frame = true;
                }
            } else if margin_needs {
                self.needs_followup_frame = true;
            }

            if !measured_any {
                break;
            }
            self.recompute_virtual_y();
            if follow_mode {
                scroll_offset = self.follow_scroll_to_bottom();
            } else {
                let max_off = self
                    .total_lines
                    .saturating_sub(usize::from(self.viewport_height));
                scroll_offset = (scroll_offset as usize).min(max_off) as u32;
            }
            if *budget == 0 {
                self.needs_followup_frame = true;
                break;
            }
        }
        scroll_offset
    }

    fn needs_measure(&self, idx: usize) -> bool {
        self.measured.get(idx).is_some_and(|m| !m)
            && self.segments.get(idx).is_some_and(|s| s.height > 0)
    }

    /// Returns `(first_visible, last_visible, end_with_margin)`.
    fn measurement_window_parts(&self, scroll_offset: u32) -> Option<(usize, usize, usize)> {
        if self.item_starts.is_empty() {
            return None;
        }
        let base_row = scroll_offset as usize;
        let end_row = base_row.saturating_add(usize::from(self.viewport_height.max(1)));

        let first = self
            .item_starts
            .partition_point(|&y| y <= base_row)
            .saturating_sub(1)
            .min(self.segments.len().saturating_sub(1));
        let last_visible = self
            .item_starts
            .partition_point(|&y| y < end_row)
            .saturating_sub(1)
            .min(self.segments.len().saturating_sub(1))
            .max(first);
        let end = (last_visible + MEASURE_MARGIN_ENTRIES).min(self.segments.len().saturating_sub(1));
        Some((first, last_visible, end))
    }

    /// Exact-measure `[start, end]` low→high. Stops when budget hits 0.
    fn measure_range_exact(
        &mut self,
        items: &[ChatItem],
        width: u16,
        runs: &[VerbGroupRun],
        start: usize,
        end: usize,
        budget: &mut usize,
        required: bool,
    ) -> bool {
        let mut measured_any = false;
        for idx in start..=end {
            if !self.measure_one(items, width, runs, idx, budget, required) {
                if *budget == 0 && !required {
                    break;
                }
                continue;
            }
            measured_any = true;
        }
        measured_any
    }

    /// Exact-measure `[start, end]` high→low (prefer live tail when following).
    fn measure_range_exact_rev(
        &mut self,
        items: &[ChatItem],
        width: u16,
        runs: &[VerbGroupRun],
        start: usize,
        end: usize,
        budget: &mut usize,
    ) -> bool {
        let mut measured_any = false;
        for idx in (start..=end).rev() {
            if *budget == 0 {
                break;
            }
            if self.measure_one(items, width, runs, idx, budget, false) {
                measured_any = true;
            }
        }
        measured_any
    }

    /// Returns true if this index was newly exact-measured.
    fn measure_one(
        &mut self,
        items: &[ChatItem],
        width: u16,
        runs: &[VerbGroupRun],
        idx: usize,
        budget: &mut usize,
        required: bool,
    ) -> bool {
        if idx >= self.segments.len() || idx >= items.len() {
            return false;
        }
        if !self.needs_measure(idx) {
            if self.segments[idx].height == 0 && !self.measured[idx] {
                self.measured[idx] = true;
            }
            return false;
        }
        if !required && *budget == 0 {
            return false;
        }
        let prev = self.segments[idx].stream_commit.clone();
        let built = build_exact_segment(
            idx,
            &items[idx],
            items,
            width,
            runs,
            prev.as_ref(),
        );
        self.segments[idx].visual_lines = built.visual_lines;
        self.segments[idx].height = built.height;
        self.segments[idx].stream_commit = built.stream_commit;
        self.segments[idx].fingerprint = built.fingerprint;
        self.measured[idx] = true;
        if *budget > 0 {
            *budget -= 1;
        }
        true
    }

    fn warm_pages_above(
        &mut self,
        items: &[ChatItem],
        width: u16,
        verb_group_expanded: &HashSet<String>,
        budget: &mut usize,
    ) {
        if self.viewport_height == 0 || self.segments.is_empty() || *budget == 0 {
            return;
        }
        let scroll = self.follow_scroll_to_bottom();
        let Some((first_visible, _, _)) = self.measurement_window_parts(scroll) else {
            return;
        };
        if first_visible == 0 {
            return;
        }
        let start = first_visible.saturating_sub(RESUME_WARM_ENTRIES);
        let end = first_visible.saturating_sub(1);
        let structure_version = self.indexed_structure_version;
        let thread_id = self
            .indexed_thread_id
            .clone()
            .expect("thread_id set by rebuild_if_stale before warm");
        let runs = self.runs_snapshot(&thread_id, items, structure_version, verb_group_expanded);
        let _ = self.measure_range_exact(
            items,
            width,
            &runs,
            start,
            end,
            budget,
            /*required=*/ false,
        );
        if (start..=end).any(|idx| self.needs_measure(idx)) {
            self.needs_followup_frame = true;
        }
        self.recompute_virtual_y();
    }

    fn follow_scroll_to_bottom(&self) -> u32 {
        let max_off = self
            .total_lines
            .saturating_sub(usize::from(self.viewport_height.max(1)));
        u32::try_from(max_off).unwrap_or(u32::MAX)
    }

    fn rebuild_all(
        &mut self,
        thread_id: &str,
        items: &[ChatItem],
        width: u16,
        can_reuse: bool,
        structure_version: u64,
        verb_group_expanded: &HashSet<String>,
    ) {
        let previous = if can_reuse {
            std::mem::take(&mut self.segments)
        } else {
            Vec::new()
        };
        let prev_measured = if can_reuse {
            std::mem::take(&mut self.measured)
        } else {
            Vec::new()
        };
        let runs = self.runs_snapshot(thread_id, items, structure_version, verb_group_expanded);
        let mut segments = Vec::with_capacity(items.len());
        let mut measured = Vec::with_capacity(items.len());
        let mut saw_visible = false;

        for (idx, item) in items.iter().enumerate() {
            let fingerprint = item_fingerprint_with_runs(item, items, idx, &runs);
            let mode = paint_mode_with_runs(items, idx, &runs);
            let has_gap = saw_visible && !matches!(mode, PaintMode::Hidden);
            if !matches!(mode, PaintMode::Hidden) {
                saw_visible = true;
            }

            // Reuse exact segment only when fingerprint matches and was measured.
            if let Some(prev) = previous.get(idx) {
                if prev.fingerprint == fingerprint
                    && prev_measured.get(idx).copied().unwrap_or(false)
                    && !prev.visual_lines.is_empty()
                {
                    segments.push(CachedSegment {
                        start: 0,
                        ..prev.clone()
                    });
                    measured.push(true);
                    continue;
                }
            }

            // Streaming rows (almost always the live tail) are exact immediately.
            if item_is_streaming(item) && !matches!(mode, PaintMode::Hidden) {
                let built = build_exact_segment(
                    idx,
                    item,
                    items,
                    width,
                    &runs,
                    previous.get(idx).and_then(|p| p.stream_commit.as_ref()),
                );
                segments.push(built);
                measured.push(true);
                continue;
            }

            let height = estimate_height(item, mode, has_gap, width);
            segments.push(CachedSegment {
                fingerprint,
                start: 0,
                visual_lines: Vec::new(),
                height,
                stream_commit: None,
            });
            measured.push(false);
        }

        self.segments = segments;
        self.measured = measured;
        self.recompute_virtual_y();
    }

    fn rebuild_dirty_segments(
        &mut self,
        thread_id: &str,
        items: &[ChatItem],
        width: u16,
        structure_version: u64,
        verb_group_expanded: &HashSet<String>,
    ) {
        debug_assert_eq!(items.len(), self.segments.len());
        let runs = self.runs_snapshot(thread_id, items, structure_version, verb_group_expanded);
        let mut saw_visible = false;

        for (idx, item) in items.iter().enumerate() {
            let fingerprint = item_fingerprint_with_runs(item, items, idx, &runs);
            let mode = paint_mode_with_runs(items, idx, &runs);
            let has_gap = saw_visible && !matches!(mode, PaintMode::Hidden);
            if !matches!(mode, PaintMode::Hidden) {
                saw_visible = true;
            }

            if self.segments[idx].fingerprint == fingerprint {
                continue;
            }

            // Content changed. Streaming / already-exact / near-tail rows are
            // measured exactly (Grok live-append path). Older estimated history
            // only re-estimates until the viewport settles them.
            let near_tail = idx + MEASURE_MARGIN_ENTRIES >= items.len();
            let streaming = item_is_streaming(item);
            if self.measured[idx] || streaming || near_tail {
                let prev = self.segments[idx].stream_commit.clone();
                let built = build_exact_segment(
                    idx,
                    item,
                    items,
                    width,
                    &runs,
                    prev.as_ref(),
                );
                self.segments[idx] = built;
                self.measured[idx] = true;
            } else {
                self.segments[idx] = CachedSegment {
                    fingerprint,
                    start: 0,
                    visual_lines: Vec::new(),
                    height: estimate_height(item, mode, has_gap, width),
                    stream_commit: None,
                };
                self.measured[idx] = false;
            }
        }
        self.recompute_virtual_y();
    }

    fn append_new_segments(
        &mut self,
        thread_id: &str,
        items: &[ChatItem],
        width: u16,
        structure_version: u64,
        verb_group_expanded: &HashSet<String>,
    ) {
        let start_idx = self.segments.len();
        // Append changes item count → runs must refresh (key includes items.len()).
        let runs = self.runs_snapshot(thread_id, items, structure_version, verb_group_expanded);
        for (idx, item) in items.iter().enumerate().take(start_idx) {
            if self.segments[idx].fingerprint != item_fingerprint_with_runs(item, items, idx, &runs)
            {
                self.rebuild_all(
                    thread_id,
                    items,
                    width,
                    true,
                    structure_version,
                    verb_group_expanded,
                );
                return;
            }
        }

        let mut saw_visible = self.segments.iter().any(|s| s.height > 0);
        for (idx, item) in items.iter().enumerate().skip(start_idx) {
            let fingerprint = item_fingerprint_with_runs(item, items, idx, &runs);
            let mode = paint_mode_with_runs(items, idx, &runs);
            let has_gap = saw_visible && !matches!(mode, PaintMode::Hidden);
            if !matches!(mode, PaintMode::Hidden) {
                saw_visible = true;
            }
            // Live append at the bottom: measure exactly (Grok marks append measured).
            let built = build_exact_segment(idx, item, items, width, &runs, None);
            // Prefer exact; if hidden, height 0.
            let segment = if matches!(mode, PaintMode::Hidden) {
                CachedSegment {
                    fingerprint,
                    start: 0,
                    visual_lines: Vec::new(),
                    height: 0,
                    stream_commit: None,
                }
            } else {
                built
            };
            self.measured.push(true);
            self.segments.push(segment);
            let _ = has_gap;
        }
        self.recompute_virtual_y();
    }

    fn recompute_virtual_y(&mut self) {
        let mut item_starts = Vec::with_capacity(self.segments.len());
        let mut y = 0usize;
        for segment in &mut self.segments {
            item_starts.push(y);
            segment.start = y;
            y = y.saturating_add(segment.height);
        }
        self.item_starts = item_starts;
        self.total_lines = y;
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

    pub(crate) fn foldable_header_item_at_row(
        &self,
        items: &[ChatItem],
        absolute_row: usize,
        verb_group_expanded: &HashSet<String>,
    ) -> Option<usize> {
        if self.item_starts.is_empty() || self.item_starts.len() != items.len() {
            return None;
        }
        let idx = self
            .item_starts
            .partition_point(|&start| start <= absolute_row)
            .saturating_sub(1);
        if idx >= items.len() {
            return None;
        }

        let runs = find_runs(items, verb_group_expanded);
        let mode = paint_mode_with_runs(items, idx, &runs);
        let is_group_header = matches!(
            mode,
            PaintMode::CollapsedHeader | PaintMode::ExpandedHeader
        );
        if !items[idx].is_foldable() && !is_group_header {
            return None;
        }
        if matches!(mode, PaintMode::Hidden) {
            return None;
        }

        let segment_start = self.item_starts[idx];
        let has_prior_visible = (0..idx).any(|i| {
            !matches!(
                paint_mode_with_runs(items, i, &runs),
                PaintMode::Hidden
            )
        });
        let header_row = if has_prior_visible {
            segment_start.saturating_add(1)
        } else {
            segment_start
        };
        (absolute_row == header_row).then_some(idx)
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
        for (seg_i, segment) in self.segments[start_index..end_index].iter().enumerate() {
            let idx = start_index + seg_i;
            // Settle should have measured the viewport; fall back to blank rows
            // sized by estimate if something slipped through.
            if !self.measured.get(idx).copied().unwrap_or(false) || segment.visual_lines.is_empty()
            {
                let segment_start = segment.start;
                for row in 0..segment.height {
                    let absolute_row = segment_start + row;
                    if absolute_row < base_row {
                        continue;
                    }
                    if absolute_row >= end_row {
                        return out;
                    }
                    out.push(VisualLine {
                        line: ratatui::text::Line::from(""),
                        text: String::new(),
                    });
                    if out.len() >= height {
                        return out;
                    }
                }
                continue;
            }

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

fn build_exact_segment(
    idx: usize,
    item: &ChatItem,
    items: &[ChatItem],
    width: u16,
    runs: &[VerbGroupRun],
    previous_commit: Option<&StreamCommitSnapshot>,
) -> CachedSegment {
    let fingerprint = item_fingerprint_with_runs(item, items, idx, runs);
    let mode = paint_mode_with_runs(items, idx, runs);
    if matches!(mode, PaintMode::Hidden) {
        return CachedSegment {
            fingerprint,
            start: 0,
            visual_lines: Vec::new(),
            height: 0,
            stream_commit: None,
        };
    }

    let (visual_lines, stream_commit) = if streaming_text_source(item)
        .is_some_and(|(_, streaming)| streaming)
        && matches!(mode, PaintMode::Normal | PaintMode::ExpandedMember)
    {
        build_streaming_segment_with_commit(idx, item, items, width, runs, previous_commit)
    } else {
        (
            build_segment_visual_lines(idx, item, items, width, runs),
            None,
        )
    };

    let height = visual_lines.len();
    CachedSegment {
        fingerprint,
        start: 0,
        visual_lines,
        height,
        stream_commit,
    }
}

/// Stable hash of expand ids (sorted) so `HashSet` iteration order cannot miss cache hits.
fn hash_expanded_ids(expanded: &HashSet<String>) -> u64 {
    let mut ids: Vec<&str> = expanded.iter().map(String::as_str).collect();
    ids.sort_unstable();
    let mut hasher = DefaultHasher::new();
    ids.len().hash(&mut hasher);
    for id in ids {
        id.hash(&mut hasher);
    }
    hasher.finish()
}

fn estimate_height(item: &ChatItem, mode: PaintMode, has_gap: bool, width: u16) -> usize {
    if matches!(mode, PaintMode::Hidden) {
        return 0;
    }
    let gap = usize::from(has_gap);
    if matches!(mode, PaintMode::CollapsedHeader) {
        return gap + 1;
    }

    let (chars, newlines) = rough_text_metrics(item);
    let cols = usize::from(width.max(1));
    let wrapped = chars
        .saturating_add(newlines.saturating_mul(cols))
        / cols
        + newlines
        + 1;
    let body = wrapped.clamp(1, MAX_ESTIMATE_BODY_LINES);
    gap + body
}

fn rough_text_metrics(item: &ChatItem) -> (usize, usize) {
    match item {
        ChatItem::UserMessage { text_parts, .. } | ChatItem::AssistantText { text_parts, .. } => {
            let mut chars = 0usize;
            let mut newlines = 0usize;
            for part in text_parts {
                match part {
                    crate::translation::TextPart::Plain(t) => {
                        chars = chars.saturating_add(t.len());
                        newlines = newlines.saturating_add(t.bytes().filter(|&b| b == b'\n').count());
                    }
                    crate::translation::TextPart::Code { code, .. } => {
                        chars = chars.saturating_add(code.len());
                        newlines =
                            newlines.saturating_add(code.bytes().filter(|&b| b == b'\n').count());
                    }
                }
            }
            (chars, newlines)
        }
        ChatItem::Reasoning { text, .. }
        | ChatItem::SystemMessage { text }
        | ChatItem::Error { text, .. } => (
            text.len(),
            text.bytes().filter(|&b| b == b'\n').count(),
        ),
        ChatItem::ToolCall {
            args_summary,
            output_summary,
            args_detail,
            output_detail,
            is_expanded,
            is_user_toggled,
            ..
        } => {
            let expanded = is_user_toggled.unwrap_or(*is_expanded);
            if !expanded {
                return (args_summary.len().saturating_add(16), 0);
            }
            let mut chars = args_summary.len();
            let mut newlines = 0usize;
            if let Some(d) = args_detail {
                chars = chars.saturating_add(d.len());
                newlines = newlines.saturating_add(d.bytes().filter(|&b| b == b'\n').count());
            }
            if let Some(s) = output_summary {
                chars = chars.saturating_add(s.len());
            }
            if let Some(d) = output_detail {
                chars = chars.saturating_add(d.len());
                newlines = newlines.saturating_add(d.bytes().filter(|&b| b == b'\n').count());
            }
            (chars, newlines)
        }
        ChatItem::SubagentCall {
            prompt_summary,
            ..
        } => (prompt_summary.as_ref().map_or(24, String::len), 0),
    }
}

fn item_is_streaming(item: &ChatItem) -> bool {
    match item {
        ChatItem::UserMessage { is_streaming, .. }
        | ChatItem::AssistantText { is_streaming, .. }
        | ChatItem::Reasoning { is_streaming, .. }
        | ChatItem::ToolCall { is_streaming, .. }
        | ChatItem::SubagentCall { is_streaming, .. } => *is_streaming,
        ChatItem::SystemMessage { .. } | ChatItem::Error { .. } => false,
    }
}

fn item_fingerprint_with_runs(
    item: &ChatItem,
    items: &[ChatItem],
    idx: usize,
    runs: &[VerbGroupRun],
) -> u64 {
    let mut hasher = DefaultHasher::new();
    item.hash(&mut hasher);
    let mode = paint_mode_with_runs(items, idx, runs);
    mode.hash(&mut hasher);
    if let Some(run) = runs.iter().find(|r| r.contains(idx)) {
        run.start.hash(&mut hasher);
        run.end.hash(&mut hasher);
        run.expanded.hash(&mut hasher);
        run.members.hash(&mut hasher);
        for member in &items[run.start..run.end] {
            member.hash(&mut hasher);
        }
    }
    hasher.finish()
}
