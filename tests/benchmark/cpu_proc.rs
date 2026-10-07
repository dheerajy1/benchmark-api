use std::fs;

use benchmark_api::features::v1::benchmark::metrics::cpu::proc::{
    parse_stat, read_group_ticks, read_group_ticks_in,
};
use tempfile::TempDir;

// Builds a /proc/<pid>/stat line, utime and stime are fields 14 and 15
fn stat_line(pid: u32, comm: &str, group: u32, user: u64, system: u64) -> String {
    format!(
        "{pid} ({comm}) S 1 {group} {group} 0 -1 4194304 10 0 0 0 {user} {system} 0 0 20 0 1 0 100 1000 100 18446744073709551615"
    )
}

fn write_process(root: &TempDir, pid: u32, content: &str) {
    let dir = root.path().join(pid.to_string());

    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("stat"), content).unwrap();
}

#[test]
fn parses_group_user_and_system_ticks() {
    let parsed = parse_stat(&stat_line(500, "bun", 321, 7, 3)).unwrap();

    assert_eq!(parsed.group_id, 321);
    assert_eq!(parsed.user, 7);
    assert_eq!(parsed.system, 3);
}

#[test]
fn total_is_user_plus_system() {
    let parsed = parse_stat(&stat_line(500, "bun", 321, 7, 3)).unwrap();

    assert_eq!(parsed.total(), 10);
}

#[test]
fn parses_comm_with_spaces_and_parentheses() {
    let parsed = parse_stat(&stat_line(500, "my (odd) app", 321, 40, 2)).unwrap();

    assert_eq!(parsed.group_id, 321);
    assert_eq!(parsed.total(), 42);
}

#[test]
fn rejects_malformed_stat_lines() {
    assert!(parse_stat("").is_none());
    assert!(parse_stat("500 bun S 1 321 321").is_none());
    assert!(parse_stat("500 (bun) S 1 321 321 0 -1").is_none());
    assert!(parse_stat(&stat_line(500, "bun", 321, 7, 3).replace(" 7 3 ", " x 3 ")).is_none());
}

#[test]
fn parses_the_stat_of_the_current_process() {
    let stat = fs::read_to_string("/proc/self/stat").unwrap();

    assert!(parse_stat(&stat).is_some());
}

#[test]
fn aggregates_only_processes_of_the_requested_group() {
    let root = TempDir::new().unwrap();

    write_process(&root, 100, &stat_line(100, "leader", 100, 7, 3));
    write_process(&root, 101, &stat_line(101, "worker", 100, 20, 5));
    write_process(&root, 200, &stat_line(200, "other", 200, 999, 1));

    let mut readings = read_group_ticks_in(root.path(), 100).unwrap();

    readings.sort_by_key(|item| item.pid);

    assert_eq!(readings.len(), 2);
    assert_eq!((readings[0].pid, readings[0].ticks), (100, 10));
    assert_eq!((readings[1].pid, readings[1].ticks), (101, 25));
}

#[test]
fn skips_entries_that_are_not_readable_processes() {
    let root = TempDir::new().unwrap();

    write_process(&root, 100, &stat_line(100, "leader", 100, 1, 1));
    write_process(&root, 400, "garbage");

    // Not a pid, and a pid directory that vanished before stat was read
    fs::create_dir_all(root.path().join("self")).unwrap();
    fs::create_dir_all(root.path().join("300")).unwrap();

    let readings = read_group_ticks_in(root.path(), 100).unwrap();

    assert_eq!(readings.len(), 1);
    assert_eq!(readings[0].pid, 100);
}

#[test]
fn group_without_live_processes_is_an_error() {
    let root = TempDir::new().unwrap();

    write_process(&root, 200, &stat_line(200, "other", 200, 1, 1));

    assert!(read_group_ticks_in(root.path(), 100).is_err());
}

#[test]
fn missing_proc_root_is_an_error() {
    let root = TempDir::new().unwrap();

    assert!(read_group_ticks_in(&root.path().join("missing"), 100).is_err());
}

#[test]
fn survivors_are_read_after_the_leader_exits() {
    let root = TempDir::new().unwrap();

    write_process(&root, 101, &stat_line(101, "worker", 100, 4, 4));

    let readings = read_group_ticks_in(root.path(), 100).unwrap();

    assert_eq!(readings.len(), 1);
    assert_eq!(readings[0].ticks, 8);
}

#[test]
fn unused_group_in_the_real_proc_is_an_error() {
    // Larger than any pid the kernel can hand out
    assert!(read_group_ticks(u32::MAX).is_err());
}
