use paneflow_config::schema::TabSession;

use super::Workspace;

impl Workspace {
    pub fn parent_tab_index(&self, index: usize) -> Option<usize> {
        let parent = self.tabs.get(index)?.parent_id?;
        self.tabs.iter().position(|tab| tab.id == parent)
    }

    pub fn tab_depth(&self, mut index: usize) -> usize {
        let mut depth = 0;
        while let Some(parent) = self.parent_tab_index(index) {
            depth += 1;
            if depth >= self.tabs.len() {
                break;
            }
            index = parent;
        }
        depth
    }

    pub fn tab_has_children(&self, index: usize) -> bool {
        self.tabs
            .get(index)
            .is_some_and(|parent| self.tabs.iter().any(|tab| tab.parent_id == Some(parent.id)))
    }

    pub fn reveal_tab_ancestors(&mut self, mut index: usize) {
        for _ in 0..self.tabs.len() {
            let Some(parent) = self.parent_tab_index(index) else {
                break;
            };
            self.tabs[parent].children_collapsed = false;
            index = parent;
        }
    }

    pub fn restore_tab_hierarchy(&mut self, saved: &[TabSession]) {
        for (index, session) in saved.iter().take(self.tabs.len()).enumerate() {
            let parent = session
                .parent_tab
                .filter(|parent| *parent != index)
                .and_then(|parent| self.tabs.get(parent))
                .map(|tab| tab.id);
            self.tabs[index].parent_id = parent;
            self.tabs[index].children_collapsed = session.children_collapsed;
        }
        for index in 0..self.tabs.len() {
            let mut visited = std::collections::HashSet::new();
            let mut cursor = Some(index);
            while let Some(current) = cursor {
                if !visited.insert(current) {
                    self.tabs[index].parent_id = None;
                    break;
                }
                cursor = self.parent_tab_index(current);
            }
        }
        self.reveal_tab_ancestors(self.active_tab_idx());
    }

    pub fn tab_tree_order(&self, reveal_collapsed: bool) -> Vec<usize> {
        fn visit(
            ws: &Workspace,
            index: usize,
            hidden: bool,
            reveal: bool,
            seen: &mut [bool],
            rows: &mut Vec<usize>,
        ) {
            if seen[index] {
                return;
            }
            seen[index] = true;
            if !hidden {
                rows.push(index);
            }
            let hidden = hidden || (!reveal && ws.tabs[index].children_collapsed);
            for child in 0..ws.tabs.len() {
                if ws.tabs[child].parent_id == Some(ws.tabs[index].id) {
                    visit(ws, child, hidden, reveal, seen, rows);
                }
            }
        }
        let mut seen = vec![false; self.tabs.len()];
        let mut rows = Vec::with_capacity(self.tabs.len());
        for index in 0..self.tabs.len() {
            if self.parent_tab_index(index).is_none() {
                visit(self, index, false, reveal_collapsed, &mut seen, &mut rows);
            }
        }
        for index in 0..self.tabs.len() {
            if !seen[index] {
                visit(self, index, false, reveal_collapsed, &mut seen, &mut rows);
            }
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Tab;

    fn workspace() -> Workspace {
        Workspace::restored_with_id(
            1,
            "team",
            std::path::PathBuf::new(),
            (0..4)
                .map(|i| Tab::new(format!("agent-{i}"), None))
                .collect(),
            0,
        )
    }

    #[test]
    fn children_follow_their_parent_after_reordering_and_respect_folding() {
        let mut ws = workspace();
        ws.tabs[2].parent_id = Some(ws.tabs[0].id);
        ws.tabs[3].parent_id = Some(ws.tabs[2].id);
        assert_eq!(ws.tab_tree_order(false), [0, 2, 3, 1]);
        assert_eq!(ws.tab_depth(3), 2);
        ws.tabs[0].children_collapsed = true;
        assert_eq!(ws.tab_tree_order(false), [0, 1]);
        assert_eq!(ws.tab_tree_order(true), [0, 2, 3, 1]);
        ws.set_active_tab(3);
        assert!(!ws.tabs[0].children_collapsed);
        ws.reorder_tab(0, 3);
        assert_eq!(ws.tab_tree_order(false), [0, 3, 1, 2]);
    }

    #[test]
    fn closing_or_moving_a_parent_keeps_workers_and_promotes_them() {
        let mut ws = workspace();
        ws.tabs[1].parent_id = Some(ws.tabs[0].id);
        ws.tabs[2].parent_id = Some(ws.tabs[1].id);
        let root = ws.tabs[0].id;
        let removed = ws.close_tab(1).unwrap();
        assert_eq!(ws.tabs[1].parent_id, Some(root));
        let mut dest = workspace();
        assert!(dest.open_tab(removed));
        assert_eq!(dest.active_tab().parent_id, None);
    }

    #[test]
    fn restoring_rebinds_ephemeral_ids_and_keeps_bad_references_visible() {
        let mut ws = workspace();
        let saved = [None, Some(0), Some(1), Some(99)].map(|parent_tab| TabSession {
            parent_tab,
            ..TabSession::default()
        });
        ws.restore_tab_hierarchy(&saved);
        assert_eq!(ws.parent_tab_index(2), Some(1));
        assert_eq!(ws.tab_depth(2), 2);
        assert_eq!(ws.parent_tab_index(3), None);
        let cyclic = [Some(1), Some(0), Some(2), Some(99)].map(|parent_tab| TabSession {
            parent_tab,
            ..TabSession::default()
        });
        ws.restore_tab_hierarchy(&cyclic);
        assert_eq!(ws.tab_tree_order(false).len(), 4);
        assert!((0..4).all(|index| ws.tab_depth(index) < 4));
    }
}
