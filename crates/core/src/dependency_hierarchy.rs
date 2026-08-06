//! Presentation-only hierarchy for graph nodes.
//!
//! Callers provide already-resolved group segments. This keeps package naming
//! rules in the language adapter while giving every renderer the same stable,
//! language-neutral hierarchy contract.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DependencyHierarchyMember {
    pub node_id: String,
    pub group_segments: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DependencyHierarchyGroup {
    pub id: String,
    pub name: String,
    pub qualified_name: String,
    pub parent: Option<String>,
    pub direct_modules: Vec<String>,
    pub descendants: Vec<String>,
}

pub fn build_dependency_hierarchy(
    members: &[DependencyHierarchyMember],
) -> Vec<DependencyHierarchyGroup> {
    let mut paths = BTreeSet::new();
    for member in members {
        for depth in 1..=member.group_segments.len() {
            paths.insert(member.group_segments[..depth].to_vec());
        }
    }
    let ids = paths
        .iter()
        .enumerate()
        .map(|(index, path)| (path.clone(), format!("g{index:04}")))
        .collect::<BTreeMap<_, _>>();

    paths
        .into_iter()
        .map(|path| {
            let direct_modules = members
                .iter()
                .filter(|member| member.group_segments == path)
                .map(|member| member.node_id.clone())
                .collect();
            let descendants = members
                .iter()
                .filter(|member| member.group_segments.starts_with(&path))
                .map(|member| member.node_id.clone())
                .collect();
            DependencyHierarchyGroup {
                id: ids[&path].clone(),
                name: path.last().cloned().unwrap_or_default(),
                qualified_name: path.join("."),
                parent: path
                    .split_last()
                    .and_then(|(_, parent)| ids.get(parent).cloned()),
                direct_modules,
                descendants,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_members_by_resolved_segments() {
        let groups = build_dependency_hierarchy(&[
            DependencyHierarchyMember {
                node_id: "shop-init".to_owned(),
                group_segments: vec!["shop".to_owned()],
            },
            DependencyHierarchyMember {
                node_id: "api".to_owned(),
                group_segments: vec!["shop".to_owned()],
            },
            DependencyHierarchyMember {
                node_id: "admin-init".to_owned(),
                group_segments: vec!["shop".to_owned(), "admin".to_owned()],
            },
            DependencyHierarchyMember {
                node_id: "users".to_owned(),
                group_segments: vec!["shop".to_owned(), "admin".to_owned()],
            },
        ]);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].qualified_name, "shop");
        assert_eq!(groups[0].direct_modules, ["shop-init", "api"]);
        assert_eq!(groups[0].descendants.len(), 4);
        assert_eq!(groups[1].qualified_name, "shop.admin");
        assert_eq!(groups[1].parent.as_deref(), Some("g0000"));
        assert_eq!(groups[1].direct_modules, ["admin-init", "users"]);
    }
}
