use ratatui::layout::Rect;

/// Direction of a split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    /// Left | Right
    Horizontal,
    /// Top / Bottom
    Vertical,
}

/// A node in the layout tree.
#[derive(Debug, Clone)]
pub enum LayoutNode {
    /// A leaf pane.
    Leaf { pane_id: String },
    /// A split containing two children.
    Split {
        direction: SplitDirection,
        /// Ratio of first child's share (0.0..1.0). Default 0.5.
        ratio: f32,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

/// Placeholder used during in-place node replacement via `std::mem::replace`.
const PLACEHOLDER: LayoutNode = LayoutNode::Leaf {
    pane_id: String::new(),
};

/// A layout tree managing the arrangement of panes.
#[derive(Debug, Clone)]
pub struct LayoutTree {
    root: LayoutNode,
    focused_pane: String,
}

impl LayoutNode {
    /// Count leaves in this subtree.
    fn leaf_count(&self) -> usize {
        match self {
            LayoutNode::Leaf { .. } => 1,
            LayoutNode::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }

    /// Collect all pane IDs in depth-first order.
    fn collect_pane_ids(&self, out: &mut Vec<String>) {
        match self {
            LayoutNode::Leaf { pane_id } => out.push(pane_id.clone()),
            LayoutNode::Split { first, second, .. } => {
                first.collect_pane_ids(out);
                second.collect_pane_ids(out);
            }
        }
    }

    /// Check if this subtree contains a pane.
    fn contains(&self, target: &str) -> bool {
        match self {
            LayoutNode::Leaf { pane_id } => pane_id == target,
            LayoutNode::Split { first, second, .. } => {
                first.contains(target) || second.contains(target)
            }
        }
    }

    /// Calculate areas recursively, splitting along the primary axis.
    fn calculate_areas(&self, area: Rect, out: &mut Vec<(String, Rect)>) {
        match self {
            LayoutNode::Leaf { pane_id } => {
                out.push((pane_id.clone(), area));
            }
            LayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let (first_area, second_area) = split_rect(area, *direction, *ratio);
                first.calculate_areas(first_area, out);
                second.calculate_areas(second_area, out);
            }
        }
    }

    /// Split a specific pane by ID, returning true if found and split.
    fn split_pane(
        &mut self,
        target_pane_id: &str,
        direction: SplitDirection,
        new_pane_id: String,
    ) -> bool {
        match self {
            LayoutNode::Leaf { pane_id } if pane_id == target_pane_id => {
                let original = std::mem::replace(self, PLACEHOLDER);
                *self = LayoutNode::Split {
                    direction,
                    ratio: 0.5,
                    first: Box::new(original),
                    second: Box::new(LayoutNode::Leaf {
                        pane_id: new_pane_id,
                    }),
                };
                true
            }
            LayoutNode::Split { first, second, .. } => {
                first.split_pane(target_pane_id, direction, new_pane_id.clone())
                    || second.split_pane(target_pane_id, direction, new_pane_id)
            }
            _ => false,
        }
    }

    /// Remove a pane by ID. Returns true if found and removed.
    /// When a direct child is removed, the sibling is promoted in place.
    fn remove_pane(&mut self, target_pane_id: &str) -> bool {
        let LayoutNode::Split { first, second, .. } = self else {
            return false;
        };

        // Direct child removal: promote sibling in place
        if matches!(first.as_ref(), LayoutNode::Leaf { pane_id } if pane_id == target_pane_id) {
            *self = std::mem::replace(second, PLACEHOLDER);
            return true;
        }
        if matches!(second.as_ref(), LayoutNode::Leaf { pane_id } if pane_id == target_pane_id) {
            *self = std::mem::replace(first, PLACEHOLDER);
            return true;
        }

        // Recurse into children
        first.remove_pane(target_pane_id) || second.remove_pane(target_pane_id)
    }

    /// Adjust the ratio of the split that directly contains the given pane.
    fn adjust_ratio_for_pane(&mut self, pane_id: &str, delta: f32) -> bool {
        let LayoutNode::Split {
            ratio,
            first,
            second,
            ..
        } = self
        else {
            return false;
        };

        let is_direct_child = first.is_leaf(pane_id) || second.is_leaf(pane_id);
        if is_direct_child {
            *ratio = (*ratio + delta).clamp(0.1, 0.9);
            return true;
        }
        first.adjust_ratio_for_pane(pane_id, delta) || second.adjust_ratio_for_pane(pane_id, delta)
    }

    /// Check if this node is a leaf with the given pane ID.
    fn is_leaf(&self, target: &str) -> bool {
        matches!(self, LayoutNode::Leaf { pane_id } if pane_id == target)
    }
}

/// Split a `Rect` into two along the given direction at the given ratio.
fn split_rect(area: Rect, direction: SplitDirection, ratio: f32) -> (Rect, Rect) {
    match direction {
        SplitDirection::Horizontal => {
            let w = (area.width as f32 * ratio) as u16;
            (
                Rect::new(area.x, area.y, w, area.height),
                Rect::new(
                    area.x + w,
                    area.y,
                    area.width.saturating_sub(w),
                    area.height,
                ),
            )
        }
        SplitDirection::Vertical => {
            let h = (area.height as f32 * ratio) as u16;
            (
                Rect::new(area.x, area.y, area.width, h),
                Rect::new(
                    area.x,
                    area.y + h,
                    area.width,
                    area.height.saturating_sub(h),
                ),
            )
        }
    }
}

// --- LayoutTree ---

impl LayoutTree {
    /// Create a new tree with a single pane.
    pub fn new(pane_id: impl Into<String>) -> Self {
        let id = pane_id.into();
        Self {
            root: LayoutNode::Leaf {
                pane_id: id.clone(),
            },
            focused_pane: id,
        }
    }

    /// Split the focused pane, creating a new pane alongside it.
    /// The focused pane stays in the `first` position.
    pub fn split(&mut self, direction: SplitDirection, new_pane_id: impl Into<String>) {
        let new_id = new_pane_id.into();
        self.root.split_pane(&self.focused_pane, direction, new_id);
    }

    /// Remove a pane by ID. If it's a leaf in a split, promote the sibling.
    /// Returns true if the pane was found and removed.
    pub fn remove(&mut self, pane_id: &str) -> bool {
        if self.pane_count() <= 1 {
            return false;
        }
        if !self.root.remove_pane(pane_id) {
            return false;
        }
        // If the removed pane was focused, move focus to first available pane
        if self.focused_pane == pane_id {
            if let Some(first_id) = self.pane_ids().first() {
                self.focused_pane = first_id.clone();
            }
        }
        true
    }

    /// Calculate the Rect for each pane given the total area.
    pub fn calculate_areas(&self, area: Rect) -> Vec<(String, Rect)> {
        let mut out = Vec::new();
        self.root.calculate_areas(area, &mut out);
        out
    }

    /// Get the currently focused pane ID.
    pub fn focused_pane(&self) -> &str {
        &self.focused_pane
    }

    /// Set focus to a specific pane.
    pub fn set_focus(&mut self, pane_id: &str) -> bool {
        if self.root.contains(pane_id) {
            self.focused_pane = pane_id.to_string();
            true
        } else {
            false
        }
    }

    /// Move focus to the next pane (depth-first order).
    pub fn focus_next(&mut self) {
        let ids = self.pane_ids();
        if let Some(pos) = ids.iter().position(|id| id == &self.focused_pane) {
            let next = (pos + 1) % ids.len();
            self.focused_pane = ids[next].clone();
        }
    }

    /// Move focus to the previous pane.
    pub fn focus_prev(&mut self) {
        let ids = self.pane_ids();
        if let Some(pos) = ids.iter().position(|id| id == &self.focused_pane) {
            let prev = if pos == 0 { ids.len() - 1 } else { pos - 1 };
            self.focused_pane = ids[prev].clone();
        }
    }

    /// Move focus in a direction (for arrow key navigation).
    /// `direction` indicates horizontal or vertical movement.
    /// `first` means move toward the first child (left/up), otherwise second (right/down).
    pub fn focus_direction(&mut self, direction: SplitDirection, first: bool) {
        let areas = self.calculate_areas(Rect::new(0, 0, 1000, 1000));
        let current_area = areas
            .iter()
            .find(|(id, _)| id == &self.focused_pane)
            .map(|(_, r)| *r);

        let Some(current) = current_area else {
            return;
        };

        // Find the nearest pane in the given direction
        let mut best: Option<(&str, u32)> = None;

        for (id, rect) in &areas {
            if id == &self.focused_pane {
                continue;
            }

            let candidate_dist = match (direction, first) {
                // Left
                (SplitDirection::Horizontal, true) if rects_overlap_v(rect, &current) => {
                    (current.x as u32).checked_sub(rect.x as u32 + rect.width as u32)
                }
                // Right
                (SplitDirection::Horizontal, false) if rects_overlap_v(rect, &current) => {
                    (rect.x as u32).checked_sub(current.x as u32 + current.width as u32)
                }
                // Up
                (SplitDirection::Vertical, true) if rects_overlap_h(rect, &current) => {
                    (current.y as u32).checked_sub(rect.y as u32 + rect.height as u32)
                }
                // Down
                (SplitDirection::Vertical, false) if rects_overlap_h(rect, &current) => {
                    (rect.y as u32).checked_sub(current.y as u32 + current.height as u32)
                }
                _ => None,
            };

            if let Some(dist) = candidate_dist {
                if best.is_none_or(|(_, d)| dist < d) {
                    best = Some((id.as_str(), dist));
                }
            }
        }

        if let Some((id, _)) = best {
            self.focused_pane = id.to_string();
        }
    }

    /// Count the number of panes (leaves).
    pub fn pane_count(&self) -> usize {
        self.root.leaf_count()
    }

    /// Check if a pane exists in the tree.
    pub fn contains(&self, pane_id: &str) -> bool {
        self.root.contains(pane_id)
    }

    /// Get all pane IDs in depth-first order.
    pub fn pane_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        self.root.collect_pane_ids(&mut ids);
        ids
    }

    /// Adjust the ratio of the split containing the focused pane.
    pub fn adjust_ratio(&mut self, delta: f32) {
        let focused = self.focused_pane.clone();
        self.root.adjust_ratio_for_pane(&focused, delta);
    }
}

/// Check if two rects overlap vertically (share some y range).
fn rects_overlap_v(a: &Rect, b: &Rect) -> bool {
    a.y < b.y + b.height && b.y < a.y + a.height
}

/// Check if two rects overlap horizontally (share some x range).
fn rects_overlap_h(a: &Rect, b: &Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Construction ---

    #[test]
    fn test_new_single_pane() {
        let tree = LayoutTree::new("pane1");
        assert_eq!(tree.focused_pane(), "pane1");
        assert!(tree.contains("pane1"));
    }

    #[test]
    fn test_pane_count_single() {
        let tree = LayoutTree::new("pane1");
        assert_eq!(tree.pane_count(), 1);
    }

    // --- Splitting ---

    #[test]
    fn test_split_horizontal_creates_two_panes() {
        let mut tree = LayoutTree::new("pane1");
        tree.split(SplitDirection::Horizontal, "pane2");
        assert_eq!(tree.pane_count(), 2);
        assert!(tree.contains("pane1"));
        assert!(tree.contains("pane2"));
    }

    #[test]
    fn test_split_vertical_creates_two_panes() {
        let mut tree = LayoutTree::new("pane1");
        tree.split(SplitDirection::Vertical, "pane2");
        assert_eq!(tree.pane_count(), 2);
        assert!(tree.contains("pane1"));
        assert!(tree.contains("pane2"));
    }

    #[test]
    fn test_split_preserves_focus_on_original() {
        let mut tree = LayoutTree::new("pane1");
        tree.split(SplitDirection::Horizontal, "pane2");
        assert_eq!(tree.focused_pane(), "pane1");
    }

    #[test]
    fn test_nested_splits() {
        let mut tree = LayoutTree::new("pane1");
        tree.split(SplitDirection::Horizontal, "pane2");
        tree.split(SplitDirection::Vertical, "pane3");
        assert_eq!(tree.pane_count(), 3);
        assert!(tree.contains("pane1"));
        assert!(tree.contains("pane2"));
        assert!(tree.contains("pane3"));
    }

    // --- Removal ---

    #[test]
    fn test_remove_pane_promotes_sibling() {
        let mut tree = LayoutTree::new("pane1");
        tree.split(SplitDirection::Horizontal, "pane2");
        assert!(tree.remove("pane2"));
        assert_eq!(tree.pane_count(), 1);
        assert!(tree.contains("pane1"));
        assert!(!tree.contains("pane2"));
    }

    #[test]
    fn test_remove_nonexistent_returns_false() {
        let mut tree = LayoutTree::new("pane1");
        tree.split(SplitDirection::Horizontal, "pane2");
        assert!(!tree.remove("pane_missing"));
    }

    #[test]
    fn test_remove_last_pane_returns_false() {
        let mut tree = LayoutTree::new("pane1");
        assert!(!tree.remove("pane1"));
        assert_eq!(tree.pane_count(), 1);
    }

    // --- Area calculation ---

    #[test]
    fn test_single_pane_gets_full_area() {
        let tree = LayoutTree::new("pane1");
        let area = Rect::new(0, 0, 100, 50);
        let areas = tree.calculate_areas(area);
        assert_eq!(areas.len(), 1);
        assert_eq!(areas[0], ("pane1".to_string(), area));
    }

    #[test]
    fn test_horizontal_split_divides_width() {
        let mut tree = LayoutTree::new("pane1");
        tree.split(SplitDirection::Horizontal, "pane2");
        let areas = tree.calculate_areas(Rect::new(0, 0, 100, 50));
        assert_eq!(areas.len(), 2);
        // Default ratio 0.5: first gets width 50, second gets width 50
        assert_eq!(areas[0].1.width, 50);
        assert_eq!(areas[1].1.width, 50);
        assert_eq!(areas[0].1.height, 50);
        assert_eq!(areas[1].1.height, 50);
    }

    #[test]
    fn test_vertical_split_divides_height() {
        let mut tree = LayoutTree::new("pane1");
        tree.split(SplitDirection::Vertical, "pane2");
        let areas = tree.calculate_areas(Rect::new(0, 0, 100, 50));
        assert_eq!(areas.len(), 2);
        assert_eq!(areas[0].1.height, 25);
        assert_eq!(areas[1].1.height, 25);
        assert_eq!(areas[0].1.width, 100);
        assert_eq!(areas[1].1.width, 100);
    }

    #[test]
    fn test_nested_split_areas() {
        let mut tree = LayoutTree::new("pane1");
        tree.split(SplitDirection::Horizontal, "pane2");
        // Focus pane1, split vertically
        tree.set_focus("pane1");
        tree.split(SplitDirection::Vertical, "pane3");
        let areas = tree.calculate_areas(Rect::new(0, 0, 100, 50));
        assert_eq!(areas.len(), 3);
        // pane1 and pane3 share the left half vertically
        // pane2 occupies the right half
        assert_eq!(areas[0].0, "pane1");
        assert_eq!(areas[0].1, Rect::new(0, 0, 50, 25));
        assert_eq!(areas[1].0, "pane3");
        assert_eq!(areas[1].1, Rect::new(0, 25, 50, 25));
        assert_eq!(areas[2].0, "pane2");
        assert_eq!(areas[2].1, Rect::new(50, 0, 50, 50));
    }

    #[test]
    fn test_area_sum_equals_total() {
        let mut tree = LayoutTree::new("p1");
        tree.split(SplitDirection::Horizontal, "p2");
        tree.set_focus("p1");
        tree.split(SplitDirection::Vertical, "p3");
        tree.set_focus("p2");
        tree.split(SplitDirection::Vertical, "p4");

        let total = Rect::new(0, 0, 200, 100);
        let areas = tree.calculate_areas(total);

        let total_pixels: u32 = areas
            .iter()
            .map(|(_, r)| r.width as u32 * r.height as u32)
            .sum();
        assert_eq!(total_pixels, total.width as u32 * total.height as u32);
    }

    // --- Focus navigation ---

    #[test]
    fn test_focus_next_cycles() {
        let mut tree = LayoutTree::new("pane1");
        tree.split(SplitDirection::Horizontal, "pane2");
        tree.split(SplitDirection::Vertical, "pane3");
        // Focus is on pane1
        assert_eq!(tree.focused_pane(), "pane1");
        tree.focus_next();
        assert_eq!(tree.focused_pane(), "pane3");
        tree.focus_next();
        assert_eq!(tree.focused_pane(), "pane2");
        tree.focus_next();
        // Cycles back
        assert_eq!(tree.focused_pane(), "pane1");
    }

    #[test]
    fn test_focus_prev_cycles() {
        let mut tree = LayoutTree::new("pane1");
        tree.split(SplitDirection::Horizontal, "pane2");
        assert_eq!(tree.focused_pane(), "pane1");
        tree.focus_prev();
        assert_eq!(tree.focused_pane(), "pane2");
        tree.focus_prev();
        assert_eq!(tree.focused_pane(), "pane1");
    }

    #[test]
    fn test_set_focus() {
        let mut tree = LayoutTree::new("pane1");
        tree.split(SplitDirection::Horizontal, "pane2");
        assert!(tree.set_focus("pane2"));
        assert_eq!(tree.focused_pane(), "pane2");
    }

    #[test]
    fn test_focus_nonexistent_returns_false() {
        let mut tree = LayoutTree::new("pane1");
        assert!(!tree.set_focus("nonexistent"));
        assert_eq!(tree.focused_pane(), "pane1");
    }

    // --- Contains ---

    #[test]
    fn test_contains_existing_pane() {
        let mut tree = LayoutTree::new("pane1");
        tree.split(SplitDirection::Horizontal, "pane2");
        assert!(tree.contains("pane1"));
        assert!(tree.contains("pane2"));
    }

    #[test]
    fn test_contains_missing_pane() {
        let tree = LayoutTree::new("pane1");
        assert!(!tree.contains("pane99"));
    }

    // --- Pane IDs ---

    #[test]
    fn test_pane_ids_depth_first_order() {
        let mut tree = LayoutTree::new("pane1");
        tree.split(SplitDirection::Horizontal, "pane2");
        tree.set_focus("pane1");
        tree.split(SplitDirection::Vertical, "pane3");
        // Tree structure: Split(H, Split(V, pane1, pane3), pane2)
        let ids = tree.pane_ids();
        assert_eq!(ids, vec!["pane1", "pane3", "pane2"]);
    }

    // --- Ratio adjustment ---

    #[test]
    fn test_adjust_ratio_increases() {
        let mut tree = LayoutTree::new("pane1");
        tree.split(SplitDirection::Horizontal, "pane2");
        // Default ratio is 0.5
        tree.adjust_ratio(0.1);
        let areas = tree.calculate_areas(Rect::new(0, 0, 100, 50));
        // First pane should now be wider (ratio ~0.6)
        assert!(areas[0].1.width > 50);
    }

    #[test]
    fn test_adjust_ratio_clamps_to_bounds() {
        let mut tree = LayoutTree::new("pane1");
        tree.split(SplitDirection::Horizontal, "pane2");
        // Try to push ratio way past 0.9
        tree.adjust_ratio(10.0);
        let areas = tree.calculate_areas(Rect::new(0, 0, 100, 50));
        // Should be clamped — first pane width = (100 * 0.9) = 90
        assert_eq!(areas[0].1.width, 90);

        // Try to push ratio way below 0.1
        tree.adjust_ratio(-20.0);
        let areas = tree.calculate_areas(Rect::new(0, 0, 100, 50));
        // Should be clamped — first pane width = (100 * 0.1) = 10
        assert_eq!(areas[0].1.width, 10);
    }
}
