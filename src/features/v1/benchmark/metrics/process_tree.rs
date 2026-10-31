use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

// Parent id, field 4 of /proc/<pid>/stat, read after the last ')' because comm may contain
// spaces and parentheses
pub fn parse_parent_id(stat: &str) -> Option<u32> {
    let end = stat.rfind(')')?;

    stat[end + 1..]
        .split_whitespace()
        .nth(1)?
        .parse::<u32>()
        .ok()
}

// The process and everything below it, found through parent ids on the host. A container's
// processes show up here with host ids, so this follows a container's process tree too. Err
// when the root itself is not running any more.
pub fn tree_members(root: u32) -> Result<Vec<u32>, String> {
    let entries =
        fs::read_dir("/proc").map_err(|error| format!("Failed to read /proc: {error}"))?;

    let mut parents: BTreeMap<u32, u32> = BTreeMap::new();

    for entry in entries.flatten() {
        let name = entry.file_name();

        let Some(pid) = name.to_str().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };

        // A process may exit between listing and reading, skip it
        let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };

        if let Some(parent) = parse_parent_id(&stat) {
            parents.insert(pid, parent);
        }
    }

    collect_tree(&parents, root)
}

// Pure part of the search, parents maps every process to its parent
pub fn collect_tree(parents: &BTreeMap<u32, u32>, root: u32) -> Result<Vec<u32>, String> {
    if !parents.contains_key(&root) {
        return Err(format!("Process {root} is not running"));
    }

    let mut members = vec![root];
    let mut seen: BTreeSet<u32> = BTreeSet::from([root]);
    let mut index = 0;

    while index < members.len() {
        let current = members[index];

        for (pid, parent) in parents {
            if *parent == current && seen.insert(*pid) {
                members.push(*pid);
            }
        }

        index += 1;
    }

    Ok(members)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_parent_even_with_odd_names() {
        let stat = "42 (my (odd) name) S 7 42 42 0 -1";

        assert_eq!(parse_parent_id(stat), Some(7));
    }

    #[test]
    fn collects_children_and_grandchildren_only() {
        let parents = BTreeMap::from([(10, 1), (11, 10), (12, 11), (13, 10), (20, 1)]);

        let mut members = collect_tree(&parents, 10).unwrap();
        members.sort_unstable();

        assert_eq!(members, vec![10, 11, 12, 13]);
    }

    #[test]
    fn a_missing_root_is_an_error() {
        let parents = BTreeMap::from([(10, 1)]);

        assert!(collect_tree(&parents, 99).is_err());
    }
}
