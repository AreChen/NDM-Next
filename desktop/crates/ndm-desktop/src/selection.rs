use ndm_core::TaskId;
use std::collections::HashSet;

/// The anchor is an ID rather than an index: sorting never changes its meaning.
pub(crate) fn select(
    selected: &mut HashSet<TaskId>,
    anchor: &mut Option<TaskId>,
    visible: &[TaskId],
    clicked: TaskId,
    shift: bool,
) {
    let Some(end) = visible.iter().position(|id| *id == clicked) else {
        return;
    };
    if shift && let Some(start) = anchor.and_then(|id| visible.iter().position(|item| *item == id))
    {
        selected.clear();
        selected.extend(&visible[start.min(end)..=start.max(end)]);
        return;
    }
    *anchor = Some(clicked);
    if !selected.insert(clicked) {
        selected.remove(&clicked);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_range_uses_current_filtered_order_in_both_directions() {
        let ids: Vec<_> = (0..6).map(|_| TaskId::new_v4()).collect();
        let visible = [ids[4], ids[1], ids[5], ids[0]];
        let mut selected = HashSet::new();
        let mut anchor = None;
        select(&mut selected, &mut anchor, &visible, ids[0], true);
        select(&mut selected, &mut anchor, &visible, ids[1], true);
        assert_eq!(selected, HashSet::from([ids[0], ids[1], ids[5]]));
        assert_eq!(anchor, Some(ids[0]));
        // Reorder after anchoring: never use the stale ordinal or select hidden IDs.
        select(&mut selected, &mut anchor, &[ids[0], ids[4]], ids[4], true);
        assert_eq!(selected, HashSet::from([ids[0], ids[4]]));
        assert!(!selected.contains(&ids[2]));
        assert!(!selected.contains(&ids[3]));
    }

    #[test]
    fn missing_anchor_falls_back_and_stale_click_is_ignored() {
        let a = TaskId::new_v4();
        let b = TaskId::new_v4();
        let mut selected = HashSet::new();
        let mut anchor = Some(a);
        select(&mut selected, &mut anchor, &[b], b, true);
        assert_eq!(selected, HashSet::from([b]));
        assert_eq!(anchor, Some(b));
        select(&mut selected, &mut anchor, &[b], a, true);
        assert_eq!(anchor, Some(b));
        assert_eq!(selected, HashSet::from([b]));
        select(&mut selected, &mut anchor, &[b], b, false);
        assert!(selected.is_empty());
    }
}
