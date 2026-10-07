use std::{fs, path::Path};

// Linux USER_HZ, the unit of utime and stime in /proc/<pid>/stat
pub const USER_HZ: u64 = 100;

const PROC_ROOT: &str = "/proc";

// CPU accounting fields of one /proc/<pid>/stat line
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatTicks {
    pub group_id: u32,
    pub user: u64,
    pub system: u64,
}

impl StatTicks {
    // User plus system ticks
    pub fn total(&self) -> u64 {
        self.user.saturating_add(self.system)
    }
}

// Total CPU ticks of one process at the time of reading
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessTicks {
    pub pid: u32,
    pub ticks: u64,
}

// Fields 5 (pgrp), 14 (utime) and 15 (stime), parsed after the last ')' because comm may contain spaces
pub fn parse_stat(stat: &str) -> Option<StatTicks> {
    let end = stat.rfind(')')?;

    // Index 0 is field 3 (state), so field N is at index N - 3
    let fields: Vec<&str> = stat[end + 1..].split_whitespace().collect();

    let group_id = fields.get(2)?.parse::<u32>().ok()?;
    let user = fields.get(11)?.parse::<u64>().ok()?;
    let system = fields.get(12)?.parse::<u64>().ok()?;

    Some(StatTicks {
        group_id,
        user,
        system,
    })
}

// Reads user + system ticks of every live process in the process group
pub fn read_group_ticks(group_id: u32) -> Result<Vec<ProcessTicks>, String> {
    read_group_ticks_in(Path::new(PROC_ROOT), group_id)
}

// Same as read_group_ticks with an explicit proc root
pub fn read_group_ticks_in(root: &Path, group_id: u32) -> Result<Vec<ProcessTicks>, String> {
    let entries = fs::read_dir(root)
        .map_err(|error| format!("Failed to read {}: {error}", root.display()))?;

    let mut readings = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name();

        let Some(pid) = name.to_str().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };

        // A process may exit between listing and reading, skip it
        let Ok(stat) = fs::read_to_string(root.join(pid.to_string()).join("stat")) else {
            continue;
        };

        let Some(parsed) = parse_stat(&stat) else {
            continue;
        };

        if parsed.group_id != group_id {
            continue;
        }

        readings.push(ProcessTicks {
            pid,
            ticks: parsed.total(),
        });
    }

    if readings.is_empty() {
        return Err("No live processes found in the target process group".to_string());
    }

    Ok(readings)
}
